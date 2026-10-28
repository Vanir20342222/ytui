//! Local postprocessing orchestration and no-clobber publication.
use crate::{
    AudioOptions, DownloadOptions, DownloadPhase, DownloadProgress, Error, Result,
    SubtitleEmbedding, VideoInfo, VideoOptions, YoutubeDl, audio_original_path, audio_output_path,
};
use std::{
    fs,
    io::Read,
    ops::ControlFlow,
    path::{Path, PathBuf},
};

/// Local processors run in upstream order: audio, remux/recode, then subtitles.
/// Source and intermediate retention is enabled by either keep flag.
#[derive(Debug, Clone, Default)]
pub struct PostprocessOptions {
    pub audio: Option<AudioOptions>,
    pub video: Option<VideoOptions>,
    pub keep_video: bool,
    pub embed: Option<SubtitleEmbedding>,
    /// `--embed-metadata`: upstream `FFmpegMetadataPP` after subtitle embedding.
    pub metadata: bool,
    /// `--embed-chapters` (implied by `--embed-metadata`): the same processor
    /// adds the info dict's chapters.
    pub chapters: bool,
    /// `--embed-info-json` (`if_exists` with `--embed-metadata`): attach the
    /// info JSON to mkv/mka files.
    pub infojson: Option<InfoJsonEmbed>,
    /// Upstream `fixup()` before the processors; `None` skips fixups.
    pub fixup: Option<crate::FixupOptions>,
    /// `--embed-thumbnail`: a local thumbnail embedded after metadata.
    pub thumbnail: Option<PathBuf>,
    /// `--ignore-errors`: a failed processor is reported and skipped, and the
    /// remaining processors continue with the latest completed file.
    pub ignore_errors: bool,
    /// `--no-post-overwrites`: audio extraction keeps an existing output.
    pub no_post_overwrites: bool,
    /// `--remove-chapters`: upstream `ModifyChaptersPP` after subtitle
    /// embedding.
    pub modify_chapters: Option<ModifyChapters>,
    /// `--parse-metadata`/`--replace-in-metadata` actions with their
    /// template options: the `post_process` ones run after the fixups and
    /// before the other processors, changing the info dict they read.
    pub metadata_parser: Option<(
        crate::metadata_parser::MetadataActions,
        crate::output::TemplateOptions,
    )>,
    /// An already-downloaded file (the destination) to process instead of
    /// downloading, like upstream's `post_process` after `has already been
    /// downloaded`. It is replaced in place by same-named results and
    /// removed after a successful conversion unless kept.
    pub existing: Option<PathBuf>,
}

/// `--remove-chapters REGEX` (with `--force-keyframes-at-cuts`).
#[derive(Debug, Clone)]
pub struct ModifyChapters {
    pub patterns: Vec<regex::Regex>,
    /// `--sponsorblock-remove` categories (non-skippable ones are ignored).
    pub remove_sponsor: Vec<String>,
    /// `--sponsorblock-chapter-title`, an output template.
    pub sponsor_title: String,
    pub force_keyframes: bool,
    /// Subtitle files on disk (with their extensions), cut as well.
    pub subtitles: Vec<(PathBuf, String)>,
}

/// Upstream `prepend_extension(path, infix)`: `NAME.INFIX.EXT`.
fn prepend_extension(path: &Path, infix: &str) -> PathBuf {
    let mut name = path.file_stem().unwrap_or_default().to_os_string();
    name.push(format!(".{infix}"));
    if let Some(ext) = path.extension() {
        name.push(".");
        name.push(ext);
    }
    path.with_file_name(name)
}

/// Upstream `FFmpegMetadataPP` `add_infojson`.
#[derive(Debug, Clone, Default)]
pub struct InfoJsonEmbed {
    /// The info JSON written for this video, if any.
    pub written: Option<PathBuf>,
    /// `True` (`--embed-info-json`) rather than `if_exists`: a missing info
    /// JSON is written from `json`, and other containers report that it
    /// can only be attached to mkv/mka files.
    pub explicit: bool,
    pub json: Vec<u8>,
}

/// Published final media and any sources/intermediates retained by keep-video.
#[derive(Debug, Clone)]
pub struct ProcessedDownload {
    pub path: PathBuf,
    pub bytes: u64,
    /// The original downloaded/merged source, if retained.
    pub kept_video: Option<PathBuf>,
    /// All retained files, including kept_video when present.
    pub kept_files: Vec<PathBuf>,
    pub warnings: Vec<String>,
    /// Processor failures skipped under `ignore_errors`.
    pub errors: Vec<String>,
    /// Chapters and duration after `--remove-chapters` changed them.
    pub chapters: Option<(Vec<crate::info::Chapter>, Option<crate::info::PyNumber>)>,
    /// The embedded thumbnail after its WebP extension fixup.
    pub thumbnail: Option<PathBuf>,
    /// The info dict as `post_process` metadata parsing and
    /// `ModifyChaptersPP` left it, when they changed it.
    pub info: Option<VideoInfo>,
}

/// The `post_process` `MetadataParserPP` on a copy of `info`, if it runs.
fn parse_post_process(
    processors: &PostprocessOptions,
    info: &VideoInfo,
) -> Result<Option<VideoInfo>> {
    let Some((actions, options)) = &processors.metadata_parser else {
        return Ok(None);
    };
    if !actions.has("post_process") {
        return Ok(None);
    }
    let mut parsed = info.clone();
    actions.run_reported("post_process", &mut parsed, options)?;
    Ok(Some(parsed))
}

struct Stage {
    disk: PathBuf,
    path: PathBuf,
    ext: String,
    container: String,
    bytes: u64,
}

fn retained_paths(stages: &[Stage], final_path: Option<&Path>) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for (index, stage) in stages.iter().enumerate() {
        let collision = final_path == Some(stage.path.as_path())
            || stages[index + 1..]
                .iter()
                .any(|later| later.path == stage.path);
        let path = if collision {
            audio_original_path(&stage.path)?
        } else {
            stage.path.clone()
        };
        if paths.contains(&path) || final_path == Some(path.as_path()) {
            return Err(Error::Postprocess("retained media paths collide".into()));
        }
        paths.push(path);
    }
    Ok(paths)
}

fn absent(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(Error::OutputExists(path.into())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}
/// Upstream processors overwrite their outputs (`os.replace`, `ffmpeg -y`);
/// only existing directories, symlinks and other nonregular files are
/// refused.
fn writable(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(()),
        Ok(_) => Err(Error::OutputExists(path.into())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}
fn publish(source: &Path, destination: &Path) -> Result<()> {
    fs::OpenOptions::new()
        .write(true)
        .open(source)?
        .sync_all()?;
    writable(destination)?;
    tempfile::TempPath::try_from_path(source)?
        .persist(destination)
        .map_err(|error| Error::Io(error.error))?;
    Ok(())
}
fn checkpoint(
    callback: &mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
    bytes: u64,
    phase: DownloadPhase,
) -> ControlFlow<()> {
    callback(DownloadProgress {
        attempt: 1,
        downloaded_bytes: bytes,
        total_bytes: Some(bytes),
        phase,
        stream_index: None,
        stream_count: None,
        fragment_index: None,
        fragment_count: None,
        fragment: None,
    })
}
fn probe_path(downloader: &YoutubeDl) -> Result<PathBuf> {
    if let Some(path) = &downloader.ffprobe {
        return Ok(path.clone());
    }
    let ffmpeg = downloader.ffmpeg.as_ref().ok_or_else(|| Error::Postprocess("audio inspection requires --ffprobe-location PATH or ffprobe beside explicit --ffmpeg-location".into()))?;
    let name = if ffmpeg
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
    {
        "ffprobe.exe"
    } else {
        "ffprobe"
    };
    Ok(ffmpeg.with_file_name(name))
}

pub(crate) fn download(
    downloader: &YoutubeDl,
    info: &VideoInfo,
    destination: &Path,
    options: DownloadOptions,
    processors: &PostprocessOptions,
    callback: &mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
) -> Result<ProcessedDownload> {
    let existing = processors.existing.as_deref();
    // `postprocessed_by_ffmpeg`: VideoRemuxer/VideoConvertor mapping the
    // extension to another container.
    let converted = |fixup: &crate::FixupOptions| crate::FixupOptions {
        converted: processors
            .video
            .as_ref()
            .and_then(|video| video.plan(&info.ext).ok())
            .is_some_and(|plan| !plan.skip),
        ..fixup.clone()
    };
    if existing.is_none() && !options.overwrite {
        absent(destination)?;
    }
    let needs_audio = processors
        .audio
        .as_ref()
        .is_some_and(|audio| audio.needs_probe(&info.ext));
    let needs_video = processors
        .video
        .as_ref()
        .map(|video| video.plan(&info.ext))
        .transpose()?
        .is_some_and(|plan| !plan.skip);
    if !needs_audio
        && !needs_video
        && processors.embed.is_none()
        && !processors.metadata
        && !processors.chapters
        && processors.infojson.is_none()
        && processors.thumbnail.is_none()
        && processors.modify_chapters.is_none()
    {
        let bytes = match existing {
            Some(existing) => fs::metadata(existing)?.len(),
            None => {
                downloader.download_with_progress(info, destination, options, &mut *callback)?
            }
        };
        // `fixup()` of a fresh download precedes the processors.
        if let Some(fixup) = &processors.fixup
            && existing.is_none()
        {
            if checkpoint(callback, bytes, DownloadPhase::Completed).is_break() {
                return Err(Error::DownloadCancelled);
            }
            downloader.fixup(info, destination, destination, &converted(fixup))?;
        }
        let parsed = parse_post_process(processors, info)?;
        // Upstream still runs the processors, which report why they skip.
        if let Some(audio) = &processors.audio {
            crate::screen::postprocessor("ExtractAudio", "started");
            let reason = match audio.target(&info.ext) {
                None => format!("could not find a mapping for {}", info.ext),
                Some(_) => "the file is already in a common audio format".into(),
            };
            crate::screen::screen(&format!(
                "[ExtractAudio] Not converting audio {}; {reason}",
                destination.display()
            ));
            crate::screen::postprocessor("ExtractAudio", "finished");
        }
        if let Some(video) = &processors.video {
            let (action, name) = video.action();
            crate::screen::postprocessor(name, "started");
            if let Some(reason) = video.skip_reason(&info.ext) {
                crate::screen::screen(&format!(
                    "[{name}] Not {action} media file \"{}\"; {reason}",
                    destination.display()
                ));
            }
            crate::screen::postprocessor(name, "finished");
        }
        return Ok(ProcessedDownload {
            path: destination.into(),
            bytes,
            kept_video: None,
            kept_files: Vec::new(),
            warnings: Vec::new(),
            errors: Vec::new(),
            chapters: None,
            thumbnail: None,
            info: parsed,
        });
    }
    let keep = processors.keep_video
        || processors
            .audio
            .as_ref()
            .is_some_and(|audio| audio.keep_video);
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let workspace = tempfile::Builder::new()
        .prefix(".yt-dlp-rs-process-")
        .tempdir_in(parent)?;
    let input = workspace.path().join(format!(
        "source.{}",
        crate::output::safe_component(&info.ext)
    ));
    // Upstream merges selected formats from `NAME.fID.EXT` files beside the
    // destination (printing their downloads and the merge) before the
    // processors run; the merged file then becomes the private source.
    let upstream_merge = existing.is_none()
        && !info.requested_formats.is_empty()
        && matches!(options.merge_files, crate::MergeFiles::Upstream { .. })
        && !downloader.uses_ffmpeg_fd(info);
    let bytes = if let Some(existing) = existing {
        // A private link (or copy) of the existing file is the source.
        if fs::hard_link(existing, &input).is_err() {
            fs::copy(existing, &input)?;
        }
        fs::metadata(&input)?.len()
    } else if upstream_merge {
        // Each stream's download reports its own completion.
        let bytes =
            downloader.download_with_progress(info, destination, options, &mut *callback)?;
        fs::rename(destination, &input)?;
        bytes
    } else {
        // The workspace source is private; part files apply to unprocessed
        // media only.
        let options = DownloadOptions {
            part_files: crate::PartFiles::Private,
            merge_files: crate::MergeFiles::Private,
            ..options
        };
        downloader.download_with_progress(info, &input, options, |progress| {
            if progress.phase == DownloadPhase::Publishing {
                ControlFlow::Continue(())
            } else {
                callback(progress)
            }
        })?
    };
    // Upstream `process_info` fixes the download (after its progress ends)
    // before `post_process` and its prints.
    // Like upstream's default policy, existing files are not fixed again.
    if let Some(fixup) = &processors.fixup
        && existing.is_none()
    {
        if checkpoint(callback, bytes, DownloadPhase::Completed).is_break() {
            return Err(Error::DownloadCancelled);
        }
        downloader.fixup(info, &input, destination, &converted(fixup))?;
    }
    // End the transfer's progress before processors print (upstream reports
    // the download finished before running postprocessors).
    if checkpoint(callback, bytes, DownloadPhase::Postprocessing).is_break() {
        return Err(Error::DownloadCancelled);
    }
    let parsed = parse_post_process(processors, info)?;
    // HLS media is labeled `mp4` like upstream; FFmpeg reads its real container.
    let container = crate::container::sniff(&input)
        .map(str::to_owned)
        .unwrap_or_else(|| info.ext.to_lowercase());
    let source = input.clone();
    // Publish a stage's file to `path`: an unchanged existing source stays in
    // place, and results named like the existing file replace it.
    let place = |disk: &Path, path: &Path| -> Result<()> {
        if existing == Some(path) {
            if disk != source {
                fs::rename(disk, path)?;
            }
            return Ok(());
        }
        if disk == source
            && let Some(existing) = existing
        {
            writable(path)?;
            return Ok(fs::rename(existing, path)?);
        }
        publish(disk, path)
    };
    let mut stages = vec![Stage {
        disk: input,
        path: destination.into(),
        ext: info.ext.clone(),
        container,
        bytes,
    }];
    let mut publishing = false;
    // Info dict changes made by processors (`ModifyChaptersPP`).
    let parsed_metadata = parsed.is_some();
    let mut working = parsed.unwrap_or_else(|| info.clone());
    let mut chapters_changed = false;
    let mut embedded_thumbnail = None;
    // Files kept with -k beside the results (`NAME.uncut.EXT`).
    let mut uncut: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut delete_subtitles = Vec::new();
    let mut warnings = Vec::new();
    let mut errors = Vec::new();
    // Upstream `run_pp` deletion notices for a processor's replaced inputs,
    // printed after the processor (and its `finished` hook) returns.
    let notices = std::cell::RefCell::new(Vec::new());
    let deleted = |path: &Path| {
        if !keep {
            notices.borrow_mut().push(format!(
                "Deleting original file {} (pass -k to keep)",
                path.display()
            ));
        }
    };
    let process = (|| -> Result<ProcessedDownload> {
        // Upstream `run_pp`: with --ignore-errors a failed processor is
        // reported and skipped without its file deletions.
        // `run_wrapper`'s progress hooks surround a processor that runs.
        macro_rules! stage {
            ($key:expr, $runs:expr, $body:block) => {{
                let subtitle_count = delete_subtitles.len();
                let runs: bool = $runs;
                if runs {
                    crate::screen::postprocessor($key, "started");
                }
                let outcome = (|| -> Result<()> {
                    $body
                    Ok(())
                })();
                if runs && outcome.is_ok() {
                    crate::screen::postprocessor($key, "finished");
                }
                // A failed processor deletes nothing.
                for notice in notices.borrow_mut().drain(..) {
                    if outcome.is_ok() {
                        crate::screen::screen(&notice);
                    }
                }
                match outcome {
                    Ok(()) => {}
                    Err(Error::DownloadCancelled) => return Err(Error::DownloadCancelled),
                    Err(error) if processors.ignore_errors => {
                        delete_subtitles.truncate(subtitle_count);
                        errors.push(error.to_string());
                    }
                    Err(error) => return Err(error),
                }
            }};
        }
        stage!("ExtractAudio", processors.audio.is_some(), {
            if needs_audio {
                let current = stages.last().unwrap();
                if checkpoint(callback, current.bytes, DownloadPhase::Postprocessing).is_break() {
                    return Err(Error::DownloadCancelled);
                }
                let mut observe =
                    || checkpoint(callback, current.bytes, DownloadPhase::Postprocessing);
                let codec = crate::audio::ffmpeg::probe(
                    &probe_path(downloader)?,
                    &current.disk,
                    &current.container,
                    &mut observe,
                )?;
                let plan = processors
                    .audio
                    .as_ref()
                    .unwrap()
                    .plan(&current.ext, Some(&codec))?;
                let path = audio_output_path(&current.path, &current.ext, &plan.extension)?;
                let options = processors.audio.as_ref().unwrap();
                if plan.skip {
                    // Upstream `Not converting audio PATH; REASON`.
                    let reason = match options.target(&current.ext) {
                        None => format!("could not find a mapping for {}", current.ext),
                        Some(_) => "the file is already in a common audio format".into(),
                    };
                    crate::screen::screen(&format!(
                        "[ExtractAudio] Not converting audio {}; {reason}",
                        current.path.display()
                    ));
                } else if plan.encoder.as_deref() == Some("copy") && path == current.path {
                    crate::screen::screen(&format!(
                        "[ExtractAudio] Not converting audio {}; file is already in target format {}",
                        current.path.display(),
                        options.target(&current.ext).unwrap_or("best")
                    ));
                }
                let converts = !plan.skip
                    && !(plan.encoder.as_deref() == Some("copy") && path == current.path);
                // `--no-post-overwrites`: an existing output (with its
                // original on disk) is kept.
                let original_exists = path != current.path
                    || fs::symlink_metadata(audio_original_path(&current.path)?).is_ok();
                let kept = converts
                    && processors.no_post_overwrites
                    && fs::symlink_metadata(&path).is_ok()
                    && original_exists;
                if kept {
                    crate::screen::screen(&format!(
                        "[ExtractAudio] Post-process file {} exists, skipping",
                        path.display()
                    ));
                }
                if converts && !kept {
                    crate::screen::screen(&format!(
                        "[ExtractAudio] Destination: {}",
                        path.display()
                    ));
                    writable(&path)?;
                    let executable = downloader.ffmpeg.as_ref().ok_or_else(|| {
                        Error::Postprocess(
                            "audio conversion requires an explicit --ffmpeg-location PATH".into(),
                        )
                    })?;
                    let output = workspace.path().join(format!("audio.{}", plan.extension));
                    let mut observe =
                        || checkpoint(callback, current.bytes, DownloadPhase::Postprocessing);
                    crate::audio::ffmpeg::convert(
                        executable,
                        &current.disk,
                        &current.container,
                        &output,
                        &plan,
                        &mut observe,
                    )?;
                    // Upstream FFmpeg outputs take the oldest input mtime.
                    crate::filetime::inherit(
                        &output,
                        crate::filetime::oldest([current.disk.as_path()]),
                    );
                    let bytes = fs::metadata(&output)?.len();
                    // AAC copy deliberately keeps upstream's m4a name with ADTS bytes.
                    let container = if plan.muxer == "adts" {
                        "aac".into()
                    } else {
                        plan.extension.clone()
                    };
                    // A same-named output moves the input to `NAME.orig.EXT`.
                    let original = if path == current.path {
                        audio_original_path(&current.path)?
                    } else {
                        current.path.clone()
                    };
                    stages.push(Stage {
                        disk: output,
                        path,
                        ext: plan.extension,
                        container,
                        bytes,
                    });
                    deleted(&original);
                }
            }
        });
        stage!(
            processors
                .video
                .as_ref()
                .map_or("", |video| video.action().1),
            processors.video.is_some(),
            {
                if let Some(video) = &processors.video {
                    let current = stages.last().unwrap();
                    let plan = video.plan(&current.ext)?;
                    let (action, name) = video.action();
                    if let Some(reason) = video.skip_reason(&current.ext).filter(|_| plan.skip) {
                        crate::screen::screen(&format!(
                            "[{name}] Not {action} media file \"{}\"; {reason}",
                            current.path.display()
                        ));
                    }
                    if !plan.skip {
                        let path = audio_output_path(
                            &current.path,
                            &current.ext.to_lowercase(),
                            &plan.extension,
                        )?;
                        let mut title = action.to_owned();
                        title[..1].make_ascii_uppercase();
                        crate::screen::screen(&format!(
                            "[{name}] {title} video from {} to {}; Destination: {}",
                            current.ext.to_lowercase(),
                            plan.extension,
                            path.display()
                        ));
                        writable(&path)?;
                        if checkpoint(callback, current.bytes, DownloadPhase::Postprocessing)
                            .is_break()
                        {
                            return Err(Error::DownloadCancelled);
                        }
                        let executable = downloader.ffmpeg.as_ref().ok_or_else(|| {
                            Error::Postprocess(
                                "video processing requires an explicit --ffmpeg-location PATH"
                                    .into(),
                            )
                        })?;
                        let output = workspace.path().join(format!("video.{}", plan.extension));
                        let mut observe =
                            || checkpoint(callback, current.bytes, DownloadPhase::Postprocessing);
                        crate::video::ffmpeg::convert(
                            executable,
                            &current.disk,
                            &current.container,
                            &output,
                            &plan,
                            &mut observe,
                        )?;
                        // Upstream FFmpeg outputs take the oldest input mtime.
                        crate::filetime::inherit(
                            &output,
                            crate::filetime::oldest([current.disk.as_path()]),
                        );
                        let bytes = fs::metadata(&output)?.len();
                        let original = current.path.clone();
                        stages.push(Stage {
                            disk: output,
                            path,
                            ext: plan.extension.clone(),
                            container: plan.extension,
                            bytes,
                        });
                        deleted(&original);
                    }
                }
            }
        );
        stage!("EmbedSubtitle", processors.embed.is_some(), {
            if let Some(embedding) = &processors.embed {
                let subtitle_start = delete_subtitles.len();
                let current = stages.last_mut().unwrap();
                let mut plan = crate::embed_subtitles::plan(&current.ext, &embedding.subtitles)?;
                warnings.append(&mut plan.warnings);
                warnings.append(&mut plan.messages);
                if !plan.skip {
                    if checkpoint(callback, current.bytes, DownloadPhase::Postprocessing).is_break()
                    {
                        return Err(Error::DownloadCancelled);
                    }
                    let mut total = 0u64;
                    let mut seen = std::collections::HashSet::new();
                    // Upstream FFmpeg inputs are the original subtitle files.
                    let mut oldest = crate::filetime::oldest([current.disk.as_path()]);
                    for (index, file) in plan.files.iter_mut().enumerate() {
                        if checkpoint(callback, current.bytes, DownloadPhase::Postprocessing)
                            .is_break()
                        {
                            return Err(Error::DownloadCancelled);
                        }
                        let metadata = subtitle_metadata(&file.path)?;
                        oldest = oldest.into_iter().chain(metadata.modified().ok()).min();
                        if metadata.len() > 16 * 1024 * 1024 {
                            return Err(Error::Postprocess(
                                "subtitle embedding input exceeds 16 MiB".into(),
                            ));
                        }
                        let canonical = file.path.canonicalize()?;
                        let mut bytes = Vec::new();
                        fs::File::open(&file.path)?
                            .take(16 * 1024 * 1024 + 1)
                            .read_to_end(&mut bytes)?;
                        total += bytes.len() as u64;
                        if bytes.len() > 16 * 1024 * 1024 || total > 256 * 1024 * 1024 {
                            return Err(Error::Postprocess(
                                "subtitle embedding inputs exceed byte budget".into(),
                            ));
                        }
                        if !same_subtitle(&metadata, &subtitle_metadata(&file.path)?) {
                            return Err(Error::Postprocess(
                                "subtitle changed during embedding".into(),
                            ));
                        }
                        if !embedding.keep_subtitles && !keep && seen.insert(canonical) {
                            delete_subtitles.push((file.path.clone(), metadata));
                        }
                        let private = workspace
                            .path()
                            .join(format!("subtitle-{index}.{}", file.ext));
                        fs::write(&private, bytes)?;
                        file.path = private;
                    }
                    let executable = downloader.ffmpeg.as_ref().ok_or_else(|| {
                        Error::Postprocess(
                            "subtitle embedding requires an explicit --ffmpeg-location PATH".into(),
                        )
                    })?;
                    crate::screen::screen(&format!(
                        "[EmbedSubtitle] Embedding subtitles in \"{}\"",
                        current.path.display()
                    ));
                    let output = workspace.path().join(format!("embedded.{}", current.ext));
                    let mut observe =
                        || checkpoint(callback, current.bytes, DownloadPhase::Postprocessing);
                    crate::embed_subtitles::ffmpeg::embed(
                        executable,
                        &current.disk,
                        &current.container,
                        &output,
                        &plan,
                        &mut observe,
                    )?;
                    crate::filetime::inherit(&output, oldest);
                    // Upstream replaces the current media and returns no media deletion.
                    // Keep-video therefore retains earlier processors' inputs, not a
                    // new unembedded copy of this stage.
                    current.bytes = fs::metadata(&output)?.len();
                    current.disk = output;
                    current.container = current.ext.clone();
                    // Upstream returns the embedded subtitle files for deletion.
                    for (path, _) in &delete_subtitles[subtitle_start..] {
                        notices.borrow_mut().push(format!(
                            "Deleting original file {} (pass -k to keep)",
                            path.display()
                        ));
                    }
                }
            }
        });
        stage!("ModifyChapters", processors.modify_chapters.is_some(), {
            if let Some(modify) = &processors.modify_chapters {
                use crate::{
                    info::PyNumber,
                    modify_chapters::{self as mc, Segment},
                };
                let current = stages.last_mut().unwrap();
                let say = |text: &str| crate::screen::screen(&format!("[ModifyChapters] {text}"));
                let ffprobe = probe_path(downloader)?;
                let probe = || mc::probe_duration(&ffprobe, &current.disk, &current.container);
                // `_fixup_chapters`: an open last chapter ends with the media.
                let mut real = None;
                if working
                    .metadata
                    .chapters
                    .last()
                    .is_some_and(|chapter| chapter.end_time.is_none())
                {
                    let duration = probe()?;
                    real = Some(duration);
                    if let Some(last) = working.metadata.chapters.last_mut() {
                        last.end_time = Some(PyNumber::Float(duration));
                    }
                }
                // `_mark_chapters_to_remove`.
                let mut segments: Vec<Segment> = working
                    .metadata
                    .chapters
                    .iter()
                    .map(|chapter| {
                        Segment::chapter(
                            chapter.start_time.unwrap_or(PyNumber::Int(0)),
                            chapter.end_time.unwrap_or(PyNumber::Int(0)),
                            chapter.title.clone(),
                        )
                    })
                    .collect();
                if !modify.patterns.is_empty() {
                    let mut nothing_matched = true;
                    if segments.is_empty() {
                        say("Chapter information is unavailable");
                        nothing_matched = false;
                    }
                    for segment in &mut segments {
                        let title = segment.title.as_deref().unwrap_or("");
                        if modify.patterns.iter().any(|regex| regex.is_match(title)) {
                            segment.remove = true;
                            nothing_matched = false;
                        }
                    }
                    if nothing_matched {
                        say("There are no chapters matching the regex");
                    }
                }
                let mut sponsors: Vec<Segment> = working
                    .extra
                    .get("sponsorblock_chapters")
                    .and_then(serde_json::Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(mc::Segment::from_sponsor_json)
                    .collect();
                let remove: Vec<&String> = modify
                    .remove_sponsor
                    .iter()
                    .filter(|category| {
                        !crate::sponsorblock::NON_SKIPPABLE.contains(&category.as_str())
                    })
                    .collect();
                if !remove.is_empty() {
                    let mut nothing_matched = true;
                    if sponsors.is_empty() {
                        say("SponsorBlock information is unavailable");
                        nothing_matched = false;
                    }
                    for sponsor in &mut sponsors {
                        if sponsor
                            .category()
                            .is_some_and(|category| remove.iter().any(|r| r.as_str() == category))
                        {
                            sponsor.remove = true;
                            nothing_matched = false;
                        }
                    }
                    if nothing_matched {
                        say("There are no matching SponsorBlock chapters");
                    }
                }
                if segments.is_empty() && sponsors.is_empty() {
                    return Ok(());
                }
                let real = match real {
                    Some(real) => real,
                    None => probe()?,
                };
                if segments.is_empty() {
                    let end = working
                        .metadata
                        .duration
                        .filter(|duration| duration.as_f64() != 0.0)
                        .unwrap_or(PyNumber::Float(real));
                    segments.push(Segment::chapter(
                        PyNumber::Int(0),
                        end,
                        Some(working.title.clone()),
                    ));
                }
                segments.extend(sponsors);
                // `evaluate_outtmpl(sponsorblock_chapter_title, chapter)`.
                let title = |sponsor: &mc::Sponsor, chapter: &serde_json::Value| {
                    let mut dict = chapter.clone();
                    if let Some(object) = dict.as_object_mut() {
                        object.insert("category".into(), sponsor.category.clone().into());
                        object.insert("categories".into(), sponsor.categories.clone().into());
                        object.insert("name".into(), sponsor.name.clone().into());
                        object.insert(
                            "category_names".into(),
                            sponsor.category_names.clone().into(),
                        );
                    }
                    crate::output::evaluate_dict_template(
                        &modify.sponsor_title,
                        &dict,
                        &crate::output::TemplateOptions::default(),
                    )
                    .unwrap_or_else(|_| mc::default_sponsor_title(sponsor))
                };
                let (arranged, cuts) = mc::arrange_titled(segments, &title);
                working.metadata.chapters = arranged.iter().map(mc::Arranged::chapter).collect();
                chapters_changed = true;
                if cuts.is_empty() {
                    return Ok(());
                }
                let Some(last) = arranged.last() else {
                    crate::screen::warning(
                        "You have requested to remove the entire video, which is not possible",
                    );
                    return Ok(());
                };
                let original = working.metadata.duration.map(PyNumber::as_f64);
                let duration = last.end.as_f64();
                // `info['duration'] = info['chapters'][-1]['end_time']`.
                working.metadata.duration = Some(if duration < 0.0 {
                    PyNumber::Int(0)
                } else {
                    last.end
                });
                // `_duration_mismatch`.
                let mismatch = |a: Option<f64>, b: f64, tolerance: f64| {
                    a.is_some_and(|a| a != 0.0 && b != 0.0 && (a - b).abs() > tolerance)
                };
                if mismatch(original, real, 1.0) {
                    if !mismatch(Some(real), duration, 2.0) {
                        say("Skipping ModifyChapters since the video appears to be already cut");
                        return Ok(());
                    }
                    if existing.is_some() {
                        return Err(Error::Postprocess(
                            "Cannot cut video since the real and expected durations mismatch. Different chapters may have already been removed".into(),
                        ));
                    }
                    crate::screen::debug("Expected and actual durations mismatch");
                }
                let options = mc::concat_options(&cuts, real);
                crate::screen::debug(&format!(
                    "Concat spec = {}",
                    options
                        .iter()
                        .map(|entry| format!(
                            "{}-{}",
                            entry.inpoint.as_deref().unwrap_or("0.0"),
                            entry.outpoint.as_deref().unwrap_or("inf")
                        ))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
                if checkpoint(callback, current.bytes, DownloadPhase::Postprocessing).is_break() {
                    return Err(Error::DownloadCancelled);
                }
                let executable = downloader.ffmpeg.as_ref().ok_or_else(|| {
                    Error::Postprocess(
                        "chapter removal requires an explicit --ffmpeg-location PATH".into(),
                    )
                })?;
                let ext = current.ext.to_lowercase();
                let mut input = current.disk.clone();
                if modify.force_keyframes {
                    say(&format!(
                        "Re-encoding \"{}\" with appropriate keyframes",
                        current.path.display()
                    ));
                    let keyframes = workspace.path().join(format!("keyframes.{ext}"));
                    let times: Vec<PyNumber> =
                        cuts.iter().flat_map(|cut| [cut.start, cut.end]).collect();
                    mc::force_keyframes(
                        executable,
                        "ModifyChapters",
                        &input,
                        &current.container,
                        &ext,
                        &keyframes,
                        &times,
                    )?;
                    input = keyframes;
                }
                say(&format!(
                    "Removing chapters from {}",
                    current.path.display()
                ));
                let output = workspace.path().join(format!("cut.{ext}"));
                mc::concat(
                    executable,
                    "ModifyChapters",
                    &input,
                    &ext,
                    Some(crate::embed_metadata::muxer(&ext)),
                    &output,
                    &options,
                )?;
                // Subtitles on disk are cut too (embedded ones are gone).
                let mut subtitles = Vec::new();
                for (index, (path, sub_ext)) in modify.subtitles.iter().enumerate() {
                    if !path.is_file() || delete_subtitles.iter().any(|(gone, _)| gone == path) {
                        continue;
                    }
                    if !matches!(sub_ext.as_str(), "srt" | "vtt" | "ass" | "lrc") {
                        crate::screen::warning(&format!(
                            "Cannot remove chapters from external {sub_ext} subtitles; \"{}\" is now out of sync",
                            path.display()
                        ));
                        continue;
                    }
                    say(&format!("Removing chapters from {}", path.display()));
                    let cut = workspace.path().join(format!("subtitle-{index}.{sub_ext}"));
                    mc::concat(
                        executable,
                        "ModifyChapters",
                        path,
                        sub_ext,
                        None,
                        &cut,
                        &options,
                    )?;
                    subtitles.push((path.clone(), cut));
                }
                // Renaming happens after every file is cut; outputs keep the
                // inputs' modification times.
                crate::filetime::inherit(
                    &output,
                    crate::filetime::oldest([current.disk.as_path()]),
                );
                let previous = std::mem::replace(&mut current.disk, output);
                current.bytes = fs::metadata(&current.disk)?.len();
                let mut removed = vec![prepend_extension(&current.path, "uncut")];
                if keep {
                    uncut.push((previous, removed[0].clone()));
                }
                for (path, cut) in subtitles {
                    crate::filetime::inherit(&cut, crate::filetime::oldest([path.as_path()]));
                    let original = prepend_extension(&path, "uncut");
                    if keep {
                        writable(&original)?;
                        fs::rename(&path, &original)?;
                    } else {
                        fs::remove_file(&path)?;
                    }
                    if fs::rename(&cut, &path).is_err() {
                        fs::copy(&cut, &path)?;
                    }
                    removed.push(original);
                }
                for path in &removed {
                    deleted(path);
                }
            }
        });
        stage!(
            "Metadata",
            processors.metadata || processors.chapters || processors.infojson.is_some(),
            {
                if processors.metadata || processors.chapters || processors.infojson.is_some() {
                    let current = stages.last_mut().unwrap();
                    let chapters = processors.chapters && !working.metadata.chapters.is_empty();
                    // `_get_infojson_opts`: only mkv/mka files take the attachment.
                    let mut attachment = None;
                    if let Some(embed) = &processors.infojson {
                        if matches!(current.ext.to_lowercase().as_str(), "mkv" | "mka") {
                            attachment = match embed.written.as_ref().filter(|path| path.is_file())
                            {
                                // FFmpeg runs in the workspace, so name it absolutely.
                                Some(path) => Some(path.canonicalize()?),
                                None if embed.explicit => {
                                    let path = workspace.path().join("info.json.temp");
                                    fs::write(&path, &embed.json)?;
                                    Some(path)
                                }
                                None => None,
                            };
                        } else if embed.explicit {
                            crate::screen::screen(
                                "[Metadata] The info-json can only be attached to mkv/mka files",
                            );
                        }
                    }
                    let Some(args) = crate::embed_metadata::plan_args(
                        &working,
                        &current.ext,
                        processors.metadata,
                        chapters,
                        attachment.as_deref(),
                    ) else {
                        crate::screen::screen("[Metadata] There isn't any metadata to add");
                        return Ok(());
                    };
                    if checkpoint(callback, current.bytes, DownloadPhase::Postprocessing).is_break()
                    {
                        return Err(Error::DownloadCancelled);
                    }
                    let executable = downloader.ffmpeg.as_ref().ok_or_else(|| {
                        Error::Postprocess(
                            "metadata embedding requires an explicit --ffmpeg-location PATH".into(),
                        )
                    })?;
                    // Upstream writes `NAME.meta` beside the media and deletes it.
                    let metadata_input = if chapters {
                        let path = workspace.path().join("chapters.meta");
                        fs::write(
                            &path,
                            crate::embed_metadata::ffmetadata(&working.metadata.chapters),
                        )?;
                        Some(path)
                    } else {
                        None
                    };
                    let plan = crate::video::VideoPlan {
                        extension: current.ext.clone(),
                        args,
                        muxer: crate::embed_metadata::muxer(&current.ext.to_lowercase()).into(),
                        skip: false,
                        metadata_input,
                        pp: "Metadata",
                    };
                    crate::screen::screen(&format!(
                        "[Metadata] Adding metadata to \"{}\"",
                        current.path.display()
                    ));
                    let output = workspace.path().join(format!("metadata.{}", current.ext));
                    let mut observe =
                        || checkpoint(callback, current.bytes, DownloadPhase::Postprocessing);
                    crate::video::ffmpeg::convert(
                        executable,
                        &current.disk,
                        &current.container,
                        &output,
                        &plan,
                        &mut observe,
                    )?;
                    crate::filetime::inherit(
                        &output,
                        crate::filetime::oldest([current.disk.as_path()]),
                    );
                    // Like upstream, metadata replaces the current file in place.
                    current.bytes = fs::metadata(&output)?.len();
                    current.disk = output;
                    current.container = current.ext.to_lowercase();
                }
            }
        );
        stage!("EmbedThumbnail", processors.thumbnail.is_some(), {
            if let Some(thumbnail) = &processors.thumbnail {
                let current = stages.last_mut().unwrap();
                if checkpoint(callback, current.bytes, DownloadPhase::Postprocessing).is_break() {
                    return Err(Error::DownloadCancelled);
                }
                let executable = downloader.ffmpeg.as_ref().ok_or_else(|| {
                    Error::Postprocess(
                        "thumbnail embedding requires an explicit --ffmpeg-location PATH".into(),
                    )
                })?;
                let output = workspace
                    .path()
                    .join(format!("thumbnail-embedded.{}", current.ext));
                // Upstream `EmbedThumbnailPP` first corrects a WebP extension.
                let thumbnail = crate::fixup_webp(thumbnail)?;
                if thumbnail != *processors.thumbnail.as_ref().unwrap() {
                    crate::screen::screen(&format!(
                        "[ThumbnailsConvertor] Correcting thumbnail \"{}\" extension to webp",
                        processors.thumbnail.as_ref().unwrap().display()
                    ));
                }
                embedded_thumbnail = Some(thumbnail.clone());
                let thumbnail = &thumbnail;
                let mut observe =
                    || checkpoint(callback, current.bytes, DownloadPhase::Postprocessing);
                crate::thumbnail_process::embed(
                    executable,
                    &probe_path(downloader)?,
                    &current.disk,
                    &current.path,
                    &current.ext.to_lowercase(),
                    thumbnail,
                    workspace.path(),
                    &output,
                    &mut observe,
                )?;
                // Upstream restores the media's own modification time.
                crate::filetime::inherit(
                    &output,
                    crate::filetime::oldest([current.disk.as_path()]),
                );
                current.bytes = fs::metadata(&output)?.len();
                current.disk = output;
                current.container = current.ext.to_lowercase();
            }
        });
        publishing = true;
        let last = stages.last().unwrap();
        let retained = if keep {
            retained_paths(&stages[..stages.len() - 1], Some(&last.path))?
        } else {
            Vec::new()
        };
        writable(&last.path)?;
        for path in &retained {
            writable(path)?;
        }
        if checkpoint(callback, last.bytes, DownloadPhase::Publishing).is_break() {
            return Err(Error::DownloadCancelled);
        }
        for (stage, path) in stages.iter().zip(&retained) {
            place(&stage.disk, path)?;
        }
        for (disk, path) in &uncut {
            writable(path)?;
            place(disk, path)?;
        }
        place(&last.disk, &last.path)?;
        // Upstream deletes the processed original unless it is kept (also
        // the file a repeated download replaced).
        let replaced = existing.or(options.overwrite.then_some(destination));
        if let Some(existing) = replaced
            && existing != last.path
            && !retained.iter().any(|path| path == existing)
            && fs::symlink_metadata(existing).is_ok()
            && !keep
        {
            fs::remove_file(existing)?;
        }
        Ok(ProcessedDownload {
            path: last.path.clone(),
            bytes: last.bytes,
            kept_video: retained.first().cloned(),
            kept_files: retained,
            warnings,
            errors,
            chapters: chapters_changed
                .then(|| (working.metadata.chapters.clone(), working.metadata.duration)),
            thumbnail: embedded_thumbnail,
            info: (parsed_metadata || chapters_changed).then(|| working.clone()),
        })
    })();
    match process {
        Err(Error::DownloadCancelled) => Err(Error::DownloadCancelled),
        Err(error) => {
            // Retain the latest completed input on failure. Earlier successful
            // processor inputs are retained as well when keep-video is enabled.
            let count = if publishing {
                stages.len().saturating_sub(1).max(1)
            } else {
                stages.len()
            };
            let completed = &stages[..count];
            let paths = retained_paths(completed, None)?;
            let first = if keep { 0 } else { completed.len() - 1 };
            let mut failures = Vec::new();
            for (stage, path) in completed.iter().zip(paths).skip(first) {
                if stage.disk.exists()
                    && let Err(preserve) = place(&stage.disk, &path)
                {
                    // One retained-path collision must not discard another
                    // completed stage whose destination is still available.
                    failures.push(preserve.to_string());
                }
            }
            if !failures.is_empty() {
                return Err(Error::Postprocess(format!(
                    "{error}; cannot preserve completed media: {}",
                    failures.join("; ")
                )));
            }
            Err(error)
        }
        Ok(result) => {
            let mut failures = Vec::new();
            for (path, metadata) in delete_subtitles {
                let remove = (|| -> Result<()> {
                    if !same_subtitle(&metadata, &subtitle_metadata(&path)?) {
                        return Err(Error::Postprocess(
                            "embedded subtitle changed; source retained".into(),
                        ));
                    }
                    fs::remove_file(path)?;
                    Ok(())
                })();
                if let Err(error) = remove {
                    failures.push(error.to_string());
                }
            }
            if !failures.is_empty() {
                return Err(Error::Postprocess(format!(
                    "media published, but embedded subtitle cleanup failed: {}",
                    failures.join("; ")
                )));
            }
            Ok(result)
        }
    }
}

fn subtitle_metadata(path: &Path) -> Result<fs::Metadata> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(Error::Postprocess(
            "subtitle embedding requires regular local sidecars".into(),
        ));
    }
    Ok(metadata)
}
fn same_subtitle(before: &fs::Metadata, after: &fs::Metadata) -> bool {
    let same = before.len() == after.len() && before.modified().ok() == after.modified().ok();
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        same && before.dev() == after.dev() && before.ino() == after.ino()
    }
    #[cfg(not(unix))]
    {
        same
    }
}

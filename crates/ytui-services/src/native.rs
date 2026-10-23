//! Pinned native yt-dlp-rs integration. A worker process isolates blocking upstream I/O.
use crate::process::{self, ProcessResult, ProcessSpec};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    io::Write,
    ops::ControlFlow,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio_util::sync::CancellationToken;
use yt_dlp_rs::{
    AudioOptions, CookieJar, DownloadOptions, DownloadPhase, DownloadProgress, HttpOptions,
    PlaylistSelection, PostprocessOptions, SubtitleEmbedding, SubtitleFile, SubtitleOptions,
    VideoInfo, VideoMode, VideoOptions, YoutubeDl, YoutubeOptions,
};
use ytui_core::{Config, Job};

pub const UPSTREAM_REV: &str = "f67d96bdf98c5b821d0c9bf6175d328a0fa8ceab";
#[derive(Clone, Serialize, Deserialize)]
pub struct WorkerRequest {
    pub job: Job,
    pub config: Config,
    pub cookies: Option<PathBuf>,
    pub resolve: bool,
    #[serde(default)]
    pub staging: Option<PathBuf>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Resolved {
    pub url: String,
    pub audio_url: Option<String>,
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    #[serde(default)]
    pub catalog: crate::formats::Catalog,
}
pub(crate) fn playback_source(info: &VideoInfo, audio: bool) -> Result<Resolved> {
    // HLS manifests and direct HTTP URLs remain playable by mpv even when the
    // downloader also exposes an ordered fragment plan. Do not send a fragment
    // file or reject the entire stream merely because that plan exists.
    let stream = crate::playback_extract::parse(&serde_json::to_value(info)?, audio)?;
    Ok(Resolved {
        url: stream.url,
        audio_url: stream.audio,
        headers: stream.headers,
        catalog: crate::formats::Catalog::parse(&serde_json::to_value(info)?),
    })
}
pub fn worker(path: &Path) -> Result<()> {
    let request: WorkerRequest = serde_json::from_slice(&std::fs::read(path)?)?;
    let o = &request.job.options;
    if !request.resolve
        && o.playlist
        && ytui_core::http_url(&request.job.media.url)
            .is_ok_and(|url| ytui_core::youtube_host(url.host_str().unwrap_or_default()))
    {
        bail!(
            "Native YouTube playlists require yt-dlp fallback; no items were downloaded. Choose auto or yt-dlp to process the collection"
        );
    }
    // Do not pretend that upstream supports these features or silently discard options.
    if !request.resolve {
        ensure!(
            o.sponsorblock.is_empty() || !o.sponsorblock_remove || o.subtitles.is_empty(),
            "Native SponsorBlock removal with requested captions needs explicit yt-dlp fallback to preserve timing"
        );
        ensure!(
            !o.thumbnail || !request.job.media.thumbnail.is_empty(),
            "Native extractor did not supply cover art; choose a browse item with a thumbnail or retry explicitly with yt-dlp"
        );
    }
    let cookies = request
        .cookies
        .as_ref()
        .map(|p| CookieJar::load(p).map(Arc::new))
        .transpose()?;
    let client = YoutubeDl::with_options(
        YoutubeOptions {
            js_runtime: request.config.native_js_runtime.as_ref().map(|p| {
                crate::paths::executable(&p.to_string_lossy()).unwrap_or_else(|| p.clone())
            }),
            include_translated_subtitles: o.auto_subtitles,
            no_playlist: request.resolve || !o.playlist,
        },
        HttpOptions {
            cookies,
            proxy: request.config.proxy.clone(),
            socket_timeout: Some(15.0),
            ..Default::default()
        },
    )?
    .with_ffmpeg(
        crate::paths::executable(&request.config.ffmpeg)
            .unwrap_or_else(|| PathBuf::from(&request.config.ffmpeg)),
    );
    let selector = if let Some(selector) = &o.format_selector {
        selector.clone()
    } else if o.audio {
        "bestaudio/best".into()
    } else if o.quality == "best" {
        "bestvideo+bestaudio/best".into()
    } else {
        format!(
            "bestvideo[height<=?{}]+bestaudio/best[height<=?{}]",
            o.quality, o.quality
        )
    };
    let mut extracted =
        client.extract_result_with_format(&request.job.media.url, Some(&selector))?;
    if !o.playlist
        && !request.resolve
        && let yt_dlp_rs::ExtractResult::Playlist(ref mut p) = extracted
    {
        p.entries.truncate(1);
    }
    if let Some(items) = &o.playlist_items {
        PlaylistSelection::new(Some(items), 1, None, false)?.apply(&mut extracted);
    }
    let infos = extracted.into_entries();
    ensure!(!infos.is_empty(), "Native extractor returned no media");
    if request.resolve {
        let info = &infos[0];
        println!(
            "YTUI_RESOLVED:{}",
            serde_json::to_string(&playback_source(info, o.audio)?)?
        );
        std::io::stdout().flush()?;
        return Ok(());
    }
    let owned_stage = if request.staging.is_none() {
        Some(
            tempfile::Builder::new()
                .prefix(".ytui-job-")
                .tempdir_in(o.output_directory())?,
        )
    } else {
        None
    };
    let stage_root = request
        .staging
        .as_deref()
        .or_else(|| owned_stage.as_ref().map(|s| s.path()))
        .context("Missing native staging directory")?;
    let mut throttle = crate::throttle::Throttle::new(ytui_core::schedule::rate_bytes(
        o.bandwidth_kib,
        o.global_bandwidth_kib,
        request.config.concurrency,
    ));
    for (entry, mut info) in infos.into_iter().enumerate() {
        println!(
            "YTUI_META:{}",
            json!({"id":info.id,"title":info.title,"webpage_url":info.webpage_url,"uploader":request.job.media.creator,"thumbnail":request.job.media.thumbnail})
        );
        let filename = yt_dlp_rs::output::render_template(&o.filename_template, &info)?;
        ensure!(
            !filename.is_absolute()
                && !filename
                    .components()
                    .any(|p| matches!(p, std::path::Component::ParentDir)),
            "Native output filename escaped its destination"
        );
        let destination = o.output_directory().join(filename);
        std::fs::create_dir_all(destination.parent().context("No output parent")?)?;
        let entry_stage = stage_root.join(entry.to_string());
        std::fs::create_dir_all(&entry_stage)?;
        let staged_media = entry_stage.join(destination.file_name().context("No output filename")?);
        let subtitles = prepare_subtitles(&client, &mut info, &staged_media, o, &mut throttle)?;
        for subtitle in &subtitles {
            let published = destination
                .with_file_name(subtitle.path.file_name().context("No subtitle filename")?);
            ensure!(
                !published.try_exists()?,
                "Subtitle destination already exists; choose another output directory"
            );
        }
        let processors = PostprocessOptions {
            embed: if o.embed_subtitles && !subtitles.is_empty() {
                Some(SubtitleEmbedding {
                    subtitles: subtitles.clone(),
                    keep_subtitles: true,
                })
            } else {
                None
            },
            audio: if o.audio {
                Some(AudioOptions::new(&o.audio_container, &o.audio_quality)?)
            } else {
                None
            },
            keep_video: false,
            video: if !o.audio {
                Some(VideoOptions::new(&o.container, VideoMode::Remux)?)
            } else {
                None
            },
            ..Default::default()
        };
        throttle.new_resource();
        let result = client.download_processed_with_progress(
            &info,
            &staged_media,
            DownloadOptions {
                retries: 0,
                fragment_retries: 0,
                ..Default::default()
            },
            &processors,
            |progress| {
                throttle.pace(progress);
                report(progress);
                ControlFlow::Continue(())
            },
        )?;
        ensure!(
            result.warnings.is_empty(),
            "Native postprocessing reported unsupported options; media was preserved; retry explicitly with yt-dlp"
        );
        let output = result.path;
        println!(
            "YTUI_SOURCE:{}",
            serde_json::to_string(&output.to_string_lossy())?
        );
        std::io::stdout().flush()?;
        ensure!(
            output.is_file(),
            "Native backend did not publish a final file"
        );
        if o.split_chapters || !o.sponsorblock.is_empty() {
            println!("YTUI_POST:{{\"postprocessor\":\"NativePost\"}}");
            std::io::stdout().flush()?;
        }
        let mut post = request.clone();
        post.job.media.id = info.id.clone();
        if o.split_chapters
            && post.job.media.chapters.is_empty()
            && ytui_core::http_url(&request.job.media.url)
                .is_ok_and(|u| ytui_core::youtube_host(u.host_str().unwrap_or_default()))
        {
            let session = request
                .cookies
                .as_ref()
                .and_then(|p| crate::auth::import_netscape(p).ok())
                .map(Arc::new);
            let browser = crate::browse::Browser::new(
                crate::http_client(request.config.proxy.as_deref())?,
                session,
                request.config.account_index,
            );
            if let Ok(page) = tokio::runtime::Handle::current()
                .block_on(browser.fetch(false, &ytui_core::BrowseRequest::Details(info.id.clone())))
                && let Some(media) = page.items.first()
            {
                post.job.media.description = media.description.clone();
                post.job.media.chapters = media.chapters.clone();
            }
        }
        let extra_outputs = crate::native_post::apply(&post, &output)
            .map_err(|e| anyhow::anyhow!("Native postprocessing failed: {e}"))?;
        if o.metadata || o.thumbnail {
            println!("YTUI_POST:{{\"postprocessor\":\"Metadata\"}}");
            tag(&request, &output, &info.title)?;
        }
        let mut paths = vec![output];
        paths.extend(extra_outputs);
        paths.extend(subtitles.into_iter().map(|s| s.path));
        let published = publish(&paths, destination.parent().context("No output parent")?)?;
        for path in published {
            println!(
                "YTUI_OUTPUT:{}",
                serde_json::to_string(&path.to_string_lossy())?
            );
        }
        std::io::stdout().flush()?;
    }
    Ok(())
}
fn publish(paths: &[PathBuf], directory: &Path) -> Result<Vec<PathBuf>> {
    let mut pairs = Vec::new();
    for source in paths {
        let target = directory.join(source.file_name().context("No output filename")?);
        ensure!(
            !target.try_exists()?,
            "Output already exists; existing files were preserved"
        );
        ensure!(
            !pairs.iter().any(|(_, p)| p == &target),
            "Output filenames collide"
        );
        pairs.push((source, target));
    }
    let mut created = Vec::new();
    for (source, target) in &pairs {
        if let Err(error) = std::fs::hard_link(source, target) {
            for (source, target) in created {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    if let (Ok(a), Ok(b)) =
                        (std::fs::metadata(source), std::fs::symlink_metadata(target))
                        && a.ino() == b.ino()
                        && a.dev() == b.dev()
                    {
                        let _ = std::fs::remove_file(target);
                    }
                }
            }
            return Err(error).context(
                "Cannot publish outputs; destination is unwritable or another file appeared",
            );
        }
        created.push((source, target));
    }
    Ok(pairs.into_iter().map(|(_, target)| target).collect())
}
fn prepare_subtitles(
    client: &YoutubeDl,
    info: &mut VideoInfo,
    destination: &Path,
    options: &ytui_core::DownloadOptions,
    throttle: &mut crate::throttle::Throttle,
) -> Result<Vec<SubtitleFile>> {
    if options.subtitles.is_empty() {
        return Ok(Vec::new());
    }
    let languages = options.subtitles.join(",");
    let selection = SubtitleOptions::new(Some(&languages), "vtt/best")?.select_sources(
        &info.subtitles,
        &info.automatic_captions,
        true,
        options.auto_subtitles,
    )?;
    ensure!(
        selection.warnings.is_empty() && !selection.requested.is_empty(),
        "Requested subtitles are unavailable in the native extractor; choose another language or explicit yt-dlp fallback"
    );
    info.requested_subtitles = Some(selection.requested);
    let paths = yt_dlp_rs::subtitle_paths(info, destination)?;
    let mut files = Vec::new();
    for (language, subtitle, path) in paths {
        // Never reuse an arbitrary pre-existing sidecar or overwrite the user's file.
        ensure!(
            !path.try_exists()?,
            "Subtitle destination already exists; choose another output directory"
        );
        throttle.new_resource();
        client.download_subtitle_with_progress(
            info,
            &subtitle,
            &path,
            DownloadOptions {
                retries: 0,
                fragment_retries: 0,
                ..Default::default()
            },
            |progress| {
                throttle.pace(progress);
                ControlFlow::Continue(())
            },
        )?;
        files.push(SubtitleFile {
            path,
            ext: subtitle.ext,
            language,
            name: subtitle.name,
        });
    }
    Ok(files)
}
fn report(progress: DownloadProgress) {
    match progress.phase {
        DownloadPhase::Merging => println!("YTUI_POST:{{\"postprocessor\":\"Merger\"}}"),
        DownloadPhase::Postprocessing => println!("YTUI_POST:{{\"postprocessor\":\"Convert\"}}"),
        _ => println!(
            "YTUI_PROGRESS:{}",
            json!({"downloaded_bytes":progress.downloaded_bytes,"total_bytes":progress.total_bytes})
        ),
    }
    let _ = std::io::stdout().flush();
}
fn tag(request: &WorkerRequest, path: &Path, title: &str) -> Result<()> {
    let o = &request.job.options;
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .context("Output has no extension")?;
    let dir = path.parent().context("Output has no directory")?;
    let temp = tempfile::Builder::new()
        .prefix(".ytui-metadata-")
        .suffix(&format!(".{extension}"))
        .tempfile_in(dir)?;
    let mut args = vec![
        "-nostdin".to_owned(),
        "-y".into(),
        "-i".into(),
        path.to_string_lossy().into(),
    ];
    let mut cover = None;
    if o.thumbnail && !request.job.media.thumbnail.is_empty() {
        ytui_core::http_url(&request.job.media.thumbnail)?;
        let mut builder =
            reqwest::blocking::Client::builder().timeout(std::time::Duration::from_secs(20));
        if let Some(proxy) = &request.config.proxy {
            builder = builder.proxy(reqwest::Proxy::all(proxy)?);
        }
        let mut response = builder
            .build()?
            .get(&request.job.media.thumbnail)
            .send()?
            .error_for_status()?;
        use std::io::Read;
        let mut bytes = Vec::new();
        Read::take(&mut response, 4 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= 4 * 1024 * 1024, "Cover image too large");
        let mut file = tempfile::NamedTempFile::new_in(dir)?;
        file.write_all(&bytes)?;
        args.extend(["-i".into(), file.path().to_string_lossy().into()]);
        cover = Some(file);
    }
    args.extend(["-map".into(), "0".into()]);
    if cover.is_some() {
        args.extend([
            "-map".into(),
            "1:v:0".into(),
            if o.audio {
                "-disposition:v:0".into()
            } else {
                "-disposition:v:1".into()
            },
            "attached_pic".into(),
        ]);
    }
    args.extend(["-c".into(), "copy".into()]);
    if o.metadata {
        args.extend([
            "-metadata".into(),
            format!("title={title}"),
            "-metadata".into(),
            format!("artist={}", request.job.media.creator),
        ]);
    }
    args.push(temp.path().to_string_lossy().into());
    let output = crate::worker_process::run(&request.config.ffmpeg, &args)?;
    ensure!(
        output.success,
        "Native metadata/cover embedding failed: {}",
        output.stderr
    );
    temp.persist(path).map_err(|e| e.error)?;
    Ok(())
}
pub async fn run(
    mut request: WorkerRequest,
    dir: &Path,
    cancel: CancellationToken,
    on_line: impl FnMut(&str) + Send,
) -> Result<ProcessResult> {
    if ytui_core::http_url(&request.job.media.url)
        .is_ok_and(|url| ytui_core::youtube_host(url.host_str().unwrap_or_default()))
    {
        request.config.native_js_runtime =
            crate::native_runtime::select(&request.config, cancel.clone()).await?;
    }
    crate::paths::private_dir(dir)?;
    // Parent ownership ensures ordinary cancellation cleans staging after the helper is reaped.
    let staging = if request.resolve {
        None
    } else {
        Some(
            tempfile::Builder::new()
                .prefix(".ytui-job-")
                .tempdir_in(request.job.options.output_directory())?,
        )
    };
    request.staging = staging.as_ref().map(|d| d.path().to_owned());
    let mut manifest = tempfile::Builder::new()
        .prefix("native-")
        .suffix(".json")
        .tempfile_in(dir)?;
    manifest.write_all(&serde_json::to_vec(&request)?)?;
    manifest.flush()?;
    // Linux retains the running inode through /proc even after an atomic upgrade
    // unlinks its old pathname. current_exe() then includes " (deleted)".
    #[cfg(target_os = "linux")]
    let exe = PathBuf::from("/proc/self/exe");
    #[cfg(not(target_os = "linux"))]
    let exe = std::env::current_exe()?;
    let result = process::run(
        ProcessSpec::new(
            exe.to_string_lossy(),
            vec![
                "__native-worker".into(),
                manifest.path().to_string_lossy().into(),
            ],
            if request.resolve { 75 } else { 24 * 60 * 60 },
        ),
        cancel.clone(),
        on_line,
    )
    .await;
    if let Ok(mut output) = result {
        if !output.success
            && !cancel.is_cancelled()
            && output
                .lines
                .iter()
                .any(|line| line.starts_with("YTUI_SOURCE:"))
            && let Some(staging) = staging
        {
            let path = staging.keep();
            output
                .stderr
                .push_str(&format!(" · Recoverable source kept in {}", path.display()));
        }
        Ok(output)
    } else {
        result
    }
}
pub async fn resolve(
    job: Job,
    config: Config,
    cookies: Option<PathBuf>,
    dir: &Path,
    cancel: CancellationToken,
) -> Result<Resolved> {
    let result = run(
        WorkerRequest {
            job,
            config,
            cookies,
            resolve: true,
            staging: None,
        },
        dir,
        cancel,
        |_| {},
    )
    .await?;
    let reason = result
        .stderr
        .lines()
        .find(|line| {
            let line = line.to_lowercase();
            [
                "not a bot",
                "sign in",
                "video is unavailable",
                "private video",
            ]
            .iter()
            .any(|part| line.contains(part))
        })
        .unwrap_or(&result.stderr);
    let diagnostic = process::diagnostic(reason);
    let guidance = crate::failure::recovery(&diagnostic)
        .map(|recovery| recovery.instruction)
        .unwrap_or("Check the installed Node.js 22+ or Deno 2.9.7 runtime in Settings → Tools, or open Playback repair.");
    ensure!(
        result.success,
        "Native playback extraction failed: {}. {} No external extractor was used in native mode.",
        diagnostic,
        guidance
    );
    for line in result.lines {
        if let Some(json) = line.strip_prefix("YTUI_RESOLVED:") {
            return Ok(serde_json::from_str(json)?);
        }
    }
    bail!("Native backend did not return a playback stream")
}

/// Auto mode falls back for extractor limitations, without changing the explicit
/// native mode or retrying cancellations/local publication failures externally.
pub fn fallback_eligible(message: &str) -> bool {
    let message = message.to_lowercase();
    [
        "unsupported youtube",
        "native youtube playlists require yt-dlp fallback",
        "http status client error (403",
        "http status client error (404",
        "http 403",
        "http 404",
        "http error 403",
        "http error 404",
        "native youtube solving requires",
        "configured native javascript runtime was not found",
        "no eligible media",
        "signature",
        "url challenges",
        "native extractor returned no media",
        "native backend did not return",
        "native preview of fragmented",
        "needs explicit yt-dlp fallback",
        "native extractor did not supply",
    ]
    .iter()
    .any(|s| message.contains(s))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn info(extra: serde_json::Value) -> VideoInfo {
        let mut value = json!({"id":"fixture","title":"Fixture","ext":"mp4","url":"https://media.test/video.mp4","webpage_url":"https://www.youtube.com/watch?v=abcdefghijk","extractor":"youtube","extractor_key":"Youtube"});
        value
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        serde_json::from_value(value).unwrap()
    }
    #[test]
    fn reported_format_ids_select_the_actual_video_and_audio_language() {
        let mut video = info(json!({"formats":[
            {"format_id":"137","url":"https://media.test/1080.mp4","ext":"mp4","height":1080,"vcodec":"avc1","acodec":"none","stream_kind":"video_only"},
            {"format_id":"247","url":"https://media.test/720.webm","ext":"webm","height":720,"vcodec":"vp9","acodec":"none","stream_kind":"video_only"},
            {"format_id":"140-de","url":"https://media.test/de.m4a","ext":"m4a","vcodec":"none","acodec":"aac","language":"de","stream_kind":"audio_only"}
        ]}));
        video.select_format(Some("247+140-de/best")).unwrap();
        let source = playback_source(&video, false).unwrap();
        assert_eq!(source.url, "https://media.test/720.webm");
        assert_eq!(
            source.audio_url.as_deref(),
            Some("https://media.test/de.m4a")
        );
        assert_eq!(source.catalog.formats.len(), 3);
    }
    #[test]
    fn native_hls_keeps_the_manifest_url_and_extractor_request_headers() {
        let video = info(
            json!({"url":"https://media.test/video.m3u8","protocol":"m3u8_native","fragments":[{"url":"https://media.test/segment.ts"}],"http_headers":{"User-Agent":"native-client","Referer":"https://www.youtube.com/","Authorization":"must-not-leak"}}),
        );
        let source = playback_source(&video, false).unwrap();
        assert_eq!(source.url, "https://media.test/video.m3u8");
        assert!(source.audio_url.is_none());
        assert!(
            source
                .headers
                .contains(&("User-Agent".into(), "native-client".into()))
        );
        assert!(!source.headers.iter().any(|(k, _)| k == "Authorization"));
    }
    #[test]
    fn native_pair_roles_are_independent_of_order_and_audio_only_is_not_video() {
        let video = info(json!({"requested_formats":[
            {"format_id":"audio","url":"https://media.test/audio.m4a","ext":"m4a","stream_kind":"audio_only"},
            {"format_id":"video","url":"https://media.test/video.mp4","ext":"mp4","stream_kind":"video_only","http_headers":{"User-Agent":"native-video"}}
        ]}));
        let source = playback_source(&video, false).unwrap();
        assert_eq!(source.url, "https://media.test/video.mp4");
        assert_eq!(
            source.audio_url.as_deref(),
            Some("https://media.test/audio.m4a")
        );
        assert!(
            source
                .headers
                .contains(&("User-Agent".into(), "native-video".into()))
        );
        let source = playback_source(&video, true).unwrap();
        assert_eq!(source.url, "https://media.test/audio.m4a");
        assert!(source.audio_url.is_none());
    }
    #[test]
    fn native_subtitles_download_and_refuse_to_overwrite() {
        use std::{
            io::{Read, Write},
            net::TcpListener,
            thread,
            time::{Duration, Instant},
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/captions.vtt", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let started = Instant::now();
            while started.elapsed() < Duration::from_secs(3) {
                if let Ok((mut stream, _)) = listener.accept() {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(1)))
                        .unwrap();
                    let mut input = [0u8; 4096];
                    let _ = stream.read(&mut input);
                    let data = "WEBVTT\n\n00:00.000 --> 00:01.000\nSynthetic captions\n";
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{data}",
                        data.len()
                    )
                    .unwrap();
                    return;
                }
                thread::sleep(Duration::from_millis(5));
            }
            panic!("No subtitle request");
        });
        let mut info: VideoInfo = serde_json::from_value(json!({"id":"fixture","title":"Fixture","ext":"mp4","url":url,"webpage_url":url,"extractor":"generic","extractor_key":"Generic","direct":true,"subtitles":{"en":[{"url":url,"ext":"vtt"}]}})).unwrap();
        let client = YoutubeDl::with_options(
            YoutubeOptions::default(),
            HttpOptions {
                proxy: Some(String::new()),
                ..Default::default()
            },
        )
        .unwrap();
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("fixture.mp4");
        let options = ytui_core::DownloadOptions {
            subtitles: vec!["en".into()],
            ..Default::default()
        };
        let files = prepare_subtitles(
            &client,
            &mut info,
            &destination,
            &options,
            &mut crate::throttle::Throttle::new(0),
        )
        .unwrap();
        server.join().unwrap();
        assert_eq!(files.len(), 1);
        assert!(
            std::fs::read_to_string(&files[0].path)
                .unwrap()
                .contains("Synthetic captions")
        );
        assert!(
            prepare_subtitles(
                &client,
                &mut info,
                &destination,
                &options,
                &mut crate::throttle::Throttle::new(0)
            )
            .is_err()
        );
        let missing = ytui_core::DownloadOptions {
            subtitles: vec!["zz".into()],
            ..Default::default()
        };
        assert!(
            prepare_subtitles(
                &client,
                &mut info,
                &destination,
                &missing,
                &mut crate::throttle::Throttle::new(0)
            )
            .is_err()
        );
    }
    #[test]
    fn multi_output_publish_refuses_collisions_before_creating_any_files() {
        let temp = tempfile::tempdir().unwrap();
        let stage = temp.path().join("stage");
        let out = temp.path().join("out");
        std::fs::create_dir_all(&stage).unwrap();
        std::fs::create_dir_all(&out).unwrap();
        let a = stage.join("video.mp4");
        let b = stage.join("chapter.mp4");
        std::fs::write(&a, b"video").unwrap();
        std::fs::write(&b, b"chapter").unwrap();
        std::fs::write(out.join("chapter.mp4"), b"unrelated").unwrap();
        assert!(publish(&[a.clone(), b.clone()], &out).is_err());
        assert!(!out.join("video.mp4").exists());
        assert_eq!(
            std::fs::read(out.join("chapter.mp4")).unwrap(),
            b"unrelated"
        );
        std::fs::remove_file(out.join("chapter.mp4")).unwrap();
        assert_eq!(publish(&[a, b], &out).unwrap().len(), 2);
    }
}

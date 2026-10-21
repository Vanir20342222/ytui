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

pub const UPSTREAM_REV: &str = "10223c019ef6ec5552627b90251d8e967404e440";
#[derive(Serialize, Deserialize)]
pub struct WorkerRequest {
    pub job: Job,
    pub config: Config,
    pub cookies: Option<PathBuf>,
    pub resolve: bool,
}
#[derive(Serialize, Deserialize)]
pub struct Resolved {
    pub url: String,
    pub audio_url: Option<String>,
}
pub fn worker(path: &Path) -> Result<()> {
    let request: WorkerRequest = serde_json::from_slice(&std::fs::read(path)?)?;
    let o = &request.job.options;
    // Do not pretend that upstream supports these features or silently discard options.
    if !request.resolve {
        ensure!(
            !o.thumbnail || !request.job.media.thumbnail.is_empty(),
            "Native extractor did not supply cover art; choose a browse item with a thumbnail or retry explicitly with yt-dlp"
        );
        ensure!(
            o.sponsorblock.is_empty(),
            "Native backend does not support SponsorBlock; use an explicit yt-dlp retry"
        );
        ensure!(
            !o.split_chapters,
            "Native backend does not support chapter splitting; use an explicit yt-dlp retry"
        );
        ensure!(
            o.bandwidth_kib == 0 && o.global_bandwidth_kib == 0,
            "Native bandwidth limiting is not supported yet; use an explicit yt-dlp retry"
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
        },
        HttpOptions {
            cookies,
            proxy: request.config.proxy.clone(),
            ..Default::default()
        },
    )?
    .with_ffmpeg(
        crate::paths::executable(&request.config.ffmpeg)
            .unwrap_or_else(|| PathBuf::from(&request.config.ffmpeg)),
    );
    let selector = if o.audio {
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
        ensure!(
            info.fragments.is_empty()
                && info
                    .requested_formats
                    .iter()
                    .all(|format| format.fragments.is_empty()),
            "Native preview of fragmented media is not supported; choose the yt-dlp playback backend explicitly"
        );
        let (url, audio_url) = if info.requested_formats.len() == 2 {
            (
                info.requested_formats[0].url.to_string(),
                Some(info.requested_formats[1].url.to_string()),
            )
        } else {
            (info.url.to_string(), None)
        };
        println!(
            "YTUI_RESOLVED:{}",
            serde_json::to_string(&Resolved { url, audio_url })?
        );
        std::io::stdout().flush()?;
        return Ok(());
    }
    for mut info in infos {
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
        let subtitle_stage = tempfile::Builder::new()
            .prefix(".ytui-subtitles-")
            .tempdir_in(destination.parent().context("No output parent")?)?;
        let staged_media = subtitle_stage
            .path()
            .join(destination.file_name().context("No output filename")?);
        let subtitles = prepare_subtitles(&client, &mut info, &staged_media, o)?;
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
        };
        let result = client.download_processed_with_progress(
            &info,
            &destination,
            DownloadOptions { retries: 0 },
            &processors,
            |progress| {
                report(progress);
                ControlFlow::Continue(())
            },
        )?;
        ensure!(
            result.warnings.is_empty(),
            "Native postprocessing reported unsupported options; media was preserved; retry explicitly with yt-dlp"
        );
        let output = result.path;
        ensure!(
            output.is_file(),
            "Native backend did not publish a final file"
        );
        if o.metadata || o.thumbnail {
            println!("YTUI_POST:{{\"postprocessor\":\"Metadata\"}}");
            tag(&request, &output, &info.title)?;
        }
        println!(
            "YTUI_OUTPUT:{}",
            serde_json::to_string(&output.to_string_lossy())?
        );
        for subtitle in subtitles {
            let published = destination
                .with_file_name(subtitle.path.file_name().context("No subtitle filename")?);
            std::fs::hard_link(&subtitle.path, &published)
                .context("Cannot publish subtitle; downloaded media was preserved")?;
            println!(
                "YTUI_OUTPUT:{}",
                serde_json::to_string(&published.to_string_lossy())?
            );
        }
        std::io::stdout().flush()?;
    }
    Ok(())
}
fn prepare_subtitles(
    client: &YoutubeDl,
    info: &mut VideoInfo,
    destination: &Path,
    options: &ytui_core::DownloadOptions,
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
        client.download_subtitle_with_options(
            info,
            &subtitle,
            &path,
            DownloadOptions { retries: 0 },
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
    let output = std::process::Command::new(&request.config.ffmpeg)
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()?;
    ensure!(
        output.status.success(),
        "Native metadata/cover embedding failed; the downloaded media was preserved"
    );
    temp.persist(path).map_err(|e| e.error)?;
    Ok(())
}
pub async fn run(
    request: WorkerRequest,
    dir: &Path,
    cancel: CancellationToken,
    on_line: impl FnMut(&str) + Send,
) -> Result<ProcessResult> {
    crate::paths::private_dir(dir)?;
    let mut manifest = tempfile::Builder::new()
        .prefix("native-")
        .suffix(".json")
        .tempfile_in(dir)?;
    manifest.write_all(&serde_json::to_vec(&request)?)?;
    manifest.flush()?;
    let exe = std::env::current_exe()?;
    process::run(
        ProcessSpec::new(
            exe.to_string_lossy(),
            vec![
                "__native-worker".into(),
                manifest.path().to_string_lossy().into(),
            ],
            if request.resolve { 40 } else { 24 * 60 * 60 },
        ),
        cancel,
        on_line,
    )
    .await
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
        },
        dir,
        cancel,
        |_| {},
    )
    .await?;
    ensure!(
        result.success,
        "Native playback extraction failed: {}. Select yt-dlp explicitly in settings to use the fallback.",
        process::diagnostic(&result.stderr)
    );
    for line in result.lines {
        if let Some(json) = line.strip_prefix("YTUI_RESOLVED:") {
            return Ok(serde_json::from_str(json)?);
        }
    }
    bail!("Native backend did not return a playback stream")
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let files = prepare_subtitles(&client, &mut info, &destination, &options).unwrap();
        server.join().unwrap();
        assert_eq!(files.len(), 1);
        assert!(
            std::fs::read_to_string(&files[0].path)
                .unwrap()
                .contains("Synthetic captions")
        );
        assert!(prepare_subtitles(&client, &mut info, &destination, &options).is_err());
        let missing = ytui_core::DownloadOptions {
            subtitles: vec!["zz".into()],
            ..Default::default()
        };
        assert!(prepare_subtitles(&client, &mut info, &destination, &missing).is_err());
    }
}

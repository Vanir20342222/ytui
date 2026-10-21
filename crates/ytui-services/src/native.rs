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
    PlaylistSelection, PostprocessOptions, VideoMode, VideoOptions, YoutubeDl, YoutubeOptions,
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
        ensure!(
            o.subtitles.is_empty(),
            "Subtitle options currently require an explicit yt-dlp retry in ytui"
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
            ..Default::default()
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
            info.fragments.is_empty(),
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
    for info in infos {
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
        let processors = PostprocessOptions {
            audio: if o.audio {
                Some(AudioOptions::new(&o.audio_container, &o.audio_quality)?)
            } else {
                None
            },
            video: if !o.audio {
                Some(VideoOptions::new(&o.container, VideoMode::Remux)?)
            } else {
                None
            },
            ..Default::default()
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
        std::io::stdout().flush()?;
    }
    Ok(())
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

use crate::{
    auth::Session,
    paths::Paths,
    process::{self, ProcessSpec},
};
use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;
use std::{path::Path, sync::Arc, time::Duration};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use ytui_core::*;

pub fn args(job: &Job, config: &Config, cookie_file: Option<&Path>) -> Result<Vec<String>> {
    let o = &job.options;
    o.validate()?;
    http_url(&job.media.url)?;
    let mut args = vec![
        "--ignore-config".into(),
        "--no-colors".into(),
        "--newline".into(),
        "--progress".into(),
        "--progress-delta".into(),
        "0.4".into(),
        "--continue".into(),
        "--retries".into(),
        "0".into(),
        "--fragment-retries".into(),
        "0".into(),
        "--socket-timeout".into(),
        "20".into(),
        "--no-simulate".into(),
        "--progress-template".into(),
        "download:YTUI_PROGRESS:%(progress)j".into(),
        "--progress-template".into(),
        "postprocess:YTUI_POST:%(progress)j".into(),
        "--print".into(),
        "before_dl:YTUI_META:%(.{id,title,uploader,duration,thumbnail,webpage_url,description})j"
            .into(),
        "--print".into(),
        "after_move:YTUI_OUTPUT:%(filepath)j".into(),
        "--ffmpeg-location".into(),
        crate::paths::executable(&config.ffmpeg)
            .unwrap_or_else(|| config.ffmpeg.clone().into())
            .to_string_lossy()
            .into(),
        "--paths".into(),
        o.output_directory().to_string_lossy().into(),
        "--output".into(),
        o.filename_template.clone(),
    ];
    let mut add = |key: &str, value: String| {
        args.push(key.into());
        args.push(value);
    };
    if o.audio {
        add("--format", "bestaudio/best".into());
        args.push("--extract-audio".into());
        args.extend([
            "--audio-format".into(),
            o.audio_container.clone(),
            "--audio-quality".into(),
            o.audio_quality.clone(),
        ]);
    } else {
        let format = if o.quality == "best" {
            "bv*+ba/b".into()
        } else {
            format!("bv*[height<=?{}]+ba/b[height<=?{}]", o.quality, o.quality)
        };
        add("--format", format);
        add("--merge-output-format", o.container.clone());
        add("--remux-video", o.container.clone());
    }
    if o.playlist {
        args.push("--yes-playlist".into());
    } else {
        args.push("--no-playlist".into());
    }
    if let Some(items) = &o.playlist_items {
        args.extend(["--playlist-items".into(), items.clone()]);
    }
    if let Some(proxy) = &config.proxy {
        args.extend(["--proxy".into(), proxy.clone()]);
    }
    if let Some(cookies) = cookie_file {
        args.extend(["--cookies".into(), cookies.to_string_lossy().into()]);
    }
    if !o.subtitles.is_empty() {
        args.extend([
            "--write-subs".into(),
            "--sub-langs".into(),
            o.subtitles.join(","),
        ]);
        if o.auto_subtitles {
            args.push("--write-auto-subs".into());
        }
        if o.embed_subtitles {
            args.push("--embed-subs".into());
        }
    }
    if o.metadata {
        args.push("--embed-metadata".into());
    }
    if o.thumbnail {
        args.push("--embed-thumbnail".into());
    }
    if o.split_chapters {
        args.push("--split-chapters".into());
    }
    if !o.sponsorblock.is_empty() {
        args.extend(["--sponsorblock-api".into(), o.sponsorblock_api.clone()]);
        args.extend([
            if o.sponsorblock_remove {
                "--sponsorblock-remove"
            } else {
                "--sponsorblock-mark"
            }
            .into(),
            o.sponsorblock.join(","),
        ]);
    }
    let rate = schedule::rate_bytes(o.bandwidth_kib, o.global_bandwidth_kib, config.concurrency);
    if rate > 0 {
        args.extend(["--limit-rate".into(), rate.to_string()]);
    }
    if o.skip_duplicates {
        args.push("--no-overwrites".into());
    }
    args.push("--".into());
    args.push(job.media.url.clone());
    Ok(args)
}
pub fn metadata(value: &Value, url: &str) -> Media {
    Media {
        id: value
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .into(),
        title: clean_text(value.get("title").and_then(Value::as_str).unwrap_or(url)),
        creator: clean_text(
            value
                .get("uploader")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        ),
        url: value
            .get("webpage_url")
            .and_then(Value::as_str)
            .unwrap_or(url)
            .into(),
        thumbnail: value
            .get("thumbnail")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .into(),
        duration: value.get("duration").and_then(Value::as_u64),
        description: clean_text(
            value
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        ),
        ..Default::default()
    }
}
pub fn parse_progress(line: &str) -> Option<JobUpdate> {
    if let Some(raw) = line.strip_prefix("YTUI_PROGRESS:") {
        let data: Value = serde_json::from_str(raw).ok()?;
        Some(JobUpdate::Progress(Progress {
            downloaded: data
                .get("downloaded_bytes")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            total: data
                .get("total_bytes")
                .or_else(|| data.get("total_bytes_estimate"))
                .and_then(Value::as_u64),
            speed: data.get("speed").and_then(Value::as_f64),
            eta: data.get("eta").and_then(Value::as_f64),
        }))
    } else if let Some(raw) = line.strip_prefix("YTUI_POST:") {
        let data: Value = serde_json::from_str(raw).ok()?;
        let processor = data
            .get("postprocessor")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_lowercase();
        Some(JobUpdate::Phase(if processor.contains("merger") {
            JobState::Merging
        } else {
            JobState::Converting
        }))
    } else {
        None
    }
}
pub fn transient(error: &str) -> bool {
    let e = error.to_ascii_lowercase();
    [
        "timed out",
        "timeout",
        "connection",
        "http error 5",
        "http error 429",
        "temporary failure",
        "network is unreachable",
        "error sending request",
        "http 5",
        "server error (5",
        "http 429",
    ]
    .iter()
    .any(|s| e.contains(s))
        && ![
            "private video",
            "video unavailable",
            "video is unavailable",
            "content is unavailable",
            "no space left",
            "permission denied",
            "sign in",
            "cookies",
            "requested format",
            "postprocessing failed",
        ]
        .iter()
        .any(|s| e.contains(s))
}
async fn send(tx: &mpsc::Sender<JobEvent>, job: &Job, update: JobUpdate) {
    let _ = tx
        .send(JobEvent {
            id: job.id,
            generation: job.generation,
            update,
        })
        .await;
}
pub async fn execute(
    job: Job,
    config: Config,
    paths: Paths,
    session: Option<Arc<Session>>,
    cancel: CancellationToken,
    tx: mpsc::Sender<JobEvent>,
) {
    let outcome = execute_inner(&job, &config, &paths, session, cancel.clone(), &tx).await;
    if cancel.is_cancelled() {
        return;
    }
    match outcome {
        Ok(()) => send(&tx, &job, JobUpdate::Finished).await,
        Err(e) => {
            send(
                &tx,
                &job,
                JobUpdate::Failed {
                    message: format!(
                        "{}{}",
                        process::diagnostic(&e.to_string()),
                        if job
                            .options
                            .backend
                            .as_deref()
                            .unwrap_or(&config.download_backend)
                            == "native"
                        {
                            " · Y retries explicitly with yt-dlp"
                        } else {
                            ""
                        }
                    ),
                    transient: false,
                },
            )
            .await
        }
    }
}
async fn execute_inner(
    job: &Job,
    config: &Config,
    paths: &Paths,
    session: Option<Arc<Session>>,
    cancel: CancellationToken,
    tx: &mpsc::Sender<JobEvent>,
) -> Result<()> {
    std::fs::create_dir_all(job.options.output_directory())
        .context("Cannot create download destination")?;
    if Path::new(&job.media.url).is_file() {
        return convert_local(job, config, cancel, tx).await;
    }
    let cookies = session
        .map(|s| s.export(&paths.state.join("auth/transient")))
        .transpose()?;
    let backend = job
        .options
        .backend
        .as_deref()
        .unwrap_or(&config.download_backend);
    let args = if backend == "yt-dlp" {
        args(job, config, cookies.as_ref().map(|f| f.path()))?
    } else {
        Vec::new()
    };
    for attempt in 0..=config.max_retries {
        if cancel.is_cancelled() {
            bail!("Operation cancelled");
        }
        send(tx, job, JobUpdate::Phase(JobState::Resolving)).await;
        let sender = tx.clone();
        let id = job.id;
        let generation = job.generation;
        let progress = move |line: &str| {
            if let Some(update) = parse_progress(line) {
                let _ = sender.try_send(JobEvent {
                    id,
                    generation,
                    update,
                });
            }
            if let Some(raw) = line.strip_prefix("YTUI_META:")
                && let Ok(value) = serde_json::from_str::<Value>(raw)
            {
                let update = JobUpdate::Metadata(metadata(&value, ""));
                let _ = sender.try_send(JobEvent {
                    id,
                    generation,
                    update,
                });
            }
        };
        let result = if backend == "native" {
            crate::native::run(
                crate::native::WorkerRequest {
                    job: job.clone(),
                    config: config.clone(),
                    cookies: cookies.as_ref().map(|f| f.path().to_owned()),
                    resolve: false,
                    staging: None,
                },
                &paths.state.join("auth/transient"),
                cancel.clone(),
                progress,
            )
            .await
        } else {
            process::run(
                ProcessSpec::new(&config.yt_dlp, args.clone(), 24 * 60 * 60),
                cancel.clone(),
                progress,
            )
            .await
        };
        let error = match result {
            Ok(result) if result.success => {
                let mut outputs = Vec::new();
                let mut resolved_media = job.media.clone();
                for line in &result.lines {
                    if let Some(json) = line.strip_prefix("YTUI_META:")
                        && let Ok(value) = serde_json::from_str::<Value>(json)
                    {
                        resolved_media = metadata(&value, &job.media.url);
                        send(tx, job, JobUpdate::Metadata(resolved_media.clone())).await;
                    }
                    if let Some(json) = line.strip_prefix("YTUI_OUTPUT:") {
                        let path: std::path::PathBuf = serde_json::from_str::<String>(json)
                            .context("Invalid final output path from yt-dlp")?
                            .into();
                        ensure!(
                            path.is_file(),
                            "yt-dlp exited successfully but the final output file is missing"
                        );
                        outputs.push(path);
                    }
                }
                ensure!(
                    !outputs.is_empty(),
                    "yt-dlp did not report a final output file; check the URL, playlist selection, and installed yt-dlp version"
                );
                for output in &outputs {
                    send(tx, job, JobUpdate::Output(output.clone())).await;
                    run_hook(job, &resolved_media, output, cancel.clone()).await?;
                }
                return Ok(());
            }
            Ok(result) => {
                if result.stderr.is_empty() {
                    "yt-dlp exited without completing the download".into()
                } else {
                    result.stderr
                }
            }
            Err(e) => e.to_string(),
        };
        if cancel.is_cancelled() {
            bail!("Operation cancelled");
        }
        if attempt == config.max_retries || !transient(&error) {
            bail!("{}", process::diagnostic(&error));
        }
        send(tx, job, JobUpdate::Retrying(attempt + 1)).await;
        let delay = config
            .retry_delay_seconds
            .saturating_mul(1u64 << attempt.min(6))
            .min(300);
        tokio::select! {_=cancel.cancelled()=>bail!("Operation cancelled"),_=tokio::time::sleep(Duration::from_secs(delay))=>{}}
    }
    bail!("Download retry budget exhausted")
}

async fn run_hook(
    job: &Job,
    media: &Media,
    output: &Path,
    cancel: CancellationToken,
) -> Result<()> {
    if let Some(hook) = &job.options.post_download_hook {
        let mut spec = ProcessSpec::new(
            hook.to_string_lossy(),
            vec![output.to_string_lossy().into()],
            120,
        );
        spec.env = vec![
            ("YTUI_OUTPUT_PATH".into(), output.to_string_lossy().into()),
            ("YTUI_ITEM_ID".into(), job.id.to_string()),
            ("YTUI_VIDEO_ID".into(), media.id.clone()),
            ("YTUI_TITLE".into(), media.title.clone()),
            ("YTUI_URL".into(), job.media.url.clone()),
            (
                "YTUI_DOWNLOAD_MODE".into(),
                if job.options.audio { "audio" } else { "video" }.into(),
            ),
        ];
        let result = process::run(spec, cancel, |_| {}).await?;
        ensure!(
            result.success,
            "Post-download hook failed; downloaded media was preserved"
        );
    }
    Ok(())
}

async fn convert_local(
    job: &Job,
    config: &Config,
    cancel: CancellationToken,
    tx: &mpsc::Sender<JobEvent>,
) -> Result<()> {
    let input = Path::new(&job.media.url);
    let stem = input
        .file_stem()
        .context("Input has no filename")?
        .to_string_lossy();
    let extension = if job.options.audio {
        job.options.audio_container.as_str()
    } else {
        job.options.container.as_str()
    };
    let target = job
        .options
        .output_directory()
        .join(format!("{stem}_converted.{extension}"));
    ensure!(
        !target.exists(),
        "Conversion destination already exists; choose another directory or filename"
    );
    let partial = job
        .options
        .output_directory()
        .join(format!(".{stem}_{}.part.{extension}", job.id));
    let mut args = vec!["-nostdin".into()];
    if config.offline {
        args.extend(["-protocol_whitelist".into(), "file,pipe".into()]);
    }
    args.extend(["-y".into(), "-i".into(), input.to_string_lossy().into()]);
    if job.options.audio {
        args.push("-vn".into());
    }
    args.push(partial.to_string_lossy().into());
    send(tx, job, JobUpdate::Phase(JobState::Converting)).await;
    let result = process::run(
        ProcessSpec::new(&config.ffmpeg, args, 24 * 60 * 60),
        cancel.clone(),
        |_| {},
    )
    .await?;
    ensure!(
        result.success,
        "FFmpeg conversion failed: {}",
        process::diagnostic(&result.stderr)
    );
    ensure!(partial.is_file(), "FFmpeg did not produce an output file");
    // hard_link atomically refuses to overwrite an unrelated file created concurrently.
    std::fs::hard_link(&partial, &target)?;
    std::fs::remove_file(partial)?;
    send(tx, job, JobUpdate::Output(target.clone())).await;
    run_hook(job, &job.media, &target, cancel).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn never_uses_shell_or_unbounded_retry_settings() {
        let job = Job::new(
            Media {
                url: "https://youtube.com/watch?v=abcdefghijk".into(),
                ..Default::default()
            },
            DownloadOptions::default(),
        );
        let a = args(&job, &Config::default(), None).unwrap();
        assert!(a.contains(&"--ignore-config".into()));
        assert!(a.windows(2).any(|w| w == ["--retries", "0"]));
        assert!(a.iter().any(|v| v.starts_with("after_move:")));
        let mut job = job;
        job.options.global_bandwidth_kib = 1;
        let c = Config {
            concurrency: 16,
            ..Default::default()
        };
        let a = args(&job, &c, None).unwrap();
        assert!(a.windows(2).any(|w| w == ["--limit-rate", "64"]));
    }
    #[test]
    fn progress_handles_unknown_totals_and_permanent_errors() {
        let JobUpdate::Progress(p) = parse_progress(
            "YTUI_PROGRESS:{\"downloaded_bytes\":42,\"total_bytes\":null,\"eta\":null}",
        )
        .unwrap() else {
            panic!()
        };
        assert_eq!(p.fraction(), None);
        assert!(!transient("Private video; connection closed"));
        assert!(transient("Connection timed out"));
        assert!(transient("HTTP 503 Service Unavailable"));
        assert!(!transient(
            "Native postprocessing failed: connection closed"
        ));
    }
}

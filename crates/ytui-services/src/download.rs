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
        add(
            "--format",
            o.format_selector
                .clone()
                .unwrap_or_else(|| "bestaudio/best".into()),
        );
        args.push("--extract-audio".into());
        args.extend([
            "--audio-format".into(),
            o.audio_container.clone(),
            "--audio-quality".into(),
            o.audio_quality.clone(),
        ]);
    } else {
        let format = if let Some(selector) = &o.format_selector {
            selector.clone()
        } else if o.quality == "best" {
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
    ([
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
        || crate::failure::rate_limited(error))
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
            "tunnel connection failed: 403",
            "connect tunnel failed, response 403",
            "proxy authentication required",
            "read-only file system",
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
    let args = if backend != "native" {
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
        let mut progress = move |line: &str| {
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
                let update = JobUpdate::Metadata(Box::new(metadata(&value, "")));
                let _ = sender.try_send(JobEvent {
                    id,
                    generation,
                    update,
                });
            }
        };
        let mut result = if backend != "yt-dlp" {
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
                &mut progress,
            )
            .await
        } else {
            process::run(
                ProcessSpec::new(&config.yt_dlp, args.clone(), 24 * 60 * 60),
                cancel.clone(),
                &mut progress,
            )
            .await
        };
        if backend == "auto" && !cancel.is_cancelled() {
            let limitation = match &result {
                Ok(result) if !result.success => crate::native::fallback_eligible(&result.stderr),
                Err(error) => crate::native::fallback_eligible(&error.to_string()),
                _ => false,
            };
            if limitation {
                ensure!(
                    crate::paths::executable(&config.yt_dlp).is_some(),
                    "Native extraction requires fallback. Install yt-dlp and its supported JavaScript runtime; yt-dlp was not found. F2 → Playback repair configures the executable."
                );
                result = process::run(
                    ProcessSpec::new(&config.yt_dlp, args.clone(), 24 * 60 * 60),
                    cancel.clone(),
                    &mut progress,
                )
                .await;
            }
        }
        // Publication is independent from the tool's eventual exit status or a
        // later malformed output record. Keep every real file before validating
        // the complete response or running fallible post-download hooks.
        if let Ok(output) = &result {
            let mut registered = std::collections::HashSet::new();
            for line in &output.lines {
                if let Some(raw) = line.strip_prefix("YTUI_OUTPUT:")
                    && let Ok(path) = serde_json::from_str::<std::path::PathBuf>(raw)
                    && valid_output(&path)
                    && registered.insert(path.clone())
                {
                    send(tx, job, JobUpdate::Output(path)).await;
                }
            }
        }
        let error = match result {
            Ok(result) if result.success => {
                let mut outputs = Vec::new();
                let mut resolved_media = job.media.clone();
                for line in &result.lines {
                    if let Some(json) = line.strip_prefix("YTUI_META:")
                        && let Ok(value) = serde_json::from_str::<Value>(json)
                    {
                        resolved_media = metadata(&value, &job.media.url);
                        send(
                            tx,
                            job,
                            JobUpdate::Metadata(Box::new(resolved_media.clone())),
                        )
                        .await;
                    }
                    if let Some(json) = line.strip_prefix("YTUI_OUTPUT:") {
                        let path: std::path::PathBuf = serde_json::from_str::<String>(json)
                            .context("Invalid final output path from yt-dlp")?
                            .into();
                        ensure!(
                            valid_output(&path),
                            "yt-dlp exited successfully but the final output file is missing or empty"
                        );
                        if !outputs.iter().any(|(existing, _)| existing == &path) {
                            outputs.push((path, resolved_media.clone()));
                        }
                    }
                }
                ensure!(
                    !outputs.is_empty(),
                    "yt-dlp did not report a final output file; check the URL, playlist selection, and installed yt-dlp version"
                );
                // Valid files are already registered. Each unique output keeps
                // its own entry metadata and invokes its hook only once.
                for (output, media) in &outputs {
                    run_hook(job, media, output, cancel.clone()).await?;
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
        let delay = retry_delay(config.retry_delay_seconds, attempt, &error);
        tokio::select! {_=cancel.cancelled()=>bail!("Operation cancelled"),_=tokio::time::sleep(Duration::from_secs(delay))=>{}}
    }
    bail!("Download retry budget exhausted")
}

fn retry_delay(base: u64, attempt: u32, error: &str) -> u64 {
    let backoff = base.saturating_mul(1u64 << attempt.min(6)).min(300);
    // CLI/worker errors do not expose an HTTP Retry-After header. Use a
    // conservative minimum, while keeping the wait cancellable and bounded.
    if crate::failure::rate_limited(error) {
        backoff.max(60)
    } else {
        backoff
    }
}

fn valid_output(path: &Path) -> bool {
    path.metadata().is_ok_and(|m| m.is_file() && m.len() > 0)
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
    ensure!(
        job.options.subtitles.is_empty()
            && !job.options.thumbnail
            && job.options.sponsorblock.is_empty()
            && !job.options.split_chapters,
        "Local conversion supports format, resolution, audio quality, metadata and filename. Subtitle selection, cover embedding, SponsorBlock and chapter splitting require the original online download; disable those options for local conversion"
    );
    let stem = input
        .file_stem()
        .context("Input has no filename")?
        .to_string_lossy();
    let extension = if job.options.audio {
        job.options.audio_container.as_str()
    } else {
        job.options.container.as_str()
    };
    let extension = if extension == "vorbis" {
        "ogg"
    } else {
        extension
    };
    let filename = if job.options.filename_template == DownloadOptions::default().filename_template
    {
        std::path::PathBuf::from(format!("{stem}_converted.{extension}"))
    } else {
        let info: yt_dlp_rs::VideoInfo = serde_json::from_value(
            serde_json::json!({"id":job.id.to_string(),"title":stem,"ext":extension,"url":url::Url::from_file_path(input).map_err(|_| anyhow::anyhow!("Invalid local input path"))?,"webpage_url":job.media.url,"extractor":"local","extractor_key":"Local"}),
        )?;
        yt_dlp_rs::output::render_template(&job.options.filename_template, &info)?
    };
    ensure!(
        !filename.is_absolute()
            && !filename
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir)),
        "Conversion filename escaped its destination"
    );
    let target = job.options.output_directory().join(filename);
    ensure!(
        !target.exists(),
        "Conversion destination already exists; choose another directory or filename"
    );
    let staging = tempfile::Builder::new()
        .prefix(".ytui-convert-")
        .tempdir_in(job.options.output_directory())?;
    let partial = staging.path().join(format!("media.{extension}"));
    let mut args = vec!["-nostdin".into()];
    if config.offline {
        args.extend(["-protocol_whitelist".into(), "file,pipe".into()]);
    }
    args.extend(["-y".into(), "-i".into(), input.to_string_lossy().into()]);
    if job.options.audio {
        args.extend(["-vn".into(), "-map".into(), "0:a:0".into()]);
        args.extend(crate::audio::encoding(
            &job.options.audio_container,
            &job.options.audio_quality,
        )?);
    } else if job.options.quality != "best" {
        args.extend([
            "-vf".into(),
            format!("scale=-2:trunc(min(ih\\,{})/2)*2", job.options.quality),
        ]);
    }
    args.extend([
        "-map_metadata".into(),
        if job.options.metadata { "0" } else { "-1" }.into(),
    ]);
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
    ensure!(
        partial.metadata().is_ok_and(|m| m.is_file() && m.len() > 0),
        "FFmpeg did not produce a nonempty output file"
    );
    std::fs::create_dir_all(target.parent().context("No conversion output parent")?)?;
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

#[cfg(test)]
mod rate_limit_tests {
    use super::*;
    #[test]
    fn rate_limit_backoff_has_a_floor_and_keeps_normal_retry_bounds() {
        for message in [
            "HTTP error 429",
            "Server returned 429",
            "HTTP/2 429 Too Many Requests",
            "Too Many Requests",
        ] {
            assert!(transient(message));
            assert_eq!(retry_delay(0, 0, message), 60);
            assert_eq!(retry_delay(5, 4, message), 80);
            assert_eq!(retry_delay(300, 10, message), 300);
        }
        assert_eq!(retry_delay(5, 0, "Connection reset"), 5);
        assert_eq!(retry_delay(0, 0, "HTTP error 503"), 0);
        assert!(!transient(
            "HTTP error 429: Sign in to confirm you're not a bot"
        ));
    }
    #[tokio::test]
    async fn rate_limited_download_waits_and_cancellation_interrupts_the_cooldown() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let attempts = temp.path().join("attempts");
        let extractor = temp.path().join("extractor");
        std::fs::write(&extractor, format!("#!/bin/sh\nprintf 'attempt\\n' >> '{}'\necho 'ERROR: HTTP Error 429: Too Many Requests' >&2\nexit 1\n", attempts.display())).unwrap();
        std::fs::set_permissions(&extractor, std::fs::Permissions::from_mode(0o700)).unwrap();
        let paths = Paths {
            config_override: None,
            config: temp.path().join("config"),
            state: temp.path().join("state"),
            cache: temp.path().join("cache"),
            xdg_state: temp.path().join("xdg"),
            legacy_data: temp.path().join("legacy"),
            home: temp.path().into(),
        };
        paths.create().unwrap();
        let options = ytui_core::DownloadOptions {
            directory: temp.path().join("downloads"),
            ..Default::default()
        };
        let job = Job::new(
            Media {
                url: "https://www.youtube.com/watch?v=abcdefghijk".into(),
                ..Default::default()
            },
            options,
        );
        let config = Config {
            download_backend: "yt-dlp".into(),
            yt_dlp: extractor.to_string_lossy().into(),
            max_retries: 1,
            retry_delay_seconds: 0,
            ..Default::default()
        };
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let (tx, mut rx) = mpsc::channel(32);
        let task = tokio::spawn(execute(job, config, paths, None, token, tx));
        let observed = tokio::time::timeout(Duration::from_secs(3), async {
            while let Some(event) = rx.recv().await {
                if matches!(event.update, JobUpdate::Retrying(_)) {
                    return true;
                }
            }
            false
        })
        .await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        let count = std::fs::read_to_string(&attempts)
            .unwrap_or_default()
            .lines()
            .count();
        cancel.cancel();
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .expect("Cooldown cancellation stalled")
            .unwrap();
        assert!(
            observed.unwrap_or(false),
            "Retry cooldown was never entered"
        );
        assert_eq!(count, 1, "Download retried HTTP 429 immediately");
        while let Ok(event) = rx.try_recv() {
            assert!(!matches!(
                event.update,
                JobUpdate::Finished | JobUpdate::Failed { .. }
            ));
        }
    }
}

//! Resolve external playback explicitly rather than delegating extraction to mpv's Lua hook.
use crate::process::{self, ProcessSpec};
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::path::Path;
use tokio_util::sync::CancellationToken;
use ytui_core::Config;

pub(crate) struct Stream {
    pub url: String,
    pub audio: Option<String>,
    pub headers: Vec<(String, String)>,
    pub catalog: crate::formats::Catalog,
}
pub(crate) fn parse(data: &Value, audio: bool) -> Result<Stream> {
    let has_video = |format: &Value| {
        format.get("vcodec").and_then(Value::as_str) != Some("none")
            && format.get("stream_kind").and_then(Value::as_str) != Some("audio_only")
    };
    let has_audio = |format: &Value| {
        format.get("acodec").and_then(Value::as_str) != Some("none")
            && format.get("stream_kind").and_then(Value::as_str) != Some("video_only")
    };
    let formats = data.get("requested_formats").and_then(Value::as_array);
    let primary = formats
        .and_then(|formats| {
            formats.iter().find(|format| {
                if audio {
                    has_audio(format)
                } else {
                    has_video(format)
                }
            })
        })
        .unwrap_or(data);
    let url = |format: &Value| -> Result<String> {
        let url = format
            .get("url")
            .and_then(Value::as_str)
            .context("yt-dlp returned no playable stream URL")?;
        ytui_core::http_url(url).context("yt-dlp returned an unsupported playback URL")?;
        Ok(url.into())
    };
    let mut headers = Vec::new();
    // Forward the extractor's stream identity instead of overriding it with a
    // hardcoded browser UA. Authentication stays in the private cookie file.
    for source in [data, primary] {
        if let Some(map) = source.get("http_headers").and_then(Value::as_object) {
            for (name, value) in map {
                if ![
                    "user-agent",
                    "referer",
                    "origin",
                    "accept",
                    "accept-language",
                ]
                .contains(&name.to_ascii_lowercase().as_str())
                {
                    continue;
                }
                let value = value
                    .as_str()
                    .context("yt-dlp returned an invalid stream header")?;
                ensure!(
                    !value.chars().any(char::is_control),
                    "yt-dlp returned an unsafe stream header"
                );
                headers.retain(|(key, _): &(String, String)| !key.eq_ignore_ascii_case(name));
                headers.push((name.clone(), value.into()));
            }
        }
    }
    let extra_audio = if audio {
        None
    } else {
        formats.and_then(|formats| {
            formats
                .iter()
                .find(|format| !has_video(format) && has_audio(format))
        })
    };
    Ok(Stream {
        url: url(primary)?,
        audio: extra_audio.map(url).transpose()?,
        headers,
        catalog: crate::formats::Catalog::parse(data),
    })
}
pub(crate) async fn resolve(
    config: &Config,
    url: &str,
    format: &str,
    audio: bool,
    cookies: Option<&Path>,
    cancel: CancellationToken,
) -> Result<Stream> {
    ensure!(
        crate::paths::executable(&config.yt_dlp).is_some(),
        "yt-dlp is required for playback fallback. Install yt-dlp and a supported JavaScript runtime (Deno), or configure its path in Settings → Tools"
    );
    let mut args = vec![
        "--ignore-config".into(),
        "--no-colors".into(),
        "--no-playlist".into(),
        "--dump-single-json".into(),
        "--skip-download".into(),
        "--socket-timeout".into(),
        "15".into(),
        "--retries".into(),
        "1".into(),
        "--extractor-retries".into(),
        "1".into(),
        "--format".into(),
        format.into(),
    ];
    if let Some(proxy) = &config.proxy {
        args.extend(["--proxy".into(), proxy.clone()]);
    }
    if let Some(cookies) = cookies {
        args.extend(["--cookies".into(), cookies.to_string_lossy().into()]);
    }
    args.extend(["--".into(), url.into()]);
    let result = process::run(ProcessSpec::new(&config.yt_dlp, args, 60), cancel, |_| {})
        .await
        .context("yt-dlp playback extraction could not complete")?;
    ensure!(
        result.success,
        "yt-dlp playback extraction failed: {}",
        if result.stderr.is_empty() {
            "extractor exited without a diagnostic; check yt-dlp and its Deno runtime".into()
        } else {
            process::diagnostic(&result.stderr)
        }
    );
    let data: Value = serde_json::from_str(&result.lines.join("\n"))
        .context("yt-dlp returned invalid playback metadata")?;
    parse(&data, audio)
}
pub(crate) fn direct(mut spec: ProcessSpec, stream: Stream, cookies: Option<&Path>) -> ProcessSpec {
    spec.args.retain(|arg| {
        !arg.starts_with("--script-opts=")
            && !arg.starts_with("--ytdl-raw-options")
            && !arg.starts_with("--ytdl-format=")
            && !arg.starts_with("--user-agent=")
            && !arg.starts_with("--ytdl=")
    });
    spec.args.pop(); // original watch URL
    spec.args.pop(); // --
    spec.args.push("--ytdl=no".into());
    for (name, value) in stream.headers {
        if name.eq_ignore_ascii_case("user-agent") {
            spec.args.push(format!("--user-agent={value}"));
        } else if name.eq_ignore_ascii_case("referer") {
            spec.args.push(format!("--referrer={value}"));
        } else {
            let header = format!("{name}: {value}");
            // mpv string-list quoting preserves commas in header values.
            spec.args.push(format!(
                "--http-header-fields-append=%{}%{header}",
                header.len()
            ));
        }
    }
    if let Some(cookies) = cookies {
        spec.args.extend([
            "--cookies=yes".into(),
            format!("--cookies-file={}", cookies.display()),
        ]);
    }
    if let Some(audio) = stream.audio {
        spec.args.push(format!("--audio-file={audio}"));
    }
    spec.args.extend(["--".into(), stream.url]);
    spec
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn split_streams_keep_video_audio_and_extractor_headers() {
        let data = json!({"http_headers":{"User-Agent":"default-agent","Accept-Language":"en-US,en;q=0.9","Cookie":"excluded"},
        "requested_formats":[
            {"url":"https://media.test/audio","vcodec":"none","acodec":"opus"},
            {"url":"https://media.test/video","vcodec":"vp9","acodec":"none","http_headers":{"User-Agent":"stream-agent","Referer":"https://www.youtube.com/"}}
        ]});
        let stream = parse(&data, false).unwrap();
        assert_eq!(stream.url, "https://media.test/video");
        assert_eq!(stream.audio.as_deref(), Some("https://media.test/audio"));
        let spec = direct(
            ProcessSpec::new(
                "mpv",
                vec![
                    "--pause=yes".into(),
                    "--ytdl=yes".into(),
                    "--user-agent=wrong".into(),
                    "--script-opts=ytdl_hook-ytdl_path=unused".into(),
                    "--ytdl-format=best".into(),
                    "--".into(),
                    "https://www.youtube.com/watch?v=synthetic00".into(),
                ],
                30,
            ),
            stream,
            None,
        );
        assert!(spec.args.contains(&"--user-agent=stream-agent".into()));
        assert!(
            spec.args
                .contains(&"--referrer=https://www.youtube.com/".into())
        );
        assert!(
            spec.args
                .contains(&"--audio-file=https://media.test/audio".into())
        );
        assert!(spec.args.contains(&"--pause=yes".into()));
        assert!(spec.args.contains(&"--ytdl=no".into()));
        assert!(
            !spec
                .args
                .iter()
                .any(|a| a.contains("Cookie") || a.contains("wrong") || a.contains("ytdl_hook"))
        );
        let audio = parse(&data, true).unwrap();
        assert_eq!(audio.url, "https://media.test/audio");
        assert!(audio.audio.is_none());
        assert!(parse(&json!({"url":"file:///synthetic/private"}), false).is_err());
        assert!(parse(&json!({"url":"https://media.test/video","http_headers":{"User-Agent":"bad\r\nCookie: secret"}}),false).is_err());
        assert!(parse(&json!({"requested_formats":[]}), false).is_err());
    }
    fn tool(temp: &tempfile::TempDir, script: &str) -> String {
        let path = temp.path().join("extractor");
        std::fs::write(&path, format!("#!/bin/sh\n{script}")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.display().to_string()
    }
    #[tokio::test]
    async fn explicit_extraction_keeps_cookie_proxy_and_format_arguments_separate() {
        let temp = tempfile::tempdir().unwrap();
        let record = temp.path().join("arguments");
        let extractor = tool(
            &temp,
            &format!(
                "printf '%s\\n' \"$@\" > '{}'\nprintf '%s\\n' '{{\"url\":\"https://media.test/video\"}}'\n",
                record.display()
            ),
        );
        let config = Config {
            yt_dlp: extractor,
            proxy: Some("http://proxy.test:8080".into()),
            ..Default::default()
        };
        let cookies = temp.path().join("cookies with, commas.txt");
        let result = resolve(
            &config,
            "https://www.youtube.com/watch?v=synthetic00",
            "bestvideo+bestaudio/best",
            false,
            Some(&cookies),
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(result.url, "https://media.test/video");
        let args = std::fs::read_to_string(record).unwrap();
        assert!(args.contains("--cookies\n"));
        assert!(args.contains(&format!("{}\n", cookies.display())));
        assert!(args.contains("--proxy\nhttp://proxy.test:8080\n"));
        assert!(args.contains("--format\nbestvideo+bestaudio/best\n"));
        assert!(args.contains("--dump-single-json\n"));
        assert!(args.ends_with("--\nhttps://www.youtube.com/watch?v=synthetic00\n"));
    }
    #[tokio::test]
    async fn extraction_failure_is_specific_and_cancellation_reaps_worker() {
        let temp = tempfile::tempdir().unwrap();
        let extractor = tool(
            &temp,
            "echo 'ERROR: JavaScript runtime missing https://example.test/?token=synthetic-private' >&2\nexit 1\n",
        );
        let mut config = Config {
            yt_dlp: extractor,
            ..Default::default()
        };
        let error = resolve(
            &config,
            "https://www.youtube.com/watch?v=synthetic00",
            "best",
            false,
            None,
            CancellationToken::new(),
        )
        .await
        .err()
        .unwrap()
        .to_string();
        assert!(error.contains("yt-dlp playback extraction failed"));
        assert!(error.contains("JavaScript runtime missing"));
        assert!(!error.contains("synthetic-private"));
        config.yt_dlp = tool(&temp, "sleep 30\n");
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let task = tokio::spawn(async move {
            resolve(
                &config,
                "https://www.youtube.com/watch?v=synthetic00",
                "best",
                false,
                None,
                token,
            )
            .await
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        cancel.cancel();
        let result = tokio::time::timeout(std::time::Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap();
        assert!(format!("{:#}", result.err().unwrap()).contains("cancelled"));
    }
    #[tokio::test]
    async fn malformed_and_unsafe_extractor_responses_fail_without_exposing_urls() {
        let temp = tempfile::tempdir().unwrap();
        let metadata = temp.path().join("metadata.json");
        let extractor = tool(&temp, &format!("cat '{}'\n", metadata.display()));
        let config = Config {
            yt_dlp: extractor,
            ..Default::default()
        };
        for data in [
            "<html>not JSON</html>",
            "{}",
            r#"{"url":"ftp://media.test/stream?token=synthetic-private"}"#,
            r#"{"requested_formats":[{"url":"https://media.test/video","vcodec":"vp9","acodec":"none"},{"url":"file:///synthetic/private","vcodec":"none","acodec":"opus"}]}"#,
        ] {
            std::fs::write(&metadata, data).unwrap();
            let error = resolve(
                &config,
                "https://www.youtube.com/watch?v=synthetic00",
                "bestvideo+bestaudio/best",
                false,
                None,
                CancellationToken::new(),
            )
            .await
            .err()
            .expect("invalid metadata was accepted");
            let error = format!("{error:#}");
            assert!(!error.contains("synthetic-private"));
            assert!(!error.contains("file:///synthetic/private"));
        }
    }
    #[tokio::test]
    async fn missing_or_nonexecutable_extractors_report_the_actual_failure() {
        let temp = tempfile::tempdir().unwrap();
        let mut config = Config {
            yt_dlp: temp.path().join("missing").display().to_string(),
            ..Default::default()
        };
        let error = resolve(
            &config,
            "https://www.youtube.com/watch?v=synthetic00",
            "best",
            false,
            None,
            CancellationToken::new(),
        )
        .await
        .err()
        .unwrap();
        assert!(error.to_string().contains("yt-dlp is required"));
        config.yt_dlp = tool(&temp, "exit 0");
        std::fs::set_permissions(&config.yt_dlp, std::fs::Permissions::from_mode(0o644)).unwrap();
        let error = resolve(
            &config,
            "https://www.youtube.com/watch?v=synthetic00",
            "best",
            false,
            None,
            CancellationToken::new(),
        )
        .await
        .err()
        .unwrap();
        assert!(
            format!("{error:#}").contains("Permission denied"),
            "{error:#}"
        );
    }
    #[test]
    fn audio_stream_header_overrides_are_not_taken_from_video_and_urls_remain_encoded() {
        let data = json!({"http_headers":{"User-Agent":"common"},"requested_formats":[
            {"url":"https://media.test/video?signature=synthetic%2Bvalue","vcodec":"vp9","acodec":"none","http_headers":{"User-Agent":"video"}},
            {"url":"https://media.test/audio?signature=synthetic%2Fvalue","vcodec":"none","acodec":"opus","http_headers":{"User-Agent":"audio"}}
        ]});
        let audio = parse(&data, true).unwrap();
        assert_eq!(
            audio.url,
            "https://media.test/audio?signature=synthetic%2Fvalue"
        );
        assert!(
            audio
                .headers
                .contains(&("User-Agent".into(), "audio".into()))
        );
        let video = parse(&data, false).unwrap();
        assert_eq!(
            video.url,
            "https://media.test/video?signature=synthetic%2Bvalue"
        );
        assert!(
            video
                .headers
                .contains(&("User-Agent".into(), "video".into()))
        );
        assert_eq!(
            video.audio.as_deref(),
            Some("https://media.test/audio?signature=synthetic%2Fvalue")
        );
    }
}

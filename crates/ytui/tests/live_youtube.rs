//! Explicit public YouTube smoke checks. No browser profile or account is imported.
use std::{os::unix::fs::PermissionsExt, path::Path, process::Stdio, time::Duration};
use ytui_core::{Config, JobState};

fn backend() -> String {
    let backend = std::env::var("YTUI_LIVE_BACKEND").unwrap_or_else(|_| "auto".into());
    assert!(["auto", "native", "yt-dlp"].contains(&backend.as_str()));
    backend
}
fn video_url(music: bool) -> String {
    let variable = if music {
        "YTUI_LIVE_MUSIC_URL"
    } else {
        "YTUI_LIVE_VIDEO_URL"
    };
    std::env::var(variable).unwrap_or_else(|_| {
        format!(
            "https://{}.youtube.com/watch?v=jNQXAC9IVRw",
            if music { "music" } else { "www" }
        )
    })
}
async fn run(args: &[&str]) -> (std::process::Output, tempfile::TempDir) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let config = root.join("config.toml");
    let mpv = std::env::var("YTUI_TEST_MPV").unwrap_or_else(|_| "mpv".into());
    let wrapper = root.join("mpv-headless");
    if args.first() == Some(&"playback-check") {
        let program =
            ytui_services::paths::executable(&mpv).expect("Install mpv or set YTUI_TEST_MPV");
        let quoted = program.to_string_lossy().replace('\'', "'\\''");
        std::fs::write(
            &wrapper,
            format!("#!/bin/sh\nexec '{quoted}' --vo=null --ao=null \"$@\"\n"),
        )
        .unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let mut settings = Config {
        theme: "ytui-dark".into(),
        auth_store: "file".into(),
        notifications: false,
        download_backend: backend(),
        playback_backend: backend(),
        max_retries: 0,
        mpv: wrapper.display().to_string(),
        yt_dlp: std::env::var("YTUI_TEST_YT_DLP").unwrap_or_else(|_| "yt-dlp".into()),
        ..Default::default()
    };
    settings.download.quality = "360".into();
    settings.download.audio_quality = "128K".into();
    // No thumbnail/sidecar fetches or third-party SponsorBlock requests in these checks.
    settings.download.metadata = false;
    std::fs::write(&config, toml::to_string(&settings).unwrap()).unwrap();
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_ytui"));
    command
        .args([
            "--config",
            config.to_str().unwrap(),
            "--state-dir",
            root.join("state").to_str().unwrap(),
        ])
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for (key, name) in [
        ("XDG_CONFIG_HOME", "config"),
        ("XDG_STATE_HOME", "state"),
        ("XDG_CACHE_HOME", "cache"),
        ("XDG_DATA_HOME", "data"),
    ] {
        command.env(key, root.join(name));
    }
    let result = tokio::time::timeout(Duration::from_secs(240), command.output())
        .await
        .expect("Live request exceeded 240 seconds")
        .unwrap();
    (result, temp)
}
fn success(output: &std::process::Output) {
    assert!(
        output.status.success(),
        "{}",
        ytui_services::process::diagnostic(&String::from_utf8_lossy(&output.stderr))
    );
}
fn page(output: std::process::Output) -> ytui_core::Page {
    assert!(
        output.status.success(),
        "{}",
        ytui_services::process::diagnostic(&String::from_utf8_lossy(&output.stderr))
    );
    serde_json::from_slice(&output.stdout).expect("CLI did not return a page")
}
#[tokio::test]
#[ignore = "requires live public YouTube access; no account or cookies"]
async fn live_youtube_public_search() {
    let (output, _temp) = run(&["browse", "search", "Rust programming"]).await;
    assert!(!page(output).items.is_empty());
}
#[tokio::test]
#[ignore = "requires live public YouTube Music access; no account or cookies"]
async fn live_music_public_search() {
    let (output, _temp) = run(&["browse", "--music", "search", "classical music"]).await;
    assert!(!page(output).items.is_empty());
}
#[tokio::test]
#[ignore = "requires live public YouTube player/details access; no account or cookies"]
async fn live_youtube_video_details() {
    let url = url::Url::parse(&video_url(false)).unwrap();
    let id = url
        .query_pairs()
        .find(|(key, _)| key == "v")
        .map(|(_, value)| value.into_owned())
        .unwrap_or_else(|| url.path().trim_start_matches('/').into());
    assert_eq!(
        id.len(),
        11,
        "Use a public watch or youtu.be URL for the video fixture"
    );
    let (output, _temp) = run(&["browse", "details", &id]).await;
    let page = page(output);
    assert_eq!(page.items[0].id, id);
    assert!(!page.items[0].title.is_empty());
}

async fn playback(music: bool, audio: bool) {
    let url = video_url(music);
    let mut args = vec!["playback-check", url.as_str()];
    if audio {
        args.push("--audio");
    }
    let (output, _temp) = run(&args).await;
    success(&output);
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(result["position"].as_f64().is_some_and(|v| v >= 3.0));
    assert!(result["duration"].as_f64().is_some_and(|v| v > 3.0));
    assert_eq!(result["audio_only"].as_bool(), Some(audio));
    println!("Actual mpv playback advanced: {}", result);
}
#[tokio::test]
#[ignore = "real public YouTube video playback through ytui, yt-dlp/native and mpv; no cookies"]
async fn live_youtube_video_playback() {
    playback(false, false).await;
}
#[tokio::test]
#[ignore = "real public YouTube audio playback through ytui and mpv; no cookies"]
async fn live_youtube_audio_playback() {
    playback(false, true).await;
}
#[tokio::test]
#[ignore = "real public Music URL audio playback through ytui and mpv; no cookies"]
async fn live_music_audio_playback() {
    playback(true, true).await;
}

async fn download(audio: bool, playlist: bool) {
    let destination = tempfile::tempdir().unwrap();
    // Public four-entry playlist from yt-dlp's own YouTube tab tests (JODA15).
    let source = if playlist {
        std::env::var("YTUI_LIVE_PLAYLIST_URL").unwrap_or_else(|_| {
            "https://music.youtube.com/playlist?list=PL6IaIsEjSbf96XFRuNccS_RuEXwNdsoEu".into()
        })
    } else {
        video_url(audio)
    };
    let mut args = vec![
        "download",
        source.as_str(),
        "--directory",
        destination.path().to_str().unwrap(),
        "--now",
    ];
    if audio {
        args.push("--audio");
    }
    if playlist {
        args.extend(["--items", "1:2"]);
    }
    let (output, temp) = run(&args).await;
    success(&output);
    let store = ytui_services::store::Store::open(&temp.path().join("state")).unwrap();
    let snapshot = store.load().await.unwrap();
    store.close().await.unwrap();
    let job = &snapshot.downloads.jobs[0];
    assert_eq!(job.state, JobState::Done);
    assert_eq!(
        job.media.url, source,
        "playlist identity was replaced by its first song"
    );
    assert_eq!(
        job.outputs.len(),
        if playlist { 2 } else { 1 },
        "not all selected entries were saved"
    );
    if playlist {
        assert_ne!(job.outputs[0], job.outputs[1]);
    }
    for file in &job.outputs {
        assert!(file.is_file() && file.metadata().unwrap().len() > 0);
        let probe = tokio::process::Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "stream=codec_type:format=duration",
                "-of",
                "json",
            ])
            .arg(file)
            .output()
            .await
            .unwrap();
        success(&probe);
        let value: serde_json::Value = serde_json::from_slice(&probe.stdout).unwrap();
        let streams = value["streams"].as_array().unwrap();
        assert!(
            streams.iter().any(|s| s["codec_type"] == "audio"),
            "file has no audio stream"
        );
        if !audio {
            assert!(
                streams.iter().any(|s| s["codec_type"] == "video"),
                "file has no video stream"
            );
        }
        assert!(
            value["format"]["duration"]
                .as_str()
                .and_then(|s| s.parse::<f64>().ok())
                .is_some_and(|v| v > 0.0)
        );
        let decode = tokio::process::Command::new("ffmpeg")
            .args([
                "-nostdin",
                "-v",
                "error",
                "-xerror",
                "-err_detect",
                "explode",
                "-threads",
                "2",
                "-i",
            ])
            .arg(file)
            .args(["-map", "0:v?", "-map", "0:a?", "-f", "null", "-"])
            .output()
            .await
            .unwrap();
        success(&decode);
    }
    println!(
        "Verified {} real media file(s) with ffprobe and complete FFmpeg decoding",
        job.outputs.len()
    );
}
#[tokio::test]
#[ignore = "downloads a public YouTube video through the selected backend and verifies actual tracks"]
async fn live_youtube_video_download() {
    download(false, false).await;
}
#[tokio::test]
#[ignore = "downloads and converts public Music audio through the selected backend, then probes it"]
async fn live_music_audio_download() {
    download(true, false).await;
}
#[tokio::test]
#[ignore = "downloads and verifies two distinct public playlist entries, catching first-song-only regressions"]
async fn live_music_two_entry_playlist_download() {
    download(true, true).await;
}
#[tokio::test]
#[ignore = "downloads the small public yt-dlp test video with native Rust; requires YouTube and FFmpeg"]
async fn live_native_youtube_test_video_download() {
    let destination = tempfile::tempdir().unwrap();
    let source = video_url(false);
    let (output, _temp) = run(&[
        "download",
        &source,
        "--quality",
        "360",
        "--directory",
        destination.path().to_str().unwrap(),
        "--backend",
        "native",
        "--now",
    ])
    .await;
    assert!(
        output.status.success(),
        "{}",
        ytui_services::process::diagnostic(&String::from_utf8_lossy(&output.stderr))
    );
    assert!(
        std::fs::read_dir(destination.path())
            .unwrap()
            .flatten()
            .any(|e| e.path().is_file()
                && e.metadata().is_ok_and(|m| m.len() > 0)
                && Path::new(&e.file_name())
                    .extension()
                    .is_some_and(|e| e == "mp4"))
    );
}

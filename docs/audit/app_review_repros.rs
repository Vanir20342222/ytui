//! Manual audit regressions for e2b01ea. These assert the desired behavior and
//! intentionally fail on that revision. Copy into crates/ytui/tests/ to run.
//! Synthetic data only; no browser stores, VPN changes or public services.
use ratatui::layout::Rect;
use ratatui_image::{
    Resize,
    picker::{Picker, ProtocolType},
    protocol::Protocol,
};
use std::{io::Cursor, sync::Arc, time::Duration};
use tokio::sync::mpsc;
use ytui::app::{App, RuntimeHandles, UiEvent};
use ytui_core::{Config, DownloadOptions, Job, JobState, Media, MediaKind, shortcuts::Action};
use ytui_services::{
    auth::{Cookie, Session},
    images::ImageCache,
    paths::Paths,
    player::PlayerCommand,
    store::Snapshot,
};

fn fixture() -> (App, mpsc::Receiver<PlayerCommand>, tempfile::TempDir) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let paths = Paths {
        config_override: None,
        xdg_state: root.join("xdg"),
        config: root.join("config"),
        state: root.join("state"),
        cache: root.join("cache"),
        legacy_data: root.join("legacy"),
        home: root.into(),
    };
    paths.create().unwrap();
    let config = Config {
        theme: "ytui-dark".into(),
        auth_store: "file".into(),
        notifications: false,
        image_protocol: "halfblocks".into(),
        ..Default::default()
    };
    let (player, rx) = mpsc::channel(32);
    let (jobs, _) = mpsc::channel(32);
    let app = App::new(
        config,
        paths,
        Snapshot::default(),
        None,
        RuntimeHandles {
            player,
            jobs,
            picker: Picker::from_fontsize((8, 16)),
        },
        true,
        false,
    )
    .unwrap();
    (app, rx, temp)
}

#[tokio::test]
async fn audit_b01_queued_video_keeps_video_when_music_tab_is_open() {
    let (mut app, mut rx, _temp) = fixture();
    app.tab = 2;
    app.play_existing(Media {
        url: "https://www.youtube.com/watch?v=abcdefghijk".into(),
        kind: MediaKind::Video,
        ..Default::default()
    });
    let audio = match tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .unwrap()
        .unwrap()
    {
        PlayerCommand::Play { audio, .. } => audio,
        _ => panic!("Expected Play"),
    };
    app.shutdown().await;
    assert!(
        !audio,
        "A queued video became audio-only because Music is the visible tab"
    );
}

#[tokio::test]
async fn audit_b01_downloaded_audio_uses_the_jobs_audio_intent() {
    let (mut app, mut rx, temp) = fixture();
    app.demo = false;
    app.tab = 0;
    app.snapshot.downloads.jobs.clear();
    let path = temp.path().join("song.mp3");
    std::fs::write(&path, b"synthetic-audio").unwrap();
    let mut job = Job::new(
        Media {
            url: "https://www.youtube.com/watch?v=abcdefghijk".into(),
            kind: MediaKind::Video,
            ..Default::default()
        },
        DownloadOptions {
            audio: true,
            ..Default::default()
        },
    );
    job.state = JobState::Done;
    job.outputs.push(path);
    app.snapshot.downloads.jobs.push(job);
    app.action(Action::Open);
    let audio = match tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .unwrap()
        .unwrap()
    {
        PlayerCommand::Play { audio, .. } => audio,
        _ => panic!("Expected Play"),
    };
    app.shutdown().await;
    assert!(audio, "Opening an audio download requested a video window");
}

#[tokio::test]
async fn audit_b02_changing_image_protocol_invalidates_encoded_cards() {
    let (mut app, _rx, _temp) = fixture();
    let area = Rect::new(0, 0, 8, 4);
    let mut picker = app.picker.clone();
    picker.set_protocol_type(ProtocolType::Halfblocks);
    let protocol = picker
        .new_protocol(
            image::DynamicImage::new_rgb8(64, 64),
            area,
            Resize::Fit(None),
        )
        .unwrap();
    assert!(matches!(protocol, Protocol::Halfblocks(_)));
    let key = app.card_key(&app.tabs[1].page.items[0], area);
    app.card_images.insert(key, protocol);
    let mut config = app.config.clone();
    config.image_protocol = "sixel".into();
    app.apply(UiEvent::Config {
        config: Box::new(config),
        account: None,
        result: Ok(()),
    });
    let stale = !app.card_images.is_empty();
    app.shutdown().await;
    assert!(
        !stale,
        "Cards retain bytes encoded for the old terminal protocol"
    );
}

#[tokio::test]
async fn audit_b02_auto_does_not_unconditionally_downgrade_kitty() {
    let (mut app, _rx, _temp) = fixture();
    app.config.image_protocol = "kitty".into();
    app.picker.set_protocol_type(ProtocolType::Kitty);
    let mut config = app.config.clone();
    config.image_protocol = "auto".into();
    app.apply(UiEvent::Config {
        config: Box::new(config),
        account: None,
        result: Ok(()),
    });
    let actual = app.picker.protocol_type();
    app.shutdown().await;
    assert_eq!(
        actual,
        ProtocolType::Kitty,
        "Auto should preserve/re-detect terminal support instead of forcing Halfblocks"
    );
}

#[tokio::test]
async fn audit_b03_transient_card_error_can_recover_without_manual_refresh() {
    let (mut app, _rx, _temp) = fixture();
    app.tasks.abort_all();
    while app.tasks.join_next().await.is_some() {}
    let area = Rect::new(0, 0, 8, 4);
    app.card_areas = vec![(0, area)];
    let key = app.card_key(&app.tabs[1].page.items[0], area);
    app.refresh_image();
    assert!(!app.tasks.is_empty());
    app.tasks.abort_all();
    while app.tasks.join_next().await.is_some() {}
    app.apply(UiEvent::CardImage {
        key,
        result: Err(anyhow::anyhow!("temporary timeout")),
    });
    tokio::time::sleep(Duration::from_secs(3)).await;
    app.refresh_image();
    let retried = !app.tasks.is_empty();
    app.shutdown().await;
    assert!(
        retried,
        "Transient thumbnail failure permanently suppresses this card until explicit Refresh"
    );
}

async fn public_fixture(body: Vec<u8>) -> (String, tokio::task::JoinHandle<()>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 8192];
            let _ = socket.read(&mut request).await;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.write_all(&body).await;
        }
    });
    (url, task)
}

#[tokio::test]
async fn audit_b04_valid_thumbnail_survives_a_disk_cache_write_failure() {
    let temp = tempfile::tempdir().unwrap();
    let cache_dir = temp.path().join("blocked-cache");
    std::fs::write(&cache_dir, b"Synthetic filesystem obstruction").unwrap();
    let mut body = Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(16, 16)
        .write_to(&mut body, image::ImageFormat::Png)
        .unwrap();
    let (url, server) = public_fixture(body.into_inner()).await;
    // Route even the fixture's request through its own loopback server, so no
    // inherited proxy configuration can make this test contact a public host.
    let client = ytui_services::http_client(Some(&url)).unwrap();
    let result = ImageCache::new(client, cache_dir)
        .load_sized(&url, 16, 16)
        .await;
    server.abort();
    let _ = server.await;
    assert!(
        result.is_ok(),
        "Successful image download/decode was discarded because its optional disk cache failed: {}",
        result.err().unwrap()
    );
}

#[tokio::test]
async fn audit_b05_partial_playlist_outputs_are_visible_offline() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("first-song.mp3");
    std::fs::write(&path, b"synthetic completed track").unwrap();
    let mut job = Job::new(
        Media {
            url: "https://www.youtube.com/playlist?list=PLsynthetic".into(),
            ..Default::default()
        },
        DownloadOptions {
            audio: true,
            playlist: true,
            ..Default::default()
        },
    );
    job.state = JobState::Error;
    job.outputs.push(path);
    let queue = ytui_core::DownloadQueue { jobs: vec![job] };
    let page = ytui_services::library::catalog(&queue, "", Some(true));
    assert_eq!(
        page.items.len(),
        1,
        "A valid completed track is hidden solely because a later playlist entry failed"
    );
}

fn synthetic_session() -> Session {
    Session::new(vec![Cookie {
        domain: ".youtube.com".into(),
        path: "/".into(),
        secure: true,
        expires: None,
        name: "SAPISID".into(),
        value: "synthetic-only".into(),
        http_only: true,
    }])
    .unwrap()
}

#[tokio::test]
async fn audit_b06_late_auth_completion_does_not_undo_a_disconnect() {
    let (mut app, _rx, _temp) = fixture();
    app.session = Some(Arc::new(synthetic_session()));
    // Deterministic replay of completion order, not a claim of a timed live race.
    app.apply(UiEvent::Disconnected(Ok(())));
    assert!(app.session.is_none());
    app.apply(UiEvent::Auth(Ok(synthetic_session())));
    let connected = app.session.is_some();
    app.shutdown().await;
    assert!(
        !connected,
        "An older auth task's completion restored the disconnected session"
    );
}

#[test]
#[ignore = "requires actual FFmpeg/ffprobe; local files only"]
fn audit_b07_local_conversion_honors_requested_audio_bitrate() {
    use std::process::Command;
    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("source.wav");
    let generated = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440",
            "-t",
            "1",
            "-ac",
            "2",
        ])
        .arg(&input)
        .output()
        .unwrap();
    assert!(
        generated.status.success(),
        "{}",
        String::from_utf8_lossy(&generated.stderr)
    );
    let config = temp.path().join("config.toml");
    std::fs::write(
        &config,
        "theme='ytui-dark'\nauth_store='file'\nnotifications=false\n",
    )
    .unwrap();
    let output_dir = temp.path().join("output");
    let result = Command::new(env!("CARGO_BIN_EXE_ytui"))
        .arg("--config")
        .arg(&config)
        .arg("--state-dir")
        .arg(temp.path().join("state"))
        .args(["--offline", "download"])
        .arg(&input)
        .args(["--audio", "--audio-quality", "320K", "--directory"])
        .arg(&output_dir)
        .env("XDG_CONFIG_HOME", temp.path().join("config"))
        .env("XDG_STATE_HOME", temp.path().join("xdg-state"))
        .env("XDG_CACHE_HOME", temp.path().join("cache"))
        .env("XDG_DATA_HOME", temp.path().join("data"))
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let probe = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream=bit_rate",
            "-of",
            "json",
        ])
        .arg(output_dir.join("source_converted.mp3"))
        .output()
        .unwrap();
    assert!(probe.status.success());
    let data: serde_json::Value = serde_json::from_slice(&probe.stdout).unwrap();
    assert_eq!(
        data["streams"][0]["bit_rate"].as_str(),
        Some("320000"),
        "The CLI accepts --audio-quality but local conversion ignores it"
    );
}

#[tokio::test]
async fn audit_b08_different_audio_formats_are_not_duplicate_jobs() {
    let (mut app, _rx, _temp) = fixture();
    app.demo = false;
    app.snapshot.downloads.jobs.clear();
    let media = Media {
        url: "https://www.youtube.com/watch?v=abcdefghijk".into(),
        ..Default::default()
    };
    let mp3 = DownloadOptions {
        audio: true,
        audio_container: "mp3".into(),
        ..Default::default()
    };
    let flac = DownloadOptions {
        audio_container: "flac".into(),
        ..mp3.clone()
    };
    app.enqueue_download(media.clone(), mp3);
    app.enqueue_download(media, flac);
    let count = app.snapshot.downloads.jobs.len();
    app.shutdown().await;
    assert_eq!(
        count, 2,
        "A distinct FLAC download was rejected as a duplicate of MP3"
    );
}

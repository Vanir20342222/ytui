//! Audit regressions promoted from the e2b01ea defect harness.
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

#[tokio::test]
async fn completed_tool_panels_never_reopen_after_escape_or_replace_a_newer_request() {
    use ytui::app::Modal;
    let (mut app, _rx, _temp) = fixture();
    app.command("thumbnail-cache").unwrap();
    let Some(Modal::PendingText { id: old, .. }) = app.modal else {
        panic!("Expected loading panel")
    };
    app.key(crossterm::event::KeyCode::Esc.into());
    app.apply(UiEvent::Text {
        id: old,
        result: Ok(vec!["Old result".into()]),
    });
    assert!(app.modal.is_none());
    app.command("thumbnail-cache").unwrap();
    let Some(Modal::PendingText { id: new, .. }) = app.modal else {
        panic!("Expected new loading panel")
    };
    assert_ne!(old, new);
    app.apply(UiEvent::Text {
        id: old,
        result: Err(anyhow::anyhow!("Obsolete error")),
    });
    assert!(matches!(app.modal, Some(Modal::PendingText { id, .. }) if id == new));
    app.apply(UiEvent::Text {
        id: new,
        result: Ok(vec!["Latest result".into()]),
    });
    assert!(matches!(&app.modal, Some(Modal::Text { lines, .. }) if lines == &["Latest result"]));
    app.shutdown().await;
}

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
    let operation = app.operations.config(false);
    app.apply(UiEvent::Config {
        operation,
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
    app.detected_image_protocol = ProtocolType::Kitty;
    app.picker.set_protocol_type(ProtocolType::Kitty);
    let mut config = app.config.clone();
    config.image_protocol = "auto".into();
    let operation = app.operations.config(false);
    app.apply(UiEvent::Config {
        operation,
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
        generation: 0,
        key,
        result: Err(anyhow::anyhow!("request timed out").context("Thumbnail unavailable")),
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
    let old_import = app.operations.account();
    let disconnect = app.operations.account();
    app.apply(UiEvent::Disconnected {
        operation: disconnect,
        result: Ok(()),
    });
    assert!(app.session.is_none());
    app.apply(UiEvent::Auth {
        operation: old_import,
        result: Ok(synthetic_session()),
    });
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

#[tokio::test]
async fn old_eof_and_busy_commands_do_not_skip_or_replace_new_queue_items() {
    let (mut app, mut rx, _temp) = fixture();
    app.demo = false;
    let current = Media {
        url: "https://www.youtube.com/watch?v=abcdefghijk".into(),
        ..Default::default()
    };
    let old = Media {
        url: "https://www.youtube.com/watch?v=oldfixture1".into(),
        ..Default::default()
    };
    app.snapshot.playback.play(current.clone());
    app.snapshot.playback.push(old.clone());
    app.playback_ended(&old);
    assert_eq!(app.snapshot.playback.current, Some(0));
    assert!(rx.try_recv().is_err());
    while app.player.try_send(PlayerCommand::TogglePause).is_ok() {}
    let before = app.snapshot.playback.items.len();
    app.play(old);
    assert_eq!(app.snapshot.playback.items.len(), before);
    assert_eq!(app.snapshot.playback.current, Some(0));
    assert!(app.status.contains("busy"));
    app.shutdown().await;
}

#[test]
fn download_identity_compares_files_and_not_transfer_policy() {
    let options = DownloadOptions::default();
    let mut other = options.clone();
    other.backend = Some("yt-dlp".into());
    other.bandwidth_kib = 250;
    assert!(options.same_output(&other));
    other.container = "mkv".into();
    assert!(!options.same_output(&other));
    other = options.clone();
    other.directory = "/another/destination".into();
    assert!(!options.same_output(&other));
    other = options.clone();
    other.subtitles = vec!["en".into()];
    assert!(!options.same_output(&other));
}

#[tokio::test]
async fn partial_collection_files_can_be_selected_and_played_from_downloads() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let (mut app, mut player, temp) = fixture();
    app.demo = false;
    let mut job = Job::new(
        Media::default(),
        DownloadOptions {
            audio: true,
            playlist: true,
            ..Default::default()
        },
    );
    job.state = JobState::Error;
    for name in ["first.mp3", "second.mp3"] {
        let path = temp.path().join(name);
        std::fs::write(&path, b"fixture").unwrap();
        job.outputs.push(path);
    }
    app.snapshot.downloads.jobs = vec![job];
    app.tab = 0;
    app.action(Action::Open);
    assert!(matches!(app.modal, Some(ytui::app::Modal::Outputs { .. })));
    app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    match player.recv().await.unwrap() {
        PlayerCommand::Play { media, audio } => {
            assert!(media.url.ends_with("second.mp3"));
            assert!(audio);
        }
        _ => panic!("Expected second output playback"),
    }
    assert_eq!(app.snapshot.downloads.jobs[0].state, JobState::Error);
    app.shutdown().await;
}

#[tokio::test]
async fn disconnect_waits_for_committed_settings_and_busy_form_keeps_the_draft() {
    let (mut app, _player, _temp) = fixture();
    app.demo = false;
    app.session = Some(Arc::new(synthetic_session()));
    let mut saved = app.config.clone();
    saved.account_index = 1;
    app.paths.save_config(&saved).unwrap();
    ytui_services::auth::SessionStore::new(&app.paths, "file")
        .save(&synthetic_session())
        .unwrap();
    let ticket = app.operations.config(true);
    app.pending_operation = Some((ticket.clone(), "Saving settings".into()));
    let mut draft = saved.clone();
    draft.theme = "ytui-light".into();
    app.save_config(draft);
    assert!(matches!(app.modal, Some(ytui::app::Modal::Form(_))));
    app.command("disconnect").unwrap();
    // The write has already committed. Its completion must be applied before
    // disconnect uses the new account/store, instead of abandoning saved config.
    app.apply(UiEvent::Config {
        operation: ticket,
        config: Box::new(saved),
        account: Some(synthetic_session()),
        result: Ok(()),
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        while app.pending_operation.is_some() {
            let event = app.tasks.join_next().await.unwrap().unwrap();
            app.apply(event);
        }
    })
    .await
    .unwrap();
    assert!(app.session.is_none());
    assert_eq!(app.config.account_index, 1);
    assert_eq!(app.paths.load_config().unwrap().account_index, 1);
    assert!(
        ytui_services::auth::SessionStore::new(&app.paths, "file")
            .load()
            .unwrap()
            .is_none()
    );
    app.shutdown().await;
}

#[tokio::test]
async fn long_continuation_feeds_have_bounded_memory_without_stopping_pagination() {
    let (mut app, _player, _temp) = fixture();
    app.tab = 1;
    for page in 0..40 {
        let items = (0..100)
            .map(|index| Media {
                id: format!("{page}-{index}"),
                title: format!("Video {page}-{index}"),
                description: "x".repeat(4096),
                ..Default::default()
            })
            .collect();
        app.tabs[1].selected = app.tabs[1].page.items.len().saturating_sub(1);
        app.apply(UiEvent::Page {
            tab: 1,
            generation: 0,
            append: true,
            result: Ok(ytui_core::Page {
                items,
                continuation: Some(format!("page-{}", page + 1)),
                ..Default::default()
            }),
        });
        assert!(app.tabs[1].page.items.len() <= 2000);
        assert!(app.tabs[1].page.retained_bytes() < 8 * 1024 * 1024 + 1024);
        assert!(app.tabs[1].page.continuation.is_some());
        assert!(app.tabs[1].selected < app.tabs[1].page.items.len());
    }
    assert!(app.tabs[1].page.items.last().unwrap().id.starts_with("39-"));
    app.shutdown().await;
}

#[tokio::test]
async fn collection_entries_resume_individually_and_survive_store_restart() {
    let (mut app, _player, temp) = fixture();
    app.snapshot.downloads.jobs.clear();
    let media = (0..3)
        .map(|index| Media {
            id: format!("track{index}"),
            title: format!("Track {index}"),
            url: format!("https://www.youtube.com/watch?v=abcdefghij{index}"),
            ..Default::default()
        })
        .collect::<Vec<_>>();
    let options = DownloadOptions {
        audio: true,
        backend: Some("native".into()),
        ..Default::default()
    };
    app.queue_collection(
        "VLfixture".into(),
        "Three tracks".into(),
        options.clone(),
        "all",
        media.clone(),
    )
    .unwrap();
    let group = app.snapshot.downloads.jobs[0].group_id.clone();
    for (index, job) in app.snapshot.downloads.jobs.iter_mut().enumerate() {
        assert!(!job.options.playlist);
        assert_eq!(job.entry_index, Some(index + 1));
        if index < 2 {
            let file = temp.path().join(format!("track{index}.mp3"));
            std::fs::write(&file, b"fixture").unwrap();
            job.outputs.push(file);
            job.state = JobState::Done;
        } else {
            job.state = JobState::Error;
        }
    }
    let store = ytui_services::store::Store::open(&app.paths.state).unwrap();
    store.save(app.snapshot.clone()).await.unwrap();
    store.close().await.unwrap();
    let store = ytui_services::store::Store::open(&app.paths.state).unwrap();
    app.snapshot = store.load().await.unwrap();
    app.queue_collection(
        "VLfixture".into(),
        "Three tracks".into(),
        options.clone(),
        "all",
        media.clone(),
    )
    .unwrap();
    assert_eq!(app.snapshot.downloads.jobs.len(), 3);
    assert_eq!(app.snapshot.downloads.jobs[0].state, JobState::Done);
    assert_eq!(app.snapshot.downloads.jobs[1].state, JobState::Done);
    assert_eq!(app.snapshot.downloads.jobs[2].state, JobState::Queued);
    assert!(
        app.snapshot
            .downloads
            .jobs
            .iter()
            .all(|job| job.group_id == group)
    );
    assert!(
        app.queue_collection(
            "VLfixture".into(),
            "Three tracks".into(),
            options,
            "1,99",
            media
        )
        .is_err()
    );
    assert_eq!(app.snapshot.downloads.jobs.len(), 3);
    store.close().await.unwrap();
    app.shutdown().await;
}

#[tokio::test]
async fn browser_profile_selection_reviews_the_path_without_importing_cookies() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let (mut app, _player, temp) = fixture();
    app.demo = false;
    let profile = temp.path().join("Brave-Origin/Default");
    app.modal = Some(ytui::app::Modal::Account {
        profiles: vec![],
        selected: 0,
    });
    app.apply(UiEvent::AccountProfiles(vec![
        ytui_services::auth::BrowserProfile {
            browser: "brave-origin".into(),
            path: profile.clone(),
        },
    ]));
    app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let Some(ytui::app::Modal::Form(form)) = &app.modal else {
        panic!("Expected review form")
    };
    assert_eq!(form.fields[1].value.text, profile.display().to_string());
    assert!(app.pending_operation.is_none());
    assert!(app.session.is_none());
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    app.apply(UiEvent::AccountProfiles(vec![]));
    assert!(
        app.modal.is_none(),
        "late discovery must not reopen a closed panel"
    );
    app.shutdown().await;
}

#[tokio::test]
async fn playback_dialog_selects_reported_formats_tracks_and_captions() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let (mut app, mut received, _temp) = fixture();
    app.player_status.catalog = ytui_services::formats::Catalog::parse(
        &serde_json::json!({"formats":[{"format_id":"137","vcodec":"avc1","acodec":"none","height":1080}],"subtitles":{"de":[{"ext":"vtt","url":"https://example.test/caption"}]}}),
    );
    app.player_status.tracks = ytui_services::formats::tracks(
        &serde_json::json!([{"id":2,"type":"audio","lang":"de","codec":"aac"}]),
    );
    for section in 0..3 {
        app.modal = Some(ytui::app::Modal::PlayerSettings {
            section,
            selected: 0,
        });
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        match (section, received.recv().await.unwrap()) {
            (0, PlayerCommand::Format(format)) => assert_eq!(format.id, "137"),
            (1, PlayerCommand::SelectTrack { kind, id }) => {
                assert_eq!(kind, "audio");
                assert_eq!(id, 2);
            }
            (2, PlayerCommand::Caption(caption)) => assert!(caption.label.contains("de")),
            _ => panic!("Wrong playback selection command"),
        }
    }
    app.shutdown().await;
}

#[tokio::test]
async fn automatic_queue_advance_waits_for_a_full_player_channel_and_stop_cancels_it() {
    let (mut app, mut received, _temp) = fixture();
    let first = Media {
        url: "https://example.test/first.mp4".into(),
        ..Default::default()
    };
    let second = Media {
        url: "https://example.test/second.mp4".into(),
        ..Default::default()
    };
    app.snapshot.playback.items = vec![first.clone(), second.clone()];
    app.snapshot.playback.current = Some(0);
    for _ in 0..32 {
        app.player.try_send(PlayerCommand::TogglePause).unwrap();
    }
    app.playback_ended(&first);
    assert_eq!(app.snapshot.playback.current, Some(1));
    for _ in 0..32 {
        received.recv().await.unwrap();
    }
    app.tick().await;
    assert!(
        matches!(received.recv().await, Some(PlayerCommand::Play { media, .. }) if media.url == second.url)
    );
    app.action(Action::NextTrack);
    assert_eq!(
        app.snapshot.playback.current,
        Some(1),
        "next at queue end must preserve the playing selection"
    );
    app.snapshot.playback.current = Some(0);
    for _ in 0..32 {
        app.player.try_send(PlayerCommand::TogglePause).unwrap();
    }
    app.playback_ended(&first);
    app.action(Action::Stop);
    for _ in 0..32 {
        received.recv().await.unwrap();
    }
    app.tick().await;
    assert!(
        received.try_recv().is_err(),
        "Stop must cancel deferred auto-play"
    );
    app.shutdown().await;
}

#[tokio::test]
async fn recent_destination_history_is_bounded_and_persisted_without_moving_files() {
    let (mut app, _player, temp) = fixture();
    app.demo = false;
    app.snapshot.downloads.jobs.clear();
    for index in 0..15 {
        app.enqueue_download(
            Media {
                url: format!("https://example.test/{index}.mp4"),
                ..Default::default()
            },
            DownloadOptions {
                directory: temp.path().join(format!("folder-{index}")),
                ..Default::default()
            },
        );
    }
    assert_eq!(app.snapshot.recent_destinations.len(), 12);
    assert!(app.snapshot.recent_destinations[0].ends_with("folder-14"));
    let store = ytui_services::store::Store::open(&app.paths.state).unwrap();
    store.save(app.snapshot.clone()).await.unwrap();
    let loaded = store.load().await.unwrap();
    assert_eq!(loaded.recent_destinations, app.snapshot.recent_destinations);
    assert!(
        !temp.path().join("folder-14").exists(),
        "history must not create/move destination contents"
    );
    store.close().await.unwrap();
    app.shutdown().await;
}

#[tokio::test]
async fn disconnect_handoff_waits_for_player_capacity_and_blocks_old_account_playback() {
    let (mut app, mut received, _temp) = fixture();
    app.session = Some(Arc::new(synthetic_session()));
    for _ in 0..32 {
        app.player.try_send(PlayerCommand::TogglePause).unwrap();
    }
    let operation = app.operations.account();
    app.apply(UiEvent::Disconnected {
        operation,
        result: Ok(()),
    });
    assert!(app.player_session_pending);
    assert!(!app.play_existing(Media {
        url: "https://example.test/new.mp4".into(),
        ..Default::default()
    }));
    for _ in 0..32 {
        received.recv().await.unwrap();
    }
    app.tick().await;
    assert!(matches!(
        received.recv().await,
        Some(PlayerCommand::Session(None))
    ));
    assert!(!app.player_session_pending);
    assert!(app.session.is_none());
    app.shutdown().await;
}

#[tokio::test]
async fn repeated_disconnect_completes_cleanup_without_leaving_settings_busy() {
    let (mut app, _player, _temp) = fixture();
    app.demo = false;
    app.session = Some(Arc::new(synthetic_session()));
    let store = ytui_services::auth::SessionStore::new(&app.paths, "file");
    store.save(&synthetic_session()).unwrap();
    app.command("disconnect").unwrap();
    app.command("disconnect").unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while app.pending_operation.is_some() {
            let event = app.tasks.join_next().await.unwrap().unwrap();
            app.apply(event);
        }
    })
    .await
    .unwrap();
    assert!(store.load().unwrap().is_none());
    assert!(app.session.is_none());
    app.save_config(app.config.clone());
    assert!(
        app.pending_operation.is_some(),
        "settings must accept another save"
    );
    app.shutdown().await;
}

#[tokio::test]
async fn network_settings_cancel_old_feed_requests_and_reject_their_results() {
    use ytui::app::FeedLoad;
    use ytui_core::{BrowseRequest, Page};
    let (mut app, _rx, _temp) = fixture();
    let request = BrowseRequest::Search("kept query".into());
    app.tabs[1].request = Some(request.clone());
    app.tabs[1].load = Some(FeedLoad {
        request,
        append: false,
        preserve_selection: true,
        failures: 3,
        retry_at: None,
    });
    app.tabs[1].loading = true;
    app.tabs[3].request = Some(BrowseRequest::Cliamp);
    app.tabs[3].error = Some("Old proxy failed".into());
    let old_generation = app.tabs[1].generation;
    let cancel = app.tabs[1].cancel.clone();
    let kept_title = app.tabs[1].page.items[0].title.clone();
    let mut config = app.config.clone();
    config.proxy = Some("http://127.0.0.1:9".into());
    let operation = app.operations.config(false);
    app.apply(UiEvent::Config {
        operation,
        config: Box::new(config),
        account: None,
        result: Ok(()),
    });
    assert!(cancel.is_cancelled(), "Old proxy request remains in flight");
    assert!(
        app.tabs[3].load.is_some(),
        "Failed radio tab needs automatic recovery on the new route"
    );
    assert!(app.tabs[1].generation > old_generation);
    app.apply(UiEvent::Page {
        tab: 1,
        generation: old_generation,
        append: false,
        result: Ok(Page::default()),
    });
    assert_eq!(app.tabs[1].page.items[0].title, kept_title);
    assert!(
        app.tabs[1].load.is_some(),
        "Pending feed must remain recoverable"
    );
    app.shutdown().await;
}

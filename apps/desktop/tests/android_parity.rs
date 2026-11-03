//! Desktop counterparts of Android settings: seek step, saved offline copies,
//! next-item preloading and catalog-row details. Synthetic state only; HTTP goes to
//! a closed local proxy and media files are temporary fixtures.
use ratatui::{Terminal, backend::TestBackend};
use ratatui_image::picker::Picker;
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;
use ytui::{
    app::{App, RuntimeHandles},
    ui,
};
use ytui_core::{
    Config, DownloadOptions, Job, JobState, Media, MediaKind, Page, shortcuts::Action,
};
use ytui_services::{
    paths::Paths,
    player::{PlayerCommand, PlayerStatus},
    store::Snapshot,
};

fn fixture(demo: bool) -> (App, mpsc::Receiver<PlayerCommand>, tempfile::TempDir) {
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
        image_protocol: "none".into(),
        thumbnails: false,
        proxy: Some("http://127.0.0.1:9".into()),
        ..Default::default()
    };
    let (player, rx) = mpsc::channel(64);
    let (jobs, _) = mpsc::channel(32);
    let mut app = App::new(
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
    app.demo = demo;
    (app, rx, temp)
}

fn video(id: &str) -> Media {
    Media {
        id: id.into(),
        title: format!("Video {id}"),
        url: format!("https://www.youtube.com/watch?v={id}"),
        kind: MediaKind::Video,
        ..Default::default()
    }
}

/// A completed single-file download of `media` with a real fixture file.
fn saved(root: &Path, media: &Media, audio: bool) -> (Job, PathBuf) {
    let path = root.join(format!(
        "{}.{}",
        media.id,
        if audio { "m4a" } else { "mp4" }
    ));
    std::fs::write(&path, b"synthetic").unwrap();
    let mut job = Job::new(
        Media {
            // Downloads may record another URL form of the same video.
            url: format!("https://youtu.be/{}", media.id),
            ..media.clone()
        },
        DownloadOptions {
            audio,
            ..Default::default()
        },
    );
    job.state = JobState::Done;
    job.outputs = vec![path.clone()];
    (job, std::fs::canonicalize(path).unwrap())
}

fn drain(rx: &mut mpsc::Receiver<PlayerCommand>) -> Vec<PlayerCommand> {
    let mut commands = Vec::new();
    while let Ok(command) = rx.try_recv() {
        commands.push(command);
    }
    commands
}

#[tokio::test]
async fn seek_keys_use_the_configured_step() {
    let (mut app, mut rx, _temp) = fixture(true);
    app.action(Action::SeekForward);
    app.config.seek_seconds = 30;
    app.action(Action::SeekForward);
    app.action(Action::SeekBack);
    let seeks = drain(&mut rx)
        .into_iter()
        .filter_map(|c| match c {
            PlayerCommand::Seek(step) => Some(step),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(seeks, [5.0, 30.0, -30.0]);
    let config = Config {
        seek_seconds: 0,
        ..Default::default()
    };
    assert!(config.validate().is_err());
    app.shutdown().await;
}

#[tokio::test]
async fn saved_copies_play_locally_in_the_matching_mode_and_keep_the_queue_identity() {
    let (mut app, mut rx, temp) = fixture(false);
    let media = video("aaaaaaaaaaa");
    let (job, path) = saved(temp.path(), &media, false);
    app.snapshot.downloads.jobs.push(job);
    assert!(app.play_existing(media.clone()));
    match drain(&mut rx).pop() {
        Some(PlayerCommand::PlayLocal {
            media: played,
            audio: false,
            path: opened,
            seconds: None,
        }) => {
            assert_eq!(played.url, media.url, "history and queue keep the original");
            assert_eq!(opened, path);
        }
        _ => panic!("expected the saved copy"),
    }
    assert!(app.status.contains("saved offline copy"));

    // An audio-only copy does not replace video playback (Android matches the mode).
    let other = video("bbbbbbbbbbb");
    let (job, _) = saved(temp.path(), &other, true);
    app.snapshot.downloads.jobs.push(job);
    assert!(app.play_existing(other.clone()));
    assert!(matches!(
        drain(&mut rx).pop(),
        Some(PlayerCommand::Play { .. })
    ));
    let mut song = other.clone();
    song.kind = MediaKind::Song;
    assert!(app.play_existing(song));
    assert!(matches!(
        drain(&mut rx).pop(),
        Some(PlayerCommand::PlayLocal { audio: true, .. })
    ));

    // Copies with removed segments differ from the stream's timeline.
    let cut = video("ccccccccccc");
    let (mut job, _) = saved(temp.path(), &cut, false);
    job.options.sponsorblock_remove = true;
    app.snapshot.downloads.jobs.push(job);
    assert!(app.play_existing(cut));
    assert!(matches!(
        drain(&mut rx).pop(),
        Some(PlayerCommand::Play { .. })
    ));

    // Turning the setting off streams again.
    app.config.prefer_offline = false;
    assert!(app.play_existing(media.clone()));
    assert!(matches!(
        drain(&mut rx).pop(),
        Some(PlayerCommand::Play { .. })
    ));
    app.shutdown().await;
}

#[tokio::test]
async fn saved_copies_resume_and_play_in_offline_mode() {
    let (mut app, mut rx, temp) = fixture(false);
    let media = Media {
        duration: Some(600),
        ..video("aaaaaaaaaaa")
    };
    let (job, path) = saved(temp.path(), &media, false);
    app.snapshot.downloads.jobs.push(job);
    app.snapshot
        .positions
        .record(&media, 125.0, Some(600.0), false);
    app.offline = true;
    assert!(app.play_existing(media.clone()));
    assert!(matches!(
        drain(&mut rx).pop(),
        Some(PlayerCommand::PlayLocal { path: opened, seconds: Some(s), .. }) if opened == path && s == 125.0
    ));
    let remote = video("ddddddddddd");
    assert!(
        !app.play_existing(remote),
        "no copy: offline mode still refuses"
    );
    assert!(drain(&mut rx).is_empty());
    assert_eq!(app.resume_positions().len(), 1);
    app.shutdown().await;
}

fn status(media: &Media, position: f64, duration: f64) -> PlayerStatus {
    PlayerStatus {
        media: Some(media.clone()),
        active: true,
        position,
        duration: Some(duration),
        ..Default::default()
    }
}

#[tokio::test]
async fn the_next_item_is_prepared_once_near_the_end_of_the_current_one() {
    let (mut app, mut rx, temp) = fixture(false);
    let first = video("aaaaaaaaaaa");
    let second = video("bbbbbbbbbbb");
    app.snapshot.playback.play(first.clone());
    app.snapshot.playback.push(second.clone());
    app.player_update(status(&first, 10.0, 300.0));
    assert!(drain(&mut rx).is_empty(), "too early to prepare");
    app.player_update(status(&first, 250.0, 300.0));
    app.player_update(status(&first, 251.0, 300.0));
    let prepared = drain(&mut rx)
        .into_iter()
        .filter_map(|c| match c {
            PlayerCommand::Prepare { media, audio } => Some((media.url, audio)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(prepared, [(second.url.clone(), false)], "once per pair");

    // A saved copy needs no preparation; neither does a disabled setting.
    let (mut app, mut rx, _t) = fixture(false);
    app.snapshot.playback.play(first.clone());
    app.snapshot.playback.push(second.clone());
    app.snapshot
        .downloads
        .jobs
        .push(saved(temp.path(), &second, false).0);
    app.player_update(status(&first, 250.0, 300.0));
    assert!(
        !drain(&mut rx)
            .iter()
            .any(|c| matches!(c, PlayerCommand::Prepare { .. }))
    );
    let (mut app, mut rx, _t) = fixture(false);
    app.config.preload_next = false;
    app.snapshot.playback.play(first.clone());
    app.snapshot.playback.push(second.clone());
    app.player_update(status(&first, 250.0, 300.0));
    assert!(
        !drain(&mut rx)
            .iter()
            .any(|c| matches!(c, PlayerCommand::Prepare { .. }))
    );
    app.shutdown().await;
}

#[tokio::test]
async fn the_first_recommendation_is_prepared_when_the_queue_ends() {
    let (mut app, mut rx, _temp) = fixture(false);
    let only = video("sssssssssss");
    app.snapshot.playback.play(only.clone());
    app.ensure_related(false);
    app.apply(ytui::app::UiEvent::Related {
        generation: app.related_generation(),
        result: Ok(Page {
            items: vec![video("rrrrrrrrrr1")],
            ..Default::default()
        }),
    });
    assert_eq!(app.next_up().unwrap().url, video("rrrrrrrrrr1").url);
    app.player_update(status(&only, 280.0, 300.0));
    assert!(drain(&mut rx).iter().any(
        |c| matches!(c, PlayerCommand::Prepare { media, .. } if media.url == video("rrrrrrrrrr1").url)
    ));
    app.shutdown().await;
}

#[tokio::test]
async fn catalog_rows_show_the_selected_details() {
    let (mut app, _rx, _temp) = fixture(true);
    app.tabs[1].page.items = vec![Media {
        creator: "Synthetic channel".into(),
        duration: Some(185),
        views_text: "12K views".into(),
        published_text: "3 days ago".into(),
        like_count: Some(950),
        section: "Fixtures".into(),
        ..video("aaaaaaaaaaa")
    }];
    app.tab = 1;
    let render = |app: &mut App| {
        let mut terminal = Terminal::new(TestBackend::new(160, 40)).unwrap();
        terminal.draw(|f| ui::render(f, app)).unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>()
    };
    let text = render(&mut app);
    assert!(text.contains("Synthetic channel") && text.contains("3:05 · Fixtures"));
    assert!(!text.contains("12K views") && !text.contains("950 likes"));
    app.config.card_info.views = true;
    app.config.card_info.upload_date = true;
    app.config.card_info.likes = true;
    app.config.card_info.section = false;
    app.config.card_info.channel = false;
    let text = render(&mut app);
    assert!(text.contains("3:05 · 12K views · 3 days ago · 950 likes"));
    assert!(!text.contains("Synthetic channel"));
    app.shutdown().await;
}

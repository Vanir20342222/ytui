//! Isolated feature journeys: no browser/session access and no public requests.
use crossterm::event::KeyCode;
use ratatui::{Terminal, backend::TestBackend};
use ratatui_image::picker::Picker;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use ytui::{
    app::{App, Modal, RuntimeHandles},
    ui,
};
use ytui_core::{Config, Media, Page, shortcuts::Action};
use ytui_services::{paths::Paths, player::PlayerCommand, store::Snapshot};

fn fixture() -> (App, mpsc::Receiver<PlayerCommand>, tempfile::TempDir) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let paths = Paths {
        config_override: None,
        config: root.join("config"),
        state: root.join("state"),
        cache: root.join("cache"),
        xdg_state: root.join("xdg"),
        legacy_data: root.join("legacy"),
        home: root.into(),
    };
    paths.create().unwrap();
    let (player, rx) = mpsc::channel(2);
    let (jobs, _) = mpsc::channel(32);
    let app = App::new(
        Config {
            auth_store: "file".into(),
            notifications: false,
            theme: "ytui-dark".into(),
            ..Default::default()
        },
        paths,
        Snapshot::default(),
        None,
        RuntimeHandles {
            player,
            jobs,
            picker: Picker::from_fontsize((8, 16)),
        },
        false,
        true,
    )
    .unwrap();
    (app, rx, temp)
}
fn media() -> Media {
    Media {
        id: "abcdefghijk".into(),
        title: "Saved fixture".into(),
        creator: "Fixture artist".into(),
        url: "https://www.youtube.com/watch?v=abcdefghijk".into(),
        ..Default::default()
    }
}
async fn settle(app: &mut App) {
    while let Some(event) = tokio::time::timeout(Duration::from_secs(3), app.tasks.join_next())
        .await
        .unwrap()
    {
        app.apply(event.unwrap());
    }
}
fn render(app: &mut App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|f| ui::render(f, app)).unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect()
}
#[tokio::test]
async fn saved_shelf_is_available_offline_and_search_never_calls_youtube() {
    let (mut app, _rx, _temp) = fixture();
    app.tab = 1;
    app.tabs[1].page = Page {
        items: vec![media()],
        ..Default::default()
    };
    app.action(Action::SaveMedia);
    assert_eq!(app.snapshot.saved_media.entries.len(), 1);
    app.command("saved").unwrap();
    settle(&mut app).await;
    assert_eq!(app.tabs[1].page.items[0].title, "Saved fixture");
    assert!(!app.browser.authenticated());
    app.search("missing");
    settle(&mut app).await;
    assert!(app.tabs[1].page.items.is_empty());
    app.search("ARTIST");
    settle(&mut app).await;
    assert_eq!(app.tabs[1].page.items.len(), 1);
    app.action(Action::SaveMedia);
    settle(&mut app).await;
    assert!(app.snapshot.saved_media.entries.is_empty());
    assert!(app.tabs[1].page.items.is_empty());
    app.shutdown().await;
}
#[tokio::test]
async fn saved_shortcut_can_be_remapped_and_does_not_fire_in_text_inputs() {
    let (mut app, _rx, _temp) = fixture();
    app.tab = 1;
    app.tabs[1].page = Page {
        items: vec![media()],
        ..Default::default()
    };
    app.key(KeyCode::Char('S').into());
    assert_eq!(app.snapshot.saved_media.entries.len(), 1);
    app.key(KeyCode::Char('S').into());
    assert!(app.snapshot.saved_media.entries.is_empty());
    app.config
        .keybindings
        .insert("f8".into(), "save_media".into());
    app.key(KeyCode::F(8).into());
    assert_eq!(app.snapshot.saved_media.entries.len(), 1);
    app.key(KeyCode::Char('/').into());
    app.key(KeyCode::Char('S').into());
    assert_eq!(app.snapshot.saved_media.entries.len(), 1);
    app.key(KeyCode::Esc.into());
    app.key(KeyCode::F(8).into());
    assert!(app.snapshot.saved_media.entries.is_empty());
    app.shutdown().await;
}
#[tokio::test]
async fn account_disconnect_keeps_the_local_saved_shelf() {
    let (mut app, _rx, _temp) = fixture();
    app.snapshot.saved_media.toggle(media()).unwrap();
    app.command("disconnect").unwrap();
    settle(&mut app).await;
    assert_eq!(app.snapshot.saved_media.entries.len(), 1);
    app.command("saved").unwrap();
    settle(&mut app).await;
    assert_eq!(app.tabs[1].page.items.len(), 1);
    app.shutdown().await;
}
#[tokio::test]
async fn sleep_expiry_retries_a_busy_player_and_suppresses_eof_queue_advance() {
    let (mut app, mut rx, _temp) = fixture();
    app.snapshot.playback.play(media());
    app.snapshot.playback.push(media());
    app.player.try_send(PlayerCommand::TogglePause).unwrap();
    app.player.try_send(PlayerCommand::TogglePause).unwrap();
    let now = Instant::now();
    app.sleep_timer.arm(Duration::from_secs(1), now).unwrap();
    assert!(app.enforce_sleep_timer(now + Duration::from_secs(1)));
    assert!(app.sleep_timer.stopping());
    assert!(app.snapshot.playback.current.is_none());
    app.playback_ended(&media());
    assert!(!app.play_existing(media()));
    rx.try_recv().unwrap();
    rx.try_recv().unwrap();
    app.tick().await;
    assert!(matches!(rx.try_recv().unwrap(), PlayerCommand::Stop));
    assert!(rx.try_recv().is_err());
    assert_eq!(app.snapshot.playback.items.len(), 2);
    assert!(app.snapshot.playback.current.is_none());
    assert!(app.sleep_timer.remaining(Instant::now()).is_none());
    assert!(!app.quit);
    app.offline = false;
    app.tab = 1;
    app.tabs[1].page = Page {
        items: vec![media()],
        ..Default::default()
    };
    for action in [Action::Preview, Action::Fallback] {
        app.sleep_timer
            .arm(
                Duration::from_secs(1),
                Instant::now() - Duration::from_secs(2),
            )
            .unwrap();
        app.action(action);
        assert!(matches!(rx.try_recv().unwrap(), PlayerCommand::Stop));
        assert!(
            rx.try_recv().is_err(),
            "Expired timer allowed another player launch"
        );
    }
    app.shutdown().await;
}
#[tokio::test]
async fn sleep_expiry_is_checked_when_eof_arrives_before_the_next_tick() {
    let (mut app, mut rx, _temp) = fixture();
    app.snapshot.playback.play(media());
    app.snapshot.playback.push(media());
    app.sleep_timer
        .arm(
            Duration::from_secs(1),
            Instant::now() - Duration::from_secs(2),
        )
        .unwrap();
    app.playback_ended(&media());
    assert!(matches!(rx.try_recv().unwrap(), PlayerCommand::Stop));
    assert!(rx.try_recv().is_err());
    assert!(app.snapshot.playback.current.is_none());
    app.shutdown().await;
}
#[tokio::test]
async fn cancel_timer_allows_queue_advance_and_invalid_duration_keeps_the_draft() {
    let (mut app, mut rx, _temp) = fixture();
    app.offline = false;
    app.command("sleep 30m").unwrap();
    let index = ytui::actions::ENTRIES
        .iter()
        .position(|e| e.title == "Set sleep timer")
        .unwrap();
    app.open_action(index);
    if let Some(Modal::Form(form)) = &mut app.modal {
        form.fields[0].value = ytui::editor::Editor::new("25h");
    }
    app.key(KeyCode::Enter.into());
    assert!(matches!(&app.modal,Some(Modal::Form(form)) if form.error.is_some()));
    assert!(app.sleep_timer.remaining(Instant::now()).is_some());
    app.key(KeyCode::Esc.into());
    app.command("sleep off").unwrap();
    app.snapshot.playback.play(media());
    app.snapshot.playback.push(media());
    app.playback_ended(&media());
    assert!(matches!(rx.try_recv().unwrap(), PlayerCommand::Play { .. }));
    assert_eq!(app.snapshot.playback.current, Some(1));
    app.shutdown().await;
}
#[tokio::test]
async fn a_closed_player_reports_failure_without_claiming_a_successful_stop() {
    let (mut app, rx, _temp) = fixture();
    drop(rx);
    app.sleep_timer
        .arm(
            Duration::from_secs(1),
            Instant::now() - Duration::from_secs(2),
        )
        .unwrap();
    app.enforce_sleep_timer(Instant::now());
    assert!(app.status.contains("could not reach player"));
    assert!(app.sleep_timer.remaining(Instant::now()).is_none());
    app.shutdown().await;
}
#[tokio::test]
async fn saved_and_sleep_panels_render_across_themes_languages_and_sizes() {
    let (mut app, _rx, _temp) = fixture();
    app.snapshot.saved_media.toggle(media()).unwrap();
    let arm = ytui::actions::ENTRIES
        .iter()
        .position(|e| e.title == "Set sleep timer")
        .unwrap();
    for language in ["en", "de"] {
        app.config.language = language.into();
        for theme in ytui_services::theme::THEMES {
            app.palette = ytui_services::theme::builtin(theme).unwrap();
            for (w, h) in [(212, 58), (160, 45), (100, 30), (60, 20), (35, 12)] {
                app.modal = None;
                app.command("saved").unwrap();
                settle(&mut app).await;
                app.command("sleep 30m").unwrap();
                let text = render(&mut app, w, h);
                assert!(!text.trim().is_empty());
                if w >= 100 {
                    assert!(text.contains("Saved fixture"));
                }
                app.open_action(arm);
                assert!(matches!(app.modal, Some(Modal::Form(_))));
                let text = render(&mut app, w, h);
                if w >= 100 {
                    assert!(text.contains(app.tr("Set sleep timer")));
                    assert!(
                        !text.contains("Playlist indices:"),
                        "Sleep form showed download-only help"
                    );
                    assert!(text.contains("24h"));
                }
                app.modal = None;
                app.command("sleep").unwrap();
                let text = render(&mut app, w, h);
                if w >= 100 {
                    assert!(text.contains(app.tr("Sleep timer")));
                }
            }
        }
    }
    app.shutdown().await;
}

#[tokio::test]
#[ignore = "requires installed mpv and FFmpeg; synthetic local audio and null outputs only"]
async fn real_mpv_sleep_timer_stops_audio_without_advancing_the_queue() {
    use std::os::unix::fs::PermissionsExt;
    use ytui_services::player::Player;
    let (mut app, rx, temp) = fixture();
    drop(rx);
    let source = temp.path().join("tone.wav");
    let output = tokio::process::Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440",
            "-t",
            "10",
        ])
        .arg(&source)
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mpv = std::env::var("YTUI_TEST_MPV").unwrap_or_else(|_| "mpv".into());
    let wrapper = temp.path().join("mpv-null");
    std::fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nexec '{}' --ao=null --vo=null \"$@\"\n",
            mpv.replace('\'', "'\\''")
        ),
    )
    .unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut config = app.config.clone();
    config.mpv = wrapper.to_string_lossy().into();
    let mut player = Player::spawn(config, app.paths.clone(), None);
    app.player = player.commands.clone();
    let result = tokio::time::timeout(Duration::from_secs(15), async {
        app.play(Media {
            url: source.to_string_lossy().into(),
            kind: ytui_core::MediaKind::Song,
            ..media()
        });
        loop {
            let status = player.status.borrow_and_update().clone();
            if let Some(error) = status.error {
                anyhow::bail!(error);
            }
            if status.active && status.position >= 0.2 {
                break;
            }
            player.status.changed().await?;
        }
        app.command("sleep 1s")?;
        loop {
            app.tick().await;
            let status = player.status.borrow().clone();
            if let Some(error) = status.error {
                anyhow::bail!(error);
            }
            if !status.active && app.sleep_timer.remaining(Instant::now()).is_none() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
        anyhow::ensure!(
            app.snapshot.playback.current.is_none(),
            "Timer advanced or retained a playing queue entry"
        );
        anyhow::ensure!(
            app.snapshot.playback.items.len() == 1,
            "Timer erased or grew the queue"
        );
        Ok::<(), anyhow::Error>(())
    })
    .await;
    app.shutdown().await;
    player.close().await;
    result.expect("Actual mpv timer deadline").unwrap();
}

#[tokio::test]
async fn saved_video_keeps_video_intent_even_when_the_music_tab_is_visible() {
    let (mut app, mut rx, _temp) = fixture();
    app.offline = false;
    app.tab = 2;
    app.tabs[2].request = Some(ytui_core::BrowseRequest::SavedMedia {
        query: String::new(),
    });
    app.play(media());
    assert!(matches!(
        rx.try_recv().unwrap(),
        PlayerCommand::Play { audio: false, .. }
    ));
    let mut song = media();
    song.kind = ytui_core::MediaKind::Song;
    app.play(song);
    assert!(matches!(
        rx.try_recv().unwrap(),
        PlayerCommand::Play { audio: true, .. }
    ));
    app.shutdown().await;
}

#[tokio::test]
async fn saved_intent_is_shared_by_enqueue_download_recent_folders_and_explicit_fallback() {
    let (mut app, mut rx, temp) = fixture();
    app.offline = false;
    app.tab = 2;
    app.tabs[2].request = Some(ytui_core::BrowseRequest::SavedMedia {
        query: String::new(),
    });
    app.tabs[2].page = Page {
        items: vec![media()],
        ..Default::default()
    };
    app.action(Action::Enqueue);
    assert_eq!(
        app.snapshot.playback.items[0].kind,
        ytui_core::MediaKind::Video
    );
    app.action(Action::Download);
    assert!(
        matches!(&app.modal,Some(Modal::Form(f)) if matches!(&f.kind,ytui::forms::FormKind::Download(_,o) if !o.audio))
    );
    app.snapshot.recent_destinations.push(temp.path().into());
    app.modal = Some(Modal::Destinations(0));
    app.key(KeyCode::Enter.into());
    assert!(
        matches!(&app.modal,Some(Modal::Form(f)) if matches!(&f.kind,ytui::forms::FormKind::Download(_,o) if !o.audio))
    );
    app.modal = None;
    app.action(Action::Fallback);
    assert!(matches!(
        rx.try_recv().unwrap(),
        PlayerCommand::PlayFallback { audio: false, .. }
    ));
    app.tab = 1;
    app.tabs[1].request = Some(ytui_core::BrowseRequest::SavedMedia {
        query: String::new(),
    });
    let mut song = media();
    song.kind = ytui_core::MediaKind::Song;
    app.tabs[1].page = Page {
        items: vec![song],
        ..Default::default()
    };
    app.action(Action::Download);
    assert!(
        matches!(&app.modal,Some(Modal::Form(f)) if matches!(&f.kind,ytui::forms::FormKind::Download(_,o) if o.audio))
    );
    app.shutdown().await;
}

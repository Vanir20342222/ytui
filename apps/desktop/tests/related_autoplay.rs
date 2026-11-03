//! Related-video continuation after the queue (parity with Android's
//! "Related media autoplay" and watch-page Up next). Synthetic data only: the
//! fixture routes HTTP to a closed local proxy, so no public service is contacted.
use crossterm::event::KeyCode;
use ratatui::{Terminal, backend::TestBackend};
use ratatui_image::picker::Picker;
use tokio::sync::mpsc;
use ytui::{
    app::{App, Modal, RuntimeHandles, UiEvent},
    ui,
};
use ytui_core::{Config, Media, MediaKind, Page, shortcuts::Action};
use ytui_services::{paths::Paths, player::PlayerCommand, store::Snapshot};

fn fixture(autoplay: bool) -> (App, mpsc::Receiver<PlayerCommand>, tempfile::TempDir) {
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
        // Closed local port: a stray fetch fails locally instead of reaching YouTube.
        proxy: Some("http://127.0.0.1:9".into()),
        autoplay_related: autoplay,
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
    app.demo = false;
    (app, rx, temp)
}

fn video(id: &str, title: &str) -> Media {
    Media {
        id: id.into(),
        title: title.into(),
        creator: "Synthetic".into(),
        url: format!("https://www.youtube.com/watch?v={id}"),
        kind: MediaKind::Video,
        ..Default::default()
    }
}

fn related_page() -> Page {
    let mut playlist = video("ppppppppppp", "A playlist");
    playlist.browse_id = Some("VLsynthetic".into());
    Page {
        items: vec![
            video("sssssssssss", "The source again"),
            playlist,
            video("rrrrrrrrrr1", "First recommendation"),
            video("rrrrrrrrrr2", "Second recommendation"),
        ],
        ..Default::default()
    }
}

/// Plays `source` as the only queue entry and resolves its related list.
fn start_single(app: &mut App, source: &Media) {
    app.snapshot.playback.play(source.clone());
    app.ensure_related(false);
    let pending = app.related.as_ref().expect("related list requested");
    assert!(pending.loading && pending.source == source.url);
    app.apply(UiEvent::Related {
        generation: app.related_generation(),
        result: Ok(related_page()),
    });
}

fn played(rx: &mut mpsc::Receiver<PlayerCommand>) -> Vec<String> {
    let mut urls = Vec::new();
    while let Ok(command) = rx.try_recv() {
        match command {
            PlayerCommand::Play { media, .. } | PlayerCommand::PlayAt { media, .. } => {
                urls.push(media.url)
            }
            _ => {}
        }
    }
    urls
}

#[tokio::test]
async fn queue_end_continues_with_related_videos_and_skips_collections_and_the_source() {
    let (mut app, mut rx, _temp) = fixture(true);
    let source = video("sssssssssss", "Source");
    start_single(&mut app, &source);
    let related = app.up_next_related().expect("shown after the queue");
    let titles = related
        .items
        .iter()
        .map(|m| m.title.as_str())
        .collect::<Vec<_>>();
    assert_eq!(titles, ["First recommendation", "Second recommendation"]);
    // A late duplicate result for the same request is ignored.
    app.apply(UiEvent::Related {
        generation: app.related_generation(),
        result: Err(anyhow::anyhow!("late failure")),
    });
    assert_eq!(app.related.as_ref().unwrap().items.len(), 2);

    app.playback_ended(&source);
    let queue = &app.snapshot.playback;
    assert_eq!(
        queue.items.len(),
        2,
        "only the entry that plays joins the queue"
    );
    assert_eq!(queue.current, Some(1));
    assert_eq!(queue.items[1].title, "First recommendation");
    assert_eq!(played(&mut rx), [video("rrrrrrrrrr1", "").url]);
    app.shutdown().await;
}

#[tokio::test]
async fn next_at_queue_end_takes_the_first_recommendation_or_reports_the_end() {
    let (mut app, mut rx, _temp) = fixture(true);
    let source = video("sssssssssss", "Source");
    start_single(&mut app, &source);
    app.action(Action::NextTrack);
    assert_eq!(app.snapshot.playback.current, Some(1));
    assert_eq!(played(&mut rx), [video("rrrrrrrrrr1", "").url]);

    // Next is a no-op (with its usual status) when nothing is recommended.
    let (mut app, mut rx, _temp) = fixture(true);
    app.snapshot.playback.play(source.clone());
    app.action(Action::NextTrack);
    assert_eq!(app.snapshot.playback.current, Some(0));
    assert!(app.status.starts_with("End of queue"));
    assert!(played(&mut rx).is_empty());
    app.shutdown().await;
}

#[tokio::test]
async fn autoplay_off_shows_recommendations_but_never_plays_them_automatically() {
    let (mut app, mut rx, _temp) = fixture(false);
    let source = video("sssssssssss", "Source");
    app.snapshot.playback.play(source.clone());
    app.ensure_related(false);
    assert!(
        app.related.is_none(),
        "no background fetch while autoplay is off"
    );
    // Opening the queue fetches a display-only list, like Android's Up next.
    app.action(Action::PlaybackQueue);
    assert!(app.related.as_ref().is_some_and(|r| r.loading));
    app.apply(UiEvent::Related {
        generation: app.related_generation(),
        result: Ok(related_page()),
    });
    assert_eq!(app.up_next_related().unwrap().items.len(), 2);
    app.playback_ended(&source);
    assert_eq!(app.snapshot.playback.current, None);
    assert!(played(&mut rx).is_empty());
    app.shutdown().await;
}

#[tokio::test]
async fn a_list_that_arrives_after_the_video_ended_still_continues_playback() {
    let (mut app, mut rx, _temp) = fixture(true);
    let source = video("sssssssssss", "Short");
    app.snapshot.playback.play(source.clone());
    app.ensure_related(false);
    app.playback_ended(&source);
    assert_eq!(app.snapshot.playback.current, None);
    app.apply(UiEvent::Related {
        generation: app.related_generation(),
        result: Ok(related_page()),
    });
    assert_eq!(app.snapshot.playback.current, Some(1));
    assert_eq!(played(&mut rx), [video("rrrrrrrrrr1", "").url]);
    app.shutdown().await;
}

#[tokio::test]
async fn later_queue_entries_win_and_local_or_offline_media_never_fetch() {
    let (mut app, _rx, _temp) = fixture(true);
    app.snapshot.playback.play(video("aaaaaaaaaaa", "First"));
    app.snapshot.playback.push(video("bbbbbbbbbbb", "Second"));
    app.ensure_related(false);
    assert!(app.related.is_none(), "the queue still has a next entry");
    app.snapshot.playback.items.clear();
    app.snapshot.playback.play(Media {
        url: "/synthetic/local.mp4".into(),
        kind: MediaKind::Video,
        ..Default::default()
    });
    app.ensure_related(false);
    assert!(app.related.is_none(), "local files have no related list");
    app.snapshot.playback.play(video("ccccccccccc", "Offline"));
    app.offline = true;
    app.ensure_related(true);
    assert!(app.related.is_none(), "offline mode never fetches");
    app.shutdown().await;
}

#[tokio::test]
async fn queue_popup_lists_recommendations_after_the_queue_and_plays_or_removes_them() {
    let (mut app, mut rx, _temp) = fixture(true);
    let source = video("sssssssssss", "Source video");
    start_single(&mut app, &source);
    app.action(Action::PlaybackQueue);
    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    terminal.draw(|f| ui::render(f, &mut app)).unwrap();
    let screen = format!("{:?}", terminal.backend().buffer());
    for text in [
        "Source video",
        "Recommended · plays when the queue ends",
        "» First recommendation",
        "Second recommendation",
    ] {
        assert!(screen.contains(text), "missing {text}");
    }
    // Down to the second recommendation (index 2): Del removes it from the list only.
    app.key(KeyCode::Down.into());
    app.key(KeyCode::Down.into());
    assert!(matches!(app.modal, Some(Modal::Queue(2))));
    app.key(KeyCode::Delete.into());
    assert_eq!(app.up_next_related().unwrap().items.len(), 1);
    assert_eq!(app.snapshot.playback.items.len(), 1);
    assert!(matches!(app.modal, Some(Modal::Queue(1))));
    // Enter plays the first recommendation and adds it to the queue.
    app.key(KeyCode::Enter.into());
    assert_eq!(app.snapshot.playback.current, Some(1));
    assert_eq!(app.snapshot.playback.items[1].title, "First recommendation");
    assert_eq!(played(&mut rx), [video("rrrrrrrrrr1", "").url]);
    app.shutdown().await;
}

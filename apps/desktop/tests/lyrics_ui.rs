use crossterm::event::KeyCode;
use ratatui::{Terminal, backend::TestBackend};
use ratatui_image::picker::Picker;
use tokio::sync::mpsc;
use ytui::{
    app::{App, Modal, RuntimeHandles, UiEvent},
    lyrics::View,
    ui,
};
use ytui_core::Config;
use ytui_services::{
    lyrics::{Track, parse},
    paths::Paths,
    store::Snapshot,
};

fn fixture() -> (App, tempfile::TempDir) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let paths = Paths {
        config_override: None,
        config: root.join("config"),
        state: root.join("state"),
        cache: root.join("cache"),
        xdg_state: root.join("xdg-state"),
        legacy_data: root.join("legacy"),
        home: root.into(),
    };
    paths.create().unwrap();
    let (player, _) = mpsc::channel(32);
    let (jobs, _) = mpsc::channel(32);
    let app = App::new(
        Config {
            auth_store: "file".into(),
            notifications: false,
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
        true,
        true,
    )
    .unwrap();
    (app, temp)
}
fn track() -> Track {
    Track {
        title: "Settings".into(),
        artist: "Fixture artist".into(),
        album: "Fixture album".into(),
        duration: 90.0,
        instrumental: false,
        lines: parse("[00:01]First line\n[00:03]Grüße 🎵\n[00:05]Last line").unwrap(),
    }
}
fn text(app: &mut App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| ui::render(frame, app)).unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect()
}
#[tokio::test]
async fn lyrics_results_cannot_reopen_closed_or_replace_newer_panel() {
    let (mut app, _temp) = fixture();
    let view = View::new(None, "LRCLIB");
    let old_id = view.id;
    let cancel = view.cancel.clone();
    app.modal = Some(Modal::Lyrics(Box::new(view)));
    app.key(KeyCode::Esc.into());
    assert!(cancel.is_cancelled());
    app.apply(UiEvent::Lyrics {
        id: old_id,
        result: Ok(vec![track()]),
    });
    assert!(app.modal.is_none());
    let view = View::new(None, "LRCLIB");
    let new_id = view.id;
    app.modal = Some(Modal::Lyrics(Box::new(view)));
    app.apply(UiEvent::Lyrics {
        id: old_id,
        result: Ok(vec![track()]),
    });
    assert!(
        matches!(&app.modal, Some(Modal::Lyrics(view)) if view.loading && view.tracks.is_empty())
    );
    app.apply(UiEvent::Lyrics {
        id: new_id,
        result: Ok(vec![track()]),
    });
    app.key(KeyCode::Enter.into());
    assert!(matches!(&app.modal, Some(Modal::Lyrics(view)) if view.opened));
    app.key(KeyCode::Down.into());
    assert!(matches!(&app.modal, Some(Modal::Lyrics(view)) if !view.follow && view.offset == 1));
    app.key(KeyCode::Char('f').into());
    assert!(matches!(&app.modal, Some(Modal::Lyrics(view)) if view.follow));
    app.key(KeyCode::Backspace.into());
    assert!(matches!(&app.modal, Some(Modal::Lyrics(view)) if !view.opened));
    app.shutdown().await;
}
#[tokio::test]
async fn local_lyrics_work_offline_and_lookup_requires_explicit_online_review() {
    let (mut app, temp) = fixture();
    while let Some(event) = app.tasks.join_next().await {
        app.apply(event.unwrap());
    }
    assert!(app.command("lyrics-find 'Song' 'Artist'").is_err());
    assert!(app.tasks.is_empty());
    let file = temp.path().join("Song with spaces.lrc");
    std::fs::write(&file, "[00:01]Offline words").unwrap();
    app.command(&format!(
        "lyrics-local {}",
        shell_words::quote(file.to_str().unwrap())
    ))
    .unwrap();
    let event = app.tasks.join_next().await.unwrap().unwrap();
    app.apply(event);
    assert!(text(&mut app, 100, 30).contains("Offline words"));
    let entry = ytui::actions::ENTRIES
        .iter()
        .position(|entry| entry.command == "lyrics-find")
        .unwrap();
    app.open_action(entry);
    let rendered = text(&mut app, 120, 40);
    assert!(rendered.contains("sent to LRCLIB"));
    assert!(rendered.contains("No cookies or history"));
    assert!(
        app.tasks.is_empty(),
        "Opening review must never perform a request"
    );
    // A form can remain open while the player advances; its reviewed song
    // identity must not silently become the next song.
    app.player_status.media = Some(ytui_core::Media {
        url: "fixture-A".into(),
        ..Default::default()
    });
    let entry = ytui::actions::ENTRIES
        .iter()
        .position(|entry| entry.command == "lyrics-local")
        .unwrap();
    app.open_action(entry);
    let Some(Modal::Form(form)) = &mut app.modal else {
        panic!("Expected review form")
    };
    form.fields[0].value = ytui::editor::Editor::new(file.to_str().unwrap());
    app.player_status.media.as_mut().unwrap().url = "fixture-B".into();
    app.key(KeyCode::Enter.into());
    assert!(
        matches!(&app.modal, Some(Modal::Lyrics(view)) if view.media_url.as_deref() == Some("fixture-A"))
    );
    app.shutdown().await;
}
#[tokio::test]
async fn lyrics_all_states_render_in_every_theme_size_and_language_without_translating_content() {
    let (mut app, _temp) = fixture();
    for language in ["en", "de"] {
        app.config.language = language.into();
        for theme in ytui_services::theme::THEMES {
            app.palette = ytui_services::theme::builtin(theme).unwrap();
            for (width, height) in [(35, 12), (80, 24), (120, 40), (180, 50), (212, 58)] {
                for state in 0..5 {
                    let mut view = View::new(Some("fixture-A".into()), "LRCLIB");
                    view.loading = state == 0;
                    view.opened = state >= 3;
                    if state >= 2 {
                        view.tracks.push(track());
                    }
                    if state == 4 {
                        view.error = Some("Fixture failure".into());
                    }
                    app.modal = Some(Modal::Lyrics(Box::new(view)));
                    let rendered = text(&mut app, width, height);
                    assert!(rendered.contains(if language == "de" {
                        "Liedtext"
                    } else {
                        "Lyrics"
                    }));
                    if state == 2 {
                        assert!(
                            rendered.contains("Settings"),
                            "Media titles must stay unchanged"
                        );
                    }
                    assert_eq!(app.image_area.width, 0);
                }
            }
        }
    }
    // A timestamp is highlighted only for the bound playing media, never another track.
    let mut view = View::new(Some("fixture-A".into()), "Local file");
    view.loading = false;
    view.opened = true;
    view.tracks.push(track());
    app.modal = Some(Modal::Lyrics(Box::new(view)));
    let mut media = ytui_core::Media {
        title: "Song".into(),
        url: "fixture-B".into(),
        ..Default::default()
    };
    app.player_status.media = Some(media.clone());
    app.player_status.position = 3.1;
    let rendered = text(&mut app, 120, 40);
    assert!(rendered.contains("Synchronisierung pausiert"));
    media.url = "fixture-A".into();
    app.player_status.media = Some(media);
    let rendered = text(&mut app, 120, 40);
    assert!(rendered.contains("› Grüße"));
    app.shutdown().await;
}

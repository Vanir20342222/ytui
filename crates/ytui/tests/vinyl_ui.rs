use ratatui::{Terminal, backend::TestBackend};
use ratatui_image::picker::Picker;
use tokio::sync::mpsc;
use ytui::{
    app::{App, RuntimeHandles},
    editor::Editor,
    forms::Form,
    ui,
};
use ytui_core::{Config, shortcuts::Action};
use ytui_services::{paths::Paths, store::Snapshot};
fn fixture() -> (App, tempfile::TempDir) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let paths = Paths {
        config_override: None,
        xdg_state: root.join("xdg-state"),
        config: root.join("config"),
        state: root.join("state"),
        cache: root.join("cache"),
        legacy_data: root.join("legacy"),
        home: root.into(),
    };
    paths.create().unwrap();
    let mut config = Config {
        theme: "ytui-dark".into(),
        auth_store: "file".into(),
        notifications: false,
        ..Default::default()
    };
    paths.fill_directories(&mut config);
    let (player, _) = mpsc::channel(32);
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
    (app, temp)
}

fn render(app: &mut App, w: u16, h: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(|f| ui::render(f, app)).unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect()
}
#[test]
fn legacy_config_and_settings_style_roundtrip_and_validate() {
    let c: Config = toml::from_str("theme='ytui-dark'").unwrap();
    assert_eq!(c.music_player_style, "classic");
    let mut f = Form::settings(c);
    for style in ["classic", "vinyl", "vinyl-fullscreen"] {
        f.fields[55].value = Editor::new(style);
        let c = f.config_values().unwrap();
        assert_eq!(
            toml::from_str::<Config>(&toml::to_string(&c).unwrap())
                .unwrap()
                .music_player_style,
            style
        );
    }
    f.fields[55].value = Editor::new("invalid");
    assert!(f.config_values().is_err());
}
#[tokio::test]
async fn modes_are_scoped_to_music_radio_and_escape_keeps_playback() {
    let (mut app, _temp) = fixture();
    for tab in [0, 1] {
        app.tab = tab;
        app.action(Action::MusicPlayerStyle);
        assert_eq!(app.config.music_player_style, "classic");
    }
    app.tab = 2;
    app.action(Action::MusicPlayerStyle);
    assert_eq!(app.config.music_player_style, "vinyl");
    app.action(Action::MusicPlayerStyle);
    assert_eq!(app.config.music_player_style, "vinyl-fullscreen");
    app.action(Action::Back);
    assert_eq!(app.config.music_player_style, "vinyl");
    assert!(app.player_status.active);
    app.tab = 3;
    assert!(app.vinyl_enabled());
    for tab in [0, 1] {
        app.tab = tab;
        assert!(!app.vinyl_enabled());
        assert!(!render(&mut app, 120, 40).contains("Vinyl"));
    }
}
#[tokio::test]
async fn all_themes_sizes_modes_and_modal_overlays_render_playing_track() {
    let (mut app, _temp) = fixture();
    app.player_status.media.as_mut().unwrap().title = "Currently playing fixture".into();
    for theme in ytui_services::theme::THEMES {
        app.palette = ytui_services::theme::builtin(theme).unwrap();
        for tab in [2, 3] {
            app.tab = tab;
            for style in ["vinyl", "vinyl-fullscreen"] {
                app.config.music_player_style = style.into();
                for (w, h) in [(35, 12), (80, 24), (120, 40), (212, 58)] {
                    let text = render(&mut app, w, h);
                    assert!(text.contains("Vinyl"), "{theme} {style} {w}x{h}");
                    if w >= 80 {
                        assert!(text.contains("Currently playing fixture"));
                    }
                    assert!(app.image_area.right() <= w && app.image_area.bottom() <= h);
                }
            }
        }
    }
    app.modal = Some(ytui::app::Modal::Help(0));
    assert!(render(&mut app, 120, 40).contains("Keyboard & navigation"));
}
#[tokio::test]
async fn rotation_stays_still_for_pause_buffer_stop_reduced_motion_and_other_tabs() {
    let (mut app, _temp) = fixture();
    app.tab = 2;
    app.config.music_player_style = "vinyl".into();
    app.tick().await;
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    app.tick().await;
    assert!(app.vinyl_motion.phase > 0.0);
    for mode in 0..5 {
        app.player_status.active = mode != 0;
        app.player_status.paused = mode == 1;
        app.player_status.buffering = mode == 2;
        app.config.reduced_motion = mode == 3;
        app.tab = if mode == 4 { 1 } else { 2 };
        app.tick().await;
        let phase = app.vinyl_motion.phase;
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        app.tick().await;
        assert_eq!(phase, app.vinyl_motion.phase);
    }
}

#[tokio::test]
async fn artwork_is_cached_for_the_playing_song_independently_of_browse_selection() {
    let (mut app, _temp) = fixture();
    app.tab = 2;
    app.config.music_player_style = "vinyl-fullscreen".into();
    render(&mut app, 120, 40);
    app.refresh_image();
    while let Some(event) =
        tokio::time::timeout(std::time::Duration::from_secs(5), app.tasks.join_next())
            .await
            .unwrap()
    {
        app.apply(event.unwrap());
    }
    assert!(app.image.is_some());
    assert!(app.image_matches("demo://art/1"));
    app.tabs[2].selected = 5;
    render(&mut app, 120, 40);
    app.refresh_image();
    assert!(app.image_matches("demo://art/1"));
    assert!(!app.image_matches("demo://art/5"));
    assert!(
        app.tasks.is_empty(),
        "Browsing caused a new playing-art request"
    );
}

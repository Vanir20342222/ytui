//! First-run setup (parity with Android onboarding). Synthetic state only; HTTP is
//! routed to a closed local proxy so startup feeds fail locally and never reach YouTube.
use crossterm::event::KeyCode;
use ratatui::{Terminal, backend::TestBackend};
use ratatui_image::picker::Picker;
use tokio::sync::mpsc;
use ytui::{
    app::{App, Modal, RuntimeHandles},
    setup, ui,
};
use ytui_core::Config;
use ytui_services::{paths::Paths, store::Snapshot, theme};

fn fixture(snapshot: Snapshot, demo: bool, offline: bool) -> (App, tempfile::TempDir) {
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
    let (player, _) = mpsc::channel(32);
    let (jobs, _) = mpsc::channel(32);
    let app = App::new(
        config,
        paths,
        snapshot,
        None,
        RuntimeHandles {
            player,
            jobs,
            picker: Picker::from_fontsize((8, 16)),
        },
        demo,
        offline,
    )
    .unwrap();
    (app, temp)
}

fn step(app: &App) -> usize {
    match &app.modal {
        Some(Modal::Setup(setup)) => setup.step,
        _ => panic!("setup is not open"),
    }
}

fn screen(app: &mut App, cols: u16, rows: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(cols, rows)).unwrap();
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
async fn setup_opens_once_per_version_and_never_in_demo_or_offline_mode() {
    let (mut app, _temp) = fixture(Snapshot::default(), false, false);
    assert_eq!(step(&app), setup::WELCOME);
    app.shutdown().await;
    let done = Snapshot {
        setup_version: setup::VERSION,
        ..Default::default()
    };
    let (mut app, _temp) = fixture(done, false, false);
    assert!(app.modal.is_none(), "finished setup is not shown again");
    app.shutdown().await;
    for (demo, offline) in [(true, false), (false, true)] {
        let (mut app, _temp) = fixture(Snapshot::default(), demo, offline);
        assert!(app.modal.is_none(), "demo={demo} offline={offline}");
        app.shutdown().await;
    }
}

#[tokio::test]
async fn skip_records_completion_and_keeps_the_settings_unchanged() {
    let (mut app, _temp) = fixture(Snapshot::default(), false, false);
    let before = toml::to_string(&app.config).unwrap();
    let palette = app.palette.clone();
    app.key(KeyCode::Enter.into());
    assert_eq!(step(&app), setup::LOOK);
    // Preview another theme, then return to the original one and skip.
    app.key(KeyCode::Right.into());
    assert_ne!(app.palette, palette, "theme previews immediately");
    app.key(KeyCode::Left.into());
    app.key(KeyCode::Esc.into());
    assert!(app.modal.is_none());
    assert_eq!(app.snapshot.setup_version, setup::VERSION);
    assert!(app.dirty, "completion is saved with the state snapshot");
    assert_eq!(
        app.palette, palette,
        "an unchanged theme restores the palette"
    );
    assert_eq!(toml::to_string(&app.config).unwrap(), before);
    app.shutdown().await;
}

#[tokio::test]
async fn finishing_applies_the_drafted_choices_and_replay_starts_over() {
    // Demo mode applies settings without writing a configuration file.
    let (mut app, _temp) = fixture(Snapshot::default(), true, false);
    app.command("setup").unwrap();
    assert_eq!(step(&app), setup::WELCOME);
    app.key(KeyCode::Enter.into());
    app.key(KeyCode::Char(' ').into());
    let theme = match &app.modal {
        Some(Modal::Setup(s)) => s.draft.theme.clone(),
        _ => unreachable!(),
    };
    assert_ne!(theme, "ytui-dark");
    app.key(KeyCode::Down.into());
    app.key(KeyCode::Char(' ').into()); // reduce motion on
    app.key(KeyCode::Enter.into());
    assert_eq!(step(&app), setup::PLAYBACK);
    app.key(KeyCode::Char(' ').into()); // related autoplay off
    app.key(KeyCode::Backspace.into());
    assert_eq!(step(&app), setup::LOOK, "Back keeps the draft");
    app.key(KeyCode::Enter.into());
    app.key(KeyCode::Enter.into());
    assert_eq!(step(&app), setup::ACCOUNT);
    app.key(KeyCode::Enter.into()); // maybe later
    assert_eq!(step(&app), setup::READY);
    let text = screen(&mut app, 120, 40);
    assert!(text.contains("You're all set") && text.contains("Not signed in"));
    app.key(KeyCode::Enter.into());
    assert!(app.modal.is_none());
    assert_eq!(app.config.theme, theme);
    assert!(app.config.reduced_motion && !app.config.autoplay_related);
    assert_eq!(app.palette, theme::builtin(&theme).unwrap());
    assert_eq!(app.snapshot.setup_version, setup::VERSION);
    // Replay from the action menu starts at the welcome step with current settings.
    app.command("setup").unwrap();
    assert_eq!(step(&app), setup::WELCOME);
    app.shutdown().await;
}

#[tokio::test]
async fn account_step_continues_into_the_connect_flow() {
    let (mut app, _temp) = fixture(Snapshot::default(), true, false);
    app.open_setup();
    for _ in 0..3 {
        app.key(KeyCode::Enter.into());
    }
    assert_eq!(step(&app), setup::ACCOUNT);
    app.key(KeyCode::Char(' ').into());
    assert_eq!(app.snapshot.setup_version, setup::VERSION);
    assert!(
        matches!(&app.modal, Some(Modal::Form(form)) if form.title() == "Connect your YouTube account")
    );
    app.shutdown().await;
}

#[tokio::test]
async fn every_step_renders_in_all_themes_and_supported_sizes() {
    let (mut app, _temp) = fixture(Snapshot::default(), true, false);
    for name in theme::THEMES {
        if let Ok(palette) = theme::builtin(name) {
            app.palette = palette;
        }
        for (cols, rows) in [(35, 12), (80, 24), (120, 40), (212, 58)] {
            app.open_setup();
            for index in 0..setup::TITLES.len() {
                let text = screen(&mut app, cols, rows);
                if cols >= 80 {
                    assert!(
                        text.contains(&format!("Step {} of 5", index + 1)),
                        "{name} {cols}x{rows} step {index}"
                    );
                    assert!(text.contains(setup::TITLES[index]));
                }
                if index + 1 < setup::TITLES.len() {
                    app.key(KeyCode::Enter.into());
                }
            }
            app.modal = None;
        }
    }
    app.shutdown().await;
}

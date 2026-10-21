use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{Terminal, backend::TestBackend};
use ratatui_image::picker::Picker;
use tokio::sync::mpsc;
use ytui::{
    app::{App, HitTarget, Modal, RuntimeHandles},
    editor::Editor,
    forms::{Form, SETTINGS_SECTIONS},
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
#[test]
fn settings_sections_cover_every_field_and_new_defaults_roundtrip() {
    let config = Config::default();
    let mut form = Form::settings(config.clone());
    let mut indices = SETTINGS_SECTIONS
        .iter()
        .flat_map(|(_, fields)| fields.iter().copied())
        .collect::<Vec<_>>();
    indices.sort();
    assert_eq!(indices, (0..form.fields.len()).collect::<Vec<_>>());
    assert_eq!(
        toml::to_string(&form.config_values().unwrap()).unwrap(),
        toml::to_string(&config).unwrap()
    );
    for (index, value) in [
        (38, "yes"),
        (39, "yes"),
        (40, "720"),
        (41, "mkv"),
        (42, "flac"),
        (43, "yes"),
        (44, "no"),
        (45, "250"),
        (46, "my-cliamp"),
        (47, "my-nmcli"),
        (48, "de,us"),
        (49, "https://radio.test"),
    ] {
        form.fields[index].value = Editor::new(value);
    }
    let saved = form.config_values().unwrap();
    assert!(saved.reduced_motion && saved.download.audio && saved.download.playlist);
    assert!(!saved.download.skip_duplicates);
    assert_eq!(saved.download.quality, "720");
    assert_eq!(saved.download.container, "mkv");
    assert_eq!(saved.download.audio_container, "flac");
    assert_eq!(saved.download.bandwidth_kib, 250);
    assert_eq!(saved.radio_pinned_countries, ["DE", "US"]);
    assert_eq!(saved.cliamp, "my-cliamp");
    assert_eq!(saved.nmcli, "my-nmcli");
    form.fields[49].value = Editor::new("file:///invalid");
    assert!(form.config_values().is_err());
}
#[tokio::test]
async fn every_settings_section_renders_first_and_last_fields_in_all_themes_and_sizes() {
    let (mut app, _temp) = fixture();
    for theme in ytui_services::theme::THEMES {
        app.palette = ytui_services::theme::builtin(theme).unwrap();
        for (cols, rows) in [(35, 12), (80, 24), (120, 40), (180, 50), (212, 58)] {
            let mut terminal = Terminal::new(TestBackend::new(cols, rows)).unwrap();
            for (section, (_, indices)) in SETTINGS_SECTIONS.iter().enumerate() {
                for selected in [indices[0], *indices.last().unwrap()] {
                    let mut form = Form::settings(app.config.clone());
                    form.select_section(section);
                    form.selected = selected;
                    app.modal = Some(Modal::Form(Box::new(form)));
                    terminal.draw(|f| ui::render(f, &mut app)).unwrap();
                    let text = terminal
                        .backend()
                        .buffer()
                        .content
                        .iter()
                        .map(|c| c.symbol())
                        .collect::<String>();
                    assert!(text.contains("General settings"), "{theme} {cols}x{rows}");
                    assert!(text.contains("Enter Save"), "Save missing in {section}");
                    assert_eq!(app.image_area.width, 0);
                    assert!(
                        app.hit_targets
                            .iter()
                            .any(|(_, target)| *target == HitTarget::SettingField(selected)),
                        "Selected field {selected} is offscreen"
                    );
                    if *theme == "terminal" {
                        assert!(
                            terminal
                                .backend()
                                .buffer()
                                .content
                                .iter()
                                .all(|c| !matches!(c.fg, ratatui::style::Color::Rgb(..)))
                        );
                    }
                }
            }
        }
    }
    app.shutdown().await;
}
#[tokio::test]
async fn settings_keyboard_validates_before_save_and_cancel_discards_edits() {
    let (mut app, _temp) = fixture();
    app.action(Action::Settings);
    app.key(KeyCode::F(6).into());
    app.key(KeyCode::Down.into());
    app.key(KeyCode::Enter.into());
    app.key(KeyCode::Right.into());
    app.key(KeyCode::PageDown.into());
    app.key(KeyCode::Char(' ').into());
    for _ in 0..7 {
        app.key(KeyCode::Down.into());
    }
    app.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
    app.key(KeyCode::Char('0').into());
    app.key(KeyCode::Enter.into());
    let Some(Modal::Form(form)) = &app.modal else {
        panic!("Invalid settings closed the window")
    };
    assert!(form.error.as_deref().unwrap().contains("concurrency"));
    assert_eq!(app.config.concurrency, 3);
    assert!(!app.config.download.audio);
    app.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
    app.key(KeyCode::Char('5').into());
    app.key(KeyCode::Enter.into());
    assert!(app.modal.is_none());
    assert_eq!(app.config.concurrency, 5);
    assert!(app.config.download.audio);
    app.action(Action::Settings);
    app.key(KeyCode::Char(' ').into());
    app.key(KeyCode::Esc.into());
    assert!(!app.config.clipboard_watcher);
    app.shutdown().await;
}
#[tokio::test]
async fn settings_mouse_buttons_and_persistence_use_the_visible_window() {
    let (mut app, _temp) = fixture();
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    app.action(Action::Settings);
    terminal.draw(|f| ui::render(f, &mut app)).unwrap();
    let click = |app: &mut App, target: HitTarget| {
        let rect = app
            .hit_targets
            .iter()
            .find(|(_, hit)| *hit == target)
            .unwrap()
            .0;
        app.mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: rect.x,
            row: rect.y,
            modifiers: KeyModifiers::NONE,
        });
    };
    click(&mut app, HitTarget::SettingSection(2));
    terminal.draw(|f| ui::render(f, &mut app)).unwrap();
    click(&mut app, HitTarget::SettingField(39));
    click(&mut app, HitTarget::SettingsCancel);
    assert!(!app.config.download.audio);
    app.action(Action::Settings);
    terminal.draw(|f| ui::render(f, &mut app)).unwrap();
    click(&mut app, HitTarget::SettingField(16));
    app.demo = false;
    app.offline = true;
    click(&mut app, HitTarget::SettingsSave);
    assert!(app.modal.is_none());
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        if app.paths.config_file().exists() && app.config.notifications {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "Settings save did not finish: {}",
            app.status
        );
        if let Ok(Some(event)) =
            tokio::time::timeout(std::time::Duration::from_millis(100), app.tasks.join_next()).await
        {
            app.apply(event.unwrap());
        }
    }
    assert!(app.paths.load_config().unwrap().notifications);
    app.shutdown().await;
}

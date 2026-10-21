mod app;
mod cli;
mod editor;
mod forms;
mod terminal;
mod ui;

use anyhow::Result;
use clap::Parser;
use crossterm::event::{Event, EventStream, KeyEventKind, MouseEventKind};
use futures_util::StreamExt;
use ratatui::{Terminal, backend::CrosstermBackend};
use ratatui_image::picker::{Picker, ProtocolType};
use std::{
    io::{self, IsTerminal},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::mpsc;
use ytui_core::Config;
use ytui_services::{
    auth::SessionStore,
    paths::Paths,
    player::{Player, PlayerCommand, PlayerEvent},
    store::{Snapshot, Store},
};

#[tokio::main]
async fn main() {
    if let Err(e) = entry().await {
        eprintln!(
            "ytui: {}",
            ytui_services::process::diagnostic(&e.to_string())
        );
        std::process::exit(1);
    }
}
async fn entry() -> Result<()> {
    let args = cli::Args::parse();
    if let Some(cli::Command::NativeWorker { request }) = &args.command {
        let request = request.clone();
        return tokio::task::spawn_blocking(move || ytui_services::native::worker(&request))
            .await?;
    }
    let mut paths = Paths::discover()?;
    if let Some(state) = &args.state_dir {
        paths.state = state.clone();
    }
    paths.config_override = args.config.clone();
    paths.create()?;
    let mut config = if let Some(path) = &args.config {
        toml::from_str::<Config>(&std::fs::read_to_string(path)?)?
    } else {
        paths.load_config()?
    };
    paths.fill_directories(&mut config);
    config.offline = args.offline;
    config.validate()?;
    if let Some(command) = args.command {
        if args.offline {
            anyhow::ensure!(
                !matches!(
                    &command,
                    cli::Command::Browse { .. }
                        | cli::Command::Radio { .. }
                        | cli::Command::Auth {
                            command: cli::AuthCommand::Import { .. }
                        }
                ) && !matches!(&command, cli::Command::Download { url, .. } if !std::path::Path::new(url).is_file()),
                "This action requires network access; restart without --offline"
            );
        }
        return cli::run(command, config, paths).await;
    }
    anyhow::ensure!(
        io::stdin().is_terminal() && io::stdout().is_terminal(),
        "The interface needs a terminal; use --help for headless commands"
    );
    let session = if args.demo || args.offline {
        None
    } else {
        let store = SessionStore::new(&paths, &config.auth_store);
        match tokio::task::spawn_blocking(move || store.load()).await? {
            Ok(s) => s.map(Arc::new),
            Err(e) => {
                eprintln!("{e}");
                None
            }
        }
    };
    let store = if args.demo {
        None
    } else {
        Some(Store::open(&paths.state)?)
    };
    let snapshot = if let Some(store) = &store {
        store.load().await?
    } else {
        Snapshot::default()
    };
    let mut player = Player::spawn(config.clone(), paths.clone(), session.clone());
    let guard = terminal::Guard::enter()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    terminal.clear()?;
    let mut picker = if config.image_protocol == "auto" {
        Picker::from_query_stdio().unwrap_or_else(|_| Picker::from_fontsize((8, 16)))
    } else {
        Picker::from_fontsize((8, 16))
    };
    picker.set_protocol_type(match config.image_protocol.as_str() {
        "kitty" => ProtocolType::Kitty,
        "sixel" => ProtocolType::Sixel,
        "iterm2" => ProtocolType::Iterm2,
        "halfblocks" | "none" => ProtocolType::Halfblocks,
        _ => picker.protocol_type(),
    });
    let (job_tx, mut job_rx) = mpsc::channel(256);
    let mut app = app::App::new(
        config,
        paths,
        snapshot,
        session,
        app::RuntimeHandles {
            player: player.commands.clone(),
            jobs: job_tx,
            picker,
        },
        args.demo,
        args.offline,
    )?;
    let outcome = interface(
        &mut terminal,
        &mut app,
        &mut player,
        store.as_ref(),
        &mut job_rx,
    )
    .await;
    // Close the event channel first: final worker messages must never deadlock shutdown.
    job_rx.close();
    app.shutdown().await;
    player.close().await;
    let saved = if let Some(store) = &store {
        store.save(app.snapshot.clone()).await
    } else {
        Ok(())
    };
    drop(guard);
    if let Some(store) = store {
        store.close().await?;
    }
    saved?;
    outcome
}
async fn interface(
    terminal: &mut terminal::Term,
    app: &mut app::App,
    player: &mut Player,
    store: Option<&Store>,
    job_rx: &mut mpsc::Receiver<ytui_core::JobEvent>,
) -> Result<()> {
    let mut events = EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_millis(50));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_save = Instant::now();
    #[cfg(unix)]
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    while !app.quit {
        if app.redraw {
            terminal.draw(|f| ui::render(f, app))?;
            app.redraw = false;
        }
        tokio::select! {
            event=events.next()=>match event {
                Some(Ok(Event::Key(key))) if key.kind!=KeyEventKind::Release=>app.key(key),
                Some(Ok(Event::Paste(text)))=>app.paste(&text),
                Some(Ok(Event::Resize(_, _)))=>{app.image=None;app.redraw=true;},
                Some(Ok(Event::Mouse(mouse))) if app.modal.is_none()&&app.input.is_none()=>match mouse.kind{
                    MouseEventKind::ScrollDown=>app.move_selection(1),MouseEventKind::ScrollUp=>app.move_selection(-1),
                    MouseEventKind::Down(_) if mouse.row>=2&&mouse.row<5=>{let width=terminal.size()?.width;let tab=if mouse.column<width*23/100{0}else if mouse.column<width*46/100{1}else if mouse.column<width*69/100{2}else{3};app.change_tab(tab);},_=>{}
                },
                Some(Err(e))=>return Err(e.into()),None=>app.quit=true,_=>{}
            },
            Some(event)=job_rx.recv()=>{
                app.job_event(event);
            },
            Some(result)=app.tasks.join_next(),if !app.tasks.is_empty()=>match result{Ok(event)=>app.apply(event),Err(e)if !e.is_cancelled()=>{app.status="A background operation stopped unexpectedly; refresh or retry".into();app.redraw=true;},_=>{}},
            Some(event)=player.events.recv()=>match event{
                PlayerEvent::Ended(media)=>app.playback_ended(&media),
                PlayerEvent::Error(message)=>{app.status=message;app.redraw=true;}
            },
            _=tick.tick()=>{
                app.tick().await;
                if player.status.has_changed().unwrap_or(false){app.player_update(player.status.borrow_and_update().clone());}
                if app.dirty&&last_save.elapsed()>Duration::from_millis(500){if let Some(store)=store{store.save(app.snapshot.clone()).await?;}app.dirty=false;last_save=Instant::now();}
            },
            _=tokio::signal::ctrl_c()=>app.quit=true,
            _=terminate.recv()=>app.quit=true,
        }
        if app.launch_cliamp {
            app.launch_cliamp = false;
            let _ = player.commands.send(PlayerCommand::Stop).await;
            terminal::Guard::suspend()?;
            let result = tokio::process::Command::new(&app.config.cliamp)
                .status()
                .await;
            terminal::Guard::resume()?;
            terminal.clear()?;
            app.status = match result {
                Ok(s) if s.success() => "Returned from cliamp".into(),
                Ok(_) => "cliamp exited with an error".into(),
                Err(_) => "Cannot start cliamp; install it or set its executable path".into(),
            };
            app.redraw = true;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend};
    async fn fixture() -> (app::App, mpsc::Receiver<PlayerCommand>, tempfile::TempDir) {
        let temp = tempfile::tempdir().unwrap();
        let paths = Paths {
            config_override: None,
            xdg_state: temp.path().join("state"),
            config: temp.path().join("config/ytui"),
            state: temp.path().join("state/ytui/rust"),
            cache: temp.path().join("cache"),
            legacy_data: temp.path().join("data"),
            home: temp.path().into(),
        };
        paths.create().unwrap();
        let config = Config {
            theme: "ytui-dark".into(),
            ..Default::default()
        };
        let (tx, rx) = mpsc::channel(32);
        let (jobs, _) = mpsc::channel(32);
        let a = app::App::new(
            config,
            paths,
            Snapshot::default(),
            None,
            app::RuntimeHandles {
                player: tx,
                jobs,
                picker: Picker::from_fontsize((8, 16)),
            },
            true,
            false,
        )
        .unwrap();
        (a, rx, temp)
    }
    #[tokio::test]
    async fn typing_and_paste_never_trigger_global_actions() {
        let (mut a, _rx, _temp) = fixture().await;
        a.key(KeyCode::Char('/').into());
        for c in "qvds".chars() {
            a.key(KeyCode::Char(c).into());
        }
        a.paste("日本語");
        assert!(!a.quit);
        assert!(a.modal.is_none());
        assert_eq!(a.input.as_ref().unwrap().editor.text, "qvds日本語");
        a.key(KeyCode::Esc.into());
        a.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(a.quit);
        a.shutdown().await;
    }
    #[tokio::test]
    async fn all_tabs_and_modals_render_at_supported_sizes() {
        let (mut a, _rx, _temp) = fixture().await;
        for (w, h) in [(80, 24), (120, 40), (180, 50), (35, 12)] {
            let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
            for tab in 0..4 {
                a.change_tab(tab);
                terminal.draw(|f| ui::render(f, &mut a)).unwrap();
            }
            for modal in [
                app::Modal::Feeds(0),
                app::Modal::Help(0),
                app::Modal::Themes(0),
                app::Modal::Form(Box::new(forms::Form::settings(a.config.clone()))),
                app::Modal::Form(Box::new(forms::Form::auth())),
                app::Modal::Queue(0),
                app::Modal::History(Default::default()),
                app::Modal::Groups(0),
                app::Modal::Resume(0),
                app::Modal::Watches(0),
                app::Modal::Text {
                    title: "VPN connections".into(),
                    lines: vec!["Fixture".into()],
                    offset: 0,
                },
                app::Modal::Form(Box::new(forms::Form::keys(a.config.clone()))),
            ] {
                a.modal = Some(modal);
                terminal.draw(|f| ui::render(f, &mut a)).unwrap();
                a.modal = None;
            }
        }
        a.shutdown().await;
    }
    #[tokio::test]
    async fn feed_picker_is_remappable_scoped_and_requires_an_explicit_selection() {
        let (mut a, mut rx, _temp) = fixture().await;
        a.config.keybindings.insert("f8".into(), "feeds".into());
        a.key(KeyCode::Char('b').into());
        assert!(a.modal.is_none());
        a.key(KeyCode::F(8).into());
        assert!(matches!(a.modal, Some(app::Modal::Feeds(0))));
        let before = a.snapshot.downloads.jobs.len();
        a.key(KeyCode::Down.into());
        assert!(matches!(a.modal, Some(app::Modal::Feeds(1))));
        assert_eq!(a.snapshot.downloads.jobs.len(), before);
        assert!(rx.try_recv().is_err());
        a.key(KeyCode::Esc.into());
        a.offline = true;
        assert!(
            a.feed_choices()
                .iter()
                .all(|(_, _, command)| command.starts_with("offline")
                    || ["resume", "watches"].contains(command))
        );
        a.key(KeyCode::F(8).into());
        a.key(KeyCode::Down.into());
        a.key(KeyCode::Down.into());
        a.key(KeyCode::Enter.into());
        assert!(matches!(a.modal, Some(app::Modal::Resume(0))));
        assert!(rx.try_recv().is_err());
        a.modal = None;
        a.offline = false;
        a.tab = 3;
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal.draw(|f| ui::render(f, &mut a)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(text.contains("save station"));
        assert!(
            !text.contains("v preview"),
            "stations must not advertise unsupported previews"
        );
        a.tab = 1;
        a.tabs[1].page.items[0].kind = ytui_core::MediaKind::Playlist;
        terminal.draw(|f| ui::render(f, &mut a)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(text.contains("open collection"));
        assert!(!text.contains("v preview"));
        a.shutdown().await;
    }
    #[tokio::test]
    async fn responsive_workspace_keeps_selection_errors_and_remapped_shortcuts_visible() {
        use ratatui_image::Resize;
        let (mut a, _rx, _temp) = fixture().await;
        a.config.keybindings.insert("f8".into(), "feeds".into());
        a.tab = 1;
        a.tabs[1].selected = 17;
        a.tabs[1].page.items[17].title = "日本語 · final selected item".into();
        a.tabs[1].error = Some("Fixture feed failed; loaded content remains".into());
        a.tabs[1].page.items[17].thumbnail = "https://example.test/synthetic.jpg".into();
        a.image = Some(
            a.picker
                .new_protocol(
                    image::DynamicImage::new_rgb8(160, 90),
                    ratatui::layout::Rect::new(0, 0, 36, 9),
                    Resize::Fit(None),
                )
                .unwrap(),
        );
        for (width, height) in [(35, 12), (80, 24), (120, 40), (180, 50)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|f| ui::render(f, &mut a)).unwrap();
            let buffer = terminal.backend().buffer();
            let lines = (0..height)
                .map(|y| {
                    (0..width)
                        .map(|x| buffer[(x, y)].symbol())
                        .collect::<String>()
                })
                .collect::<Vec<_>>();
            let text = lines.join("\n");
            assert!(
                text.contains("final selected item"),
                "selection lost at {width}x{height}"
            );
            assert!(text.contains("18 / 18"));
            assert!(text.contains("Fixture feed failed"));
            assert!(
                lines.last().unwrap().contains("help"),
                "help hidden at {width}x{height}"
            );
            assert!(
                !lines.last().unwrap().contains("b feeds"),
                "stale default binding in footer"
            );
            if width >= 100 {
                assert!(a.image_area.width > 0 && a.image_area.height > 0);
                assert!(a.image_area.right() <= width && a.image_area.bottom() <= height);
            } else {
                assert_eq!(a.image_area.width, 0);
            }
            a.modal = Some(app::Modal::Feeds(0));
            terminal.draw(|f| ui::render(f, &mut a)).unwrap();
            assert_eq!(
                a.image_area.width, 0,
                "image must not draw through an overlay"
            );
            a.modal = None;
        }
        a.config.keybindings.clear();
        a.config.keybindings.insert("b".into(), "help".into());
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal.draw(|f| ui::render(f, &mut a)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(text.contains(":feeds  Browse feeds"));
        assert!(
            !text.contains("b  Browse feeds"),
            "a key claimed by another action must not be advertised"
        );
        a.shutdown().await;
    }
    #[tokio::test]
    async fn download_counts_and_player_time_stay_readable_in_small_and_light_layouts() {
        use ytui_core::JobState;
        let (mut a, _rx, _temp) = fixture().await;
        a.tab = 0;
        a.snapshot.downloads.jobs[0].state = JobState::Error;
        a.snapshot.downloads.jobs[0].error =
            Some("Fixture transfer failed; retry available".into());
        a.snapshot.downloads.jobs[0].progress = ytui_core::Progress {
            downloaded: 42,
            total: Some(100),
            eta: Some(30.0),
            ..Default::default()
        };
        a.player_status = ytui_services::player::PlayerStatus {
            active: true,
            title: "A deliberately long fictional title that must not hide playback timing".into(),
            position: 135.0,
            duration: Some(300.0),
            ..Default::default()
        };
        for palette in ["ytui-dark", "ytui-light", "terminal"] {
            a.palette = ytui_services::theme::builtin(palette).unwrap();
            for (w, h) in [(35, 12), (80, 24), (120, 40)] {
                let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
                terminal.draw(|f| ui::render(f, &mut a)).unwrap();
                let text = terminal
                    .backend()
                    .buffer()
                    .content
                    .iter()
                    .map(|cell| cell.symbol())
                    .collect::<String>();
                assert!(text.contains("2:15"), "time hidden at {w}x{h} ({palette})");
                assert!(text.contains("Failed"));
                assert!(text.contains("42%"));
                assert!(text.contains(if w < 70 { "1 fail" } else { "1 failed" }));
                assert!(text.contains(if w < 70 { "1 done" } else { "1 complete" }));
                if w >= 115 {
                    assert!(text.contains("Fixture transfer failed"));
                }
            }
        }
        a.player_status.preview = true;
        a.player_status.active = false;
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|f| ui::render(f, &mut a)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(text.contains("Muted preview"));
        assert!(text.contains("esc closes"));
        a.shutdown().await;
    }
    #[tokio::test]
    async fn schedule_closure_cancels_work_preserves_manual_pause_and_rejects_late_finish() {
        use tokio_util::sync::CancellationToken;
        use ytui_core::{JobEvent, JobState, JobUpdate};
        let (mut a, _rx, _temp) = fixture().await;
        a.demo = false;
        a.config.notifications = false;
        let minute = (ytui_services::desktop::local_minute() + 20) % 1440;
        let end = (minute + 1) % 1440;
        a.config.schedule.download_window = Some(format!(
            "{:02}:{:02}-{:02}:{:02}",
            minute / 60,
            minute % 60,
            end / 60,
            end % 60
        ));
        a.snapshot.downloads.jobs[0].state = JobState::Downloading;
        a.snapshot.downloads.jobs[1].state = JobState::Paused;
        let id = a.snapshot.downloads.jobs[0].id;
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let task = tokio::spawn(async move {
            token.cancelled().await;
        });
        a.running.insert(
            id,
            app::Running {
                cancel: cancel.clone(),
                task,
            },
        );
        let tasks_before = a.tasks.len();
        a.tick().await;
        assert!(cancel.is_cancelled());
        assert_eq!(a.snapshot.downloads.jobs[0].state, JobState::Queued);
        assert_eq!(a.snapshot.downloads.jobs[1].state, JobState::Paused);
        assert_eq!(a.tasks.len(), tasks_before); // Opt-out does not start a clipboard reader.
        a.job_event(JobEvent {
            id,
            generation: 0,
            update: JobUpdate::Finished,
        });
        assert_eq!(a.snapshot.downloads.jobs[0].state, JobState::Queued);
        a.snapshot.downloads.jobs[0].not_before = Some(ytui_core::schedule::unix_now() + 3600);
        a.config.schedule.download_window = None;
        tokio::task::yield_now().await;
        a.tick().await;
        assert!(a.running.is_empty());
        assert_eq!(a.snapshot.downloads.jobs[1].state, JobState::Paused);
        a.shutdown().await;
    }
    #[tokio::test]
    async fn stale_clipboard_results_and_slash_commands_do_not_trigger_downloads() {
        let (mut a, _rx, _temp) = fixture().await;
        a.config.clipboard_watcher = false;
        a.apply(app::UiEvent::Clipboard {
            generation: 0,
            result: Ok("https://youtu.be/abcdefghijk".into()),
        });
        assert!(a.clipboard_url.is_none());
        a.command("/history").unwrap();
        assert!(matches!(a.modal, Some(app::Modal::History(_))));
        a.command("/keys").unwrap();
        assert!(matches!(a.modal, Some(app::Modal::Form(_))));
        assert_eq!(a.snapshot.downloads.jobs.len(), 2);
        a.shutdown().await;
    }
    #[tokio::test]
    async fn preview_positions_are_ignored_and_resume_is_explicit() {
        use ytui_core::{Media, MediaKind};
        let (mut a, mut rx, _temp) = fixture().await;
        a.demo = false;
        let media = Media {
            id: "abcdefghijk".into(),
            title: "Saved video".into(),
            url: "https://www.youtube.com/watch?v=abcdefghijk".into(),
            kind: MediaKind::Video,
            ..Default::default()
        };
        let mut status = ytui_services::player::PlayerStatus {
            media: Some(media.clone()),
            position: 42.0,
            duration: Some(200.0),
            preview: true,
            active: true,
            ..Default::default()
        };
        a.player_update(status.clone());
        assert!(a.snapshot.positions.entries.is_empty());
        status.preview = false;
        a.player_update(status);
        assert_eq!(a.snapshot.positions.entries.len(), 1);
        assert!(rx.try_recv().is_err());
        a.tabs[1].page.items[0] = media;
        a.command("resume selected").unwrap();
        assert!(
            matches!(rx.recv().await.unwrap(), PlayerCommand::PlayAt { seconds, .. } if seconds == 42.0)
        );
        a.command("resume clear").unwrap();
        assert!(a.snapshot.positions.entries.is_empty());
        a.shutdown().await;
    }
    #[tokio::test]
    async fn paused_watch_rejects_old_results_and_baseline_never_enqueues() {
        use ytui_core::{BrowseRequest, Media, watch::CollectionWatch};
        let (mut a, _rx, _temp) = fixture().await;
        a.demo = false;
        let id = uuid::Uuid::new_v4();
        let watch = CollectionWatch {
            id,
            title: "Collection".into(),
            request: BrowseRequest::Collection("VLfixture".into()),
            music: false,
            options: a.config.download.clone(),
            interval_seconds: 60,
            next_check: u64::MAX,
            enabled: true,
            generation: 0,
            owner: "anonymous".into(),
            seen: None,
            failures: 0,
            error: None,
        };
        a.snapshot.watches.push(watch);
        let media = Media {
            id: "abcdefghijk".into(),
            url: "https://www.youtube.com/watch?v=abcdefghijk".into(),
            ..Default::default()
        };
        let before = a.snapshot.downloads.jobs.len();
        a.apply(app::UiEvent::Watch {
            id,
            generation: 0,
            result: Ok(vec![media.clone()]),
        });
        assert_eq!(a.snapshot.downloads.jobs.len(), before);
        a.command(&format!("watch pause {id}")).unwrap();
        a.apply(app::UiEvent::Watch {
            id,
            generation: 0,
            result: Ok(vec![Media {
                id: "new".into(),
                ..media
            }]),
        });
        assert_eq!(a.snapshot.downloads.jobs.len(), before);
        assert!(!a.snapshot.watches[0].enabled);
        a.shutdown().await;
    }
    #[tokio::test]
    async fn offline_mode_holds_remote_jobs_and_never_plays_remote_queue_items() {
        use ytui_core::{JobState, Media};
        let (mut a, mut rx, _temp) = fixture().await;
        a.demo = false;
        a.offline = true;
        a.snapshot.downloads.jobs[0].state = JobState::Queued;
        a.snapshot.downloads.jobs[0].media.url =
            "https://www.youtube.com/watch?v=abcdefghijk".into();
        let tasks = a.tasks.len();
        a.tick().await;
        assert!(a.running.is_empty());
        assert_eq!(a.snapshot.downloads.jobs[0].state, JobState::Queued);
        a.play(Media {
            url: "https://example.test/video".into(),
            ..Default::default()
        });
        assert!(rx.try_recv().is_err());
        a.navigate(1, ytui_core::BrowseRequest::Home, false);
        assert_eq!(a.tasks.len(), tasks);
        a.shutdown().await;
    }
}

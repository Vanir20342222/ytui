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
    config.validate()?;
    if let Some(command) = args.command {
        return cli::run(command, config, paths).await;
    }
    anyhow::ensure!(
        io::stdin().is_terminal() && io::stdout().is_terminal(),
        "The interface needs a terminal; use --help for headless commands"
    );
    let session = if args.demo {
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
                PlayerEvent::Ended=>{if let Some(media)=app.snapshot.playback.advance(true){app.play_existing(media);}app.dirty=true;},
                PlayerEvent::Error(message)=>{app.status=message;app.redraw=true;}
            },
            _=tick.tick()=>{
                app.tick().await;
                if player.status.has_changed().unwrap_or(false){app.player_status=player.status.borrow_and_update().clone();app.redraw=true;}
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
                app::Modal::Help(0),
                app::Modal::Themes(0),
                app::Modal::Form(Box::new(forms::Form::settings(a.config.clone()))),
                app::Modal::Form(Box::new(forms::Form::auth())),
                app::Modal::Queue(0),
                app::Modal::History(Default::default()),
                app::Modal::Groups(0),
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
}

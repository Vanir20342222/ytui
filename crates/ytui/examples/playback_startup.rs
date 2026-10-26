//! Opt-in startup benchmark; private temporary state, native-only, no browser import.
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::json;
use std::{
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    time::{Duration, Instant},
};
use ytui_core::{Config, Media};
use ytui_services::{
    paths::Paths,
    player::{Player, PlayerCommand},
};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    url: String,
    #[arg(long)]
    audio: bool,
    #[arg(long, default_value_t = 2)]
    rounds: u32,
    /// An explicitly imported private test session; never discovers browser cookies.
    #[arg(long)]
    session_file: Option<PathBuf>,
    /// Time spent browsing before pressing Play; reported separately from startup.
    #[arg(long, default_value_t = 0)]
    prepare_seconds: u64,
    /// Also require real EOF after the measured first frame.
    #[arg(long)]
    complete: bool,
}
#[tokio::main]
async fn main() -> Result<()> {
    let raw: Vec<String> = std::env::args().collect();
    if raw.get(1).is_some_and(|arg| arg == "__native-worker") {
        let path = PathBuf::from(raw.get(2).context("missing worker request")?);
        return tokio::task::spawn_blocking(move || ytui_services::native::worker(&path)).await?;
    }
    let args = Args::parse();
    let temp = tempfile::tempdir()?;
    let root = temp.path();
    let mpv = ytui_services::paths::executable("mpv").context("mpv missing")?;
    let wrapper = root.join("mpv-null");
    std::fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nexec '{}' --vo=null --ao=null \"$@\"\n",
            mpv.display().to_string().replace('\'', "'\\''")
        ),
    )?;
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700))?;
    let paths = Paths {
        config_override: None,
        xdg_state: root.join("xdg"),
        config: root.join("config"),
        state: root.join("state"),
        cache: root.join("cache"),
        legacy_data: root.join("legacy"),
        home: root.into(),
    };
    paths.create()?;
    let config = Config {
        mpv: wrapper.display().to_string(),
        playback_backend: "native".into(),
        yt_dlp: "/must-not-run-external-extraction".into(),
        playback_sponsorblock: "off".into(),
        auth_store: "file".into(),
        notifications: false,
        ..Default::default()
    };
    let session = args
        .session_file
        .as_ref()
        .map(|path| -> Result<_> {
            Ok(std::sync::Arc::new(serde_json::from_slice::<
                ytui_services::auth::Session,
            >(&std::fs::read(path)?)?))
        })
        .transpose()?;
    let mut player = Player::spawn(config, paths, session);
    let result = async {
        for round in 0..args.rounds {
            if round == 0 && args.prepare_seconds > 0 {
                player.commands.send(PlayerCommand::Prepare { media: Media { url: args.url.clone(), ..Default::default() }, audio: args.audio }).await?;
                tokio::time::sleep(Duration::from_secs(args.prepare_seconds)).await;
            }
            let started = Instant::now();
            player.commands.send(PlayerCommand::Play { media: Media { url: args.url.clone(), ..Default::default() }, audio: args.audio }).await?;
            let mut backend_selected_ms = None;
            let startup = tokio::time::timeout(Duration::from_secs(90), async {
                loop {
                    let state = player.status.borrow_and_update().clone();
                    ensure!(state.error.is_none(), "{}", state.error.unwrap_or_default());
                    if backend_selected_ms.is_none() && !state.backend.is_empty() { backend_selected_ms = Some(started.elapsed().as_millis()); }
                    if state.active && !state.buffering && state.position >= 0.05 {
                        return Ok::<_, anyhow::Error>(json!({"round":round,"preparation_wait_seconds":if round == 0 {args.prepare_seconds} else {0},"backend_selected_ms":backend_selected_ms,"first_progress_ms":started.elapsed().as_millis(),"position":state.position,"decoded_height":state.video_height,"backend":state.backend,"audio":args.audio}));
                    }
                    player.status.changed().await?;
                }
            }).await.context("startup deadline")??;
            println!("{startup}");
            if args.complete {
                let duration = player.status.borrow().duration.unwrap_or(600.0);
                tokio::time::timeout(Duration::from_secs_f64(duration.clamp(1.0, 3600.0) + 120.0), async {
                    loop {
                        let state = player.status.borrow_and_update().clone();
                        ensure!(state.error.is_none(), "{}", state.error.unwrap_or_default());
                        tokio::select! {
                            event = player.events.recv() => match event {
                                Some(ytui_services::player::PlayerEvent::Ended(_)) => {
                                    let state = player.status.borrow().clone();
                                    ensure!(state.duration.is_some_and(|duration| state.position >= duration - 2.0), "premature completion");
                                    println!("{}", json!({"round":round,"completed":true,"position":state.position,"duration":state.duration,"elapsed_ms":started.elapsed().as_millis()}));
                                    return Ok::<_, anyhow::Error>(());
                                },
                                Some(ytui_services::player::PlayerEvent::Error(error)) => anyhow::bail!("{error}"),
                                None => anyhow::bail!("player event channel closed"),
                            },
                            result = player.status.changed() => result?,
                        }
                    }
                }).await.context("completion deadline")??;
            }
            player.commands.send(PlayerCommand::Stop).await?;
            tokio::time::timeout(Duration::from_secs(4), async {
                while player.status.borrow_and_update().active { player.status.changed().await?; }
                Ok::<_, anyhow::Error>(())
            }).await??;
        }
        Ok::<_, anyhow::Error>(())
    }.await;
    player.close().await;
    result
}

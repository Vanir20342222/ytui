//! Opt-in acceptance of one public prerecorded video through the real player.
//! The same executable handles native worker requests, just like the application.
use anyhow::{Context, Result, bail, ensure};
use clap::Parser;
use serde_json::{Value, json};
use std::{
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
};
use ytui_core::{Config, Media};
use ytui_services::{
    paths::Paths,
    player::{Player, PlayerCommand, PlayerEvent, PlayerStatus},
};

const VIDEO: &str = "https://www.youtube.com/watch?v=jNQXAC9IVRw";
#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "native", value_parser = ["native", "auto", "yt-dlp"])]
    backend: String,
    /// Comma-separated case names; omit for the complete native sweep.
    #[arg(long)]
    cases: Option<String>,
    /// Show one muted completion run using the desktop's real video/audio outputs.
    #[arg(long)]
    desktop: bool,
}
struct Fixture {
    _temp: tempfile::TempDir,
    paths: Paths,
    config: Config,
    launches: PathBuf,
    fault: PathBuf,
    desktop: bool,
}
impl Fixture {
    fn new(backend: &str, desktop: bool) -> Result<Self> {
        let temp = tempfile::tempdir()?;
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
        paths.create()?;
        if desktop {
            std::fs::write(
                paths.state.join("player.json"),
                br#"{"volume":70.0,"muted":true,"speed":1.0}"#,
            )?;
        }
        let launches = root.join("launches");
        let fault = root.join("fault");
        let wrapper = root.join("mpv");
        let mpv = ytui_services::paths::executable("mpv").context("mpv is required")?;
        // Only IPC paths and preview/mute flags are recorded, never stream URLs.
        let quote = |p: &Path| format!("'{}'", p.display().to_string().replace('\'', "'\\''"));
        let script = format!(
            r#"#!/bin/sh
socket=''; preview=no; mute=''; start=''
for arg in "$@"; do
    case "$arg" in
        --input-ipc-server=*) socket="${{arg#*=}}";;
        --title=*) preview=yes;;
        --mute=*) mute="${{arg#*=}}";;
        --start=*) start="${{arg#*=}}";;
    esac
done
printf '%s\t%s\t%s\t%s\n' "$preview" "$socket" "$mute" "$start" >> {launches}
if [ -f {fault} ]; then
    mode="$(cat {fault})"
    case "$mode" in
        once) rm -f {fault}; printf '[ffmpeg] HTTP error 403 Forbidden\n' >&2; exit 1;;
        always) printf '[ffmpeg] HTTP error 403 Forbidden\n' >&2; exit 1;;
        clean) exit 0;;
        noipc) sleep 130; exit 1;;
        resume) rm -f {fault}; {mpv} --vo=null --ao=null --length=3 "$@"; printf '[ffmpeg] HTTP error 403 Forbidden\n' >&2; exit 1;;
        truncated) rm -f {fault}; exec {mpv} --vo=null --ao=null --length=3 "$@";;
    esac
fi
exec {mpv} {outputs} "$@"
"#,
            launches = quote(&launches),
            fault = quote(&fault),
            mpv = quote(&mpv),
            outputs = if desktop { "" } else { "--vo=null --ao=null" }
        );
        std::fs::write(&wrapper, script)?;
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755))?;
        let config = Config {
            mpv: wrapper.display().to_string(),
            playback_backend: backend.into(),
            playback_sponsorblock: "off".into(),
            auth_store: "file".into(),
            notifications: false,
            yt_dlp: if backend == "native" {
                "/native-must-not-use-external-yt-dlp".into()
            } else {
                "yt-dlp".into()
            },
            ..Default::default()
        };
        Ok(Self {
            _temp: temp,
            paths,
            config,
            launches,
            fault,
            desktop,
        })
    }
    fn media(&self) -> Media {
        Media {
            id: "jNQXAC9IVRw".into(),
            url: VIDEO.into(),
            title: "Public single-video acceptance".into(),
            ..Default::default()
        }
    }
    fn count(&self) -> usize {
        std::fs::read_to_string(&self.launches)
            .unwrap_or_default()
            .lines()
            .count()
    }
    async fn property(&self, preview: bool, name: &str) -> Result<Value> {
        let text = tokio::fs::read_to_string(&self.launches).await?;
        let socket = text
            .lines()
            .rev()
            .find_map(|line| {
                let mut parts = line.split('\t');
                let kind = parts.next()?;
                (kind == if preview { "yes" } else { "no" })
                    .then(|| parts.next())
                    .flatten()
            })
            .context("mpv has not launched")?;
        tokio::time::timeout(Duration::from_secs(2), async {
            let stream = UnixStream::connect(socket).await?;
            let (read, mut write) = stream.into_split();
            write
                .write_all(
                    format!(
                        "{}\n",
                        json!({"command":["get_property",name],"request_id":1})
                    )
                    .as_bytes(),
                )
                .await?;
            let mut lines = BufReader::new(read).lines();
            while let Some(line) = lines.next_line().await? {
                let value: Value = serde_json::from_str(&line)?;
                if value["request_id"] == 1 {
                    ensure!(
                        value["error"] == "success",
                        "mpv property {name} unavailable"
                    );
                    return Ok(value["data"].clone());
                }
            }
            bail!("mpv disconnected")
        })
        .await
        .context("mpv IPC deadline")?
    }
}
async fn wait(
    player: &mut Player,
    predicate: impl Fn(&PlayerStatus) -> bool,
) -> Result<PlayerStatus> {
    tokio::time::timeout(Duration::from_secs(100), async {
        loop {
            let state = player.status.borrow_and_update().clone();
            if let Some(error) = &state.error {
                bail!("{}", ytui_services::process::diagnostic(error));
            }
            if predicate(&state) {
                return Ok(state);
            }
            player
                .status
                .changed()
                .await
                .context("player worker stopped")?;
        }
    })
    .await
    .context("player did not reach the required state in 100 seconds")?
}
async fn stop(player: &mut Player) -> Result<()> {
    player.commands.send(PlayerCommand::Stop).await?;
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let state = player.status.borrow_and_update().clone();
            if state.media.is_none() && !state.active && !state.buffering {
                return Ok::<_, anyhow::Error>(());
            }
            player.status.changed().await?;
        }
    })
    .await
    .context("Stop did not acknowledge within three seconds")??;
    while player.events.try_recv().is_ok() {}
    Ok(())
}
async fn play(player: &mut Player, media: Media, audio: bool) -> Result<PlayerStatus> {
    stop(player).await?;
    player
        .commands
        .send(PlayerCommand::Play { media, audio })
        .await?;
    wait(player, |s| s.active && !s.buffering && s.position >= 0.5).await
}
async fn failed(player: &mut Player) -> Result<String> {
    tokio::time::timeout(Duration::from_secs(100), async {
        loop {
            let state = player.status.borrow_and_update().clone();
            if let Some(error) = state.error {
                ensure!(!state.active && !state.buffering, "failure left player active/buffering");
                return Ok(error);
            }
            tokio::select! {
                changed = player.status.changed() => { changed?; },
                event = player.events.recv() => {
                    if matches!(event, Some(PlayerEvent::Ended(_))) { bail!("failed launch incorrectly completed the video"); }
                }
            }
        }
    }).await.context("failure was not surfaced within 100 seconds")?
}
async fn scenario(f: &Fixture, player: &mut Player, case: &str) -> Result<Value> {
    match case {
        "completion" => {
            let state = play(player, f.media(), false).await?;
            ensure!(
                state.duration.is_some_and(|d| d > 15.0),
                "missing video duration"
            );
            let outputs = if f.desktop {
                let vo = f.property(false, "current-vo").await?;
                let ao = f.property(false, "current-ao").await?;
                ensure!(
                    vo.as_str().is_some_and(|v| v != "null")
                        && ao.as_str().is_some_and(|v| v != "null"),
                    "real desktop outputs were not opened"
                );
                ensure!(
                    f.property(false, "mute").await? == true,
                    "desktop test must stay muted"
                );
                json!({"video_output":vo,"audio_output":ao,"muted":true})
            } else {
                json!({})
            };
            tokio::time::timeout(Duration::from_secs(40), async {
                match player.events.recv().await {
                    Some(PlayerEvent::Ended(media)) => {
                        ensure!(media.id == "jNQXAC9IVRw", "wrong completed media");
                        Ok(())
                    }
                    Some(PlayerEvent::Error(error)) => bail!("{error}"),
                    None => bail!("player event channel stopped"),
                }
            })
            .await
            .context("video did not reach its real end")??;
            return Ok(outputs);
        }
        "controls-preview" => {
            play(player, f.media(), false).await?;
            player.commands.send(PlayerCommand::TogglePause).await?;
            wait(player, |s| s.paused).await?;
            ensure!(
                f.property(false, "pause").await? == true,
                "mpv did not pause"
            );
            let position = f
                .property(false, "time-pos")
                .await?
                .as_f64()
                .context("position")?;
            tokio::time::sleep(Duration::from_millis(400)).await;
            ensure!(
                (f.property(false, "time-pos")
                    .await?
                    .as_f64()
                    .context("paused position")?
                    - position)
                    .abs()
                    < 0.2,
                "paused video kept advancing"
            );
            player.commands.send(PlayerCommand::SeekTo(5.0)).await?;
            player.commands.send(PlayerCommand::Speed(0.5)).await?;
            player.commands.send(PlayerCommand::Volume(-10.0)).await?;
            player.commands.send(PlayerCommand::Mute).await?;
            wait(player, |s| s.speed == 1.5 && s.muted && s.volume == 60.0).await?;
            ensure!(
                f.property(false, "speed").await? == 1.5,
                "mpv speed mismatch"
            );
            ensure!(
                f.property(false, "mute").await? == true,
                "mpv mute mismatch"
            );
            player
                .commands
                .send(PlayerCommand::Preview(f.media()))
                .await?;
            wait(player, |s| s.preview).await?;
            tokio::time::timeout(Duration::from_secs(50), async {
                loop {
                    if let Some(error) = player.status.borrow().error.clone() {
                        bail!(
                            "preview failed: {}",
                            ytui_services::process::diagnostic(&error)
                        );
                    }
                    if f.property(true, "time-pos").await.is_ok() {
                        return Ok::<_, anyhow::Error>(());
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            })
            .await
            .context("preview never opened")??;
            ensure!(
                f.property(true, "mute").await? == true,
                "preview was not muted"
            );
            ensure!(
                f.property(false, "pause").await? == true,
                "preview did not pause the main player"
            );
            player.commands.send(PlayerCommand::ClosePreview).await?;
            wait(player, |s| !s.preview).await?;
            ensure!(
                f.property(false, "pause").await? == true,
                "preview resumed manually paused playback"
            );
            player.commands.send(PlayerCommand::TogglePause).await?;
            wait(player, |s| !s.paused && s.position >= 6.0).await?;
            player
                .commands
                .send(PlayerCommand::Preview(f.media()))
                .await?;
            wait(player, |s| s.preview).await?;
            wait(player, |s| !s.preview && !s.paused).await?;
            ensure!(
                f.property(false, "pause").await? == false,
                "natural preview end did not resume playback"
            );
            ensure!(
                f.property(false, "volume").await? == 60.0,
                "preview changed main volume"
            );
        }
        "replay" => {
            let start = Instant::now();
            play(player, f.media(), false).await?;
            let cold = start.elapsed();
            let start = Instant::now();
            play(player, f.media(), false).await?;
            return Ok(json!({"cold_ms":cold.as_millis(),"replay_ms":start.elapsed().as_millis()}));
        }
        "quality-switch" => {
            play(player, f.media(), false).await?;
            player.commands.send(PlayerCommand::TogglePause).await?;
            wait(player, |s| s.paused).await?;
            player.commands.send(PlayerCommand::SeekTo(5.0)).await?;
            let launches = f.count();
            player
                .commands
                .send(PlayerCommand::Quality("144".into()))
                .await?;
            tokio::time::timeout(Duration::from_secs(80), async {
                while f.count() == launches {
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
            })
            .await
            .context("quality relaunch did not occur")?;
            wait(player, |s| s.paused && !s.buffering && s.position >= 4.5).await?;
            ensure!(
                f.property(false, "video-params/h").await? == 144,
                "quality change was not applied"
            );
            ensure!(
                f.property(false, "pause").await? == true,
                "quality change lost pause state"
            );
            ensure!(
                f.property(false, "time-pos")
                    .await?
                    .as_f64()
                    .context("position after quality change")?
                    >= 4.5,
                "quality change lost position"
            );
        }
        "quality-144" | "quality-240" | "audio" | "watch-playlist-context" => {
            let mut config = f.config.clone();
            if let Some(quality) = case.strip_prefix("quality-") {
                config.download.quality = quality.into();
            }
            player
                .commands
                .send(PlayerCommand::Configure(Box::new(config)))
                .await?;
            let mut media = f.media();
            if case == "watch-playlist-context" {
                media.url.push_str("&list=PLsynthetic&index=2");
            }
            play(player, media, case == "audio").await?;
            if let Some(quality) = case.strip_prefix("quality-") {
                let height = f
                    .property(false, "video-params/h")
                    .await?
                    .as_u64()
                    .context("decoded video height")?;
                ensure!(
                    height <= quality.parse::<u64>()?,
                    "decoded height {height} exceeded requested {quality}"
                );
                return Ok(json!({"decoded_height":height}));
            }
            if case == "audio" {
                ensure!(
                    !f.property(false, "audio-codec-name").await?.is_null(),
                    "audio decoder missing"
                );
            }
        }
        "missing-runtime" | "missing-mpv" => {
            let mut config = f.config.clone();
            if case == "missing-runtime" {
                config.native_js_runtime = Some("/missing-native-runtime".into());
            } else {
                config.mpv = "/missing-mpv".into();
            }
            player
                .commands
                .send(PlayerCommand::Configure(Box::new(config)))
                .await?;
            player
                .commands
                .send(PlayerCommand::Play {
                    media: f.media(),
                    audio: false,
                })
                .await?;
            failed(player).await?;
            player
                .commands
                .send(PlayerCommand::Configure(Box::new(f.config.clone())))
                .await?;
            play(player, f.media(), false).await?;
        }
        "cdn-renew-once" | "cdn-resume" | "truncated-eof" | "cdn-retry-bounded" | "clean-exit"
        | "no-ipc" => {
            std::fs::write(
                &f.fault,
                match case {
                    "cdn-renew-once" => "once",
                    "cdn-retry-bounded" => "always",
                    "no-ipc" => "noipc",
                    "cdn-resume" => "resume",
                    "truncated-eof" => "truncated",
                    _ => "clean",
                },
            )?;
            player
                .commands
                .send(PlayerCommand::Play {
                    media: f.media(),
                    audio: false,
                })
                .await?;
            if case == "cdn-renew-once" || case == "cdn-resume" || case == "truncated-eof" {
                wait(player, |s| {
                    s.position
                        >= if case == "cdn-resume" || case == "truncated-eof" {
                            4.0
                        } else {
                            1.0
                        }
                        && !s.buffering
                        && (!(case == "cdn-resume" || case == "truncated-eof") || f.count() >= 2)
                })
                .await?;
            } else {
                failed(player).await?;
            }
            ensure!(
                f.count()
                    == if case == "clean-exit" || case == "no-ipc" {
                        1
                    } else {
                        2
                    },
                "unexpected launch/retry count: {}",
                f.count()
            );
            if case == "cdn-resume" || case == "truncated-eof" {
                let log = std::fs::read_to_string(&f.launches)?;
                let position = log
                    .lines()
                    .last()
                    .context("relaunch")?
                    .split('\t')
                    .nth(3)
                    .context("resume flag")?
                    .parse::<f64>()?;
                ensure!(position >= 2.0, "renewal lost the decoded position");
                return Ok(json!({"resumed_at":position}));
            }
        }
        "cancel-loading" => {
            player
                .commands
                .send(PlayerCommand::Play {
                    media: f.media(),
                    audio: false,
                })
                .await?;
            wait(player, |s| s.active).await?;
            let start = Instant::now();
            stop(player).await?;
            ensure!(
                start.elapsed() < Duration::from_secs(3),
                "cancel took too long"
            );
            play(player, f.media(), false).await?;
        }
        _ => bail!("unknown case {case}"),
    }
    Ok(json!({}))
}
#[tokio::main]
async fn main() -> Result<()> {
    let arguments: Vec<String> = std::env::args().collect();
    if arguments.get(1).is_some_and(|a| a == "__native-worker") {
        let request = PathBuf::from(arguments.get(2).context("missing worker request")?);
        return tokio::task::spawn_blocking(move || ytui_services::native::worker(&request))
            .await?;
    }
    let args = Args::parse();
    let cases = args.cases.unwrap_or_else(|| "completion,controls-preview,replay,quality-144,quality-240,quality-switch,audio,watch-playlist-context,missing-runtime,missing-mpv,cdn-renew-once,cdn-resume,truncated-eof,cdn-retry-bounded,clean-exit,cancel-loading".into());
    ensure!(
        !args.desktop || cases == "completion",
        "--desktop requires --cases completion"
    );
    let mut failures = 0;
    for case in cases.split(',') {
        let fixture = Fixture::new(&args.backend, args.desktop)?;
        let mut player = Player::spawn(fixture.config.clone(), fixture.paths.clone(), None);
        let started = Instant::now();
        let result = tokio::select! {
            result = scenario(&fixture, &mut player, case) => Some(result),
            _ = tokio::signal::ctrl_c() => None,
        };
        player.close().await;
        let result = result.context("single-video sweep interrupted; player was stopped")?;
        match result {
            Ok(detail) => println!(
                "{}",
                json!({"case":case,"backend":args.backend,"passed":true,"elapsed_ms":started.elapsed().as_millis(),"detail":detail})
            ),
            Err(error) => {
                failures += 1;
                println!(
                    "{}",
                    json!({"case":case,"backend":args.backend,"passed":false,"error":ytui_services::process::diagnostic(&format!("{error:#}"))})
                );
            }
        }
    }
    ensure!(failures == 0, "{failures} single-video sweep cases failed");
    Ok(())
}

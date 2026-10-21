use crate::{
    auth::Session,
    paths::Paths,
    process::{self, ProcessResult, ProcessSpec},
};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
    sync::{mpsc, watch},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;
use ytui_core::{Config, Media, youtube_host};

#[derive(Clone, Debug)]
pub enum PlayerCommand {
    Play { media: Media, audio: bool },
    PlayFallback { media: Media, audio: bool },
    Configure(Box<Config>),
    Preview(Media),
    ClosePreview,
    TogglePause,
    Seek(f64),
    Volume(f64),
    Mute,
    Stop,
    Session(Option<Arc<Session>>),
    Shutdown,
}
#[derive(Clone, Debug, Default)]
pub struct PlayerStatus {
    pub title: String,
    pub position: f64,
    pub duration: Option<f64>,
    pub volume: f64,
    pub paused: bool,
    pub preview: bool,
    pub buffering: bool,
    pub error: Option<String>,
}
#[derive(Clone, Debug)]
pub enum PlayerEvent {
    Ended,
    Error(String),
}
pub struct Player {
    pub commands: mpsc::Sender<PlayerCommand>,
    pub status: watch::Receiver<PlayerStatus>,
    pub events: mpsc::Receiver<PlayerEvent>,
    task: JoinHandle<()>,
}
struct Active {
    cancel: CancellationToken,
    task: JoinHandle<Result<ProcessResult>>,
    socket: PathBuf,
    _dir: tempfile::TempDir,
    _cookies: Option<tempfile::NamedTempFile>,
}
impl Active {
    async fn stop(self) {
        self.cancel.cancel();
        let _ = self.task.await;
    }
    async fn command(&self, args: Value) -> Result<Value> {
        ipc(&self.socket, args).await
    }
}
async fn ipc(path: &Path, args: Value) -> Result<Value> {
    tokio::time::timeout(Duration::from_millis(400), async {
        let stream = UnixStream::connect(path).await?;
        let (read, mut write) = stream.into_split();
        write
            .write_all(format!("{}\n", json!({"command":args,"request_id":1})).as_bytes())
            .await?;
        let mut lines = BufReader::new(read).lines();
        while let Some(line) = lines.next_line().await? {
            let v: Value = serde_json::from_str(&line)?;
            if v.get("request_id").and_then(Value::as_u64) == Some(1) {
                ensure!(
                    v.get("error").and_then(Value::as_str) == Some("success"),
                    "mpv command failed"
                );
                return Ok(v.get("data").cloned().unwrap_or(Value::Null));
            }
        }
        anyhow::bail!("mpv disconnected")
    })
    .await
    .context("mpv IPC timed out")?
}
async fn start(
    config: &Config,
    paths: &Paths,
    session: Option<&Session>,
    media: &Media,
    audio: bool,
    preview: bool,
) -> Result<Active> {
    let local = Path::new(&media.url).is_file();
    let youtube = if local {
        false
    } else {
        let url = ytui_core::http_url(&media.url)?;
        youtube_host(url.host_str().unwrap_or_default())
    };
    let dir = tempfile::Builder::new().prefix("ytui-mpv-").tempdir()?;
    let socket = dir.path().join("ipc");
    let cookies = if youtube {
        session
            .map(|s| s.export(&paths.state.join("auth/transient")))
            .transpose()?
    } else {
        None
    };
    let mut args = vec![
        "--no-config".into(),
        "--no-terminal".into(),
        "--input-default-bindings=yes".into(),
        "--idle=no".into(),
        "--keep-open=no".into(),
        "--ytdl=yes".into(),
        format!("--input-ipc-server={}", socket.display()),
        format!("--script-opts=ytdl_hook-ytdl_path={}", config.yt_dlp),
        "--volume=70".into(),
    ];
    if audio {
        args.push("--no-video".into());
    } else {
        args.push("--force-window=yes".into());
    }
    if preview {
        args.extend([
            "--mute=yes".into(),
            "--length=10".into(),
            "--ytdl-format=best[height<=360]/best".into(),
            "--title=ytui · Preview".into(),
        ]);
    }
    if let Some(cookies) = &cookies {
        args.push(format!(
            "--ytdl-raw-options=cookies={}",
            cookies.path().display()
        ));
    }
    if let Some(proxy) = &config.proxy {
        args.push(format!("--http-proxy={proxy}"));
    }
    args.push("--".into());
    args.push(media.url.clone());
    let cancel = CancellationToken::new();
    let token = cancel.clone();
    if config.download_backend == "native" && !youtube {
        args.retain(|a| a != "--ytdl=yes" && !a.starts_with("--script-opts="));
        args.insert(0, "--ytdl=no".into());
    }
    let spec = ProcessSpec::new(&config.mpv, args, if preview { 45 } else { 24 * 60 * 60 });
    let config = config.clone();
    let media = media.clone();
    let cookie_path = cookies.as_ref().map(|f| f.path().to_owned());
    let native_dir = paths.state.join("auth/transient");
    let task = tokio::spawn(async move {
        let spec = if youtube && config.download_backend == "native" {
            let mut options = config.download.clone();
            options.audio = audio;
            options.quality = if preview { "360" } else { "720" }.into();
            let resolved = crate::native::resolve(
                ytui_core::Job::new(media, options),
                config,
                cookie_path.clone(),
                &native_dir,
                token.clone(),
            )
            .await?;
            let mut direct = Vec::new();
            for arg in spec.args {
                if !arg.starts_with("--script-opts=")
                    && !arg.starts_with("--ytdl-raw-options=")
                    && !arg.starts_with("--ytdl-format=")
                    && arg != "--ytdl=yes"
                {
                    direct.push(arg);
                }
            }
            direct.pop();
            direct.pop();
            direct.push("--ytdl=no".into());
            if let Some(audio) = resolved.audio_url {
                direct.push(format!("--audio-file={audio}"));
            }
            if let Some(path) = cookie_path {
                direct.push("--cookies=yes".into());
                direct.push(format!("--cookies-file={}", path.display()));
            }
            direct.push("--".into());
            direct.push(resolved.url);
            ProcessSpec::new(
                spec.program,
                direct,
                if preview { 20 } else { 24 * 60 * 60 },
            )
        } else if !youtube && !local {
            let radio = crate::radio::Radio::new(
                crate::http_client(config.proxy.as_deref())?,
                config.radio_servers.clone(),
            );
            let url = tokio::select! {_=token.cancelled()=>anyhow::bail!("Operation cancelled"),r=radio.resolve_playlist(&media.url)=>r?};
            let mut spec = spec;
            spec.args.pop();
            spec.args.push(url);
            spec
        } else {
            spec
        };
        process::run(spec, token, |_| {}).await
    });
    Ok(Active {
        cancel,
        task,
        socket,
        _dir: dir,
        _cookies: cookies,
    })
}
struct Actor {
    config: Config,
    paths: Paths,
    session: Option<Arc<Session>>,
    full: Option<Active>,
    preview: Option<Active>,
    state: PlayerStatus,
    resume_after_preview: bool,
    playing: Option<(Media, bool, bool)>,
    reconnect_count: u32,
    reconnect_at: Option<tokio::time::Instant>,
}
impl Actor {
    async fn stop(&mut self) {
        if let Some(p) = self.preview.take() {
            p.stop().await;
        }
        if let Some(p) = self.full.take() {
            p.stop().await;
        }
        self.reconnect_at = None;
        self.playing = None;
        self.state = PlayerStatus {
            volume: 70.0,
            ..Default::default()
        };
    }
    async fn play(&mut self, media: Media, audio: bool, fallback: bool) {
        self.stop().await;
        self.reconnect_count = 0;
        self.resume_after_preview = false;
        self.playing = Some((media.clone(), audio, fallback));
        self.state = PlayerStatus {
            title: media.title.clone(),
            volume: 70.0,
            buffering: true,
            ..Default::default()
        };
        let mut config = self.config.clone();
        if fallback {
            config.download_backend = "yt-dlp".into();
        }
        match start(
            &config,
            &self.paths,
            self.session.as_deref(),
            &media,
            audio,
            false,
        )
        .await
        {
            Ok(p) => self.full = Some(p),
            Err(e) => self.state.error = Some(process::diagnostic(&e.to_string())),
        }
    }
    async fn close_preview(&mut self) {
        if let Some(p) = self.preview.take() {
            p.stop().await;
        }
        if self.resume_after_preview
            && let Some(p) = &self.full
        {
            let _ = p.command(json!(["set_property", "pause", false])).await;
        }
        self.state.preview = false;
    }
    async fn preview(&mut self, media: Media) {
        if let Some(p) = self.preview.take() {
            p.stop().await;
        } else {
            self.resume_after_preview = !self.state.paused;
            if let Some(p) = &self.full
                && p.command(json!(["set_property", "pause", true]))
                    .await
                    .is_err()
            {
                self.state.error = Some(
                    "Playback is still loading; wait until it starts, or stop it before previewing"
                        .into(),
                );
                return;
            }
        }
        match start(
            &self.config,
            &self.paths,
            self.session.as_deref(),
            &media,
            false,
            true,
        )
        .await
        {
            Ok(p) => {
                self.preview = Some(p);
                self.state.preview = true;
            }
            Err(e) => {
                self.state.error = Some(process::diagnostic(&e.to_string()));
                self.close_preview().await;
            }
        }
    }
    async fn handle(&mut self, command: PlayerCommand) {
        match command {
            PlayerCommand::Play { media, audio } => self.play(media, audio, false).await,
            PlayerCommand::PlayFallback { media, audio } => self.play(media, audio, true).await,
            PlayerCommand::Preview(media) => self.preview(media).await,
            PlayerCommand::ClosePreview => self.close_preview().await,
            PlayerCommand::Configure(config) => self.config = *config,
            PlayerCommand::Session(session) => {
                self.stop().await;
                self.session = session;
            }
            PlayerCommand::Stop => self.stop().await,
            command => {
                let active = self.preview.as_ref().or(self.full.as_ref());
                if let Some(p) = active {
                    let args = match command {
                        PlayerCommand::TogglePause => json!(["cycle", "pause"]),
                        PlayerCommand::Seek(s) => json!(["seek", s, "relative"]),
                        PlayerCommand::Volume(v) => json!(["add", "volume", v]),
                        PlayerCommand::Mute => json!(["cycle", "mute"]),
                        _ => return,
                    };
                    if let Err(e) = p.command(args).await {
                        self.state.error = Some(process::diagnostic(&e.to_string()));
                    }
                }
            }
        }
    }
    async fn tick(&mut self) -> Option<PlayerEvent> {
        if self.preview.as_ref().is_some_and(|p| p.task.is_finished()) {
            if let Some(p) = self.preview.take() {
                match p.task.await {
                    Ok(Ok(r)) if !r.success => {
                        self.state.error = Some(process::diagnostic(&r.stderr))
                    }
                    Ok(Err(e)) => self.state.error = Some(process::diagnostic(&e.to_string())),
                    _ => {}
                }
            }
            self.close_preview().await;
        }
        let mut event = None;
        if self.full.as_ref().is_some_and(|p| p.task.is_finished()) {
            let p = self.full.take().expect("Finished player exists");
            let result = p.task.await;
            let success = matches!(&result,Ok(Ok(r)) if r.success);
            let message = match result {
                Ok(Ok(r)) => process::diagnostic(&r.stderr),
                Ok(Err(e)) => process::diagnostic(&e.to_string()),
                Err(_) => "mpv worker exited unexpectedly".into(),
            };
            self.state.buffering = false;
            if self.playing.as_ref().is_some_and(|(m, _, _)| m.live) && self.reconnect_count < 3 {
                self.reconnect_count += 1;
                self.reconnect_at = Some(
                    tokio::time::Instant::now() + Duration::from_secs(1 << self.reconnect_count),
                );
                self.state.buffering = true;
                self.state.error = Some(format!(
                    "Radio disconnected · reconnecting ({}/3)",
                    self.reconnect_count
                ));
            } else if success {
                event = Some(PlayerEvent::Ended);
            } else {
                self.state.error = Some(if message.is_empty() {
                    "Playback failed".into()
                } else {
                    message.clone()
                });
                event = Some(PlayerEvent::Error(message));
            }
        }
        if self
            .reconnect_at
            .is_some_and(|when| tokio::time::Instant::now() >= when)
        {
            self.reconnect_at = None;
            if let Some((media, audio, _)) = &self.playing {
                match start(
                    &self.config,
                    &self.paths,
                    self.session.as_deref(),
                    media,
                    *audio,
                    false,
                )
                .await
                {
                    Ok(p) => {
                        self.full = Some(p);
                        self.state.error = None;
                    }
                    Err(e) => {
                        self.state.error = Some(process::diagnostic(&e.to_string()));
                    }
                }
            }
        }
        if let Some(p) = &self.full {
            if let Ok(v) = p.command(json!(["get_property", "time-pos"])).await {
                self.state.position = v.as_f64().unwrap_or(0.0);
                self.state.buffering = false;
            }
            if let Ok(v) = p.command(json!(["get_property", "duration"])).await {
                self.state.duration = v.as_f64();
            }
            if let Ok(v) = p.command(json!(["get_property", "pause"])).await {
                self.state.paused = v.as_bool().unwrap_or(false);
            }
            if let Ok(v) = p.command(json!(["get_property", "volume"])).await {
                self.state.volume = v.as_f64().unwrap_or(70.0);
            }
            if let Ok(v) = p.command(json!(["get_property", "media-title"])).await
                && let Some(title) = v.as_str()
            {
                self.state.title = ytui_core::clean_text(title);
            }
        }
        event
    }
}
impl Player {
    pub fn spawn(config: Config, paths: Paths, session: Option<Arc<Session>>) -> Self {
        let (commands, mut rx) = mpsc::channel(32);
        let (status_tx, status) = watch::channel(PlayerStatus {
            volume: 70.0,
            ..Default::default()
        });
        let (event_tx, events) = mpsc::channel(16);
        let task = tokio::spawn(async move {
            let mut actor = Actor {
                config,
                paths,
                session,
                full: None,
                preview: None,
                state: PlayerStatus {
                    volume: 70.0,
                    ..Default::default()
                },
                resume_after_preview: false,
                playing: None,
                reconnect_count: 0,
                reconnect_at: None,
            };
            let mut tick = tokio::time::interval(Duration::from_millis(500));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    command=rx.recv()=>match command{Some(PlayerCommand::Shutdown)|None=>break,Some(command)=>actor.handle(command).await},
                    _=tick.tick()=>{if let Some(event)=actor.tick().await{let _=event_tx.try_send(event);}}
                }
                let _ = status_tx.send(actor.state.clone());
            }
            actor.stop().await;
        });
        Self {
            commands,
            status,
            events,
            task,
        }
    }
    pub async fn close(self) {
        let _ = self.commands.send(PlayerCommand::Shutdown).await;
        let _ = self.task.await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::UnixListener;
    #[tokio::test]
    async fn preview_restores_prior_playback_pause_state_and_is_muted() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let socket = temp.path().join("full-ipc");
        let listener = UnixListener::bind(&socket).unwrap();
        let paused = Arc::new(std::sync::Mutex::new(false));
        let recorded = paused.clone();
        let server = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                let (read, mut write) = stream.into_split();
                let mut lines = BufReader::new(read).lines();
                if let Ok(Some(line)) = lines.next_line().await {
                    let request: Value = serde_json::from_str(&line).unwrap();
                    let command = &request["command"];
                    if command[0] == "set_property" && command[1] == "pause" {
                        *recorded.lock().unwrap() = command[2].as_bool().unwrap();
                    }
                    let _ = write
                        .write_all(b"{\"request_id\":1,\"error\":\"success\",\"data\":null}\n")
                        .await;
                }
            }
        });
        let fake = temp.path().join("mpv");
        let args = temp.path().join("args");
        std::fs::write(
            &fake,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\nsleep 30\n",
                args.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let paths = Paths {
            config_override: None,
            xdg_state: temp.path().join("state"),
            config: temp.path().join("config"),
            state: temp.path().join("state"),
            cache: temp.path().join("cache"),
            legacy_data: temp.path().join("legacy"),
            home: temp.path().into(),
        };
        paths.create().unwrap();
        let token = CancellationToken::new();
        let stopping = token.clone();
        let full = Active {
            cancel: token,
            task: tokio::spawn(async move {
                stopping.cancelled().await;
                Ok(ProcessResult {
                    success: true,
                    lines: vec![],
                    stderr: String::new(),
                })
            }),
            socket,
            _dir: tempfile::tempdir().unwrap(),
            _cookies: None,
        };
        let mut actor = Actor {
            config: Config {
                mpv: fake.to_string_lossy().into(),
                ..Default::default()
            },
            paths,
            session: None,
            full: Some(full),
            preview: None,
            state: PlayerStatus::default(),
            resume_after_preview: false,
            playing: None,
            reconnect_count: 0,
            reconnect_at: None,
        };
        actor
            .preview(Media {
                url: "https://radio.test/preview.mp4".into(),
                ..Default::default()
            })
            .await;
        for _ in 0..50 {
            if args.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(*paused.lock().unwrap());
        let recorded = std::fs::read_to_string(args).unwrap();
        assert!(recorded.contains("--mute=yes"));
        assert!(recorded.contains("--length=10"));
        assert!(recorded.contains("--ytdl=no"));
        actor.close_preview().await;
        assert!(!*paused.lock().unwrap());
        actor.stop().await;
        server.abort();
        let _ = server.await;
    }
}

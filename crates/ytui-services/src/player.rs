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
    Play {
        media: Media,
        audio: bool,
    },
    PlayAt {
        media: Media,
        audio: bool,
        seconds: f64,
    },
    PlayFallback {
        media: Media,
        audio: bool,
    },
    Configure(Box<Config>),
    Preview(Media),
    ClosePreview,
    TogglePause,
    Seek(f64),
    Volume(f64),
    Mute,
    SeekTo(f64),
    Quality(String),
    Speed(f64),
    CycleAudio,
    CycleSubtitles,
    ToggleSubtitles,
    Fullscreen,
    Chapter(i64),
    ToggleSponsorBlock,
    UndoSkip,
    Stop,
    Session(Option<Arc<Session>>),
    Shutdown,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayerStatus {
    pub title: String,
    pub media: Option<Media>,
    pub position: f64,
    pub duration: Option<f64>,
    pub volume: f64,
    pub paused: bool,
    pub muted: bool,
    pub speed: f64,
    pub sponsor_enabled: bool,
    pub segments: Vec<crate::sponsorblock::Segment>,
    pub last_skip: Option<(f64, f64)>,
    pub notice: Option<String>,
    pub backend: String,
    pub preview: bool,
    pub buffering: bool,
    pub active: bool,
    pub error: Option<String>,
}
#[derive(Clone, Debug)]
pub enum PlayerEvent {
    Ended(Box<Media>),
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
    backend: watch::Receiver<String>,
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
    tokio::time::timeout(Duration::from_millis(120), async {
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
async fn properties(path: &Path) -> Result<Value> {
    tokio::time::timeout(Duration::from_millis(90), async {
        let stream = UnixStream::connect(path).await?;
        let (read, mut write) = stream.into_split();
        let names = [
            "time-pos",
            "duration",
            "pause",
            "volume",
            "media-title",
            "mute",
            "speed",
            "paused-for-cache",
            "user-data/ytui/action",
        ];
        let mut requests = String::new();
        for (id, name) in names.iter().enumerate() {
            requests.push_str(&format!(
                "{}\n",
                json!({"command":["get_property", name], "request_id":id+1})
            ));
        }
        write.write_all(requests.as_bytes()).await?;
        let mut lines = BufReader::new(read).lines();
        let mut values = json!({});
        let mut received = std::collections::HashSet::new();
        while let Some(line) = lines.next_line().await? {
            let response: Value = serde_json::from_str(&line)?;
            if let Some(id) = response
                .get("request_id")
                .and_then(Value::as_u64)
                .filter(|id| *id > 0 && *id <= names.len() as u64)
            {
                if response["error"] == "success" {
                    values[names[id as usize - 1]] = response["data"].clone();
                }
                received.insert(id);
                if received.len() == names.len() {
                    return Ok(values);
                }
            }
        }
        anyhow::bail!("mpv disconnected")
    })
    .await
    .context("mpv status timed out")?
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct Preferences {
    volume: f64,
    muted: bool,
    speed: f64,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            volume: 70.0,
            muted: false,
            speed: 1.0,
        }
    }
}
impl Preferences {
    fn load(paths: &Paths) -> Self {
        std::fs::read(paths.state.join("player.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<Self>(&b).ok())
            .filter(|p| {
                p.volume.is_finite()
                    && (0.0..=100.0).contains(&p.volume)
                    && p.speed.is_finite()
                    && (0.25..=4.0).contains(&p.speed)
            })
            .unwrap_or_default()
    }
    async fn save(&self, paths: &Paths) {
        let path = paths.state.join("player.json");
        if let Ok(bytes) = serde_json::to_vec(self) {
            let _ = tokio::task::spawn_blocking(move || crate::paths::atomic_write(&path, &bytes))
                .await;
        }
    }
}
struct Launch<'a> {
    audio: bool,
    preview: bool,
    paused: bool,
    position: Option<f64>,
    preferences: &'a Preferences,
}
async fn start(
    config: &Config,
    paths: &Paths,
    session: Option<&Session>,
    media: &Media,
    launch: Launch<'_>,
) -> Result<Active> {
    let Launch {
        audio,
        preview,
        paused,
        position,
        preferences,
    } = launch;
    let local = Path::new(&media.url).is_file();
    let youtube = if local {
        false
    } else {
        let url = ytui_core::http_url(&media.url)?;
        youtube_host(url.host_str().unwrap_or_default())
    };
    let dir = tempfile::Builder::new().prefix("ytui-mpv-").tempdir()?;
    let socket = dir.path().join("ipc");
    let script = dir.path().join("ytui-player.lua");
    tokio::fs::write(&script, include_str!("../assets/player-menu.lua")).await?;
    let cookies = if youtube {
        session
            .map(|s| s.export(&paths.state.join("auth/transient")))
            .transpose()?
    } else {
        None
    };
    let mut args = vec![
        "--no-config".into(),
        // Output is piped by process::run, so terminal logging cannot corrupt the
        // TUI. Disabling it also hides mpv/ytdl errors and caused "Playback failed".
        "--terminal=yes".into(),
        "--input-terminal=no".into(),
        "--msg-color=no".into(),
        "--msg-level=all=warn".into(),
        "--input-default-bindings=yes".into(),
        "--idle=no".into(),
        "--keep-open=no".into(),
        "--ytdl=no".into(),
        format!("--input-ipc-server={}", socket.display()),
        format!("--volume={}", preferences.volume),
        format!("--mute={}", if preferences.muted { "yes" } else { "no" }),
        format!("--speed={}", preferences.speed),
        "--osc=yes".into(),
        "--volume-max=100".into(),
        format!("--pause={}", if paused { "yes" } else { "no" }),
        format!("--script={}", script.display()),
        "--osd-level=1".into(),
    ];
    if youtube {
        args.push(format!(
            "--force-media-title={}",
            if media.title.is_empty() {
                "YouTube"
            } else {
                &media.title
            }
        ));
        args.push("--user-agent=Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/132.0.0.0 Safari/537.36".into());
    }
    if config.offline {
        args.extend([
            "--access-references=no".into(),
            "--load-unsafe-playlists=no".into(),
        ]);
    }
    if let Some(position) = position.filter(|p| p.is_finite() && *p >= 0.0) {
        args.push(format!("--start={position:.3}"));
    }
    if audio {
        args.push("--no-video".into());
    } else {
        args.push("--force-window=yes".into());
    }
    if preview {
        args.retain(|a| !a.starts_with("--mute="));
        args.extend([
            "--mute=yes".into(),
            "--length=10".into(),
            "--title=ytui · Preview".into(),
        ]);
    }
    if let Some(proxy) = &config.proxy {
        args.push(format!("--http-proxy={proxy}"));
    }
    args.push("--".into());
    args.push(media.url.clone());
    let cancel = CancellationToken::new();
    let token = cancel.clone();
    let spec = ProcessSpec::new(&config.mpv, args, if preview { 45 } else { 24 * 60 * 60 });
    let config = config.clone();
    let media = media.clone();
    let cookie_path = cookies.as_ref().map(|f| f.path().to_owned());
    let native_dir = paths.state.join("auth/transient");
    let (backend_tx, backend) = watch::channel(String::new());
    let task = tokio::spawn(async move {
        let fallback_spec = (youtube && config.download_backend == "auto").then(|| spec.clone());
        let spec = if youtube && config.download_backend != "yt-dlp" {
            let mut options = config.download.clone();
            options.audio = audio;
            if preview {
                options.quality = "360".into();
            }
            let resolved = crate::native::resolve(
                ytui_core::Job::new(media.clone(), options),
                config.clone(),
                cookie_path.clone(),
                &native_dir,
                token.clone(),
            )
            .await;
            let resolved = match resolved {
                Ok(resolved) => resolved,
                Err(error)
                    if config.download_backend == "auto"
                        && !token.is_cancelled()
                        && crate::native::fallback_eligible(&error.to_string()) =>
                {
                    ensure!(
                        crate::paths::executable(&config.yt_dlp).is_some(),
                        "Native extraction needs fallback. Install yt-dlp and a supported JavaScript runtime (usually Deno); yt-dlp was not found. F2 → Playback repair configures tools."
                    );
                    let _ = backend_tx.send("yt-dlp fallback".into());
                    return external_playback(
                        spec,
                        &config,
                        &media,
                        audio,
                        preview,
                        cookie_path.as_deref(),
                        token,
                    )
                    .await;
                }
                Err(error) => return Err(error),
            };
            let _ = backend_tx.send("native Rust".into());
            let mut direct = spec.args;
            direct.pop();
            direct.pop();
            if let Some(audio) = resolved.audio_url {
                direct.push(format!("--audio-file={audio}"));
            }
            if let Some(path) = &cookie_path {
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
        } else if youtube {
            let _ = backend_tx.send("yt-dlp".into());
            return external_playback(
                spec,
                &config,
                &media,
                audio,
                preview,
                cookie_path.as_deref(),
                token,
            )
            .await;
        } else if !local {
            let _ = backend_tx.send("direct stream".into());
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
            let _ = backend_tx.send(if youtube { "yt-dlp" } else { "local" }.into());
            spec
        };
        let result = run_mpv(spec, token.clone()).await;
        if let Some(fallback) = fallback_spec
            && !token.is_cancelled()
            && matches!(&result,Ok(result) if !result.success && ["403", "404", "Failed to open", "Failed to recognize"].iter().any(|part| result.stderr.contains(part)))
        {
            ensure!(
                crate::paths::executable(&config.yt_dlp).is_some(),
                "The native stream was rejected and yt-dlp fallback was not found. F2 → Playback repair."
            );
            let _ = backend_tx.send("yt-dlp fallback".into());
            return external_playback(
                fallback,
                &config,
                &media,
                audio,
                preview,
                cookie_path.as_deref(),
                token,
            )
            .await;
        }
        result
    });
    Ok(Active {
        cancel,
        task,
        socket,
        backend,
        _dir: dir,
        _cookies: cookies,
    })
}
async fn external_playback(
    spec: ProcessSpec,
    config: &Config,
    media: &Media,
    audio: bool,
    preview: bool,
    cookies: Option<&Path>,
    cancel: CancellationToken,
) -> Result<ProcessResult> {
    let format = if preview {
        "best[height<=360]/best".into()
    } else if audio {
        "bestaudio/best".into()
    } else if config.download.quality == "best" {
        "bestvideo+bestaudio/best".into()
    } else {
        format!(
            "bestvideo[height<=?{}]+bestaudio/best[height<=?{}]",
            config.download.quality, config.download.quality
        )
    };
    let stream = crate::playback_extract::resolve(
        config,
        &media.url,
        &format,
        audio,
        cookies,
        cancel.clone(),
    )
    .await?;
    run_mpv(
        crate::playback_extract::direct(spec, stream, cookies),
        cancel,
    )
    .await
}
fn playback_diagnostic(result: &ProcessResult) -> String {
    // mpv emits most decoder/network errors on stdout, including Lua hook errors.
    let output = result
        .lines
        .iter()
        .rev()
        .take(16)
        .rev()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    let text = if result.stderr.is_empty() {
        output
    } else {
        format!("{}\n{output}", result.stderr)
    };
    let message = process::diagnostic(&text);
    if message.is_empty() {
        "mpv exited without a diagnostic. Check the mpv executable and audio/video output in Settings → Tools.".into()
    } else {
        message
    }
}
async fn run_mpv(spec: ProcessSpec, cancel: CancellationToken) -> Result<ProcessResult> {
    let mut result = process::run(spec, cancel, |_| {}).await?;
    if !result.success {
        result.stderr = playback_diagnostic(&result);
    }
    Ok(result)
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
    sponsor_task: Option<JoinHandle<Result<Vec<crate::sponsorblock::Segment>>>>,
    ignore_segments_until: f64,
}
impl Actor {
    fn preferences(&self) -> Preferences {
        Preferences {
            volume: self.state.volume,
            muted: self.state.muted,
            speed: if self.state.speed > 0.0 {
                self.state.speed
            } else {
                1.0
            },
        }
    }
    async fn stop(&mut self) {
        if let Some(p) = self.preview.take() {
            p.stop().await;
        }
        if let Some(p) = self.full.take() {
            p.stop().await;
        }
        self.reconnect_at = None;
        if let Some(task) = self.sponsor_task.take() {
            task.abort();
            let _ = task.await;
        }
        self.playing = None;
        self.state = PlayerStatus {
            volume: self.state.volume,
            muted: self.state.muted,
            speed: self.state.speed,
            ..Default::default()
        };
    }
    async fn play(
        &mut self,
        media: Media,
        audio: bool,
        fallback: bool,
        position: Option<f64>,
        paused: bool,
    ) {
        self.stop().await;
        self.reconnect_count = 0;
        self.resume_after_preview = false;
        self.playing = Some((media.clone(), audio, fallback));
        self.state = PlayerStatus {
            title: media.title.clone(),
            media: Some(media.clone()),
            volume: self.state.volume,
            muted: self.state.muted,
            speed: self.state.speed,
            sponsor_enabled: self.config.playback_sponsorblock == "skip",
            paused,
            buffering: true,
            active: true,
            ..Default::default()
        };
        self.ignore_segments_until = 0.0;
        if self.config.playback_sponsorblock != "off"
            && crate::sponsorblock::video_id(&media).is_some()
        {
            let config = self.config.clone();
            let media = media.clone();
            self.sponsor_task = Some(tokio::spawn(async move {
                crate::sponsorblock::fetch(&config, &media).await
            }));
        }
        let mut config = self.config.clone();
        if fallback {
            config.download_backend = "yt-dlp".into();
        }
        match start(
            &config,
            &self.paths,
            self.session.as_deref(),
            &media,
            Launch {
                audio,
                preview: false,
                paused,
                position,
                preferences: &self.preferences(),
            },
        )
        .await
        {
            Ok(p) => self.full = Some(p),
            Err(e) => {
                self.state.error = Some(process::diagnostic(&format!("{e:#}")));
                self.state.active = false;
                self.state.buffering = false;
            }
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
            Launch {
                audio: false,
                preview: true,
                paused: false,
                position: None,
                preferences: &self.preferences(),
            },
        )
        .await
        {
            Ok(p) => {
                self.preview = Some(p);
                self.state.preview = true;
            }
            Err(e) => {
                self.state.error = Some(process::diagnostic(&format!("{e:#}")));
                self.close_preview().await;
            }
        }
    }
    async fn handle(&mut self, command: PlayerCommand) {
        match command {
            PlayerCommand::Play { media, audio } => {
                self.play(media, audio, false, None, false).await
            }
            PlayerCommand::PlayAt {
                media,
                audio,
                seconds,
            } => self.play(media, audio, false, Some(seconds), false).await,
            PlayerCommand::PlayFallback { media, audio } => {
                self.play(media, audio, true, None, false).await
            }
            PlayerCommand::Quality(quality) => {
                if quality == "best"
                    || quality
                        .parse::<u32>()
                        .is_ok_and(|v| (144..=8640).contains(&v))
                {
                    self.config.download.quality = quality;
                    if let Some((media, audio, fallback)) = self.playing.clone() {
                        let position = self.state.position;
                        self.play(media, audio, fallback, Some(position), self.state.paused)
                            .await;
                    }
                }
            }
            PlayerCommand::Preview(media) => self.preview(media).await,
            PlayerCommand::ClosePreview => self.close_preview().await,
            PlayerCommand::Configure(config) => {
                let changed = self.config.playback_sponsorblock != config.playback_sponsorblock
                    || self.config.playback_sponsor_categories
                        != config.playback_sponsor_categories
                    || self.config.download.sponsorblock_api != config.download.sponsorblock_api
                    || self.config.proxy != config.proxy;
                if changed {
                    self.state.sponsor_enabled = config.playback_sponsorblock == "skip";
                    if let Some(task) = self.sponsor_task.take() {
                        task.abort();
                    }
                    if config.playback_sponsorblock == "off" {
                        self.state.segments.clear();
                    } else if let Some((media, _, _)) = &self.playing
                        && crate::sponsorblock::video_id(media).is_some()
                    {
                        let config = (*config).clone();
                        let media = media.clone();
                        self.sponsor_task = Some(tokio::spawn(async move {
                            crate::sponsorblock::fetch(&config, &media).await
                        }));
                    }
                }
                self.config = *config;
            }
            PlayerCommand::ToggleSponsorBlock => {
                self.state.sponsor_enabled = !self.state.sponsor_enabled;
            }
            PlayerCommand::UndoSkip => {
                if let Some((from, to)) = self.state.last_skip
                    && let Some(p) = &self.full
                {
                    self.ignore_segments_until = to;
                    let _ = p.command(json!(["seek", from, "absolute+exact"])).await;
                    self.state.position = from;
                    self.state.notice = Some("SponsorBlock: replaying skipped segment".into());
                }
            }
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
                        PlayerCommand::SeekTo(s) if s.is_finite() => {
                            json!(["seek", s.max(0.0), "absolute+exact"])
                        }
                        PlayerCommand::Speed(delta) => json!([
                            "set_property",
                            "speed",
                            (self.state.speed + delta).clamp(0.25, 4.0)
                        ]),
                        PlayerCommand::CycleAudio => json!(["cycle", "aid"]),
                        PlayerCommand::CycleSubtitles => json!(["cycle", "sid"]),
                        PlayerCommand::ToggleSubtitles => json!(["cycle", "sub-visibility"]),
                        PlayerCommand::Fullscreen => json!(["cycle", "fullscreen"]),
                        PlayerCommand::Chapter(delta) => json!(["add", "chapter", delta]),
                        PlayerCommand::Volume(v) => json!(["add", "volume", v]),
                        PlayerCommand::Mute => json!(["cycle", "mute"]),
                        _ => return,
                    };
                    if let Err(e) = p.command(args).await {
                        self.state.error = Some(process::diagnostic(&format!("{e:#}")));
                    } else {
                        match command {
                            PlayerCommand::TogglePause => self.state.paused = !self.state.paused,
                            PlayerCommand::Volume(delta) => {
                                self.state.volume = (self.state.volume + delta).clamp(0.0, 100.0)
                            }
                            PlayerCommand::Mute => self.state.muted = !self.state.muted,
                            PlayerCommand::Speed(delta) => {
                                self.state.speed = (self.state.speed + delta).clamp(0.25, 4.0)
                            }
                            PlayerCommand::SeekTo(s) => self.state.position = s.max(0.0),
                            _ => {}
                        }
                        self.state.error = None;
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
                    Ok(Err(e)) => self.state.error = Some(process::diagnostic(&format!("{e:#}"))),
                    _ => {}
                }
            }
            self.close_preview().await;
        }
        let mut event = None;
        if let Some(active) = &mut self.full
            && active.backend.has_changed().unwrap_or(false)
        {
            self.state.backend = active.backend.borrow_and_update().clone();
            if self.state.backend == "yt-dlp fallback" {
                self.state.notice = Some(
                    "Automatic yt-dlp fallback · native extractor could not play this media".into(),
                );
            }
        }
        if self.full.as_ref().is_some_and(|p| p.task.is_finished()) {
            let p = self.full.take().expect("Finished player exists");
            let result = p.task.await;
            let success = matches!(&result,Ok(Ok(r)) if r.success);
            let message = match result {
                Ok(Ok(r)) if !r.success => playback_diagnostic(&r),
                Ok(Ok(_)) => String::new(),
                Ok(Err(e)) => process::diagnostic(&format!("{e:#}")),
                Err(_) => "mpv worker exited unexpectedly".into(),
            };
            self.state.buffering = false;
            self.state.active = false;
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
            } else if success
                && self
                    .state
                    .duration
                    .is_none_or(|duration| self.state.position >= duration - 2.0)
            {
                if let Some((media, _, _)) = &self.playing {
                    event = Some(PlayerEvent::Ended(Box::new(media.clone())));
                }
            } else if !success {
                self.state.error = Some(if message.is_empty() {
                    "Playback failed".into()
                } else {
                    message.clone()
                });
                event = Some(PlayerEvent::Error(
                    self.state.error.clone().unwrap_or(message),
                ));
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
                    Launch {
                        audio: *audio,
                        preview: false,
                        paused: self.state.paused,
                        position: None,
                        preferences: &self.preferences(),
                    },
                )
                .await
                {
                    Ok(p) => {
                        self.full = Some(p);
                        self.state.active = true;
                        self.state.error = None;
                    }
                    Err(e) => {
                        self.state.error = Some(process::diagnostic(&format!("{e:#}")));
                    }
                }
            }
        }
        if self.sponsor_task.as_ref().is_some_and(|t| t.is_finished())
            && let Some(task) = self.sponsor_task.take()
        {
            match task.await {
                Ok(Ok(segments)) => self.state.segments = segments,
                Ok(Err(_)) => {
                    self.state.notice = Some("SponsorBlock unavailable · playback continues".into())
                }
                _ => {}
            }
        }
        let mut window_action = None;
        if let Some(p) = &self.full
            && let Ok(v) = properties(&p.socket).await
        {
            if let Some(action) = v["user-data/ytui/action"]
                .as_str()
                .filter(|s| !s.is_empty())
            {
                window_action = Some(action.to_owned());
                let _ = p
                    .command(json!(["set_property", "user-data/ytui/action", ""]))
                    .await;
            }
            if let Some(position) = v["time-pos"].as_f64() {
                self.state.position = position;
                self.state.buffering = v["paused-for-cache"].as_bool().unwrap_or(false);
            }
            self.state.duration = v["duration"].as_f64();
            if let Some(paused) = v["pause"].as_bool() {
                self.state.paused = paused;
            }
            if let Some(volume) = v["volume"].as_f64() {
                self.state.volume = volume.clamp(0.0, 100.0);
            }
            if let Some(mute) = v["mute"].as_bool() {
                self.state.muted = mute;
            }
            if let Some(speed) = v["speed"].as_f64() {
                self.state.speed = speed;
            }
            if let Some(title) = v["media-title"].as_str() {
                self.state.title = ytui_core::clean_text(title);
            }
            if self.state.sponsor_enabled
                && !self.state.paused
                && !self.state.buffering
                && let Some(target) = crate::sponsorblock::skip_target(
                    &self.state.segments,
                    self.state.position,
                    self.ignore_segments_until,
                )
                && p.command(json!([
                    "seek",
                    self.state
                        .duration
                        .filter(|d| *d > 0.0)
                        .map_or(target, |d| target.min(d)),
                    "absolute+exact"
                ]))
                .await
                .is_ok()
            {
                self.state.last_skip = Some((self.state.position, target));
                self.state.position = target;
                self.state.notice =
                    Some("SponsorBlock skipped · U replays · B toggles skipping".into());
            }
        }
        if let Some(action) = window_action {
            if let Some(quality) = action.strip_prefix("quality:") {
                self.handle(PlayerCommand::Quality(quality.into())).await;
            } else if action == "sponsor-toggle" {
                self.handle(PlayerCommand::ToggleSponsorBlock).await;
            } else if action == "sponsor-undo" {
                self.handle(PlayerCommand::UndoSkip).await;
            }
        }
        event
    }
}
impl Player {
    pub fn spawn(config: Config, paths: Paths, session: Option<Arc<Session>>) -> Self {
        let preferences = Preferences::load(&paths);
        let initial = PlayerStatus {
            volume: preferences.volume,
            muted: preferences.muted,
            speed: preferences.speed,
            ..Default::default()
        };
        let (commands, mut rx) = mpsc::channel(32);
        let (status_tx, status) = watch::channel(initial.clone());
        let (event_tx, events) = mpsc::channel(16);
        let task = tokio::spawn(async move {
            let mut actor = Actor {
                config,
                paths,
                session,
                full: None,
                preview: None,
                state: initial,
                resume_after_preview: false,
                playing: None,
                reconnect_count: 0,
                reconnect_at: None,
                sponsor_task: None,
                ignore_segments_until: 0.0,
            };
            let mut tick = tokio::time::interval(Duration::from_millis(125));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut saved_at = tokio::time::Instant::now();
            let mut saved = serde_json::to_vec(&preferences).unwrap_or_default();
            loop {
                tokio::select! {
                    biased;
                    command=rx.recv()=>match command{Some(PlayerCommand::Shutdown)|None=>break,Some(command)=>actor.handle(command).await},
                    _=tick.tick()=>{if let Some(event)=actor.tick().await{let _=event_tx.try_send(event);}}
                }
                status_tx.send_if_modified(|last| {
                    if *last == actor.state {
                        false
                    } else {
                        *last = actor.state.clone();
                        true
                    }
                });
                if saved_at.elapsed() >= Duration::from_secs(2) {
                    let prefs = actor.preferences();
                    let bytes = serde_json::to_vec(&prefs).unwrap_or_default();
                    if bytes != saved {
                        prefs.save(&actor.paths).await;
                        saved = bytes;
                    }
                    saved_at = tokio::time::Instant::now();
                }
            }
            actor.preferences().save(&actor.paths).await;
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
        let commands = Arc::new(std::sync::Mutex::new(Vec::new()));
        let command_log = commands.clone();
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
                    command_log.lock().unwrap().push(command.clone());
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
            backend: watch::channel(String::new()).1,
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
            sponsor_task: None,
            ignore_segments_until: 0.0,
        };
        for command in [
            PlayerCommand::Seek(5.0),
            PlayerCommand::Seek(-5.0),
            PlayerCommand::Volume(5.0),
            PlayerCommand::Volume(-5.0),
            PlayerCommand::Mute,
            PlayerCommand::TogglePause,
        ] {
            actor.handle(command).await;
        }
        for expected in [
            json!(["seek", 5.0, "relative"]),
            json!(["seek", -5.0, "relative"]),
            json!(["add", "volume", 5.0]),
            json!(["add", "volume", -5.0]),
            json!(["cycle", "mute"]),
            json!(["cycle", "pause"]),
        ] {
            assert!(commands.lock().unwrap().contains(&expected));
        }
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
        assert!(
            *paused.lock().unwrap(),
            "a previously paused player must stay paused"
        );
        actor.handle(PlayerCommand::TogglePause).await;
        actor
            .preview(Media {
                url: "https://radio.test/preview.mp4".into(),
                ..Default::default()
            })
            .await;
        actor.close_preview().await;
        assert!(
            !*paused.lock().unwrap(),
            "a previously playing player must resume"
        );
        actor.stop().await;
        server.abort();
        let _ = server.await;
    }
    #[tokio::test]
    async fn local_resume_passes_start_time_and_disables_extraction() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("local.mp4");
        std::fs::write(&source, b"fixture").unwrap();
        let script = temp.path().join("mpv");
        std::fs::write(&script, "#!/bin/sh\nprintf '%s\\n' \"$@\"\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let config = Config {
            mpv: script.to_string_lossy().into(),
            download_backend: "yt-dlp".into(),
            ..Default::default()
        };
        let paths = Paths {
            config_override: None,
            xdg_state: temp.path().join("state"),
            config: temp.path().join("config"),
            state: temp.path().join("state/ytui"),
            cache: temp.path().join("cache"),
            legacy_data: temp.path().join("legacy"),
            home: temp.path().into(),
        };
        let player = start(
            &config,
            &paths,
            None,
            &Media {
                url: source.to_string_lossy().into(),
                ..Default::default()
            },
            Launch {
                audio: false,
                preview: false,
                paused: false,
                position: Some(42.25),
                preferences: &Preferences::default(),
            },
        )
        .await
        .unwrap();
        let result = player.task.await.unwrap().unwrap();
        assert!(result.success);
        assert!(result.lines.contains(&"--start=42.250".into()));
        assert!(result.lines.contains(&"--ytdl=no".into()));
        assert!(
            !result
                .lines
                .iter()
                .any(|line| line.starts_with("--script-opts="))
        );
    }
    #[tokio::test]
    async fn mpv_stdout_failures_are_visible_and_sanitized() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let tool = temp.path().join("mpv");
        std::fs::write(&tool,"#!/bin/sh\necho '[ffmpeg] HTTP error 403 Forbidden https://media.test/?token=synthetic-private'\nexit 2\n").unwrap();
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        let result = run_mpv(
            ProcessSpec::new(tool.display().to_string(), vec![], 5),
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert!(!result.success);
        assert!(result.stderr.contains("403 Forbidden"));
        assert!(!result.stderr.contains("synthetic-private"));
        assert!(
            !playback_diagnostic(&ProcessResult {
                success: false,
                lines: vec![],
                stderr: String::new()
            })
            .is_empty()
        );
    }
    #[tokio::test]
    async fn extraction_error_reaches_player_state_and_event_without_generic_failure() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let tool = temp.path().join("extractor");
        std::fs::write(
            &tool,
            "#!/bin/sh\necho 'ERROR: JavaScript runtime is unavailable' >&2\nexit 1\n",
        )
        .unwrap();
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
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
        let mut actor = Actor {
            config: Config {
                mpv: "/must/not/start/mpv".into(),
                yt_dlp: tool.display().to_string(),
                download_backend: "yt-dlp".into(),
                playback_sponsorblock: "off".into(),
                ..Default::default()
            },
            paths,
            session: None,
            full: None,
            preview: None,
            state: PlayerStatus {
                volume: 37.0,
                speed: 1.0,
                ..Default::default()
            },
            resume_after_preview: false,
            playing: None,
            reconnect_count: 0,
            reconnect_at: None,
            sponsor_task: None,
            ignore_segments_until: 0.0,
        };
        actor
            .play(
                Media {
                    url: "https://www.youtube.com/watch?v=synthetic00".into(),
                    ..Default::default()
                },
                false,
                false,
                None,
                false,
            )
            .await;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        let event = loop {
            if let Some(event) = actor.tick().await {
                break event;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "failed extractor did not finish"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        assert!(
            matches!(event,PlayerEvent::Error(message) if message.contains("JavaScript runtime"))
        );
        assert!(
            actor
                .state
                .error
                .as_ref()
                .unwrap()
                .contains("yt-dlp playback extraction failed")
        );
        assert!(!actor.state.active && !actor.state.buffering);
        // Synchronous launch errors must also clear the loading state.
        actor
            .play(
                Media {
                    url: "file:///unsupported".into(),
                    ..Default::default()
                },
                false,
                false,
                None,
                false,
            )
            .await;
        assert!(actor.state.error.is_some());
        assert!(!actor.state.active && !actor.state.buffering);
        actor.stop().await;
    }
    #[tokio::test]
    async fn player_command_channel_recovers_from_failure_and_stops_replacement_worker() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
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
        let failed = temp.path().join("failed-once");
        let extractor = temp.path().join("extractor");
        std::fs::write(&extractor,format!("#!/bin/sh\nif [ ! -f '{}' ]; then : > '{}'; echo 'ERROR: extraction failed' >&2; exit 1; fi\nprintf '%s\n' '{{\"url\":\"https://media.test/audio\"}}'\n",failed.display(),failed.display())).unwrap();
        let args = temp.path().join("mpv-arguments");
        let mpv = temp.path().join("mpv");
        std::fs::write(
            &mpv,
            format!(
                r#"#!/bin/sh
printf '%s\n' "$@" > '{}'
sleep 30
"#,
                args.display()
            ),
        )
        .unwrap();
        for path in [&mpv, &extractor] {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let mut player = Player::spawn(
            Config {
                mpv: mpv.display().to_string(),
                yt_dlp: extractor.display().to_string(),
                download_backend: "yt-dlp".into(),
                playback_sponsorblock: "off".into(),
                ..Default::default()
            },
            paths,
            None,
        );
        let media = Media {
            url: "https://www.youtube.com/watch?v=synthetic00".into(),
            ..Default::default()
        };
        player
            .commands
            .send(PlayerCommand::Play {
                media: media.clone(),
                audio: true,
            })
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if player.status.borrow().error.is_some() {
                    break;
                }
                player.status.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        assert!(!args.exists(), "mpv started despite failed extraction");
        player
            .commands
            .send(PlayerCommand::Play { media, audio: true })
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if args.exists()
                    && player.status.borrow().active
                    && player.status.borrow().error.is_none()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let args = std::fs::read_to_string(args).unwrap();
        assert!(args.contains("--ytdl=no") && args.contains("--no-video"));
        tokio::time::timeout(Duration::from_secs(2), player.close())
            .await
            .expect("replacement player was not reaped");
    }
    #[tokio::test]
    async fn closing_a_window_early_does_not_complete_the_track_or_advance_the_queue() {
        let temp = tempfile::tempdir().unwrap();
        let paths = Paths {
            config_override: None,
            xdg_state: temp.path().join("state"),
            config: temp.path().join("config"),
            state: temp.path().join("state/ytui"),
            cache: temp.path().join("cache"),
            legacy_data: temp.path().join("legacy"),
            home: temp.path().into(),
        };
        for (position, should_end) in [(40.0, false), (99.5, true)] {
            let task = tokio::spawn(async {
                Ok(ProcessResult {
                    success: true,
                    lines: Vec::new(),
                    stderr: String::new(),
                })
            });
            tokio::task::yield_now().await;
            let media = Media {
                title: "Fixture".into(),
                url: "/tmp/fixture.mp4".into(),
                ..Default::default()
            };
            let mut actor = Actor {
                config: Config::default(),
                paths: paths.clone(),
                session: None,
                full: Some(Active {
                    cancel: CancellationToken::new(),
                    task,
                    socket: temp.path().join("unused-ipc"),
                    backend: watch::channel(String::new()).1,
                    _dir: tempfile::tempdir().unwrap(),
                    _cookies: None,
                }),
                preview: None,
                state: PlayerStatus {
                    title: media.title.clone(),
                    media: Some(media.clone()),
                    position,
                    duration: Some(100.0),
                    active: true,
                    ..Default::default()
                },
                resume_after_preview: false,
                playing: Some((media, false, false)),
                reconnect_count: 0,
                reconnect_at: None,
                sponsor_task: None,
                ignore_segments_until: 0.0,
            };
            let event = actor.tick().await;
            assert_eq!(matches!(event, Some(PlayerEvent::Ended(_))), should_end);
            assert!(!actor.state.active);
        }
    }
}

#[cfg(test)]
mod responsiveness_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    fn paths(root: &Path) -> Paths {
        Paths {
            config_override: None,
            xdg_state: root.join("state"),
            config: root.join("config"),
            state: root.join("state"),
            cache: root.join("cache"),
            legacy_data: root.join("legacy"),
            home: root.into(),
        }
    }
    #[tokio::test]
    async fn batched_status_matches_response_ids_and_has_one_timeout_budget() {
        let temp = tempfile::tempdir().unwrap();
        let socket = temp.path().join("ipc");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (read, mut write) = stream.into_split();
            let mut lines = BufReader::new(read).lines();
            let mut requests = Vec::new();
            for _ in 0..9 {
                requests.push(
                    serde_json::from_str::<Value>(&lines.next_line().await.unwrap().unwrap())
                        .unwrap(),
                );
            }
            for request in requests.into_iter().rev() {
                let name = request["command"][1].as_str().unwrap();
                let data = match name {
                    "time-pos" => json!(12.5),
                    "duration" => json!(60),
                    "volume" => json!(23),
                    "pause" => json!(true),
                    "speed" => json!(1.25),
                    _ => Value::Null,
                };
                write.write_all(format!("{}\n",json!({"request_id":request["request_id"],"error":"success","data":data})).as_bytes()).await.unwrap();
            }
        });
        let status = properties(&socket).await.unwrap();
        assert_eq!(status["time-pos"], 12.5);
        assert_eq!(status["pause"], true);
        server.await.unwrap();
        let listener = tokio::net::UnixListener::bind(temp.path().join("silent")).unwrap();
        let silent = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let _stream = stream;
            std::future::pending::<()>().await;
        });
        let start = std::time::Instant::now();
        assert!(properties(&temp.path().join("silent")).await.is_err());
        assert!(
            start.elapsed() < Duration::from_millis(350),
            "status polling multiplied timeouts and delayed controls"
        );
        silent.abort();
        let _ = silent.await;
    }
    #[tokio::test]
    async fn station_switch_and_restart_keep_volume_mute_and_speed() {
        let temp = tempfile::tempdir().unwrap();
        let paths = paths(temp.path());
        paths.create().unwrap();
        let script = temp.path().join("mpv");
        let args = temp.path().join("args");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\nsleep 30\n",
                args.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut actor = Actor {
            config: Config {
                mpv: script.display().to_string(),
                ..Default::default()
            },
            paths: paths.clone(),
            session: None,
            full: None,
            preview: None,
            state: PlayerStatus {
                volume: 23.0,
                muted: true,
                speed: 1.25,
                ..Default::default()
            },
            resume_after_preview: false,
            playing: None,
            reconnect_count: 0,
            reconnect_at: None,
            sponsor_task: None,
            ignore_segments_until: 0.0,
        };
        let media = Media {
            url: "https://radio.test/live".into(),
            kind: ytui_core::MediaKind::Station,
            live: true,
            ..Default::default()
        };
        actor.play(media.clone(), true, false, None, false).await;
        actor.state.sponsor_enabled = false;
        let mut appearance = actor.config.clone();
        appearance.theme = "ytui-light".into();
        actor
            .handle(PlayerCommand::Configure(Box::new(appearance)))
            .await;
        assert!(
            !actor.state.sponsor_enabled,
            "appearance changes must preserve a temporary skip override"
        );
        assert!(
            actor.sponsor_task.is_none(),
            "appearance changes must not refetch SponsorBlock"
        );
        for _ in 0..100 {
            if args.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(actor.state.volume, 23.0);
        assert!(
            std::fs::read_to_string(&args)
                .unwrap()
                .contains("--volume=23\n")
        );
        let listener =
            tokio::net::UnixListener::bind(&actor.full.as_ref().unwrap().socket).unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (read, mut write) = stream.into_split();
            let line = BufReader::new(read)
                .lines()
                .next_line()
                .await
                .unwrap()
                .unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(request["command"], json!(["add", "volume", -5.0]));
            write
                .write_all(b"{\"request_id\":1,\"error\":\"success\",\"data\":null}\n")
                .await
                .unwrap();
        });
        actor.handle(PlayerCommand::Volume(-5.0)).await;
        server.await.unwrap();
        assert_eq!(actor.state.volume, 18.0);
        actor.play(media, true, false, None, false).await;
        for _ in 0..100 {
            if std::fs::read_to_string(&args)
                .unwrap_or_default()
                .contains("--volume=18\n")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let recorded = std::fs::read_to_string(&args).unwrap();
        assert!(recorded.contains("--volume=18\n"));
        assert!(recorded.contains("--mute=yes\n"));
        assert!(recorded.contains("--speed=1.25\n"));
        actor.preferences().save(&paths).await;
        actor.stop().await;
        let saved = Preferences::load(&paths);
        assert_eq!(saved.volume, 18.0);
        assert!(saved.muted);
        assert_eq!(saved.speed, 1.25);
    }
}

#[cfg(test)]
mod real_mpv_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    async fn ready(actor: &mut Actor) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            actor.tick().await;
            if actor.state.duration.is_some() && !actor.state.buffering {
                break;
            }
            assert!(
                actor.state.error.is_none(),
                "mpv failed: {:?}",
                actor.state.error
            );
            assert!(
                tokio::time::Instant::now() < deadline,
                "mpv did not become ready"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    #[tokio::test]
    #[ignore = "requires modern mpv and FFmpeg; uses generated media, null video/audio outputs and no network"]
    async fn real_mpv_controls_window_menu_and_sponsor_replay_without_an_audio_device() {
        let mpv = std::env::var("YTUI_TEST_MPV").unwrap_or_else(|_| "mpv".into());
        let program =
            crate::paths::executable(&mpv).expect("Install modern mpv or set YTUI_TEST_MPV");
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("fixture.mkv");
        let subtitles = temp.path().join("fixture.srt");
        std::fs::write(
            &subtitles,
            "1\n00:00:00,000 --> 00:00:10,000\nSynthetic subtitle\n",
        )
        .unwrap();
        let output = std::process::Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=black:s=160x90:r=10",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=8000",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=880:sample_rate=8000",
                "-i",
                subtitles.to_str().unwrap(),
                "-map",
                "0:v",
                "-map",
                "1:a",
                "-map",
                "2:a",
                "-map",
                "3:s",
                "-t",
                "12",
                "-c:v",
                "mpeg4",
                "-threads",
                "1",
                "-c:a",
                "pcm_s16le",
                "-c:s",
                "srt",
                source.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let original = std::fs::read(&source).unwrap();
        let wrapper = temp.path().join("mpv-headless");
        // A test-only wrapper adds null outputs. Production process supervision,
        // IPC, startup arguments, menu script and actor are otherwise unchanged.
        let quoted = program.to_string_lossy().replace('\'', "'\\''");
        std::fs::write(
            &wrapper,
            format!("#!/bin/sh\nexec '{quoted}' --vo=null --ao=null \"$@\"\n"),
        )
        .unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
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
        let mut actor = Actor {
            config: Config {
                mpv: wrapper.display().to_string(),
                offline: true,
                ..Default::default()
            },
            paths: paths.clone(),
            session: None,
            full: None,
            preview: None,
            state: PlayerStatus {
                volume: 37.0,
                speed: 1.0,
                ..Default::default()
            },
            resume_after_preview: false,
            playing: None,
            reconnect_count: 0,
            reconnect_at: None,
            sponsor_task: None,
            ignore_segments_until: 0.0,
        };
        actor
            .play(
                Media {
                    title: "Synthetic fixture".into(),
                    url: source.display().to_string(),
                    ..Default::default()
                },
                false,
                false,
                None,
                false,
            )
            .await;
        ready(&mut actor).await;
        let started = std::time::Instant::now();
        actor.handle(PlayerCommand::TogglePause).await;
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "pause was delayed"
        );
        assert_eq!(
            actor
                .full
                .as_ref()
                .unwrap()
                .command(json!(["get_property", "pause"]))
                .await
                .unwrap(),
            true
        );
        actor.handle(PlayerCommand::Volume(-5.0)).await;
        actor.handle(PlayerCommand::Mute).await;
        actor.handle(PlayerCommand::Speed(0.25)).await;
        actor.handle(PlayerCommand::SeekTo(4.0)).await;
        actor.tick().await;
        assert_eq!(actor.state.volume, 32.0);
        assert!(actor.state.muted);
        assert_eq!(actor.state.speed, 1.25);
        assert!((actor.state.position - 4.0).abs() < 0.3);
        let audio = actor
            .full
            .as_ref()
            .unwrap()
            .command(json!(["get_property", "aid"]))
            .await
            .unwrap();
        actor.handle(PlayerCommand::CycleAudio).await;
        assert_ne!(
            actor
                .full
                .as_ref()
                .unwrap()
                .command(json!(["get_property", "aid"]))
                .await
                .unwrap(),
            audio
        );
        let subtitle = actor
            .full
            .as_ref()
            .unwrap()
            .command(json!(["get_property", "sid"]))
            .await
            .unwrap();
        actor.handle(PlayerCommand::CycleSubtitles).await;
        assert_ne!(
            actor
                .full
                .as_ref()
                .unwrap()
                .command(json!(["get_property", "sid"]))
                .await
                .unwrap(),
            subtitle
        );
        actor.handle(PlayerCommand::ToggleSubtitles).await;
        assert_eq!(
            actor
                .full
                .as_ref()
                .unwrap()
                .command(json!(["get_property", "sub-visibility"]))
                .await
                .unwrap(),
            false
        );
        let old_socket = actor.full.as_ref().unwrap().socket.clone();
        {
            let player = actor.full.as_ref().unwrap();
            let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
            while !player
                .command(json!(["get_property", "input-bindings"]))
                .await
                .unwrap()
                .to_string()
                .contains("ytui-player-menu")
            {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "mpv did not load the window menu script"
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            player
                .command(json!(["script-message", "ytui-menu"]))
                .await
                .unwrap();
            let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
            while !player
                .command(json!(["get_property", "input-bindings"]))
                .await
                .unwrap()
                .to_string()
                .contains("ytui-menu-down")
            {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "mpv did not open the window menu"
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            player.command(json!(["keypress", "DOWN"])).await.unwrap();
            player.command(json!(["keypress", "ENTER"])).await.unwrap();
            let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
            loop {
                if player
                    .command(json!(["get_property", "user-data/ytui/action"]))
                    .await
                    .ok()
                    == Some(json!("quality:best"))
                {
                    break;
                }
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "the real mpv Lua menu did not request quality"
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
        actor.tick().await;
        ready(&mut actor).await;
        assert_ne!(actor.full.as_ref().unwrap().socket, old_socket);
        assert_eq!(actor.config.download.quality, "best");
        assert!(
            actor.state.paused,
            "changing video quality should keep a paused video paused"
        );
        assert_eq!(
            actor
                .full
                .as_ref()
                .unwrap()
                .command(json!(["get_property", "pause"]))
                .await
                .unwrap(),
            true
        );
        assert_eq!(actor.state.volume, 32.0);
        assert!(actor.state.muted);
        assert_eq!(actor.state.speed, 1.25);
        actor.state.segments = vec![crate::sponsorblock::Segment {
            start: 5.0,
            end: 7.0,
            category: "sponsor".into(),
        }];
        actor.state.sponsor_enabled = true;
        actor.handle(PlayerCommand::TogglePause).await;
        actor.handle(PlayerCommand::SeekTo(5.5)).await;
        // The seek command is acknowledged before mpv's decoder updates time-pos.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            actor.tick().await;
            if actor.state.last_skip.is_some() {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "real mpv was not asked to skip the marked range"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        actor.handle(PlayerCommand::UndoSkip).await;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            actor.tick().await;
            if actor.state.position < 6.5 {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "undo did not replay the marked range"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        actor.tick().await;
        assert!(
            actor.state.position < 6.5,
            "undo immediately re-skipped the range"
        );
        actor.preferences().save(&paths).await;
        actor.stop().await;
        assert_eq!(Preferences::load(&paths).volume, 32.0);
        assert_eq!(
            std::fs::read(&source).unwrap(),
            original,
            "playback controls changed the source file"
        );
    }
    #[tokio::test]
    #[ignore = "requires modern mpv and FFmpeg; uses synthetic extraction and loopback streams only"]
    async fn real_mpv_external_playback_uses_split_streams_and_extractor_http_headers() {
        let mpv = std::env::var("YTUI_TEST_MPV").unwrap_or_else(|_| "mpv".into());
        let program = crate::paths::executable(&mpv).expect("Install mpv or set YTUI_TEST_MPV");
        let temp = tempfile::tempdir().unwrap();
        let video = temp.path().join("video.mkv");
        let audio = temp.path().join("audio.wav");
        for (source, output, codec) in [
            ("color=c=black:s=160x90:r=10", &video, "mpeg4"),
            ("sine=frequency=440:sample_rate=8000", &audio, "pcm_s16le"),
        ] {
            let result = std::process::Command::new("ffmpeg")
                .args([
                    "-v",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    source,
                    "-t",
                    "5",
                    "-threads",
                    "1",
                    "-c",
                    codec,
                    output.to_str().unwrap(),
                ])
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
        let video_bytes = std::fs::read(video).unwrap();
        let audio_bytes = std::fs::read(audio).unwrap();
        let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let observed = requests.clone();
        let (base, server) = crate::test_http::serve_request(move |request| {
            let lower = request.to_ascii_lowercase();
            assert!(
                lower.contains("user-agent: synthetic-stream-agent"),
                "extractor UA missing"
            );
            assert!(
                lower.contains("referer: https://www.youtube.com/"),
                "extractor Referer missing"
            );
            assert!(
                lower.contains("accept-language: en-us,en;q=0.9"),
                "comma-containing header changed: {lower}"
            );
            observed
                .lock()
                .unwrap()
                .push(request.lines().next().unwrap().to_string());
            let bytes = if request.starts_with("GET /audio") { &audio_bytes } else { &video_bytes };
            let range = lower.lines().find_map(|line| line.strip_prefix("range: "))
                .and_then(|value| value.strip_prefix("bytes="))
                .and_then(|value| value.split_once('-'))
                .and_then(|(start,end)| Some((start.parse::<usize>().ok()?,end.parse::<usize>().unwrap_or(bytes.len()-1))));
            if let Some((start,end)) = range {
                if start>=bytes.len() {return (416,format!("Content-Range: bytes */{}\r\n",bytes.len()),vec![]);}
                let end = end.min(bytes.len()-1);
                return (206,format!("Content-Type: application/octet-stream\r\nAccept-Ranges: bytes\r\nContent-Range: bytes {start}-{end}/{}\r\n",bytes.len()),bytes[start..=end].to_vec());
            }
            (200,"Content-Type: application/octet-stream\r\nAccept-Ranges: bytes\r\n".into(),bytes.clone())
        })
        .await;
        let metadata = temp.path().join("metadata.json");
        std::fs::write(&metadata,json!({"http_headers":{"User-Agent":"synthetic-stream-agent","Referer":"https://www.youtube.com/","Accept-Language":"en-US,en;q=0.9"},"requested_formats":[
            {"url":format!("{base}/video"),"vcodec":"mpeg4","acodec":"none"},
            {"url":format!("{base}/audio"),"vcodec":"none","acodec":"pcm_s16le"}]}).to_string()).unwrap();
        let extractor = temp.path().join("yt-dlp");
        std::fs::write(
            &extractor,
            format!("#!/bin/sh\ncat '{}'\n", metadata.display()),
        )
        .unwrap();
        std::fs::set_permissions(&extractor, std::fs::Permissions::from_mode(0o755)).unwrap();
        let wrapper = temp.path().join("mpv-headless");
        let quoted = program.to_string_lossy().replace('\'', "'\\''");
        std::fs::write(
            &wrapper,
            format!("#!/bin/sh\nexec '{quoted}' --vo=null --ao=null \"$@\"\n"),
        )
        .unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
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
        let mut actor = Actor {
            config: Config {
                mpv: wrapper.display().to_string(),
                yt_dlp: extractor.display().to_string(),
                download_backend: "yt-dlp".into(),
                playback_sponsorblock: "off".into(),
                ..Default::default()
            },
            paths,
            session: None,
            full: None,
            preview: None,
            state: PlayerStatus {
                volume: 37.0,
                speed: 1.0,
                ..Default::default()
            },
            resume_after_preview: false,
            playing: None,
            reconnect_count: 0,
            reconnect_at: None,
            sponsor_task: None,
            ignore_segments_until: 0.0,
        };
        let valid_metadata = std::fs::read(&metadata).unwrap();
        std::fs::write(&metadata, b"invalid metadata").unwrap();
        actor
            .play(
                Media {
                    url: "https://www.youtube.com/watch?v=synthetic00".into(),
                    ..Default::default()
                },
                false,
                false,
                None,
                false,
            )
            .await;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        loop {
            actor.tick().await;
            if actor.state.error.is_some() {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "bad extraction did not report an error"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(!actor.state.active);
        std::fs::write(&metadata, valid_metadata).unwrap();
        for audio_only in [false, true] {
            actor
                .play(
                    Media {
                        title: "Synthetic YouTube playback".into(),
                        url: "https://www.youtube.com/watch?v=synthetic00".into(),
                        ..Default::default()
                    },
                    audio_only,
                    false,
                    Some(1.0),
                    true,
                )
                .await;
            ready(&mut actor).await;
            assert!(actor.state.error.is_none());
            let tracks = actor
                .full
                .as_ref()
                .unwrap()
                .command(json!(["get_property", "track-list"]))
                .await
                .unwrap();
            assert!(
                tracks
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|track| track["type"] == "audio"),
                "audio missing: {tracks}"
            );
            if !audio_only {
                assert!(
                    tracks
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|track| track["type"] == "video"),
                    "video missing: {tracks}"
                );
            }
            assert!(actor.state.paused, "resume start lost pause state");
            // Duration is available before the initial seek/frame has settled.
            let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
            while (actor.state.position - 1.0).abs() >= 0.4 {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "resume position was lost: {}",
                    actor.state.position
                );
                actor.tick().await;
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            if !audio_only {
                actor
                    .preview(Media {
                        url: "https://www.youtube.com/watch?v=synthetic00".into(),
                        ..Default::default()
                    })
                    .await;
                let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
                loop {
                    let preview = actor.preview.as_ref().expect("preview did not start");
                    if preview
                        .command(json!(["get_property", "duration"]))
                        .await
                        .ok()
                        .is_some_and(|duration| duration.as_f64().is_some())
                    {
                        break;
                    }
                    assert!(!preview.task.is_finished(), "preview exited unexpectedly");
                    assert!(
                        tokio::time::Instant::now() < deadline,
                        "preview did not load"
                    );
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                assert_eq!(
                    actor
                        .preview
                        .as_ref()
                        .unwrap()
                        .command(json!(["get_property", "mute"]))
                        .await
                        .unwrap(),
                    true
                );
                actor.close_preview().await;
                assert_eq!(
                    actor
                        .full
                        .as_ref()
                        .unwrap()
                        .command(json!(["get_property", "pause"]))
                        .await
                        .unwrap(),
                    true
                );
            }
            actor.handle(PlayerCommand::TogglePause).await;
            assert!(!actor.state.paused);
            actor.stop().await;
        }
        assert!(
            requests
                .lock()
                .unwrap()
                .iter()
                .any(|line| line.contains("/video"))
        );
        assert!(
            requests
                .lock()
                .unwrap()
                .iter()
                .any(|line| line.contains("/audio"))
        );
        // Real mpv reports file-open errors on stdout. They must survive capture.
        let result = run_mpv(
            ProcessSpec::new(
                wrapper.display().to_string(),
                vec![
                    "--no-config".into(),
                    "--terminal=yes".into(),
                    "--input-terminal=no".into(),
                    "--msg-level=all=warn".into(),
                    "--ytdl=no".into(),
                    "--".into(),
                    temp.path().join("missing.mkv").display().to_string(),
                ],
                5,
            ),
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert!(!result.success);
        assert!(
            result.stderr.contains("Failed to open") || result.stderr.contains("Cannot open"),
            "lost mpv error: {}",
            result.stderr
        );
        server.abort();
    }
}

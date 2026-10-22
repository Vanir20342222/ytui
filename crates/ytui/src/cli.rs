use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use std::{path::PathBuf, sync::Arc};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use ytui_core::*;
use ytui_services::{
    auth::{self, SessionStore},
    browse::Browser,
    paths::Paths,
    radio::Radio,
    store::Store,
};

#[derive(Parser)]
#[command(
    version,
    about = "YouTube, Music, downloads and radio in your terminal"
)]
pub struct Args {
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,
    #[arg(long, global = true)]
    pub state_dir: Option<PathBuf>,
    #[arg(long)]
    pub demo: bool,
    /// Start with local downloads; disable account access, remote playback and background network work.
    #[arg(long)]
    pub offline: bool,
    #[command(subcommand)]
    pub command: Option<Command>,
}
#[derive(Subcommand)]
pub enum Command {
    /// Check installed tools, desktop themes and configuration without reading browser stores.
    Doctor {
        /// Also probe public YouTube, Music and radio requests, without importing a browser session.
        #[arg(long)]
        network: bool,
    },
    /// Write a documented TOML configuration (never overwrites an existing file).
    Init,
    /// Locate browser profiles, import a session, or disconnect.
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
    },
    /// Browse YouTube or Music and print normalized JSON.
    Browse {
        #[arg(long)]
        music: bool,
        #[arg(default_value = "home")]
        feed: String,
        query: Option<String>,
    },
    /// Check actual mpv playback of a public URL without loading browser cookies.
    PlaybackCheck {
        url: String,
        #[arg(long)]
        audio: bool,
        #[arg(long, value_parser = ["auto", "native", "yt-dlp"])]
        backend: Option<String>,
        /// Require this many seconds of playback before reporting success.
        #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u32).range(2..=30))]
        seconds: u32,
    },
    /// Explicit control of existing NetworkManager VPN/WireGuard profiles (never runs at startup).
    Vpn {
        #[arg(default_value = "list", value_parser = ["list", "status", "connect", "disconnect"])]
        action: String,
        uuid: Option<String>,
    },
    /// Print cliamp channels or Radio Browser search results.
    Radio {
        query: Option<String>,
        #[arg(long)]
        country: Option<String>,
        #[arg(long)]
        tag: Option<String>,
    },
    /// Download with Rust first, falling back for supported extractor limitations in auto mode.
    Download {
        url: String,
        #[arg(long)]
        audio: bool,
        #[arg(long)]
        quality: Option<String>,
        #[arg(long)]
        audio_quality: Option<String>,
        #[arg(short = 'o', long)]
        directory: Option<PathBuf>,
        #[arg(long)]
        playlist: bool,
        #[arg(long)]
        items: Option<String>,
        #[arg(long)]
        backend: Option<String>,
        /// Explicitly bypass the configured daily download window for this headless job.
        #[arg(long)]
        now: bool,
    },
    /// Preview/import Python state without modifying the original files.
    Migrate {
        #[arg(long)]
        apply: bool,
    },
    /// Add URLs from a Rust/legacy JSON export or newline-separated text file.
    Import { file: PathBuf },
    /// Import cliamp stations/favorites; preserves the original cliamp files.
    ImportRadio { file: PathBuf },
    #[command(name = "__native-worker", hide = true)]
    NativeWorker { request: PathBuf },
}
#[derive(Subcommand)]
pub enum AuthCommand {
    /// List profile directories without reading their cookies.
    Profiles,
    /// Validate a selected browser/cookie-file session before storing it.
    Import {
        #[arg(long, default_value = "brave-origin")]
        browser: String,
        #[arg(long)]
        profile: Option<PathBuf>,
        #[arg(long)]
        cookies: Option<PathBuf>,
        #[arg(long)]
        file_store: bool,
    },
    Status,
    Disconnect,
}
async fn load_session(config: &Config, paths: &Paths) -> Option<Arc<auth::Session>> {
    let store = SessionStore::new(paths, &config.auth_store);
    match tokio::task::spawn_blocking(move || store.load()).await {
        Ok(Ok(s)) => s.map(Arc::new),
        _ => None,
    }
}
pub async fn run(command: Command, mut config: Config, paths: Paths) -> Result<()> {
    match command {
        Command::PlaybackCheck {
            url,
            audio,
            backend,
            seconds,
        } => {
            ensure!(!config.offline, "Playback checks require network access");
            if let Some(backend) = backend {
                config.download_backend = backend;
            }
            config.playback_sponsorblock = "off".into();
            let media = Media {
                url: normalize_media_url(&url)?,
                title: "Public playback check".into(),
                ..Default::default()
            };
            let mut player = ytui_services::player::Player::spawn(config, paths, None);
            player
                .commands
                .send(ytui_services::player::PlayerCommand::Play { media, audio })
                .await?;
            let result = tokio::time::timeout(std::time::Duration::from_secs(120), async {
                loop {
                    let state = player.status.borrow_and_update().clone();
                    if let Some(error) = state.error {
                        anyhow::bail!("{}", ytui_services::process::diagnostic(&error));
                    }
                    if state.active
                        && !state.paused
                        && !state.buffering
                        && state.position >= f64::from(seconds)
                    {
                        return Ok(serde_json::json!({
                            "backend": state.backend,
                            "audio_only": audio,
                            "position": state.position,
                            "duration": state.duration,
                        }));
                    }
                    tokio::select! {
                        changed = player.status.changed() => changed.context("Playback worker stopped")?,
                        event = player.events.recv() => match event {
                            Some(ytui_services::player::PlayerEvent::Error(error)) => anyhow::bail!("{}", ytui_services::process::diagnostic(&error)),
                            _ => anyhow::bail!("Playback ended before the required progress"),
                        },
                        _ = tokio::signal::ctrl_c() => anyhow::bail!("Playback check interrupted"),
                    }
                }
            })
            .await;
            player.close().await;
            println!(
                "{}",
                result.context("Playback did not advance within 120 seconds")??
            );
        }
        Command::NativeWorker { request } => {
            tokio::task::spawn_blocking(move || ytui_services::native::worker(&request)).await??
        }
        Command::Init => {
            ensure!(
                !paths.config_file().exists(),
                "rust.toml already exists; it was not overwritten"
            );
            ytui_services::paths::atomic_write_new(
                &paths.config_file(),
                toml::to_string_pretty(&config)?.as_bytes(),
            )?;
            println!("{}", paths.config_file().display());
        }
        Command::Doctor { network } => {
            ensure!(
                !network || !config.offline,
                "Network diagnostics require network access; restart without --offline"
            );
            println!("ytui {} · Rust application", env!("CARGO_PKG_VERSION"));
            println!("Executable: {}", std::env::current_exe()?.display());
            println!("Configuration: {}", paths.config_file().display());
            println!("State: {}", paths.state.display());
            println!(
                "Backend: {} (auto uses yt-dlp for supported native limitations)",
                config.download_backend
            );
            println!(
                "yt-dlp-rs revision: {}",
                ytui_services::native::UPSTREAM_REV
            );
            for (label, tool) in [
                ("FFmpeg", &config.ffmpeg),
                ("FFprobe (chapters/SponsorBlock)", &config.ffprobe),
                ("mpv", &config.mpv),
                ("yt-dlp (required for auto fallback)", &config.yt_dlp),
                ("cliamp (optional)", &config.cliamp),
            ] {
                println!(
                    "{label}: {}",
                    if executable(tool).is_some() {
                        "found"
                    } else {
                        "missing"
                    }
                );
            }
            println!(
                "Deno runtime: {}",
                if config
                    .native_js_runtime
                    .as_ref()
                    .is_some_and(|p| p.is_file())
                {
                    "configured"
                } else {
                    "not configured (native YouTube challenges may fail)"
                }
            );
            println!(
                "Terminal: {} · image mode {}",
                std::env::var("TERM").unwrap_or_else(|_| "unknown".into()),
                config.image_protocol
            );
            match ytui_services::theme::resolve(&config, &paths).await {
                Ok(p) => println!("Theme: {} · {}", p.name, p.source),
                Err(_) => println!(
                    "Theme provider unavailable; last valid palette/default will be retained"
                ),
            }
            println!(
                "Clipboard watcher: {}",
                if !config.clipboard_watcher {
                    "disabled (opt-in)"
                } else if ytui_services::desktop::clipboard_reader().is_ok() {
                    "enabled; reader found"
                } else {
                    "enabled; install wl-clipboard or xclip/xsel"
                }
            );
            println!(
                "Download window: {} · bandwidth window: {}",
                config
                    .schedule
                    .download_window
                    .as_deref()
                    .unwrap_or("all day"),
                config
                    .schedule
                    .bandwidth_window
                    .as_deref()
                    .unwrap_or("none")
            );
            println!(
                "Browser stores were not opened. Use `ytui-rs auth profiles` to list profile paths."
            );
            if network {
                let client = ytui_services::http_client(config.proxy.as_deref())?;
                let browser = Browser::new(client.clone(), None, 0);
                let mut failures = 0;
                for (label, music) in [
                    ("YouTube public search", false),
                    ("Music public search", true),
                ] {
                    failures += usize::from(!print_probe(
                        label,
                        browser
                            .fetch(music, &BrowseRequest::Search("music".into()))
                            .await,
                    ));
                }
                failures += usize::from(!print_probe(
                    "Radio catalog/directory",
                    Radio::new(client, config.radio_servers.clone())
                        .fetch(&BrowseRequest::Cliamp, 0)
                        .await,
                ));
                println!(
                    "These public probes do not validate signed-in feeds or media playback. VPN configuration was not changed."
                );
                ensure!(
                    failures == 0,
                    "{failures} public network probe(s) failed or returned no supported items; see the results above"
                );
            }
        }
        Command::Auth { command } => match command {
            AuthCommand::Profiles => {
                for p in auth::profiles(&paths) {
                    println!("{}\t{}", p.browser, p.path.display());
                }
            }
            AuthCommand::Import {
                browser,
                profile,
                cookies,
                file_store,
            } => {
                if file_store {
                    config.auth_store = "file".into();
                }
                let mut session = tokio::task::spawn_blocking(move || {
                    if let Some(file) = cookies {
                        auth::import_netscape(&file)
                    } else {
                        auth::import_browser(&browser, profile.as_deref())
                    }
                })
                .await??;
                let api = Browser::new(
                    ytui_services::http_client(config.proxy.as_deref())?,
                    Some(Arc::new(session.clone())),
                    config.account_index,
                );
                session.account = api.validate().await?;
                let store = SessionStore::new(&paths, &config.auth_store);
                let account = session.account.clone();
                tokio::task::spawn_blocking(move || store.save(&session)).await??;
                if file_store {
                    paths.save_config(&config)?;
                }
                println!("Connected as {account}");
            }
            AuthCommand::Status => match load_session(&config, &paths).await {
                Some(s) => println!("Stored session for {} (not live-validated)", s.account),
                None => println!(
                    "No accessible session; unlock the keyring or import a browser profile"
                ),
            },
            AuthCommand::Disconnect => {
                let store = SessionStore::new(&paths, &config.auth_store);
                tokio::task::spawn_blocking(move || store.disconnect()).await??;
                println!("Disconnected; browser profile unchanged");
            }
        },
        Command::Browse { music, feed, query } => {
            let session = load_session(&config, &paths).await;
            let browser = Browser::new(
                ytui_services::http_client(config.proxy.as_deref())?,
                session,
                config.account_index,
            );
            let request = match feed.as_str() {
                "home" => BrowseRequest::Home,
                "search" => BrowseRequest::Search(query.context("Provide a search query")?),
                "library" => BrowseRequest::Library,
                "liked" => BrowseRequest::Liked,
                "later" => BrowseRequest::WatchLater,
                "subscriptions" => BrowseRequest::Subscriptions,
                "history" => BrowseRequest::History,
                "collection" => {
                    BrowseRequest::Collection(query.context("Provide a collection ID")?)
                }
                "details" => BrowseRequest::Details(query.context("Provide a video ID")?),
                "related" => BrowseRequest::Related(query.context("Provide a video ID")?),
                "songs" | "albums" | "artists" | "videos" | "playlists" if music => {
                    BrowseRequest::MusicSearch {
                        query: query.context("Provide a Music search query")?,
                        category: feed.clone(),
                    }
                }
                _ => anyhow::bail!("Unknown feed"),
            };
            println!(
                "{}",
                serde_json::to_string_pretty(&browser.fetch(music, &request).await?)?
            );
        }
        Command::Vpn { action, uuid } => {
            let cancel = CancellationToken::new();
            if action == "connect" || action == "disconnect" {
                let id = uuid.context("Provide a UUID from `ytui-rs vpn list`")?;
                ytui_services::vpn::change(&config.nmcli, &id, action == "connect", cancel).await?;
                println!("VPN action completed");
            } else {
                println!(
                    "{}",
                    serde_json::to_string_pretty(
                        &ytui_services::vpn::list(&config.nmcli, cancel).await?
                    )?
                );
            }
        }
        Command::Radio {
            query,
            country,
            tag,
        } => {
            let request = if let Some(country) = country {
                BrowseRequest::RadioCountry(country)
            } else if let Some(tag) = tag {
                BrowseRequest::RadioTag(tag)
            } else if let Some(query) = query {
                BrowseRequest::RadioSearch(query)
            } else {
                BrowseRequest::Cliamp
            };
            let radio = Radio::new(
                ytui_services::http_client(config.proxy.as_deref())?,
                config.radio_servers.clone(),
            )
            .with_cache(paths.cache.join("radio"));
            println!(
                "{}",
                serde_json::to_string_pretty(&radio.fetch(&request, 0).await?)?
            );
        }
        Command::Download {
            url,
            audio,
            quality,
            audio_quality,
            directory,
            playlist,
            items,
            backend,
            now,
        } => {
            let policy = config.schedule.policy(
                ytui_services::desktop::local_minute(),
                config.download.global_bandwidth_kib,
                1,
            );
            ensure!(
                policy.allowed || now,
                "Outside the configured download window; use --now to bypass it explicitly"
            );
            config.concurrency = 1; // This command owns exactly one transfer.
            config.download.global_bandwidth_kib = policy.global_bandwidth_kib;
            let normalized = if std::path::Path::new(&url).is_file() {
                std::fs::canonicalize(&url)?.to_string_lossy().into_owned()
            } else {
                normalize_media_url(&url)?
            };
            let mut options = config.download.clone();
            options.audio = audio;
            options.playlist = playlist || items.is_some() || is_playlist_url(&normalized);
            options.playlist_items = items;
            options.backend = backend;
            if let Some(q) = quality {
                options.quality = q;
            }
            if let Some(q) = audio_quality {
                options.audio_quality = q;
            }
            if let Some(dir) = directory {
                if audio {
                    options.audio_directory = dir;
                } else {
                    options.directory = dir;
                }
            }
            options.validate()?;
            let session = if std::path::Path::new(&normalized).is_file() {
                None
            } else {
                load_session(&config, &paths).await
            };
            let store = Store::open(&paths.state)?;
            let mut snapshot = store.load().await?;
            let mut job = Job::new(
                Media {
                    title: "Resolving media…".into(),
                    url: normalized,
                    ..Default::default()
                },
                options,
            );
            job.state = JobState::Resolving;
            let id = job.id;
            snapshot.downloads.jobs.push(job.clone());
            store.save(snapshot.clone()).await?;
            let (tx, mut rx) = mpsc::channel(128);
            let cancel = CancellationToken::new();
            let worker = tokio::spawn(ytui_services::download::execute(
                job,
                config,
                paths,
                session,
                cancel.clone(),
                tx,
            ));
            let mut failure = None;
            let mut interrupted = false;
            let mut terminal_received = false;
            loop {
                tokio::select! {
                    event=rx.recv()=>{let Some(event)=event else{break;};let terminal=matches!(event.update,JobUpdate::Finished|JobUpdate::Failed{..});snapshot.downloads.apply(event);
                        if terminal{terminal_received=true;let job=snapshot.downloads.jobs.iter().find(|j|j.id==id).context("Download disappeared")?;if job.state==JobState::Done{for p in &job.outputs{println!("Saved {}",p.display());}}else{failure=job.error.clone();}break;}
                    },
                    _=tokio::signal::ctrl_c()=>{interrupted=true;cancel.cancel();if let Some(j)=snapshot.downloads.jobs.iter_mut().find(|j|j.id==id){j.invalidate(JobState::Paused);}break;}
                }
            }
            rx.close();
            cancel.cancel();
            let joined = worker.await;
            if !interrupted && (!terminal_received || joined.is_err()) {
                failure = Some(
                    "Download worker exited without a terminal result; retry the saved job".into(),
                );
                if let Some(job) = snapshot.downloads.jobs.iter_mut().find(|j| j.id == id) {
                    job.invalidate(JobState::Error);
                    job.error = failure.clone();
                }
            }
            store.save(snapshot).await?;
            store.close().await?;
            if interrupted {
                anyhow::bail!("Download interrupted; paused job saved");
            }
            if let Some(error) = failure {
                anyhow::bail!("{error}");
            }
        }
        Command::Migrate { apply } => {
            let store = Store::open(&paths.state)?;
            let mut snapshot = store.load().await?;
            let p = paths.clone();
            let mut c = config.clone();
            let (result, snapshot, new_config) = tokio::task::spawn_blocking(move || {
                let result = ytui_services::migration::import(&p, &mut c, &mut snapshot, apply);
                (result, snapshot, c)
            })
            .await?;
            let report = result?;
            if apply {
                store.save(snapshot).await?;
                config = new_config;
                let _ = &config;
            }
            store.close().await?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Import { file } => {
            let c = config.clone();
            let jobs =
                tokio::task::spawn_blocking(move || ytui_services::queue_import::read(&file, &c))
                    .await??;
            let store = Store::open(&paths.state)?;
            let mut snapshot = store.load().await?;
            let count = ytui_services::queue_import::append(&mut snapshot.downloads, jobs);
            store.save(snapshot).await?;
            store.close().await?;
            println!("Imported {count} queued items; start the TUI to run them");
        }
        Command::ImportRadio { file } => {
            let raw = std::fs::read_to_string(file)?;
            let value: toml::Value = toml::from_str(&raw)?;
            let mut imported = 0;
            for key in ["station", "stations", "radio", "favorites"] {
                if let Some(entries) = value.get(key).and_then(toml::Value::as_array) {
                    for station in entries {
                        let Some(url) = station
                            .get("url")
                            .or_else(|| station.get("stream"))
                            .and_then(toml::Value::as_str)
                        else {
                            continue;
                        };
                        if http_url(url).is_err()
                            || config.custom_stations.iter().any(|s| s.url == url)
                        {
                            continue;
                        }
                        config.custom_stations.push(Media {
                            id: url.into(),
                            url: url.into(),
                            title: station
                                .get("name")
                                .and_then(toml::Value::as_str)
                                .unwrap_or("Custom station")
                                .into(),
                            kind: MediaKind::Station,
                            live: true,
                            ..Default::default()
                        });
                        imported += 1;
                    }
                }
            }
            paths.save_config(&config)?;
            println!("Imported {imported} stations; originals were left intact");
        }
    }
    Ok(())
}

fn print_probe(label: &str, result: Result<Page>) -> bool {
    match result {
        Ok(page) if !page.items.is_empty() => {
            println!("{label}: OK ({} items)", page.items.len());
            if let Some(notice) = page.notice {
                println!("  {}", ytui_services::process::diagnostic(&notice));
            }
            true
        }
        Ok(page) => {
            println!(
                "{label}: EMPTY · {}",
                ytui_services::process::diagnostic(
                    page.notice
                        .as_deref()
                        .unwrap_or("No supported items returned")
                )
            );
            false
        }
        Err(error) => {
            println!(
                "{label}: FAILED · {}",
                ytui_services::process::diagnostic(&error.to_string())
            );
            false
        }
    }
}
pub fn executable(program: &str) -> Option<PathBuf> {
    let p = PathBuf::from(program);
    if p.components().count() > 1 {
        return p.is_file().then_some(p);
    }
    std::env::var_os("PATH").and_then(|v| {
        std::env::split_paths(&v)
            .map(|dir| dir.join(program))
            .find(|p| p.is_file())
    })
}

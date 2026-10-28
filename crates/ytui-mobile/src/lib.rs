//! Android application engine. JNI entrypoints run on platform worker threads.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};
use tokio_util::sync::CancellationToken;
use ytui_core::{BrowseRequest, Config, Media, Page};
use ytui_providers::{
    auth::Session, browse::Browser, images::ImageCache, paths::atomic_write, radio::Radio,
};
mod account;
mod lyrics;
mod resolution;

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MobileState {
    pub saved: Vec<Media>,
    pub offline_playlists: Vec<ytui_core::offline::OfflinePlaylist>,
    pub queue: Vec<Media>,
    pub positions: HashMap<String, u64>,
    pub playback: ytui_core::PlaybackQueue,
    pub recent: Vec<Media>,
}
struct PlaybackContext {
    id: String,
    items: std::collections::VecDeque<Media>,
    prepared: bool,
    consumed: std::collections::HashSet<String>,
}
pub struct Engine {
    settings_commit: Mutex<()>,
    root: PathBuf,
    cache: PathBuf,
    config: Mutex<Config>,
    state: Mutex<MobileState>,
    session: Mutex<Option<Arc<Session>>>,
    account_notice: Mutex<Option<String>>,
    account_intent: std::sync::atomic::AtomicU64,
    browser: Mutex<Browser>,
    radio: Mutex<Radio>,
    images: Mutex<ImageCache>,
    cancellations: Mutex<HashMap<u64, CancellationToken>>,
    playback_context: Mutex<Option<PlaybackContext>>,
    lyrics: Mutex<HashMap<String, lyrics::Document>>,
    resolver: Arc<resolution::Resolver>,
}
impl Engine {
    pub fn new(root: &Path, cache: &Path) -> Result<Self> {
        std::fs::create_dir_all(root)?;
        std::fs::create_dir_all(cache)?;
        let config: Config = read_json(&root.join("config.json"))?.unwrap_or_else(|| Config {
            auth_store: "file".into(),
            notifications: true,
            download: ytui_core::DownloadOptions {
                quality: "1080".into(),
                ..Default::default()
            },
            ..Default::default()
        });
        config.validate()?;
        let state = read_json(&root.join("library.json"))?.unwrap_or_default();
        let stored=account::load(root).unwrap_or_else(|_|account::Stored {
            session:None,notice:Some("Stored account is unavailable; reconnect or disconnect it. Saved media and downloads are intact.".into()),
        });
        let session = stored.session.map(Arc::new);
        let mut account_notice = stored.notice;
        if account::clear_transient_auth(cache).is_err() {
            account_notice=Some("An older private session staging file could not be removed; reconnect or clear the app cache after stopping playback.".into());
        }
        let client = ytui_providers::http_client(config.proxy.as_deref())?;
        Ok(Self {
            settings_commit: Mutex::new(()),
            resolver: Arc::new(resolution::Resolver::default()),
            root: root.into(),
            cache: cache.into(),
            config: Mutex::new(config.clone()),
            state: Mutex::new(state),
            session: Mutex::new(session.clone()),
            account_notice: Mutex::new(account_notice),
            account_intent: std::sync::atomic::AtomicU64::new(0),
            browser: Mutex::new(Browser::new(client.clone(), session, config.account_index)),
            radio: Mutex::new(
                Radio::new(client.clone(), config.radio_servers.clone())
                    .with_cache(cache.join("radio")),
            ),
            images: Mutex::new(ImageCache::new(client, cache.join("images"))),
            cancellations: Mutex::new(HashMap::new()),
            playback_context: Mutex::new(None),
            lyrics: Mutex::new(read_json(&root.join("lyrics.json"))?.unwrap_or_default()),
        })
    }
    pub fn activate_context(&self, id: Option<String>) {
        let mut active = self.playback_context.lock().expect("playback context");
        if id
            .as_deref()
            .is_some_and(|id| active.as_ref().is_some_and(|c| c.id == id))
        {
            return;
        }
        *active = id.map(|id| PlaybackContext {
            id,
            items: Default::default(),
            prepared: false,
            consumed: Default::default(),
        });
    }
    pub async fn thumbnail(
        &self,
        media: &Media,
        width: u32,
        height: u32,
    ) -> Result<image::DynamicImage> {
        let width = width.clamp(32, 768);
        let height = height.clamp(32, 768);
        let cache = self.images.lock().expect("images").clone();
        let url = ytui_providers::images::thumbnail_url(media, width, height);
        cache.load_sized(&url, width, height).await
    }
    pub fn cancel(&self, id: u64) {
        if let Some(c) = self.cancellations.lock().expect("cancellations").get(&id) {
            c.cancel();
        }
    }
    pub fn account_intent(&self) -> u64 {
        self.account_intent
            .load(std::sync::atomic::Ordering::SeqCst)
    }
    pub fn begin_account_change(&self) -> u64 {
        self.resolver.clear();
        self.account_intent
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            + 1
    }
    fn check_account_intent(&self, r: &Value) -> Result<()> {
        ensure!(
            r["account_intent"].as_u64() == Some(self.account_intent()),
            "Account action was superseded"
        );
        Ok(())
    }
    pub async fn request(&self, mut request: Value) -> Result<Value> {
        if matches!(
            request["op"].as_str(),
            Some("import_cookie_file" | "import_cookies" | "disconnect" | "reload_account")
        ) && request["account_intent"].is_null()
        {
            request["account_intent"] = json!(self.begin_account_change());
        }
        let id = request["request_id"].as_u64().unwrap_or(0);
        let cancel = CancellationToken::new();
        if id > 0 {
            let mut jobs = self.cancellations.lock().expect("cancellations");
            ensure!(jobs.len() < 32, "Too many pending requests");
            ensure!(!jobs.contains_key(&id), "Duplicate request ID");
            jobs.insert(id, cancel.clone());
        }
        let _registration = RequestRegistration { engine: self, id };
        let result = tokio::select! {_=cancel.cancelled()=>Err(anyhow::anyhow!("Request cancelled")),r=self.dispatch(&request)=>r};
        result
    }
    async fn dispatch(&self, r: &Value) -> Result<Value> {
        let op = r["op"].as_str().context("Missing operation")?;
        match op {
            "settings" => Ok(serde_json::to_value(
                self.config.lock().expect("config").clone(),
            )?),
            "save_settings" => {
                let _commit = self.settings_commit.lock().expect("settings commit");
                let config: Config = serde_json::from_value(r["config"].clone())?;
                config.validate()?;
                if let Some(proxy) = &config.proxy {
                    let url = url::Url::parse(proxy)?;
                    ensure!(
                        url.scheme() == "http"
                            && url.username().is_empty()
                            && url.password().is_none(),
                        "Android supports an HTTP proxy without embedded credentials, or your system VPN"
                    );
                }
                let previous = self.config.lock().expect("config").clone();
                atomic_write(
                    &self.root.join("config.json"),
                    &serde_json::to_vec(&config)?,
                )?;
                let proxy_changed = previous.proxy != config.proxy;
                let account_changed = previous.account_index != config.account_index;
                if proxy_changed
                    || account_changed
                    || previous.radio_servers != config.radio_servers
                {
                    let client = ytui_providers::http_client(config.proxy.as_deref())?;
                    if proxy_changed || account_changed {
                        let session = self.session.lock().expect("session").clone();
                        *self.browser.lock().expect("browser") =
                            Browser::new(client.clone(), session, config.account_index);
                        self.resolver.clear();
                    }
                    if proxy_changed || previous.radio_servers != config.radio_servers {
                        *self.radio.lock().expect("radio") =
                            Radio::new(client.clone(), config.radio_servers.clone())
                                .with_cache(self.cache.join("radio"));
                    }
                    if proxy_changed {
                        *self.images.lock().expect("images") =
                            ImageCache::new(client, self.cache.join("images"));
                    }
                }
                *self.config.lock().expect("config") = config;
                Ok(json!({"saved":true}))
            }
            "import_theme" => {
                let _commit = self.settings_commit.lock().expect("settings commit");
                let text = r["text"].as_str().context("Missing palette document")?;
                ensure!(
                    text.len() <= 2 * 1024 * 1024,
                    "Palette document exceeds limit"
                );
                let palette = serde_json::from_str::<ytui_core::theme::Palette>(text)
                    .map_err(anyhow::Error::from)
                    .or_else(|_| ytui_core::theme::caelestia(text))
                    .or_else(|_| ytui_core::theme::omarchy(text, false))?;
                atomic_write(
                    &self.root.join("palette.json"),
                    &serde_json::to_vec(&palette)?,
                )?;
                let mut config = self.config.lock().expect("config");
                let mut draft = config.clone();
                draft.theme = "custom".into();
                atomic_write(&self.root.join("config.json"), &serde_json::to_vec(&draft)?)?;
                *config = draft;
                Ok(serde_json::to_value(palette)?)
            }
            "themes" => Ok(json!(ytui_core::theme::THEMES)),
            "theme" => {
                let config = self.config.lock().expect("config").clone();
                let name = if config.theme == "system" {
                    if r["dark"].as_bool().unwrap_or(true) {
                        "ytui-dark"
                    } else {
                        "ytui-light"
                    }
                } else {
                    config.theme.as_str()
                };
                if name == "custom" {
                    return Ok(serde_json::to_value(
                        read_json::<ytui_core::theme::Palette>(&self.root.join("palette.json"))?
                            .context("No imported palette")?,
                    )?);
                }
                Ok(serde_json::to_value(ytui_core::theme::builtin(name)?)?)
            }
            "browse" => {
                let query: BrowseRequest = serde_json::from_value(r["request"].clone())?;
                let tab = r["tab"].as_u64().unwrap_or(1);
                let mut page = if matches!(
                    query,
                    BrowseRequest::SavedMedia { .. } | BrowseRequest::Favorites
                ) {
                    let state = self.state.lock().expect("state");
                    Page {
                        title: "Saved locally".into(),
                        items: state.saved.clone(),
                        ..Default::default()
                    }
                } else if tab == 3 {
                    let radio = self.radio.lock().expect("radio").clone();
                    radio
                        .fetch(
                            &query,
                            r["offset"].as_u64().unwrap_or(0).min(100_000) as usize,
                        )
                        .await?
                } else {
                    let browser = self.browser.lock().expect("browser").clone();
                    browser.fetch(tab == 2, &query).await?
                };
                page.notice = page.notice.map(mobile_notice);
                ensure!(
                    page.retained_bytes() <= 16 * 1024 * 1024,
                    "Feed response exceeds memory budget"
                );
                Ok(serde_json::to_value(page)?)
            }
            "saved" => Ok(serde_json::to_value(
                self.state.lock().expect("state").saved.clone(),
            )?),
            "offline_playlists" => Ok(serde_json::to_value(
                &self.state.lock().expect("state").offline_playlists,
            )?),
            "offline_playlist_create" | "offline_playlist_update" | "offline_playlist_delete" => {
                self.mutate(|state| {
                    match r["op"].as_str().unwrap_or_default() {
                        "offline_playlist_create" => {
                            state
                                .offline_playlists
                                .push(ytui_core::offline::OfflinePlaylist {
                                    id: uuid::Uuid::new_v4().to_string(),
                                    title: r["title"].as_str().unwrap_or_default().trim().into(),
                                    items: vec![],
                                });
                        }
                        "offline_playlist_update" => {
                            let playlist = state
                                .offline_playlists
                                .iter_mut()
                                .find(|p| Some(p.id.as_str()) == r["id"].as_str())
                                .context("Offline playlist is no longer available")?;
                            if let Some(title) = r["title"].as_str() {
                                playlist.title = title.trim().into();
                            }
                            if let Some(items) = r.get("items") {
                                playlist.items = serde_json::from_value(items.clone())?;
                            }
                        }
                        _ => state
                            .offline_playlists
                            .retain(|p| Some(p.id.as_str()) != r["id"].as_str()),
                    }
                    ytui_core::offline::validate_library(&state.offline_playlists)?;
                    Ok(serde_json::to_value(&state.offline_playlists)?)
                })
            }
            "save_media" => {
                let media: Media = serde_json::from_value(r["media"].clone())?;
                ensure!(media.retained_bytes() <= 256 * 1024, "Media is too large");
                self.mutate(|state| {
                    let saved = if let Some(i) = state.saved.iter().position(|m| m.url == media.url)
                    {
                        state.saved.remove(i);
                        false
                    } else {
                        ensure!(state.saved.len() < 2000, "Saved library is full");
                        state.saved.push(media);
                        true
                    };
                    Ok(json!({"count":state.saved.len(),"saved":saved}))
                })
            }
            "queue" => Ok(serde_json::to_value(
                self.state.lock().expect("state").queue.clone(),
            )?),
            "enqueue" => {
                let media: Media = serde_json::from_value(r["media"].clone())?;
                ensure!(media.retained_bytes() <= 256 * 1024, "Media is too large");
                self.mutate(|state| {
                    ensure!(state.queue.len() < 500, "Queue is full");
                    state.queue.push(media);
                    Ok(json!({"count":state.queue.len()}))
                })
            }
            "queue_peek" => {
                let state = self.state.lock().expect("state");
                let index = if r["shuffle"].as_bool().unwrap_or(false) && !state.queue.is_empty() {
                    if let Some(index) = state
                        .queue
                        .iter()
                        .position(|m| Some(m.url.as_str()) == r["preferred_url"].as_str())
                    {
                        index
                    } else {
                        u64::from_le_bytes(
                            uuid::Uuid::new_v4().as_bytes()[..8]
                                .try_into()
                                .expect("UUID bytes"),
                        ) as usize
                            % state.queue.len()
                    }
                } else {
                    0
                };
                let explicit = state
                    .queue
                    .get(index)
                    .map(serde_json::to_value)
                    .transpose()?;
                drop(state);
                if let Some(media) = explicit {
                    return Ok(media);
                }
                let context = self.playback_context.lock().expect("playback context");
                if let Some(context) = context
                    .as_ref()
                    .filter(|c| Some(c.id.as_str()) == r["context_id"].as_str())
                    && let Some(media) = context.items.get(
                        if r["shuffle"].as_bool().unwrap_or(false) && !context.items.is_empty() {
                            context
                                .items
                                .iter()
                                .position(|m| Some(m.url.as_str()) == r["preferred_url"].as_str())
                                .unwrap_or_else(|| {
                                    (uuid::Uuid::new_v4().as_u128() % context.items.len() as u128)
                                        as usize
                                })
                        } else {
                            0
                        },
                    )
                {
                    let mut value = serde_json::to_value(media)?;
                    value["context_entry"] = json!(true);
                    return Ok(value);
                }
                Ok(json!({}))
            }
            "queue_commit" => {
                if r["context_entry"].as_bool().unwrap_or(false) {
                    let mut context = self.playback_context.lock().expect("playback context");
                    let committed = if let Some(context) = context
                        .as_mut()
                        .filter(|c| Some(c.id.as_str()) == r["context_id"].as_str())
                    {
                        if let Some(index) = context
                            .items
                            .iter()
                            .position(|m| Some(m.url.as_str()) == r["url"].as_str())
                        {
                            if let Some(media) = context.items.remove(index) {
                                context.consumed.insert(media.url);
                            }
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    };
                    return Ok(json!({"committed":committed}));
                }
                self.mutate(|state| {
                    let chosen = state
                        .queue
                        .iter()
                        .position(|m| Some(m.url.as_str()) == r["url"].as_str());
                    if let Some(index) = chosen {
                        state.queue.remove(index);
                    }
                    Ok(json!({"committed":chosen.is_some()}))
                })
            }
            "history_list" => {
                let state = self.state.lock().expect("state");
                let recent = if state.recent.is_empty() {
                    &state.playback.items
                } else {
                    &state.recent
                };
                Ok(json!(recent.iter().rev().collect::<Vec<_>>()))
            }
            "history_clear" => self.mutate(|state| {
                state.playback = Default::default();
                state.recent.clear();
                state.positions.clear();
                Ok(json!([]))
            }),
            "history_peek" => {
                if !self.config.lock().expect("config").playback_history {
                    return Ok(json!({}));
                }
                let state = self.state.lock().expect("state");
                let timeline = &state.playback;
                let target = timeline.current.and_then(|index| {
                    if r["direction"] == "previous" {
                        index.checked_sub(1)
                    } else {
                        index.checked_add(1)
                    }
                });
                Ok(target
                    .and_then(|index| {
                        timeline
                            .items
                            .get(index)
                            .map(|media| json!({"media":media,"index":index}))
                    })
                    .unwrap_or_else(|| json!({})))
            }
            "history_record" => {
                if !self.config.lock().expect("config").playback_history {
                    return Ok(json!({"recorded":false}));
                }
                let media: Media = serde_json::from_value(r["media"].clone())?;
                ensure!(media.retained_bytes() <= 256 * 1024, "Media is too large");
                ytui_core::http_url(&media.url)?;
                self.mutate(|state| {
                    if state.recent.is_empty() {
                        state.recent = state.playback.items.clone();
                    }
                    let accepted = media.clone();
                    let timeline = &mut state.playback;
                    let requested = r["index"].as_u64().and_then(|n| usize::try_from(n).ok());
                    let same = |i: usize| {
                        timeline
                            .items
                            .get(i)
                            .is_some_and(|old| old.url == media.url)
                    };
                    if let Some(index) = requested.filter(|i| same(*i)) {
                        timeline.current = Some(index);
                        timeline.items[index] = media;
                    } else if timeline.current.is_none_or(|i| !same(i)) {
                        if let Some(i) = timeline.current {
                            timeline.items.truncate(i.saturating_add(1));
                        }
                        timeline.play(media);
                    } else if let Some(index) = timeline.current {
                        timeline.items[index] = media;
                    }
                    let mut bytes = timeline
                        .items
                        .iter()
                        .map(Media::retained_bytes)
                        .sum::<usize>();
                    while timeline.items.len() > 100 || bytes > 2 * 1024 * 1024 {
                        bytes = bytes.saturating_sub(timeline.items.remove(0).retained_bytes());
                        timeline.current = timeline.current.map(|i| i.saturating_sub(1));
                    }
                    state.recent.retain(|m| m.url != accepted.url);
                    state.recent.push(accepted);
                    let mut bytes = state
                        .recent
                        .iter()
                        .map(Media::retained_bytes)
                        .sum::<usize>();
                    while state.recent.len() > 100 || bytes > 2 * 1024 * 1024 {
                        bytes = bytes.saturating_sub(state.recent.remove(0).retained_bytes());
                    }
                    Ok(json!({"recorded":true}))
                })
            }
            "queue_remove" => {
                let i = r["index"].as_u64().context("Missing queue index")? as usize;
                self.mutate(|state| {
                    ensure!(i < state.queue.len(), "Queue index is out of bounds");
                    ensure!(
                        r["url"]
                            .as_str()
                            .is_none_or(|url| state.queue[i].url == url),
                        "The queue changed; review its current order"
                    );
                    state.queue.remove(i);
                    Ok(json!({"removed":true,"items":state.queue}))
                })
            }
            "queue_move" => {
                let from = r["from"].as_u64().context("Missing source index")? as usize;
                let to = r["to"].as_u64().context("Missing destination index")? as usize;
                let expected = r["url"].as_str().context("Missing queue item URL")?;
                self.mutate(|state| {
                    ensure!(
                        from < state.queue.len() && to < state.queue.len(),
                        "Queue index is out of bounds"
                    );
                    ensure!(
                        state.queue[from].url == expected,
                        "The queue changed; review its current order"
                    );
                    let media = state.queue.remove(from);
                    state.queue.insert(to, media);
                    Ok(json!({"items":state.queue}))
                })
            }
            "position" => {
                let url = r["url"].as_str().context("Missing media URL")?;
                ensure!(url.len() <= 8192, "Media URL exceeds limit");
                if let Some(ms) = r["milliseconds"].as_u64() {
                    ensure!(ms <= 7 * 24 * 3600 * 1000, "Invalid media position");
                    self.mutate(|state| {
                        if state.positions.len() >= 2000
                            && !state.positions.contains_key(url)
                            && let Some(key) = state.positions.keys().next().cloned()
                        {
                            state.positions.remove(&key);
                        }
                        state.positions.insert(url.into(), ms);
                        Ok(json!({"saved":true}))
                    })
                } else {
                    Ok(
                        json!({"milliseconds":self.state.lock().expect("state").positions.get(url).copied().unwrap_or(0)}),
                    )
                }
            }
            "account" => Ok(
                json!({"connected":self.session.lock().expect("session").is_some(),"notice":self.account_notice.lock().expect("account notice").clone()}),
            ),
            "reload_account" => {
                let _commit = self.settings_commit.lock().expect("settings commit");
                self.check_account_intent(r)?;
                let stored=account::load(&self.root).unwrap_or_else(|_|account::Stored {
                    session:None,notice:Some("Stored account is unavailable; reconnect or disconnect it. Saved media and downloads are intact.".into()),
                });
                let config = self.config.lock().expect("config").clone();
                let client = ytui_providers::http_client(config.proxy.as_deref())?;
                let session = stored.session.map(Arc::new);
                *self.browser.lock().expect("browser") =
                    Browser::new(client, session.clone(), config.account_index);
                *self.session.lock().expect("session") = session.clone();
                *self.account_notice.lock().expect("account notice") = stored.notice.clone();
                self.resolver.clear();
                self.activate_context(None);
                Ok(json!({"connected":session.is_some(),"notice":stored.notice}))
            }
            "lyrics" => {
                let media: Media = serde_json::from_value(r["media"].clone())?;
                let proxy = self.config.lock().expect("config").proxy.clone();
                let title = r["title"].as_str().unwrap_or(&media.title);
                let artist = r["artist"].as_str().unwrap_or(&media.creator);
                let tracks =
                    ytui_providers::lyrics::lookup(proxy.as_deref(), title, artist).await?;
                Ok(serde_json::to_value(tracks)?)
            }
            "lyrics_get" | "lyrics_save" | "lyrics_remove" => self.local_lyrics(r),
            "import_cookie_file" => {
                let _commit = self.settings_commit.lock().expect("settings commit");
                self.check_account_intent(r)?;
                let text = r["text"].as_str().context("Missing cookie document")?;
                ensure!(
                    text.len() <= 2 * 1024 * 1024,
                    "Cookie document exceeds limit"
                );
                let session = ytui_providers::auth::parse_netscape(text)?;
                let config = self.config.lock().expect("config").clone();
                let client = ytui_providers::http_client(config.proxy.as_deref())?;
                let notice = account::save(&self.root, &session)?;
                *self.account_notice.lock().expect("account notice") = notice.clone();
                let session = Arc::new(session);
                *self.session.lock().expect("session") = Some(session.clone());
                self.resolver.clear();
                self.activate_context(None);
                *self.browser.lock().expect("browser") =
                    Browser::new(client, Some(session), config.account_index);
                Ok(json!({"connected":true,"notice":notice}))
            }
            "import_cookies" => {
                let _commit = self.settings_commit.lock().expect("settings commit");
                self.check_account_intent(r)?;
                let cookies = if let Some(header) = r["header"].as_str() {
                    ensure!(
                        header.len() <= 2 * 1024 * 1024,
                        "Cookie header exceeds limit"
                    );
                    header
                        .split(';')
                        .filter_map(|pair| pair.trim().split_once('='))
                        .map(|(name, value)| ytui_providers::auth::Cookie {
                            domain: ".youtube.com".into(),
                            path: "/".into(),
                            secure: true,
                            expires: None,
                            name: name.into(),
                            value: value.into(),
                            http_only: true,
                        })
                        .collect()
                } else {
                    serde_json::from_value(r["cookies"].clone())?
                };
                let session = Arc::new(Session::new(cookies)?);
                let notice = account::save(&self.root, &session)?;
                *self.account_notice.lock().expect("account notice") = notice.clone();
                *self.session.lock().expect("session") = Some(session.clone());
                self.resolver.clear();
                self.activate_context(None);
                let config = self.config.lock().expect("config").clone();
                *self.browser.lock().expect("browser") = Browser::new(
                    ytui_providers::http_client(config.proxy.as_deref())?,
                    Some(session),
                    config.account_index,
                );
                Ok(json!({"connected":true,"notice":notice}))
            }
            "disconnect" => {
                let _commit = self.settings_commit.lock().expect("settings commit");
                self.check_account_intent(r)?;
                account::remove(&self.root)?;
                *self.account_notice.lock().expect("account notice") = None;
                *self.session.lock().expect("session") = None;
                self.resolver.clear();
                self.activate_context(None);
                let config = self.config.lock().expect("config").clone();
                *self.browser.lock().expect("browser") = Browser::new(
                    ytui_providers::http_client(config.proxy.as_deref())?,
                    None,
                    config.account_index,
                );
                Ok(json!({"connected":false}))
            }
            "resolve" => self.resolve(r).await,
            "resolve_cache_clear" => {
                self.resolver.clear();
                Ok(json!({"cleared":true}))
            }
            "seed_context" | "offline_context" => {
                let id = r["context_id"]
                    .as_str()
                    .context("Missing playback context")?;
                let items: Vec<Media> = serde_json::from_value(r["items"].clone())?;
                let offline = r["op"].as_str() == Some("offline_context");
                if offline {
                    ytui_core::offline::OfflinePlaylist {
                        id: uuid::Uuid::new_v4().to_string(),
                        title: "Playback".into(),
                        items: items.clone(),
                    }
                    .validate()?;
                }
                ensure!(
                    items.len() <= if offline { 1000 } else { 500 }
                        && items.iter().map(Media::retained_bytes).sum::<usize>()
                            <= 4 * 1024 * 1024,
                    "Playback seed exceeds budget"
                );
                let source = r["source_url"].as_str().unwrap_or("");
                let offset = items
                    .iter()
                    .position(|m| m.url == source)
                    .map_or(0, |i| i + 1);
                let mut active = self.playback_context.lock().expect("playback context");
                if let Some(context) = active
                    .as_mut()
                    .filter(|c| c.id == id && !c.prepared && c.items.is_empty())
                {
                    context.items = items
                        .into_iter()
                        .skip(offset)
                        .filter(|m| {
                            !m.live
                                && !m.collection()
                                && m.url != source
                                && !context.consumed.contains(&m.url)
                        })
                        .collect();
                    context.prepared = offline;
                    Ok(json!({"prepared":true,"count":context.items.len()}))
                } else {
                    Ok(json!({"prepared":false}))
                }
            }
            "context_status" => {
                let active = self.playback_context.lock().expect("playback context");
                Ok(
                    json!({"active":active.as_ref().is_some_and(|c|Some(c.id.as_str())==r["context_id"].as_str())}),
                )
            }
            "prepare_context" => {
                let id = r["context_id"]
                    .as_str()
                    .context("Missing playback context")?;
                if let Some(context) = self
                    .playback_context
                    .lock()
                    .expect("playback context")
                    .as_ref()
                    .filter(|c| c.id == id && c.prepared)
                {
                    return Ok(json!({"prepared":true,"count":context.items.len()}));
                }
                if self
                    .playback_context
                    .lock()
                    .expect("playback context")
                    .as_ref()
                    .is_none_or(|c| c.id != id)
                {
                    return Ok(json!({"prepared":false}));
                }
                let query: BrowseRequest = serde_json::from_value(r["request"].clone())?;
                ensure!(
                    matches!(
                        query,
                        BrowseRequest::Collection(_)
                            | BrowseRequest::Details(_)
                            | BrowseRequest::CliampTracks(_)
                            | BrowseRequest::Related(_)
                    ),
                    "This feed is not a finite playback collection"
                );
                let related = matches!(query, BrowseRequest::Related(_));
                let items = if related {
                    let browser = self.browser.lock().expect("browser").clone();
                    browser
                        .fetch(r["music"].as_bool().unwrap_or(false), &query)
                        .await?
                        .items
                } else if matches!(query, BrowseRequest::CliampTracks(_)) {
                    let radio = self.radio.lock().expect("radio").clone();
                    radio.fetch(&query, 0).await?.items
                } else {
                    let browser = self.browser.lock().expect("browser").clone();
                    ytui_providers::watch::collect(
                        browser,
                        ytui_core::watch::CollectionWatch {
                            id: uuid::Uuid::new_v4(),
                            title: "Playback collection".into(),
                            request: query,
                            music: r["music"].as_bool().unwrap_or(false),
                            options: Default::default(),
                            interval_seconds: 1800,
                            next_check: 0,
                            enabled: true,
                            generation: 0,
                            owner: "mobile".into(),
                            seen: None,
                            failures: 0,
                            error: None,
                        },
                        CancellationToken::new(),
                    )
                    .await?
                };
                let selected = if related {
                    None
                } else {
                    Some(
                        items
                            .iter()
                            .position(|m| {
                                Some(m.url.as_str()) == r["source_url"].as_str()
                                    || !m.id.is_empty()
                                        && Some(m.id.as_str()) == r["source_id"].as_str()
                            })
                            .context("Selected song is no longer present in the collection")?,
                    )
                };
                let tail = items
                    .into_iter()
                    .skip(selected.map_or(0, |i| i + 1))
                    .filter(|m| {
                        !m.live
                            && !m.collection()
                            && Some(m.url.as_str()) != r["source_url"].as_str()
                    })
                    .collect::<std::collections::VecDeque<_>>();
                ensure!(
                    tail.len() <= 2000
                        && tail.iter().map(Media::retained_bytes).sum::<usize>()
                            <= 16 * 1024 * 1024,
                    "Playback collection exceeds the memory budget"
                );
                let mut active = self.playback_context.lock().expect("playback context");
                if let Some(context) = active.as_mut().filter(|c| c.id == id) {
                    if !context.prepared {
                        context.items = tail
                            .into_iter()
                            .filter(|m| !context.consumed.contains(&m.url))
                            .collect();
                        context.prepared = true;
                    }
                    let count = context.items.len();
                    Ok(json!({"prepared":true,"count":count}))
                } else {
                    Ok(json!({"prepared":false}))
                }
            }
            "collection" | "enqueue_collection" => {
                let music = r["music"].as_bool().unwrap_or(false);
                let request: BrowseRequest = serde_json::from_value(r["request"].clone())?;
                let browser = self.browser.lock().expect("browser").clone();
                let watch = ytui_core::watch::CollectionWatch {
                    id: uuid::Uuid::new_v4(),
                    title: "Mobile playlist".into(),
                    request,
                    music,
                    options: Default::default(),
                    interval_seconds: 1800,
                    next_check: 0,
                    enabled: true,
                    generation: 0,
                    owner: "mobile".into(),
                    seen: None,
                    failures: 0,
                    error: None,
                };
                let items =
                    ytui_providers::watch::collect(browser, watch, CancellationToken::new())
                        .await?;
                if op == "enqueue_collection" {
                    self.mutate(|state| {
                        ensure!(
                            state.queue.len() + items.len() <= 500,
                            "This collection exceeds the 500-item mobile queue budget"
                        );
                        let count = items.len();
                        state.queue.extend(items);
                        Ok(json!({"added":count}))
                    })
                } else {
                    Ok(json!(items))
                }
            }
            "sponsor" => {
                if self.config.lock().expect("config").playback_sponsorblock == "off" {
                    return Ok(json!([]));
                }
                let media: Media = serde_json::from_value(r["media"].clone())?;
                let config = self.config.lock().expect("config").clone();
                Ok(json!(
                    ytui_providers::sponsorblock::fetch(&config, &media).await?
                ))
            }
            _ => anyhow::bail!("Unknown mobile operation"),
        }
    }
    fn mutate<T>(&self, update: impl FnOnce(&mut MobileState) -> Result<T>) -> Result<T> {
        let mut state = self.state.lock().expect("state");
        let mut draft = state.clone();
        let value = update(&mut draft)?;
        self.persist(&draft)?;
        *state = draft;
        Ok(value)
    }
    fn persist(&self, state: &MobileState) -> Result<()> {
        let bytes = serde_json::to_vec(state)?;
        ensure!(
            bytes.len() <= 16 * 1024 * 1024,
            "Local library exceeds memory budget"
        );
        atomic_write(&self.root.join("library.json"), &bytes)
    }
    async fn resolve(&self, r: &Value) -> Result<Value> {
        let media: Media = serde_json::from_value(r["media"].clone())?;
        let url = ytui_core::http_url(&media.url)?;
        if let Some(session) = self.session.lock().expect("session").as_ref() {
            session.headers("https://www.youtube.com")?;
        }
        let config = self.config.lock().expect("config").clone();
        let audio = r["audio"].as_bool().unwrap_or(false);
        let quality = r["quality"]
            .as_str()
            .unwrap_or(&config.download.quality)
            .to_owned();
        let key = resolution::Key {
            epoch: self.resolver.epoch(),
            account: self
                .account_intent
                .load(std::sync::atomic::Ordering::Acquire),
            account_index: config.account_index,
            proxy: config.proxy.clone(),
            url: media.url.clone(),
            audio,
            quality: quality.clone(),
        };
        let client_key = key.clone();
        self.resolver
            .get_or_resolve(key, r["fresh"].as_bool().unwrap_or(false), async {
                if !ytui_core::youtube_host(url.host_str().unwrap_or_default()) {
                    let radio = self.radio.lock().expect("radio").clone();
                    let resolved = radio.resolve_playlist(url.as_str()).await?;
                    return Ok(
                        json!({"url":resolved,"headers":{},"audio_url":null,"live":media.live}),
                    );
                }
                let cookies = self.session.lock().expect("session").clone();
                let pool = self.resolver.clone();
                let permit = extractor_slots().acquire_owned().await?;
                let data = tokio::task::spawn_blocking(move || -> Result<Value> {
                    let _permit = permit;
                    let client = pool.client(&client_key, || {
                        let jar = cookies.as_deref().map(account::native_jar).transpose()?;
                        Ok(yt_dlp_rs::YoutubeDl::with_options(
                            yt_dlp_rs::YoutubeOptions {
                                no_playlist: true,
                                js_runtime: if cfg!(target_os = "android") {
                                    Some("ytui-android-js".into())
                                } else {
                                    None
                                },
                                ..Default::default()
                            },
                            yt_dlp_rs::HttpOptions {
                                proxy: config.proxy,
                                cookies: jar,
                                socket_timeout: Some(12.0),
                                ..Default::default()
                            },
                        )?)
                    })?;
                    Ok(serde_json::to_value(client.extract_info_with_format(
                        &media.url,
                        Some(&mobile_format(audio, &quality)?),
                    )?)?)
                })
                .await??;
                stream(&data, audio)
            })
            .await
    }
}
fn extractor_slots() -> Arc<tokio::sync::Semaphore> {
    static EXTRACTORS: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
    EXTRACTORS
        .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(2)))
        .clone()
}
fn mobile_format(audio: bool, quality: &str) -> Result<String> {
    if audio {
        return Ok("bestaudio/best".into());
    }
    if quality == "best" {
        return Ok("bestvideo+bestaudio/best".into());
    }
    ensure!(
        ["360", "480", "720", "1080"].contains(&quality),
        "Invalid quality"
    );
    // H.264/AAC decodes and exports on API 26. Keep bounded codec fallbacks.
    Ok(format!(
        "bestvideo[height<={quality}][vcodec^=avc1]+bestaudio[acodec^=mp4a]/bestvideo[height<={quality}][vcodec^=avc1]+bestaudio/bestvideo[height<={quality}][vcodec!^=av01]+bestaudio/best[height<={quality}]"
    ))
}
#[cfg(test)]
mod format_tests {
    use super::*;
    #[tokio::test]
    async fn cancelled_and_dropped_resolve_requests_release_ids_without_starting_network() {
        let dir = tempfile::tempdir().unwrap();
        let engine =
            Arc::new(Engine::new(&dir.path().join("files"), &dir.path().join("cache")).unwrap());
        // Hold both actual extraction permits. Requests can register but cannot start an extractor.
        let slots = extractor_slots();
        let _permits = slots.acquire_many_owned(2).await.unwrap();
        for id in [77u64, 78] {
            let pending = engine.clone();
            let task = tokio::spawn(async move {
                pending.request(json!({"op":"resolve","request_id":id,"media":{"url":"https://www.youtube.com/watch?v=abcdefghijk","title":"Synthetic"}})).await
            });
            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                while !engine.cancellations.lock().unwrap().contains_key(&id) {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            assert!(
                engine
                    .request(json!({"op":"settings","request_id":id}))
                    .await
                    .is_err()
            );
            if id == 77 {
                engine.cancel(id);
                assert!(
                    tokio::time::timeout(std::time::Duration::from_secs(2), task)
                        .await
                        .unwrap()
                        .unwrap()
                        .is_err()
                );
            } else {
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
            }
            assert!(
                engine
                    .request(json!({"op":"settings","request_id":id}))
                    .await
                    .is_ok()
            );
        }
    }
    #[tokio::test]
    async fn playlist_seed_is_available_before_the_full_collection_and_commits_only_on_start() {
        let temp = tempfile::tempdir().unwrap();
        let engine = Engine::new(temp.path(), &temp.path().join("cache")).unwrap();
        engine.activate_context(Some("collection".into()));
        let items = json!([{"url":"https://media.test/a","title":"First","kind":"song"},
            {"url":"https://media.test/b","title":"Second","kind":"song"},
            {"url":"https://media.test/c","title":"Third","kind":"song"}]);
        let seeded=engine.request(json!({"op":"seed_context","context_id":"collection","source_url":"https://media.test/a","items":items})).await.unwrap();
        assert_eq!(seeded["count"], 2);
        for _ in 0..2 {
            assert_eq!(
                engine
                    .request(json!({"op":"queue_peek","context_id":"collection"}))
                    .await
                    .unwrap()["title"],
                "Second"
            );
        }
        engine.request(json!({"op":"queue_commit","context_id":"collection","context_entry":true,"url":"https://media.test/b"})).await.unwrap();
        assert_eq!(
            engine
                .request(json!({"op":"queue_peek","context_id":"collection"}))
                .await
                .unwrap()["title"],
            "Third"
        );
        assert!(
            engine
                .playback_context
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .consumed
                .contains("https://media.test/b")
        );
        engine.activate_context(Some("new intent".into()));
        assert_eq!(
            engine
                .request(json!({"op":"seed_context","context_id":"collection","items":[]}))
                .await
                .unwrap()["prepared"],
            false
        );
    }

    #[tokio::test]
    async fn warm_shuffle_choice_stays_stable_without_overriding_manual_queue_priority() {
        let temp = tempfile::tempdir().unwrap();
        let engine = Engine::new(temp.path(), &temp.path().join("cache")).unwrap();
        engine.activate_context(Some("collection".into()));
        engine.request(json!({"op":"seed_context","context_id":"collection","source_url":"https://media.test/a","items":[
            {"url":"https://media.test/a","kind":"song"}, {"url":"https://media.test/b","kind":"song"}, {"url":"https://media.test/c","kind":"song"}]})).await.unwrap();
        let peek = json!({"op":"queue_peek","context_id":"collection","shuffle":true,"preferred_url":"https://media.test/c"});
        for _ in 0..3 {
            assert_eq!(
                engine.request(peek.clone()).await.unwrap()["url"],
                "https://media.test/c"
            );
        }
        engine
            .request(
                json!({"op":"enqueue","media":{"url":"https://media.test/manual","kind":"song"}}),
            )
            .await
            .unwrap();
        let candidate = engine.request(peek.clone()).await.unwrap();
        assert_eq!(candidate["url"], "https://media.test/manual");
        assert!(candidate.get("context_entry").is_none());
        engine
            .request(json!({"op":"queue_commit","url":"https://media.test/manual"}))
            .await
            .unwrap();
        assert_eq!(
            engine.request(peek).await.unwrap()["url"],
            "https://media.test/c"
        );
        assert_eq!(engine.request(json!({"op":"queue_peek","context_id":"collection","preferred_url":"https://media.test/c"})).await.unwrap()["url"], "https://media.test/b");
    }

    #[tokio::test]
    async fn offline_playlists_are_transactional_persisted_and_prepare_without_network() {
        let temp = tempfile::tempdir().unwrap();
        let engine = Engine::new(temp.path(), &temp.path().join("cache")).unwrap();
        let created = engine
            .request(json!({"op":"offline_playlist_create","title":"Road trip"}))
            .await
            .unwrap();
        let id = created[0]["id"].as_str().unwrap();
        let items = json!([
            {"url":"https://media.test/a","title":"First","kind":"song","offline_id":"a".repeat(64)},
            {"url":"https://media.test/b","title":"Second","kind":"song","offline_id":"b".repeat(64)}
        ]);
        engine
            .request(json!({"op":"offline_playlist_update","id":id,"items":items}))
            .await
            .unwrap();
        let previous = std::fs::read(temp.path().join("library.json")).unwrap();
        assert!(
            engine
                .request(json!({"op":"offline_playlist_update","id":id,"title":""}))
                .await
                .is_err()
        );
        assert_eq!(
            previous,
            std::fs::read(temp.path().join("library.json")).unwrap()
        );
        engine.activate_context(Some("offline".into()));
        assert_eq!(engine.request(json!({"op":"offline_context","context_id":"offline","items":items,"source_url":"https://media.test/a"})).await.unwrap()["count"],1);
        let next = engine
            .request(json!({"op":"queue_peek","context_id":"offline"}))
            .await
            .unwrap();
        assert_eq!(next["offline_id"], "b".repeat(64));
        assert_eq!(
            engine.request(json!({"op":"history_list"})).await.unwrap(),
            json!([])
        );
        drop(engine);
        let restored = Engine::new(temp.path(), &temp.path().join("cache")).unwrap();
        let restored_items: Vec<Media> = serde_json::from_value(
            restored
                .request(json!({"op":"offline_playlists"}))
                .await
                .unwrap()[0]["items"]
                .clone(),
        )
        .unwrap();
        assert_eq!(
            restored_items,
            serde_json::from_value::<Vec<Media>>(items).unwrap()
        );
        restored
            .request(json!({"op":"offline_playlist_delete","id":id}))
            .await
            .unwrap();
        assert_eq!(
            restored
                .request(json!({"op":"offline_playlists"}))
                .await
                .unwrap(),
            json!([])
        );
    }

    #[test]
    fn phone_quality_prefers_compatible_video_and_honors_the_selected_cap() {
        let formats = vec![
            json!({"format_id":"audio","url":"https://media.test/a","ext":"m4a","vcodec":"none","acodec":"mp4a.40.2"}),
            json!({"format_id":"720","url":"https://media.test/720","ext":"mp4","height":720,"vcodec":"avc1.64001F","acodec":"none"}),
            json!({"format_id":"1080","url":"https://media.test/1080","ext":"mp4","height":1080,"vcodec":"avc1.640028","acodec":"none"}),
            json!({"format_id":"av1","url":"https://media.test/av1","ext":"mp4","height":1080,"vcodec":"av01.0.08M.08","acodec":"none"}),
            json!({"format_id":"4k","url":"https://media.test/4k","ext":"mp4","height":2160,"vcodec":"av01.0.12M.08","acodec":"none"}),
            json!({"format_id":"opus","url":"https://media.test/opus","ext":"webm","vcodec":"none","acodec":"opus"}),
        ];
        for (quality, expected, sound) in [("720", 1, 0), ("1080", 2, 0), ("best", 4, 5)] {
            let selector =
                yt_dlp_rs::format_select::parse(&mobile_format(false, quality).unwrap()).unwrap();
            let selected = selector
                .select(&formats, &Default::default(), None)
                .unwrap();
            assert_eq!(selected[0].formats, vec![expected, sound]);
        }
        assert!(mobile_format(false, "invalid").is_err());
        let audio = yt_dlp_rs::format_select::parse(&mobile_format(true, "1080").unwrap()).unwrap();
        assert_eq!(
            audio.select(&formats, &Default::default(), None).unwrap()[0].formats,
            vec![5]
        );
        let fallback =
            yt_dlp_rs::format_select::parse(&mobile_format(false, "1080").unwrap()).unwrap();
        let no_aac = formats[1..].to_vec();
        assert_eq!(
            fallback.select(&no_aac, &Default::default(), None).unwrap()[0].formats,
            vec![1, 4]
        );
    }
}
struct RequestRegistration<'a> {
    engine: &'a Engine,
    id: u64,
}
impl Drop for RequestRegistration<'_> {
    fn drop(&mut self) {
        if self.id > 0 {
            self.engine
                .cancellations
                .lock()
                .expect("cancellations")
                .remove(&self.id);
        }
    }
}
fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    match std::fs::read(path) {
        Ok(bytes) => {
            ensure!(
                bytes.len() <= 16 * 1024 * 1024,
                "Stored state exceeds memory budget"
            );
            Ok(Some(serde_json::from_slice(&bytes)?))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}
fn stream(data: &Value, audio: bool) -> Result<Value> {
    let formats = data["requested_formats"].as_array();
    let primary = formats
        .and_then(|fs| {
            fs.iter().find(|f| {
                if audio {
                    f["acodec"] != "none"
                } else {
                    f["vcodec"] != "none"
                }
            })
        })
        .unwrap_or(data);
    let url=primary["url"].as_str().context("Native extractor returned no playable stream; this video may require an unsupported signature challenge")?;
    ytui_core::http_url(url)?;
    let audio_url = if audio {
        None
    } else {
        formats
            .and_then(|fs| fs.iter().find(|f| f["vcodec"] == "none"))
            .and_then(|f| f["url"].as_str())
    };
    if let Some(url) = audio_url {
        ytui_core::http_url(url)?;
    }
    let mut headers = serde_json::Map::new();
    for source in [data, primary] {
        if let Some(h) = source["http_headers"].as_object() {
            for (key, value) in h {
                if [
                    "user-agent",
                    "referer",
                    "origin",
                    "accept",
                    "accept-language",
                ]
                .contains(&key.to_ascii_lowercase().as_str())
                    && value
                        .as_str()
                        .is_some_and(|v| !v.chars().any(char::is_control))
                {
                    headers.insert(key.clone(), value.clone());
                }
            }
        }
    }
    Ok(
        json!({"url":url,"audio_url":audio_url,"format_id":primary["format_id"],"audio_format_id":formats.and_then(|fs|fs.iter().find(|f|f["vcodec"]=="none")).map(|f|f["format_id"].clone()),"headers":headers,"live":data["is_live"].as_bool().unwrap_or(false)||data["live_status"]=="is_live","title":data["title"],"creator":data["uploader"].as_str().or_else(||data["channel"].as_str()).unwrap_or(""),"thumbnail":data["thumbnail"],"view_count":data["view_count"],"upload_date":data["upload_date"],"like_count":data["like_count"],"channel_follower_count":data["channel_follower_count"],"description":data["description"].as_str().map(|text|text.chars().take(8192).collect::<String>()).unwrap_or_default(),"duration":data["duration"],"catalog":ytui_providers::formats::Catalog::parse(data)}),
    )
}
pub(crate) fn runtime() -> &'static tokio::runtime::Runtime {
    static R: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    R.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(3)
            .enable_all()
            .build()
            .expect("mobile runtime")
    })
}
static ENGINE: OnceLock<Arc<Engine>> = OnceLock::new();
pub fn initialize(root: &Path, cache: &Path) -> Result<()> {
    ENGINE
        .set(Arc::new(Engine::new(root, cache)?))
        .map_err(|_| anyhow::anyhow!("Mobile engine already initialized"))
}
pub fn call(raw: &str) -> String {
    let result = (|| -> Result<Value> {
        ensure!(
            raw.len() <= 2 * 1024 * 1024,
            "Request exceeds memory budget"
        );
        let value = serde_json::from_str(raw)?;
        runtime().block_on(
            ENGINE
                .get()
                .context("Engine is not initialized")?
                .request(value),
        )
    })();
    match result {
        Ok(value) => json!({"ok":true,"data":value}).to_string(),
        Err(error) => mobile_error(&error).to_string(),
    }
}
fn mobile_error(error: &anyhow::Error) -> Value {
    let text = ytui_providers::diagnostic(&format!("{error:#}"));
    let lower = text.to_ascii_lowercase();
    let (code, message) = if lower.contains("no anonymous home recommendations") {
        ("anonymous_home_empty","YouTube did not provide anonymous recommendations. Search for a video or sign in for a personalized feed.".into())
    } else if lower.contains("sign in first") || lower.contains("session expired") {
        (
            "auth_required",
            "Sign in to open this feed. Use Account → Sign in, or import a cookie document/header."
                .into(),
        )
    } else if text.contains("F2") || text.contains("ytui-rs auth") {
        ("consent_required","YouTube requires consent or a refreshed account session. Use Account → Sign in or import cookies.".into())
    } else if lower.contains("likely being blocked")
        || lower.contains("http 429")
        || lower.contains("rate limit")
    {
        ("provider_rejected", text)
    } else {
        ("provider_error", text)
    };
    let retry = error
        .downcast_ref::<ytui_providers::browse::FeedRetry>()
        .map(|r| r.after.as_millis().min(u64::MAX as u128) as u64);
    json!({"ok":false,"code":code,"error":message,"retryable":retry.is_some(),"retry_after_ms":retry})
}
fn mobile_notice(text: String) -> String {
    if text.contains("F2") || text.contains("ytui-rs auth") {
        mobile_error(&anyhow::anyhow!(text))["error"]
            .as_str()
            .unwrap_or("Use Account to sign in or import cookies.")
            .to_owned()
    } else {
        text
    }
}
#[cfg(test)]
mod mobile_error_tests {
    use super::*;
    #[test]
    fn successful_empty_pages_do_not_keep_desktop_instructions() {
        let notice = mobile_notice("No anonymous Home recommendations were returned. Press / to search, or F2 → Account → Connect browser".into());
        assert!(notice.contains("Search") && notice.contains("sign in"));
        assert!(!notice.contains("F2") && !notice.contains("Press /"));
        assert_eq!(mobile_notice("End of results".into()), "End of results");
    }
    #[test]
    fn account_errors_offer_android_actions_without_desktop_commands() {
        for text in [
            "Sign in first: use F2 → Account → Connect browser or ytui-rs auth import --browser brave-origin",
            "YouTube requires browser consent. Open YouTube then F2 → Account → Connect browser",
        ] {
            let value = mobile_error(&anyhow::anyhow!(text));
            let text = value["error"].as_str().unwrap();
            assert!(text.contains("Account"));
            assert!(
                !text.contains("F2") && !text.contains("brave-origin") && !text.contains("ytui-rs")
            );
        }
        let value = mobile_error(&anyhow::anyhow!(
            "No anonymous Home recommendations were returned. Press / to search, or F2"
        ));
        assert_eq!(value["code"], "anonymous_home_empty");
        assert!(value["error"].as_str().unwrap().contains("Search"));
    }
    #[test]
    fn transient_transport_errors_remain_retryable_without_a_service_delay() {
        let error = ytui_providers::browse::FeedRetry::new(
            "Temporary connection failure",
            std::time::Duration::ZERO,
        );
        let value = mobile_error(&error.into());
        assert_eq!(value["retryable"], true);
        assert_eq!(value["retry_after_ms"], 0);
        assert_eq!(
            mobile_error(&anyhow::anyhow!("Bad request"))["retryable"],
            false
        );
    }
}
#[cfg(target_os = "android")]
mod android {
    use super::*;
    use jni::{
        JNIEnv,
        objects::{JClass, JString},
        sys::{jlong, jstring},
    };
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_dev_ytui_android_NativeBridge_activateContext(
        mut env: JNIEnv,
        _class: JClass,
        id: JString,
    ) {
        let result = (|| -> Result<()> {
            let id: String = env.get_string(&id)?.into();
            ensure!(id.len() <= 128, "Invalid playback context");
            ENGINE
                .get()
                .context("Engine is not initialized")?
                .activate_context((!id.is_empty()).then_some(id));
            Ok(())
        })();
        if result.is_err() {
            let _ = env.throw_new(
                "java/lang/IllegalArgumentException",
                "Invalid playback context",
            );
        }
    }
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_dev_ytui_android_NativeBridge_init(
        mut env: JNIEnv,
        _class: JClass,
        root: JString,
        cache: JString,
    ) -> jstring {
        let result = (|| -> Result<()> {
            crate::platform::initialize_bridge(&mut env)?;
            let root: String = env.get_string(&root)?.into();
            let cache: String = env.get_string(&cache)?.into();
            initialize(Path::new(&root), Path::new(&cache))
        })();
        let text = match result {
            Ok(()) => String::new(),
            Err(e) => ytui_providers::diagnostic(&format!("{e:#}")),
        };
        env.new_string(text)
            .map(|s| s.into_raw())
            .unwrap_or(std::ptr::null_mut())
    }
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_dev_ytui_android_NativeBridge_beginAccountChange(
        _env: JNIEnv,
        _class: JClass,
    ) -> jlong {
        ENGINE
            .get()
            .map(|e| e.begin_account_change() as jlong)
            .unwrap_or(0)
    }
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_dev_ytui_android_NativeBridge_accountIntent(
        _env: JNIEnv,
        _class: JClass,
    ) -> jlong {
        ENGINE
            .get()
            .map(|e| e.account_intent() as jlong)
            .unwrap_or(0)
    }
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_dev_ytui_android_NativeBridge_thumbnail(
        mut env: JNIEnv,
        _class: JClass,
        media: JString,
        pixels: jni::sys::jint,
    ) -> jni::sys::jintArray {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
            || -> Result<jni::sys::jintArray> {
                let raw: String = env.get_string(&media)?.into();
                ensure!(raw.len() <= 256 * 1024, "Thumbnail request exceeds limit");
                let media: Media = serde_json::from_str(&raw)?;
                let width = pixels.clamp(32, 768) as u32;
                let image = runtime()
                    .block_on(
                        ENGINE
                            .get()
                            .context("Engine is not initialized")?
                            .thumbnail(&media, width, width),
                    )?
                    .to_rgba8();
                let mut values =
                    Vec::with_capacity(2 + image.width() as usize * image.height() as usize);
                values.extend([image.width() as i32, image.height() as i32]);
                values.extend(image.pixels().map(|p| {
                    ((p[3] as u32) << 24 | (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32)
                        as i32
                }));
                let output = env.new_int_array(values.len() as i32)?;
                env.set_int_array_region(&output, 0, &values)?;
                Ok(output.into_raw())
            },
        ));
        result
            .ok()
            .and_then(Result::ok)
            .unwrap_or(std::ptr::null_mut())
    }
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_dev_ytui_android_NativeBridge_request(
        mut env: JNIEnv,
        _class: JClass,
        request: JString,
    ) -> jstring {
        let raw = env
            .get_string(&request)
            .map(|v| v.into())
            .unwrap_or_else(|_| String::new());
        let value = std::panic::catch_unwind(|| call(&raw))
            .unwrap_or_else(|_| json!({"ok":false,"error":"Native operation failed"}).to_string());
        env.new_string(value)
            .map(|s| s.into_raw())
            .unwrap_or(std::ptr::null_mut())
    }
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_dev_ytui_android_NativeBridge_meter(
        env: JNIEnv,
        _class: JClass,
        buffer: jni::objects::JByteBuffer,
        offset: jni::sys::jint,
        length: jni::sys::jint,
        channels: jni::sys::jint,
    ) -> jlong {
        if offset < 0 || length < 0 || channels < 1 {
            return 0;
        }
        let Ok(capacity) = env.get_direct_buffer_capacity(&buffer) else {
            return 0;
        };
        let Some(end) = (offset as usize).checked_add(length as usize) else {
            return 0;
        };
        if end > capacity {
            return 0;
        }
        let Ok(pointer) = env.get_direct_buffer_address(&buffer) else {
            return 0;
        };
        // Java owns the direct decoder buffer and keeps it immutable for this call.
        let bytes =
            unsafe { std::slice::from_raw_parts(pointer.add(offset as usize), length as usize) };
        crate::meter::pcm16(bytes, channels as usize) as jlong
    }
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_dev_ytui_android_NativeBridge_cancel(
        _env: JNIEnv,
        _class: JClass,
        id: jlong,
    ) {
        if let Some(engine) = ENGINE.get() {
            engine.cancel(id as u64);
        }
    }
}

pub mod meter;
#[cfg(target_os = "android")]
mod platform;
pub mod ui;

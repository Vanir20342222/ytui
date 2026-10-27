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

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MobileState {
    pub saved: Vec<Media>,
    pub queue: Vec<Media>,
    pub positions: HashMap<String, u64>,
}
pub struct Engine {
    settings_commit: Mutex<()>,
    root: PathBuf,
    cache: PathBuf,
    config: Mutex<Config>,
    state: Mutex<MobileState>,
    session: Mutex<Option<Arc<Session>>>,
    browser: Mutex<Browser>,
    radio: Mutex<Radio>,
    images: Mutex<ImageCache>,
    cancellations: Mutex<HashMap<u64, CancellationToken>>,
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
        let session: Option<Session> = read_json(&root.join("account.json"))?;
        let session = session.map(Arc::new);
        let client = ytui_providers::http_client(config.proxy.as_deref())?;
        Ok(Self {
            settings_commit: Mutex::new(()),
            root: root.into(),
            cache: cache.into(),
            config: Mutex::new(config.clone()),
            state: Mutex::new(state),
            session: Mutex::new(session.clone()),
            browser: Mutex::new(Browser::new(client.clone(), session, config.account_index)),
            radio: Mutex::new(
                Radio::new(client.clone(), config.radio_servers.clone())
                    .with_cache(cache.join("radio")),
            ),
            images: Mutex::new(ImageCache::new(client, cache.join("images"))),
            cancellations: Mutex::new(HashMap::new()),
        })
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
    pub async fn request(&self, request: Value) -> Result<Value> {
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
                let client = ytui_providers::http_client(config.proxy.as_deref())?;
                atomic_write(
                    &self.root.join("config.json"),
                    &serde_json::to_vec(&config)?,
                )?;
                let session = self.session.lock().expect("session").clone();
                *self.browser.lock().expect("browser") =
                    Browser::new(client.clone(), session, config.account_index);
                *self.radio.lock().expect("radio") =
                    Radio::new(client.clone(), config.radio_servers.clone())
                        .with_cache(self.cache.join("radio"));
                *self.images.lock().expect("images") =
                    ImageCache::new(client, self.cache.join("images"));
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
                let page = if matches!(
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
                ensure!(
                    page.retained_bytes() <= 16 * 1024 * 1024,
                    "Feed response exceeds memory budget"
                );
                Ok(serde_json::to_value(page)?)
            }
            "saved" => Ok(serde_json::to_value(
                self.state.lock().expect("state").saved.clone(),
            )?),
            "save_media" => {
                let media: Media = serde_json::from_value(r["media"].clone())?;
                ensure!(media.retained_bytes() <= 256 * 1024, "Media is too large");
                self.mutate(|state| {
                    if let Some(i) = state.saved.iter().position(|m| m.url == media.url) {
                        state.saved.remove(i);
                    } else {
                        ensure!(state.saved.len() < 2000, "Saved library is full");
                        state.saved.push(media);
                    }
                    Ok(json!({"count":state.saved.len()}))
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
                    u64::from_le_bytes(
                        uuid::Uuid::new_v4().as_bytes()[..8]
                            .try_into()
                            .expect("UUID bytes"),
                    ) as usize
                        % state.queue.len()
                } else {
                    0
                };
                Ok(state
                    .queue
                    .get(index)
                    .map(serde_json::to_value)
                    .transpose()?
                    .unwrap_or_else(|| json!({})))
            }
            "queue_commit" => self.mutate(|state| {
                let chosen = state
                    .queue
                    .iter()
                    .position(|m| Some(m.url.as_str()) == r["url"].as_str());
                if let Some(index) = chosen {
                    state.queue.remove(index);
                }
                Ok(json!({"committed":chosen.is_some()}))
            }),
            "queue_remove" => {
                let i = r["index"].as_u64().context("Missing queue index")? as usize;
                self.mutate(|state| {
                    ensure!(i < state.queue.len(), "Queue index is out of bounds");
                    state.queue.remove(i);
                    Ok(json!({"removed":true}))
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
            "account" => Ok(json!({"connected":self.session.lock().expect("session").is_some()})),
            "lyrics" => {
                let media: Media = serde_json::from_value(r["media"].clone())?;
                let proxy = self.config.lock().expect("config").proxy.clone();
                let tracks =
                    ytui_providers::lyrics::lookup(proxy.as_deref(), &media.title, &media.creator)
                        .await?;
                Ok(serde_json::to_value(tracks)?)
            }
            "import_cookie_file" => {
                let _commit = self.settings_commit.lock().expect("settings commit");
                use std::io::Write;
                let text = r["text"].as_str().context("Missing cookie document")?;
                ensure!(
                    text.len() <= 2 * 1024 * 1024,
                    "Cookie document exceeds limit"
                );
                let mut file = tempfile::NamedTempFile::new_in(&self.cache)?;
                file.write_all(text.as_bytes())?;
                let session = ytui_providers::auth::import_netscape(file.path())?;
                let config = self.config.lock().expect("config").clone();
                let client = ytui_providers::http_client(config.proxy.as_deref())?;
                atomic_write(
                    &self.root.join("account.json"),
                    &serde_json::to_vec(&session)?,
                )?;
                let session = Arc::new(session);
                *self.session.lock().expect("session") = Some(session.clone());
                *self.browser.lock().expect("browser") =
                    Browser::new(client, Some(session), config.account_index);
                Ok(json!({"connected":true}))
            }
            "import_cookies" => {
                let _commit = self.settings_commit.lock().expect("settings commit");
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
                atomic_write(
                    &self.root.join("account.json"),
                    &serde_json::to_vec(session.as_ref())?,
                )?;
                *self.session.lock().expect("session") = Some(session.clone());
                let config = self.config.lock().expect("config").clone();
                *self.browser.lock().expect("browser") = Browser::new(
                    ytui_providers::http_client(config.proxy.as_deref())?,
                    Some(session),
                    config.account_index,
                );
                Ok(json!({"connected":true}))
            }
            "disconnect" => {
                let _commit = self.settings_commit.lock().expect("settings commit");
                match std::fs::remove_file(self.root.join("account.json")) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.into()),
                };
                *self.session.lock().expect("session") = None;
                let config = self.config.lock().expect("config").clone();
                *self.browser.lock().expect("browser") = Browser::new(
                    ytui_providers::http_client(config.proxy.as_deref())?,
                    None,
                    config.account_index,
                );
                Ok(json!({"connected":false}))
            }
            "resolve" => self.resolve(r).await,
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
        if !ytui_core::youtube_host(url.host_str().unwrap_or_default()) {
            let radio = self.radio.lock().expect("radio").clone();
            let url = radio.resolve_playlist(url.as_str()).await?;
            return Ok(json!({"url":url,"headers":{},"audio_url":null}));
        }
        let config = self.config.lock().expect("config").clone();
        let proxy = config.proxy;
        let audio = r["audio"].as_bool().unwrap_or(false);
        let quality = r["quality"]
            .as_str()
            .unwrap_or(&config.download.quality)
            .to_owned();
        let cookies = self.session.lock().expect("session").clone();
        let cookie_dir = self.cache.join("transient-auth");
        let permit = extractor_slots().acquire_owned().await?;
        let data = tokio::task::spawn_blocking(move || -> Result<Value> {
            let _permit = permit;
            let cookie_file = if let Some(session) = cookies {
                Some(session.export(&cookie_dir)?)
            } else {
                None
            };
            let jar = cookie_file
                .as_ref()
                .map(|f| yt_dlp_rs::CookieJar::load(f.path()).map(Arc::new))
                .transpose()?;
            let client = yt_dlp_rs::YoutubeDl::with_options(
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
                    proxy,
                    cookies: jar,
                    socket_timeout: Some(15.0),
                    ..Default::default()
                },
            )?;
            let format = mobile_format(audio, &quality)?;
            Ok(serde_json::to_value(
                client.extract_info_with_format(&media.url, Some(&format))?,
            )?)
        })
        .await??;
        stream(&data, audio)
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
    // H.264 is hardware-decodable on API 26 devices. Preserve a bounded VP9/muxed fallback.
    Ok(format!(
        "bestvideo[height<={quality}][vcodec^=avc1]+bestaudio/bestvideo[height<={quality}][vcodec!^=av01]+bestaudio/best[height<={quality}]"
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
    #[test]
    fn phone_quality_prefers_compatible_video_and_honors_the_selected_cap() {
        let formats = vec![
            json!({"format_id":"audio","url":"https://media.test/a","ext":"m4a","vcodec":"none","acodec":"mp4a.40.2"}),
            json!({"format_id":"720","url":"https://media.test/720","ext":"mp4","height":720,"vcodec":"avc1.64001F","acodec":"none"}),
            json!({"format_id":"1080","url":"https://media.test/1080","ext":"mp4","height":1080,"vcodec":"avc1.640028","acodec":"none"}),
            json!({"format_id":"av1","url":"https://media.test/av1","ext":"mp4","height":1080,"vcodec":"av01.0.08M.08","acodec":"none"}),
            json!({"format_id":"4k","url":"https://media.test/4k","ext":"mp4","height":2160,"vcodec":"av01.0.12M.08","acodec":"none"}),
        ];
        for (quality, expected) in [("720", 1), ("1080", 2), ("best", 4)] {
            let selector =
                yt_dlp_rs::format_select::parse(&mobile_format(false, quality).unwrap()).unwrap();
            let selected = selector
                .select(&formats, &Default::default(), None)
                .unwrap();
            assert_eq!(selected[0].formats, vec![expected, 0]);
        }
        assert!(mobile_format(false, "invalid").is_err());
        let audio = yt_dlp_rs::format_select::parse(&mobile_format(true, "1080").unwrap()).unwrap();
        assert_eq!(
            audio.select(&formats, &Default::default(), None).unwrap()[0].formats,
            vec![0]
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
        json!({"url":url,"audio_url":audio_url,"headers":headers,"live":data["is_live"].as_bool().unwrap_or(false)||data["live_status"]=="is_live","title":data["title"],"creator":data["uploader"].as_str().or_else(||data["channel"].as_str()).unwrap_or(""),"thumbnail":data["thumbnail"],"catalog":ytui_providers::formats::Catalog::parse(data)}),
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
        Err(error) => json!({"ok":false,"error":ytui_providers::diagnostic(&format!("{error:#}"))})
            .to_string(),
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

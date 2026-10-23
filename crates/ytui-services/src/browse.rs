//! Separate YouTube WEB and Music WEB_REMIX contexts; read-only InnerTube adapter.
use crate::auth::Session;
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};
use tokio::sync::Mutex;
use ytui_core::{BrowseRequest, Media, MediaKind, Page, clean_text};

/// Only transient transport/service failures may trigger automatic feed recovery.
#[derive(Debug)]
pub struct FeedRetry {
    message: String,
    pub after: Duration,
}
impl FeedRetry {
    pub fn new(message: impl Into<String>, after: Duration) -> Self {
        Self {
            message: message.into(),
            after,
        }
    }
}
impl std::fmt::Display for FeedRetry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for FeedRetry {}

fn transient(error: anyhow::Error) -> anyhow::Error {
    FeedRetry::new(error.to_string(), Duration::ZERO).into()
}
fn response_status(response: &reqwest::Response, bootstrap: bool) -> Result<()> {
    let status = response.status().as_u16();
    let wait = response
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            v.parse::<u64>().ok().map(Duration::from_secs).or_else(|| {
                httpdate::parse_http_date(v)
                    .ok()
                    .map(|t| t.duration_since(SystemTime::now()).unwrap_or_default())
            })
        })
        .unwrap_or(if status == 429 {
            Duration::from_secs(60)
        } else {
            Duration::ZERO
        });
    match status {
        200..300 => Ok(()),
        408 | 425 | 429 | 500..=599 => Err(FeedRetry::new(
            if status == 429 {
                "YouTube rate limit; retrying automatically after the service cooldown".into()
            } else {
                format!("YouTube temporarily unavailable (HTTP {status})")
            },
            wait,
        )
        .into()),
        401 | 403 => bail!(
            "YouTube rejected this session/request; re-import the selected browser profile or check service access"
        ),
        _ if bootstrap => bail!(
            "YouTube bootstrap returned HTTP {status}. Check connectivity, consent, or browser session."
        ),
        _ => bail!("YouTube returned HTTP {status}"),
    }
}

#[derive(Debug)]
struct StaleContext;
impl std::fmt::Display for StaleContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("YouTube rejected the refreshed client context")
    }
}
impl std::error::Error for StaleContext {}

#[derive(Clone)]
struct ClientContext {
    config: Value,
    loaded: Instant,
    initial_home: Option<Page>,
}

#[derive(Clone)]
pub struct Browser {
    client: reqwest::Client,
    session: Option<Arc<Session>>,
    account: u32,
    web: Arc<Mutex<Option<ClientContext>>>,
    music: Arc<Mutex<Option<ClientContext>>>,
    origins: [String; 2],
    pages: Arc<crate::cache::ResponseCache<Page>>,
}
impl Browser {
    pub fn new(client: reqwest::Client, session: Option<Arc<Session>>, account: u32) -> Self {
        Self {
            client,
            session,
            account,
            pages: Arc::new(crate::cache::ResponseCache::new(64, 8 * 1024 * 1024)),
            web: Arc::new(Mutex::new(None)),
            music: Arc::new(Mutex::new(None)),
            origins: [
                "https://www.youtube.com".into(),
                "https://music.youtube.com".into(),
            ],
        }
    }
    pub fn authenticated(&self) -> bool {
        self.session.is_some()
    }
    fn origin(&self, music: bool) -> &str {
        &self.origins[usize::from(music)]
    }
    fn authorized(
        &self,
        request: reqwest::RequestBuilder,
        origin: &str,
    ) -> Result<reqwest::RequestBuilder> {
        if let Some(s) = &self.session {
            let (cookie, authorization) = s.headers(origin)?;
            Ok(request
                .header("Cookie", cookie)
                .header("Authorization", authorization)
                .header("X-Goog-AuthUser", self.account.to_string()))
        } else {
            Ok(request)
        }
    }
    async fn bootstrap(&self, music: bool) -> Result<Value> {
        let cache = if music { &self.music } else { &self.web };
        let mut guard = cache.lock().await;
        if let Some(c) = &*guard
            && c.loaded.elapsed() < Duration::from_secs(30 * 60)
        {
            return Ok(c.config.clone());
        }
        let origin = self.origin(music);
        let mut start = url::Url::parse(origin)?;
        start.query_pairs_mut().append_pair("hl", "en");
        let response = crate::http::get(
            start,
            |url| {
                self.authorized(
                    self.client
                        .get(url.clone())
                        .header("Accept-Language", "en-US,en;q=0.9"),
                    origin,
                )
            },
            crate::http::Redirects::SameOrigin,
            if music { "YouTube Music" } else { "YouTube" },
        )
        .await
        .map_err(|error| {
            if error
                .downcast_ref::<crate::http::TransportError>()
                .is_some()
            {
                transient(error)
            } else {
                error
            }
        })?;
        response_status(&response, true)?;
        let text = limited_text(response, 8 * 1024 * 1024).await?;
        let config=extract_config(&text).context("YouTube did not return a supported client context; browser consent or a service change may require attention")?;
        let initial_home = if let Some(data) = extract_initial_data(&text) {
            self.check_session(&data)?;
            Some(parse_page(&data, music))
                .filter(|page| !page.items.is_empty() || page.notice.is_some())
        } else {
            None
        };
        *guard = Some(ClientContext {
            config: config.clone(),
            loaded: Instant::now(),
            initial_home,
        });
        Ok(config)
    }
    async fn invalidate(&self, music: bool) {
        *(if music { &self.music } else { &self.web }).lock().await = None;
    }
    async fn post(&self, music: bool, endpoint: &str, body: Value) -> Result<Value> {
        // A stale client version/visitor context can be rejected with 400/412.
        // Renew it once; never loop on authentication, consent, or rate limits.
        for attempt in 0..2 {
            let result = self.post_once(music, endpoint, body.clone()).await;
            if attempt == 0
                && result
                    .as_ref()
                    .err()
                    .is_some_and(|e| e.is::<StaleContext>())
            {
                self.invalidate(music).await;
            } else {
                return result;
            }
        }
        unreachable!()
    }
    async fn post_once(&self, music: bool, endpoint: &str, mut body: Value) -> Result<Value> {
        let config = self.bootstrap(music).await?;
        let origin = self.origin(music);
        let mut context = config
            .get("INNERTUBE_CONTEXT")
            .cloned()
            .unwrap_or_else(|| json!({"client":{}}));
        let version = config
            .get("INNERTUBE_CLIENT_VERSION")
            .and_then(Value::as_str)
            .or_else(|| {
                context
                    .pointer("/client/clientVersion")
                    .and_then(Value::as_str)
            })
            .context("Missing YouTube client version")?
            .to_owned();
        context["client"]["clientName"] = json!(if music { "WEB_REMIX" } else { "WEB" });
        context["client"]["clientVersion"] = json!(version);
        context["client"]["hl"] = json!("en");
        body["context"] = context;
        let mut request = self
            .client
            .post(format!("{origin}/youtubei/v1/{endpoint}"))
            .query(&[("prettyPrint", "false")])
            .header("Origin", origin)
            .header("X-Origin", origin)
            .header("X-Youtube-Client-Name", if music { "67" } else { "1" })
            .header("X-Youtube-Client-Version", &version);
        if let Some(key) = config.get("INNERTUBE_API_KEY").and_then(Value::as_str) {
            request = request.query(&[("key", key)]);
        }
        if let Some(visitor) = config
            .get("VISITOR_DATA")
            .or_else(|| config.pointer("/INNERTUBE_CONTEXT/client/visitorData"))
            .and_then(Value::as_str)
        {
            request = request.header("X-Goog-Visitor-Id", visitor);
        }
        let response = self
            .authorized(request, origin)?
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                transient(crate::http::request_error(
                    "YouTube API",
                    &url::Url::parse(origin).expect("validated origin"),
                    e,
                ))
            })?;
        if matches!(response.status().as_u16(), 400 | 412) {
            return Err(StaleContext.into());
        }
        response_status(&response, false)?;
        let data: Value = serde_json::from_str(&limited_text(response, 16 * 1024 * 1024).await?)
            .map_err(|_| {
                anyhow::Error::from(FeedRetry::new(
                    "YouTube returned an incomplete response",
                    Duration::from_secs(5),
                ))
            })?;
        if let Some(error) = data.get("error") {
            match error.get("code").and_then(Value::as_u64) {
                Some(400 | 412) => return Err(StaleContext.into()),
                Some(code @ (408 | 425 | 429 | 500..=599)) => {
                    return Err(FeedRetry::new(
                        format!("YouTube API temporarily unavailable ({code})"),
                        if code == 429 {
                            Duration::from_secs(60)
                        } else {
                            Duration::ZERO
                        },
                    )
                    .into());
                }
                _ => bail!(
                    "YouTube API rejected the request; check your session or this endpoint's availability"
                ),
            }
        }
        self.check_session(&data)?;
        Ok(data)
    }
    fn check_session(&self, data: &Value) -> Result<()> {
        ensure!(
            self.session.is_none()
                || data
                    .pointer("/responseContext/mainAppWebResponseContext/loggedOut")
                    .and_then(Value::as_bool)
                    != Some(true),
            "Browser session has expired; import it again"
        );
        Ok(())
    }
    pub async fn validate(&self) -> Result<String> {
        ensure!(self.session.is_some(), "No session loaded");
        let data = self.post(false, "account/account_menu", json!({})).await?;
        let account = find_key(&data, "accountName")
            .map(text)
            .filter(|s| !s.is_empty())
            .or_else(|| {
                find_key(&data, "activeAccountHeaderRenderer")
                    .and_then(|v| v.get("accountName"))
                    .map(text)
            });
        account.context("Could not validate the selected account/channel; import a signed-in YouTube session and select the correct account index")
    }
    pub async fn fetch(&self, music: bool, request: &BrowseRequest) -> Result<Page> {
        let fetch = async {
            // Home remains live so recovery/refresh never repeats an old recommendation shell.
            if matches!(request, BrowseRequest::Home) {
                return self.fetch_page(music, request).await;
            }
            let ttl = Duration::from_secs(if matches!(request, BrowseRequest::Details(_)) {
                300
            } else {
                60
            });
            self.pages
                .load(Self::page_key(music, request)?, ttl, || {
                    self.fetch_page(music, request)
                })
                .await
        };
        tokio::time::timeout(Duration::from_secs(35), fetch)
            .await
            .unwrap_or_else(|_| {
                Err(FeedRetry::new(
                    "YouTube feed request timed out; reconnecting automatically",
                    Duration::ZERO,
                )
                .into())
            })
    }
    fn page_key(music: bool, request: &BrowseRequest) -> Result<String> {
        Ok(format!("{music}:{}", serde_json::to_string(request)?))
    }
    pub fn invalidate_page(&self, music: bool, request: &BrowseRequest) {
        if let Ok(key) = Self::page_key(music, request) {
            self.pages.invalidate(&key);
        }
    }
    async fn fetch_page(&self, music: bool, request: &BrowseRequest) -> Result<Page> {
        let mut page = self.fetch_once(music, request).await?;
        // YouTube occasionally returns a bare Home shell. Renew the visitor context
        // once instead of making the user press refresh. Explicit empty-feed/consent
        // messages are meaningful and must not be hidden by retries.
        if matches!(request, BrowseRequest::Home)
            && page.items.is_empty()
            && page.continuation.is_none()
            && page.notice.is_none()
        {
            self.invalidate(music).await;
            page = self.fetch_once(music, request).await?;
            if self.session.is_some() && page.items.is_empty() && page.notice.is_none() {
                return Err(FeedRetry::new(
                    "YouTube returned an incomplete Home feed",
                    Duration::from_secs(30),
                )
                .into());
            }
        }
        if page.items.is_empty() && page.notice.is_none() {
            page.notice = Some(if matches!(request, BrowseRequest::Home) && self.session.is_none() {
                "No anonymous Home recommendations were returned. Press / to search, or F2 → Account → Connect browser to import your signed-in browser profile."
            } else if matches!(request, BrowseRequest::Search(_) | BrowseRequest::MusicSearch { .. }) {
                "No search results were returned. Try a different query; if this persists, the service response may have changed."
            } else {
                "No supported items were returned. This feed may be empty, unavailable for this account, or using a changed YouTube response. Choose another feed or check your session."
            }.into());
        }
        Ok(page)
    }
    async fn fetch_once(&self, music: bool, request: &BrowseRequest) -> Result<Page> {
        let (endpoint, body, title) = match request {
            BrowseRequest::Category { endpoint, payload } => {
                ensure!(
                    ["browse", "search", "next"].contains(&endpoint.as_str()),
                    "Unsupported category endpoint"
                );
                (endpoint.as_str(), payload.clone(), "Recommendations".into())
            }
            BrowseRequest::MusicSearch { query, category } => {
                ensure!(music, "Music categories require the Music tab");
                let bits = match category.as_str() {
                    "songs" => "II",
                    "albums" => "IY",
                    "artists" => "Ig",
                    "playlists" => "Io",
                    "videos" => "IQ",
                    _ => bail!("Unknown Music search category"),
                };
                let params = format!("EgWKAQ{bits}AUICCAFqDBAOEAoQAxAEEAkQBQ%3D%3D");
                (
                    "search",
                    json!({"query":query,"params":params}),
                    format!("{category} · {query}"),
                )
            }
            BrowseRequest::Details(id) => ("player", json!({"videoId":id}), "Video details".into()),
            BrowseRequest::Search(query) => (
                "search",
                json!({"query":query}),
                format!("Search · {query}"),
            ),
            BrowseRequest::Home => (
                "browse",
                json!({"browseId":if music{"FEmusic_home"}else{"FEwhat_to_watch"}}),
                "Home".into(),
            ),
            BrowseRequest::Subscriptions => {
                ensure!(
                    !music,
                    "Music does not provide the YouTube subscriptions feed"
                );
                self.require_auth()?;
                (
                    "browse",
                    json!({"browseId":"FEsubscriptions"}),
                    "Subscriptions".into(),
                )
            }
            BrowseRequest::Library => {
                self.require_auth()?;
                (
                    "browse",
                    json!({"browseId":if music{"FEmusic_liked_playlists"}else{"FElibrary"}}),
                    "Your library".into(),
                )
            }
            BrowseRequest::WatchLater => {
                self.require_auth()?;
                ("browse", json!({"browseId":"VLWL"}), "Watch Later".into())
            }
            BrowseRequest::Liked => {
                self.require_auth()?;
                (
                    "browse",
                    json!({"browseId":if music{"VLLM"}else{"VLLL"}}),
                    "Liked".into(),
                )
            }
            BrowseRequest::History => {
                self.require_auth()?;
                (
                    "browse",
                    json!({"browseId":if music{"FEmusic_history"}else{"FEhistory"}}),
                    "History".into(),
                )
            }
            BrowseRequest::Collection(id) => {
                ("browse", json!({"browseId":id}), "Collection".into())
            }
            BrowseRequest::Related(id) => ("next", json!({"videoId":id}), "Related".into()),
            BrowseRequest::Continue(token) => {
                let (endpoint, token) = token.split_once(':').unwrap_or(("browse", token.as_str()));
                ensure!(
                    ["browse", "search", "next", "player"].contains(&endpoint),
                    "Invalid continuation endpoint"
                );
                (endpoint, json!({"continuation":token}), "More".into())
            }
            _ => bail!("This feed belongs to the radio provider"),
        };
        if matches!(request, BrowseRequest::Home) {
            // The HTML bootstrap often already contains the complete personalized
            // feed. Consume it once while fresh, saving another network round trip.
            self.bootstrap(music).await?;
            let mut guard = (if music { &self.music } else { &self.web }).lock().await;
            if let Some(context) = &mut *guard {
                let initial = context.initial_home.take();
                if context.loaded.elapsed() < Duration::from_secs(60)
                    && let Some(mut page) = initial
                {
                    page.title = title;
                    page.continuation =
                        page.continuation.map(|token| format!("{endpoint}:{token}"));
                    return Ok(page);
                }
            }
        }
        // The watch page supplies metadata in one request. WEB's player API can
        // reject a playable video without playback context; use it only when the
        // page has no supported metadata, preserving its specific service errors.
        let embedded = if let BrowseRequest::Details(id) = request {
            match self.watch_response(music, id).await {
                Ok(data) => Some(data),
                // Do not turn a transport failure or server cooldown into an
                // immediate second request through a different endpoint.
                Err(error) if error.is::<FeedRetry>() => return Err(error),
                Err(_) => None,
            }
        } else {
            None
        };
        let data = match embedded {
            Some(data) => data,
            None => self.post(music, endpoint, body).await?,
        };
        if endpoint == "player"
            && let Some(status) = data
                .pointer("/playabilityStatus/status")
                .and_then(Value::as_str)
            && status != "OK"
        {
            let reason = data
                .pointer("/playabilityStatus/reason")
                .and_then(Value::as_str)
                .map(clean_text)
                .unwrap_or_else(|| "This video cannot be played with the selected session".into());
            bail!("Video unavailable: {reason}");
        }
        let mut page = if let Some(details) = data.get("videoDetails") {
            Page {
                items: vec![Media {
                    id: details
                        .get("videoId")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .into(),
                    title: details
                        .get("title")
                        .and_then(Value::as_str)
                        .map(clean_text)
                        .unwrap_or_default(),
                    creator: details
                        .get("author")
                        .and_then(Value::as_str)
                        .map(clean_text)
                        .unwrap_or_default(),
                    chapters: ytui_core::chapters::from_description(
                        details
                            .get("shortDescription")
                            .and_then(Value::as_str)
                            .unwrap_or_default(),
                    ),
                    description: details
                        .get("shortDescription")
                        .and_then(Value::as_str)
                        .map(clean_text)
                        .unwrap_or_default(),
                    thumbnail: thumb(details),
                    thumbnails: thumbnails(details),
                    duration: details
                        .get("lengthSeconds")
                        .and_then(Value::as_str)
                        .and_then(|s| s.parse().ok()),
                    url: format!(
                        "https://www.youtube.com/watch?v={}",
                        details
                            .get("videoId")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                    ),
                    kind: if music {
                        MediaKind::Song
                    } else {
                        MediaKind::Video
                    },
                    live: details
                        .get("isLiveContent")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    ..Default::default()
                }],
                ..Default::default()
            }
        } else {
            parse_page(&data, music)
        };
        page.title = title;
        if let Some(token) = page.continuation.take() {
            page.continuation = Some(format!("{endpoint}:{token}"));
        }
        Ok(page)
    }
    async fn watch_response(&self, music: bool, id: &str) -> Result<Value> {
        ensure!(
            id.len() == 11
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'),
            "Invalid video ID"
        );
        let origin = self.origin(music);
        let mut url = url::Url::parse(&format!("{origin}/watch"))?;
        url.query_pairs_mut()
            .append_pair("v", id)
            .append_pair("hl", "en");
        let response = crate::http::get(
            url,
            |url| self.authorized(self.client.get(url.clone()), origin),
            crate::http::Redirects::SameOrigin,
            "YouTube details",
        )
        .await
        .map_err(|error| {
            if error.is::<crate::http::TransportError>() {
                transient(error)
            } else {
                error
            }
        })?;
        response_status(&response, true)?;
        let html = limited_text(response, 8 * 1024 * 1024).await?;
        let data = extract_assignment(&html, "ytInitialPlayerResponse")
            .context("No watch-page metadata")?;
        ensure!(
            data.pointer("/videoDetails/videoId")
                .and_then(Value::as_str)
                == Some(id),
            "Watch-page video identity mismatch"
        );
        self.check_session(&data)?;
        Ok(data)
    }
    fn require_auth(&self) -> Result<()> {
        ensure!(
            self.session.is_some(),
            "Sign in first: use F2 → Account → Connect browser or `ytui-rs auth import --browser brave-origin --profile PATH`"
        );
        Ok(())
    }
}
pub async fn limited_text(mut response: reqwest::Response, limit: usize) -> Result<String> {
    let mut bytes = Vec::new();
    let origin = response.url().clone();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| transient(crate::http::request_error("Service response", &origin, e)))?
    {
        ensure!(
            bytes.len() + chunk.len() <= limit,
            "Service response exceeds the size limit"
        );
        bytes.extend_from_slice(&chunk);
    }
    String::from_utf8(bytes).context("Response was not UTF-8")
}
fn extract_config(html: &str) -> Option<Value> {
    let mut config = json!({});
    for part in html.split("ytcfg.set(").skip(1) {
        if let Some(Ok(Value::Object(obj))) = serde_json::Deserializer::from_str(part)
            .into_iter::<Value>()
            .next()
        {
            for (key, value) in obj {
                config[key] = value;
            }
        }
    }
    if config.get("INNERTUBE_CONTEXT").is_some() || config.get("INNERTUBE_CLIENT_VERSION").is_some()
    {
        Some(config)
    } else {
        None
    }
}
// Parse JSON only; never evaluate the bootstrap page's JavaScript.
fn extract_initial_data(html: &str) -> Option<Value> {
    extract_assignment(html, "ytInitialData")
}
fn extract_assignment(html: &str, name: &str) -> Option<Value> {
    for part in html.split(name).skip(1) {
        let part = part.trim_start();
        let part = part
            .strip_prefix("\"]")
            .or_else(|| part.strip_prefix("']"))
            .unwrap_or(part)
            .trim_start();
        if let Some(json) = part.strip_prefix('=')
            && let Some(Ok(value @ Value::Object(_))) =
                serde_json::Deserializer::from_str(json.trim_start())
                    .into_iter::<Value>()
                    .next()
        {
            return Some(value);
        }
    }
    None
}
pub fn text(value: &Value) -> String {
    let result = if let Some(s) = value.as_str() {
        s.into()
    } else if let Some(s) = value.get("simpleText").and_then(Value::as_str) {
        s.into()
    } else if let Some(runs) = value.get("runs").and_then(Value::as_array) {
        runs.iter()
            .filter_map(|v| v.get("text").and_then(Value::as_str))
            .collect::<String>()
    } else {
        String::new()
    };
    clean_text(&result)
}
fn find_key<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    match value {
        Value::Object(map) => map
            .get(key)
            .or_else(|| map.values().find_map(|v| find_key(v, key))),
        Value::Array(items) => items.iter().find_map(|v| find_key(v, key)),
        _ => None,
    }
}
fn duration(label: &str) -> Option<u64> {
    label
        .split(':')
        .try_fold(0u64, |v, n| Some(v * 60 + n.trim().parse::<u64>().ok()?))
}
fn thumbnails(value: &Value) -> Vec<ytui_core::Thumbnail> {
    let mut images = Vec::new();
    fn walk(v: &Value, images: &mut Vec<ytui_core::Thumbnail>) {
        match v {
            Value::Object(m) => {
                if let (Some(url), Some(width), Some(height)) = (
                    m.get("url").and_then(Value::as_str),
                    m.get("width").and_then(Value::as_u64),
                    m.get("height").and_then(Value::as_u64),
                ) && ytui_core::http_url(url).is_ok()
                    && width > 0
                    && height > 0
                    && width <= 4096
                    && height <= 4096
                    && !images.iter().any(|t| t.url == url)
                {
                    images.push(ytui_core::Thumbnail {
                        url: url.into(),
                        width: width as u32,
                        height: height as u32,
                    });
                }
                for (name, child) in m {
                    if !["avatar", "avatarViewModel", "channelThumbnail", "menu"]
                        .contains(&name.as_str())
                    {
                        walk(child, images);
                    }
                }
            }
            Value::Array(a) => {
                for child in a {
                    walk(child, images)
                }
            }
            _ => {}
        }
    }
    walk(value, &mut images);
    images
}
fn thumb(value: &Value) -> String {
    thumbnails(value)
        .into_iter()
        .max_by_key(|t| t.width as u64 * t.height as u64)
        .map(|t| t.url)
        .unwrap_or_default()
}
fn parse_lockup(r: &Value, music: bool) -> Option<Media> {
    let id = r.get("contentId")?.as_str()?;
    let metadata = r.pointer("/metadata/lockupMetadataViewModel")?;
    let title = metadata
        .pointer("/title/content")
        .and_then(Value::as_str)
        .map(clean_text)?;
    let creator = metadata
        .pointer("/metadata/contentMetadataViewModel/metadataRows/0/metadataParts/0/text/content")
        .and_then(Value::as_str)
        .map(clean_text)
        .unwrap_or_default();
    let kind = r.get("contentType").and_then(Value::as_str).unwrap_or("");
    let collection = kind.contains("PLAYLIST");
    Some(Media {
        id: id.into(),
        title,
        creator,
        thumbnail: thumb(r),
        thumbnails: thumbnails(r),
        kind: if collection {
            MediaKind::Playlist
        } else if music {
            MediaKind::Song
        } else {
            MediaKind::Video
        },
        url: if collection {
            format!("https://www.youtube.com/playlist?list={id}")
        } else {
            format!("https://www.youtube.com/watch?v={id}")
        },
        browse_id: collection.then(|| format!("VL{id}")),
        ..Default::default()
    })
}
fn parse_renderer(name: &str, r: &Value, music: bool) -> Option<Media> {
    let mut title = r
        .get("title")
        .or_else(|| r.get("headline"))
        .map(text)
        .unwrap_or_default();
    if title.is_empty() {
        title = r
            .pointer("/flexColumns/0/musicResponsiveListItemFlexColumnRenderer/text")
            .map(text)
            .unwrap_or_default();
    }
    if title.is_empty() {
        return None;
    }
    let mut item = Media {
        title,
        thumbnail: thumb(r),
        thumbnails: thumbnails(r),
        ..Default::default()
    };
    item.creator = r
        .get("ownerText")
        .or_else(|| r.get("shortBylineText"))
        .or_else(|| r.get("longBylineText"))
        .or_else(|| r.get("subtitle"))
        .map(text)
        .unwrap_or_default();
    if item.creator.is_empty() {
        item.creator = r
            .pointer("/flexColumns/1/musicResponsiveListItemFlexColumnRenderer/text")
            .map(text)
            .unwrap_or_default();
    }
    item.description = r.get("descriptionSnippet").map(text).unwrap_or_default();
    item.duration = r
        .get("lengthText")
        .map(text)
        .as_deref()
        .and_then(duration)
        .or_else(|| {
            r.pointer("/fixedColumns/0/musicResponsiveListItemFixedColumnRenderer/text")
                .map(text)
                .as_deref()
                .and_then(duration)
        });
    let video = r
        .get("videoId")
        .and_then(Value::as_str)
        .or_else(|| {
            r.pointer("/playlistItemData/videoId")
                .and_then(Value::as_str)
        })
        .or_else(|| {
            r.pointer("/navigationEndpoint/watchEndpoint/videoId")
                .and_then(Value::as_str)
        })
        .or_else(|| {
            r.pointer("/onTap/watchEndpoint/videoId")
                .and_then(Value::as_str)
        })
        .or_else(|| {
            find_key(r, "watchEndpoint")
                .and_then(|v| v.get("videoId"))
                .and_then(Value::as_str)
        });
    // Album/artist/playlist navigation takes precedence over decorative play buttons.
    let browse=r.pointer("/navigationEndpoint/browseEndpoint/browseId").and_then(Value::as_str)
        .or_else(||r.pointer("/title/runs/0/navigationEndpoint/browseEndpoint/browseId").and_then(Value::as_str))
        .or_else(||r.pointer("/flexColumns/0/musicResponsiveListItemFlexColumnRenderer/text/runs/0/navigationEndpoint/browseEndpoint/browseId").and_then(Value::as_str));
    let playlist = r.get("playlistId").and_then(Value::as_str);
    if let Some(id) = browse {
        item.id = id.into();
        item.browse_id = Some(id.into());
        item.kind = if id.starts_with("UC") {
            if music {
                MediaKind::Artist
            } else {
                MediaKind::Channel
            }
        } else if id.starts_with("MPRE") {
            MediaKind::Album
        } else {
            MediaKind::Playlist
        };
        item.url = if let Some(list) = id.strip_prefix("VL") {
            format!("https://www.youtube.com/playlist?list={list}")
        } else {
            format!(
                "https://{}/browse/{id}",
                if music {
                    "music.youtube.com"
                } else {
                    "www.youtube.com"
                }
            )
        };
    } else if let Some(id) = playlist {
        item.id = id.into();
        item.kind = MediaKind::Playlist;
        item.browse_id = Some(format!("VL{id}"));
        item.url = format!("https://www.youtube.com/playlist?list={id}");
    } else if let Some(id) = video {
        item.id = id.into();
        item.kind = if music {
            MediaKind::Song
        } else {
            MediaKind::Video
        };
        item.url = format!("https://www.youtube.com/watch?v={id}");
    } else if name == "channelRenderer" {
        let id = r.get("channelId").and_then(Value::as_str)?;
        item.id = id.into();
        item.kind = MediaKind::Channel;
        item.browse_id = Some(id.into());
        item.url = format!("https://www.youtube.com/channel/{id}");
    } else {
        return None;
    }
    item.live = r.to_string().contains("BADGE_STYLE_TYPE_LIVE_NOW");
    Some(item)
}
pub fn parse_page(data: &Value, music: bool) -> Page {
    let mut page = Page::default();
    let mut seen = HashSet::new();
    fn walk(
        value: &Value,
        music: bool,
        page: &mut Page,
        seen: &mut HashSet<String>,
        section: &str,
    ) {
        match value {
            Value::Object(map) => {
                for (name, v) in map {
                    if [
                        "shelfRenderer",
                        "richShelfRenderer",
                        "musicCarouselShelfRenderer",
                        "musicShelfRenderer",
                    ]
                    .contains(&name.as_str())
                    {
                        let title = v
                            .get("title")
                            .or_else(|| {
                                v.pointer("/header/musicCarouselShelfBasicHeaderRenderer/title")
                            })
                            .map(text)
                            .filter(|s| !s.is_empty())
                            .unwrap_or_else(|| section.into());
                        walk(v, music, page, seen, &title);
                    } else if [
                        "chipCloudChipRenderer",
                        "musicNavigationButtonRenderer",
                        "chipViewModel",
                    ]
                    .contains(&name.as_str())
                    {
                        let title = v
                            .get("text")
                            .or_else(|| v.get("buttonText"))
                            .map(text)
                            .filter(|s| !s.is_empty())
                            .or_else(|| {
                                v.pointer("/text/content")
                                    .and_then(Value::as_str)
                                    .map(clean_text)
                            })
                            .unwrap_or_default();
                        let target = [
                            ("browseEndpoint", "browse"),
                            ("searchEndpoint", "search"),
                            ("watchEndpoint", "next"),
                        ]
                        .into_iter()
                        .find_map(|(key, endpoint)| {
                            find_key(v, key).map(|payload| (endpoint, payload.clone()))
                        });
                        if let Some((endpoint, payload)) = target
                            && !title.is_empty()
                            && !page.categories.iter().any(|c| c.title == title)
                        {
                            page.categories.push(ytui_core::Category {
                                title,
                                request: BrowseRequest::Category {
                                    endpoint: endpoint.into(),
                                    payload,
                                },
                            });
                        }
                    } else if name == "lockupViewModel" {
                        if let Some(mut item) = parse_lockup(v, music)
                            && seen.insert(item.id.clone())
                        {
                            item.section = section.into();
                            page.items.push(item);
                        }
                    } else if [
                        "videoRenderer",
                        "gridVideoRenderer",
                        "compactVideoRenderer",
                        "playlistVideoRenderer",
                        "playlistRenderer",
                        "gridPlaylistRenderer",
                        "channelRenderer",
                        "musicResponsiveListItemRenderer",
                        "musicTwoRowItemRenderer",
                    ]
                    .contains(&name.as_str())
                    {
                        if let Some(mut item) = parse_renderer(name, v, music)
                            && seen.insert(item.id.clone())
                        {
                            item.section = section.into();
                            page.items.push(item);
                        }
                        // Do not recurse into menus/overlays and mistake their actions for cards.
                    } else if name == "continuationCommand" || name == "nextContinuationData" {
                        if page.continuation.is_none() {
                            page.continuation = v
                                .get("token")
                                .or_else(|| v.get("continuation"))
                                .and_then(Value::as_str)
                                .map(str::to_owned);
                        }
                    } else if name == "messageRenderer" || name == "backgroundPromoRenderer" {
                        let message = v
                            .get("text")
                            .or_else(|| v.get("bodyText"))
                            .map(text)
                            .unwrap_or_default();
                        if !message.is_empty() {
                            page.notice = Some(message);
                        }
                    } else if name != "menu" && name != "trackingParams" {
                        walk(v, music, page, seen, section);
                    }
                }
            }
            Value::Array(items) => {
                for item in items {
                    walk(item, music, page, seen, section);
                }
            }
            _ => {}
        }
    }
    walk(data, music, &mut page, &mut seen, "");
    page
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_music_album_and_song_without_overlay_duplicates() {
        let data = json!({"contents":[{"musicTwoRowItemRenderer":{"title":{"runs":[{"text":"Album"}]},"navigationEndpoint":{"browseEndpoint":{"browseId":"MPREtest"}},"overlay":{"watchEndpoint":{"videoId":"decoration"}}}},
            {"musicResponsiveListItemRenderer":{"flexColumns":[{"musicResponsiveListItemFlexColumnRenderer":{"text":{"runs":[{"text":"Song"}]}}}],"playlistItemData":{"videoId":"song"}}}],"continuations":[{"nextContinuationData":{"continuation":"token"}}]});
        let page = parse_page(&data, true);
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.items[0].kind, MediaKind::Album);
        assert_eq!(page.items[1].id, "song");
        assert_eq!(page.continuation.as_deref(), Some("token"));
    }
    #[test]
    fn combines_multiple_ytcfg_calls() {
        let c=extract_config("ytcfg.set({\"INNERTUBE_CLIENT_VERSION\":\"test\"});ytcfg.set({\"VISITOR_DATA\":\"x\"});").unwrap();
        assert_eq!(c["VISITOR_DATA"], "x");
    }
}

#[cfg(test)]
mod modern_tests {
    use super::*;
    #[test]
    fn parses_home_lockups_and_preserves_search_continuations() {
        let data = json!({"contents":[{"lockupViewModel":{"contentId":"abcdefghijk","contentType":"LOCKUP_CONTENT_TYPE_VIDEO","metadata":{"lockupMetadataViewModel":{"title":{"content":"A modern card"},"metadata":{"contentMetadataViewModel":{"metadataRows":[{"metadataParts":[{"text":{"content":"Creator"}}]}]}}}}}}],"continuationCommand":{"token":"next"}});
        let page = parse_page(&data, false);
        assert_eq!(page.items[0].title, "A modern card");
        assert_eq!(page.items[0].creator, "Creator");
        assert_eq!(page.continuation.as_deref(), Some("next"));
    }
}

#[cfg(test)]
mod request_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    fn client() -> reqwest::Client {
        reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .unwrap()
    }
    #[tokio::test]
    async fn repeated_and_concurrent_searches_share_one_request_per_provider() {
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let (base, task) = crate::test_http::serve_request(move |request| {
            let body = if request.starts_with("GET") {
                r#"ytcfg.set({"INNERTUBE_CLIENT_VERSION":"fixture"});"#.into()
            } else {
                count.fetch_add(1, Ordering::SeqCst);
                json!({"contents":[{"videoRenderer":{"videoId":"cached","title":{"simpleText":"Cached"}}}]}).to_string()
            };
            (200, String::new(), body.into_bytes())
        }).await;
        let mut browser = Browser::new(client(), None, 0);
        browser.origins = [base.clone(), base];
        let request = BrowseRequest::Search("repeat".into());
        let (a, b) = tokio::join!(
            browser.fetch(false, &request),
            browser.fetch(false, &request)
        );
        assert_eq!(a.unwrap().items[0].id, "cached");
        assert_eq!(b.unwrap().items[0].id, "cached");
        browser.fetch(false, &request).await.unwrap();
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "identical searches must share their network result"
        );
        browser.fetch(true, &request).await.unwrap();
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "Music and YouTube must not share pages"
        );
        browser.invalidate_page(false, &request);
        browser.fetch(false, &request).await.unwrap();
        assert_eq!(
            calls.load(Ordering::SeqCst),
            3,
            "explicit refresh must fetch again"
        );
        task.abort();
    }
    #[tokio::test]
    async fn rejected_player_api_recovers_metadata_from_the_watch_page() {
        let (base, task) = crate::test_http::serve_request(|request| {
            let body = if request.starts_with("GET /watch?") {
                r#"var ytInitialPlayerResponse = {"playabilityStatus":{"status":"OK"},"videoDetails":{"videoId":"abcdefghijk","title":"Playable video","lengthSeconds":"19"},"streamingData":{"formats":[{"url":"https://cdn.test/private?sig=synthetic"}]}};"#.into()
            } else if request.starts_with("GET") {
                r#"ytcfg.set({"INNERTUBE_CLIENT_VERSION":"fixture"});"#.into()
            } else {
                json!({"playabilityStatus":{"status":"ERROR","reason":"Video unavailable"}}).to_string()
            };
            (200, String::new(), body.into_bytes())
        }).await;
        let mut browser = Browser::new(client(), None, 0);
        browser.origins = [base.clone(), base];
        let page = browser
            .fetch(false, &BrowseRequest::Details("abcdefghijk".into()))
            .await
            .unwrap();
        assert_eq!(page.items[0].title, "Playable video");
        assert!(!serde_json::to_string(&page).unwrap().contains("synthetic"));
        task.abort();
    }
    #[tokio::test]
    async fn redirected_bootstrap_search_empty_home_and_music_keep_separate_contexts() {
        let bootstraps = Arc::new(AtomicUsize::new(0));
        let count = bootstraps.clone();
        let (base, task) = crate::test_http::serve_request(move |request| {
            let (headers, body) = request.split_once("\r\n\r\n").unwrap();
            let target = headers.lines().next().unwrap().split_whitespace().nth(1).unwrap();
            let lower = headers.to_ascii_lowercase();
            assert!(!lower.contains("\ncookie:"));
            assert!(!lower.contains("\nauthorization:"));
            let music = target.starts_with("/music");
            let version = if music {"music-fixture"} else {"web-fixture"};
            if target.starts_with("/web?") || target.starts_with("/music?") {
                count.fetch_add(1, Ordering::SeqCst);
                return (302, format!("Location: /{}-ready\r\n", if music {"music"} else {"web"}), vec![]);
            }
            if target.ends_with("-ready") {
                let config = json!({"INNERTUBE_CONTEXT":{"client":{"clientVersion":version,"visitorData":"synthetic-visitor"}},"INNERTUBE_API_KEY":"synthetic-key"});
                return (200, String::new(), format!("<script>ytcfg.set({config});</script>").into_bytes());
            }
            if target.contains("/watch?") {
                return (200, String::new(), b"No embedded player metadata".to_vec());
            }
            let body: Value = serde_json::from_str(body).unwrap();
            assert_eq!(body.pointer("/context/client/clientName").unwrap(), if music {"WEB_REMIX"} else {"WEB"});
            assert_eq!(body.pointer("/context/client/clientVersion").unwrap(), version);
            assert!(lower.contains("x-goog-visitor-id: synthetic-visitor"));
            assert!(lower.contains(&format!("x-youtube-client-name: {}", if music {"67"} else {"1"})));
            let data = if body.get("videoId").is_some() {
                if body["videoId"] == "unavailable" {json!({"playabilityStatus":{"status":"ERROR","reason":"Fixture video is private"}})}
                else {json!({"playabilityStatus":{"status":"OK"},"videoDetails":{"videoId":"synthetic","title":"Fixture details","lengthSeconds":"20"}})}
            } else if body.get("continuation").is_some() {
                assert!(target.contains("/search?"));
                assert_eq!(body["continuation"], "page2");
                json!({"contents":[{"videoRenderer":{"videoId":"synthetic-page2","title":{"simpleText":"Next result"}}}]})
            } else if target.contains("/search?") {
                assert_eq!(body["query"], "fixture & song");
                json!({"contents":[{"videoRenderer":{"videoId":"synthetic","title":{"simpleText":"Fixture result"}}}],"continuationCommand":{"token":"page2"}})
            } else {
                assert_eq!(body["browseId"], if music {"FEmusic_home"} else {"FEwhat_to_watch"});
                json!({"contents":[]})
            };
            (200, "Content-Type: application/json\r\n".into(), data.to_string().into_bytes())
        }).await;
        let mut browser = Browser::new(client(), None, 0);
        browser.origins = [format!("{base}/web"), format!("{base}/music")];
        for music in [false, true] {
            let home = browser.fetch(music, &BrowseRequest::Home).await.unwrap();
            assert!(
                home.notice
                    .unwrap()
                    .contains("F2 → Account → Connect browser")
            );
            let results = browser
                .fetch(music, &BrowseRequest::Search("fixture & song".into()))
                .await
                .unwrap();
            assert_eq!(results.items.len(), 1);
            assert_eq!(results.continuation.as_deref(), Some("search:page2"));
            let next = browser
                .fetch(
                    music,
                    &BrowseRequest::Continue(results.continuation.unwrap()),
                )
                .await
                .unwrap();
            assert_eq!(next.items[0].id, "synthetic-page2");
            let details = browser
                .fetch(music, &BrowseRequest::Details("synthetic".into()))
                .await
                .unwrap();
            assert_eq!(details.items[0].title, "Fixture details");
            let error = browser
                .fetch(music, &BrowseRequest::Details("unavailable".into()))
                .await
                .unwrap_err()
                .to_string();
            assert!(error.contains("Fixture video is private"));
        }
        for category in ["songs", "albums", "artists", "videos", "playlists"] {
            let page = browser
                .fetch(
                    true,
                    &BrowseRequest::MusicSearch {
                        query: "fixture & song".into(),
                        category: category.into(),
                    },
                )
                .await
                .unwrap();
            assert_eq!(page.items.len(), 1);
        }
        assert_eq!(bootstraps.load(Ordering::SeqCst), 4);
        task.abort();
        let _ = task.await;
    }
    #[tokio::test]
    async fn consent_rate_limit_and_rejected_requests_have_actionable_errors() {
        for (status, location, body, expected) in [
            (
                302,
                "Location: https://consent.youtube.com/m?token=synthetic-secret\r\n",
                "",
                "F2 → Account → Connect browser",
            ),
            (429, "", "", "rate limit"),
            (403, "", "", "re-import"),
        ] {
            let (base, task) = crate::test_http::serve_request(move |request| {
                if request.starts_with("GET") && status != 302 {
                    return (
                        200,
                        String::new(),
                        b"ytcfg.set({\"INNERTUBE_CLIENT_VERSION\":\"fixture\"});".to_vec(),
                    );
                }
                (status, location.into(), body.as_bytes().to_vec())
            })
            .await;
            let mut browser = Browser::new(client(), None, 0);
            browser.origins = [base.clone(), base];
            let error = browser
                .fetch(true, &BrowseRequest::Search("test".into()))
                .await
                .unwrap_err()
                .to_string();
            assert!(error.contains(expected), "{error}");
            assert!(!error.contains("synthetic-secret"));
            task.abort();
        }
    }
    #[test]
    fn initial_home_json_is_parsed_without_executing_scripts() {
        for assignment in [
            "var ytInitialData =",
            "window[\"ytInitialData\"] =",
            "window['ytInitialData']=",
            "ytInitialData=",
        ] {
            let html = format!(
                "<script>{assignment} {{\"contents\":[]}}; throw new Error('never execute');</script>"
            );
            assert_eq!(extract_initial_data(&html), Some(json!({"contents":[]})));
        }
        assert!(extract_initial_data("ytInitialData = window.secret;").is_none());
        let browser = Browser::new(
            client(),
            Some(Arc::new(Session {
                cookies: vec![],
                account: "synthetic".into(),
                imported_at: 0,
            })),
            0,
        );
        assert!(
            browser
                .check_session(
                    &json!({"responseContext":{"mainAppWebResponseContext":{"loggedOut":true}}})
                )
                .is_err()
        );
    }
    #[tokio::test]
    async fn home_uses_fresh_html_feed_once_and_then_requests_new_data() {
        for music in [false, true] {
            let gets = Arc::new(AtomicUsize::new(0));
            let posts = Arc::new(AtomicUsize::new(0));
            let (g, p) = (gets.clone(), posts.clone());
            let (base,task) = crate::test_http::serve_request(move |request| {
                let id = if request.starts_with("GET") {g.fetch_add(1,Ordering::SeqCst); "html-home"}
                    else {p.fetch_add(1,Ordering::SeqCst); "fresh-home"};
                let data = json!({"contents":[{"videoRenderer":{"videoId":id,"title":{"simpleText":"Recommendation"}}}],"continuationCommand":{"token":"next-page"}});
                let body = if request.starts_with("GET") {
                    format!("ytcfg.set({{\"INNERTUBE_CLIENT_VERSION\":\"fixture\"}});var ytInitialData = {data};")
                } else {data.to_string()};
                (200,String::new(),body.into_bytes())
            }).await;
            let mut browser = Browser::new(client(), None, 0);
            browser.origins = [base.clone(), base];
            let first = browser.fetch(music, &BrowseRequest::Home).await.unwrap();
            assert_eq!(first.items[0].id, "html-home");
            assert_eq!(first.continuation.as_deref(), Some("browse:next-page"));
            assert_eq!(
                posts.load(Ordering::SeqCst),
                0,
                "initial home should need only one request"
            );
            let second = browser.fetch(music, &BrowseRequest::Home).await.unwrap();
            assert_eq!(second.items[0].id, "fresh-home");
            assert_eq!(gets.load(Ordering::SeqCst), 1);
            assert_eq!(posts.load(Ordering::SeqCst), 1);
            // A bootstrap loaded by another endpoint cannot provide an old Home snapshot.
            browser.invalidate(music).await;
            browser.bootstrap(music).await.unwrap();
            (if music { &browser.music } else { &browser.web })
                .lock()
                .await
                .as_mut()
                .unwrap()
                .loaded = Instant::now() - Duration::from_secs(61);
            let third = browser.fetch(music, &BrowseRequest::Home).await.unwrap();
            assert_eq!(third.items[0].id, "fresh-home");
            assert_eq!(posts.load(Ordering::SeqCst), 2);
            task.abort();
        }
    }
    #[tokio::test]
    async fn stale_context_renews_once_and_expired_context_refreshes_before_use() {
        let gets = Arc::new(AtomicUsize::new(0));
        let posts = Arc::new(AtomicUsize::new(0));
        let (g, p) = (gets.clone(), posts.clone());
        let (base, task) = crate::test_http::serve_request(move |request| {
            if request.starts_with("GET") {
                let n = g.fetch_add(1, Ordering::SeqCst) + 1;
                return (200, String::new(), format!("ytcfg.set({{\"INNERTUBE_CLIENT_VERSION\":\"v{n}\"}});").into_bytes());
            }
            p.fetch_add(1, Ordering::SeqCst);
            if request.contains("x-youtube-client-version: v1") {
                return (400, String::new(), vec![]);
            }
            (200, String::new(), br#"{"contents":[{"videoRenderer":{"videoId":"recovered","title":{"simpleText":"Recovered"}}}]}"#.to_vec())
        }).await;
        let mut browser = Browser::new(client(), None, 0);
        browser.origins = [base.clone(), base];
        let page = browser
            .fetch(false, &BrowseRequest::Search("test".into()))
            .await
            .unwrap();
        assert_eq!(page.items[0].id, "recovered");
        assert_eq!(gets.load(Ordering::SeqCst), 2);
        assert_eq!(posts.load(Ordering::SeqCst), 2);
        browser.web.lock().await.as_mut().unwrap().loaded =
            Instant::now() - Duration::from_secs(1801);
        browser.invalidate_page(false, &BrowseRequest::Search("test".into()));
        browser
            .fetch(false, &BrowseRequest::Search("test".into()))
            .await
            .unwrap();
        assert_eq!(gets.load(Ordering::SeqCst), 3);
        // Refreshing WEB must not prime/replace the Music context.
        assert!(browser.music.lock().await.is_none());
        task.abort();
    }
    #[tokio::test]
    async fn bare_home_shell_recovers_without_refresh_and_explicit_notice_is_not_retried() {
        for music in [false, true] {
            let calls = Arc::new(AtomicUsize::new(0));
            let count = calls.clone();
            let (base, task) = crate::test_http::serve_request(move |request| {
                if request.starts_with("GET") {
                    return (200, String::new(), br#"ytcfg.set({"INNERTUBE_CLIENT_VERSION":"fixture"});"#.to_vec());
                }
                let n = count.fetch_add(1, Ordering::SeqCst);
                let data = match n {
                    0 => json!({"contents": []}),
                    1 => json!({"contents":[{"videoRenderer":{"videoId":"home","title":{"simpleText":"Recovered Home"}}}]}),
                    _ => json!({"backgroundPromoRenderer":{"bodyText":{"simpleText":"Turn on watch history"}}}),
                };
                (200, String::new(), data.to_string().into_bytes())
            }).await;
            let mut browser = Browser::new(client(), None, 0);
            browser.origins = [base.clone(), base];
            let page = browser.fetch(music, &BrowseRequest::Home).await.unwrap();
            assert_eq!(page.items[0].id, "home");
            assert_eq!(calls.load(Ordering::SeqCst), 2);
            let page = browser.fetch(music, &BrowseRequest::Home).await.unwrap();
            assert_eq!(page.notice.as_deref(), Some("Turn on watch history"));
            assert_eq!(calls.load(Ordering::SeqCst), 3);
            task.abort();
        }
    }
    #[tokio::test]
    async fn interrupted_bootstrap_and_api_bodies_retry_then_recover() {
        for bootstrap in [false, true] {
            let calls = Arc::new(AtomicUsize::new(0));
            let observed = calls.clone();
            let (base, task) = crate::test_http::serve_raw_request(move |request| {
                let get = request.starts_with("GET");
                if get == bootstrap && observed.fetch_add(1, Ordering::SeqCst) == 0 {
                    return b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\nConnection: close\r\n\r\n{partial".to_vec();
                }
                let body = if get { r#"ytcfg.set({"INNERTUBE_CLIENT_VERSION":"fixture"});"# }
                    else { r#"{"contents":[{"videoRenderer":{"videoId":"recovered","title":{"simpleText":"Recovered"}}}]}"# };
                format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).into_bytes()
            }).await;
            let mut browser = Browser::new(client(), None, 0);
            browser.origins = [base.clone(), base];
            let request = BrowseRequest::Search("fixture".into());
            let error = browser.fetch(false, &request).await.unwrap_err();
            assert!(error.is::<FeedRetry>(), "{error}");
            assert_eq!(
                browser.fetch(false, &request).await.unwrap().items[0].id,
                "recovered"
            );
            task.abort();
        }
    }

    #[tokio::test]
    async fn details_obey_watch_page_rate_limits_without_an_api_fallback() {
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let (base, task) = crate::test_http::serve_request(move |request| {
            observed.fetch_add(1, Ordering::SeqCst);
            if request.starts_with("GET /watch?") {
                (429, "Retry-After: 120\r\n".into(), vec![])
            } else if request.starts_with("GET") {
                (
                    200,
                    String::new(),
                    br#"ytcfg.set({"INNERTUBE_CLIENT_VERSION":"fixture"});"#.to_vec(),
                )
            } else {
                (
                    200,
                    String::new(),
                    br#"{"videoDetails":{"videoId":"abcdefghijk","title":"Unexpected fallback"}}"#
                        .to_vec(),
                )
            }
        })
        .await;
        let mut browser = Browser::new(client(), None, 0);
        browser.origins = [base.clone(), base];
        let result = browser
            .fetch(false, &BrowseRequest::Details("abcdefghijk".into()))
            .await;
        task.abort();
        let error = result.expect_err("Watch-page cooldown must not be bypassed");
        assert_eq!(
            error.downcast_ref::<FeedRetry>().unwrap().after,
            Duration::from_secs(120)
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn transient_statuses_retry_after_and_permanent_errors_are_distinguished() {
        for (status, headers, body, retry, minimum) in [
            (503, "", "", true, 0),
            (408, "", "", true, 0),
            (429, "Retry-After: 120\r\n", "", true, 120),
            (429, "", "", true, 60),
            (200, "", r#"{"error":{"code":503}}"#, true, 0),
            (200, "", "{incomplete", true, 5),
            (403, "", "", false, 0),
            (401, "", "", false, 0),
            (404, "", "", false, 0),
            (400, "", "", false, 0),
        ] {
            let posts = Arc::new(AtomicUsize::new(0));
            let count = posts.clone();
            let (base, task) = crate::test_http::serve_request(move |request| {
                if request.starts_with("GET") {
                    return (
                        200,
                        String::new(),
                        br#"ytcfg.set({"INNERTUBE_CLIENT_VERSION":"fixture"});"#.to_vec(),
                    );
                }
                count.fetch_add(1, Ordering::SeqCst);
                (status, headers.into(), body.as_bytes().to_vec())
            })
            .await;
            let mut browser = Browser::new(client(), None, 0);
            browser.origins = [base.clone(), base];
            let error = browser
                .fetch(false, &BrowseRequest::Home)
                .await
                .unwrap_err();
            assert_eq!(error.is::<FeedRetry>(), retry, "HTTP {status}: {error}");
            if retry {
                assert!(
                    error.downcast_ref::<FeedRetry>().unwrap().after
                        >= Duration::from_secs(minimum)
                );
            }
            assert_eq!(
                posts.load(Ordering::SeqCst),
                if status == 400 { 2 } else { 1 }
            );
            task.abort();
        }
    }
    #[tokio::test]
    async fn bootstrap_rate_limits_http_dates_and_transport_failures_are_retryable() {
        let date = httpdate::fmt_http_date(SystemTime::now() + Duration::from_secs(120));
        let (base, task) =
            crate::test_http::serve(move |_| (429, format!("Retry-After: {date}\r\n"), vec![]))
                .await;
        let mut browser = Browser::new(client(), None, 0);
        browser.origins = [base.clone(), base.clone()];
        let error = browser
            .fetch(false, &BrowseRequest::Home)
            .await
            .unwrap_err();
        let delay = error.downcast_ref::<FeedRetry>().unwrap().after;
        assert!(delay > Duration::from_secs(115) && delay <= Duration::from_secs(120));
        task.abort();
        let _ = task.await;
        let error = browser
            .fetch(false, &BrowseRequest::Home)
            .await
            .unwrap_err();
        assert!(error.is::<FeedRetry>(), "{error}");
        assert!(error.to_string().contains("cannot reach"));
        assert!(!error.to_string().contains("?"));
    }
    #[test]
    fn empty_feed_preserves_service_message_without_terminal_controls() {
        let page = parse_page(
            &json!({"contents":{"backgroundPromoRenderer":{"bodyText":{"simpleText":"Turn on watch history\u{001b}[31m"}}}}),
            false,
        );
        let message = page.notice.unwrap();
        assert!(message.contains("Turn on watch history"));
        assert!(!message.contains('\u{001b}'));
    }
}

#[cfg(test)]
mod category_tests {
    use super::*;
    #[test]
    fn shelves_keep_labels_thumbnail_variants_and_real_category_payloads() {
        let page = parse_page(
            &json!({"contents":[
                {"chipCloudChipRenderer":{"text":{"simpleText":"Music"},"navigationEndpoint":{"browseEndpoint":{"browseId":"FEwhat_to_watch","params":"server-filter"}}}},
                {"musicCarouselShelfRenderer":{"header":{"musicCarouselShelfBasicHeaderRenderer":{"title":{"runs":[{"text":"Listen again"}]}}},"contents":[
                    {"musicResponsiveListItemRenderer":{"flexColumns":[{"musicResponsiveListItemFlexColumnRenderer":{"text":{"runs":[{"text":"Song"}]}}}],"playlistItemData":{"videoId":"fixture"},"thumbnail":{"thumbnails":[{"url":"https://cdn.test/small","width":120,"height":120},{"url":"https://cdn.test/large","width":600,"height":600}]}}}
                ]}},
                {"continuationCommand":{"token":"next"}}
            ]}),
            true,
        );
        assert_eq!(page.items[0].section, "Listen again");
        assert_eq!(page.items[0].thumbnails.len(), 2);
        assert_eq!(page.categories[0].title, "Music");
        let BrowseRequest::Category { endpoint, payload } = &page.categories[0].request else {
            panic!("category was lost")
        };
        assert_eq!(endpoint, "browse");
        assert_eq!(payload["params"], "server-filter");
        assert_eq!(page.continuation.as_deref(), Some("next"));
    }
}

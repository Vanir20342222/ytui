//! Separate YouTube WEB and Music WEB_REMIX contexts; read-only InnerTube adapter.
use crate::auth::Session;
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{collections::HashSet, sync::Arc};
use tokio::sync::Mutex;
use ytui_core::{BrowseRequest, Media, MediaKind, Page, clean_text};

#[derive(Clone)]
pub struct Browser {
    client: reqwest::Client,
    session: Option<Arc<Session>>,
    account: u32,
    web: Arc<Mutex<Option<Value>>>,
    music: Arc<Mutex<Option<Value>>>,
}
impl Browser {
    pub fn new(client: reqwest::Client, session: Option<Arc<Session>>, account: u32) -> Self {
        Self {
            client,
            session,
            account,
            web: Arc::new(Mutex::new(None)),
            music: Arc::new(Mutex::new(None)),
        }
    }
    pub fn authenticated(&self) -> bool {
        self.session.is_some()
    }
    fn origin(music: bool) -> &'static str {
        if music {
            "https://music.youtube.com"
        } else {
            "https://www.youtube.com"
        }
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
        if let Some(c) = &*guard {
            return Ok(c.clone());
        }
        let origin = Self::origin(music);
        let response = self
            .authorized(
                self.client
                    .get(origin)
                    .header("Accept-Language", "en-US,en;q=0.9"),
                origin,
            )?
            .send()
            .await
            .context("Cannot contact YouTube; check network/proxy")?;
        ensure!(
            response.status().is_success(),
            "YouTube bootstrap returned HTTP {}. Check connectivity, consent, or browser session.",
            response.status().as_u16()
        );
        let text = limited_text(response, 8 * 1024 * 1024).await?;
        let config=extract_config(&text).context("YouTube did not return a supported client context; browser consent or a service change may require attention")?;
        *guard = Some(config.clone());
        Ok(config)
    }
    async fn post(&self, music: bool, endpoint: &str, mut body: Value) -> Result<Value> {
        let config = self.bootstrap(music).await?;
        let origin = Self::origin(music);
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
        if let Some(visitor) = config.get("VISITOR_DATA").and_then(Value::as_str) {
            request = request.header("X-Goog-Visitor-Id", visitor);
        }
        let response = self
            .authorized(request, origin)?
            .json(&body)
            .send()
            .await
            .context("YouTube request failed; check network/proxy")?;
        match response.status().as_u16() {
            401 | 403 => bail!(
                "YouTube rejected this session/request; re-import the selected browser profile or check service access"
            ),
            429 => bail!("YouTube rate limit; wait before refreshing"),
            n if !(200..300).contains(&n) => bail!("YouTube returned HTTP {n}"),
            _ => {}
        }
        let data: Value = serde_json::from_str(&limited_text(response, 16 * 1024 * 1024).await?)
            .context("YouTube returned an invalid response")?;
        ensure!(
            data.get("error").is_none(),
            "YouTube API rejected the request; refresh your session or check this endpoint's availability"
        );
        if self.session.is_some()
            && data
                .pointer("/responseContext/mainAppWebResponseContext/loggedOut")
                .and_then(Value::as_bool)
                == Some(true)
        {
            bail!("Browser session has expired; import it again");
        }
        Ok(data)
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
        let (endpoint, body, title) = match request {
            BrowseRequest::MusicSearch { query, category } => {
                ensure!(music, "Music categories require the Music tab");
                let bits = match category.as_str() {
                    "songs" => "II",
                    "albums" => "IY",
                    "artists" => "Ig",
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
        let data = self.post(music, endpoint, body).await?;
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
        if page.items.is_empty() {
            page.notice=Some("No supported items were returned. This may be an empty feed, an unavailable account feature, or a changed YouTube response. Try another feed or refresh your session.".into());
        }
        Ok(page)
    }
    fn require_auth(&self) -> Result<()> {
        ensure!(
            self.session.is_some(),
            "Sign in first: use :auth or `ytui auth import --browser brave --profile PATH`"
        );
        Ok(())
    }
}
pub async fn limited_text(mut response: reqwest::Response, limit: usize) -> Result<String> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
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
fn thumb(value: &Value) -> String {
    let mut best = (0, String::new());
    fn walk(v: &Value, best: &mut (u64, String)) {
        match v {
            Value::Object(m) => {
                if let Some(url) = m.get("url").and_then(Value::as_str)
                    && m.contains_key("width")
                {
                    let width = m.get("width").and_then(Value::as_u64).unwrap_or(0);
                    if width >= best.0 {
                        *best = (width, url.to_owned());
                    }
                }
                for child in m.values() {
                    walk(child, best);
                }
            }
            Value::Array(a) => {
                for child in a {
                    walk(child, best);
                }
            }
            _ => {}
        }
    }
    walk(value, &mut best);
    best.1
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
    fn walk(value: &Value, music: bool, page: &mut Page, seen: &mut HashSet<String>) {
        match value {
            Value::Object(map) => {
                for (name, v) in map {
                    if name == "lockupViewModel" {
                        if let Some(item) = parse_lockup(v, music)
                            && seen.insert(item.id.clone())
                        {
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
                        if let Some(item) = parse_renderer(name, v, music)
                            && seen.insert(item.id.clone())
                        {
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
                    } else if name != "menu" && name != "trackingParams" {
                        walk(v, music, page, seen);
                    }
                }
            }
            Value::Array(items) => {
                for item in items {
                    walk(item, music, page, seen);
                }
            }
            _ => {}
        }
    }
    walk(data, music, &mut page, &mut seen);
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

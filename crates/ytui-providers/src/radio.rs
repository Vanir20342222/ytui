use crate::browse::limited_text;
use anyhow::{Context, Result, bail, ensure};
use futures_util::{StreamExt, stream::FuturesUnordered};
use serde_json::Value;
use ytui_core::{BrowseRequest, Media, MediaKind, Page, Thumbnail, http_url};

#[derive(Clone)]
pub struct Radio {
    client: reqwest::Client,
    pub servers: Vec<String>,
    cache: Option<std::path::PathBuf>,
    catalog: String,
    discovery_endpoint: String,
    discovery: std::sync::Arc<crate::cache::ResponseCache<Vec<String>>>,
    pages: std::sync::Arc<crate::cache::ResponseCache<Page>>,
}
impl Radio {
    pub fn new(client: reqwest::Client, servers: Vec<String>) -> Self {
        Self {
            client,
            servers,
            cache: None,
            catalog: "https://radio.cliamp.stream/stations".into(),
            discovery_endpoint: "https://all.api.radio-browser.info/json/servers".into(),
            discovery: std::sync::Arc::new(crate::cache::ResponseCache::new(1, 4096)),
            pages: std::sync::Arc::new(crate::cache::ResponseCache::new(32, 4 * 1024 * 1024)),
        }
    }
    pub fn with_cache(mut self, path: std::path::PathBuf) -> Self {
        self.cache = Some(path);
        self.pages = std::sync::Arc::new(crate::cache::ResponseCache::new(32, 4 * 1024 * 1024));
        self
    }
    pub async fn fetch(&self, request: &BrowseRequest, offset: usize) -> Result<Page> {
        self.pages
            .load(
                self.page_key(request, offset)?,
                std::time::Duration::from_secs(60),
                || self.fetch_uncached(request, offset),
            )
            .await
    }
    fn page_key(&self, request: &BrowseRequest, offset: usize) -> Result<String> {
        Ok(format!(
            "{}:{:?}:{offset}:{}",
            self.catalog,
            self.servers,
            serde_json::to_string(request)?
        ))
    }
    pub fn invalidate_page(&self, request: &BrowseRequest, offset: usize) {
        self.discovery.clear();
        if let Ok(key) = self.page_key(request, offset) {
            self.pages.invalidate(&key);
        }
    }
    async fn fetch_uncached(&self, request: &BrowseRequest, offset: usize) -> Result<Page> {
        match self.fetch_cached(request, offset).await {
            Err(error) if matches!(request, BrowseRequest::Cliamp) => {
                let mut page = self
                    .fetch_cached(&BrowseRequest::RadioSearch(String::new()), offset)
                    .await
                    .map_err(|fallback| {
                        anyhow::anyhow!("{error}. Radio Browser fallback also failed: {fallback}")
                    })?;
                page.notice = Some(format!(
                    "cliamp catalog unavailable: {error}. Showing Radio Browser stations instead. Refresh to retry cliamp."
                ));
                Ok(page)
            }
            result => result,
        }
    }
    async fn fetch_cached(&self, request: &BrowseRequest, offset: usize) -> Result<Page> {
        use sha2::{Digest, Sha256};
        // Provider identity matters for disk fallback as well as memory hits.
        // Old unscoped cache files are left intact but cannot cross directories.
        let key = hex::encode(Sha256::digest(format!(
            "v2:{}",
            self.page_key(request, offset)?
        )));
        let path = self.cache.as_ref().map(|p| p.join(key));
        match self.fetch_live(request, offset).await {
            Ok(page) => {
                if let Some(path) = path {
                    let data = serde_json::to_vec(&page)?;
                    let _ = tokio::task::spawn_blocking(move || {
                        crate::paths::atomic_write(&path, &data)
                    })
                    .await;
                }
                Ok(page)
            }
            Err(error) => {
                if let Some(path) = path
                    && let Ok(bytes) = tokio::fs::read(path).await
                    && let Ok(mut page) = serde_json::from_slice::<Page>(&bytes)
                {
                    page.title.push_str(" · cached");
                    page.notice =
                        Some("Radio service unavailable · showing the last saved catalog".into());
                    return Ok(page);
                }
                Err(error)
            }
        }
    }
    async fn json(&self, url: &str) -> Result<Value> {
        // One deadline includes redirects and the response body, not eight
        // seconds per hop. Dropping this future cancels losing directory probes.
        tokio::time::timeout(std::time::Duration::from_secs(8), self.json_inner(url))
            .await
            .context("Radio catalog request timed out; check your proxy or VPN")?
    }
    async fn json_inner(&self, url: &str) -> Result<Value> {
        let target = http_url(url)?;
        let host = target.host_str().unwrap_or("radio service").to_owned();
        // Public catalog requests have no browser cookies or authorization headers.
        let response = crate::http::get(
            target,
            |url| {
                Ok(self
                    .client
                    .get(url.clone())
                    .timeout(std::time::Duration::from_secs(8))
                    .header("User-Agent", "ytui/0.2 (terminal radio client)"))
            },
            crate::http::Redirects::Public,
            "Radio",
        )
        .await?;
        ensure!(
            response.status().is_success(),
            "{host} returned HTTP {}",
            response.status().as_u16()
        );
        serde_json::from_str(&limited_text(response, 8 * 1024 * 1024).await?)
            .with_context(|| format!("{host} returned an invalid radio catalog"))
    }
    async fn directory_servers(&self) -> Result<Vec<String>> {
        // Reuse discovery across queries, continuations and concurrent callers.
        // Explicit Refresh invalidates it; new proxy settings create a new Radio.
        self.discovery
            .load(
                self.discovery_endpoint.clone(),
                std::time::Duration::from_secs(300),
                || async {
                    let data = self.json(&self.discovery_endpoint).await.ok();
                    let mut servers = Vec::new();
                    if let Some(rows) = data.as_ref().and_then(Value::as_array) {
                        for name in rows
                            .iter()
                            .filter_map(|row| row.get("name").and_then(Value::as_str))
                        {
                            let valid = name.len() <= 253
                                && name.strip_suffix(".api.radio-browser.info").is_some_and(
                                    |prefix| {
                                        !prefix.is_empty()
                                            && prefix.split('.').all(|label| {
                                                !label.is_empty()
                                                    && label.len() <= 63
                                                    && !label.starts_with('-')
                                                    && !label.ends_with('-')
                                                    && label.bytes().all(|b| {
                                                        b.is_ascii_alphanumeric() || b == b'-'
                                                    })
                                            })
                                    },
                                );
                            if !valid {
                                continue;
                            }
                            let url = format!("https://{name}");
                            if !servers.contains(&url) {
                                servers.push(url);
                            }
                            if servers.len() == 4 {
                                break;
                            }
                        }
                    }
                    // Discovery can return stale mirrors. Keep independent known mirrors
                    // even when discovery itself succeeded, without duplicating requests.
                    for server in [
                        "https://de1.api.radio-browser.info",
                        "https://nl1.api.radio-browser.info",
                        "https://at1.api.radio-browser.info",
                    ] {
                        if !servers.iter().any(|url| url == server) {
                            servers.push(server.into());
                        }
                    }
                    Ok(servers)
                },
            )
            .await
    }
    async fn fetch_live(&self, request: &BrowseRequest, offset: usize) -> Result<Page> {
        match request {
            BrowseRequest::Cliamp => {
                let page = parse_cliamp(&self.json(&self.catalog).await?);
                ensure!(
                    !page.items.is_empty(),
                    "cliamp returned no supported stations"
                );
                Ok(page)
            }
            BrowseRequest::CliampTracks(url) => {
                let parsed = http_url(url)?;
                ensure!(
                    parsed.host_str() == Some("radio.cliamp.stream"),
                    "Unexpected cliamp track-library origin"
                );
                Ok(parse_cliamp_tracks(&self.json(url).await?))
            }
            BrowseRequest::RadioSearch(_)
            | BrowseRequest::RadioCountry(_)
            | BrowseRequest::RadioTag(_) => {
                let servers = if self.servers.is_empty() {
                    self.directory_servers().await?
                } else {
                    self.servers.clone()
                };
                let mut pending = FuturesUnordered::new();
                for server in servers {
                    let mut url = http_url(&server)?;
                    url.set_path("/json/stations/search");
                    url.query_pairs_mut()
                        .append_pair("limit", "80")
                        .append_pair("offset", &offset.to_string())
                        .append_pair("hidebroken", "true")
                        .append_pair("order", "clickcount")
                        .append_pair("reverse", "true");
                    match request {
                        BrowseRequest::RadioSearch(q) => {
                            url.query_pairs_mut().append_pair("name", q);
                        }
                        BrowseRequest::RadioCountry(q) => {
                            let q = q.trim();
                            if q.len() == 2 && q.bytes().all(|b| b.is_ascii_alphabetic()) {
                                url.query_pairs_mut()
                                    .append_pair("countrycode", &q.to_ascii_uppercase());
                            } else {
                                url.query_pairs_mut()
                                    .append_pair("country", q)
                                    .append_pair("countryExact", "true");
                            }
                        }
                        BrowseRequest::RadioTag(q) => {
                            url.query_pairs_mut().append_pair("tag", q);
                        }
                        _ => {}
                    }
                    pending.push(async move { self.json(url.as_str()).await });
                }
                let mut errors = Vec::new();
                let mut empty = None;
                while let Some(result) = pending.next().await {
                    match result {
                        Ok(data) if data.is_array() => {
                            let mut page = parse_radio_browser(&data);
                            if page.items.is_empty() {
                                if data.as_array().is_some_and(Vec::is_empty) {
                                    // A healthy mirror may have newer index data.
                                    empty = Some(page);
                                } else {
                                    errors.push(
                                        "Directory returned no supported station entries".into(),
                                    );
                                }
                                continue;
                            }
                            if data.as_array().is_some_and(|rows| rows.len() == 80) {
                                page.continuation = Some((offset + 80).to_string());
                            }
                            return Ok(page);
                        }
                        Ok(_) => errors.push("Unexpected directory response format".into()),
                        Err(error) => errors.push(error.to_string()),
                    }
                }
                if let Some(page) = empty {
                    return Ok(page);
                }
                bail!(
                    "Radio Browser is unavailable on all directory servers: {}",
                    errors.join("; ")
                )
            }
            _ => bail!("Unsupported radio feed"),
        }
    }
    pub async fn resolve_playlist(&self, url: &str) -> Result<String> {
        tokio::time::timeout(
            std::time::Duration::from_secs(12),
            self.resolve_playlist_inner(url),
        )
        .await
        .context("Station playlist request timed out; check your proxy or VPN")?
    }
    async fn resolve_playlist_inner(&self, url: &str) -> Result<String> {
        let mut url = http_url(url)?;
        let mut visited = std::collections::HashSet::new();
        for depth in 0..=4 {
            let path = url.path().to_ascii_lowercase();
            if !path.ends_with(".m3u") && !path.ends_with(".m3u8") && !path.ends_with(".pls") {
                return Ok(url.into());
            }
            ensure!(depth < 4, "Station playlist nesting exceeds four levels");
            ensure!(
                visited.insert(url.clone()),
                "Station playlist contains a cycle"
            );
            let response = crate::http::get(
                url.clone(),
                |url| Ok(self.client.get(url.clone())),
                crate::http::Redirects::Public,
                "Station playlist",
            )
            .await?;
            ensure!(
                response.status().is_success(),
                "Station playlist returned HTTP {}",
                response.status()
            );
            // A redirect can change the extension, directory and origin. Parse
            // the returned playlist and resolve entries relative to its final URL.
            let final_url = response.url().clone();
            let content_type = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_ascii_lowercase();
            if (content_type.starts_with("audio/") || content_type.starts_with("video/"))
                && !content_type.contains("mpegurl")
                && !content_type.contains("scpls")
            {
                // Some playlist links redirect straight to an endless audio
                // stream. Hand that to mpv without trying to read it as text.
                return Ok(final_url.into());
            }
            let data = limited_text(response, 128 * 1024).await?;
            if data.contains("#EXT-X-") {
                return Ok(final_url.into());
            }
            let pls = final_url.path().to_ascii_lowercase().ends_with(".pls")
                || data
                    .trim_start_matches('\u{feff}')
                    .trim_start()
                    .to_ascii_lowercase()
                    .starts_with("[playlist]");
            let target = data
                .trim_start_matches('\u{feff}')
                .lines()
                .map(str::trim)
                .find_map(|line| {
                    if line.starts_with('#') || line.is_empty() {
                        None
                    } else if !pls {
                        Some(line)
                    } else {
                        let (key, value) = line.split_once('=')?;
                        let key = key.trim().to_ascii_lowercase();
                        let index = key.strip_prefix("file")?;
                        (!index.is_empty() && index.bytes().all(|b| b.is_ascii_digit()))
                            .then_some(value.trim())
                    }
                })
                .context("Station playlist has no supported HTTP(S) stream")?;
            url = final_url.join(target)?;
            http_url(url.as_str())?;
        }
        unreachable!()
    }
}
/// Curated stock photography is a fallback; provider station logos always take precedence.
fn station_art(mut media: Media, provider: &Value) -> Media {
    let supplied = ["image", "artwork", "cover", "thumbnail", "logo", "favicon"]
        .into_iter()
        .filter_map(|key| provider.get(key).and_then(Value::as_str))
        .find(|url| http_url(url).is_ok());
    if let Some(url) = supplied {
        media.thumbnail = url.into();
        return media;
    }
    let genre = format!("{} {} {}", media.title, media.creator, media.description).to_lowercase();
    let photo = if ["ambient", "meditat", "chill", "nature", "sleep"]
        .iter()
        .any(|s| genre.contains(s))
    {
        "photo-1470770841072-f978cf4d019e"
    } else if ["jazz", "soul", "blues"].iter().any(|s| genre.contains(s)) {
        "photo-1415201364774-f6f0bb35f28f"
    } else if ["rock", "metal", "pop", "synth", "dance", "electro"]
        .iter()
        .any(|s| genre.contains(s))
    {
        "photo-1493225457124-a3eb161ffa5f"
    } else {
        "photo-1511379938547-c1f69419868d"
    };
    media.thumbnails = [128, 256, 512, 768]
        .into_iter()
        .map(|size| Thumbnail {
            url: format!(
                "https://images.unsplash.com/{photo}?w={size}&h={size}&fit=crop&auto=format"
            ),
            width: size,
            height: size,
        })
        .collect();
    media.thumbnail = media.thumbnails[2].url.clone();
    media
}
pub fn parse_cliamp(data: &Value) -> Page {
    let items = data
        .get("stations")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|s| {
            let url = s.get("stream")?.as_str()?;
            http_url(url).ok()?;
            Some(station_art(
                Media {
                    id: s.get("id").and_then(Value::as_str).unwrap_or(url).into(),
                    title: s
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("Station")
                        .into(),
                    creator: s
                        .get("genre")
                        .and_then(Value::as_str)
                        .unwrap_or("cliamp radio")
                        .into(),
                    url: url.into(),
                    kind: MediaKind::Station,
                    live: true,
                    tracks_url: s
                        .get("tracks_url")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    description: s
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .into(),
                    ..Default::default()
                },
                s,
            ))
        })
        .collect();
    Page {
        title: "cliamp radio".into(),
        items,
        ..Default::default()
    }
}
pub fn parse_cliamp_tracks(data: &Value) -> Page {
    let items = data
        .get("tracks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|t| {
            let url = t.get("url")?.as_str()?;
            http_url(url).ok()?;
            Some(Media {
                id: t.get("id").and_then(Value::as_str).unwrap_or(url).into(),
                title: t
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("Track")
                    .into(),
                creator: t
                    .get("artist")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .into(),
                url: url.into(),
                kind: MediaKind::Song,
                ..Default::default()
            })
        })
        .collect();
    Page {
        title: data
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("Tracks")
            .into(),
        items,
        ..Default::default()
    }
}
pub fn parse_radio_browser(data: &Value) -> Page {
    let items = data
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| {
            let url = s
                .get("url_resolved")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .or_else(|| s.get("url").and_then(Value::as_str))?;
            http_url(url).ok()?;
            Some(station_art(
                Media {
                    id: s
                        .get("stationuuid")
                        .and_then(Value::as_str)
                        .unwrap_or(url)
                        .into(),
                    title: s
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("Station")
                        .into(),
                    creator: s
                        .get("country")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .into(),
                    description: s
                        .get("tags")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .into(),
                    url: url.into(),
                    thumbnail: s
                        .get("favicon")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .into(),
                    kind: MediaKind::Station,
                    live: true,
                    ..Default::default()
                },
                s,
            ))
        })
        .collect();
    Page {
        title: "Radio Browser".into(),
        items,
        ..Default::default()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn station_art_prefers_provider_images_and_sizes_genre_photographs() {
        let source = json!({"stations":[{"stream":"https://radio.test/live","name":"Lofi"},
            {"stream":"https://radio.test/jazz","name":"Jazz","image":"https://radio.test/official.png"},
            {"stream":"https://radio.test/ambient","name":"Ambient","image":"file:///unsafe"}]});
        let page = parse_cliamp(&source);
        assert!(page.items[0].thumbnail.contains("photo-1511379938547"));
        assert_eq!(
            page.items[0]
                .thumbnails
                .iter()
                .map(|t| t.width)
                .collect::<Vec<_>>(),
            vec![128, 256, 512, 768]
        );
        assert_eq!(page.items[1].thumbnail, "https://radio.test/official.png");
        assert!(page.items[2].thumbnail.contains("photo-1470770841072"));
    }
    #[tokio::test]
    async fn wrapper_playlists_hls_and_directory_failover_are_bounded_and_scoped() {
        let (base, server)=crate::test_http::serve(|target| {
            let (status,headers,body)= match target.split('?').next().unwrap() {
                "/list.m3u" => (200,"","#EXTM3U\nrelative/live\n".to_owned()),
                "/wrap.pls" => (200,"","[playlist]\nFile1=../live\n".to_owned()),
                "/redirect.m3u" => (302,"Location: /list.m3u\r\n",String::new()),
                "/loop.m3u" => (302,"Location: /loop.m3u\r\n",String::new()),
                "/bad.m3u" => (200,"","file:///synthetic/unsafe\n".to_owned()),
                "/direct.m3u" => (302,"Location: /audio\r\n",String::new()),
                "/audio" => (200,"Content-Type: audio/mpeg\r\n","synthetic audio".into()),
                "/hls.m3u8" => (200,"","#EXTM3U\n#EXT-X-TARGETDURATION:10\nsegment.ts\n".to_owned()),
                "/json/stations/search" => {
                    let rows=(0..80).map(|n| json!({"stationuuid":n.to_string(),"name":"Fixture radio","url_resolved":"https://example.test/live"})).collect::<Vec<_>>();
                    (200,"Content-Type: application/json\r\n",serde_json::to_string(&rows).unwrap())
                },
                _ => (404,"",String::new()),
            };
            (status,headers.into(),body.into_bytes())
        }).await;
        let dead = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let unavailable = format!("http://{}", dead.local_addr().unwrap());
        drop(dead);
        let radio = Radio::new(
            reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
            vec![unavailable, base.clone()],
        );
        assert_eq!(
            radio
                .resolve_playlist(&format!("{base}/list.m3u"))
                .await
                .unwrap(),
            format!("{base}/relative/live")
        );
        assert_eq!(
            radio
                .resolve_playlist(&format!("{base}/wrap.pls"))
                .await
                .unwrap(),
            format!("{base}/live")
        );
        assert_eq!(
            radio
                .resolve_playlist(&format!("{base}/redirect.m3u"))
                .await
                .unwrap(),
            format!("{base}/relative/live")
        );
        assert!(
            radio
                .resolve_playlist(&format!("{base}/loop.m3u"))
                .await
                .is_err()
        );
        assert!(
            radio
                .resolve_playlist(&format!("{base}/bad.m3u"))
                .await
                .is_err()
        );
        assert_eq!(
            radio
                .resolve_playlist(&format!("{base}/hls.m3u8"))
                .await
                .unwrap(),
            format!("{base}/hls.m3u8")
        );
        assert_eq!(
            radio
                .resolve_playlist(&format!("{base}/direct.m3u"))
                .await
                .unwrap(),
            format!("{base}/audio")
        );
        for request in [
            BrowseRequest::RadioSearch("Jazz & blues".into()),
            BrowseRequest::RadioCountry("de".into()),
            BrowseRequest::RadioTag("ambient".into()),
        ] {
            let page = radio.fetch(&request, 80).await.unwrap();
            assert_eq!(page.items.len(), 80);
            assert_eq!(page.continuation.as_deref(), Some("160"));
        }
        server.abort();
        let _ = server.await;
    }
    #[tokio::test]
    async fn stalled_radio_bodies_have_whole_operation_deadlines() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let mut clients = tokio::task::JoinSet::new();
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().await.unwrap();
                clients.spawn(async move {
                    let mut request = Vec::new();
                    while !request.ends_with(b"\r\n\r\n") {
                        request.push(stream.read_u8().await.unwrap());
                        assert!(request.len() <= 4096);
                    }
                    stream
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\n")
                        .await
                        .unwrap();
                    std::future::pending::<()>().await;
                    drop(stream);
                });
            }
            while clients.join_next().await.is_some() {}
        });
        let radio = Radio::new(
            reqwest::Client::builder().no_proxy().build().unwrap(),
            vec![],
        );
        let playlist = format!("{base}/stalled.m3u");
        let result = tokio::time::timeout(std::time::Duration::from_secs(14), async {
            tokio::join!(radio.json(&base), radio.resolve_playlist(&playlist))
        })
        .await;
        task.abort();
        let (catalog, stream) = result.expect("Service deadlines must beat the test deadline");
        assert!(catalog.unwrap_err().to_string().contains("timed out"));
        assert!(stream.unwrap_err().to_string().contains("timed out"));
    }

    #[tokio::test]
    async fn redirected_extensionless_playlists_resolve_against_the_final_location() {
        let (base, task) = crate::test_http::serve(|target| match target {
            "/start.pls" => (
                302,
                "Location: /moved/catalog?token=fixture\r\n".into(),
                vec![],
            ),
            "/moved/catalog?token=fixture" => {
                (200, String::new(), b"[playlist]\nFile1=stream\n".to_vec())
            }
            _ => (404, String::new(), vec![]),
        })
        .await;
        let radio = Radio::new(
            reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
            vec![],
        );
        let result = radio.resolve_playlist(&format!("{base}/start.pls")).await;
        task.abort();
        assert_eq!(result.unwrap(), format!("{base}/moved/stream"));
    }

    #[tokio::test]
    async fn fourth_nested_playlist_can_point_to_a_stream() {
        let (base, task) = crate::test_http::serve(|target| {
            let next = match target {
                "/1.m3u" => "2.m3u",
                "/2.m3u" => "3.m3u",
                "/3.m3u" => "4.m3u",
                "/4.m3u" => "live",
                _ => panic!("Must not download the live stream"),
            };
            (200, String::new(), next.as_bytes().to_vec())
        })
        .await;
        let radio = Radio::new(
            reqwest::Client::builder().no_proxy().build().unwrap(),
            vec![],
        );
        let result = radio.resolve_playlist(&format!("{base}/1.m3u")).await;
        task.abort();
        assert_eq!(result.unwrap(), format!("{base}/live"));
    }

    #[test]
    fn live_and_library_channels_are_distinct() {
        let p = parse_cliamp(
            &json!({"stations":[{"id":"one","name":"One","stream":"https://radio.test/live","tracks_url":"https://radio.cliamp.stream/one/tracks"},{"name":"Live","stream":"https://radio.test/only-live"},{"name":"Unsafe","stream":"file:///etc/passwd"}]}),
        );
        assert_eq!(p.items.len(), 2);
        assert!(p.items[0].tracks_url.is_some());
        assert!(p.items[1].tracks_url.is_none());
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;
    use sha2::{Digest, Sha256};
    #[tokio::test]
    async fn directory_outage_uses_a_saved_catalog_with_a_notice() {
        let t = tempfile::tempdir().unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let request = BrowseRequest::RadioSearch("fixture".into());
        let page = Page {
            title: "Radio Browser".into(),
            items: vec![Media {
                id: "one".into(),
                title: "Saved station".into(),
                url: "https://station.test/live".into(),
                kind: MediaKind::Station,
                live: true,
                ..Default::default()
            }],
            ..Default::default()
        };
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let radio = Radio::new(client, vec![format!("http://127.0.0.1:{port}")])
            .with_cache(t.path().into());
        let key = hex::encode(Sha256::digest(format!(
            "v2:{}",
            radio.page_key(&request, 0).unwrap()
        )));
        crate::paths::atomic_write(&t.path().join(key), &serde_json::to_vec(&page).unwrap())
            .unwrap();
        let restored = radio.fetch(&request, 0).await.unwrap();
        assert_eq!(restored.items.len(), 1);
        assert!(restored.title.contains("cached"));
        assert!(restored.notice.unwrap().contains("unavailable"));
    }
}

#[cfg(test)]
mod outage_tests {
    use super::*;
    use serde_json::json;
    #[tokio::test]
    async fn discovery_is_shared_cached_refreshable_and_keeps_fallback_mirrors() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let (endpoint, task) = crate::test_http::serve(move |_| {
            count.fetch_add(1, Ordering::SeqCst);
            (
                200,
                String::new(),
                json!([
                    {"name":"stale.api.radio-browser.info"},
                    {"name":"stale.api.radio-browser.info"},
                    {"name":"user@evil.api.radio-browser.info"},
                    {"name":"bad:443.api.radio-browser.info"},
                    {"name":"-bad.api.radio-browser.info"},
                    {"name":"evil.test"}
                ])
                .to_string()
                .into_bytes(),
            )
        })
        .await;
        let mut radio = Radio::new(
            reqwest::Client::builder().no_proxy().build().unwrap(),
            vec![],
        );
        radio.discovery_endpoint = endpoint;
        let clone = radio.clone();
        let (a, b) = tokio::join!(radio.directory_servers(), clone.directory_servers());
        let servers = a.unwrap();
        assert_eq!(servers, b.unwrap());
        assert_eq!(
            servers,
            [
                "https://stale.api.radio-browser.info",
                "https://de1.api.radio-browser.info",
                "https://nl1.api.radio-browser.info",
                "https://at1.api.radio-browser.info"
            ]
        );
        radio.directory_servers().await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        radio.invalidate_page(&BrowseRequest::RadioSearch("fixture".into()), 0);
        radio.directory_servers().await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        task.abort();
    }

    #[tokio::test]
    async fn empty_directory_results_are_valid_but_malformed_rows_are_errors() {
        for (body, valid) in [
            ("[]", true),
            (
                r#"[{"name":"Unsupported","url_resolved":"file:///invalid"}]"#,
                false,
            ),
        ] {
            let (base, task) =
                crate::test_http::serve(move |_| (200, String::new(), body.as_bytes().to_vec()))
                    .await;
            let radio = Radio::new(
                reqwest::Client::builder().no_proxy().build().unwrap(),
                vec![base],
            );
            let result = radio
                .fetch(&BrowseRequest::RadioSearch("fixture".into()), 0)
                .await;
            task.abort();
            if valid {
                assert!(result.unwrap().items.is_empty());
            } else {
                assert!(
                    result
                        .unwrap_err()
                        .to_string()
                        .contains("no supported station entries")
                );
            }
        }
    }

    #[tokio::test]
    async fn empty_or_malformed_fast_mirror_cannot_hide_a_healthy_mirror() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for fast_body in [
            "[]",
            r#"[{"name":"broken","url_resolved":"file:///invalid"}]"#,
        ] {
            let (fast, fast_task) = crate::test_http::serve(move |_| {
                (200, String::new(), fast_body.as_bytes().to_vec())
            })
            .await;
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let slow = format!("http://{}", listener.local_addr().unwrap());
            let slow_task = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    request.push(stream.read_u8().await.unwrap());
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                let body =
                    r#"[{"name":"Healthy station","url_resolved":"https://station.test/live"}]"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
            });
            let radio = Radio::new(
                reqwest::Client::builder().no_proxy().build().unwrap(),
                vec![fast, slow],
            );
            let result = radio
                .fetch(&BrowseRequest::RadioSearch("fixture".into()), 0)
                .await;
            fast_task.abort();
            slow_task.abort();
            let page = result.unwrap();
            assert_eq!(
                page.items.len(),
                1,
                "Fast mirror suppressed the healthy station"
            );
            assert_eq!(page.items[0].title, "Healthy station");
        }
    }

    #[tokio::test]
    async fn disk_fallback_is_scoped_to_the_configured_directory() {
        let cache = tempfile::tempdir().unwrap();
        let (base, server) = crate::test_http::serve(|_| {
            (
                200,
                String::new(),
                br#"[{"name":"Old directory","url_resolved":"https://station.test/live"}]"#
                    .to_vec(),
            )
        })
        .await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let radio = Radio::new(client.clone(), vec![base]).with_cache(cache.path().into());
        let request = BrowseRequest::RadioSearch("fixture".into());
        radio.fetch(&request, 0).await.unwrap();
        let unavailable = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", unavailable.local_addr().unwrap());
        drop(unavailable);
        let changed = Radio::new(client, vec![address]).with_cache(cache.path().into());
        let result = changed.fetch(&request, 0).await;
        server.abort();
        assert!(
            result.is_err(),
            "Different directory silently reused old stations"
        );
    }

    #[tokio::test]
    async fn country_names_and_codes_use_the_matching_directory_filter() {
        let (base, task) = crate::test_http::serve_request(|request| {
            let target = request
                .lines()
                .next()
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap();
            let url = url::Url::parse(&format!("http://fixture{target}")).unwrap();
            let pairs = url
                .query_pairs()
                .collect::<std::collections::HashMap<_, _>>();
            if pairs.get("countrycode").is_some_and(|v| v == "DE")
                || pairs.get("country").is_some_and(|v| v == "Germany")
            {
                (
                    200,
                    String::new(),
                    json!([{"name":"German station","url_resolved":"https://station.test/live"}])
                        .to_string()
                        .into_bytes(),
                )
            } else {
                (200, String::new(), b"[]".to_vec())
            }
        })
        .await;
        let radio = Radio::new(
            reqwest::Client::builder().no_proxy().build().unwrap(),
            vec![base],
        );
        for country in ["de", "Germany"] {
            let page = radio
                .fetch(&BrowseRequest::RadioCountry(country.into()), 0)
                .await
                .unwrap();
            assert_eq!(page.items.len(), 1, "country {country} failed");
        }
        task.abort();
    }
    #[tokio::test]
    async fn catalog_redirects_and_outages_use_a_labelled_directory_fallback() {
        let (base, task) = crate::test_http::serve(|path| {
            let (status, headers, body) = match path.split('?').next().unwrap() {
                "/catalog" => (302, "Location: /catalog-ready\r\n", json!({})),
                "/catalog-ready" => (200, "", json!({"stations":[{"name":"cliamp fixture", "stream":"https://example.test/live"}]})),
                "/outage" => (503, "", json!({})),
                "/empty" => (200, "", json!({"stations":[]})),
                "/loop" => (302, "Location: /loop\r\n", json!({})),
                "/json/stations/search" => (200, "", json!([{"name":"Directory fixture", "url_resolved":"https://example.test/live"}])),
                _ => (404, "", json!({})),
            };
            (status, headers.into(), body.to_string().into_bytes())
        }).await;
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let mut radio = Radio::new(client, vec![base.clone()]);
        radio.catalog = format!("{base}/catalog");
        let catalog = radio.fetch(&BrowseRequest::Cliamp, 0).await.unwrap();
        assert_eq!(catalog.items[0].title, "cliamp fixture");
        assert!(catalog.notice.is_none());
        for (path, reason) in [
            ("outage", "503"),
            ("empty", "no supported stations"),
            ("loop", "redirect limit"),
        ] {
            radio.catalog = format!("{base}/{path}");
            let page = radio.fetch(&BrowseRequest::Cliamp, 0).await.unwrap();
            assert_eq!(page.items[0].title, "Directory fixture");
            let notice = page.notice.unwrap();
            assert!(notice.contains("Radio Browser"));
            assert!(notice.contains(reason), "{notice}");
        }
        let cache = tempfile::tempdir().unwrap();
        let cached = radio.with_cache(cache.path().into());
        assert_eq!(
            cached.fetch(&BrowseRequest::Cliamp, 0).await.unwrap().items[0].title,
            "Directory fixture"
        );
        task.abort();
        let _ = task.await;
        cached.invalidate_page(&BrowseRequest::Cliamp, 0);
        let restored = cached.fetch(&BrowseRequest::Cliamp, 0).await.unwrap();
        assert_eq!(restored.items[0].title, "Directory fixture");
        assert!(restored.title.contains("cached"));
    }
    #[tokio::test]
    async fn network_errors_retain_the_host_and_cause_without_private_url_queries() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "http://{}/?token=synthetic-secret",
            listener.local_addr().unwrap()
        );
        drop(listener);
        let radio = Radio::new(
            reqwest::Client::builder().no_proxy().build().unwrap(),
            vec![],
        );
        let error = radio.json(&url).await.unwrap_err().to_string();
        assert!(error.contains("127.0.0.1"));
        assert!(error.contains("connect"), "{error}");
        assert!(!error.contains("synthetic-secret"));
    }
}

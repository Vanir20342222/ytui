use crate::browse::limited_text;
use anyhow::{Context, Result, bail, ensure};
use futures_util::{StreamExt, stream::FuturesUnordered};
use serde_json::Value;
use ytui_core::{BrowseRequest, Media, MediaKind, Page, http_url};

#[derive(Clone)]
pub struct Radio {
    client: reqwest::Client,
    pub servers: Vec<String>,
    cache: Option<std::path::PathBuf>,
    catalog: String,
}
impl Radio {
    pub fn new(client: reqwest::Client, servers: Vec<String>) -> Self {
        Self {
            client,
            servers,
            cache: None,
            catalog: "https://radio.cliamp.stream/stations".into(),
        }
    }
    pub fn with_cache(mut self, path: std::path::PathBuf) -> Self {
        self.cache = Some(path);
        self
    }
    pub async fn fetch(&self, request: &BrowseRequest, offset: usize) -> Result<Page> {
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
        let key = hex::encode(Sha256::digest(format!(
            "{}:{offset}",
            serde_json::to_string(request)?
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
                    let mut discovered = self
                        .json("https://all.api.radio-browser.info/json/servers")
                        .await
                        .ok()
                        .and_then(|v| v.as_array().cloned())
                        .unwrap_or_default()
                        .into_iter()
                        .filter_map(|v| v.get("name").and_then(Value::as_str).map(str::to_owned))
                        .filter(|s| s.ends_with(".api.radio-browser.info") && !s.contains('/'))
                        .take(4)
                        .map(|s| format!("https://{s}"))
                        .collect::<Vec<_>>();
                    if discovered.is_empty() {
                        discovered = vec![
                            "https://de1.api.radio-browser.info".into(),
                            "https://nl1.api.radio-browser.info".into(),
                            "https://at1.api.radio-browser.info".into(),
                        ];
                    }
                    discovered
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
                            url.query_pairs_mut()
                                .append_pair("countrycode", &q.to_uppercase());
                        }
                        BrowseRequest::RadioTag(q) => {
                            url.query_pairs_mut().append_pair("tag", q);
                        }
                        _ => {}
                    }
                    pending.push(async move { self.json(url.as_str()).await });
                }
                let mut errors = Vec::new();
                while let Some(result) = pending.next().await {
                    match result {
                        Ok(data) if data.is_array() => {
                            let mut page = parse_radio_browser(&data);
                            if data.as_array().is_some_and(|rows| rows.len() == 80) {
                                page.continuation = Some((offset + 80).to_string());
                            }
                            return Ok(page);
                        }
                        Ok(_) => errors.push("Unexpected directory response format".into()),
                        Err(error) => errors.push(error.to_string()),
                    }
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
        let mut url = http_url(url)?;
        for _ in 0..4 {
            let path = url.path().to_ascii_lowercase();
            if !path.ends_with(".m3u") && !path.ends_with(".m3u8") && !path.ends_with(".pls") {
                return Ok(url.into());
            }
            // HLS manifests belong to mpv; only resolve ordinary wrapper playlists.
            let response = self.client.get(url.clone()).send().await?;
            if response.status().is_redirection() {
                let location = response
                    .headers()
                    .get("location")
                    .and_then(|v| v.to_str().ok())
                    .context("Radio redirect has no destination")?;
                url = url.join(location)?;
                http_url(url.as_str())?;
                continue;
            }
            ensure!(
                response.status().is_success(),
                "Station playlist returned HTTP {}",
                response.status()
            );
            let data = limited_text(response, 128 * 1024).await?;
            if data.contains("#EXT-X-") {
                return Ok(url.into());
            }
            let target = data
                .lines()
                .map(str::trim)
                .find_map(|line| {
                    if line.starts_with('#') || line.is_empty() {
                        None
                    } else if !path.ends_with(".pls") {
                        Some(line)
                    } else if let Some((key, value)) = line.split_once('=') {
                        if key.to_ascii_lowercase().starts_with("file") {
                            Some(value)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                })
                .context("Station playlist has no supported HTTP(S) stream")?;
            url = url.join(target)?;
            http_url(url.as_str())?;
        }
        bail!("Station playlist/redirect nesting exceeds four levels")
    }
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
            Some(Media {
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
            })
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
            Some(Media {
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
            })
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
    #[tokio::test]
    async fn wrapper_playlists_hls_and_directory_failover_are_bounded_and_scoped() {
        let (base, server)=crate::test_http::serve(|target| {
            let (status,headers,body)= match target.split('?').next().unwrap() {
                "/list.m3u" => (200,"","#EXTM3U\nrelative/live\n".to_owned()),
                "/wrap.pls" => (200,"","[playlist]\nFile1=../live\n".to_owned()),
                "/redirect.m3u" => (302,"Location: /list.m3u\r\n",String::new()),
                "/loop.m3u" => (302,"Location: /loop.m3u\r\n",String::new()),
                "/bad.m3u" => (200,"","file:///synthetic/unsafe\n".to_owned()),
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
        let key = hex::encode(Sha256::digest(format!(
            "{}:0",
            serde_json::to_string(&request).unwrap()
        )));
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
        crate::paths::atomic_write(&t.path().join(key), &serde_json::to_vec(&page).unwrap())
            .unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let radio = Radio::new(client, vec![format!("http://127.0.0.1:{port}")])
            .with_cache(t.path().into());
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

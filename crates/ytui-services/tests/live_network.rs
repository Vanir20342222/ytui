//! Opt-in public network acceptance. No account, browser store or system setting is used.
use std::time::{Duration, Instant};
use ytui_core::{BrowseRequest, Config, MediaKind};
use ytui_services::{
    browse::Browser,
    images::ImageCache,
    paths::Paths,
    player::{Player, PlayerCommand},
    radio::Radio,
};

#[tokio::test]
#[ignore = "public YouTube search, pagination, details and CDN thumbnails; no cookies"]
async fn live_youtube_pages_and_thumbnail_cache() {
    let client = ytui_services::http_client(None).unwrap();
    let browser = Browser::new(client.clone(), None, 0);
    let request = BrowseRequest::Search("Rust programming".into());
    let start = Instant::now();
    let page = browser.fetch(false, &request).await.unwrap();
    let cold = start.elapsed();
    assert!(!page.items.is_empty());
    let start = Instant::now();
    for _ in 0..100 {
        assert_eq!(
            browser.fetch(false, &request).await.unwrap().items[0].id,
            page.items[0].id
        );
    }
    println!(
        "YouTube search: cold {:?}; 100 memory hits {:?}",
        cold,
        start.elapsed()
    );
    let token = page.continuation.expect("public search continuation");
    assert!(
        !browser
            .fetch(false, &BrowseRequest::Continue(token))
            .await
            .unwrap()
            .items
            .is_empty()
    );
    let details = browser
        .fetch(false, &BrowseRequest::Details("jNQXAC9IVRw".into()))
        .await
        .unwrap();
    assert_eq!(details.items[0].id, "jNQXAC9IVRw");
    let temp = tempfile::tempdir().unwrap();
    let images = ImageCache::new(client.clone(), temp.path().into());
    let url = &page
        .items
        .iter()
        .find(|m| !m.thumbnail.is_empty())
        .expect("search thumbnail")
        .thumbnail;
    let first = images.load_sized(url, 320, 180).await.unwrap();
    let start = Instant::now();
    images.load_sized(url, 320, 180).await.unwrap();
    println!("Thumbnail memory hit {:?}", start.elapsed());
    let restored = ImageCache::new(client, temp.path().into())
        .load_sized(url, 320, 180)
        .await
        .unwrap();
    assert_eq!(
        (first.width(), first.height()),
        (restored.width(), restored.height())
    );
}

#[tokio::test]
#[ignore = "public cliamp catalog and track library; no cookies"]
async fn live_cliamp_catalog_and_track_library() {
    let radio = Radio::new(ytui_services::http_client(None).unwrap(), vec![]);
    let page = radio.fetch(&BrowseRequest::Cliamp, 0).await.unwrap();
    assert!(
        page.notice.is_none(),
        "cliamp fallback is not cliamp acceptance"
    );
    assert!(!page.items.is_empty());
    let library = page
        .items
        .iter()
        .find_map(|item| item.tracks_url.clone())
        .expect("cliamp track library");
    assert!(
        !radio
            .fetch(&BrowseRequest::CliampTracks(library), 0)
            .await
            .unwrap()
            .items
            .is_empty()
    );
    println!(
        "cliamp: {} entries and a nonempty track library",
        page.items.len()
    );
}

#[tokio::test]
#[ignore = "public Radio Browser discovery/search/country/tag/pagination; no cookies"]
async fn live_radio_directory_filters_and_cache() {
    let radio = Radio::new(ytui_services::http_client(None).unwrap(), vec![]);
    for request in [
        BrowseRequest::RadioSearch("jazz".into()),
        BrowseRequest::RadioCountry("Germany".into()),
        BrowseRequest::RadioCountry("de".into()),
        BrowseRequest::RadioTag("classical".into()),
    ] {
        let start = Instant::now();
        let page = radio.fetch(&request, 0).await.unwrap();
        let cold = start.elapsed();
        assert!(!page.items.is_empty(), "empty {request:?}");
        let start = Instant::now();
        assert_eq!(
            radio.fetch(&request, 0).await.unwrap().items[0].url,
            page.items[0].url
        );
        println!(
            "{request:?}: {} entries; cold {:?}, memory hit {:?}",
            page.items.len(),
            cold,
            start.elapsed()
        );
        if page.continuation.is_some() {
            assert!(!radio.fetch(&request, 80).await.unwrap().items.is_empty());
        }
    }
}

#[tokio::test]
#[ignore = "actual public cliamp station and mpv playback; null audio/video outputs"]
async fn live_radio_stream_advances_in_mpv() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let mpv = ytui_services::paths::executable("mpv").expect("Install mpv");
    let wrapper = root.join("mpv-headless");
    std::fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nexec '{}' --vo=null --ao=null \"$@\"\n",
            mpv.display().to_string().replace('\'', "'\\''")
        ),
    )
    .unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
    let radio = Radio::new(ytui_services::http_client(None).unwrap(), vec![]);
    let catalog = radio.fetch(&BrowseRequest::Cliamp, 0).await.unwrap();
    let library = catalog
        .items
        .iter()
        .find_map(|m| m.tracks_url.clone())
        .expect("track library");
    let track = radio
        .fetch(&BrowseRequest::CliampTracks(library), 0)
        .await
        .unwrap()
        .items
        .into_iter()
        .next()
        .expect("library track");
    let station = catalog
        .items
        .into_iter()
        .find(|m| m.kind == MediaKind::Station && m.live)
        .expect("live cliamp station");
    let paths = Paths {
        config_override: None,
        xdg_state: root.join("xdg"),
        config: root.join("config"),
        state: root.join("state"),
        cache: root.join("cache"),
        legacy_data: root.join("legacy"),
        home: root.into(),
    };
    paths.create().unwrap();
    for (kind, media) in [("station", station), ("library track", track)] {
        let mut player = Player::spawn(
            Config {
                mpv: wrapper.display().to_string(),
                playback_sponsorblock: "off".into(),
                ..Default::default()
            },
            paths.clone(),
            None,
        );
        player
            .commands
            .send(PlayerCommand::Play { media, audio: true })
            .await
            .unwrap();
        let result = tokio::time::timeout(Duration::from_secs(45), async {
            loop {
                let state = player.status.borrow_and_update().clone();
                if let Some(error) = state.error {
                    return Err(error);
                }
                if state.position >= 3.0 && !state.buffering {
                    return Ok(state.position);
                }
                if player.status.changed().await.is_err() {
                    return Err("player stopped".into());
                }
            }
        })
        .await;
        player.close().await;
        let position = result.expect("45-second radio startup deadline").unwrap();
        println!("Actual cliamp {kind} playback advanced {position:.3} seconds");
    }
}

#[tokio::test]
#[ignore = "public YouTube thumbnail CDN independently of browsing; no cookies"]
async fn live_public_thumbnail_cdn_and_disk_cache() {
    let temp = tempfile::tempdir().unwrap();
    let client = ytui_services::http_client(None).unwrap();
    let cache = ImageCache::new(client.clone(), temp.path().into());
    let url = "https://i.ytimg.com/vi/jNQXAC9IVRw/mqdefault.jpg";
    let image = cache.load_sized(url, 160, 90).await.unwrap();
    assert!(image.width() > 0 && image.width() <= 160 && image.height() <= 90);
    let restored = ImageCache::new(client, temp.path().into())
        .load_sized(url, 160, 90)
        .await
        .unwrap();
    assert_eq!(restored.as_bytes(), image.as_bytes());
}

#[tokio::test]
#[ignore = "public LRCLIB lookup for a fixed public song; no account or playback state"]
async fn live_public_lyrics_lookup() {
    let tracks = ytui_services::lyrics::lookup(None, "Never Gonna Give You Up", "Rick Astley")
        .await
        .unwrap();
    assert!(!tracks.is_empty(), "No public song candidates returned");
}

#[tokio::test]
#[ignore = "public SponsorBlock API for a fixed public video; no account"]
async fn live_public_sponsorblock_api() {
    let media = ytui_core::Media {
        id: "dQw4w9WgXcQ".into(),
        url: "https://www.youtube.com/watch?v=dQw4w9WgXcQ".into(),
        ..Default::default()
    };
    let segments = ytui_services::sponsorblock::fetch(&Config::default(), &media)
        .await
        .unwrap();
    // A real 404 is a supported empty result, not a claim that segments exist.
    assert!(
        segments
            .iter()
            .all(|s| s.start.is_finite() && s.end > s.start)
    );
}

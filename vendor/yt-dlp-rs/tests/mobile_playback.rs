//! Offline request-count and fallback checks for the opt-in mobile player path.
#[path = "helpers/site_proxy.rs"]
mod site_proxy;
use yt_dlp_rs::{HttpOptions, YoutubeDl, YoutubeOptions};

fn downloader(proxy: &site_proxy::SiteProxy, format: Option<&str>) -> YoutubeDl {
    YoutubeDl::with_options(
        YoutubeOptions {
            no_playlist: true,
            playback_format: format.map(str::to_owned),
            playback_visitor_data: Some("synthetic-visitor".into()),
            ..Default::default()
        },
        HttpOptions {
            proxy: Some(proxy.url.clone()),
            no_check_certificates: true,
            socket_timeout: Some(2.0),
            ..Default::default()
        },
    )
    .unwrap()
}
const VIDEO: &str = "https://www.youtube.com/watch?v=BaW_jenozKc";
const FORMAT: &str = "bestvideo[height<=1080]+bestaudio/best";

#[test]
fn mobile_paired_video_uses_one_player_request_and_keeps_requested_quality() {
    let proxy = site_proxy::SiteProxy::start("youtube-direct");
    let info = downloader(&proxy, Some(FORMAT))
        .extract_info_with_format(VIDEO, Some(FORMAT))
        .unwrap();
    assert_eq!(info.height, Some(1080));
    assert_eq!(info.extra["ytui_playback_route"], "direct");
    assert_eq!(info.requested_formats.len(), 2);
    let requests = proxy.requests.lock().unwrap();
    assert_eq!(requests.len(), 1, "requests: {requests:?}");
    assert!(requests[0].contains("/youtubei/v1/player"));
}

#[test]
fn mobile_audio_selects_audio_without_fetching_watch_page_or_player_js() {
    let proxy = site_proxy::SiteProxy::start("youtube-direct");
    let info = downloader(&proxy, Some("bestaudio/best"))
        .extract_info_with_format(VIDEO, Some("bestaudio/best"))
        .unwrap();
    assert_eq!(info.vcodec.as_deref(), Some("none"));
    assert_eq!(proxy.requests.lock().unwrap().len(), 1);
}

#[test]
fn restricted_direct_response_falls_back_to_watch_acquisition() {
    let proxy = site_proxy::SiteProxy::start("youtube-direct-fallback");
    let info = downloader(&proxy, Some(FORMAT))
        .extract_info_with_format(VIDEO, Some(FORMAT))
        .unwrap();
    assert_eq!(info.height, Some(1080));
    assert_eq!(info.extra["ytui_playback_route"], "full");
    let requests = proxy.requests.lock().unwrap();
    assert_eq!(requests.len(), 3, "requests: {requests:?}");
    assert!(requests[0].contains("/youtubei/v1/player"));
    assert!(requests[1].contains("/watch?"));
}

#[test]
fn direct_rate_limit_does_not_hammer_other_clients_or_watch_page() {
    let proxy = site_proxy::SiteProxy::start("youtube-direct-limited");
    let error = downloader(&proxy, Some(FORMAT))
        .extract_info_with_format(VIDEO, Some(FORMAT))
        .unwrap_err();
    assert!(error.to_string().contains("429"), "{error}");
    assert_eq!(proxy.requests.lock().unwrap().len(), 1);
}

#[test]
fn default_download_acquisition_still_begins_with_watch_page() {
    let proxy = site_proxy::SiteProxy::start("youtube-direct");
    let _ = downloader(&proxy, None).extract_info_with_format(VIDEO, Some(FORMAT));
    let requests = proxy.requests.lock().unwrap();
    assert!(requests[0].contains("/watch?"), "requests: {requests:?}");
}

#[test]
fn missing_browse_context_bootstraps_once_and_reuses_visitor_without_watch_requests() {
    let proxy = site_proxy::SiteProxy::start("youtube-direct");
    let dl = YoutubeDl::with_options(
        YoutubeOptions {
            no_playlist: true,
            playback_format: Some(FORMAT.into()),
            ..Default::default()
        },
        HttpOptions {
            proxy: Some(proxy.url.clone()),
            no_check_certificates: true,
            socket_timeout: Some(2.0),
            ..Default::default()
        },
    )
    .unwrap();
    for _ in 0..2 {
        dl.extract_info_with_format(VIDEO, Some(FORMAT)).unwrap();
    }
    let requests = proxy.requests.lock().unwrap();
    assert_eq!(requests.len(), 3, "requests: {requests:?}");
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.contains("/visitor_id?"))
            .count(),
        1
    );
    assert!(requests.iter().all(|request| !request.contains("/watch?")));
}

#[test]
fn full_fallback_preserves_rate_limit_instead_of_querying_more_clients() {
    let proxy = site_proxy::SiteProxy::start("youtube-direct-fallback-limited");
    let dl = YoutubeDl::with_options(
        YoutubeOptions {
            no_playlist: true,
            playback_format: Some(FORMAT.into()),
            playback_visitor_data: Some("synthetic-visitor".into()),
            js_runtime: Some("nonexistent-runtime-never-needed".into()),
            ..Default::default()
        },
        HttpOptions {
            proxy: Some(proxy.url.clone()),
            no_check_certificates: true,
            socket_timeout: Some(2.0),
            ..Default::default()
        },
    )
    .unwrap();
    let error = dl
        .extract_info_with_format(VIDEO, Some(FORMAT))
        .unwrap_err();
    assert!(error.to_string().contains("429"), "{error}");
    let requests = proxy.requests.lock().unwrap();
    assert_eq!(requests.len(), 3, "requests: {requests:?}");
}

#[test]
fn direct_challenges_do_not_silently_downgrade_offered_1080p_to_a_muxed_360p_fallback() {
    let proxy = site_proxy::SiteProxy::start("youtube-direct-quality-fallback");
    let info = downloader(&proxy, Some(FORMAT))
        .extract_info_with_format(VIDEO, Some(FORMAT))
        .unwrap();
    assert_eq!(info.height, Some(1080));
    assert_eq!(info.extra["ytui_playback_route"], "full");
    assert_eq!(proxy.requests.lock().unwrap().len(), 3);
}

#[test]
fn usable_direct_streams_do_not_fetch_optional_manifests_for_video_or_audio() {
    for selector in [FORMAT, "bestaudio/best"] {
        let proxy = site_proxy::SiteProxy::start("youtube-direct-optional-manifests");
        let info = downloader(&proxy, Some(selector))
            .extract_info_with_format(VIDEO, Some(selector))
            .unwrap();
        assert_eq!(info.extra["ytui_playback_route"], "direct");
        if selector == FORMAT {
            assert_eq!(info.height, Some(1080));
        } else {
            assert_eq!(info.vcodec.as_deref(), Some("none"));
        }
        let requests = proxy.requests.lock().unwrap();
        assert_eq!(
            requests.len(),
            1,
            "Optional manifests must not be on the first-frame path: {requests:?}"
        );
    }
}

#[test]
fn manifest_only_direct_response_keeps_necessary_hls_without_watch_refetch() {
    let proxy = site_proxy::SiteProxy::start("youtube-direct-hls-only");
    let info = downloader(&proxy, Some(FORMAT))
        .extract_info_with_format(VIDEO, Some(FORMAT))
        .unwrap_or_else(|error| panic!("{error}; requests={:?}", proxy.requests.lock().unwrap()));
    assert_eq!(info.extra["ytui_playback_route"], "direct");
    assert_eq!(info.height, Some(720));
    let requests = proxy.requests.lock().unwrap();
    assert_eq!(
        requests.len(),
        2,
        "Only the player API and master: {requests:?}"
    );
    assert!(requests.iter().all(|request| !request.contains("/watch?")));
    assert!(requests[1].contains("manifest.test/hls/master.m3u8"));
}

#[test]
fn optional_manifest_is_required_when_it_can_improve_a_lower_direct_resolution() {
    let proxy = site_proxy::SiteProxy::start("youtube-direct-hls-quality");
    let info = downloader(&proxy, Some(FORMAT))
        .extract_info_with_format(VIDEO, Some(FORMAT))
        .unwrap();
    assert_eq!(info.extra["ytui_playback_route"], "direct");
    assert_eq!(
        info.height,
        Some(720),
        "Do not start the direct 360p fallback while HLS offers 720p"
    );
    let requests = proxy.requests.lock().unwrap();
    assert_eq!(
        requests.len(),
        2,
        "Child playlists belong to Media3: {requests:?}"
    );
    assert!(requests[1].contains("manifest.test/hls/master.m3u8"));
    assert!(info.fragments.is_empty());
}

#[test]
fn best_quality_uses_master_catalog_without_traversing_every_variant() {
    let proxy = site_proxy::SiteProxy::start("youtube-direct-hls-quality");
    let selector = "bestvideo+bestaudio/best";
    let info = downloader(&proxy, Some(selector))
        .extract_info_with_format(VIDEO, Some(selector))
        .unwrap();
    assert_eq!(info.height, Some(720));
    assert_eq!(info.extra["ytui_playback_route"], "direct");
    assert!(info.extra["ytui_manifest_ms"].is_u64());
    assert_eq!(info.vcodec.as_deref(), Some("avc1.64001f"));
    assert_eq!(info.acodec.as_deref(), Some("mp4a.40.2"));
    assert_eq!(
        info.extra["manifest_url"],
        "https://manifest.test/hls/master.m3u8"
    );
    assert_eq!(proxy.requests.lock().unwrap().len(), 2);
    assert!(
        info.formats
            .iter()
            .all(|format| format.fragments.is_empty())
    );
}

#[test]
fn playback_keeps_external_audio_groups_without_reading_child_playlists() {
    let proxy = site_proxy::SiteProxy::start("youtube-direct-hls-audio");
    let info = downloader(&proxy, Some(FORMAT))
        .extract_info_with_format(VIDEO, Some(FORMAT))
        .unwrap();
    assert_eq!(info.height, Some(720));
    assert_eq!(info.requested_formats.len(), 2);
    assert_eq!(info.requested_formats[1].vcodec.as_deref(), Some("none"));
    assert_eq!(info.requested_formats[1].language.as_deref(), Some("en"));
    assert!(
        info.requested_formats
            .iter()
            .all(|format| format.fragments.is_empty())
    );
    assert_eq!(proxy.requests.lock().unwrap().len(), 2);
}

#[test]
fn normal_hls_download_extraction_still_traverses_children_and_keeps_fragments() {
    let proxy = site_proxy::SiteProxy::start("youtube-direct-hls-quality");
    let info = downloader(&proxy, None)
        .extract_info_with_format("https://manifest.test/hls/master.m3u8", Some("best"))
        .unwrap();
    assert_eq!(info.height, Some(720));
    assert!(!info.fragments.is_empty());
    let requests = proxy.requests.lock().unwrap();
    assert_eq!(
        requests.len(),
        4,
        "Normal acquisition including VOD check: {requests:?}"
    );
    assert!(requests.iter().any(|url| url.contains("/itag/95/")));
    assert!(requests.iter().any(|url| url.contains("/itag/96/")));
}

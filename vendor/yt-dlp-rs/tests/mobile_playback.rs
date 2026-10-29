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

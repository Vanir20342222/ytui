mod angel;
mod archiveorg;
mod audioboom;
mod bandcamp;
mod behindkink;
mod bild;
mod bilibili;
pub(crate) mod bitchute;
mod bitmovin;
mod blogger;
mod bluesky;
mod breitbart;
mod cgtn;
mod cjsw;
mod coub;
mod cozytv;
mod crooksandliars;
mod crtvg;
mod cultureunplugged;
mod dailymotion;
mod dash;
pub(crate) mod direct;
mod ebaumsworld;
mod ebay;
mod epoch;
mod f4m;
mod facebook;
mod fathom;
mod fczenit;
mod filmweb;
mod fuyintv;
mod generic_urls;
mod glide;
mod godresource;
mod godtube;
mod graspop;
mod harpodeon;
mod historicfilms;
mod hitrecord;
pub(crate) mod hls;
pub(crate) mod html5;
mod huajiao;
mod hypem;
mod imgur;
mod instagram;
pub(crate) mod ism;
mod jove;
mod json_ld;
mod jwplatform;
mod jwplayer;
mod kick;
mod kommunetv;
mod ku6;
mod kvs;
mod lbry;
mod linkedin;
mod livestreamfails;
mod loom;
mod maariv;
mod masters;
mod megaphone;
mod mixcloud;
mod movingimage;
mod niconico;
mod nobelprize;
mod odnoklassniki;
mod oneplace;
mod parlview;
mod photobucket;
mod pinterest;
mod playerfm;
mod popcorntv;
mod premiershiprugby;
mod qingting;
mod reddit;
mod reverbnation;
mod rss;
mod rtrfm;
mod rumble;
mod rutube;
mod screenrec;
mod sen;
mod smil;
mod soundcloud;
mod streamable;
mod sztvhu;
mod ted;
mod theintercept;
mod thisamericanlife;
mod tiktok;
mod tumblr;
mod tweakers;
mod twentythreevideo;
mod twitch;
mod twitter;
mod vimeo;
mod vk;
mod vocaroo;
mod weibo;
mod xspf;
mod yfanefa;
mod youtube;
mod zhihu;

pub use angel::{Angel, suitable as angel_suitable};
pub use archiveorg::{ArchiveOrg, suitable as archiveorg_suitable};
pub use audioboom::{AudioBoom, suitable as audioboom_suitable};
pub use bandcamp::{Bandcamp, suitable as bandcamp_suitable};
pub use behindkink::{BehindKink, suitable as behindkink_suitable};
pub use bild::{Bild, suitable as bild_suitable};
pub use bilibili::{BiliBili, suitable as bilibili_suitable};
pub use bitchute::{BitChute, suitable as bitchute_suitable};
pub use bitmovin::{Bitmovin, suitable as bitmovin_suitable};
pub use blogger::{Blogger, suitable as blogger_suitable};
pub use bluesky::{Bluesky, suitable as bluesky_suitable};
pub use breitbart::{BreitBart, suitable as breitbart_suitable};
pub use cgtn::{Cgtn, suitable as cgtn_suitable};
pub use cjsw::{Cjsw, suitable as cjsw_suitable};
pub use coub::{Coub, suitable as coub_suitable};
pub use cozytv::{CozyTv, suitable as cozytv_suitable};
pub use crooksandliars::{CrooksAndLiars, suitable as crooksandliars_suitable};
pub use crtvg::{Crtvg, suitable as crtvg_suitable};
pub use cultureunplugged::{CultureUnplugged, suitable as cultureunplugged_suitable};
pub use dailymotion::{Dailymotion, suitable as dailymotion_suitable};
pub use direct::DirectHttp;
pub(crate) use direct::from_response as direct_media_info;
pub use ebaumsworld::{EbaumsWorld, suitable as ebaumsworld_suitable};
pub use ebay::{Ebay, suitable as ebay_suitable};
pub use epoch::{Epoch, suitable as epoch_suitable};
pub(crate) use f4m::fix_ampersands as fix_xml_ampersands;
pub use facebook::{
    Facebook, FacebookPluginsVideo, FacebookReel, plugins_suitable as facebook_plugins_suitable,
    reel_suitable as facebook_reel_suitable, suitable as facebook_suitable,
};
pub use fathom::{Fathom, suitable as fathom_suitable};
pub use fczenit::{Fczenit, suitable as fczenit_suitable};
pub use filmweb::{Filmweb, suitable as filmweb_suitable};
pub use fuyintv::{FuyinTv, suitable as fuyintv_suitable};
pub use glide::{Glide, suitable as glide_suitable};
pub use godresource::{GodResource, suitable as godresource_suitable};
pub use godtube::{GodTube, suitable as godtube_suitable};
pub use graspop::{Graspop, suitable as graspop_suitable};
pub use harpodeon::{Harpodeon, suitable as harpodeon_suitable};
pub use historicfilms::{HistoricFilms, suitable as historicfilms_suitable};
pub use hitrecord::{HitRecord, suitable as hitrecord_suitable};
pub use hls::set_split_discontinuity as hls_split_discontinuity;
pub use html5::GenericHttp;
pub use huajiao::{Huajiao, suitable as huajiao_suitable};
pub use hypem::{Hypem, suitable as hypem_suitable};
pub use imgur::{
    Imgur, ImgurAlbum, ImgurGallery, album_suitable as imgur_album_suitable,
    gallery_suitable as imgur_gallery_suitable, suitable as imgur_suitable,
};
pub use instagram::{Instagram, suitable as instagram_suitable};
pub use jove::{Jove, suitable as jove_suitable};
pub use jwplatform::{JwPlatform, suitable as jwplatform_suitable};
pub use kick::{
    KickClip, KickLive, KickVod, clip_suitable as kick_clip_suitable,
    live_suitable as kick_live_suitable, vod_suitable as kick_vod_suitable,
};
pub use kommunetv::{Kommunetv, suitable as kommunetv_suitable};
pub use ku6::{Ku6, suitable as ku6_suitable};
pub use lbry::{Lbry, suitable as lbry_suitable};
pub use linkedin::{LinkedIn, suitable as linkedin_suitable};
pub use livestreamfails::{Livestreamfails, suitable as livestreamfails_suitable};
pub use loom::{Loom, suitable as loom_suitable};
pub use maariv::{Maariv, suitable as maariv_suitable};
pub use masters::{Masters, suitable as masters_suitable};
pub use megaphone::{Megaphone, suitable as megaphone_suitable};
pub use mixcloud::{Mixcloud, suitable as mixcloud_suitable};
pub use movingimage::{MovingImage, suitable as movingimage_suitable};
pub use niconico::{Niconico, suitable as niconico_suitable};
pub use nobelprize::{NobelPrize, suitable as nobelprize_suitable};
pub use odnoklassniki::{Odnoklassniki, suitable as odnoklassniki_suitable};
pub use oneplace::{OnePlacePodcast, suitable as oneplace_suitable};
pub use parlview::{Parlview, suitable as parlview_suitable};
pub use photobucket::{Photobucket, suitable as photobucket_suitable};
pub use pinterest::{Pinterest, suitable as pinterest_suitable};
pub use playerfm::{PlayerFm, suitable as playerfm_suitable};
pub use popcorntv::{PopcornTv, suitable as popcorntv_suitable};
pub use premiershiprugby::{PremiershipRugby, suitable as premiershiprugby_suitable};
pub use qingting::{QingTing, suitable as qingting_suitable};
pub use reddit::{Reddit, suitable as reddit_suitable};
pub use reverbnation::{ReverbNation, suitable as reverbnation_suitable};
pub use rtrfm::{Rtrfm, suitable as rtrfm_suitable};
pub use rumble::{
    Rumble, RumbleEmbed, embed_suitable as rumble_embed_suitable, suitable as rumble_suitable,
};
pub use rutube::{
    Rutube, RutubeEmbed, embed_suitable as rutube_embed_suitable, suitable as rutube_suitable,
};
pub use screenrec::{ScreenRec, suitable as screenrec_suitable};
pub use sen::{Sen, suitable as sen_suitable};
pub use soundcloud::{Soundcloud, suitable as soundcloud_suitable};
pub use streamable::{Streamable, suitable as streamable_suitable};
pub use sztvhu::{SztvHu, suitable as sztvhu_suitable};
pub use ted::{
    TedEmbed, TedPlaylist, TedSeries, TedTalk, embed_suitable as ted_embed_suitable,
    playlist_suitable as ted_playlist_suitable, series_suitable as ted_series_suitable,
    talk_suitable as ted_talk_suitable,
};
pub use theintercept::{TheIntercept, suitable as theintercept_suitable};
pub use thisamericanlife::{ThisAmericanLife, suitable as thisamericanlife_suitable};
pub use tiktok::{
    TikTok, TikTokVm, suitable as tiktok_suitable, vm_suitable as tiktok_vm_suitable,
};
pub use tumblr::{Tumblr, suitable as tumblr_suitable};
pub use tweakers::{Tweakers, suitable as tweakers_suitable};
pub use twentythreevideo::{TwentyThreeVideo, suitable as twentythreevideo_suitable};
pub use twitch::{
    TwitchClips, TwitchVod, clips_suitable as twitch_clips_suitable,
    vod_suitable as twitch_vod_suitable,
};
pub use twitter::{
    Twitter, TwitterCard, card_suitable as twitter_card_suitable, suitable as twitter_suitable,
};
pub use vimeo::{Vimeo, suitable as vimeo_suitable};
pub use vk::{Vk, suitable as vk_suitable};
pub use vocaroo::{Vocaroo, suitable as vocaroo_suitable};
pub use weibo::{
    Weibo, WeiboVideo, suitable as weibo_suitable, video_suitable as weibo_video_suitable,
};
pub use yfanefa::{Yfanefa, suitable as yfanefa_suitable};
pub(crate) use youtube::search::{YoutubeSearch, YoutubeSearchUrl};
pub use youtube::search::{
    suitable as youtube_search_suitable, url_suitable as youtube_search_url_suitable,
};
pub(crate) use youtube::tab::YoutubeTab;
pub use youtube::tab::suitable as youtube_tab_suitable;
pub use youtube::{Youtube, YoutubeOptions, suitable as youtube_suitable};
pub use zhihu::{Zhihu, suitable as zhihu_suitable};

use crate::{ExtractResult, Result, VideoInfo};
use reqwest::blocking::Client;
use url::Url;

/// Extractors return None when they do not recognize a URL/response.
/// A result may expose candidate formats; YoutubeDl resolves selection before returning it.
/// Register site extractors before the generic direct-media fallback.
pub trait Extractor: Send + Sync {
    fn key(&self) -> &'static str;
    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>>;

    /// Override to support playlists. Existing video-only extractors need no changes.
    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        Ok(self.extract(client, url)?.map(ExtractResult::Video))
    }
}

/// `[EXTRACTOR] ID: Unable to download WHAT: ERR (caused by REPR)`.
pub(crate) fn request_error_text(
    extractor: &str,
    id: &str,
    what: &str,
    error: &crate::Error,
) -> String {
    let text = crate::upstream_error_text(error);
    let status = match error {
        crate::Error::Http(source) | crate::Error::HttpRetryAfter { source, .. } => source.status(),
        _ => None,
    };
    let cause = match status {
        Some(_) => format!("<{}>", text.replacen("HTTP Error ", "HTTPError ", 1)),
        None => format!("TransportError({})", crate::batch::py_repr(&text)),
    };
    format!("[{extractor}] {id}: Unable to download {what}: {text} (caused by {cause})")
}

/// Upstream `_report_ignoring_subs(NAME)` for `_extract_m3u8_formats` and
/// `_extract_mpd_formats`, which drop a manifest's subtitles.
fn ignore_subtitles(info: &mut crate::VideoInfo, extractor: &str, name: &str) {
    if !info.subtitles.is_empty() {
        crate::screen::warning_once(&format!(
            "[{extractor}] Ignoring subtitle tracks found in the {name} manifest; if any subtitle tracks are missing, {}",
            crate::BUG_REPORTS_MESSAGE
        ));
        info.subtitles = Default::default();
    }
}

/// Upstream fatal `_extract_m3u8_formats_and_subtitles(url, video_id,
/// m3u8_id=M3U8_ID)` (or `_extract_m3u8_formats` without
/// `keep_subtitles`): `[EXTRACTOR] ID: Downloading m3u8 information`, the
/// request (an HTTP error is `Failed to download m3u8 information`), and the
/// playlist's formats and subtitles.
pub(crate) fn extract_m3u8_fatal(
    client: &Client,
    manifest: &Url,
    extractor: &'static str,
    id: &str,
    m3u8_id: Option<&str>,
    keep_subtitles: bool,
) -> crate::Result<crate::VideoInfo> {
    let note = format!("[{extractor}] {id}: Downloading m3u8 information");
    crate::screen::expect_page(id, Some(note.clone()), None);
    let response = direct::request(client, manifest, None);
    if response.is_err() {
        crate::screen::screen(&note);
    }
    let parsed = response
        .map_err(|error| match error {
            crate::Error::Http(_) | crate::Error::HttpRetryAfter { .. } => crate::Error::Reported(
                request_error_text(extractor, id, "m3u8 information", &error).replacen(
                    ": Unable to download ",
                    ": Failed to download ",
                    1,
                ),
            ),
            error => error,
        })
        .and_then(|response| hls::from_response(client, response, manifest, None, true, m3u8_id));
    crate::screen::clear_page();
    let mut info = parsed?.ok_or_else(|| {
        crate::Error::UnsupportedHtml5(format!("the {extractor} stream is not an HLS playlist"))
    })?;
    if !keep_subtitles {
        ignore_subtitles(&mut info, extractor, "HLS");
    }
    Ok(info)
}

/// Upstream `_extract_m3u8_formats_and_subtitles(..., fatal=False)`: the
/// playlist, or `None` after a `Failed to download m3u8 information`
/// warning.
pub(crate) fn extract_m3u8_optional(
    client: &Client,
    manifest: &Url,
    extractor: &'static str,
    id: &str,
    m3u8_id: Option<&str>,
    keep_subtitles: bool,
) -> Option<crate::VideoInfo> {
    extract_manifest_optional(
        client,
        manifest,
        extractor,
        id,
        keep_subtitles,
        &|response| hls::from_response(client, response, manifest, None, true, m3u8_id),
        ("m3u8 information", "HLS"),
    )
}

/// Mobile playback needs the master catalog, not every child's segment list.
pub(crate) fn extract_m3u8_for_playback(
    client: &Client,
    manifest: &Url,
    id: &str,
) -> Option<crate::VideoInfo> {
    extract_manifest_optional(
        client,
        manifest,
        "youtube",
        id,
        true,
        &|response| hls::from_response_for_playback(client, response, manifest),
        ("m3u8 information", "HLS"),
    )
}

/// Upstream `_extract_mpd_formats_and_subtitles(..., fatal=False)` (or
/// `_extract_mpd_formats` without `keep_subtitles`), without an `mpd_id`.
pub(crate) fn extract_mpd_optional(
    client: &Client,
    manifest: &Url,
    extractor: &'static str,
    id: &str,
    keep_subtitles: bool,
) -> Option<crate::VideoInfo> {
    extract_manifest_optional(
        client,
        manifest,
        extractor,
        id,
        keep_subtitles,
        &|response| dash::from_response(response, manifest, true),
        ("MPD manifest", "DASH"),
    )
}

/// A non-fatal manifest request: `Downloading WHAT`, else a `Failed to
/// download WHAT` warning.
fn extract_manifest_optional(
    client: &Client,
    manifest: &Url,
    extractor: &'static str,
    id: &str,
    keep_subtitles: bool,
    parse: &dyn Fn(reqwest::blocking::Response) -> crate::Result<Option<crate::VideoInfo>>,
    (what, name): (&str, &str),
) -> Option<crate::VideoInfo> {
    extract_manifest_optional_note(
        client,
        manifest,
        extractor,
        id,
        keep_subtitles,
        parse,
        (what, name),
        &format!("Downloading {what}"),
    )
}

/// [`extract_m3u8_optional`] with its `note=`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn extract_m3u8_optional_note(
    client: &Client,
    manifest: &Url,
    extractor: &'static str,
    id: &str,
    m3u8_id: Option<&str>,
    keep_subtitles: bool,
    live: bool,
    note: &str,
) -> Option<crate::VideoInfo> {
    let mut stream = extract_manifest_optional_note(
        client,
        manifest,
        extractor,
        id,
        keep_subtitles,
        &|response| hls::from_response(client, response, manifest, None, true, m3u8_id),
        ("m3u8 information", "HLS"),
        note,
    )?;
    if live {
        hls::live_format_ids(&mut stream.formats, m3u8_id);
    }
    Some(stream)
}

/// `_extract_mpd_formats_and_subtitles(url, id, mpd_id, note, fatal=False)`.
pub(crate) fn extract_mpd_optional_note(
    client: &Client,
    manifest: &Url,
    extractor: &'static str,
    id: &str,
    mpd_id: Option<&str>,
    note: &str,
) -> Option<crate::VideoInfo> {
    let mut stream = extract_manifest_optional_note(
        client,
        manifest,
        extractor,
        id,
        true,
        &|response| dash::from_response(response, manifest, true),
        ("MPD manifest", "DASH"),
        note,
    )?;
    if let Some(mpd_id) = mpd_id {
        for format in &mut stream.formats {
            format.format_id = format!("{mpd_id}-{}", format.format_id);
        }
    }
    Some(stream)
}

#[allow(clippy::too_many_arguments)]
fn extract_manifest_optional_note(
    client: &Client,
    manifest: &Url,
    extractor: &'static str,
    id: &str,
    keep_subtitles: bool,
    parse: &dyn Fn(reqwest::blocking::Response) -> crate::Result<Option<crate::VideoInfo>>,
    (what, name): (&str, &str),
    note: &str,
) -> Option<crate::VideoInfo> {
    let note = format!("[{extractor}] {id}: {note}");
    crate::screen::expect_page(id, Some(note.clone()), None);
    let response = direct::request(client, manifest, None);
    if response.is_err() {
        crate::screen::screen(&note);
    }
    let parsed = response.and_then(parse);
    crate::screen::clear_page();
    match parsed {
        Ok(mut stream) => {
            if !keep_subtitles && let Some(stream) = &mut stream {
                ignore_subtitles(stream, extractor, name);
            }
            stream
        }
        Err(error) => {
            crate::screen::warning(&format!(
                "[{extractor}] {id}: Failed to download {what}: {}",
                crate::upstream_error_text(&error)
            ));
            None
        }
    }
}

/// Upstream `_download_json(url, video_id)`: `[EXTRACTOR] ID: Downloading
/// JSON metadata`, the request, and the parsed body.
pub(crate) fn download_json(
    client: &Client,
    url: &Url,
    extractor: &'static str,
    id: &str,
) -> Result<serde_json::Value> {
    download_json_note(client, url, extractor, id, "Downloading JSON metadata")
}

/// [`download_json`] with its `note=` (the error note stays the default).
pub(crate) fn download_json_note(
    client: &Client,
    url: &Url,
    extractor: &'static str,
    id: &str,
    note: &str,
) -> Result<serde_json::Value> {
    download_json_transformed(
        client,
        url,
        extractor,
        id,
        note,
        &|text| Ok(text.to_owned()),
    )
}

/// [`download_json_note`] with the request's `headers=`.
pub(crate) fn download_json_note_headers(
    client: &Client,
    url: &Url,
    extractor: &'static str,
    id: &str,
    note: &str,
    headers: &[(&str, &str)],
) -> Result<serde_json::Value> {
    download_json_with(
        client,
        url,
        extractor,
        id,
        note,
        &|text| Ok(text.to_owned()),
        headers,
    )
}

/// [`download_json`] with the request's `headers=`.
pub(crate) fn download_json_headers(
    client: &Client,
    url: &Url,
    extractor: &'static str,
    id: &str,
    headers: &[(&str, &str)],
) -> Result<serde_json::Value> {
    download_json_with(
        client,
        url,
        extractor,
        id,
        "Downloading JSON metadata",
        &|text| Ok(text.to_owned()),
        headers,
    )
}

/// [`download_json_note`] with `transform_source` applied to the body.
pub(crate) fn download_json_transformed(
    client: &Client,
    url: &Url,
    extractor: &'static str,
    id: &str,
    note: &str,
    transform: &dyn Fn(&str) -> Result<String>,
) -> Result<serde_json::Value> {
    download_json_with(client, url, extractor, id, note, transform, &[])
}

fn download_json_with(
    client: &Client,
    url: &Url,
    extractor: &'static str,
    id: &str,
    note: &str,
    transform: &dyn Fn(&str) -> Result<String>,
    headers: &[(&str, &str)],
) -> Result<serde_json::Value> {
    use std::io::Read;
    crate::screen::screen(&format!("[{extractor}] {id}: {note}"));
    let wrap = |error: crate::Error| match error {
        crate::Error::Http(_) | crate::Error::HttpRetryAfter { .. } => {
            crate::Error::ExtractorRequest {
                extractor,
                id: id.to_owned(),
                what: "JSON metadata",
                source: Box::new(error),
            }
        }
        error => error,
    };
    let mut response = direct::request_with_headers(client, url, None, headers).map_err(wrap)?;
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(crate::Error::ExtractionRead)?;
    let text = transform(&String::from_utf8_lossy(&body))?;
    serde_json::from_str(&text).map_err(|error| {
        crate::Error::Reported(format!(
            "[{extractor}] {id}: Failed to parse JSON  (caused by {error})"
        ))
    })
}

/// Upstream `update_url_query(url, query)`: the query decoded, updated and
/// re-encoded like `urlencode`.
pub(crate) fn update_url_query(url: &Url, updates: &[(&str, &str)]) -> Url {
    // `parse_qs` (blank values dropped, repeated keys grouped) and
    // `dict.update` (an updated key keeps its place).
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for (key, value) in url.query_pairs() {
        if value.is_empty() {
            continue;
        }
        match groups.iter_mut().find(|(existing, _)| *existing == key) {
            Some((_, values)) => values.push(value.into_owned()),
            None => groups.push((key.into_owned(), vec![value.into_owned()])),
        }
    }
    for (key, value) in updates {
        match groups.iter_mut().find(|(existing, _)| existing == key) {
            Some((_, values)) => *values = vec![(*value).to_owned()],
            None => groups.push(((*key).to_owned(), vec![(*value).to_owned()])),
        }
    }
    let pairs: Vec<(String, String)> = groups
        .into_iter()
        .flat_map(|(key, values)| values.into_iter().map(move |value| (key.clone(), value)))
        .collect();
    let mut url = url.clone();
    let query = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(&pairs)
        .finish();
    url.set_query((!query.is_empty()).then_some(query.as_str()));
    url
}

/// Upstream `_download_webpage(url, video_id)`: `[EXTRACTOR] ID: Downloading
/// webpage`, the request (`--dump-pages`/`--write-pages` like other
/// pages), and the body as UTF-8.
pub(crate) fn download_webpage(
    client: &Client,
    url: &Url,
    extractor: &'static str,
    id: &str,
) -> Result<String> {
    download_webpage_note(client, url, extractor, id, "Downloading webpage")
}

/// [`download_webpage`] with another `note`.
pub(crate) fn download_webpage_note(
    client: &Client,
    url: &Url,
    extractor: &'static str,
    id: &str,
    note: &str,
) -> Result<String> {
    crate::screen::screen(&format!("[{extractor}] {id}: {note}"));
    download_webpage_handle_quiet(client, url, extractor, id).map(|(text, _)| text)
}

/// `_download_webpage(url, None, note)`: the note was printed by the caller.
pub(crate) fn download_webpage_quiet(
    client: &Client,
    url: &Url,
    extractor: &'static str,
    id: &str,
) -> Result<String> {
    download_webpage_handle_quiet(client, url, extractor, id).map(|(text, _)| text)
}

/// `_download_webpage_handle(url, video_id)`: the page and its final URL.
pub(crate) fn download_webpage_handle(
    client: &Client,
    url: &Url,
    extractor: &'static str,
    id: &str,
) -> Result<(String, Url)> {
    crate::screen::screen(&format!("[{extractor}] {id}: Downloading webpage"));
    download_webpage_handle_quiet(client, url, extractor, id)
}

fn download_webpage_handle_quiet(
    client: &Client,
    url: &Url,
    extractor: &'static str,
    id: &str,
) -> Result<(String, Url)> {
    use std::io::Read;
    let wrap = |error: crate::Error| match error {
        crate::Error::Http(_) | crate::Error::HttpRetryAfter { .. } => {
            crate::Error::ExtractorRequest {
                extractor,
                id: id.to_owned(),
                what: "webpage",
                source: Box::new(error),
            }
        }
        error => error,
    };
    let mut response = direct::request(client, url, None).map_err(wrap)?;
    let page_url = response.url().clone();
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(crate::Error::ExtractionRead)?;
    crate::screen::expect_page(id, None, None);
    crate::screen::page_read(&page_url, &body)?;
    Ok((String::from_utf8_lossy(&body).into_owned(), page_url))
}

/// Upstream `_download_xml(url, video_id)`: `[EXTRACTOR] ID: Downloading
/// XML`, the request, and the body. Parse errors are this port's text
/// (upstream reports expat's).
pub(crate) fn download_xml(
    client: &Client,
    url: &Url,
    extractor: &'static str,
    id: &str,
) -> Result<String> {
    download_xml_note(client, url, extractor, id, "Downloading XML")
}

/// [`download_xml`] with its `note=` (the error note stays the default).
pub(crate) fn download_xml_note(
    client: &Client,
    url: &Url,
    extractor: &'static str,
    id: &str,
    note: &str,
) -> Result<String> {
    use std::io::Read;
    crate::screen::screen(&format!("[{extractor}] {id}: {note}"));
    let wrap = |error: crate::Error| match error {
        crate::Error::Http(_) | crate::Error::HttpRetryAfter { .. } => {
            crate::Error::ExtractorRequest {
                extractor,
                id: id.to_owned(),
                what: "XML",
                source: Box::new(error),
            }
        }
        error => error,
    };
    let mut response = direct::request(client, url, None).map_err(wrap)?;
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(crate::Error::ExtractionRead)?;
    let text = String::from_utf8(body).map_err(|_| {
        crate::Error::Reported(format!(
            "[{extractor}] {id}: Failed to parse XML (not UTF-8)"
        ))
    })?;
    roxmltree::Document::parse(text.trim_start_matches('\u{feff}')).map_err(|error| {
        crate::Error::Reported(format!(
            "[{extractor}] {id}: Failed to parse XML ({error}); {}",
            crate::BUG_REPORTS_MESSAGE
        ))
    })?;
    Ok(text)
}

/// Upstream `_WORKING = False`: the warning before extraction.
pub(crate) fn broken_warning() {
    crate::screen::warning(
        "The program functionality for this site has been marked as broken, and will probably not work.",
    );
}

/// `root.find(tag).text` of an XML document: `Err` (upstream's bare
/// `AttributeError`) for a missing child, `None` for an empty one.
pub(crate) fn find_text(root: roxmltree::Node<'_, '_>, tag: &str) -> Result<Option<String>> {
    let child = root
        .children()
        .find(|child| child.is_element() && child.tag_name().name() == tag)
        .ok_or_else(|| {
            crate::Error::Reported("'NoneType' object has no attribute 'text'".into())
        })?;
    let text: String = child
        .children()
        .take_while(|node| !node.is_element())
        .filter_map(|node| node.text())
        .collect();
    Ok((!text.is_empty()).then_some(text))
}

/// `_search_json(start_pattern, string, ...)`: the greedy `{...}` after the
/// start (`contains_pattern=r'{(?s:.+)}'`), optionally converted with
/// `js_to_json`, and its first JSON value (`ignore_extra=True`); `None`
/// without a match, `Some(None)` when it does not parse.
pub(crate) fn search_json(
    start: &regex::Regex,
    html: &str,
    js: bool,
) -> Option<Option<serde_json::Value>> {
    if js {
        search_json_with(start, html, &crate::js_to_json::js_to_json)
    } else {
        search_json_with(start, html, &str::to_owned)
    }
}

/// [`search_json`] with any `transform_source`.
pub(crate) fn search_json_with(
    start: &regex::Regex,
    html: &str,
    transform: &dyn Fn(&str) -> String,
) -> Option<Option<serde_json::Value>> {
    let found = start.find(html)?;
    let rest = &html[found.end()..];
    let trimmed = rest.trim_start();
    if !trimmed.starts_with('{') {
        return None;
    }
    let end = trimmed.rfind('}')?;
    let candidate = &trimmed[..=end];
    let text = transform(candidate);
    Some(
        serde_json::Deserializer::from_str(&text)
            .into_iter::<serde_json::Value>()
            .next()
            .and_then(std::result::Result::ok),
    )
}

/// `_search_nextjs_data(webpage, video_id)` (fatal): the `__NEXT_DATA__`
/// script's JSON (greedy to the last `}` before a `</script>`).
pub(crate) fn search_nextjs_data(
    html: &str,
    extractor: &str,
    id: &str,
) -> crate::Result<serde_json::Value> {
    static NEXT_DATA: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r#"(?s)<script[^>]+id=['"]__NEXT_DATA__['"][^>]*>\s*(\{.+\})\s*</script>"#,
        )
        .expect("static pattern")
    });
    let Some(found) = NEXT_DATA.captures(html) else {
        return Err(crate::Error::Reported(format!(
            "[{extractor}] {id}: Unable to extract next.js data; {}",
            crate::BUG_REPORTS_MESSAGE
        )));
    };
    serde_json::Deserializer::from_str(&found[1])
        .into_iter::<serde_json::Value>()
        .next()
        .and_then(std::result::Result::ok)
        .ok_or_else(|| {
            crate::Error::Reported(format!(
                "[{extractor}] {id}: Unable to extract next.js data - Failed to parse JSON"
            ))
        })
}

/// Whether an extractor's `_VALID_URL` takes this non-HTTP pseudo-URL.
pub fn pseudo_url(url: &str) -> bool {
    (url.starts_with("jwplatform:") && jwplatform::suitable(url))
        || youtube::search::suitable(url)
        || (url.starts_with("at://") && bluesky::suitable(url))
        || (url.starts_with("lbry://") && lbry::suitable(url))
        || (url.starts_with("coub:") && coub::suitable(url))
}

/// Upstream `params['http_headers']['User-Agent']`.
pub(crate) static USER_AGENT: std::sync::RwLock<String> = std::sync::RwLock::new(String::new());

/// `self.get_param('http_headers')['User-Agent']`.
pub(crate) fn user_agent() -> String {
    USER_AGENT
        .read()
        .map(|agent| agent.clone())
        .unwrap_or_default()
}

/// Upstream `params['videopassword']` (`--video-password`).
static VIDEO_PASSWORD: std::sync::RwLock<Option<String>> = std::sync::RwLock::new(None);

/// Set `--video-password` for extractors that read it.
pub fn set_video_password(password: Option<String>) {
    if let Ok(mut slot) = VIDEO_PASSWORD.write() {
        *slot = password;
    }
}

/// `self.get_param('videopassword')`.
pub(crate) fn video_password() -> Option<String> {
    VIDEO_PASSWORD
        .read()
        .ok()
        .and_then(|password| password.clone())
}

/// `self.get_param('check_formats') is False` (`--no-check-formats`).
pub(crate) static CHECK_FORMATS_NEVER: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// `self.get_param('check_formats') is False`.
pub(crate) fn check_formats_never() -> bool {
    CHECK_FORMATS_NEVER.load(std::sync::atomic::Ordering::Relaxed)
}

/// Upstream `noplaylist` (`--no-playlist`), for `_yes_playlist`.
pub(crate) static NO_PLAYLIST: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// `self.get_param('noplaylist')`.
pub(crate) fn no_playlist() -> bool {
    NO_PLAYLIST.load(std::sync::atomic::Ordering::Relaxed)
}

/// Upstream `writesubtitles or listsubtitles` (`InfoExtractor.extract_subtitles`).
pub(crate) static SUBTITLES_REQUESTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// `extract_subtitles`: whether an extractor's `_get_subtitles` runs.
pub(crate) fn subtitles_requested() -> bool {
    SUBTITLES_REQUESTED.load(std::sync::atomic::Ordering::Relaxed)
}

/// A non-fatal `_search_regex` miss: `[EXTRACTOR] unable to extract NAME`.
pub(crate) fn search_warning(extractor: &str, name: &str) {
    crate::screen::warning(&format!(
        "[{extractor}] unable to extract {name}; {}",
        crate::BUG_REPORTS_MESSAGE
    ));
}

/// `self._proto_relative_url(url, scheme='http:')`.
pub(crate) fn proto_relative_http(url: &str) -> String {
    match url.strip_prefix("//") {
        Some(rest) => format!("http://{rest}"),
        None => url.to_owned(),
    }
}

/// `self._proto_relative_url(url)`: `//host/...` becomes `https://host/...`.
pub(crate) fn proto_relative(url: &str) -> String {
    match url.strip_prefix("//") {
        Some(rest) => format!("https://{rest}"),
        None => url.to_owned(),
    }
}

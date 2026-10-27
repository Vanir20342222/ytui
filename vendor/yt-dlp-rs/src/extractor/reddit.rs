//! Upstream `RedditIE` (`yt_dlp/extractor/reddit.py`): a post's `.json`
//! listing, its Reddit-hosted video (fallback, HLS and DASH formats), the
//! videos embedded in a text post, or a `url_transparent` link.
use super::{Extractor, extract_m3u8_optional, extract_mpd_optional};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Reddit;

const NAME: &str = "Reddit";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:\w+\.)?reddit(?:media)?\.com/(?P<slug>(?:(?:r|user)/[^/]+/)?comments/(?P<id>[^/?#&]+))",
    )
    .unwrap()
});
static VIDEO_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"https?://v\.redd\.it/(?P<id>[^/?#&]+)").unwrap());

/// `urllib.parse.quote`'s unsafe characters (everything but
/// `A-Za-z0-9_.-~/`).
const QUOTE_SET: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
    .remove(b'_')
    .remove(b'.')
    .remove(b'-')
    .remove(b'~')
    .remove(b'/');

/// `_real_initialize` runs once per extractor instance.
static READY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `truncate_string(s, left=72)`.
fn truncate(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= 72 {
        return text.to_owned();
    }
    format!("{}...", chars[..69].iter().collect::<String>())
}

fn int(value: Option<&Value>) -> Value {
    value
        .and_then(|value| {
            value
                .as_i64()
                .or_else(|| value.as_f64().map(|number| number as i64))
                .or_else(|| value.as_str()?.trim().parse().ok())
        })
        .map_or(Value::Null, Value::from)
}

/// `_real_initialize` and the shreddit fallback: the anonymous `loid`
/// session and the age/gated opt-in cookies.
fn initialize(client: &Client, slug: &str, id: &str, url: &Url) {
    let logged_in =
        crate::cookies::get_cookie("https://www.reddit.com/", "reddit_session").is_some();
    if READY.swap(true, std::sync::atomic::Ordering::Relaxed) {
        shreddit_session(client, slug, id, url, logged_in);
        return;
    }
    if !logged_in {
        crate::screen::screen(&format!("[{NAME}] Setting up session via old reddit"));
        if let Err(error) = Url::parse("https://old.reddit.com/")
            .map_err(Error::from)
            .and_then(|old| super::direct::request(client, &old, None))
        {
            crate::screen::warning(&format!(
                "[{NAME}] Session request failed: {}",
                crate::upstream_error_text(&error)
            ));
        }
    }
    crate::cookies::set_cookie("reddit.com", "over18", "1");
    let mut options = crate::cookies::get_cookie("https://www.reddit.com/", "_options")
        .and_then(|value| {
            let decoded = percent_encoding::percent_decode_str(&value)
                .decode_utf8_lossy()
                .into_owned();
            serde_json::from_str::<Map<String, Value>>(&decoded).ok()
        })
        .unwrap_or_default();
    options.insert("pref_gated_sr_optin".into(), true.into());
    // `urllib.parse.quote(json.dumps(options))`.
    let dumped = crate::output::python_json_dumps(&Value::Object(options));
    let quoted = percent_encoding::utf8_percent_encode(&dumped, QUOTE_SET).to_string();
    crate::cookies::set_cookie("reddit.com", "_options", &quoted);
    shreddit_session(client, slug, id, url, logged_in);
}

/// The shreddit session request when old reddit gave no `loid`.
fn shreddit_session(client: &Client, slug: &str, id: &str, url: &Url, logged_in: bool) {
    if !logged_in && crate::cookies::get_cookie("https://www.reddit.com/", "loid").is_none() {
        crate::screen::screen(&format!("[{NAME}] {id}: Setting up session via shreddit"));
        let mut shreddit = match Url::parse(&format!("https://www.reddit.com/svc/shreddit/{slug}"))
        {
            Ok(shreddit) => shreddit,
            Err(_) => return,
        };
        shreddit
            .query_pairs_mut()
            .append_pair("seeker-session", "false")
            .append_pair("render-mode", "partial")
            .append_pair("referer", url.as_str());
        if let Err(error) = super::direct::request(client, &shreddit, None) {
            crate::screen::warning(&format!(
                "[{NAME}] {id}: Session request failed: {}",
                crate::upstream_error_text(&error)
            ));
        }
    }
}

/// `_download_json(f'https://www.reddit.com/{slug}/.json', expected_status=403)`.
fn post_json(client: &Client, slug: &str, id: &str) -> Result<Value> {
    use std::io::Read;
    let api = Url::parse(&format!("https://www.reddit.com/{slug}/.json"))?;
    crate::screen::screen(&format!("[{NAME}] {id}: Downloading JSON metadata"));
    let mut response = super::direct::request_expecting(client, &api, &[], &[403]).map_err(
        |error| match error {
            Error::Http(_) | Error::HttpRetryAfter { .. } => Error::ExtractorRequest {
                extractor: NAME,
                id: id.to_owned(),
                what: "JSON metadata",
                source: Box::new(error),
            },
            error => error,
        },
    )?;
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    serde_json::from_slice(&body).map_err(|_| {
        // A non-JSON answer: upstream's `raise_login_required`.
        if crate::cookies::get_cookie("https://www.reddit.com/", "reddit_session").is_some() {
            Error::Reported(format!(
                "[{NAME}] {id}: Your IP address is unable to access the Reddit API"
            ))
        } else {
            Error::Reported(format!(
                "[{NAME}] {id}: Account authentication is required. Use --cookies-from-browser or --cookies for the authentication. See  https://github.com/yt-dlp/yt-dlp/wiki/FAQ#how-do-i-pass-cookies-to-yt-dlp  for how to manually pass cookies"
            ))
        }
    })
}

/// The post's thumbnails (`add_thumbnail` over the preview images).
fn thumbnails(data: &Value) -> Vec<Value> {
    let mut found = Vec::new();
    let mut add = |source: Option<&Value>| {
        let Some(source) = source.filter(|value| value.is_object()) else {
            return;
        };
        let Some(url) = source
            .get("url")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
        else {
            return;
        };
        found.push(json!({
            "url": crate::html::unescape_html(&url),
            "width": int(source.get("width")),
            "height": int(source.get("height")),
            "http_headers": {"Accept": "*/*"},
        }));
    };
    for image in data
        .pointer("/preview/images")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|image| image.is_object())
    {
        add(image.get("source"));
        for resolution in image
            .get("resolutions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            add(Some(resolution));
        }
    }
    found
}

/// The `info` dict every result shares.
fn post_info(data: &Value) -> Map<String, Value> {
    let mut info = Map::new();
    info.insert("thumbnails".into(), Value::Array(thumbnails(data)));
    info.insert(
        "age_limit".into(),
        match data.get("over_18") {
            Some(Value::Bool(true)) => 18.into(),
            Some(Value::Bool(false)) => 0.into(),
            _ => Value::Null,
        },
    );
    let text = |key: &str| data.get(key).and_then(Value::as_str);
    if let Some(title) = text("title") {
        info.insert("title".into(), truncate(title).into());
        info.insert("alt_title".into(), title.into());
    }
    if let Some(description) = text("selftext").filter(|text| !text.is_empty()) {
        info.insert("description".into(), description.into());
    }
    // `float_or_none(created_utc)`.
    if let Some(timestamp) = data.get("created_utc").and_then(|value| {
        value
            .as_f64()
            .or_else(|| value.as_str()?.trim().parse().ok())
    }) {
        info.insert("timestamp".into(), json!(timestamp));
    }
    if let Some(author) = text("author") {
        info.insert("uploader".into(), author.into());
    }
    if let Some(subreddit) = text("subreddit") {
        info.insert("channel_id".into(), subreddit.into());
    }
    for (key, source) in [
        ("like_count", "ups"),
        ("dislike_count", "downs"),
        ("comment_count", "num_comments"),
    ] {
        let value = int(data.get(source));
        if !value.is_null() {
            info.insert(key.into(), value);
        }
    }
    info
}

/// `_extract_m3u8_formats(…, m3u8_id='hls', fatal=False)` (with subtitles).
fn hls(
    client: &Client,
    url: &str,
    id: &str,
    subtitles: Option<&mut crate::Subtitles>,
) -> Vec<crate::VideoFormat> {
    let Ok(manifest) = Url::parse(url) else {
        return Vec::new();
    };
    let keep = subtitles.is_some();
    let Some(stream) = extract_m3u8_optional(client, &manifest, NAME, id, Some("hls"), keep) else {
        return Vec::new();
    };
    if let Some(subtitles) = subtitles {
        super::html5::merge_subtitles(subtitles, stream.subtitles);
    }
    stream.formats
}

/// `_extract_mpd_formats(…, mpd_id='dash', fatal=False)` (with subtitles).
fn dash(
    client: &Client,
    url: &str,
    id: &str,
    subtitles: Option<&mut crate::Subtitles>,
) -> Vec<crate::VideoFormat> {
    let Ok(manifest) = Url::parse(url) else {
        return Vec::new();
    };
    let keep = subtitles.is_some();
    let Some(stream) = extract_mpd_optional(client, &manifest, NAME, id, keep) else {
        return Vec::new();
    };
    if let Some(subtitles) = subtitles {
        super::html5::merge_subtitles(subtitles, stream.subtitles);
    }
    stream
        .formats
        .into_iter()
        .map(|mut format| {
            format.format_id = format!("dash-{}", format.format_id);
            format
        })
        .collect()
}

/// A video result from upstream's dict (`dict;KEYS` order).
fn video(
    url: &Url,
    dict: Map<String, Value>,
    formats: Vec<crate::VideoFormat>,
    subtitles: crate::Subtitles,
) -> VideoInfo {
    let first = formats
        .first()
        .map_or_else(|| url.clone(), |format| format.url.clone());
    let mut info = super::direct::build_at(first.clone(), &first, String::new());
    info.direct = None;
    if formats.is_empty() {
        info.extra
            .insert(crate::info::NO_FORMATS.into(), Value::Bool(true));
    }
    info.formats = formats;
    info.subtitles = subtitles;
    info.webpage_url = url.clone();
    info.extractor = NAME.into();
    info.extractor_key = NAME.into();
    let mut keys = Vec::new();
    for (key, value) in dict {
        keys.push(key.clone());
        match key.as_str() {
            "id" => info.id = value.as_str().unwrap_or_default().to_owned(),
            "title" => info.title = value.as_str().unwrap_or_default().to_owned(),
            "formats" | "subtitles" => {}
            _ if value.is_null() => {
                info.extra.insert(key, Value::Null);
            }
            _ => crate::metadata_parser::set_field(&mut info, &key, value),
        }
    }
    if !keys.iter().any(|key| key == "title") {
        info.title = String::new();
        info.extra
            .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
    }
    crate::info_order::mark_info(
        info,
        &format!(
            "dict;{};thumbnail=url,width,height,http_headers",
            keys.join(",")
        ),
    )
}

/// `self.raise_no_formats(message, expected=True, video_id=ID)`.
fn no_formats(id: &str, message: &str) -> Error {
    Error::Reported(format!("[{NAME}] {id}: {message}"))
}

impl Extractor for Reddit {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let (slug, id) = (found["slug"].to_owned(), found["id"].to_owned());
        crate::screen::set_extractor(NAME);
        initialize(client, &slug, &id, url);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let listing = post_json(client, &slug, &id)?;
        if listing.get("error").and_then(Value::as_i64) == Some(403) {
            let reason = listing.get("reason").and_then(Value::as_str);
            let login = |message: &str| {
                Error::Reported(format!(
                    "[{NAME}] {id}: {message}. Use --cookies-from-browser or --cookies for the authentication. See  https://github.com/yt-dlp/yt-dlp/wiki/FAQ#how-do-i-pass-cookies-to-yt-dlp  for how to manually pass cookies"
                ))
            };
            return Err(match reason {
                Some("quarantined") => {
                    login("Quarantined subreddit; an account that has opted in is required")
                }
                Some("private") => {
                    login("Private subreddit; an account that has been approved is required")
                }
                _ => Error::Reported(format!(
                    "[{NAME}] {id}: HTTP Error 403 Forbidden; reason given: {}",
                    reason.unwrap_or("None")
                )),
            });
        }
        let data = listing
            .pointer("/0/data/children/0/data")
            .cloned()
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {id}: An extractor error has occurred. (caused by KeyError('data')); {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let video_url = data
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let info = post_info(&data);
        let parsed = Url::parse(&video_url).ok();
        let netloc = parsed
            .as_ref()
            .and_then(Url::host_str)
            .unwrap_or_default()
            .to_owned();
        let path = parsed.as_ref().map_or("", Url::path).to_owned();

        // Videos embedded in a text post (a link to the post itself).
        if netloc.contains("reddit.com") && path.contains(&format!("/{id}/")) {
            let mut entries = Vec::new();
            for media in data
                .get("media_metadata")
                .and_then(Value::as_object)
                .into_iter()
                .flat_map(|media| media.values())
                .filter(|media| media.is_object())
            {
                let media_id = media
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty());
                let Some(media_id) = media_id else { continue };
                if media.get("e").and_then(Value::as_str) != Some("RedditVideo") {
                    continue;
                }
                let mut formats = Vec::new();
                if let Some(hls_url) = media
                    .get("hlsUrl")
                    .and_then(Value::as_str)
                    .filter(|u| !u.is_empty())
                {
                    formats.extend(hls(client, &crate::html::unescape_html(hls_url), &id, None));
                }
                if let Some(dash_url) = media
                    .get("dashUrl")
                    .and_then(Value::as_str)
                    .filter(|u| !u.is_empty())
                {
                    formats.extend(dash(
                        client,
                        &crate::html::unescape_html(dash_url),
                        &id,
                        None,
                    ));
                }
                if formats.is_empty() {
                    continue;
                }
                let mut dict = Map::new();
                dict.insert("id".into(), media_id.into());
                dict.insert("display_id".into(), id.clone().into());
                dict.insert("formats".into(), Value::Null);
                dict.extend(info.clone());
                entries.push(video(url, dict, formats, Default::default()));
            }
            if entries.is_empty() {
                return Err(no_formats(&id, "No media found"));
            }
            let mut extra = info.clone();
            let title = extra
                .shift_remove("title")
                .and_then(|title| title.as_str().map(str::to_owned))
                .unwrap_or_default();
            let mut keys: Vec<String> = info.keys().cloned().collect();
            keys.push("id".into());
            extra.insert(
                crate::info_order::INFO_SHAPE.into(),
                format!("playlist-dict;{}", keys.join(",")).into(),
            );
            return Ok(Some(ExtractResult::Playlist(PlaylistInfo {
                id: id.clone(),
                title,
                webpage_url: url.clone(),
                extractor: NAME.into(),
                extractor_key: NAME.into(),
                entries,
                extra,
            })));
        }

        // Media hosted on Reddit (the post's, else a crosspost's).
        let reddit_video = std::iter::once(&data)
            .chain(
                data.get("crosspost_parent_list")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten(),
            )
            .find_map(|post| {
                ["secure_media", "media"].iter().find_map(|key| {
                    post.get(*key)
                        .and_then(|media| media.get("reddit_video"))
                        .filter(|value| !value.is_null())
                })
            })
            .cloned();
        if let Some(reddit_video) = reddit_video {
            let playlist = |key: &str| {
                reddit_video
                    .get(key)
                    .and_then(Value::as_str)
                    .map(crate::html::unescape_html)
                    .filter(|url| !url.is_empty())
            };
            let fallback = reddit_video
                .get("fallback_url")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    Error::Reported(format!(
                        "[{NAME}] {id}: An extractor error has occurred. (caused by KeyError('fallback_url')); {}",
                        crate::BUG_REPORTS_MESSAGE
                    ))
                })?
                .to_owned();
            let display_id = id.clone();
            let video_id = VIDEO_ID
                .captures(&fallback)
                .map_or_else(|| display_id.clone(), |found| found["id"].to_owned());
            let dash_url = playlist("dash_url")
                .unwrap_or_else(|| format!("https://v.redd.it/{video_id}/DASHPlaylist.mpd"));
            let mut hls_url = playlist("hls_url")
                .unwrap_or_else(|| format!("https://v.redd.it/{video_id}/HLSPlaylist.m3u8"));
            // `f`: the requested variants plus every subtitle track.
            if let Ok(parsed) = Url::parse(&hls_url) {
                let f = parsed
                    .query_pairs()
                    .find(|(key, value)| key == "f" && !value.is_empty())
                    .map_or_else(
                        || "hd,subsAll".to_owned(),
                        |(_, value)| format!("{value},subsAll"),
                    );
                hls_url = super::update_url_query(&parsed, &[("f", &f)]).to_string();
            }
            let mut fallback_format = Map::new();
            fallback_format.insert("url".into(), crate::html::unescape_html(&fallback).into());
            fallback_format.insert("height".into(), int(reddit_video.get("height")));
            fallback_format.insert("width".into(), int(reddit_video.get("width")));
            fallback_format.insert("tbr".into(), int(reddit_video.get("bitrate_kbps")));
            fallback_format.insert("acodec".into(), "none".into());
            fallback_format.insert("vcodec".into(), "h264".into());
            fallback_format.insert("ext".into(), "mp4".into());
            fallback_format.insert("format_id".into(), "fallback".into());
            fallback_format.insert("format_note".into(), "DASH video, mp4_dash".into());
            for key in ["height", "width", "tbr"] {
                if fallback_format.get(key).is_some_and(Value::is_null) {
                    fallback_format.shift_remove(key);
                }
            }
            let mut format: crate::VideoFormat =
                serde_json::from_value(Value::Object(fallback_format))
                    .map_err(|error| Error::Reported(format!("[{NAME}] {id}: {error}")))?;
            crate::info_order::mark_format(&mut format, "reddit-fallback");
            let mut formats = vec![format];
            let mut subtitles = crate::Subtitles::default();
            formats.extend(hls(client, &hls_url, &display_id, Some(&mut subtitles)));
            formats.extend(dash(client, &dash_url, &display_id, Some(&mut subtitles)));
            if subtitles.entries.is_empty() && super::subtitles_requested() {
                // `_get_subtitles`: the fallback caption URL, if valid.
                let caption = format!("https://v.redd.it/{video_id}/wh_ben_en.vtt");
                crate::screen::screen(&format!("[{NAME}] {video_id}: Checking subtitles URL"));
                match Url::parse(&caption)
                    .map_err(Error::from)
                    .and_then(|caption| super::direct::request(client, &caption, None))
                {
                    Ok(_) => {
                        subtitles.entries.push((
                            Some("en".into()),
                            vec![
                                serde_json::from_value(json!({"url": caption}))
                                    .map_err(|error| Error::Reported(error.to_string()))?,
                            ],
                        ));
                    }
                    Err(error) => crate::screen::screen(&format!(
                        "[{NAME}] {video_id}: subtitles URL is invalid, skipping: {}",
                        crate::upstream_error_text(&error)
                    )),
                }
            }
            let mut dict = info;
            dict.insert("id".into(), video_id.into());
            dict.insert("display_id".into(), display_id.into());
            dict.insert("formats".into(), Value::Null);
            dict.insert("subtitles".into(), Value::Null);
            dict.insert("duration".into(), int(reddit_video.get("duration")));
            return Ok(Some(ExtractResult::Video(video(
                url, dict, formats, subtitles,
            ))));
        }

        if netloc == "v.redd.it" {
            return Err(no_formats(&id, "This video is processing"));
        }

        // Not hosted on Reddit: `url_transparent`.
        let mut fields = info;
        fields.insert("display_id".into(), id.clone().into());
        let mut follow = super::direct::build_at(url.clone(), url, String::new());
        follow.formats.clear();
        follow.direct = None;
        follow.extra.insert(
            crate::FOLLOW_TRANSPARENT.into(),
            json!({"url": video_url, "fields": Value::Object(fields)}),
        );
        Ok(Some(ExtractResult::Video(follow)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.reddit.com/r/videos/comments/6rrwyj/that_small_heart_attack/"
        ));
        assert!(suitable("https://old.reddit.com/comments/6rrwyj"));
        assert!(suitable(
            "https://www.redditmedia.com/r/serbia/comments/pu9wbx/ako/"
        ));
        assert!(!suitable("https://www.reddit.com/r/videos/"));
    }

    #[test]
    fn titles_are_truncated_like_truncate_string() {
        assert_eq!(truncate("short"), "short");
        let long = "x".repeat(80);
        assert_eq!(truncate(&long), format!("{}...", "x".repeat(69)));
    }
}

//! Upstream `SoundcloudIE` (`yt_dlp/extractor/soundcloud.py`): the web
//! client ID from the site's JS assets, the resolved track JSON, its
//! requested transcodings' stream URLs, artwork thumbnails and metadata.
use super::Extractor;
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::{LazyLock, Mutex};
use url::Url;

pub struct Soundcloud;

const NAME: &str = "soundcloud";
const KEY: &str = "Soundcloud";
const API_V2_BASE: &str = "https://api-v2.soundcloud.com/";
const DEFAULT_FORMATS: [&str; 6] = [
    "http_aac",
    "hls_aac",
    "http_opus",
    "hls_opus",
    "http_mp3",
    "hls_mp3",
];
const ARTWORK_MAP: [(&str, i64); 10] = [
    ("mini", 16),
    ("tiny", 20),
    ("small", 32),
    ("badge", 47),
    ("t67x67", 67),
    ("large", 100),
    ("t300x300", 300),
    ("crop", 400),
    ("t500x500", 500),
    ("original", 0),
];

static TRACK_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:https?://)?(?:www\.|m\.)?soundcloud\.com/(?P<uploader>[\w\d-]+)/(?P<title>[\w\d-]+)(?:/(?P<token>[^?]+?))?(?:[?].*)?$")
        .unwrap()
});
static API_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:https?://)?api(?:-v2)?\.soundcloud\.com/tracks/(?:soundcloud%3Atracks%3A)?(?P<track_id>\d+)(?:/?\?secret_token=(?P<secret_token>[^&]+))?")
        .unwrap()
});
static SCRIPT_SRC: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<script[^>]+src="([^"]+)""#).unwrap());
static CLIENT_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"client_id\s*:\s*"([0-9a-zA-Z]{32})""#).unwrap());
static IMAGE_REPL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"-[0-9a-z]+\.(?P<ext>jpg|png)").unwrap());
static TAGS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""([^"]+)"|([^ ]+)"#).unwrap());
static CODECS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"codecs="([^"]+)""#).unwrap());
static ABR_PRESET: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d+)k$").unwrap());
static ABR_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\.(\d+)\.(?:opus|mp3)[/?]").unwrap());
static PREVIEW: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"/(?:preview|playlist)/0/30/").unwrap());

/// The `(?!...)` exclusions of the track URL alternative.
fn track_match(url: &str) -> Option<(String, String, Option<String>)> {
    let found = TRACK_URL.captures(url)?;
    let uploader = &found["uploader"];
    let title = &found["title"];
    if uploader == "stations" && title == "track" {
        return None;
    }
    let after_uploader = &url[found.name("title")?.start()..];
    static EXCLUDED: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^(?:tracks|albums|sets(?:/.+?)?|reposts|likes|spotlight|comments)/?(?:$|[?#])")
            .unwrap()
    });
    if EXCLUDED.is_match(after_uploader) {
        return None;
    }
    let token = found.name("token").map(|token| token.as_str().to_owned());
    if token.as_deref().is_some_and(|token| {
        token.starts_with("albums") || token.starts_with("sets") || token.starts_with("recommended")
    }) {
        return None;
    }
    Some((uploader.to_owned(), title.to_owned(), token))
}

/// `SoundcloudIE.suitable`.
pub fn suitable(url: &str) -> bool {
    API_URL.is_match(url) || track_match(url).is_some()
}

fn int(value: Option<&Value>) -> Option<i64> {
    let value = value?;
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|number| number as i64))
        .or_else(|| value.as_str()?.trim().parse().ok())
}

fn fail(id: &str, message: &str) -> Error {
    Error::Reported(format!("[{NAME}] {id}: {message}"))
}

fn read_text(response: reqwest::blocking::Response) -> Result<String> {
    use std::io::Read;
    let mut response = response;
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    Ok(String::from_utf8_lossy(&body).into_owned())
}

/// `_update_client_id`: the first `client_id` in the main page's scripts,
/// last script first.
fn update_client_id(client: &Client) -> Result<String> {
    crate::screen::screen(&format!("[{NAME}] Downloading main page"));
    let home = Url::parse("https://soundcloud.com/")?;
    let webpage = super::download_webpage_quiet(client, &home, NAME, "")?;
    let scripts: Vec<String> = SCRIPT_SRC
        .captures_iter(&webpage)
        .map(|found| crate::html::unescape_html(&found[1]))
        .collect();
    for src in scripts.iter().rev() {
        crate::screen::screen(&format!("[{NAME}] Downloading JS asset"));
        let Ok(script_url) = home.join(src) else {
            continue;
        };
        let script = super::direct::request(client, &script_url, None).and_then(read_text);
        match script {
            Ok(script) => {
                if let Some(found) = CLIENT_ID.captures(&script) {
                    return Ok(found[1].to_owned());
                }
            }
            Err(error) => crate::screen::warning(&format!(
                "[{NAME}] Unable to download webpage: {}",
                crate::upstream_error_text(&error)
            )),
        }
    }
    Err(Error::Reported(format!(
        "[{NAME}] Unable to extract client id; {}",
        crate::BUG_REPORTS_MESSAGE
    )))
}

/// The cached (per run) client ID.
static CLIENT: Mutex<Option<String>> = Mutex::new(None);

fn client_id(client: &Client) -> Result<String> {
    if let Some(id) = CLIENT.lock().ok().and_then(|id| id.clone()) {
        return Ok(id);
    }
    let id = update_client_id(client)?;
    if let Ok(mut slot) = CLIENT.lock() {
        *slot = Some(id.clone());
    }
    Ok(id)
}

/// `_call_api(url, id, note, query)`: retried once with a new client ID
/// after a 401/403.
fn call_api(
    client: &Client,
    url: &str,
    id: &str,
    note: &str,
    query: &[(&str, &str)],
) -> Result<Value> {
    for first in [true, false] {
        let client_id = client_id(client)?;
        let mut updates: Vec<(&str, &str)> = query.to_vec();
        updates.push(("client_id", &client_id));
        let url = super::update_url_query(&Url::parse(url)?, &updates);
        match super::download_json_note(client, &url, NAME, id, note) {
            Ok(value) => return Ok(value),
            Err(Error::ExtractorRequest { source, .. })
                if first
                    && matches!(
                        source.as_ref(),
                        Error::Http(error) if error.status().is_some_and(|status| matches!(status.as_u16(), 401 | 403))
                    ) =>
            {
                if let Ok(mut slot) = CLIENT.lock() {
                    *slot = None;
                }
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!("the second attempt returns")
}

/// `_extract_thumbnails`.
fn thumbnails(client: &Client, info: &Value, track_id: &str) -> Value {
    let artwork = info
        .get("artwork_url")
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none);
    let Some(thumbnail_url) = artwork.clone().or_else(|| {
        info.pointer("/user/avatar_url")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
    }) else {
        return Value::Null;
    };
    let Some(found) = IMAGE_REPL.captures(&thumbnail_url) else {
        return json!([{"url": thumbnail_url}]);
    };
    let original_ext = found["ext"].to_owned();
    let mut thumbnails = Vec::new();
    for (image_id, size) in ARTWORK_MAP {
        let mut ext = if image_id == "original" {
            original_ext.clone()
        } else {
            "jpg".into()
        };
        let replaced = |ext: &str| {
            IMAGE_REPL
                .replace(&thumbnail_url, format!("-{image_id}.{ext}").as_str())
                .into_owned()
        };
        let mut thumbnail = Map::new();
        thumbnail.insert("id".into(), image_id.into());
        thumbnail.insert("url".into(), replaced(&ext).into());
        let mut size = size;
        if image_id == "tiny" && artwork.is_none() {
            size = 18;
        } else if image_id == "original" {
            thumbnail.insert("preference".into(), 10.into());
            // A HEAD request checks the original's extension.
            crate::screen::screen(&format!(
                "[{NAME}] {track_id}: Checking thumbnail extension"
            ));
            let available = Url::parse(&replaced(&ext))
                .ok()
                .is_some_and(|url| super::direct::head_with_headers(client, &url, &[]).is_ok());
            if !available {
                ext = if ext == "png" {
                    "jpg".into()
                } else {
                    "png".into()
                };
                thumbnail.insert("url".into(), replaced(&ext).into());
            }
        }
        if size != 0 {
            thumbnail.insert("width".into(), size.into());
            thumbnail.insert("height".into(), size.into());
        }
        thumbnails.push(Value::Object(thumbnail));
    }
    Value::Array(thumbnails)
}

/// `_extract_info_dict(info, full_title, secret_token)`.
fn info_dict(
    client: &Client,
    info: &Value,
    secret_token: Option<&str>,
    url: &Url,
) -> Result<VideoInfo> {
    let track_id = match info.get("id") {
        Some(Value::Number(number)) => number.to_string(),
        Some(Value::String(text)) => text.clone(),
        _ => {
            return Err(fail(
                "",
                "An extractor error has occurred. (caused by KeyError('id'))",
            ));
        }
    };
    let mut formats = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    let mut has_drm = false;
    let mut query: Vec<(&str, &str)> = Vec::new();
    if let Some(token) = secret_token {
        query.push(("secret_token", token));
    }
    for transcoding in info
        .pointer("/media/transcodings")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let (Some(format_url), Some(preset)) = (
            transcoding
                .get("url")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none),
            transcoding
                .get("preset")
                .and_then(Value::as_str)
                .filter(|preset| !preset.is_empty()),
        ) else {
            continue;
        };
        let preset_base = preset.split('_').next().unwrap_or(preset);
        let mut protocol = transcoding
            .pointer("/format/protocol")
            .and_then(Value::as_str)
            .unwrap_or("http")
            .to_owned();
        if protocol.starts_with("ctr-") || protocol.starts_with("cbc-") {
            has_drm = true;
            continue;
        }
        if protocol == "progressive" {
            protocol = "http".into();
        }
        if protocol != "hls" && format_url.contains("/hls") {
            protocol = "hls".into();
        }
        if protocol == "encrypted-hls" || format_url.contains("/encrypted-hls") {
            protocol = "hls-aes".into();
        }
        let short = format!("{protocol}_{preset_base}");
        if preset_base == "abr" {
            crate::screen::debug(&format!("Skipping broken \"{short}\" format"));
            continue;
        }
        if !DEFAULT_FORMATS.contains(&short.as_str()) {
            crate::screen::debug(&format!("\"{short}\" is not a requested format, skipping"));
            continue;
        }
        let response = match call_api(
            client,
            &format_url,
            &track_id,
            &format!("Downloading {short} format info JSON"),
            &query,
        ) {
            Ok(response) => response,
            Err(Error::ExtractorRequest { source, .. }) if matches!(source.as_ref(), Error::Http(error) if error.status().is_some_and(|status| status.as_u16() == 404)) =>
            {
                crate::screen::warning(&format!("[{NAME}] {track_id}: {short} format not found"));
                continue;
            }
            Err(error) => return Err(error),
        };
        let Some(stream_url) = response
            .get("url")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
        else {
            continue;
        };
        if seen.contains(&stream_url) {
            continue;
        }
        seen.push(stream_url.clone());
        let mime = transcoding
            .pointer("/format/mime_type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let codec = CODECS.captures(mime).map(|found| found[1].to_owned());
        let mut ext = match codec.as_deref().map(|codec| &codec[..codec.len().min(4)]) {
            Some("mp4a") => Some("m4a".to_owned()),
            Some("opus") => Some("opus".to_owned()),
            _ => None,
        }
        .or_else(|| Some(super::direct::mimetype2ext_or(mime, None)).filter(|ext| !ext.is_empty()));
        if ext.as_deref().is_none_or(|ext| ext == "m3u8") {
            ext = Some(preset_base.to_owned());
        }
        let ext = ext.unwrap_or_default();
        let premium = transcoding.get("quality").and_then(Value::as_str) == Some("hq");
        let abr = ABR_PRESET
            .captures(preset)
            .or_else(|| ABR_URL.captures(&stream_url))
            .and_then(|found| found[1].parse::<i64>().ok())
            .or_else(|| (premium && preset.contains("aac")).then_some(256));
        let preview = transcoding.get("snipped").and_then(Value::as_bool) == Some(true)
            || format_url.contains("/preview/")
            || PREVIEW.is_match(&stream_url);
        let mut format_id = format!("{protocol}_{preset}");
        if preview {
            format_id.push_str("_preview");
        }
        let format_value = json!({
            "format_id": format_id,
            "url": stream_url,
            "ext": ext,
            "acodec": codec,
            "vcodec": "none",
            "abr": abr,
            "protocol": if matches!(protocol.as_str(), "hls" | "hls-aes") { "m3u8_native" } else { "http" },
            "container": if ext == "m4a" { Some("m4a_dash") } else { None },
            "quality": if premium { 5 } else if abr.is_some_and(|abr| abr >= 160) { 0 } else { -1 },
            "format_note": if premium { Some("Premium") } else { None },
            "preference": if preview { Some(-10) } else { None },
        });
        let keys: Vec<String> = format_value
            .as_object()
            .map(|format| format.keys().cloned().collect())
            .unwrap_or_default();
        let mut object = format_value.as_object().cloned().unwrap_or_default();
        for key in ["acodec", "abr", "container", "format_note"] {
            if object.get(key).is_some_and(Value::is_null) {
                object.shift_remove(key);
            }
        }
        let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(object))
            .map_err(|error| fail(&track_id, &error.to_string()))?;
        crate::info_order::mark_format(&mut format, &format!("dict:{}", keys.join(",")));
        crate::info_order::mark_int_rates(&mut format);
        formats.push(format);
    }
    if formats.is_empty() {
        if has_drm {
            return Err(fail(&track_id, "This video is DRM protected"));
        }
        if info.get("policy").and_then(Value::as_str) == Some("BLOCK") {
            return Err(fail(
                &track_id,
                "This video is not available from your location due to geo restriction. You might want to use a VPN or a proxy server (with --proxy) to workaround.",
            ));
        }
    }
    let text = |key: &str| info.get(key).and_then(Value::as_str);
    let timestamp =
        |key: &str| text(key).and_then(|date| crate::dates::unified_timestamp(date, false));
    let mut dict = Map::new();
    let mut put = |key: &str, value: Option<Value>| {
        if let Some(value) = value.filter(|value| !value.is_null()) {
            dict.insert(key.to_owned(), value);
        }
    };
    put(
        "uploader",
        info.pointer("/user/username")
            .filter(|v| v.is_string())
            .cloned(),
    );
    put(
        "uploader_id",
        ["id", "permalink"]
            .iter()
            .find_map(|key| match info.pointer(&format!("/user/{key}"))? {
                Value::String(text) => Some(Value::from(text.clone())),
                Value::Number(number) => Some(Value::from(number.to_string())),
                _ => None,
            }),
    );
    put(
        "uploader_url",
        info.pointer("/user/permalink_url")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
            .map(Value::from),
    );
    put("timestamp", timestamp("created_at").map(Value::from));
    put("title", text("title").map(Value::from));
    put("track", text("title").map(Value::from));
    put("description", text("description").map(Value::from));
    put(
        "duration",
        info.get("duration")
            .and_then(Value::as_f64)
            .map(|millis| json!(millis / 1000.0)),
    );
    put(
        "webpage_url",
        text("permalink_url")
            .and_then(super::json_ld::url_or_none)
            .map(Value::from),
    );
    put("license", text("license").map(Value::from));
    put(
        "view_count",
        int(info.get("playback_count")).map(Value::from),
    );
    put(
        "like_count",
        int(info.get("favoritings_count"))
            .or_else(|| int(info.get("likes_count")))
            .map(Value::from),
    );
    put(
        "comment_count",
        int(info.get("comment_count")).map(Value::from),
    );
    put(
        "repost_count",
        int(info.get("reposts_count")).map(Value::from),
    );
    put(
        "release_timestamp",
        timestamp("release_date").map(Value::from),
    );
    put(
        "modified_timestamp",
        timestamp("last_modified").map(Value::from),
    );
    put(
        "genres",
        text("genre")
            .filter(|genre| !genre.is_empty())
            .map(|genre| json!([genre])),
    );
    let tags: Vec<Value> = text("tag_list")
        .map(|tags| {
            TAGS.captures_iter(tags)
                .flat_map(|found| {
                    [found.get(1), found.get(2)]
                        .into_iter()
                        .flatten()
                        .map(|tag| tag.as_str().to_owned())
                        .filter(|tag| !tag.is_empty())
                        .collect::<Vec<_>>()
                })
                .map(Value::from)
                .collect()
        })
        .unwrap_or_default();
    if text("tag_list").is_some() {
        put("tags", Some(Value::Array(tags)));
    }
    put(
        "artists",
        info.pointer("/publisher_metadata/artist")
            .and_then(Value::as_str)
            .filter(|artist| !artist.is_empty())
            .map(|artist| json!([artist])),
    );
    dict.insert("id".into(), track_id.clone().into());
    dict.insert("thumbnails".into(), thumbnails(client, info, &track_id));
    dict.insert("formats".into(), json!([]));
    Ok(video(url, dict, formats))
}

/// A video from upstream's dict (`dict;KEYS` order).
fn video(url: &Url, dict: Map<String, Value>, formats: Vec<crate::VideoFormat>) -> VideoInfo {
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
    info.webpage_url = url.clone();
    info.extractor = NAME.into();
    info.extractor_key = KEY.into();
    let mut keys = Vec::new();
    let mut titled = false;
    for (key, value) in dict {
        keys.push(key.clone());
        match key.as_str() {
            "id" => info.id = value.as_str().unwrap_or_default().to_owned(),
            "title" => {
                titled = true;
                info.title = value.as_str().unwrap_or_default().to_owned();
            }
            "formats" => {}
            "webpage_url" => {
                if let Some(page) = value.as_str().and_then(|page| Url::parse(page).ok()) {
                    info.webpage_url = page;
                }
            }
            _ if value.is_null() => {
                info.extra.insert(key, Value::Null);
            }
            _ => crate::metadata_parser::set_field(&mut info, &key, value),
        }
    }
    if !titled {
        info.extra
            .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
    }
    crate::info_order::mark_info(
        info,
        &format!(
            "dict{};{};thumbnail=id,url,preference,width,height",
            if titled { "" } else { "-untitled" },
            keys.join(",")
        ),
    )
}

impl Extractor for Soundcloud {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let (info_url, full_title, token) = if let Some(found) = API_URL.captures(url.as_str()) {
            let track_id = found["track_id"].to_owned();
            (
                format!("{API_V2_BASE}tracks/{track_id}"),
                track_id,
                found
                    .name("secret_token")
                    .map(|token| token.as_str().to_owned()),
            )
        } else if let Some((uploader, title, token)) = track_match(url.as_str()) {
            let full_title = format!("{uploader}/{title}");
            let resolve = match &token {
                Some(token) => format!("{full_title}/{token}"),
                None => full_title.clone(),
            };
            // `_resolv_url`: the URL as the `url` query value.
            (
                format!("{API_V2_BASE}resolve?url=https://soundcloud.com/{resolve}"),
                full_title,
                token,
            )
        } else {
            return Ok(None);
        };
        crate::screen::set_extractor(NAME);
        client_id(client)?;
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let mut query: Vec<(&str, &str)> = Vec::new();
        if API_URL.is_match(url.as_str())
            && let Some(token) = &token
        {
            query.push(("secret_token", token));
        }
        let info = call_api(
            client,
            &info_url,
            &full_title,
            "Downloading info JSON",
            &query,
        )?;
        info_dict(client, &info, token.as_deref(), url).map(Some)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        Ok(self.extract(client, url)?.map(ExtractResult::Video))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://soundcloud.com/ethmusic/lostin-powers-she-so-heavy"
        ));
        assert!(suitable(
            "https://soundcloud.com/jaimemf/youtube-dl-test-video-a-y-baw/s-8Pjrp"
        ));
        assert!(suitable("https://api.soundcloud.com/tracks/123998367"));
        assert!(suitable(
            "https://api-v2.soundcloud.com/tracks/583011102?secret_token=s-abc"
        ));
        assert!(!suitable("https://soundcloud.com/the-concept-band/sets"));
        assert!(!suitable("https://soundcloud.com/the-concept-band/likes"));
        assert!(!suitable(
            "https://soundcloud.com/the-concept-band/sets/the-concept"
        ));
        assert!(!suitable("https://soundcloud.com/stations/track/foo/bar"));
    }
}

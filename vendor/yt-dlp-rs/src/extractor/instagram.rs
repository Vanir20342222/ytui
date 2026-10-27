//! Upstream `InstagramIE` (`yt_dlp/extractor/instagram.py`) without browser
//! impersonation, like upstream without `curl_cffi`: the session's LSD
//! token, the post accessibility check, then the post page's prefetched
//! `xig_polaris_media` and `_extract_product`.
use super::Extractor;
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Instagram;

const NAME: &str = "Instagram";
const ENCODING_CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
const APP_ID: &str = "936619743392459";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?P<url>https?://(?:www\.)?instagram\.com(?:/[^/?#]+)?/(?:p|tv|reels?)/(?P<id>[^/?#&]+))",
    )
    .unwrap()
});
static SJS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<script\b[^>]+\bdata-sjs>(\{.+?\})</script>").unwrap());
static EQMC: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<script\b[^>]*\bid="__eqmc"[^>]*>"#).unwrap());
static LSD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\["LSD",\[\],\{"token":"([^"]+)""#).unwrap());

/// `_match_valid_url` (`(?!share/)` and `reels?(?!/audio/)` checked apart).
pub fn suitable(url: &str) -> bool {
    let Some(found) = VALID_URL.captures(url) else {
        return false;
    };
    let path = url.split_once("instagram.com").map_or("", |(_, path)| path);
    let after_kind = &url[found.name("id").map_or(0, |id| id.start())..];
    !path.starts_with("/share/")
        && !(path.contains("/reel/audio/") || after_kind.starts_with("audio/"))
}

/// `_id_to_pk`: a shortcode (without a private post's 28 extra characters).
fn id_to_pk(shortcode: &str) -> u128 {
    let shortcode = if shortcode.len() > 28 {
        &shortcode[..shortcode.len() - 28]
    } else {
        shortcode
    };
    shortcode.bytes().fold(0u128, |pk, byte| {
        let digit = ENCODING_CHARS
            .iter()
            .position(|known| *known == byte)
            .unwrap_or(0) as u128;
        pk.wrapping_mul(64).wrapping_add(digit)
    })
}

/// `_pk_to_id`.
fn pk_to_id(pk: &Value) -> Option<String> {
    let text = match pk {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        _ => return None,
    };
    let mut pk: u128 = text.split('_').next()?.parse().ok()?;
    if pk == 0 {
        return Some("A".into());
    }
    let mut digits = Vec::new();
    while pk > 0 {
        digits.push(ENCODING_CHARS[(pk % 64) as usize]);
        pk /= 64;
    }
    digits.reverse();
    String::from_utf8(digits).ok()
}

fn int(value: Option<&Value>) -> Option<i64> {
    let value = value?;
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|number| number as i64))
        .or_else(|| value.as_str()?.trim().parse().ok())
}

fn str_or_none(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

const LOGIN_HINT: &str = "Use --cookies-from-browser or --cookies for the authentication. See  https://github.com/yt-dlp/yt-dlp/wiki/FAQ#how-do-i-pass-cookies-to-yt-dlp  for how to manually pass cookies";

fn fail(id: &str, message: &str) -> Error {
    Error::Reported(format!("[{NAME}] {id}: {message}"))
}

fn login_required(id: &str, message: &str) -> Error {
    fail(id, &format!("{message}. {LOGIN_HINT}"))
}

fn api_headers() -> [(&'static str, &'static str); 5] {
    [
        ("X-IG-App-ID", APP_ID),
        ("X-ASBD-ID", "359341"),
        ("X-IG-WWW-Claim", "0"),
        ("Origin", "https://www.instagram.com"),
        ("Accept", "*/*"),
    ]
}

/// `_is_login_redirect`.
fn login_redirect(url: &Url) -> bool {
    url.path().starts_with("/accounts/login") || url.path() == "/"
}

/// `_real_initialize`: the logged-out session's LSD token.
fn lsd_token(client: &Client) -> Result<String> {
    static TOKEN: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
    if let Some(token) = TOKEN.lock().ok().and_then(|token| token.clone()) {
        return Ok(token);
    }
    crate::screen::screen(&format!("[{NAME}] Setting up session"));
    let home = Url::parse("https://www.instagram.com/")?;
    let webpage = super::download_webpage_quiet(client, &home, NAME, "")?;
    let eqmc = match super::search_json(&EQMC, &webpage, false) {
        Some(Some(value)) => value,
        _ => Value::Null,
    };
    let token = eqmc
        .get("l")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| LSD.captures(&webpage).map(|found| found[1].to_owned()))
        .ok_or_else(|| {
            Error::Reported(format!(
                "[{NAME}] Unable to extract LSD token; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
    if let Ok(mut slot) = TOKEN.lock() {
        *slot = Some(token.clone());
    }
    Ok(token)
}

/// `_extract_product_media`.
fn product_media(media: &Value) -> Map<String, Value> {
    let mut formats = Vec::new();
    for version in media
        .get("video_versions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|version| {
            version
                .get("url")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
                .is_some()
        })
    {
        let mut format = Map::new();
        format.insert(
            "url".into(),
            version.get("url").cloned().unwrap_or(Value::Null),
        );
        // Pinned upstream's `(('id', {str}), ('type', {int}, {str_or_none}),
        // any)` yields a present `id` as it is and never the `type`.
        if let Some(format_id) = version.get("id").filter(|id| !id.is_null()) {
            format.insert("format_id".into(), format_id.clone());
        }
        if let Some(width) = int(version.get("width")) {
            format.insert("width".into(), width.into());
        }
        if let Some(height) = int(version.get("height")) {
            format.insert("height".into(), height.into());
        }
        if let Some(codec) = media.get("video_codec").and_then(Value::as_str) {
            format.insert("vcodec".into(), codec.into());
        }
        if media.get("has_audio") == Some(&Value::Bool(false)) {
            format.insert("acodec".into(), "none".into());
        }
        formats.push(Value::Object(format));
    }
    let thumbnails: Vec<Value> = media
        .pointer("/image_versions2/candidates")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|candidate| {
            candidate
                .get("url")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
                .is_some()
        })
        .map(|candidate| {
            let mut thumbnail = Map::new();
            thumbnail.insert(
                "url".into(),
                candidate.get("url").cloned().unwrap_or(Value::Null),
            );
            if let Some(width) = candidate.get("width").filter(|width| width.is_i64()) {
                thumbnail.insert("width".into(), width.clone());
            }
            if let Some(height) = candidate.get("height") {
                thumbnail.insert("height".into(), height.clone());
            }
            Value::Object(thumbnail)
        })
        .rev()
        .collect();
    let mut info = Map::new();
    info.insert(
        "id".into(),
        media
            .get("pk")
            .and_then(pk_to_id)
            .map_or(Value::Null, Value::from),
    );
    info.insert("formats".into(), Value::Array(formats));
    info.insert(
        "duration".into(),
        media
            .get("video_duration")
            .and_then(|value| value.as_f64().or_else(|| value.as_str()?.parse().ok()))
            .map_or(Value::Null, |duration| json!(duration)),
    );
    info.insert("thumbnails".into(), Value::Array(thumbnails));
    info
}

/// `_extract_product`'s shared `info_dict`.
fn product_info(product: &Value, video_id: &str) -> Map<String, Value> {
    let mut info = Map::new();
    info.insert("id".into(), video_id.into());
    let username = product.pointer("/user/username").and_then(Value::as_str);
    info.insert(
        "title".into(),
        username.map(|name| format!("Video by {name}")).into(),
    );
    if let Some(id) = product.get("pk").and_then(pk_to_id) {
        info.insert("id".into(), id.into());
    }
    if let Some(title) = product.get("title").and_then(Value::as_str) {
        info.insert("title".into(), title.into());
    }
    if let Some(description) = product.pointer("/caption/text").and_then(Value::as_str) {
        info.insert("description".into(), description.into());
    }
    if let Some(channel) = username {
        info.insert("channel".into(), channel.into());
    }
    if let Some(uploader_id) = str_or_none(product.pointer("/user/pk")) {
        info.insert("uploader_id".into(), uploader_id.into());
    }
    if let Some(uploader) = product.pointer("/user/full_name").and_then(Value::as_str) {
        info.insert("uploader".into(), uploader.into());
    }
    for (key, source) in [
        ("timestamp", "taken_at"),
        ("view_count", "view_count"),
        ("like_count", "like_count"),
        ("comment_count", "comment_count"),
    ] {
        if let Some(value) = int(product.get(source)) {
            info.insert(key.into(), value.into());
        }
    }
    info.insert(
        "http_headers".into(),
        json!({"Referer": "https://www.instagram.com/"}),
    );
    info
}

/// A video from upstream's dict (`dict;KEYS` order).
fn video(url: &Url, dict: Map<String, Value>, id: &str) -> Result<VideoInfo> {
    let mut formats = Vec::new();
    for format in dict
        .get("formats")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
    {
        let keys: Vec<String> = format.keys().cloned().collect();
        let mut format = format.clone();
        let ext = format
            .get("url")
            .and_then(Value::as_str)
            .and_then(super::direct::determine_ext)
            .unwrap_or_else(|| "unknown_video".into());
        format.insert("ext".into(), ext.into());
        if !format.contains_key("format_id") {
            format.insert("format_id".into(), "".into());
        }
        let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(format))
            .map_err(|error| fail(id, &error.to_string()))?;
        crate::info_order::mark_format(&mut format, &format!("dict:{}", keys.join(",")));
        formats.push(format);
    }
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
    info.extractor_key = NAME.into();
    let mut keys = Vec::new();
    for (key, value) in dict {
        keys.push(key.clone());
        match key.as_str() {
            "id" => info.id = value.as_str().unwrap_or_default().to_owned(),
            "title" => info.title = value.as_str().unwrap_or_default().to_owned(),
            "formats" => {}
            _ if value.is_null() => {
                info.extra.insert(key, Value::Null);
            }
            _ => crate::metadata_parser::set_field(&mut info, &key, value),
        }
    }
    Ok(crate::info_order::mark_info(
        info,
        &format!("dict;{};thumbnail=url,width,height", keys.join(",")),
    ))
}

/// `traverse_obj(webpage, ({_SJS_RE.findall}, ..., {json.loads}, 'require',
/// ..., ..., ..., '__bbox', 'require', RelayPrefetchedStreamCache, ...,
/// ..., '__bbox', 'result', 'data', 'xig_polaris_media', {dict}, any))`.
fn prefetched_media(webpage: &str) -> Option<Value> {
    fn children(value: &Value) -> Vec<&Value> {
        match value {
            Value::Array(items) => items.iter().collect(),
            Value::Object(object) => object.values().collect(),
            _ => Vec::new(),
        }
    }
    for found in SJS.captures_iter(webpage) {
        let Ok(data) = serde_json::from_str::<Value>(&found[1]) else {
            continue;
        };
        let Some(require) = data.get("require") else {
            continue;
        };
        for a in children(require) {
            for b in children(a) {
                for c in children(b) {
                    let Some(inner) = c.pointer("/__bbox/require") else {
                        continue;
                    };
                    for entry in children(inner) {
                        if entry.get(0).and_then(Value::as_str)
                            != Some("RelayPrefetchedStreamCache")
                        {
                            continue;
                        }
                        for item in children(entry) {
                            for candidate in children(item) {
                                if let Some(media) = candidate
                                    .pointer("/__bbox/result/data/xig_polaris_media")
                                    .filter(|media| media.is_object())
                                {
                                    return Some(media.clone());
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

impl Extractor for Instagram {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        if !suitable(url.as_str()) {
            return Ok(None);
        }
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        let post_url = found["url"].to_owned();
        crate::screen::set_extractor(NAME);
        if crate::cookies::get_cookie("https://www.instagram.com/", "sessionid").is_some() {
            return Err(Error::UnsupportedHtml5(
                "logged-in Instagram extraction (the media info API) is not supported natively"
                    .into(),
            ));
        }
        let _lsd = lsd_token(client)?;
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let media_id = id_to_pk(&video_id).to_string();
        // `get_ruling_for_content` (non-fatal, without an error note).
        let mut ruling =
            Url::parse("https://www.instagram.com/api/v1/web/get_ruling_for_content/")?;
        ruling
            .query_pairs_mut()
            .append_pair("content_type", "MEDIA")
            .append_pair("target_id", &media_id);
        crate::screen::screen(&format!("[{NAME}] {video_id}: Checking post accessibility"));
        let api_check = super::direct::request_with_headers(client, &ruling, None, &api_headers())
            .ok()
            .and_then(|response| {
                use std::io::Read;
                let mut response = response;
                let mut body = Vec::new();
                response.read_to_end(&mut body).ok()?;
                serde_json::from_slice::<Value>(&body).ok()
            })
            .unwrap_or_else(|| json!({}));
        match crate::cookies::get_cookie("https://www.instagram.com", "csrftoken") {
            None => crate::screen::warning(&format!(
                "[{NAME}] {video_id}: No CSRF token set by Instagram API"
            )),
            Some(_) if api_check.get("status").and_then(Value::as_str) != Some("ok") => {
                crate::screen::warning(&format!(
                    "[{NAME}] {video_id}: Instagram API is not granting access"
                ));
            }
            Some(_) => {}
        }
        // The GraphQL request needs impersonation, which is unavailable.
        let error = ["title", "description"]
            .iter()
            .filter_map(|key| api_check.get(*key))
            .filter(|value| match value {
                Value::Null => false,
                Value::String(text) => !text.is_empty(),
                _ => true,
            })
            .map(|value| match value {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            })
            .collect::<Vec<_>>()
            .join(": ");
        if error.contains("Restricted Video") {
            return Err(login_required(&video_id, &error));
        }
        if !error.is_empty() {
            return Err(fail(&video_id, &error));
        }
        if video_id.len() > 28 {
            return Err(login_required(
                &video_id,
                "This content is only available for registered users who follow this account",
            ));
        }
        let page = Url::parse(&format!("https://www.instagram.com/p/{video_id}"))?;
        let (webpage, final_url) = super::download_webpage_handle(client, &page, NAME, &video_id)?;
        if login_redirect(&final_url) {
            return Err(login_required(
                &video_id,
                "The webpage request was redirected to the login page. You have exceeded the rate-limit for accessing posts anonymously",
            ));
        }
        let media = prefetched_media(&webpage);
        let Some(product) = media
            .as_ref()
            .and_then(|media| media.get("if_not_gated_logged_out"))
            .filter(|product| product.is_object())
        else {
            return Err(fail(
                &video_id,
                &format!(
                    "Instagram sent an empty media response. Check if this post is accessible in your browser without being logged-in. If it is not, then u{}. Otherwise, if the post is accessible in browser without being logged-in, {}",
                    &LOGIN_HINT[1..],
                    crate::BUG_REPORTS_MESSAGE
                ),
            ));
        };
        let product = match product {
            Value::Array(items) => items.first().cloned().unwrap_or(Value::Null),
            other => other.clone(),
        };
        let media = media.unwrap_or(Value::Null);
        let comments: Vec<Value> = media
            .pointer("/comments_connection/edges")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|edge| {
                edge.pointer("/node/text")
                    .is_some_and(|text| !text.is_null())
            })
            .filter_map(|edge| edge.get("node"))
            .map(|node| {
                let mut comment = Map::new();
                if let Some(author) = node.pointer("/user/username").and_then(Value::as_str) {
                    comment.insert("author".into(), author.into());
                }
                if let Some(author_id) = str_or_none(node.pointer("/user/pk")) {
                    comment.insert("author_id".into(), author_id.into());
                }
                if let Some(id) = str_or_none(node.get("pk")) {
                    comment.insert("id".into(), id.into());
                }
                if let Some(text) = node.get("text").and_then(Value::as_str) {
                    comment.insert("text".into(), text.into());
                }
                if let Some(timestamp) = int(node.get("created_at")) {
                    comment.insert("timestamp".into(), timestamp.into());
                }
                if let Some(likes) = int(node.get("comment_like_count")) {
                    comment.insert("like_count".into(), likes.into());
                }
                Value::Object(comment)
            })
            .collect();
        let info = product_info(&product, &video_id);
        let post = Url::parse(&post_url).unwrap_or_else(|_| url.clone());
        let _ = post;
        if let Some(carousel) = product
            .get("carousel_media")
            .and_then(Value::as_array)
            .filter(|items| items.iter().any(Value::is_object))
        {
            let mut entries = Vec::new();
            for item in carousel.iter().filter(|item| item.is_object()) {
                let mut dict = info.clone();
                dict.extend(product_media(item));
                dict.insert("comments".into(), Value::Array(comments.clone()));
                entries.push(video(url, dict, &video_id)?);
            }
            // `{'_type': 'playlist', **info_dict, 'title': 'Post by ...'}`.
            let mut extra = info.clone();
            let id = extra
                .shift_remove("id")
                .and_then(|id| id.as_str().map(str::to_owned))
                .unwrap_or_default();
            extra.shift_remove("title");
            let title = info
                .get("channel")
                .and_then(Value::as_str)
                .map(|channel| format!("Post by {channel}"));
            let mut keys: Vec<String> = vec!["_type".into()];
            keys.extend(info.keys().cloned());
            if title.is_none() {
                extra.insert("title".into(), Value::Null);
            }
            extra.insert(
                crate::info_order::INFO_SHAPE.into(),
                format!("playlist-dict;{}", keys.join(",")).into(),
            );
            return Ok(Some(ExtractResult::Playlist(PlaylistInfo {
                id,
                title: title.unwrap_or_default(),
                webpage_url: url.clone(),
                extractor: NAME.into(),
                extractor_key: NAME.into(),
                entries,
                extra,
            })));
        }
        let mut dict = info;
        dict.extend(product_media(&product));
        // `__post_extractor` (None without comments) is popped in processing.
        dict.insert("comments".into(), Value::Array(comments));
        let no_video = dict
            .get("formats")
            .and_then(Value::as_array)
            .is_none_or(|formats| formats.is_empty());
        if no_video {
            return Err(fail(&video_id, "There is no video in this post"));
        }
        Ok(Some(ExtractResult::Video(video(url, dict, &video_id)?)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://instagram.com/p/aye83DjauH/?foo=bar#abc"));
        assert!(suitable("https://www.instagram.com/reel/Chunk8-jurw/"));
        assert!(suitable(
            "https://www.instagram.com/marvelskies.fc/reel/CWqAgUZgCku/"
        ));
        assert!(!suitable("https://www.instagram.com/share/reel/ABC/"));
        assert!(!suitable("https://www.instagram.com/reels/audio/123/"));
        assert!(!suitable("https://www.instagram.com/someuser/"));
    }

    #[test]
    fn shortcodes_and_pks_convert_like_upstream() {
        assert_eq!(id_to_pk("aye83DjauH"), 482584233761418119);
        assert_eq!(
            pk_to_id(&json!(482584233761418119_u64)).as_deref(),
            Some("aye83DjauH")
        );
        assert_eq!(
            pk_to_id(&json!("482584233761418119_1")).as_deref(),
            Some("aye83DjauH")
        );
    }
}

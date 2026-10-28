//! Upstream `ImgurIE`, `ImgurGalleryIE` and `ImgurAlbumIE`
//! (`yt_dlp/extractor/imgur.py`): the post API's media and account data,
//! the `.gifv` page's `<source>`s, GIF and Twitter player formats, and
//! galleries/albums as playlists of `url` results (a gallery with one video
//! is a `url_transparent` result).
use super::Extractor;
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Imgur;
pub struct ImgurGallery;
pub struct ImgurAlbum;

const NAME: &str = "Imgur";
const GALLERY: &str = "imgur:gallery";
const ALBUM: &str = "imgur:album";
const CLIENT_ID: &str = "546c25a59c58ad7";

// `(?!(?:a|gallery|t|topic|r)/)` is checked in `media_id`.
static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:i\.)?imgur\.com/(?P<rest>)(?:[^/?#]+-)?(?P<id>[a-zA-Z0-9]+)").unwrap()
});
static GALLERY_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:i\.)?imgur\.com/(?:gallery|(?:t(?:opic)?|r)/[^/?#]+)/(?:[^/?#]+-)?(?P<id>[a-zA-Z0-9]+)")
        .unwrap()
});
static ALBUM_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:i\.)?imgur\.com/a/(?:[^/?#]+-)?(?P<id>[a-zA-Z0-9]+)").unwrap()
});
static VIDEO_ELEMENTS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?s)<div class="video-elements">(.*?)</div>"#).unwrap());
static SOURCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<source\s+src="(?P<src>[^"]+)"\s+type="(?P<type>[^"]+)""#).unwrap()
});
static VIDEO_ITEM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"var\s+videoItem\s*=").unwrap());

fn media_id(url: &str) -> Option<String> {
    let found = VALID_URL.captures(url)?;
    let rest = &url[found.name("rest").map_or(0, |rest| rest.start())..];
    if ["a/", "gallery/", "t/", "topic/", "r/"]
        .iter()
        .any(|prefix| rest.starts_with(prefix))
    {
        return None;
    }
    Some(found["id"].to_owned())
}

/// `ImgurIE.suitable`.
pub fn suitable(url: &str) -> bool {
    media_id(url).is_some()
}

/// `ImgurGalleryIE.suitable`.
pub fn gallery_suitable(url: &str) -> bool {
    GALLERY_URL.is_match(url)
}

/// `ImgurAlbumIE.suitable`.
pub fn album_suitable(url: &str) -> bool {
    ALBUM_URL.is_match(url)
}

fn int(value: Option<&Value>) -> Option<i64> {
    let value = value?;
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|number| number as i64))
        .or_else(|| value.as_str()?.trim().parse().ok())
}

fn strip(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

/// `ImgurBaseIE.get_description`.
fn description(text: Option<&str>) -> Option<String> {
    let text = text?;
    if text.contains("Discover the magic of the internet at Imgur") || text.is_empty() {
        return None;
    }
    Some(text.to_owned())
}

fn iso8601(value: Option<&Value>) -> Option<i64> {
    crate::dates::parse_iso8601(value?.as_str()?)
}

fn fail(id: &str, extractor: &str, message: &str) -> Error {
    Error::Reported(format!("[{extractor}] {id}: {message}"))
}

/// `_call_api(endpoint, id)`: `https://api.imgur.com/post/v1/ENDPOINT/ID`.
fn api_url(endpoint: &str, id: &str) -> Result<Url> {
    Ok(Url::parse(&format!(
        "https://api.imgur.com/post/v1/{endpoint}/{id}?client_id={CLIENT_ID}&include=media,account"
    ))?)
}

fn start(extractor: &'static str, page: &Url) {
    crate::screen::set_extractor(extractor);
    crate::screen::screen(&format!(
        "[{extractor}] Extracting URL: {}",
        crate::screen::extracting_url(page.as_str())
    ));
}

/// A format dict: its keys in insertion order, `None` values dropped
/// (sanitized away upstream) and `ext` from the URL when unset.
fn format_from(mut format: Map<String, Value>, id: &str) -> Result<crate::VideoFormat> {
    let keys: Vec<String> = format.keys().cloned().collect();
    format.retain(|_, value| !value.is_null());
    if !format.contains_key("ext") {
        let ext = format
            .get("url")
            .and_then(Value::as_str)
            .and_then(super::direct::determine_ext)
            .unwrap_or_else(|| "unknown_video".into());
        format.insert("ext".into(), ext.into());
    }
    if !format.contains_key("format_id") {
        format.insert("format_id".into(), "".into());
    }
    let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(format))
        .map_err(|error| fail(id, NAME, &error.to_string()))?;
    crate::info_order::mark_format(&mut format, &format!("dict:{}", keys.join(",")));
    Ok(format)
}

/// `_og_search_property(f'{media_type}:{p}', webpage, default=None)` sizes.
fn og_size(webpage: &str, media: &str) -> [(&'static str, Option<i64>); 2] {
    ["width", "height"].map(|key| {
        (
            key,
            crate::html::og_search_property(webpage, &format!("{media}:{key}"))
                .and_then(|value| value.trim().parse().ok()),
        )
    })
}

impl Extractor for Imgur {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, page: &Url) -> Result<Option<ExtractResult>> {
        let Some(video_id) = media_id(page.as_str()) else {
            return Ok(None);
        };
        start(NAME, page);
        let data = super::download_json(client, &api_url("media", &video_id)?, NAME, &video_id)?;
        let media = data.pointer("/media/0");
        let animated = media.is_some_and(|media| {
            media.get("type").and_then(Value::as_str) == Some("video")
                || media
                    .pointer("/metadata/is_animated")
                    .is_some_and(|value| !matches!(value, Value::Null | Value::Bool(false)))
        });
        if !animated {
            return Err(fail(
                &video_id,
                NAME,
                &format!("{video_id} is not a video or animated image"),
            ));
        }
        let media = media.cloned().unwrap_or_default();
        // `_download_webpage(..., fatal=False) or ''`.
        let gifv = Url::parse(&format!("https://i.imgur.com/{video_id}.gifv"))?;
        let webpage = match super::download_webpage(client, &gifv, NAME, &video_id) {
            Ok(webpage) => webpage,
            Err(Error::ExtractorRequest { source, .. }) => {
                crate::screen::warning(&format!(
                    "[{NAME}] {video_id}: Unable to download webpage: {}",
                    crate::upstream_error_text(&source)
                ));
                String::new()
            }
            Err(error) => return Err(error),
        };
        let mut formats: Vec<Map<String, Value>> = Vec::new();
        // `traverse_obj(data, ('media', 0, {...}))`: unset keys are omitted.
        let mut media_fmt = Map::new();
        if let Some(url) = media
            .get("url")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
        {
            media_fmt.insert("url".into(), url.into());
        }
        if let Some(ext) = media.get("ext").and_then(Value::as_str) {
            media_fmt.insert("ext".into(), ext.into());
        }
        for (key, source) in [
            ("width", "width"),
            ("height", "height"),
            ("filesize", "size"),
        ] {
            if let Some(number) = int(media.get(source)) {
                media_fmt.insert(key.into(), number.into());
            }
        }
        // `{lambda b: None if b else 'none'}`, also called for a missing key.
        let has_sound = media
            .pointer("/metadata/has_sound")
            .is_some_and(|sound| match sound {
                Value::Null => false,
                Value::Bool(value) => *value,
                Value::Number(number) => number.as_f64() != Some(0.0),
                Value::String(text) => !text.is_empty(),
                Value::Array(items) => !items.is_empty(),
                Value::Object(items) => !items.is_empty(),
            });
        if !has_sound {
            media_fmt.insert("acodec".into(), "none".into());
        }
        if let Some(media_url) = media_fmt
            .get("url")
            .and_then(Value::as_str)
            .map(str::to_owned)
        {
            if media_fmt
                .get("ext")
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
            {
                let ext = media
                    .get("mime_type")
                    .and_then(Value::as_str)
                    .map(|mime| super::direct::mimetype2ext_or(mime, None))
                    .filter(|ext| !ext.is_empty())
                    .or_else(|| super::direct::determine_ext(&media_url));
                media_fmt.insert("ext".into(), ext.map_or(Value::Null, Value::from));
            }
            if media.get("type").and_then(Value::as_str) == Some("image") {
                media_fmt.insert("acodec".into(), "none".into());
                media_fmt.entry("preference").or_insert(json!(-10));
            }
            formats.push(media_fmt);
        }
        if let Some(elements) = VIDEO_ELEMENTS
            .captures(&webpage)
            .and_then(|found| found.get(1))
            .map(|found| found.as_str())
        {
            let mut size = og_size(&webpage, "video");
            if size.iter().all(|(_, value)| value.is_none()) {
                size = og_size(&webpage, "image");
            }
            formats = SOURCE
                .captures_iter(elements)
                .map(|found| {
                    let mime = &found["type"];
                    let mut format = Map::new();
                    format.insert(
                        "format_id".into(),
                        mime.split_once('/').map_or("", |(_, sub)| sub).into(),
                    );
                    format.insert(
                        "url".into(),
                        super::proto_relative(&crate::html::unescape_html(&found["src"])).into(),
                    );
                    format.insert(
                        "ext".into(),
                        super::direct::mimetype2ext_or(mime, None).into(),
                    );
                    for (key, value) in &size {
                        format.insert((*key).into(), (*value).into());
                    }
                    format
                })
                .collect();
            // The original GIF from `var videoItem = {...}`.
            match super::search_json(&VIDEO_ITEM, &webpage, true) {
                None => super::search_warning(NAME, "GIF info"),
                Some(None) => crate::screen::warning(&format!(
                    "[{NAME}] {video_id}: Unable to extract GIF info: Failed to parse JSON"
                )),
                Some(Some(item)) => {
                    let mut gif = Map::new();
                    if let Some(url) = item.get("gifUrl").and_then(Value::as_str) {
                        gif.insert("url".into(), super::proto_relative(url).into());
                    }
                    if let Some(filesize) = int(item.get("size")) {
                        gif.insert("filesize".into(), filesize.into());
                    }
                    if !gif.is_empty() {
                        if !gif.contains_key("url") {
                            return Err(fail(&video_id, NAME, "Missing \"url\" key in result"));
                        }
                        for (key, value) in &size {
                            gif.insert((*key).into(), (*value).into());
                        }
                        gif.insert("format_id".into(), "gif".into());
                        gif.insert("preference".into(), json!(-10));
                        gif.insert("ext".into(), "gif".into());
                        gif.insert("acodec".into(), "none".into());
                        gif.insert("vcodec".into(), "gif".into());
                        gif.insert("container".into(), "gif".into());
                        formats.push(gif);
                    }
                }
            }
        }
        let search = |name: &str| crate::html::html_search_meta(&webpage, name);
        if let Some(stream) = search("twitter:player:stream")
            .as_deref()
            .and_then(super::json_ld::url_or_none)
        {
            let mut format = Map::new();
            format.insert("format_id".into(), "twitter".into());
            format.insert("url".into(), stream.into());
            format.insert(
                "ext".into(),
                search("twitter:player:stream:content_type")
                    .map(|mime| super::direct::mimetype2ext_or(&mime, None))
                    .map_or(Value::Null, Value::from),
            );
            for key in ["width", "height"] {
                format.insert(
                    key.into(),
                    search(&format!("twitter:{key}"))
                        .and_then(|value| value.trim().parse::<i64>().ok())
                        .into(),
                );
            }
            formats.push(format);
        }
        if formats.is_empty() {
            return Err(fail(
                &video_id,
                NAME,
                &format!("No sources found for video {video_id}. Maybe a plain image?"),
            ));
        }
        // `_remove_duplicate_formats`.
        let mut seen = Vec::new();
        formats.retain(|format| {
            let url = format.get("url").cloned().unwrap_or_default();
            if seen.contains(&url) {
                false
            } else {
                seen.push(url);
                true
            }
        });

        let mut dict = Map::new();
        dict.insert(
            "title".into(),
            crate::html::og_search_property(&webpage, "title").map_or(Value::Null, Value::from),
        );
        dict.insert(
            "description".into(),
            description(
                crate::html::og_search_property(&webpage, "description")
                    .as_deref()
                    .or(Some("")),
            )
            .map_or(Value::Null, Value::from),
        );
        let mut set = |key: &str, value: Option<Value>| {
            if let Some(value) = value {
                dict.insert(key.into(), value);
            }
        };
        set(
            "uploader_id",
            data.get("account_id")
                .filter(|id| int(Some(id)).is_some_and(|id| id != 0))
                .map(|id| match id {
                    Value::String(text) => Value::from(text.clone()),
                    other => Value::from(other.to_string()),
                }),
        );
        set(
            "uploader",
            strip(data.pointer("/account/username")).map(Value::from),
        );
        set(
            "uploader_url",
            data.pointer("/account/avatar_url")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
                .map(Value::from),
        );
        for (key, source) in [
            ("like_count", "upvote_count"),
            ("dislike_count", "downvote_count"),
            ("comment_count", "comment_count"),
        ] {
            set(key, int(data.get(source)).map(Value::from));
        }
        set(
            "age_limit",
            data.get("is_mature")
                .filter(|mature| !matches!(mature, Value::Null | Value::Bool(false)))
                .map(|_| json!(18)),
        );
        set(
            "timestamp",
            iso8601(data.get("updated_at"))
                .or_else(|| iso8601(data.get("created_at")))
                .map(Value::from),
        );
        set(
            "release_timestamp",
            iso8601(data.get("created_at")).map(Value::from),
        );
        let metadata = media.get("metadata").cloned().unwrap_or_default();
        set("title", strip(metadata.get("title")).map(Value::from));
        set(
            "description",
            description(metadata.get("description").and_then(Value::as_str)).map(Value::from),
        );
        set(
            "duration",
            metadata
                .get("duration")
                .and_then(|value| value.as_f64().or_else(|| value.as_str()?.parse().ok()))
                .map(|duration| json!(duration)),
        );
        set(
            "timestamp",
            iso8601(metadata.get("updated_at"))
                .or_else(|| iso8601(metadata.get("created_at")))
                .map(Value::from),
        );
        set(
            "release_timestamp",
            iso8601(metadata.get("created_at")).map(Value::from),
        );
        dict.insert("id".into(), video_id.clone().into());
        dict.insert("formats".into(), json!([]));
        let thumbnail = ["thumbnailUrl", "twitter:image", "og:image"]
            .iter()
            .find_map(|name| search(name));
        dict.insert(
            "thumbnails".into(),
            thumbnail.map_or(
                Value::Null,
                |url| json!([{"url": url, "http_headers": {"Accept": "*/*"}}]),
            ),
        );
        dict.insert("http_headers".into(), json!({"Accept": "*/*"}));

        let formats = formats
            .into_iter()
            .map(|format| format_from(format, &video_id))
            .collect::<Result<Vec<_>>>()?;
        let first = formats
            .first()
            .map_or_else(|| page.clone(), |format| format.url.clone());
        let mut info = super::direct::build_at(first.clone(), &first, String::new());
        info.direct = None;
        info.formats = formats;
        info.webpage_url = page.clone();
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        // The dict always has a `title` key, `None` without one.
        if dict.get("title").is_none_or(Value::is_null) {
            info.extra
                .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
        }
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
        Ok(Some(ExtractResult::Video(crate::info_order::mark_info(
            info,
            &format!("dict;{};thumbnail=url,http_headers", keys.join(",")),
        ))))
    }
}

/// `ImgurBaseIE._imgur_result(item_id)`: `url_result(URL, ImgurIE, id)`.
fn imgur_entry(item_id: &str) -> Result<VideoInfo> {
    let target = format!("https://imgur.com/{item_id}");
    let url = Url::parse(&target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    info.id = item_id.to_owned();
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": NAME,
            "extractor": "imgur",
            "entry": {"ie_key": NAME, "id": item_id, "_type": "url", "url": target},
        }),
    );
    Ok(info)
}

/// `ImgurGalleryBaseIE._real_extract`.
fn gallery(
    client: &Client,
    page: &Url,
    extractor: &'static str,
    gallery_id: String,
    single: bool,
) -> Result<Option<ExtractResult>> {
    use std::io::Read;
    start(extractor, page);
    // `_call_api('albums', id, fatal=False, expected_status=404)`.
    crate::screen::screen(&format!(
        "[{extractor}] {gallery_id}: Downloading JSON metadata"
    ));
    let api = api_url("albums", &gallery_id)?;
    let data = match super::direct::request_expecting(client, &api, &[], &[404]) {
        Ok(mut response) => {
            let mut body = Vec::new();
            response
                .read_to_end(&mut body)
                .map_err(Error::ExtractionRead)?;
            match serde_json::from_slice::<Value>(&body) {
                Ok(data) => data,
                Err(error) => {
                    crate::screen::warning(&format!(
                        "[{extractor}] {gallery_id}: Failed to parse JSON  (caused by {error})"
                    ));
                    Value::Null
                }
            }
        }
        Err(error @ (Error::Http(_) | Error::HttpRetryAfter { .. })) => {
            crate::screen::warning(&format!(
                "[{extractor}] {gallery_id}: Unable to download JSON metadata: {}",
                crate::upstream_error_text(&error)
            ));
            Value::Null
        }
        Err(error) => return Err(error),
    };
    let mut info = Map::new();
    if let Some(title) = strip(data.get("title")) {
        info.insert("title".into(), title.into());
    }
    if let Some(text) = description(data.get("description").and_then(Value::as_str)) {
        info.insert("description".into(), text.into());
    }
    let mut target = gallery_id.clone();
    if data
        .get("is_album")
        .is_some_and(|album| !matches!(album, Value::Null | Value::Bool(false)))
    {
        let items: Vec<String> = data
            .get("media")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|item| {
                item.get("type").and_then(Value::as_str) == Some("video")
                    || item
                        .pointer("/metadata/is_animated")
                        .is_some_and(|value| !matches!(value, Value::Null | Value::Bool(false)))
            })
            .filter_map(|item| match item.get("id") {
                Some(Value::String(text)) if !text.is_empty() => Some(text.clone()),
                Some(Value::Number(number)) => Some(number.to_string()),
                _ => None,
            })
            .collect();
        if !(single && items.len() == 1) {
            let entries = items
                .iter()
                .map(|item| imgur_entry(item))
                .collect::<Result<Vec<_>>>()?;
            // `playlist_result(entries, id)` then `result.update(info)`.
            let after: Vec<String> = info.keys().cloned().collect();
            let mut extra = info;
            let title = extra
                .shift_remove("title")
                .and_then(|title| title.as_str().map(str::to_owned))
                .unwrap_or_default();
            extra.insert(
                crate::info_order::INFO_SHAPE.into(),
                format!("playlist-dict;id;after={}", after.join(",")).into(),
            );
            return Ok(Some(ExtractResult::Playlist(PlaylistInfo {
                id: gallery_id,
                title,
                webpage_url: page.clone(),
                extractor: extractor.into(),
                extractor_key: extractor_key(extractor).into(),
                entries,
                extra,
            })));
        }
        target = items[0].clone();
    }
    // `url_result(..., ImgurIE, id)` updated to `url_transparent`.
    let mut follow = super::direct::build_at(page.clone(), page, String::new());
    follow.formats.clear();
    follow.direct = None;
    follow.extra.insert(
        crate::FOLLOW_TRANSPARENT.into(),
        json!({"url": format!("https://imgur.com/{target}"), "fields": Value::Object(info)}),
    );
    Ok(Some(ExtractResult::Video(follow)))
}

fn extractor_key(extractor: &str) -> &'static str {
    if extractor == ALBUM {
        "ImgurAlbum"
    } else {
        "ImgurGallery"
    }
}

impl Extractor for ImgurGallery {
    fn key(&self) -> &'static str {
        GALLERY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, page: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = GALLERY_URL.captures(page.as_str()) else {
            return Ok(None);
        };
        gallery(client, page, GALLERY, found["id"].to_owned(), true)
    }
}

impl Extractor for ImgurAlbum {
    fn key(&self) -> &'static str {
        ALBUM
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, page: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = ALBUM_URL.captures(page.as_str()) else {
            return Ok(None);
        };
        gallery(client, page, ALBUM, found["id"].to_owned(), false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_urls() {
        assert_eq!(
            media_id("https://imgur.com/A61SaA1").as_deref(),
            Some("A61SaA1")
        );
        assert_eq!(
            media_id("https://imgur.com/mrw-gifv-is-up-running-without-any-bugs-A61SaA1")
                .as_deref(),
            Some("A61SaA1")
        );
        assert_eq!(
            media_id("https://i.imgur.com/crGpqCV.mp4").as_deref(),
            Some("crGpqCV")
        );
        assert!(!suitable("https://imgur.com/gallery/YcAQlkx"));
        assert!(!suitable("https://imgur.com/a/iX265HX"));
        assert!(!suitable("http://imgur.com/topic/Aww/ll5Vk"));
        assert!(gallery_suitable("http://imgur.com/topic/Aww/ll5Vk"));
        assert!(gallery_suitable("http://imgur.com/r/aww/VQcQPhM"));
        assert!(gallery_suitable(
            "https://imgur.com/t/unmuted/penguins-penguins-6lAn9VQ"
        ));
        assert!(album_suitable(
            "https://imgur.com/a/enen-no-shouboutai-iX265HX"
        ));
        assert!(!album_suitable("https://imgur.com/gallery/YcAQlkx"));
    }
}

//! Upstream `KickIE`, `KickVODIE` and `KickClipIE`
//! (`yt_dlp/extractor/kick.py`): the site API (requested with upstream's
//! impersonation warning, as impersonation is unavailable here) for live
//! channels, VODs and clips, their HLS (or direct) formats and metadata.
use super::Extractor;
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct KickLive;
pub struct KickVod;
pub struct KickClip;

static LIVE_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://(?:www\.)?kick\.com/(?P<id>[\w-]+)").unwrap());
static VOD_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?kick\.com/[\w-]+/videos/(?P<id>[\da-f]{8}-(?:[\da-f]{4}-){3}[\da-f]{12})")
        .unwrap()
});
static CLIP_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?kick\.com/[\w-]+(?:/clips/|/?\?(?:[^#]+&)?clip=)(?P<id>clip_[\w-]+)",
    )
    .unwrap()
});

/// `KickVODIE.suitable`.
pub fn vod_suitable(url: &str) -> bool {
    VOD_URL.is_match(url)
}

/// `KickClipIE.suitable`.
pub fn clip_suitable(url: &str) -> bool {
    CLIP_URL.is_match(url)
}

/// `KickIE.suitable`: not a VOD or clip, nor a reserved path.
pub fn live_suitable(url: &str) -> bool {
    if vod_suitable(url) || clip_suitable(url) {
        return false;
    }
    let Some(found) = LIVE_URL.captures(url) else {
        return false;
    };
    let id = &found["id"];
    let after = &url[found.get(0).map_or(0, |whole| whole.end())..];
    let reserved = matches!(id, "video" | "categories" | "search" | "auth")
        && (after.is_empty() || after.starts_with(['/', '?', '#']));
    !reserved
}

fn at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    Some(current).filter(|value| !value.is_null())
}

/// `({int}, {str_or_none})`: an int as text.
fn int_text(value: Option<&Value>) -> Option<Value> {
    value
        .filter(|value| value.is_i64() || value.is_u64())
        .map(|value| Value::from(value.to_string()))
}

fn text(value: Option<&Value>) -> Option<Value> {
    value.filter(|value| value.is_string()).cloned()
}

fn int(value: Option<&Value>) -> Option<Value> {
    let value = value?;
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|number| number as i64))
        .or_else(|| value.as_str()?.trim().parse().ok())
        .map(Value::from)
}

fn float(value: Option<&Value>, scale: f64) -> Option<Value> {
    let value = value?;
    value
        .as_f64()
        .or_else(|| value.as_str()?.trim().parse().ok())
        .map(|number| json!(number / scale))
}

fn url(value: Option<&Value>) -> Option<Value> {
    value
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
        .map(Value::from)
}

/// `('is_mature', {bool}, {lambda x: 18 if x else 0})`.
fn age(value: Option<&Value>) -> Option<Value> {
    value
        .and_then(Value::as_bool)
        .map(|mature| Value::from(if mature { 18 } else { 0 }))
}

fn names(list: Option<&Value>) -> Vec<Value> {
    list.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("name").filter(|name| name.is_string()).cloned())
        .collect()
}

fn impersonation_warning(name: &str) {
    crate::screen::warning_once(&format!(
        "[{name}] The extractor is attempting impersonation, but no impersonate target is available. If you encounter errors, then see  https://github.com/yt-dlp/yt-dlp#impersonation  for information on installing the required dependencies"
    ));
}

/// `_call_api(path, display_id)`.
fn call_api(client: &Client, name: &'static str, path: &str, id: &str) -> Result<Value> {
    impersonation_warning(name);
    let api = Url::parse(&format!("https://kick.com/api/{path}"))?;
    let token = crate::cookies::get_cookie("https://kick.com/", "session_token").map(|token| {
        format!(
            "Bearer {}",
            percent_encoding::percent_decode_str(&token).decode_utf8_lossy()
        )
    });
    let headers: Vec<(&str, &str)> = token
        .as_deref()
        .map(|token| vec![("Authorization", token)])
        .unwrap_or_default();
    super::download_json_note_headers(client, &api, name, id, "Downloading API JSON", &headers)
}

/// A video from upstream's dict (`dict;KEYS` order).
fn video(
    page: &Url,
    dict: Map<String, Value>,
    formats: Vec<crate::VideoFormat>,
    name: &str,
    key: &str,
) -> VideoInfo {
    let first = formats
        .first()
        .map_or_else(|| page.clone(), |format| format.url.clone());
    let mut info = super::direct::build_at(first.clone(), &first, String::new());
    info.direct = None;
    if formats.is_empty() {
        info.extra
            .insert(crate::info::NO_FORMATS.into(), Value::Bool(true));
    }
    info.formats = formats;
    info.webpage_url = page.clone();
    info.extractor = name.into();
    info.extractor_key = key.into();
    let titled = dict.get("title").is_some_and(|title| !title.is_null());
    if !titled {
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
    crate::info_order::mark_info(
        info,
        &format!(
            "dict{};{}",
            if titled { "" } else { "-untitled" },
            keys.join(",")
        ),
    )
}

fn announce(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

fn insert_all(dict: &mut Map<String, Value>, fields: Vec<(&str, Option<Value>)>) {
    for (key, value) in fields {
        if let Some(value) = value {
            dict.insert(key.to_owned(), value);
        }
    }
}

fn hls(
    client: &Client,
    name: &'static str,
    source: Option<&Value>,
    id: &str,
) -> Result<Vec<crate::VideoFormat>> {
    let source = source.and_then(Value::as_str).ok_or_else(|| {
        Error::Reported(format!(
            "[{name}] {id}: An extractor error has occurred. (caused by KeyError('source')); {}",
            crate::BUG_REPORTS_MESSAGE
        ))
    })?;
    let manifest = Url::parse(source)?;
    Ok(super::extract_m3u8_fatal(client, &manifest, name, id, None, false)?.formats)
}

impl Extractor for KickVod {
    fn key(&self) -> &'static str {
        "kick:vod"
    }

    fn extract(&self, client: &Client, page: &Url) -> Result<Option<VideoInfo>> {
        const NAME: &str = "kick:vod";
        let Some(found) = VOD_URL.captures(page.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        announce(NAME, page);
        let response = call_api(client, NAME, &format!("v1/video/{id}"), &id)?;
        let formats = hls(client, NAME, response.get("source"), &id)?;
        let mut dict = Map::new();
        dict.insert("id".into(), id.clone().into());
        dict.insert("formats".into(), json!([]));
        let live = response.get("livestream").cloned().unwrap_or(Value::Null);
        let categories = names(at(&live, &["categories"]));
        insert_all(
            &mut dict,
            vec![
                (
                    "title",
                    text(at(&live, &["session_title"])).or_else(|| text(at(&live, &["slug"]))),
                ),
                ("description", text(at(&live, &["channel", "user", "bio"]))),
                ("channel", text(at(&live, &["channel", "slug"]))),
                ("channel_id", int_text(at(&live, &["channel", "id"]))),
                (
                    "uploader",
                    text(at(&live, &["channel", "user", "username"])),
                ),
                ("uploader_id", int_text(at(&live, &["channel", "user_id"]))),
                (
                    "timestamp",
                    response
                        .get("created_at")
                        .and_then(Value::as_str)
                        .and_then(crate::dates::parse_iso8601)
                        .map(Value::from),
                ),
                ("duration", float(at(&live, &["duration"]), 1000.0)),
                ("thumbnail", url(at(&live, &["thumbnail"]))),
                (
                    "categories",
                    // A branching path without results is omitted.
                    (!categories.is_empty()).then_some(Value::Array(categories)),
                ),
                ("view_count", int(response.get("views"))),
                ("age_limit", age(at(&live, &["is_mature"]))),
                (
                    "is_live",
                    at(&live, &["is_live"]).filter(|v| v.is_boolean()).cloned(),
                ),
            ],
        );
        Ok(Some(video(page, dict, formats, NAME, "KickVOD")))
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        Ok(self.extract(client, url)?.map(ExtractResult::Video))
    }
}

impl Extractor for KickClip {
    fn key(&self) -> &'static str {
        "kick:clips"
    }

    fn extract(&self, client: &Client, page: &Url) -> Result<Option<VideoInfo>> {
        const NAME: &str = "kick:clips";
        let Some(found) = CLIP_URL.captures(page.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        announce(NAME, page);
        let response = call_api(client, NAME, &format!("v2/clips/{id}/play"), &id)?;
        let clip = response.get("clip").cloned().unwrap_or(Value::Null);
        let clip_url = clip.get("clip_url").and_then(Value::as_str).ok_or_else(|| {
            Error::Reported(format!(
                "[{NAME}] {id}: An extractor error has occurred. (caused by KeyError('clip_url')); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
        let formats = if super::direct::determine_ext(clip_url).as_deref() == Some("m3u8") {
            hls(client, NAME, clip.get("clip_url"), &id)?
        } else {
            let mut format: crate::VideoFormat = serde_json::from_value(json!({
                "url": clip_url,
                "format_id": "",
                "ext": super::direct::determine_ext(clip_url).unwrap_or_else(|| "unknown_video".into()),
            }))
            .map_err(|error| Error::Reported(error.to_string()))?;
            crate::info_order::mark_format(&mut format, "url-only");
            vec![format]
        };
        let mut dict = Map::new();
        dict.insert("id".into(), id.clone().into());
        dict.insert("formats".into(), json!([]));
        insert_all(
            &mut dict,
            vec![
                ("title", text(clip.get("title"))),
                ("channel", text(at(&clip, &["channel", "slug"]))),
                ("channel_id", int_text(at(&clip, &["channel", "id"]))),
                ("uploader", text(at(&clip, &["creator", "username"]))),
                ("uploader_id", int_text(at(&clip, &["creator", "id"]))),
                ("thumbnail", url(clip.get("thumbnail_url"))),
                ("duration", float(clip.get("duration"), 1.0)),
                (
                    "categories",
                    Some(Value::Array(
                        text(at(&clip, &["category", "name"])).into_iter().collect(),
                    )),
                ),
                (
                    "timestamp",
                    clip.get("created_at")
                        .and_then(Value::as_str)
                        .and_then(crate::dates::parse_iso8601)
                        .map(Value::from),
                ),
                ("view_count", int(clip.get("views"))),
                ("like_count", int(clip.get("likes"))),
                ("age_limit", age(clip.get("is_mature"))),
            ],
        );
        Ok(Some(video(page, dict, formats, NAME, "KickClip")))
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        Ok(self.extract(client, url)?.map(ExtractResult::Video))
    }
}

impl Extractor for KickLive {
    fn key(&self) -> &'static str {
        "kick:live"
    }

    fn extract(&self, client: &Client, page: &Url) -> Result<Option<VideoInfo>> {
        const NAME: &str = "kick:live";
        if !live_suitable(page.as_str()) {
            return Ok(None);
        }
        let Some(found) = LIVE_URL.captures(page.as_str()) else {
            return Ok(None);
        };
        let channel = found["id"].to_owned();
        announce(NAME, page);
        let response = call_api(client, NAME, &format!("v2/channels/{channel}"), &channel)?;
        let live = response
            .get("livestream")
            .filter(|live| live.is_object())
            .cloned();
        let Some(live) = live else {
            return Err(Error::Reported(format!(
                "[{NAME}] {channel}: The channel is not currently live"
            )));
        };
        let playback = response.get("playback_url").and_then(Value::as_str).ok_or_else(|| {
            Error::Reported(format!(
                "[{NAME}] {channel}: An extractor error has occurred. (caused by KeyError('playback_url')); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
        let mut formats =
            super::extract_m3u8_fatal(client, &Url::parse(playback)?, NAME, &channel, None, false)?
                .formats;
        super::hls::live_format_ids(&mut formats, None);
        let mut dict = Map::new();
        dict.insert("channel".into(), channel.clone().into());
        dict.insert("is_live".into(), true.into());
        dict.insert("formats".into(), json!([]));
        let date = |value: Option<&Value>| {
            value
                .and_then(Value::as_str)
                .and_then(|date| crate::dates::unified_timestamp(date, false))
                .map(Value::from)
        };
        insert_all(
            &mut dict,
            vec![
                ("id", text(live.get("slug"))),
                ("title", text(live.get("session_title"))),
                ("description", text(at(&response, &["user", "bio"]))),
                (
                    "channel_id",
                    int_text(response.get("id")).or_else(|| int_text(live.get("channel_id"))),
                ),
                (
                    "uploader",
                    text(response.get("name"))
                        .or_else(|| text(at(&response, &["user", "username"]))),
                ),
                (
                    "uploader_id",
                    int_text(response.get("user_id"))
                        .or_else(|| int_text(at(&response, &["user", "id"]))),
                ),
                ("timestamp", date(live.get("created_at"))),
                ("release_timestamp", date(live.get("start_time"))),
                ("thumbnail", url(at(&live, &["thumbnail", "url"]))),
                (
                    "categories",
                    Some(names(response.get("recent_categories")))
                        .filter(|categories| !categories.is_empty())
                        .map(Value::Array),
                ),
                ("concurrent_view_count", int(live.get("viewer_count"))),
                ("age_limit", age(live.get("is_mature"))),
            ],
        );
        Ok(Some(video(page, dict, formats, NAME, "Kick")))
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
        assert!(vod_suitable(
            "https://kick.com/xqc/videos/8dd97a8d-e17f-48fb-8bc3-565f88dbc9ea"
        ));
        assert!(clip_suitable(
            "https://kick.com/mxddy?clip=clip_01GYXVB5Y8PWAPWCWMSBCFB05X"
        ));
        assert!(clip_suitable(
            "https://kick.com/destiny/clips/clip_01H9SKET879NE7N9RJRRDS98J3"
        ));
        assert!(live_suitable("https://kick.com/buddha"));
        assert!(!live_suitable("https://kick.com/video/abc"));
        assert!(!live_suitable(
            "https://kick.com/destiny/clips/clip_01H9SKET879NE7N9RJRRDS98J3"
        ));
        assert!(!live_suitable("https://kick.com/categories"));
    }
}

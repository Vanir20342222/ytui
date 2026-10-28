//! Upstream `CoubIE` (`yt_dlp/extractor/coub.py`): the v2 coub API's HTML5
//! video/audio, iPhone and mobile-audio files with upstream's quality and
//! source preferences, and its metadata; `coub:ID` pseudo-URLs too.
use super::Extractor;
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Coub;

const NAME: &str = "Coub";
const QUALITIES: [&str; 4] = ["low", "med", "high", "higher"];
const SOURCES: [&str; 3] = ["mobile", "iphone", "html5"];

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:coub:|https?://(?:coub\.com/(?:view|embed|coubs)/|c-cdn\.coub\.com/fb-player\.swf\?.*\bcoub(?:ID|id)=))(?P<id>[\da-z]+)")
        .unwrap()
});

/// `CoubIE.suitable`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn fail(id: &str, message: &str) -> Error {
    Error::Reported(format!("[{NAME}] {id}: {message}"))
}

fn key_error(id: &str, key: &str) -> Error {
    fail(
        id,
        &format!(
            "An extractor error has occurred. (caused by KeyError('{key}')); {}",
            crate::BUG_REPORTS_MESSAGE
        ),
    )
}

/// `qualities(list)(value)`.
fn rank(list: &[&str], value: &str) -> i64 {
    list.iter()
        .position(|item| *item == value)
        .map_or(-1, |index| index as i64)
}

fn int(value: Option<&Value>) -> Option<i64> {
    let value = value?;
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|number| number as i64))
        .or_else(|| value.as_str()?.trim().parse().ok())
}

fn truthy(value: Option<&Value>) -> Option<&Value> {
    value.filter(|value| match value {
        Value::Null | Value::Bool(false) => false,
        Value::Number(number) => number.as_f64() != Some(0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(items) => !items.is_empty(),
        Value::Bool(true) => true,
    })
}

fn format_from(mut map: Map<String, Value>, id: &str) -> Result<crate::VideoFormat> {
    let keys: Vec<String> = map.keys().cloned().collect();
    map.retain(|_, value| !value.is_null());
    let ext = map
        .get("url")
        .and_then(Value::as_str)
        .and_then(super::direct::determine_ext)
        .unwrap_or_else(|| "unknown_video".into());
    map.insert("ext".into(), ext.into());
    let mut format: crate::VideoFormat =
        serde_json::from_value(Value::Object(map)).map_err(|error| fail(id, &error.to_string()))?;
    crate::info_order::mark_format(&mut format, &format!("dict:{}", keys.join(",")));
    crate::info_order::mark_int_quality(&mut format);
    Ok(format)
}

impl Extractor for Coub {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, page: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(page.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(page.as_str())
        ));
        let api = Url::parse(&format!("http://coub.com/api/v2/coubs/{video_id}.json"))?;
        let coub = super::download_json(client, &api, NAME, &video_id)?;
        if let Some(error) = truthy(coub.get("error")) {
            let text = match error {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            };
            return Err(fail(&video_id, &format!("{NAME} said: {text}")));
        }
        let title = coub
            .get("title")
            .ok_or_else(|| key_error(&video_id, "title"))?;
        let versions = coub
            .get("file_versions")
            .ok_or_else(|| key_error(&video_id, "file_versions"))?;
        let mut formats = Vec::new();
        for (kind, items) in versions
            .get("html5")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            if kind != "video" && kind != "audio" {
                continue;
            }
            for (quality, item) in items.as_object().into_iter().flatten() {
                let Some(url) = truthy(item.get("url")).and_then(Value::as_str) else {
                    continue;
                };
                let mut map = Map::new();
                map.insert("url".into(), url.into());
                map.insert("format_id".into(), format!("html5-{kind}-{quality}").into());
                map.insert("filesize".into(), int(item.get("size")).into());
                map.insert(
                    "vcodec".into(),
                    if kind == "audio" {
                        "none".into()
                    } else {
                        Value::Null
                    },
                );
                map.insert(
                    "acodec".into(),
                    if kind == "video" {
                        "none".into()
                    } else {
                        Value::Null
                    },
                );
                map.insert("quality".into(), rank(&QUALITIES, quality).into());
                map.insert("source_preference".into(), rank(&SOURCES, "html5").into());
                formats.push(format_from(map, &video_id)?);
            }
        }
        if let Some(url) = truthy(versions.pointer("/iphone/url")).and_then(Value::as_str) {
            let mut map = Map::new();
            map.insert("url".into(), url.into());
            map.insert("format_id".into(), "iphone".into());
            map.insert("source_preference".into(), rank(&SOURCES, "iphone").into());
            formats.push(format_from(map, &video_id)?);
        }
        if let Some(url) = truthy(versions.pointer("/mobile/audio_url")).and_then(Value::as_str) {
            let mut map = Map::new();
            map.insert("url".into(), url.into());
            map.insert("format_id".into(), "mobile-audio".into());
            map.insert("source_preference".into(), rank(&SOURCES, "mobile").into());
            formats.push(format_from(map, &video_id)?);
        }
        let age = coub
            .get("age_restricted")
            .or_else(|| coub.get("age_restricted_by_admin"))
            .filter(|value| !value.is_null())
            .map(|value| if value == &Value::Bool(true) { 18 } else { 0 });
        let timestamp = truthy(coub.get("published_at"))
            .or_else(|| coub.get("created_at"))
            .and_then(Value::as_str)
            .and_then(crate::dates::parse_iso8601);
        let mut dict = Map::new();
        dict.insert("id".into(), video_id.clone().into());
        dict.insert("title".into(), title.clone());
        dict.insert(
            "thumbnail".into(),
            coub.get("picture").cloned().unwrap_or(Value::Null),
        );
        dict.insert(
            "duration".into(),
            coub.get("duration")
                .and_then(|value| {
                    value
                        .as_f64()
                        .or_else(|| value.as_str()?.trim().parse().ok())
                })
                .map_or(Value::Null, |duration| json!(duration)),
        );
        dict.insert("timestamp".into(), timestamp.into());
        dict.insert(
            "uploader".into(),
            coub.pointer("/channel/title")
                .cloned()
                .unwrap_or(Value::Null),
        );
        dict.insert(
            "uploader_id".into(),
            coub.pointer("/channel/permalink")
                .cloned()
                .unwrap_or(Value::Null),
        );
        dict.insert(
            "view_count".into(),
            int(truthy(coub.get("views_count")).or_else(|| coub.get("views_increase_count")))
                .into(),
        );
        dict.insert("like_count".into(), int(coub.get("likes_count")).into());
        dict.insert("repost_count".into(), int(coub.get("recoubs_count")).into());
        dict.insert("age_limit".into(), age.into());
        dict.insert("formats".into(), json!([]));

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
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        if !title.is_string() {
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
            &format!("dict;{}", keys.join(",")),
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("http://coub.com/view/5u5n1"));
        assert!(suitable("coub:5u5n1"));
        assert!(suitable(
            "http://c-cdn.coub.com/fb-player.swf?bot_type=vk&coubID=7w5a4"
        ));
        assert!(suitable("https://coub.com/coubs/5u5n1"));
        assert!(!suitable("https://coub.com/tags/cats"));
    }
}

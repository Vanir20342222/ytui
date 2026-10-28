//! Upstream `GodResourceIE` (`yt_dlp/extractor/godresource.py`): the streams
//! API's VOD HLS or MP4 URL with its traversed metadata.
use super::{Extractor, download_json, extract_m3u8_fatal};
use crate::{Error, Result, VideoFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct GodResource;

const NAME: &str = "GodResource";

static VALID_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://new\.godresource\.com/video/(?P<id>\w+)").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// Python truthiness of a JSON value.
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(number) => number.as_f64() != Some(0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(object) => !object.is_empty(),
    }
}

impl Extractor for GodResource {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let api = Url::parse(&format!("https://api.godresource.com/api/Streams/{id}"))?;
        let data = download_json(client, &api, NAME, &id)?;
        let media = data.get("streamUrl").ok_or_else(|| {
            Error::Reported(format!(
                "{id}: An extractor error has occurred. (caused by KeyError('streamUrl')); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
        let Some(media) = media.as_str() else {
            return Err(Error::UnsupportedHtml5(
                "GodResource stream URLs that are not strings are not supported natively".into(),
            ));
        };
        // `api_data.get('isLive') or False`.
        let is_live = data.get("isLive").filter(|live| truthy(live)).cloned();
        if is_live.is_some() {
            return Err(Error::UnsupportedHtml5(
                "GodResource live streams are not supported natively".into(),
            ));
        }
        let ext = super::direct::determine_ext(media).unwrap_or_else(|| "unknown_video".into());
        let media = Url::parse(media)?;
        let mut info = match ext.as_str() {
            "m3u8" => extract_m3u8_fatal(client, &media, NAME, &id, None, true)?,
            "mp4" => {
                let mut format: VideoFormat = serde_json::from_value(json!({
                    "format_id": "",
                    "url": media.as_str(),
                    "ext": "mp4",
                }))
                .map_err(|error| Error::Reported(format!("[{NAME}] {id}: {error}")))?;
                crate::info_order::mark_format(&mut format, "url-ext");
                let mut info = super::direct::build_at(media.clone(), &media, "mp4".into());
                info.formats = vec![format];
                info
            }
            other => {
                return Err(Error::Reported(format!(
                    "[{NAME}] {id}: Unexpected video format {other}; {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        info.direct = None;
        info.id = id;
        // `'title': ''` unless the API has a string title.
        info.title = data
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        crate::metadata_parser::set_field(&mut info, "is_live", Value::Bool(false));
        info.metadata.thumbnail = data
            .get("thumbnail")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none);
        let timestamp = |key: &str| {
            data.get(key)
                .and_then(Value::as_str)
                .and_then(|date| crate::dates::unified_timestamp(date, true))
        };
        let channel_id = match data.get("channelId") {
            Some(Value::String(text)) => Some(text.clone()),
            Some(Value::Number(number)) => Some(number.to_string()),
            _ => None,
        };
        for (key, value) in [
            (
                "view_count",
                data.get("views")
                    .filter(|views| views.is_i64() || views.is_u64())
                    .cloned(),
            ),
            (
                "channel",
                data.get("channelName")
                    .filter(|name| name.is_string())
                    .cloned(),
            ),
            ("channel_id", channel_id.map(Value::from)),
            ("timestamp", timestamp("streamDateCreated").map(Value::from)),
            (
                "modified_timestamp",
                timestamp("streamDataModified").map(Value::from),
            ),
        ] {
            if let Some(value) = value {
                crate::metadata_parser::set_field(&mut info, key, value);
            }
        }
        Ok(Some(crate::info_order::mark_info(info, "godresource")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://new.godresource.com/video/A01mTKjyf6w"));
        assert!(!suitable("https://godresource.com/video/x"));
    }
}

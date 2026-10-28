//! Upstream `ZhihuIE` (`yt_dlp/extractor/zhihu.py`): the zvideos API's
//! playlist qualities as formats with the author and counts.
use super::{Extractor, download_json};
use crate::{Error, Result, VideoFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Zhihu;

const NAME: &str = "Zhihu";

static VALID_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://(?:www\.)?zhihu\.com/zvideo/(?P<id>[0-9]+)").unwrap());

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

/// `value.get(key) or {}`.
fn object(value: Option<&Value>) -> Result<Option<&serde_json::Map<String, Value>>> {
    match value {
        Some(Value::Object(object)) => Ok(Some(object)),
        Some(value) if truthy(value) => Err(Error::UnsupportedHtml5(
            "Zhihu values that are not objects are not supported natively".into(),
        )),
        _ => Ok(None),
    }
}

/// `float_or_none(value)`.
fn float_or_none(value: Option<&Value>) -> Value {
    match value {
        Some(Value::Number(number)) => number.as_f64().map_or(Value::Null, Value::from),
        Some(Value::String(text)) => text.trim().parse::<f64>().map_or(Value::Null, Value::from),
        _ => Value::Null,
    }
}

impl Extractor for Zhihu {
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
        let api = Url::parse(&format!("https://www.zhihu.com/api/v4/zvideos/{id}"))?;
        let zvideo = download_json(client, &api, NAME, &id)?;
        let title = zvideo
            .get("title")
            .ok_or_else(|| {
                Error::Reported(format!(
                    "{id}: An extractor error has occurred. (caused by KeyError('title')); {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?
            .as_str()
            .ok_or_else(|| {
                Error::UnsupportedHtml5(
                    "Zhihu titles that are not strings are not supported natively".into(),
                )
            })?
            .to_owned();
        let video = object(zvideo.get("video"))?;
        let int = super::streamable::int_or_none;
        let mut formats = Vec::new();
        for (format_id, quality) in object(video.and_then(|video| video.get("playlist")))?
            .into_iter()
            .flatten()
        {
            let get = |key: &str| quality.get(key);
            // `q.get('url') or q.get('play_url')`.
            let Some(play_url) = [get("url"), get("play_url")]
                .into_iter()
                .flatten()
                .find(|value| truthy(value))
            else {
                continue;
            };
            let Some(play_url) = play_url.as_str() else {
                return Err(Error::UnsupportedHtml5(
                    "Zhihu play URLs that are not strings are not supported natively".into(),
                ));
            };
            let play_url = Url::parse(play_url)?;
            let ext = super::direct::determine_ext(play_url.as_str())
                .unwrap_or_else(|| "unknown_video".into());
            let mut format: VideoFormat = serde_json::from_value(json!({
                "asr": int(get("sample_rate")),
                "filesize": int(get("size")),
                "format_id": format_id,
                "fps": int(get("fps")),
                "height": int(get("height")),
                "tbr": float_or_none(get("bitrate")),
                "url": play_url.as_str(),
                "width": int(get("width")),
                "ext": ext,
            }))
            .map_err(|error| Error::Reported(format!("[{NAME}] {id}: {error}")))?;
            crate::info_order::mark_format(&mut format, "zhihu");
            formats.push(format);
        }
        if formats.is_empty() {
            return Err(Error::Reported(format!(
                "[{NAME}] {id}: No video formats found!; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        }
        let author = object(zvideo.get("author"))?;
        let author_field = |key: &str| {
            author
                .and_then(|author| author.get(key))
                .cloned()
                .unwrap_or(Value::Null)
        };
        // `format_field(url_token, None, 'https://www.zhihu.com/people/%s')`.
        let uploader_url = match author_field("url_token") {
            value if !truthy(&value) => Value::from(""),
            Value::String(token) => format!("https://www.zhihu.com/people/{token}").into(),
            Value::Number(token) => format!("https://www.zhihu.com/people/{token}").into(),
            _ => {
                return Err(Error::UnsupportedHtml5(
                    "Zhihu URL tokens that are not strings are not supported natively".into(),
                ));
            }
        };
        // `video.get('thumbnail') or zvideo.get('image_url')`.
        let thumbnail = [
            video.and_then(|video| video.get("thumbnail")),
            zvideo.get("image_url"),
        ]
        .into_iter()
        .flatten()
        .find(|value| truthy(value))
        .cloned()
        .unwrap_or(Value::Null);
        let first = formats[0].url.clone();
        let mut info = super::direct::build_at(first.clone(), &first, String::new());
        info.formats = formats;
        info.direct = None;
        info.id = id;
        info.title = title;
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        for (key, value) in [
            ("thumbnail", thumbnail),
            ("uploader", author_field("name")),
            ("timestamp", int(zvideo.get("published_at"))),
            ("uploader_id", author_field("id")),
            ("uploader_url", uploader_url),
            ("view_count", int(zvideo.get("play_count"))),
            ("like_count", int(zvideo.get("liked_count"))),
            ("comment_count", int(zvideo.get("comment_count"))),
        ] {
            if value.is_null() {
                info.extra.insert(key.into(), Value::Null);
            } else {
                crate::metadata_parser::set_field(&mut info, key, value);
            }
        }
        // Processing drops a `None` duration.
        let duration = float_or_none(video.and_then(|video| video.get("duration")));
        if !duration.is_null() {
            crate::metadata_parser::set_field(&mut info, "duration", duration);
        }
        Ok(Some(crate::info_order::mark_info(info, "zhihu")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://www.zhihu.com/zvideo/1342930761977176064"));
        assert!(!suitable("https://www.zhihu.com/question/1"));
    }
}

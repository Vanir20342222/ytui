//! Upstream `HitRecordIE` (`yt_dlp/extractor/hitrecord.py`): the records
//! API's MP4 URL and metadata as an untyped result.
use super::{Extractor, download_json};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct HitRecord;

const NAME: &str = "HitRecord";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?hitrecord\.org/records/(?P<id>\d+)").unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `value[key]`'s escaped `KeyError`.
fn key<'a>(value: &'a Value, name: &str, id: &str) -> Result<&'a Value> {
    match value {
        Value::Object(object) => object.get(name).ok_or_else(|| {
            Error::Reported(format!(
                "{id}: An extractor error has occurred. (caused by KeyError('{name}')); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        }),
        _ => Err(Error::UnsupportedHtml5(
            "HitRecord values that are not objects are not supported natively".into(),
        )),
    }
}

/// Python `str(value)` for JSON scalars.
fn python_str(value: &Value) -> Option<String> {
    match value {
        Value::Null => Some("None".into()),
        Value::Bool(true) => Some("True".into()),
        Value::Bool(false) => Some("False".into()),
        Value::Number(number) => Some(number.to_string()),
        Value::String(text) => Some(text.clone()),
        _ => None,
    }
}

impl Extractor for HitRecord {
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
        let api = Url::parse(&format!("https://hitrecord.org/api/web/records/{id}"))?;
        let video = download_json(client, &api, NAME, &id)?;
        let unsupported = |what: &str| {
            Error::UnsupportedHtml5(format!(
                "HitRecord {what} values that are not strings are not supported natively"
            ))
        };
        let title = key(&video, "title", &id)?
            .as_str()
            .ok_or_else(|| unsupported("title"))?
            .to_owned();
        let media = key(key(&video, "source_url", &id)?, "mp4_url", &id)?
            .as_str()
            .ok_or_else(|| unsupported("mp4_url"))?;
        let media = Url::parse(media)?;
        // `try_get(video, lambda x: x['tags'], list)`: string texts.
        let tags = video
            .get("tags")
            .and_then(Value::as_array)
            .filter(|tags| !tags.is_empty())
            .map(|tags| {
                Value::Array(
                    tags.iter()
                        .filter_map(|tag| tag.get("text")?.as_str())
                        .filter(|text| !text.is_empty())
                        .map(Value::from)
                        .collect(),
                )
            })
            .unwrap_or(Value::Null);
        let user = video.get("user");
        let uploader = user
            .and_then(|user| user.get("username"))
            .filter(|name| name.is_string())
            .cloned()
            .unwrap_or(Value::Null);
        // `try_get(video, lambda x: str(x['user']['id']))`.
        let uploader_id = match user.and_then(|user| user.as_object()?.get("id")) {
            Some(value) => python_str(value)
                .map(Value::from)
                .ok_or_else(|| unsupported("user ID"))?,
            None => Value::Null,
        };
        let ext =
            super::direct::determine_ext(media.as_str()).unwrap_or_else(|| "unknown_video".into());
        let mut info = super::direct::with_format(
            super::direct::build_at(media.clone(), &media, ext.clone()),
            "",
        );
        for format in &mut info.formats {
            format.ext = ext.clone();
            format.format_id = String::new();
        }
        info.format_id = None;
        info.id = id;
        info.title = title;
        info.webpage_url = url.clone();
        info.direct = None;
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        info.metadata.description = video
            .get("body")
            .and_then(Value::as_str)
            .map(crate::html::clean_html);
        // `float_or_none(duration, 1000)`; processing drops `None`.
        let duration = match video.get("duration") {
            Some(Value::Number(number)) => number.as_f64(),
            Some(Value::String(text)) => text.trim().parse::<f64>().ok(),
            _ => None,
        };
        if let Some(duration) = duration {
            crate::metadata_parser::set_field(&mut info, "duration", (duration / 1000.0).into());
        }
        let int = |name: &str| super::streamable::int_or_none(video.get(name));
        for (name, value) in [
            ("timestamp", int("created_at_i")),
            ("uploader", uploader),
            ("uploader_id", uploader_id),
            ("view_count", int("total_views_count")),
            ("like_count", int("hearts_count")),
            ("comment_count", int("comments_count")),
            ("tags", tags),
        ] {
            if value.is_null() {
                info.extra.insert(name.into(), Value::Null);
            } else {
                crate::metadata_parser::set_field(&mut info, name, value);
            }
        }
        Ok(Some(crate::info_order::mark_info(info, "hitrecord")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://hitrecord.org/records/2954362"));
        assert!(!suitable("https://hitrecord.org/records/"));
    }
}

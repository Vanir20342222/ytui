//! Upstream `LivestreamfailsIE` (`yt_dlp/extractor/livestreamfails.py`):
//! the clip API JSON as an untyped result.
use super::{Extractor, download_json};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Livestreamfails;

const NAME: &str = "Livestreamfails";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?livestreamfails\.com/(?:clip|post)/(?P<id>[0-9]+)").unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// Python's `%s` of a JSON value.
fn py_str(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(value) => Some(if *value { "True" } else { "False" }.into()),
        _ => None,
    }
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!(
        "non-string Livestreamfails {what} values are not supported natively"
    ))
}

impl Extractor for Livestreamfails {
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
        let api = Url::parse(&format!("https://api.livestreamfails.com/clip/{id}"))?;
        let clip = download_json(client, &api, NAME, &id)?;
        let timestamp = match clip.get("createdAt") {
            None | Some(Value::Null) => Value::Null,
            Some(Value::String(date)) => {
                crate::dates::unified_timestamp(date, true).map_or(Value::Null, Value::from)
            }
            Some(_) => return Err(unsupported("createdAt")),
        };
        let video = match clip.get("videoId") {
            Some(value) => py_str(value).ok_or_else(|| unsupported("videoId"))?,
            None => {
                return Err(Error::Reported(format!(
                    "{id}: An extractor error has occurred. (caused by KeyError('videoId')); {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        let title = match clip.get("label") {
            None | Some(Value::Null) => None,
            Some(Value::String(title)) => Some(title.clone()),
            Some(_) => return Err(unsupported("label")),
        };
        // `format_field(api_response, 'imageId', TEMPLATE)`: `''` without one.
        let thumbnail = match clip.get("imageId") {
            None | Some(Value::Null) => String::new(),
            Some(Value::String(image)) if image.is_empty() => String::new(),
            Some(value) => format!(
                "https://livestreamfails-image-prod.b-cdn.net/image/{}",
                py_str(value).ok_or_else(|| unsupported("imageId"))?
            ),
        };
        let media = Url::parse(&format!(
            "https://livestreamfails-video-prod.b-cdn.net/video/{video}"
        ))?;
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
        match title {
            Some(title) => info.title = title,
            None => {
                info.title = String::new();
                info.extra
                    .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
            }
        }
        info.webpage_url = url.clone();
        info.direct = None;
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        info.metadata.thumbnail = Some(thumbnail);
        info.extra.insert(
            "display_id".into(),
            clip.get("sourceId").cloned().unwrap_or(Value::Null),
        );
        info.extra.insert(
            "creator".into(),
            clip.get("streamer")
                .and_then(|streamer| streamer.get("label"))
                .cloned()
                .unwrap_or(Value::Null),
        );
        if !timestamp.is_null() {
            crate::metadata_parser::set_field(&mut info, "timestamp", timestamp);
        }
        Ok(Some(crate::info_order::mark_info(info, "livestreamfails")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://livestreamfails.com/clip/39084"));
        assert!(suitable("https://www.livestreamfails.com/post/2"));
        assert!(!suitable("https://livestreamfails.com/user/2"));
    }
}

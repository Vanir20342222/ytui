//! Upstream `ParlviewIE` (`yt_dlp/extractor/parlview.py`): the ParlView
//! API's video file as fatal HLS formats with its traversed metadata.
use super::{Extractor, download_json, extract_m3u8_fatal};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Parlview;

const NAME: &str = "Parlview";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?aph\.gov\.au/News_and_Events/Watch_Read_Listen/ParlView/video/(?P<id>[^/?#]+)",
    )
    .unwrap()
});
static DURATION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?P<duration>\d+:\d+:\d+):\d+$").unwrap());

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
            "ParlView values that are not objects are not supported natively".into(),
        )),
    }
}

impl Extractor for Parlview {
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
        let api = Url::parse(&format!(
            "https://vodapi.aph.gov.au/api/search/parlview/{id}"
        ))?;
        let data = download_json(client, &api, NAME, &id)?;
        let details = key(&data, "videoDetails", &id)?;
        let file = key(key(details, "files", &id)?, "file", &id)?;
        let manifest = key(file, "url", &id)?.as_str().ok_or_else(|| {
            Error::UnsupportedHtml5(
                "ParlView file URLs that are not strings are not supported natively".into(),
            )
        })?;
        let manifest = Url::parse(manifest)?;
        let mut info = extract_m3u8_fatal(client, &manifest, NAME, &id, None, true)?;
        let text = |value: Option<&Value>| value.and_then(Value::as_str).map(str::to_owned);
        // `(('parlViewTitle', 'title'), {str}, any)`.
        let title = [details.get("parlViewTitle"), details.get("title")]
            .into_iter()
            .find_map(text);
        let duration = text(file.get("duration"))
            .and_then(|duration| {
                DURATION
                    .captures(&duration)
                    .map(|found| found["duration"].to_owned())
            })
            .and_then(|duration| crate::parse_duration(&duration));
        let timestamp =
            text(details.get("recordingFrom")).and_then(|date| crate::dates::parse_iso8601(&date));
        let shape = if title.is_some() {
            "parlview"
        } else {
            "parlview-untitled"
        };
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
        info.metadata.description = text(details.get("parlViewDescription"));
        info.metadata.duration = duration;
        info.metadata.thumbnail = text(details.get("thumbUrl"))
            .and_then(|thumbnail| super::json_ld::url_or_none(&thumbnail));
        if let Some(timestamp) = timestamp {
            crate::metadata_parser::set_field(&mut info, "timestamp", timestamp.into());
        }
        Ok(Some(crate::info_order::mark_info(info, shape)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.aph.gov.au/News_and_Events/Watch_Read_Listen/ParlView/video/3406614"
        ));
        assert!(!suitable("https://www.aph.gov.au/News_and_Events/"));
    }
}

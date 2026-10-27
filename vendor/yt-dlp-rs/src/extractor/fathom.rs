//! Upstream `FathomIE` (`yt_dlp/extractor/fathom.py`): the `app` element's
//! `data-page` props with the call's HLS manifest.
use super::{Extractor, download_webpage, extract_m3u8_fatal};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Fathom;

const NAME: &str = "Fathom";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?fathom\.video/share/(?P<id>[^/?#&]+)").unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for Fathom {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &display_id)?;
        // `traverse_obj(get_element_html_by_id('app', webpage),
        // ({extract_attributes}, 'data-page', {json.loads}, 'props'))`.
        let props = crate::html_elements::element_html_by_id("app", &webpage)
            .map_err(Error::Reported)?
            .and_then(|element| {
                let attributes = crate::html_elements::extract_attributes(&element);
                let page = attributes.get("data-page")?.as_str()?.to_owned();
                serde_json::from_str::<Value>(&page)
                    .ok()?
                    .get("props")
                    .cloned()
            })
            .ok_or_else(|| Error::Reported("'NoneType' object is not subscriptable".into()))?;
        let key_error = |name: &str| {
            Error::Reported(format!(
                "{display_id}: An extractor error has occurred. (caused by KeyError('{name}')); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        };
        let call = props.get("call").ok_or_else(|| key_error("call"))?;
        let id = match call.get("id").ok_or_else(|| key_error("id"))? {
            Value::String(id) => id.clone(),
            Value::Number(id) => id.to_string(),
            _ => {
                return Err(Error::UnsupportedHtml5(
                    "Fathom call IDs that are not strings or numbers are not supported natively"
                        .into(),
                ));
            }
        };
        let manifest = call
            .get("video_url")
            .ok_or_else(|| key_error("video_url"))?;
        let manifest = Url::parse(manifest.as_str().ok_or_else(|| {
            Error::UnsupportedHtml5(
                "Fathom video URLs that are not strings are not supported natively".into(),
            )
        })?)?;
        let mut info = extract_m3u8_fatal(client, &manifest, NAME, &id, None, false)?;
        let title = props
            .get("head")
            .and_then(|head| head.get("title"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        // `('duration', {float_or_none})`.
        let duration = match props.get("duration") {
            Some(Value::Number(number)) => number.as_f64(),
            Some(Value::String(text)) => text.trim().parse::<f64>().ok(),
            _ => None,
        };
        let timestamp = call
            .get("started_at")
            .and_then(Value::as_str)
            .and_then(crate::dates::parse_iso8601);
        let shape = if title.is_some() {
            "fathom"
        } else {
            "fathom-untitled"
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
        if let Some(duration) = duration {
            crate::metadata_parser::set_field(&mut info, "duration", duration.into());
        }
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
            "https://fathom.video/share/G9mkjkspnohVVZ_L5nrsoPycyWcB8y7s"
        ));
        assert!(!suitable("https://fathom.video/"));
    }
}

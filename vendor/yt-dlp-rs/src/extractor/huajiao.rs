//! Upstream `HuajiaoIE` (`yt_dlp/extractor/huajiao.py`): a live page's
//! `var feed` JSON with its HLS manifest.
use super::{Extractor, download_webpage, extract_m3u8_fatal, search_warning};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Huajiao;

const NAME: &str = "Huajiao";

static VALID_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://(?:www\.)?huajiao\.com/l/(?P<id>[0-9]+)").unwrap());
static FEED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"var\s+feed\s*=\s*(\{.+\})").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for Huajiao {
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
        let webpage = download_webpage(client, url, NAME, &id)?;
        let feed = FEED.captures(&webpage).ok_or_else(|| {
            Error::Reported(format!(
                "[{NAME}] {id}: Unable to extract feed json; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
        let feed: Value = serde_json::from_str(&feed[1]).map_err(|_| {
            Error::UnsupportedHtml5(
                "Huajiao feeds that do not parse are not supported natively".into(),
            )
        })?;
        let description = crate::html::html_search_meta(&webpage, "description");
        if description.is_none() {
            search_warning(NAME, "description");
        }
        let key_error = |name: &str| {
            Error::Reported(format!(
                "{id}: An extractor error has occurred. (caused by KeyError('{name}')); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        };
        let inner = feed.get("feed").ok_or_else(|| key_error("feed"))?;
        let title = inner
            .get("formated_title")
            .ok_or_else(|| key_error("formated_title"))?
            .as_str()
            .ok_or_else(|| {
                Error::UnsupportedHtml5(
                    "Huajiao titles that are not strings are not supported natively".into(),
                )
            })?
            .to_owned();
        // `feed.get(section, {}).get(field)`.
        let get = |section: &str, field: &str| {
            feed.get(section)
                .and_then(|section| section.get(field))
                .cloned()
                .unwrap_or(Value::Null)
        };
        let duration = get("feed", "duration")
            .as_str()
            .and_then(crate::parse_duration);
        let timestamp = feed
            .get("creatime")
            .and_then(Value::as_str)
            .and_then(crate::dates::parse_iso8601_space);
        let manifest = inner.get("m3u8").ok_or_else(|| key_error("m3u8"))?;
        let manifest = Url::parse(manifest.as_str().ok_or_else(|| {
            Error::UnsupportedHtml5(
                "Huajiao manifests that are not strings are not supported natively".into(),
            )
        })?)?;
        let mut info = extract_m3u8_fatal(client, &manifest, NAME, &id, None, false)?;
        info.id = id;
        info.title = title;
        info.webpage_url = url.clone();
        info.direct = None;
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        info.metadata.description = description;
        info.metadata.duration = duration;
        for (key, value) in [
            ("thumbnail", get("feed", "image")),
            ("timestamp", timestamp.map_or(Value::Null, Value::from)),
            ("uploader", get("author", "nickname")),
            ("uploader_id", get("author", "uid")),
        ] {
            if value.is_null() {
                info.extra.insert(key.into(), Value::Null);
            } else {
                crate::metadata_parser::set_field(&mut info, key, value);
            }
        }
        Ok(Some(crate::info_order::mark_info(info, "huajiao")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("http://www.huajiao.com/l/38941232"));
        assert!(!suitable("http://www.huajiao.com/u/1"));
    }
}

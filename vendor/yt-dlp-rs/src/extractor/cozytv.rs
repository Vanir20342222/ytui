//! Upstream `CozyTVIE` (`yt_dlp/extractor/cozytv.py`): a replay's cached
//! metadata and its CDN HLS manifest.
use super::{Extractor, download_json, extract_m3u8_fatal};
use crate::{Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct CozyTv;

const NAME: &str = "CozyTV";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?cozy\.tv/(?P<uploader>[^/]+)/replays/(?P<id>[^/$#&?]+)")
        .unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for CozyTv {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let (uploader, date) = (&found["uploader"], &found["id"]);
        let id = format!("{uploader}-{date}");
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let api = Url::parse(&format!(
            "https://api.cozy.tv/cache/{uploader}/replay/{date}"
        ))?;
        let data = download_json(client, &api, NAME, &id)?;
        let manifest = Url::parse(&format!(
            "https://cozycdn.foxtrotstream.xyz/replays/{uploader}/{date}/index.m3u8"
        ))?;
        let mut info = extract_m3u8_fatal(client, &manifest, NAME, &id, None, true)?;
        info.id = id;
        match data.get("title").and_then(Value::as_str) {
            Some(title) if !title.is_empty() => info.title = title.to_owned(),
            _ => {
                info.title = String::new();
                info.extra
                    .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
            }
        }
        info.webpage_url = url.clone();
        info.direct = None;
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        // `data_json.get('user') or uploader`.
        let user = data
            .get("user")
            .filter(|user| match user {
                Value::Null | Value::Bool(false) => false,
                Value::String(text) => !text.is_empty(),
                Value::Number(number) => number.as_f64() != Some(0.0),
                Value::Array(items) => !items.is_empty(),
                Value::Object(object) => !object.is_empty(),
                Value::Bool(true) => true,
            })
            .cloned()
            .unwrap_or_else(|| uploader.into());
        crate::metadata_parser::set_field(&mut info, "uploader", user);
        let upload_date = data
            .get("date")
            .and_then(Value::as_str)
            .and_then(|date| crate::dates::unified_strdate(date, true));
        match upload_date {
            Some(date) => crate::metadata_parser::set_field(&mut info, "upload_date", date.into()),
            None => {
                info.extra.insert("upload_date".into(), Value::Null);
            }
        }
        crate::metadata_parser::set_field(&mut info, "was_live", Value::Bool(true));
        // `data_json.get('duration')` as is (processing drops `None`).
        if let Some(duration) = data.get("duration").filter(|duration| !duration.is_null()) {
            crate::metadata_parser::set_field(&mut info, "duration", duration.clone());
        }
        Ok(Some(crate::info_order::mark_info(info, "cozytv")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://cozy.tv/beardson/replays/2021-11-19_1"));
        assert!(!suitable("https://cozy.tv/beardson"));
    }
}

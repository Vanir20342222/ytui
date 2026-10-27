//! Upstream `TweakersIE` (`yt_dlp/extractor/tweakers.py`): the video's
//! `s1playlist` JSON with its progressive sources as formats.
use super::{Extractor, download_json};
use crate::{Error, Result, VideoFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Tweakers;

const NAME: &str = "Tweakers";

static VALID_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://tweakers\.net/video/(?P<id>\d+)").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for Tweakers {
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
            "https://tweakers.net/video/s1playlist/{id}/1920/1080/playlist.json"
        ))?;
        let data = download_json(client, &api, NAME, &id)?;
        let key_error = |key: &str| {
            Error::Reported(format!(
                "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        };
        let items = data.get("items").ok_or_else(|| key_error("items"))?;
        // `['items'][0]` of an empty list escapes as a bare `IndexError`.
        let video = items
            .get(0)
            .ok_or_else(|| Error::Reported("list index out of range".into()))?;
        let title = video
            .get("title")
            .cloned()
            .ok_or_else(|| key_error("title"))?;
        let int = super::streamable::int_or_none;
        let mut formats = Vec::new();
        let locations = video
            .get("locations")
            .and_then(|locations| locations.get("progressive"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for location in &locations {
            let label = location.get("label").cloned().unwrap_or(Value::Null);
            let width = int(location.get("width"));
            let height = int(location.get("height"));
            for source in location
                .get("sources")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let Some(source_url) = source
                    .get("src")
                    .and_then(Value::as_str)
                    .filter(|src| !src.is_empty())
                else {
                    continue;
                };
                // `mimetype2ext(type) or determine_ext(url)`.
                let ext = source
                    .get("type")
                    .and_then(Value::as_str)
                    .map(|mime| super::direct::mimetype2ext_or(mime, None))
                    .filter(|ext| !ext.is_empty())
                    .or_else(|| super::direct::determine_ext(source_url))
                    .unwrap_or_else(|| "unknown_video".into());
                let mut format: VideoFormat = serde_json::from_value(json!({
                    "format_id": label.as_str().unwrap_or_default(),
                    "url": source_url,
                    "width": width,
                    "height": height,
                    "ext": ext,
                }))
                .map_err(|error| Error::Reported(format!("[{NAME}] {id}: {error}")))?;
                crate::info_order::mark_format(&mut format, "tweakers");
                formats.push(format);
            }
        }
        if formats.is_empty() {
            return Err(Error::Reported(format!(
                "[{NAME}] {id}: No video formats found!; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        }
        let first = formats[0].url.clone();
        let mut info = super::direct::build_at(first.clone(), &first, String::new());
        info.formats = formats;
        info.direct = None;
        info.id = id;
        match title.as_str() {
            Some(title) if !title.is_empty() => info.title = title.to_owned(),
            _ => {
                info.title = String::new();
                info.extra
                    .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
            }
        }
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        info.metadata.description = video
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_owned);
        info.metadata.thumbnail = video
            .get("poster")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let duration = int(video.get("duration"));
        if !duration.is_null() {
            crate::metadata_parser::set_field(&mut info, "duration", duration);
        }
        match video.get("account").cloned().unwrap_or(Value::Null) {
            Value::Null => {
                info.extra.insert("uploader_id".into(), Value::Null);
            }
            account => crate::metadata_parser::set_field(&mut info, "uploader_id", account),
        }
        Ok(Some(crate::info_order::mark_info(info, "tweakers")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://tweakers.net/video/9926/new-nintendo-3ds-xl.html"
        ));
        assert!(!suitable("https://tweakers.net/nieuws/9926"));
    }
}

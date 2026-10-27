//! Upstream `FczenitIE` (`yt_dlp/extractor/fczenit.py`): the page's MSI
//! video ID and the player's `msi/video` JSON qualities as formats.
use super::{Extractor, download_json, download_webpage};
use crate::{Error, Result, VideoFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Fczenit;

const NAME: &str = "Fczenit";

static VALID_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://(?:www\.)?fc-zenit\.ru/video/(?P<id>[0-9]+)").unwrap());

static MSI_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)config\s*=\s*\{.+?video_id\s*:\s*'([^']+)'").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

impl Extractor for Fczenit {
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
        let msi_id = MSI_ID
            .captures(&webpage)
            .map(|found| found[1].to_owned())
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {id}: Unable to extract msi id; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let mut api = Url::parse("http://player.fc-zenit.ru/msi/video")?;
        api.query_pairs_mut().append_pair("video", &msi_id);
        let response = download_json(client, &api, NAME, &msi_id)?;
        let data = response.get("data").ok_or_else(|| key_error(&id, "data"))?;
        let title = match data.get("name") {
            Some(Value::String(title)) => title.clone(),
            Some(_) => {
                return Err(Error::UnsupportedHtml5(
                    "non-string Fczenit names are not supported natively".into(),
                ));
            }
            None => return Err(key_error(&id, "name")),
        };
        let qualities = match data.get("qualities") {
            Some(Value::Array(qualities)) => qualities.as_slice(),
            Some(_) => {
                return Err(Error::UnsupportedHtml5(
                    "non-list Fczenit qualities are not supported natively".into(),
                ));
            }
            None => return Err(key_error(&id, "qualities")),
        };
        let mut formats = Vec::new();
        for quality in qualities {
            let Some(media) = quality.get("url").filter(|url| match url {
                Value::Null | Value::Bool(false) => false,
                Value::String(text) => !text.is_empty(),
                _ => true,
            }) else {
                continue;
            };
            let Some(media) = media.as_str() else {
                return Err(Error::UnsupportedHtml5(
                    "non-string Fczenit quality URLs are not supported natively".into(),
                ));
            };
            let media = Url::parse(media)?;
            let label = quality.get("label");
            let ext = super::direct::determine_ext(media.as_str())
                .unwrap_or_else(|| "unknown_video".into());
            let mut format: VideoFormat = serde_json::from_value(json!({
                "format_id": label.and_then(Value::as_str).unwrap_or_default(),
                "url": media.as_str(),
                "ext": ext,
                "height": super::streamable::int_or_none(label),
            }))
            .map_err(|error| Error::Reported(format!("[{NAME}] {id}: {error}")))?;
            crate::info_order::mark_format(&mut format, "fczenit");
            formats.push(format);
        }
        let tags: Vec<Value> = match data.get("tags") {
            Some(Value::Array(tags)) => tags
                .iter()
                .filter_map(|tag| {
                    tag.get("label").filter(|label| match label {
                        Value::Null | Value::Bool(false) => false,
                        Value::String(text) => !text.is_empty(),
                        _ => true,
                    })
                })
                .cloned()
                .collect(),
            _ => Vec::new(),
        };
        let duration = match data.get("duration") {
            Some(Value::Number(number)) => number.as_f64(),
            Some(Value::String(text)) => text.trim().parse::<f64>().ok(),
            _ => None,
        };
        let mut info = super::direct::build_at(url.clone(), url, String::new());
        info.formats = formats;
        info.direct = None;
        info.id = id;
        info.title = title;
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        info.metadata.thumbnail = data
            .get("preview")
            .and_then(Value::as_str)
            .map(str::to_owned);
        info.metadata.duration = duration.map(crate::info::PyNumber::Float);
        info.metadata.timestamp = super::streamable::int_or_none(data.get("date"))
            .as_i64()
            .map(crate::info::PyNumber::Int);
        info.extra.insert("tags".into(), Value::Array(tags));
        Ok(Some(crate::info_order::mark_info(info, "fczenit")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("http://www.fc-zenit.ru/video/41044/"));
        assert!(!suitable("http://www.fc-zenit.ru/news/41044/"));
    }
}

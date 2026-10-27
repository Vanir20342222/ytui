//! Upstream `KommunetvIE` (`yt_dlp/extractor/kommunetv.py`): the streams
//! API's title and first playlist file as non-fatal HLS formats.
use super::{Extractor, download_json_headers, extract_m3u8_optional};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Kommunetv;

const NAME: &str = "Kommunetv";

static VALID_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://\w+\.kommunetv\.no/archive/(?P<id>\w+)").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// An escaped `KeyError` (a bare `IndexError` is `list index out of range`).
fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `value[key]`.
fn key<'a>(value: &'a Value, name: &str, id: &str) -> Result<&'a Value> {
    match value {
        Value::Object(object) => object.get(name).ok_or_else(|| key_error(id, name)),
        _ => Err(Error::UnsupportedHtml5(
            "Kommunetv API values that are not objects are not supported natively".into(),
        )),
    }
}

/// `value[0]`.
fn first(value: &Value) -> Result<&Value> {
    match value {
        Value::Array(items) => items
            .first()
            .ok_or_else(|| Error::Reported("list index out of range".into())),
        _ => Err(Error::UnsupportedHtml5(
            "Kommunetv API playlists that are not lists are not supported natively".into(),
        )),
    }
}

impl Extractor for Kommunetv {
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
            "https://oslo.kommunetv.no/api/streams?streamType=1&id={id}"
        ))?;
        let data =
            download_json_headers(client, &api, NAME, &id, &[("Accept", "application/json")])?;
        let title = key(key(&data, "stream", &id)?, "title", &id)?;
        let Some(title) = title.as_str() else {
            return Err(Error::UnsupportedHtml5(
                "Kommunetv titles that are not strings are not supported natively".into(),
            ));
        };
        let file = key(
            first(key(first(key(&data, "playlist", &id)?)?, "playlist", &id)?)?,
            "file",
            &id,
        )?;
        let Some(file) = file.as_str() else {
            return Err(Error::UnsupportedHtml5(
                "Kommunetv files that are not strings are not supported natively".into(),
            ));
        };
        // `update_url(file, query=None, fragment=None)`.
        let mut manifest = Url::parse(file)?;
        manifest.set_query(None);
        manifest.set_fragment(None);
        let formats = extract_m3u8_optional(client, &manifest, NAME, &id, Some("hls"), false)
            .map(|stream| stream.formats)
            .unwrap_or_default();
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
        info.title = title.to_owned();
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        Ok(Some(crate::info_order::mark_info(info, "kommunetv")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://oslo.kommunetv.no/archive/921"));
        assert!(!suitable("https://oslo.kommunetv.no/live/921"));
    }
}

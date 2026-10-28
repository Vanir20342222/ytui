//! Upstream `GraspopIE` (`yt_dlp/extractor/graspop.py`): the Proximus
//! festival stream's HLS manifest (over `http:`), name and poster.
use super::{Extractor, download_json, extract_m3u8_fatal};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Graspop;

const NAME: &str = "Graspop";

static VALID_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://vod\.graspop\.be/[a-z]{2}/(?P<id>\d+)/").unwrap());

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
            "Graspop stream values that are not objects are not supported natively".into(),
        )),
    }
}

impl Extractor for Graspop {
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
            "https://tv.proximus.be/MWC/videocenter/festivals/{id}/stream"
        ))?;
        let metadata = download_json(client, &api, NAME, &id)?;
        let source = key(&metadata, "source", &id)?;
        let Some(asset) = key(source, "assetUri", &id)?.as_str() else {
            return Err(Error::UnsupportedHtml5(
                "Graspop asset URIs that are not strings are not supported natively".into(),
            ));
        };
        // `update_url(..., scheme='http')`.
        let mut manifest = Url::parse(asset)?;
        if manifest.set_scheme("http").is_err() {
            return Err(Error::UnsupportedHtml5(
                "Graspop asset URIs without an HTTP scheme are not supported natively".into(),
            ));
        }
        let mut info = extract_m3u8_fatal(client, &manifest, NAME, &id, None, false)?;
        let title = metadata.get("name").and_then(Value::as_str);
        let shape = if title.is_some() {
            "graspop"
        } else {
            "graspop-untitled"
        };
        info.id = id;
        match title {
            Some(title) => info.title = title.to_owned(),
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
        info.metadata.thumbnail = source
            .get("poster")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none);
        Ok(Some(crate::info_order::mark_info(info, shape)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://vod.graspop.be/fr/101556/thy-art-is-murder-concert/"
        ));
        assert!(!suitable("https://vod.graspop.be/fr/101556"));
    }
}

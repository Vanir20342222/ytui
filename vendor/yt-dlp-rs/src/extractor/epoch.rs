//! Upstream `EpochIE` (`yt_dlp/extractor/epoch.py`): the `videobox`
//! element's YouMaker ID and its HLS playlist.
use super::{Extractor, download_webpage, extract_m3u8_fatal, search_warning};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Epoch;

const NAME: &str = "Epoch";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://www.theepochtimes\.com/[\w-]+_(?P<id>\d+).html").unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for Epoch {
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
        // `extract_attributes(get_element_html_by_id('videobox', webpage))['data-id']`:
        // parse errors and a missing element escape as plain errors.
        let element = crate::html_elements::element_html_by_id("videobox", &webpage)
            .map_err(Error::Reported)?
            .ok_or_else(|| Error::Reported("object of type 'NoneType' has no len()".into()))?;
        let attributes = crate::html_elements::extract_attributes(&element);
        let youmaker_id = match attributes.get("data-id") {
            Some(Value::String(youmaker_id)) => youmaker_id.clone(),
            Some(_) => {
                return Err(Error::UnsupportedHtml5(
                    "Epoch video boxes with a valueless data-id are not supported natively".into(),
                ));
            }
            None => {
                return Err(Error::Reported(format!(
                    "{id}: An extractor error has occurred. (caused by KeyError('data-id')); {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        let manifest = Url::parse(&format!(
            "http://vs1.youmaker.com/assets/{youmaker_id}/playlist.m3u8"
        ))?;
        let mut info = extract_m3u8_fatal(client, &manifest, NAME, &id, Some("hls"), true)?;
        let title = crate::html::html_extract_title(&webpage);
        if title.is_none() {
            search_warning(NAME, "title");
        }
        info.id = youmaker_id;
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
        Ok(Some(crate::info_order::mark_info(info, "epoch")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.theepochtimes.com/they-can-do-audio-video-physical-surveillance-on-you-24h-365d-a-year-rex-lee-on-intrusive-apps_4661688.html"
        ));
        assert!(!suitable("https://www.theepochtimes.com/news"));
    }
}

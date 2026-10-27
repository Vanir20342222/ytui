//! Upstream `BreitBartIE` (`yt_dlp/extractor/breitbart.py`): the JW Player
//! manifest for a video page's ID with the page's generic title and
//! OpenGraph metadata.
use super::{Extractor, download_webpage, extract_m3u8_fatal, search_warning};
use crate::{Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct BreitBart;

const NAME: &str = "BreitBart";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?breitbart\.com/videos/v/(?P<id>[^/?#]+)").unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for BreitBart {
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
        let manifest = Url::parse(&format!("https://cdn.jwplayer.com/manifests/{id}.m3u8"))?;
        let mut info = extract_m3u8_fatal(client, &manifest, NAME, &id, None, false)?;
        // `_generic_title('', webpage)`: no URL basename to fall back on.
        let title = super::html5::page_title(&webpage);
        let search = |property: &str, name: &str| {
            let value = crate::html::og_search_property(&webpage, property);
            if value.is_none() {
                search_warning(NAME, name);
            }
            value
        };
        let description = search("description", "OpenGraph description");
        let thumbnail = search("image", "thumbnail URL");
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
        info.metadata.description = description;
        info.metadata.thumbnail = thumbnail;
        info.metadata.age_limit = crate::html::rta_search(&webpage);
        if info.metadata.age_limit.is_none() {
            info.extra.insert("age_limit".into(), Value::Null);
        }
        Ok(Some(crate::info_order::mark_info(info, "breitbart")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.breitbart.com/videos/v/5cOz1yup/?pl=Ij6NDOji"
        ));
        assert!(!suitable("https://www.breitbart.com/videos/"));
    }
}

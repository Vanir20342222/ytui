//! Upstream `OnePlacePodcastIE` (`yt_dlp/extractor/oneplace.py`): an
//! episode page's MP3 URL, title and description as an untyped result.
use super::{Extractor, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct OnePlacePodcast;

const NAME: &str = "OnePlacePodcast";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://www\.oneplace\.com/[\w]+/[^/]+/listen/[\w-]+-(?P<id>\d+)").unwrap()
});

static MEDIA: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        Regex::new(r#"mp3-url\s*=\s*"([^"]+)"#).unwrap(),
        Regex::new(r#"<div[^>]+id\s*=\s*"player"[^>]+data-media-url\s*=\s*"([^"]+)"#).unwrap(),
    ]
});

static DETAILS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<div[^>]class\s*=\s*"details"[^>]+>[^<]<h2[^>]+>([^>]+)>"#).unwrap()
});

static DESCRIPTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<div[^>]+class="[^"]+epDesc"[^>]*>\s*(.+?)\s*</div>"#).unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for OnePlacePodcast {
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
        let media = MEDIA
            .iter()
            .find_map(|regex| regex.captures(&webpage).map(|found| found[1].to_owned()))
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {id}: Unable to extract media url; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        // `_html_search_regex((details, _meta_regex('og:title'),
        // _meta_regex('title')), default=None)`.
        let title = DETAILS
            .captures(&webpage)
            .map(|found| crate::html::clean_html(&found[1]))
            .or_else(|| crate::html::html_search_meta(&webpage, "og:title"))
            .or_else(|| crate::html::html_search_meta(&webpage, "title"));
        let description = DESCRIPTION
            .captures(&webpage)
            .map(|found| crate::html::clean_html(&found[1]));
        let media = url.join(&media)?;
        let mut info = super::direct::with_format(
            super::direct::build_at(media.clone(), &media, "mp3".into()),
            "",
        );
        for format in &mut info.formats {
            format.ext = "mp3".into();
            format.vcodec = Some("none".into());
            format.format_id = String::new();
        }
        info.vcodec = Some("none".into());
        info.format_id = None;
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
        Ok(Some(crate::info_order::mark_info(info, "oneplace")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.oneplace.com/ministries/a-daily-walk/listen/living-in-the-last-days-part-2-958461"
        ));
        assert!(!suitable(
            "https://www.oneplace.com/ministries/a-daily-walk/"
        ));
    }
}

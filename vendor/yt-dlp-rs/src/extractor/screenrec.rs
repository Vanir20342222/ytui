//! Upstream `ScreenRecIE` (`yt_dlp/extractor/screenrec.py`): a share
//! page's `customUrl` HLS manifest with its OpenGraph metadata.
use super::{Extractor, download_webpage, extract_m3u8_fatal, search_warning};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct ScreenRec;

const NAME: &str = "ScreenRec";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?screenrec\.com/share/(?P<id>\w{10})").unwrap()
});
/// `customUrl\s*:\s*(["\'])(?P<url>(?:(?!\1).)+)\1` as one alternative per
/// quote.
static CUSTOM_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"customUrl\s*:\s*(?:"(?P<a>[^"\n]+)"|'(?P<b>[^'\n]+)')"#).unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for ScreenRec {
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
        let manifest = CUSTOM_URL
            .captures(&webpage)
            .and_then(|found| found.name("a").or_else(|| found.name("b")))
            .map(|found| found.as_str().to_owned())
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {id}: Unable to extract m3u8 URL; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        // `_og_search_title(default=None) or _html_extract_title(webpage)`.
        let title = crate::html::og_search_property(&webpage, "title")
            .filter(|title| !title.is_empty())
            .or_else(|| {
                let title = crate::html::html_extract_title(&webpage);
                if title.is_none() {
                    search_warning(NAME, "title");
                }
                title
            });
        let search = |property: &str, name: &str| {
            let value = crate::html::og_search_property(&webpage, property);
            if value.is_none() {
                search_warning(NAME, name);
            }
            value
        };
        let description = search("description", "OpenGraph description");
        let thumbnail = search("image", "thumbnail URL");
        let manifest = Url::parse(&manifest)?;
        let mut info = extract_m3u8_fatal(client, &manifest, NAME, &id, None, false)?;
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
        Ok(Some(crate::info_order::mark_info(info, "screenrec")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://screenrec.com/share/DasLtbknYo"));
        assert!(!suitable("https://screenrec.com/share/short"));
    }
}

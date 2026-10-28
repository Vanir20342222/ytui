//! Upstream `Ku6IE` (`yt_dlp/extractor/ku6.py`): a show page's heading and
//! the player JSON's download URL as an untyped result.
use super::{Extractor, download_json, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use std::sync::LazyLock;
use url::Url;

pub struct Ku6;

const NAME: &str = "Ku6";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://v\.ku6\.com/show/(?P<id>[a-zA-Z0-9\-\_]+)(?:\.)*html").unwrap()
});
static TITLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<h1 title=.*>(.*?)</h1>").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for Ku6 {
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
        let title = TITLE
            .captures(&webpage)
            .map(|found| crate::html::clean_html(&found[1]).trim().to_owned())
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {id}: Unable to extract title; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let api = Url::parse(&format!("http://v.ku6.com/fetchVideo4Player/{id}.html"))?;
        let data = download_json(client, &api, NAME, &id)?;
        let key_error = |name: &str| {
            Error::Reported(format!(
                "{id}: An extractor error has occurred. (caused by KeyError('{name}')); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        };
        let media = data
            .get("data")
            .ok_or_else(|| key_error("data"))?
            .get("f")
            .ok_or_else(|| key_error("f"))?;
        let Some(media) = media.as_str() else {
            return Err(Error::UnsupportedHtml5(
                "Ku6 download URLs that are not strings are not supported natively".into(),
            ));
        };
        let media = Url::parse(media)?;
        let ext =
            super::direct::determine_ext(media.as_str()).unwrap_or_else(|| "unknown_video".into());
        let mut info = super::direct::with_format(
            super::direct::build_at(media.clone(), &media, ext.clone()),
            "",
        );
        for format in &mut info.formats {
            format.ext = ext.clone();
            format.format_id = String::new();
        }
        info.format_id = None;
        info.id = id;
        // An empty heading gets processing's generic title.
        info.title = title;
        info.webpage_url = url.clone();
        info.direct = None;
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        Ok(Some(crate::info_order::mark_info(info, "ku6")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://v.ku6.com/show/JG-8yS14xzBr4bCn1pu0xw...html"
        ));
        assert!(!suitable("http://v.ku6.com/show/"));
    }
}

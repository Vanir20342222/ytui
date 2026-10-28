//! Upstream `PopcornTVIE` (`yt_dlp/extractor/popcorntv.py`): a page's
//! `contentUrl`/`embedUrl` link HLS manifest with its microdata.
use super::{Extractor, download_webpage, extract_m3u8_fatal, search_warning};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct PopcornTv;

const NAME: &str = "PopcornTV";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://[^/]+\.popcorntv\.it/guarda/(?P<display_id>[^/]+)/(?P<id>\d+)").unwrap()
});
static LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(<link[^>]+itemprop=["'](?:content|embed)Url[^>]*>)"#).unwrap());
static TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<h1[^>]+itemprop=["']name[^>]*>([^<]+)"#).unwrap());
static DESCRIPTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)<article[^>]+itemprop=["']description[^>]*>(.+?)</article>"#).unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for PopcornTv {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let (display_id, id) = (found["display_id"].to_owned(), found["id"].to_owned());
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &display_id)?;
        let link = LINK.captures(&webpage).ok_or_else(|| {
            Error::Reported(format!(
                "[{NAME}] {id}: Unable to extract content; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
        let manifest = match crate::html_elements::extract_attributes(&link[1]).get("href") {
            Some(Value::String(href)) => href.clone(),
            Some(_) => {
                return Err(Error::UnsupportedHtml5(
                    "PopcornTV links with a valueless href are not supported natively".into(),
                ));
            }
            None => {
                return Err(Error::Reported(format!(
                    "{id}: An extractor error has occurred. (caused by KeyError('href')); {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        let manifest = Url::parse(&manifest)?;
        let mut info =
            extract_m3u8_fatal(client, &manifest, NAME, &display_id, Some("hls"), false)?;
        let warn = |value: Option<String>, name: &str| {
            if value.is_none() {
                search_warning(NAME, name);
            }
            value
        };
        let title = TITLE
            .captures(&webpage)
            .map(|found| found[1].to_owned())
            .or_else(|| {
                warn(
                    crate::html::og_search_property(&webpage, "title"),
                    "OpenGraph title",
                )
            });
        let description = warn(
            DESCRIPTION
                .captures(&webpage)
                .map(|found| crate::html::clean_html(&found[1]).trim().to_owned()),
            "description",
        );
        let thumbnail = warn(
            crate::html::og_search_property(&webpage, "image"),
            "thumbnail URL",
        );
        let meta = |name: &str, display: &str| {
            warn(crate::html::html_search_meta(&webpage, name), display)
        };
        let timestamp = meta("uploadDate", "timestamp")
            .and_then(|date| crate::dates::unified_timestamp(&date, true));
        // `int_or_none(..., invscale=60)`.
        let duration =
            super::streamable::int_or_none(meta("duration", "duration").map(Value::from).as_ref())
                .as_i64()
                .map(|minutes| minutes * 60);
        let view_count = super::streamable::int_or_none(
            meta("interactionCount", "view count")
                .map(Value::from)
                .as_ref(),
        );
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
        crate::metadata_parser::set_field(&mut info, "display_id", display_id.into());
        info.metadata.description = description;
        info.metadata.thumbnail = thumbnail;
        for (key, value) in [
            ("timestamp", timestamp.map_or(Value::Null, Value::from)),
            ("view_count", view_count),
        ] {
            if value.is_null() {
                info.extra.insert(key.into(), Value::Null);
            } else {
                crate::metadata_parser::set_field(&mut info, key, value);
            }
        }
        // Processing drops a `None` duration.
        if let Some(duration) = duration {
            crate::metadata_parser::set_field(&mut info, "duration", duration.into());
        }
        Ok(Some(crate::info_order::mark_info(info, "popcorntv")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://animemanga.popcorntv.it/guarda/food-wars-battaglie-culinarie-episodio-01/9183"
        ));
        assert!(!suitable("https://cinema.popcorntv.it/guarda/x"));
    }
}

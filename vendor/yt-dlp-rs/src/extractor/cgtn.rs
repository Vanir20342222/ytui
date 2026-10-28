//! Upstream `CGTNIE` (`yt_dlp/extractor/cgtn.py`): a news page's
//! `data-video` HLS manifest with its date, section, author and OpenGraph
//! metadata.
use super::{Extractor, download_webpage, extract_m3u8_fatal, search_warning};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Cgtn;

const NAME: &str = "CGTN";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://news\.cgtn\.com/news/[0-9]{4}-[0-9]{2}-[0-9]{2}/[a-zA-Z0-9-]+-(?P<id>[a-zA-Z0-9-]+)/index\.html",
    )
    .unwrap()
});
static DOWNLOAD_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"data-video ="(?P<url>.+m3u8)""#).unwrap());
static DATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<span class="date">\s*(.+?)\s*</span>"#).unwrap());
static SECTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<span class="section">\s*(.+?)\s*</span>"#).unwrap());
static AUTHOR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<div class="news-author-name">\s*(.+?)\s*</div>"#).unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_html_search_regex`: the cleaned first group.
fn html_search(regex: &Regex, webpage: &str) -> Option<String> {
    regex
        .captures(webpage)
        .map(|found| crate::html::clean_html(&found[1]).trim().to_owned())
}

impl Extractor for Cgtn {
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
        let download_url = html_search(&DOWNLOAD_URL, &webpage).ok_or_else(|| {
            Error::Reported(format!(
                "[{NAME}] {id}: Unable to extract download_url; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
        let optional = |regex: &Regex, name: &str| {
            let value = html_search(regex, &webpage);
            if value.is_none() {
                search_warning(NAME, name);
            }
            value
        };
        let datetime = optional(&DATE, "datetime_str");
        let category = optional(&SECTION, "category");
        let author = AUTHOR.captures(&webpage).map(|found| found[1].to_owned());
        let title = crate::html::og_search_property(&webpage, "title");
        if title.is_none() {
            search_warning(NAME, "OpenGraph title");
        }
        let description = crate::html::og_search_property(&webpage, "description");
        let thumbnail = crate::html::og_search_property(&webpage, "image");
        if thumbnail.is_none() {
            search_warning(NAME, "thumbnail URL");
        }
        let manifest = Url::parse(&download_url)?;
        let mut info = extract_m3u8_fatal(client, &manifest, NAME, &id, Some("hls"), false)?;
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
        // `try_get(unified_timestamp(datetime_str), lambda x: x - 8 * 3600)`.
        let timestamp = datetime
            .as_deref()
            .and_then(|datetime| crate::dates::unified_timestamp(datetime, true))
            .map(|timestamp| timestamp - 8 * 3600);
        for (key, value) in [
            (
                "categories",
                category
                    .filter(|category| !category.is_empty())
                    .map_or(Value::Null, |category| serde_json::json!([category])),
            ),
            (
                "creators",
                author
                    .filter(|author| !author.is_empty())
                    .map_or(Value::Null, |author| serde_json::json!([author])),
            ),
            ("timestamp", timestamp.map_or(Value::Null, Value::from)),
        ] {
            if value.is_null() {
                info.extra.insert(key.into(), Value::Null);
            } else {
                crate::metadata_parser::set_field(&mut info, key, value);
            }
        }
        Ok(Some(crate::info_order::mark_info(info, "cgtn")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://news.cgtn.com/news/2021-03-09/Up-and-Out-of-Poverty-Ep-1-A-solemn-promise-YuOUaOzGQU/index.html"
        ));
        assert!(!suitable("https://news.cgtn.com/news/2021-03-09/x/"));
    }
}

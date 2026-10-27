//! Upstream `JoveIE` (`yt_dlp/extractor/jove.py`): the chapters XML's
//! video URL as an untyped result with the page's citation metadata.
use super::{Extractor, download_webpage, download_xml_note, search_warning};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Jove;

const NAME: &str = "Jove";

static VALID_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://(?:www\.)?jove\.com/video/(?P<id>[0-9]+)").unwrap());
static CHAPTERS_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"/video-chapters\?videoid=([0-9]+)").unwrap());
static DESCRIPTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<div id="section_body_summary"><p class="jove_content">(.+?)</p>"#).unwrap()
});
static COMMENTS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<meta name="num_comments" content="(\d+) Comments?""#).unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for Jove {
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
        let chapters_id = CHAPTERS_ID
            .captures(&webpage)
            .map(|found| crate::html::clean_html(&found[1]).trim().to_owned())
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {id}: Unable to extract chapters id; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let chapters_url = Url::parse(&format!(
            "http://www.jove.com/video-chapters?videoid={chapters_id}"
        ))?;
        let xml = download_xml_note(client, &chapters_url, NAME, &id, "Downloading chapters XML")
            .map_err(|error| match error {
            Error::ExtractorRequest {
                extractor, source, ..
            } => Error::Reported(format!(
                "[{extractor}] {id}: Failed to download chapters XML: {}",
                crate::upstream_error_text(&source)
            )),
            error => error,
        })?;
        let document = roxmltree::Document::parse(xml.trim_start_matches('\u{feff}'))
            .map_err(|error| Error::Reported(format!("[{NAME}] {id}: {error}")))?;
        let media = document
            .root_element()
            .attribute("video")
            .filter(|video| !video.is_empty())
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {id}: Failed to get the video URL; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let media = Url::parse(media)?;
        let warn = |value: Option<String>, name: &str| {
            if value.is_none() {
                search_warning(NAME, name);
            }
            value
        };
        let title = warn(
            crate::html::html_search_meta(&webpage, "citation_title"),
            "title",
        );
        let thumbnail = warn(
            crate::html::og_search_property(&webpage, "image"),
            "thumbnail URL",
        );
        let description = warn(
            DESCRIPTION
                .captures(&webpage)
                .map(|found| crate::html::clean_html(&found[1]).trim().to_owned()),
            "description",
        );
        let publish_date = warn(
            crate::html::html_search_meta(&webpage, "citation_publication_date"),
            "publish date",
        )
        .and_then(|date| crate::dates::unified_strdate(&date, true));
        // `int(_html_search_regex(..., fatal=False))`: `int(None)` escapes.
        let comment_count = warn(
            COMMENTS.captures(&webpage).map(|found| found[1].to_owned()),
            "comment count",
        )
        .ok_or_else(|| {
            Error::Reported(
                "int() argument must be a string, a bytes-like object or a real number, not 'NoneType'"
                    .into(),
            )
        })?;
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
        info.metadata.thumbnail = thumbnail;
        info.metadata.description = description;
        match publish_date {
            Some(date) => crate::metadata_parser::set_field(&mut info, "upload_date", date.into()),
            None => {
                info.extra.insert("upload_date".into(), Value::Null);
            }
        }
        let comment_count: u64 = comment_count
            .parse()
            .map_err(|_| Error::UnsupportedHtml5("oversized JoVE comment counts".into()))?;
        crate::metadata_parser::set_field(&mut info, "comment_count", comment_count.into());
        Ok(Some(crate::info_order::mark_info(info, "jove")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://www.jove.com/video/2744/electrode-positioning-montage-transcranial-direct-current"
        ));
        assert!(!suitable("http://www.jove.com/video/"));
    }
}

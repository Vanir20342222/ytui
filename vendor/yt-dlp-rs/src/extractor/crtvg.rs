//! Upstream `CrtvgIE` (`yt_dlp/extractor/crtvg.py`): a page's `var url`
//! stream base as non-fatal HLS and DASH formats with its meta tags.
use super::{Extractor, download_webpage, extract_m3u8_optional, extract_mpd_optional};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Crtvg;

const NAME: &str = "Crtvg";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?crtvg\.es/tvg/a-carta/(?P<id>[^/#?]+)").unwrap()
});
static VIDEO_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"var\s+url\s*=\s*["']([^"']+)"#).unwrap());
static OLD_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[^/#?]+-(?P<old_id>\d{7})$").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_html_search_meta(names, webpage, default=None)`: the first name with
/// a match.
fn meta(webpage: &str, names: &[&str]) -> Option<String> {
    names
        .iter()
        .find_map(|name| crate::html::html_search_meta(webpage, name))
}

impl Extractor for Crtvg {
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
        let video_url = VIDEO_URL
            .captures(&webpage)
            .map(|found| found[1].to_owned())
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {id}: Unable to extract video url; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let mut formats = Vec::new();
        let hls = Url::parse(&format!("{video_url}/playlist.m3u8"))?;
        if let Some(mut stream) = extract_m3u8_optional(client, &hls, NAME, &id, None, false) {
            super::hls::without_ext(&mut stream);
            formats.extend(stream.formats);
        }
        let dash = Url::parse(&format!("{video_url}/manifest.mpd"))?;
        if let Some(stream) = extract_mpd_optional(client, &dash, NAME, &id, false) {
            formats.extend(stream.formats);
        }

        // `make_archive_id(self, old_id)`.
        let old_ids = OLD_ID.captures(&id).map_or(Value::Null, |found| {
            serde_json::json!([format!("crtvg {}", &found["old_id"])])
        });
        let title = meta(&webpage, &["og:title", "twitter:title"]).map(|title| {
            title
                .strip_suffix(" | CRTVG")
                .map_or_else(|| title.clone(), str::to_owned)
        });
        let no_formats = formats.is_empty();
        let first = formats
            .first()
            .map_or_else(|| url.clone(), |format| format.url.clone());
        let mut info = super::direct::build_at(first.clone(), &first, String::new());
        info.formats = formats;
        if no_formats {
            info.extra
                .insert(crate::info::NO_FORMATS.into(), Value::Bool(true));
        }
        info.direct = None;
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
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        info.metadata.description = meta(&webpage, &["description"]);
        info.metadata.thumbnail = meta(&webpage, &["og:image", "twitter:image"]);
        info.extra.insert("_old_archive_ids".into(), old_ids);
        Ok(Some(crate::info_order::mark_info(info, "crtvg")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.crtvg.es/tvg/a-carta/os-caimans-do-tea-5839623"
        ));
        assert!(!suitable("https://www.crtvg.es/tvg/"));
    }
}

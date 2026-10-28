//! Upstream `EbayIE` (`yt_dlp/extractor/ebay.py`): an item page's `"video"`
//! JSON playlist map as non-fatal HLS and DASH formats.
use super::{
    Extractor, download_webpage, extract_m3u8_optional, extract_mpd_optional, search_json,
    search_warning,
};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Ebay;

const NAME: &str = "Ebay";

static VALID_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://(?:www\.)?ebay\.com/itm/(?P<id>\d+)").unwrap());
static VIDEO_JSON: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""video":"#).unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for Ebay {
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
        let video = match search_json(&VIDEO_JSON, &webpage, false) {
            Some(Some(video)) => video,
            Some(None) => {
                return Err(Error::UnsupportedHtml5(
                    "eBay video JSON that does not parse is not supported natively".into(),
                ));
            }
            None => {
                return Err(Error::Reported(format!(
                    "[{NAME}] {id}: Unable to extract video json; {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        let playlist_map = match video.get("playlistMap") {
            Some(Value::Object(map)) => map.clone(),
            Some(_) => {
                return Err(Error::UnsupportedHtml5(
                    "eBay playlist maps that are not objects are not supported natively".into(),
                ));
            }
            None => {
                return Err(Error::Reported(format!(
                    "{id}: An extractor error has occurred. (caused by KeyError('playlistMap')); {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        let mut formats = Vec::new();
        for (key, manifest) in playlist_map {
            if !matches!(key.as_str(), "HLS" | "DASH") {
                crate::screen::warning(&format!("[{NAME}] {id}: Unsupported format {key}"));
                continue;
            }
            let Some(manifest) = manifest.as_str() else {
                return Err(Error::UnsupportedHtml5(
                    "eBay manifest URLs that are not strings are not supported natively".into(),
                ));
            };
            let manifest = Url::parse(manifest)?;
            if key == "HLS" {
                if let Some(mut stream) =
                    extract_m3u8_optional(client, &manifest, NAME, &id, None, false)
                {
                    super::hls::without_ext(&mut stream);
                    formats.extend(stream.formats);
                }
            } else if let Some(stream) = extract_mpd_optional(client, &manifest, NAME, &id, false) {
                formats.extend(stream.formats);
            }
        }
        // `remove_end(self._html_extract_title(webpage), ' | eBay')`.
        let title = crate::html::html_extract_title(&webpage).map(|title| {
            title
                .strip_suffix(" | eBay")
                .map_or_else(|| title.clone(), str::to_owned)
        });
        if title.is_none() {
            search_warning(NAME, "title");
        }
        let no_formats = formats.is_empty();
        let first = formats
            .first()
            .map_or_else(|| url.clone(), |format| format.url.clone());
        let mut info = super::direct::build_at(first.clone(), &first, String::new());
        info.formats = formats;
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
        if no_formats {
            info.extra
                .insert(crate::info::NO_FORMATS.into(), Value::Bool(true));
        }
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        Ok(Some(crate::info_order::mark_info(info, "ebay")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://www.ebay.com/itm/194509326719"));
        assert!(!suitable("https://www.ebay.com/b/x"));
    }
}

//! Upstream `HistoricFilmsIE` (`yt_dlp/extractor/historicfilms.py`): a tape
//! page's tape ID, OpenGraph/meta fields, and the derived `.mov` URL as an
//! untyped result.
use super::{Extractor, download_webpage, search_warning};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct HistoricFilms;

const NAME: &str = "HistoricFilms";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?historicfilms\.com/(?:tapes/|play)(?P<id>\d+)").unwrap()
});

static TAPE_ID: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        Regex::new(r#"class="tapeId"[^>]*>([^<]+)<"#).unwrap(),
        Regex::new(r#"tapeId\s*:\s*"([^"]+)""#).unwrap(),
    ]
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for HistoricFilms {
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
        let tape_id = TAPE_ID
            .iter()
            .find_map(|regex| regex.captures(&webpage).map(|found| found[1].to_owned()))
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {id}: Unable to extract tape id; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        // Non-fatal searches warn when they find nothing.
        let search = |value: Option<String>, name: &str| {
            if value.is_none() {
                search_warning(NAME, name);
            }
            value
        };
        let title = search(
            crate::html::og_search_property(&webpage, "title"),
            "OpenGraph title",
        );
        let description = search(
            crate::html::og_search_property(&webpage, "description"),
            "OpenGraph description",
        );
        let thumbnail = search(
            crate::html::html_search_meta(&webpage, "thumbnailUrl"),
            "thumbnails",
        )
        .filter(|thumbnail| !thumbnail.is_empty())
        .or_else(|| {
            search(
                crate::html::og_search_property(&webpage, "image"),
                "thumbnail URL",
            )
        });
        let duration = search(
            crate::html::html_search_meta(&webpage, "duration"),
            "duration",
        )
        .and_then(|duration| crate::parse_duration(&duration));
        let media = Url::parse(&format!(
            "http://www.historicfilms.com/video/{tape_id}_{id}_web.mov"
        ))?;
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
        info.metadata.description = description;
        info.metadata.thumbnail = thumbnail;
        info.metadata.duration = duration;
        Ok(Some(crate::info_order::mark_info(info, "historicfilms")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("http://www.historicfilms.com/tapes/4728"));
        assert!(suitable("https://historicfilms.com/play9"));
        assert!(!suitable("https://historicfilms.com/tapes/"));
    }
}

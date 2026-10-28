//! Upstream `ThisAmericanLifeIE` (`yt_dlp/extractor/thisamericanlife.py`):
//! the episode's 64k HLS stream as an untyped `m4a` result with the page's
//! meta title, description and OpenGraph image.
use super::{Extractor, download_webpage, search_warning};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use std::sync::LazyLock;
use url::Url;

pub struct ThisAmericanLife;

const NAME: &str = "ThisAmericanLife";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?thisamericanlife\.org/(?:radio-archives/episode/|play_full\.php\?play=)(?P<id>\d+)",
    )
    .unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for ThisAmericanLife {
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
        let page = Url::parse(&format!(
            "http://www.thisamericanlife.org/radio-archives/episode/{id}"
        ))?;
        let webpage = download_webpage(client, &page, NAME, &id)?;
        let title = crate::html::html_search_meta(&webpage, "twitter:title").ok_or_else(|| {
            Error::Reported(format!(
                "[{NAME}] {id}: Unable to extract title; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
        let description = crate::html::html_search_meta(&webpage, "description");
        if description.is_none() {
            search_warning(NAME, "description");
        }
        let thumbnail = crate::html::og_search_property(&webpage, "image");
        if thumbnail.is_none() {
            search_warning(NAME, "thumbnail URL");
        }
        let media = Url::parse(&format!(
            "http://stream.thisamericanlife.org/{id}/stream/{id}_64k.m3u8"
        ))?;
        let mut info = super::direct::with_format(
            super::direct::build_at(media.clone(), &media, "m4a".into()),
            "",
        );
        for format in &mut info.formats {
            format.ext = "m4a".into();
            format.protocol = Some("m3u8_native".into());
            format.acodec = Some("aac".into());
            format.vcodec = Some("none".into());
            format.abr = Some(64.0);
            format.format_id = String::new();
        }
        info.protocol = Some("m3u8_native".into());
        info.acodec = Some("aac".into());
        info.vcodec = Some("none".into());
        info.abr = Some(64.0);
        info.format_id = None;
        info.id = id;
        info.title = title;
        info.webpage_url = url.clone();
        info.direct = None;
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        info.metadata.description = description;
        info.metadata.thumbnail = thumbnail;
        Ok(Some(crate::info_order::mark_info(info, "thisamericanlife")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://www.thisamericanlife.org/radio-archives/episode/487/harper-high-school-part-one"
        ));
        assert!(suitable(
            "http://www.thisamericanlife.org/play_full.php?play=487"
        ));
        assert!(!suitable("http://www.thisamericanlife.org/about"));
    }
}

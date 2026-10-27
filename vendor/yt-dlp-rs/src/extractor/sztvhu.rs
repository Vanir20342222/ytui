//! Upstream `SztvHuIE` (`yt_dlp/extractor/sztvhu.py`): the page's player
//! file under `media.sztv.hu/vod/` with its meta title and description.
use super::{Extractor, download_webpage, search_warning};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use std::sync::LazyLock;
use url::Url;

pub struct SztvHu;

const NAME: &str = "SztvHu";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:(?:www\.)?sztv\.hu|www\.tvszombathely\.hu)/(?:[^/]+)/.+-(?P<id>[0-9]+)",
    )
    .unwrap()
});

static FILE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"file: "...:(.*?)","#).unwrap());
static TITLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<meta name="title" content="([^"]*?) - [^-]*? - [^-]*?""#).unwrap()
});
static DESCRIPTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<meta name="description" content="([^"]*)"/>"#).unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for SztvHu {
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
        let fatal = |what: &str| {
            Error::Reported(format!(
                "[{NAME}] {id}: Unable to extract {what}; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        };
        let file = FILE
            .captures(&webpage)
            .map(|found| found[1].to_owned())
            .ok_or_else(|| fatal("video file"))?;
        let title = TITLE
            .captures(&webpage)
            .map(|found| crate::html::clean_html(&found[1]))
            .ok_or_else(|| fatal("video title"))?;
        let description = DESCRIPTION
            .captures(&webpage)
            .map(|found| crate::html::clean_html(&found[1]));
        if description.is_none() {
            search_warning(NAME, "video description");
        }
        let thumbnail = crate::html::og_search_property(&webpage, "image");
        if thumbnail.is_none() {
            search_warning(NAME, "thumbnail URL");
        }
        let media = Url::parse(&format!("http://media.sztv.hu/vod/{file}"))?;
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
        info.title = title;
        info.webpage_url = url.clone();
        info.direct = None;
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        info.metadata.description = description;
        info.metadata.thumbnail = thumbnail;
        Ok(Some(crate::info_order::mark_info(info, "sztvhu")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("http://sztv.hu/hirek/cserkeszek-20130909"));
        assert!(suitable("http://www.tvszombathely.hu/x/bare-2"));
        assert!(!suitable("http://sztv.hu/hirek"));
    }
}

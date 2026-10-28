//! Upstream `FuyinTVIE` (`yt_dlp/extractor/fuyintv.py`): the movie API's
//! URL as an untyped MP4 result with the page's description meta.
use super::{Extractor, download_json, download_webpage, search_warning};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct FuyinTv;

const NAME: &str = "FuyinTV";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?fuyin\.tv/html/(?:\d+)/(?P<id>\d+)\.html").unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for FuyinTv {
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
        let api = Url::parse(&format!(
            "https://www.fuyin.tv/api/api/tv.movie/url?urlid={id}"
        ))?;
        let json = download_json(client, &api, NAME, &id)?;
        // `_download_webpage(url, video_id, fatal=False)`: `False` on failure.
        let webpage = match download_webpage(client, url, NAME, &id) {
            Ok(webpage) => Some(webpage),
            Err(Error::ExtractorRequest { source, .. }) => {
                crate::screen::warning(&format!(
                    "[{NAME}] {id}: Unable to download webpage: {}",
                    crate::upstream_error_text(&source)
                ));
                None
            }
            Err(error) => return Err(error),
        };
        let data = json.get("data");
        let title = data
            .and_then(|data| data.get("title"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let media = data
            .and_then(|data| data.as_object()?.get("url"))
            .ok_or_else(|| {
                Error::Reported(format!(
                    "{id}: An extractor error has occurred. (caused by KeyError('{}')); {}",
                    if data.is_some() { "url" } else { "data" },
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let Some(media) = media.as_str() else {
            return Err(Error::UnsupportedHtml5(
                "Fuyin TV URLs that are not strings are not supported natively".into(),
            ));
        };
        // `_html_search_meta('description', False)` searches a bool.
        let Some(webpage) = webpage else {
            return Err(Error::Reported(
                "expected string or bytes-like object, got 'bool'".into(),
            ));
        };
        let description = crate::html::html_search_meta(&webpage, "description");
        if description.is_none() {
            search_warning(NAME, "description");
        }
        let media = Url::parse(media)?;
        let mut info = super::direct::with_format(
            super::direct::build_at(media.clone(), &media, "mp4".into()),
            "",
        );
        for format in &mut info.formats {
            format.ext = "mp4".into();
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
        Ok(Some(crate::info_order::mark_info(info, "fuyintv")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://www.fuyin.tv/html/2733/44129.html"));
        assert!(!suitable("https://www.fuyin.tv/html/44129.html"));
    }
}

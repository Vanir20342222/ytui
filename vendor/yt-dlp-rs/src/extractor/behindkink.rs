//! Upstream `BehindKinkIE` (`yt_dlp/extractor/behindkink.py`, marked
//! broken): a post's `<source>` URL and OpenGraph fields as an untyped
//! result dated by the URL.
use super::{Extractor, broken_warning, download_webpage, search_warning};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct BehindKink;

const NAME: &str = "BehindKink";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?behindkink\.com/(?P<year>[0-9]{4})/(?P<month>[0-9]{2})/(?P<day>[0-9]{2})/(?P<id>[^/#?_]+)",
    )
    .unwrap()
});

static SOURCE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"<source src="([^"]+)""#).unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for BehindKink {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = found["id"].to_owned();
        let upload_date = format!("{}{}{}", &found["year"], &found["month"], &found["day"]);
        broken_warning();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &display_id)?;
        let media = SOURCE
            .captures(&webpage)
            .map(|found| found[1].to_owned())
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {display_id}: Unable to extract video URL; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let media = url.join(&media)?;
        // `url_basename(video_url).split('_')[0]`.
        let basename = media
            .path()
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or_default();
        let id = basename.split('_').next().unwrap_or_default().to_owned();
        let search = |prop: &str, name: &str| {
            let value = crate::html::og_search_property(&webpage, prop);
            if value.is_none() {
                search_warning(NAME, name);
            }
            value
        };
        let title = search("title", "OpenGraph title");
        let thumbnail = search("image", "thumbnail URL");
        let description = search("description", "OpenGraph description");
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
        info.metadata.upload_date = Some(upload_date);
        info.metadata.age_limit = Some(18);
        info.extra.insert("display_id".into(), display_id.into());
        Ok(Some(crate::info_order::mark_info(info, "behindkink")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://www.behindkink.com/2014/12/05/what-are-you-passionate-about-marley-blaze/"
        ));
        assert!(!suitable("http://www.behindkink.com/2014/12/what"));
    }
}

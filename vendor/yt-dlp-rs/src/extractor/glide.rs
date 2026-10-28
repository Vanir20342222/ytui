//! Upstream `GlideIE` (`yt_dlp/extractor/glide.py`): a share page's
//! `<source>` (else OpenGraph video) URL as an untyped result.
use super::{Extractor, download_webpage, proto_relative, search_warning};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Glide;

const NAME: &str = "Glide";

static VALID_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://share\.glide\.me/(?P<id>[A-Za-z0-9\-=_+]+)").unwrap());

static SOURCE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<source[^>]+src=(?:"(?P<a>.+?)"|'(?P<b>.+?)')"#).unwrap());

static THUMBNAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<img[^>]+id=["']video-thumbnail["'][^>]+src=(?:"(?P<a>.+?)"|'(?P<b>.+?)')"#)
        .unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// A quoted `url` group of `_search_regex(..., default=None)`.
fn quoted(regex: &Regex, html: &str) -> Option<String> {
    let found = regex.captures(html)?;
    let value = found.name("a").or_else(|| found.name("b"))?.as_str();
    (!value.is_empty()).then(|| proto_relative(value))
}

impl Extractor for Glide {
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
        // `_generic_title('', webpage)`.
        let title = super::html5::page_title(&webpage);
        // `_og_search_video_url`: `video:secure_url`, `video`, `video:url`.
        let media = quoted(&SOURCE, &webpage)
            .or_else(|| {
                ["video:secure_url", "video", "video:url"]
                    .into_iter()
                    .find_map(|prop| crate::html::og_search_property(&webpage, prop))
            })
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {id}: Unable to extract video url; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let thumbnail = quoted(&THUMBNAIL, &webpage).or_else(|| {
            let og = crate::html::og_search_property(&webpage, "image");
            if og.is_none() {
                search_warning(NAME, "thumbnail URL");
            }
            og
        });
        let media = Url::parse(&media)?;
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
        Ok(Some(crate::info_order::mark_info(info, "glide")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("http://share.glide.me/UZF8zlmuQbe4mr+7dCiQ0w=="));
        assert!(!suitable("http://glide.me/UZF8"));
    }
}

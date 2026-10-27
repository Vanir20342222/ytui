//! Upstream `MovingImageIE` (`yt_dlp/extractor/movingimage.py`): a film
//! page's player HLS manifest and its labelled fields.
use super::{Extractor, download_webpage, extract_m3u8_fatal, search_warning};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use std::sync::LazyLock;
use url::Url;

pub struct MovingImage;

const NAME: &str = "MovingImage";

static VALID_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://movingimage\.nls\.uk/film/(?P<id>\d+)").unwrap());
static MANIFEST: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"file\s*:\s*"([^"]+)""#).unwrap());
static THUMBNAIL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"image\s*:\s*'([^']+)'").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `search_field(field_name)`'s `field_title`/`field_content` span pair.
fn search_field(webpage: &str, field: &str) -> Option<String> {
    let pattern = format!(
        r#"<span\s+class="field_title">{}:</span>\s*<span\s+class="field_content">([^<]+)</span>"#,
        regex::escape(field)
    );
    Regex::new(&pattern)
        .ok()?
        .captures(webpage)
        .map(|found| found[1].to_owned())
}

impl Extractor for MovingImage {
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
        let manifest = MANIFEST
            .captures(&webpage)
            .map(|found| crate::html::clean_html(&found[1]).trim().to_owned())
            .ok_or_else(|| fatal("m3u8 manifest URL"))?;
        let manifest = Url::parse(&manifest)?;
        let mut info = extract_m3u8_fatal(client, &manifest, NAME, &id, None, false)?;
        // Every field search reports itself as `title`.
        let optional = |field: &str| {
            let value = search_field(&webpage, field);
            if value.is_none() {
                search_warning(NAME, "title");
            }
            value
        };
        let title = search_field(&webpage, "Title").ok_or_else(|| fatal("title"))?;
        let title = crate::html::unescape_html(&title)
            .trim_matches(|c| matches!(c, '(' | ')' | '[' | ']'))
            .to_owned();
        let description = optional("Description").map(|text| crate::html::unescape_html(&text));
        let duration = optional("Running time").and_then(|text| crate::parse_duration(&text));
        let thumbnail = THUMBNAIL
            .captures(&webpage)
            .map(|found| found[1].to_owned());
        if thumbnail.is_none() {
            search_warning(NAME, "thumbnail");
        }
        info.id = id;
        info.title = title;
        info.webpage_url = url.clone();
        info.direct = None;
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        info.metadata.description = description;
        info.metadata.duration = duration;
        info.metadata.thumbnail = thumbnail;
        Ok(Some(crate::info_order::mark_info(info, "movingimage")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("http://movingimage.nls.uk/film/3561"));
        assert!(!suitable("http://movingimage.nls.uk/"));
    }
}

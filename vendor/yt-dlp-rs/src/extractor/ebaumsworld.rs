//! Upstream `EbaumsWorldIE` (`yt_dlp/extractor/ebaumsworld.py`): the
//! player config XML as an untyped result.
use super::{Extractor, download_xml};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct EbaumsWorld;

const NAME: &str = "EbaumsWorld";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?ebaumsworld\.com/videos/[^/]+/(?P<id>\d+)").unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for EbaumsWorld {
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
        let config = Url::parse(&format!("http://www.ebaumsworld.com/video/player/{id}"))?;
        let text = download_xml(client, &config, NAME, &id)?;
        let document = roxmltree::Document::parse(text.trim_start_matches('\u{feff}'))
            .map_err(|error| Error::Reported(format!("[{NAME}] {id}: {error}")))?;
        let root = document.root_element();
        // `config.find(tag).text`: a missing child escapes as an
        // `AttributeError`; an empty one is `None`.
        let field = |tag: &str| -> Result<Option<String>> {
            let child = root
                .children()
                .find(|child| child.is_element() && child.tag_name().name() == tag)
                .ok_or_else(|| {
                    Error::Reported("'NoneType' object has no attribute 'text'".into())
                })?;
            let mut text = String::new();
            for node in child.children() {
                if node.is_element() {
                    break;
                }
                text.push_str(node.text().unwrap_or_default());
            }
            Ok((!text.is_empty()).then_some(text))
        };
        let media = field("file")?;
        let title = field("title")?;
        let description = field("description")?;
        let thumbnail = field("image")?;
        let uploader = field("username")?;
        let media = media.ok_or_else(|| {
            Error::UnsupportedHtml5(
                "EbaumsWorld configs without a file URL are not supported natively".into(),
            )
        })?;
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
        info.metadata.description = description;
        info.metadata.thumbnail = thumbnail;
        info.extra
            .insert("uploader".into(), uploader.map_or(Value::Null, Value::from));
        Ok(Some(crate::info_order::mark_info(info, "ebaumsworld")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://www.ebaumsworld.com/videos/a-giant-python-opens-the-door/83367677/"
        ));
        assert!(!suitable("http://www.ebaumsworld.com/videos/83367677"));
    }
}

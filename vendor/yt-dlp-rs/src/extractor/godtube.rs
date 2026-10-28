//! Upstream `GodTubeIE` (`yt_dlp/extractor/godtube.py`, marked broken): the
//! player config and media XML as an untyped result.
use super::{Extractor, broken_warning, download_xml_note, find_text};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct GodTube;

const NAME: &str = "GodTube";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?godtube\.com/watch/\?v=(?P<id>[\da-zA-Z]+)").unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn parse(text: &str, id: &str) -> Result<String> {
    roxmltree::Document::parse(text.trim_start_matches('\u{feff}'))
        .map_err(|error| Error::Reported(format!("[{NAME}] {id}: {error}")))?;
    Ok(text.to_owned())
}

impl Extractor for GodTube {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        broken_warning();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let config_url = Url::parse(&format!(
            "http://www.godtube.com/resource/mediaplayer/{}.xml",
            id.to_lowercase()
        ))?;
        let config = parse(
            &download_xml_note(
                client,
                &config_url,
                NAME,
                &id,
                "Downloading player config XML",
            )?,
            &id,
        )?;
        let config = roxmltree::Document::parse(config.trim_start_matches('\u{feff}'))
            .map_err(|error| Error::Reported(error.to_string()))?;
        let root = config.root_element();
        let media = find_text(root, "file")?;
        let uploader = find_text(root, "author")?;
        let timestamp =
            find_text(root, "date")?.and_then(|date| crate::dates::parse_iso8601(&date));
        let duration =
            find_text(root, "duration")?.and_then(|duration| crate::parse_duration(&duration));
        let thumbnail = find_text(root, "image")?;
        let media_url = Url::parse(&format!("http://www.godtube.com/media/xml/?v={id}"))?;
        let details = download_xml_note(client, &media_url, NAME, &id, "Downloading media XML")?;
        let details = roxmltree::Document::parse(details.trim_start_matches('\u{feff}'))
            .map_err(|error| Error::Reported(format!("[{NAME}] {id}: {error}")))?;
        let title = find_text(details.root_element(), "title")?;
        let media = media.ok_or_else(|| {
            Error::UnsupportedHtml5(
                "GodTube configs without a file URL are not supported natively".into(),
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
        info.metadata.thumbnail = thumbnail;
        info.metadata.timestamp = timestamp.map(crate::info::PyNumber::Int);
        info.metadata.duration = duration;
        info.extra
            .insert("uploader".into(), uploader.map_or(Value::Null, Value::from));
        Ok(Some(crate::info_order::mark_info(info, "godtube")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://www.godtube.com/watch/?v=0wwWUv"));
        assert!(!suitable("https://www.godtube.com/watch/0wwWUv"));
    }
}

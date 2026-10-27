//! Upstream `MaarivIE` (`yt_dlp/extractor/maariv.py`): the Walla media API's
//! HLS stream and HTTP stream URLs, and the player's iframe embeds.
use super::{Extractor, download_json, extract_m3u8_optional};
use crate::{Error, Result, VideoFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct Maariv;

pub(crate) const NAME: &str = "maariv.co.il";
pub(crate) const KEY: &str = "Maariv";

const VALID: &str =
    r"https?://player\.maariv\.co\.il/public/player\.html\?(?:[^#]+&)?media=(?P<id>\d+)";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!("^{VALID}")).unwrap());
static EMBED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r#"<iframe[^>]+\bsrc=['"](?P<url>{VALID})"#)).unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_match_id`'s ID of a suitable URL.
pub(crate) fn temp_id(url: &str) -> Option<String> {
    VALID_URL.captures(url).map(|found| found["id"].to_owned())
}

/// `_extract_embed_urls` under `orderedSet`.
pub(crate) fn embed_urls(url: &Url, webpage: &str) -> Vec<Url> {
    let mut found: Vec<Url> = Vec::new();
    for captures in EMBED.captures_iter(webpage) {
        let Ok(embed) = url.join(&crate::html::unescape_html(&captures["url"])) else {
            continue;
        };
        if suitable(embed.as_str()) && !found.contains(&embed) {
            found.push(embed);
        }
    }
    found
}

impl Extractor for Maariv {
    fn key(&self) -> &'static str {
        KEY
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
            "https://dal.walla.co.il/media/{id}?origin=player.maariv.co.il"
        ))?;
        let response = download_json(client, &api, NAME, &id)?;
        let data = response.get("data").ok_or_else(|| {
            Error::Reported(format!(
                "{id}: An extractor error has occurred. (caused by KeyError('data')); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
        let video = data.get("video");
        let mut formats = Vec::new();
        if let Some(hls) = video
            .and_then(|video| video.get("url"))
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
        {
            let manifest = Url::parse(&hls)?;
            if let Some(stream) =
                extract_m3u8_optional(client, &manifest, NAME, &id, Some("hls"), false)
            {
                formats.extend(stream.formats);
            }
        }
        // `('video', 'stream_urls', ..., 'stream_url', {url_or_none})`.
        let streams: Vec<String> = video
            .and_then(|video| video.get("stream_urls"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|stream| stream.get("stream_url")?.as_str())
            .filter_map(super::json_ld::url_or_none)
            .collect();
        for stream in streams {
            let media = Url::parse(&stream)?;
            let (width, height) = super::html5::parse_resolution(&stream);
            let mut format = Map::new();
            format.insert("url".into(), media.as_str().into());
            format.insert("format_id".into(), "http".into());
            if let Some(width) = width {
                format.insert("width".into(), width.into());
            }
            if let Some(height) = height {
                format.insert("height".into(), height.into());
            }
            let ext = super::direct::determine_ext(media.as_str())
                .unwrap_or_else(|| "unknown_video".into());
            format.insert("ext".into(), ext.into());
            let mut format: VideoFormat = serde_json::from_value(Value::Object(format))
                .map_err(|error| Error::Reported(format!("[{NAME}] {id}: {error}")))?;
            crate::info_order::mark_format(&mut format, "maariv");
            formats.push(format);
        }
        let text = |value: Option<&Value>| value.and_then(Value::as_str).map(str::to_owned);
        let title = text(data.get("title"));
        let duration =
            super::streamable::int_or_none(video.and_then(|video| video.get("duration")));
        let timestamp = text(data.get("upload_date"))
            .and_then(|date| crate::dates::unified_timestamp(&date, true));
        let no_formats = formats.is_empty();
        let first = formats
            .first()
            .map_or_else(|| url.clone(), |format| format.url.clone());
        let mut info = super::direct::build_at(first.clone(), &first, String::new());
        info.formats = formats;
        info.direct = None;
        let shape = if title.is_some() {
            "maariv"
        } else {
            "maariv-untitled"
        };
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
        info.extractor_key = KEY.into();
        if !duration.is_null() {
            crate::metadata_parser::set_field(&mut info, "duration", duration);
        }
        if let Some(timestamp) = timestamp {
            crate::metadata_parser::set_field(&mut info, "timestamp", timestamp.into());
        }
        Ok(Some(crate::info_order::mark_info(info, shape)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://player.maariv.co.il/public/player.html?player=maariv-desktop&media=3611585"
        ));
        assert!(!suitable(
            "https://player.maariv.co.il/public/player.html?player=x"
        ));
    }
}

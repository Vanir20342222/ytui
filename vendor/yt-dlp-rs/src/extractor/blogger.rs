//! Upstream `BloggerIE` (`yt_dlp/extractor/blogger.py`, `blogger.com`): the
//! player page's `VIDEO_CONFIG` (after `unicode_escape`) streams as formats,
//! and its `_EMBED_REGEX` iframes on other pages.
use super::{Extractor, download_webpage};
use crate::{Error, Result, VideoFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Blogger;

pub(crate) const NAME: &str = "blogger.com";
pub(crate) const KEY: &str = "Blogger";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?blogger\.com/video\.g\?token=(?P<id>.+)").unwrap()
});
static EMBED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<iframe[^>]+src=["'](?P<url>(?:https?:)?//(?:www\.)?blogger\.com/video\.g\?token=[^"']+)["']"#)
        .unwrap()
});
static CONFIG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"var\s+VIDEO_CONFIG\s*=\s*(\{.*)").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_match_id`'s ID.
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

/// `text.encode().decode('unicode_escape')`: backslash escapes decoded and
/// every other UTF-8 byte read as a Latin-1 character.
fn unicode_escape(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(bytes.len());
    let mut index = 0;
    let hex = |digits: &[u8]| -> Option<u32> {
        u32::from_str_radix(std::str::from_utf8(digits).ok()?, 16).ok()
    };
    while index < bytes.len() {
        let byte = bytes[index];
        if byte != b'\\' {
            out.push(char::from(byte));
            index += 1;
            continue;
        }
        let next = *bytes.get(index + 1)?;
        index += 2;
        let simple = match next {
            b'\n' => Some(None),
            b'\\' => Some(Some('\\')),
            b'\'' => Some(Some('\'')),
            b'"' => Some(Some('"')),
            b'a' => Some(Some('\x07')),
            b'b' => Some(Some('\x08')),
            b'f' => Some(Some('\x0c')),
            b'n' => Some(Some('\n')),
            b'r' => Some(Some('\r')),
            b't' => Some(Some('\t')),
            b'v' => Some(Some('\x0b')),
            _ => None,
        };
        if let Some(simple) = simple {
            out.extend(simple);
            continue;
        }
        let (length, digits_start) = match next {
            b'x' => (2, index),
            b'u' => (4, index),
            b'U' => (8, index),
            b'0'..=b'7' => {
                let mut end = index - 1;
                while end < bytes.len() && end < index + 2 && (b'0'..=b'7').contains(&bytes[end]) {
                    end += 1;
                }
                let value =
                    u32::from_str_radix(std::str::from_utf8(&bytes[index - 1..end]).ok()?, 8)
                        .ok()?;
                out.push(char::from_u32(value)?);
                index = end;
                continue;
            }
            // Unknown escapes stay as written.
            other => {
                out.push('\\');
                out.push(char::from(other));
                continue;
            }
        };
        let digits = bytes.get(digits_start..digits_start + length)?;
        out.push(char::from_u32(hex(digits)?)?);
        index = digits_start + length;
    }
    Some(out)
}

/// `traverse_obj(parse_qs(url), (key, 0))`.
fn query_value(url: &str, key: &str) -> Option<String> {
    let query = url.split_once('?')?.1.split('#').next()?;
    url::form_urlencoded::parse(query.as_bytes())
        .find(|(name, value)| name == key && !value.is_empty())
        .map(|(_, value)| value.into_owned())
}

impl Extractor for Blogger {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(token) = temp_id(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &token)?;
        let config = CONFIG
            .captures(&webpage)
            .map(|found| found[1].to_owned())
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {token}: Unable to extract JSON data; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let decoded = unicode_escape(&config).ok_or_else(|| {
            Error::Reported(format!("[{NAME}] {token}: invalid escape in JSON data"))
        })?;
        let data: Value = serde_json::from_str(&decoded).map_err(|error| {
            Error::Reported(format!(
                "[{NAME}] {token}: Failed to parse JSON  (caused by {error})"
            ))
        })?;
        let key_error = |key: &str| {
            Error::Reported(format!(
                "{token}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        };
        let streams = data
            .get("streams")
            .and_then(Value::as_array)
            .ok_or_else(|| key_error("streams"))?;
        let mut formats = Vec::new();
        for stream in streams {
            let play_url = stream
                .get("play_url")
                .and_then(Value::as_str)
                .ok_or_else(|| key_error("play_url"))?;
            let ext = query_value(play_url, "mime")
                .map(|mime| super::direct::mimetype2ext_or(&mime, None))
                .filter(|ext| !ext.is_empty())
                .or_else(|| super::direct::determine_ext(play_url))
                .unwrap_or_else(|| "unknown_video".into());
            let format_id = match stream.get("format_id") {
                Some(Value::String(text)) => text.clone(),
                Some(Value::Number(number)) => number.to_string(),
                _ => String::new(),
            };
            let mut format: VideoFormat = serde_json::from_value(json!({
                "ext": ext,
                "url": play_url,
                "format_id": format_id,
            }))
            .map_err(|error| Error::Reported(format!("[{NAME}] {token}: {error}")))?;
            crate::info_order::mark_format(&mut format, "blogger");
            formats.push(format);
        }
        let first = streams
            .first()
            .ok_or_else(|| Error::Reported("list index out of range".into()))?;
        let duration = query_value(first["play_url"].as_str().unwrap_or_default(), "dur")
            .and_then(|duration| crate::parse_duration(&duration));
        let id = match data.get("iframe_id") {
            Some(Value::String(id)) => id.clone(),
            _ => token,
        };
        let media = formats[0].url.clone();
        let mut info = super::direct::build_at(media.clone(), &media, String::new());
        info.formats = formats;
        info.direct = None;
        info.title = id.clone();
        info.id = id;
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = KEY.into();
        info.metadata.thumbnail = data
            .get("thumbnail")
            .and_then(Value::as_str)
            .map(str::to_owned);
        info.metadata.duration = duration;
        Ok(Some(crate::info_order::mark_info(info, "blogger")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_escape_matches_python() {
        assert_eq!(unicode_escape(r"a&b\x41\n\\").as_deref(), Some("a&bA\n\\"));
        assert_eq!(unicode_escape("é").as_deref(), Some("\u{c3}\u{a9}"));
        assert!(suitable("https://www.blogger.com/video.g?token=AD6v5dz"));
    }
}

//! Upstream `BildIE` (`yt_dlp/extractor/bild.py`): the article's
//! `,view=json` clip data with its HLS and MP4 sources as formats.
use super::{Extractor, download_json, extract_m3u8_optional};
use crate::{Error, Result, VideoFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Bild;

const NAME: &str = "Bild";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?bild\.de/(?:[^/]+/)+(?P<display_id>[^/]+)-(?P<id>\d+)(?:,auto=true)?\.bild\.html").unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for Bild {
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
        let base = url.as_str().split(".bild.html").next().unwrap_or_default();
        let api = Url::parse(&format!("{base},view=json.bild.html"))?;
        let data = download_json(client, &api, NAME, &id)?;
        let mut formats = Vec::new();
        // `traverse_obj(data, ('clipList', 0, 'srces', lambda _, v: v['src']))`.
        let sources = data
            .get("clipList")
            .and_then(|clips| clips.get(0))
            .and_then(|clip| clip.get("srces"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for source in sources.iter().filter(|source| {
            source
                .get("src")
                .is_some_and(|src| src.as_str().is_some_and(|src| !src.is_empty()))
        }) {
            let src = source["src"].as_str().unwrap_or_default();
            match source.get("type").and_then(Value::as_str) {
                Some("application/x-mpegURL") => {
                    let manifest = Url::parse(src)?;
                    if let Some(stream) =
                        extract_m3u8_optional(client, &manifest, NAME, &id, Some("hls"), false)
                    {
                        formats.extend(stream.formats);
                    }
                }
                Some("video/mp4") => {
                    let ext =
                        super::direct::determine_ext(src).unwrap_or_else(|| "unknown_video".into());
                    let mut format: VideoFormat = serde_json::from_value(json!({
                        "url": src,
                        "format_id": "http-mp4",
                        "ext": ext,
                    }))
                    .map_err(|error| Error::Reported(format!("[{NAME}] {id}: {error}")))?;
                    crate::info_order::mark_format(&mut format, "bild");
                    formats.push(format);
                }
                other => crate::screen::warning(&format!(
                    "[{NAME}] Skipping unsupported format type: \"{}\"",
                    other.unwrap_or("None")
                )),
            }
        }
        let title = data.get("title").and_then(Value::as_str).ok_or_else(|| {
            Error::Reported(format!(
                "{id}: An extractor error has occurred. (caused by KeyError('title')); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
        if formats.is_empty() {
            return Err(Error::Reported(format!(
                "[{NAME}] {id}: No video formats found!; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        }
        let first = formats[0].url.clone();
        let mut info = super::direct::build_at(first.clone(), &first, String::new());
        info.formats = formats;
        info.direct = None;
        info.id = id;
        info.title = crate::html::unescape_html(title).trim().to_owned();
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        info.metadata.description = data
            .get("description")
            .and_then(Value::as_str)
            .map(crate::html::unescape_html);
        info.metadata.thumbnail = data
            .get("poster")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let duration = super::streamable::int_or_none(data.get("durationSec"));
        // A `None` duration is dropped in processing.
        if !duration.is_null() {
            crate::metadata_parser::set_field(&mut info, "duration", duration);
        }
        Ok(Some(crate::info_order::mark_info(info, "bild")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://www.bild.de/video/clip/apple-ipad-air/das-koennen-die-neuen-ipads-38184146.bild.html"
        ));
        assert!(suitable(
            "https://www.bild.de/x/plain-2,auto=true.bild.html"
        ));
        assert!(!suitable("https://www.bild.de/news/"));
    }
}

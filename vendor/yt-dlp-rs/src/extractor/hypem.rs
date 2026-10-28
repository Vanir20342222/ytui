//! Upstream `HypemIE` (`yt_dlp/extractor/hypem.py`): a track page's
//! `displayList-data` JSON, then the served source's URL as an untyped
//! `mp3` result.
use super::{Extractor, download_json_note, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Hypem;

const NAME: &str = "Hypem";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?hypem\.com/track/(?P<id>[0-9a-z]{5})").unwrap()
});

static TRACKS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)<script\s+type="application/json"\s+id="displayList-data">(.+?)</script>"#)
        .unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// A string key of a dict, or upstream's `KeyError` extractor error.
fn text<'a>(object: &'a Value, key: &str, id: &str) -> Result<&'a str> {
    match object.get(key) {
        Some(Value::String(text)) => Ok(text),
        Some(_) => Err(Error::UnsupportedHtml5(format!(
            "non-string Hypem {key} values are not supported natively"
        ))),
        None => Err(Error::Reported(format!(
            "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
            crate::BUG_REPORTS_MESSAGE
        ))),
    }
}

impl Extractor for Hypem {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let url_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &url_id)?;
        // `_html_search_regex` cleans the JSON text before it is parsed.
        let tracks = TRACKS
            .captures(&webpage)
            .map(|found| crate::html::clean_html(&found[1]))
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {url_id}: Unable to extract tracks; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let data: Value = serde_json::from_str(&tracks).map_err(|error| {
            Error::Reported(format!(
                "[{NAME}] {url_id}: Failed to parse JSON  (caused by {error})"
            ))
        })?;
        let tracks = match data.get("tracks") {
            Some(Value::Array(tracks)) => tracks,
            Some(_) => {
                return Err(Error::UnsupportedHtml5(
                    "non-list Hypem tracks are not supported natively".into(),
                ));
            }
            None => {
                return Err(Error::Reported(format!(
                    "{url_id}: An extractor error has occurred. (caused by KeyError('tracks')); {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        // `['tracks'][0]` of an empty list escapes as a bare `IndexError`.
        let track = tracks
            .first()
            .ok_or_else(|| Error::Reported("list index out of range".into()))?;
        let track_id = text(track, "id", &url_id)?.to_owned();
        let title = text(track, "song", &url_id)?.to_owned();
        let key = text(track, "key", &url_id)?;
        let serve = Url::parse(&format!("http://hypem.com/serve/source/{track_id}/{key}"))?;
        let source = download_json_note(client, &serve, NAME, &track_id, "Downloading metadata")?;
        let media = Url::parse(text(&source, "url", &url_id)?)?;
        let mut info = super::direct::with_format(
            super::direct::build_at(media.clone(), &media, "mp3".into()),
            "",
        );
        for format in &mut info.formats {
            format.ext = "mp3".into();
            format.format_id = String::new();
        }
        info.format_id = None;
        info.id = track_id;
        info.title = title.clone();
        info.webpage_url = url.clone();
        info.direct = None;
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        for (key, value) in [
            (
                "uploader",
                track.get("artist").cloned().unwrap_or(Value::Null),
            ),
            (
                "duration",
                super::streamable::int_or_none(track.get("time")),
            ),
            ("timestamp", super::streamable::int_or_none(track.get("ts"))),
            ("track", title.into()),
        ] {
            if !value.is_null() {
                crate::metadata_parser::set_field(&mut info, key, value);
            }
        }
        Ok(Some(crate::info_order::mark_info(info, "hypem")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("http://hypem.com/track/1v6ga/BODYWORK+-+TAME"));
        assert!(!suitable("http://hypem.com/track/1V6GA"));
    }
}

//! Upstream `CJSWIE` (`yt_dlp/extractor/cjsw.py`): an episode page's
//! audio button source with its heading, description and show name.
use super::{Extractor, download_webpage};
use crate::{Error, Result, VideoFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Cjsw;

const NAME: &str = "CJSW";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?cjsw\.com/program/(?P<program>[^/]+)/episode/(?P<id>\d+)")
        .unwrap()
});

/// `(["\'])(?P<NAME>(?:(?!\1).)+)\1` as one alternative per quote.
fn quoted(prefix: &str) -> Regex {
    Regex::new(&format!(
        r#"{prefix}=(?:"(?P<a>[^"\n]+)"|'(?P<b>[^'\n]+)')"#
    ))
    .unwrap()
}

static TITLE_HEADER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<h1[^>]+class=["']episode-header__title["'][^>]*>(?P<title>[^<]+)"#).unwrap()
});
static TITLE_DATA: LazyLock<Regex> = LazyLock::new(|| quoted("data-audio-title"));
static AUDIO: LazyLock<Regex> = LazyLock::new(|| quoted("<button[^>]+data-audio-src"));
static AUDIO_ID: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"/([\da-f]{8}-[\da-f]{4}-[\da-f]{4}-[\da-f]{4}-[\da-f]{12})\.mp3").unwrap()
});
static DESCRIPTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<p>(?P<description>.+?)</p>").unwrap());
static SERIES: LazyLock<Regex> = LazyLock::new(|| quoted("data-showname"));

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// The first alternative's text of a [`quoted`] match.
fn quoted_text(regex: &Regex, text: &str) -> Option<String> {
    regex.captures(text).and_then(|found| {
        found
            .name("a")
            .or_else(|| found.name("b"))
            .map(|group| group.as_str().to_owned())
    })
}

impl Extractor for Cjsw {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let (program, episode_id) = (found["program"].to_owned(), found["id"].to_owned());
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &episode_id)?;
        let fatal = |what: &str| {
            Error::Reported(format!(
                "[{NAME}] {episode_id}: Unable to extract {what}; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        };
        // The first pattern that matches anywhere wins.
        let title = TITLE_HEADER
            .captures(&webpage)
            .map(|found| found["title"].to_owned())
            .or_else(|| quoted_text(&TITLE_DATA, &webpage))
            .ok_or_else(|| fatal("title"))?;
        let title = crate::html::unescape_html(&title);
        let audio = quoted_text(&AUDIO, &webpage).ok_or_else(|| fatal("audio url"))?;
        let id = AUDIO_ID.captures(&audio).map_or_else(
            || format!("{program}/{episode_id}"),
            |found| found[1].to_owned(),
        );
        let description = DESCRIPTION.captures(&webpage).map(|found| {
            crate::html::clean_html(&found["description"])
                .trim()
                .to_owned()
        });
        let series = quoted_text(&SERIES, &webpage).unwrap_or_else(|| program.clone());
        let media = Url::parse(&audio)?;
        let ext = super::direct::determine_ext(media.as_str()).unwrap_or_else(|| "mp3".into());
        let mut format: VideoFormat = serde_json::from_value(serde_json::json!({
            "format_id": "",
            "url": media.as_str(),
            "ext": ext,
            "vcodec": "none",
        }))
        .map_err(|error| Error::Reported(format!("[{NAME}] {id}: {error}")))?;
        crate::info_order::mark_format(&mut format, "cjsw");
        let mut info = super::direct::build_at(media.clone(), &media, String::new());
        info.formats = vec![format];
        info.direct = None;
        info.id = id;
        info.title = title;
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        info.metadata.description = description;
        crate::metadata_parser::set_field(&mut info, "series", series.into());
        crate::metadata_parser::set_field(&mut info, "episode_id", Value::from(episode_id));
        Ok(Some(crate::info_order::mark_info(info, "cjsw")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://cjsw.com/program/freshly-squeezed/episode/20170620"
        ));
        assert!(!suitable("http://cjsw.com/program/freshly-squeezed"));
        assert_eq!(
            quoted_text(&AUDIO, r#"<button x data-audio-src='a"b'>"#).as_deref(),
            Some(r#"a"b"#)
        );
    }
}

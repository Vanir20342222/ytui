//! Upstream `JWPlatformIE` (`yt_dlp/extractor/jwplatform.py`): the
//! `cdn.jwplayer.com/v2/media/ID` JSON through `_parse_jwplayer_data`
//! (`require_title=True`).
use super::{Extractor, download_json};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct JwPlatform;

const NAME: &str = "JWPlatform";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?:https?://(?:content\.jwplatform|cdn\.jwplayer)\.com/(?:(?:feed|player|thumb|preview|manifest)s|jw6|v2/media)/|jwplatform:)(?P<id>[a-zA-Z0-9]{8})",
    )
    .unwrap()
});

/// `JWPlatformIE._extract_embed_urls`: `<script>`/`<iframe>` `src` player
/// URLs (else `<input value>`), else a `data-video-jw-id` div as
/// `jwplatform:ID`; protocol-relative URLs as `https:`, without repeats.
pub(crate) fn embed_urls(_url: &Url, webpage: &str) -> Vec<Url> {
    static TAGS: LazyLock<[Regex; 2]> = LazyLock::new(|| {
        let pattern = |tag: &str, key: &str| {
            Regex::new(&format!(
                r#"<{tag}[^>]+?{key}=\\?["']?((?:https?:)?//(?:content\.jwplatform|cdn\.jwplayer)\.com/players/[a-zA-Z0-9]{{8}})"#
            ))
            .unwrap()
        };
        [
            pattern("(?:script|iframe)", "src"),
            pattern("input", "value"),
        ]
    });
    static DIV: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"<div\b[^>]* data-video-jw-id="([a-zA-Z0-9]{8})""#).unwrap());
    let mut found: Vec<Url> = Vec::new();
    let mut push = |text: &str| {
        let text = super::proto_relative(text);
        if let Ok(url) = Url::parse(&text)
            && !found.contains(&url)
        {
            found.push(url);
        }
    };
    for regex in TAGS.iter() {
        let matches: Vec<String> = regex
            .captures_iter(webpage)
            .map(|found| found[1].to_owned())
            .collect();
        if !matches.is_empty() {
            for text in &matches {
                push(text);
            }
            return found;
        }
    }
    if let Some(found_div) = DIV.captures(webpage) {
        push(&format!("jwplatform:{}", &found_div[1]));
    }
    found
}

/// `_match_id`'s ID.
pub(crate) fn temp_id(url: &str) -> Option<String> {
    VALID_URL.captures(url).map(|found| found["id"].to_owned())
}

/// `_match_id` (also for `jwplatform:ID`).
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for JwPlatform {
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
        let api = Url::parse(&format!("https://cdn.jwplayer.com/v2/media/{id}"))?;
        let data = download_json(client, &api, NAME, &id)?;
        let no_formats = || {
            Error::Reported(format!(
                "[{NAME}] {id}: No video formats found!; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        };
        let Some(entry) = super::jwplayer::parse_required(&data, &id, &api)? else {
            return Err(no_formats());
        };
        let title_missing = entry.get("title").is_none_or(Value::is_null);
        let Some((mut info, keys)) =
            super::html5::jwplayer_entry_info(client, entry, url, &id, NAME)?
        else {
            return Err(no_formats());
        };
        if title_missing {
            info.title = String::new();
            info.extra
                .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
        }
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        let shape = format!("jwplatform;{}", keys.join(","));
        Ok(Some(crate::info_order::mark_info(info, &shape)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://content.jwplatform.com/players/nPripu9l-ALJ3XQCI.js"
        ));
        assert!(suitable("https://cdn.jwplayer.com/v2/media/nPripu9l"));
        assert!(suitable("jwplatform:nPripu9l"));
        assert!(!suitable("https://cdn.jwplayer.com/players/short"));
    }
}

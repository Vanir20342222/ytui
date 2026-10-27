//! Upstream `BitmovinIE` (`yt_dlp/extractor/bitmovin.py`): a stream's
//! player config HLS manifest, title and poster, and its iframe embeds.
use super::{Extractor, download_json, extract_m3u8_fatal};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Bitmovin;

pub(crate) const NAME: &str = "Bitmovin";
pub(crate) const KEY: &str = "Bitmovin";

static VALID_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://streams\.bitmovin\.com/(?P<id>\w+)").unwrap());
static EMBED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"<iframe\b[^>]+\bsrc=["'](?P<url>(?:https?:)?//streams\.bitmovin\.com/(?P<id>\w+)[^"']+)"#,
    )
    .unwrap()
});

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

/// `value[key]`'s escaped `KeyError`.
fn key<'a>(value: &'a Value, name: &str, id: &str) -> Result<&'a Value> {
    match value {
        Value::Object(object) => object.get(name).ok_or_else(|| {
            Error::Reported(format!(
                "{id}: An extractor error has occurred. (caused by KeyError('{name}')); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        }),
        _ => Err(Error::UnsupportedHtml5(
            "Bitmovin configs that are not objects are not supported natively".into(),
        )),
    }
}

impl Extractor for Bitmovin {
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
        let api = Url::parse(&format!("https://streams.bitmovin.com/{id}/config"))?;
        let config = download_json(client, &api, NAME, &id)?;
        let sources = key(&config, "sources", &id)?;
        let manifest = key(sources, "hls", &id)?.as_str().ok_or_else(|| {
            Error::UnsupportedHtml5(
                "Bitmovin HLS URLs that are not strings are not supported natively".into(),
            )
        })?;
        let manifest = Url::parse(manifest)?;
        let mut info = extract_m3u8_fatal(client, &manifest, NAME, &id, None, true)?;
        let text = |name: &str| sources.get(name).and_then(Value::as_str).map(str::to_owned);
        let title = text("title");
        let shape = if title.is_some() {
            "bitmovin"
        } else {
            "bitmovin-untitled"
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
        info.webpage_url = url.clone();
        info.direct = None;
        info.extractor = NAME.into();
        info.extractor_key = KEY.into();
        info.metadata.thumbnail = text("poster");
        Ok(Some(crate::info_order::mark_info(info, shape)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://streams.bitmovin.com/cqkl1t5giv3lrce7pjbg/embed"
        ));
        assert!(!suitable("https://bitmovin.com/x"));
        let page = Url::parse("https://page.test/").unwrap();
        let found: Vec<String> = embed_urls(
            &page,
            r#"<iframe src="//streams.bitmovin.com/a1/embed"><iframe src="https://streams.bitmovin.com/x">"#,
        )
        .iter()
        .map(Url::to_string)
        .collect();
        assert_eq!(found, ["https://streams.bitmovin.com/a1/embed"]);
    }
}

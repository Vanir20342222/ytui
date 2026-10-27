//! Upstream `MegaphoneIE` (`yt_dlp/extractor/megaphone.py`): a
//! `player.megaphone.fm` page's OpenGraph audio fields and its `episode`
//! script object.
use super::{Extractor, download_webpage, proto_relative};
use crate::{Error, Result, VideoFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Megaphone;

/// `IE_NAME`; the key is `Megaphone`.
pub(crate) const NAME: &str = "megaphone.fm";
pub(crate) const KEY: &str = "Megaphone";

static VALID_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://player\.megaphone\.fm/(?P<id>[A-Z0-9]+)").unwrap());

/// `_EMBED_REGEX`: the `_VALID_URL` prefix of an iframe's `src`.
static EMBED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<iframe[^>]*?\ssrc=["'](?P<url>https?://player\.megaphone\.fm/[A-Z0-9]+)"#)
        .unwrap()
});

static EPISODE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)var\s+episode\s*=\s*(\{.+?\});").unwrap());

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

fn reported(id: &str, message: &str) -> Error {
    Error::Reported(format!("[{NAME}] {id}: {message}"))
}

/// A fatal `_og_search_property(prop, webpage)`.
fn og_property(webpage: &str, id: &str, prop: &str) -> Result<String> {
    crate::html::og_search_property(webpage, prop).ok_or_else(|| {
        reported(
            id,
            &format!(
                "Unable to extract OpenGraph {prop}; {}",
                crate::BUG_REPORTS_MESSAGE
            ),
        )
    })
}

impl Extractor for Megaphone {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(id) = temp_id(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &id)?;
        let title = og_property(&webpage, &id, "audio:title")?;
        let author = og_property(&webpage, &id, "audio:artist")?;
        // `_og_search_thumbnail`: non-fatal.
        let thumbnail = crate::html::og_search_property(&webpage, "image");
        if thumbnail.is_none() {
            crate::screen::warning(&format!(
                "[{NAME}] unable to extract thumbnail URL; {}",
                crate::BUG_REPORTS_MESSAGE
            ));
        }
        let episode = EPISODE
            .captures(&webpage)
            .map(|found| found[1].to_owned())
            .ok_or_else(|| {
                reported(
                    &id,
                    &format!(
                        "Unable to extract episode JSON; {}",
                        crate::BUG_REPORTS_MESSAGE
                    ),
                )
            })?;
        let episode = crate::js_to_json::parse_js_value(&episode)
            .ok_or_else(|| reported(&id, "Failed to parse JSON "))?;
        // `ExtractorError(video_id=...)` around the `KeyError`: no extractor prefix.
        let key_error = |key: &str| {
            Error::Reported(format!(
                "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        };
        let media = match episode.get("mediaUrl") {
            Some(Value::String(media)) => media.clone(),
            Some(_) => {
                return Err(Error::UnsupportedHtml5(
                    "non-string Megaphone mediaUrl values are not supported natively".into(),
                ));
            }
            None => return Err(key_error("mediaUrl")),
        };
        let duration = episode
            .get("duration")
            .cloned()
            .ok_or_else(|| key_error("duration"))?;
        let media = Url::parse(&proto_relative(&media))?;
        let ext =
            super::direct::determine_ext(media.as_str()).unwrap_or_else(|| "unknown_video".into());
        let mut format: VideoFormat = serde_json::from_value(
            serde_json::json!({"format_id": "", "url": media.as_str(), "ext": ext}),
        )
        .map_err(|error| reported(&id, &error.to_string()))?;
        crate::info_order::mark_format(&mut format, "url-only");
        let mut info = super::direct::build_at(media.clone(), &media, String::new());
        info.formats = vec![format];
        info.id = id;
        info.title = title;
        info.webpage_url = url.clone();
        info.direct = None;
        info.extractor = NAME.into();
        info.extractor_key = KEY.into();
        info.metadata.thumbnail = thumbnail;
        info.extra
            .insert("creators".into(), serde_json::json!([author]));
        if !duration.is_null() {
            crate::metadata_parser::set_field(&mut info, "duration", duration);
        }
        Ok(Some(crate::info_order::mark_info(info, "megaphone")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_patterns() {
        assert!(suitable("https://player.megaphone.fm/GLT9749789991"));
        assert!(!suitable("https://player.megaphone.fm/lower"));
        let page = Url::parse("https://blog.invalid/").unwrap();
        let html = r#"<iframe width="1" src="https://player.megaphone.fm/AB1?light=true"></iframe><iframe src='http://player.megaphone.fm/AB1'><iframe src="https://player.megaphone.fm/AB1#x">"#;
        let found: Vec<String> = embed_urls(&page, html).iter().map(Url::to_string).collect();
        assert_eq!(
            found,
            [
                "https://player.megaphone.fm/AB1",
                "http://player.megaphone.fm/AB1"
            ]
        );
    }
}

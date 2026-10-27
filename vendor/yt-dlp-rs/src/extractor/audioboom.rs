//! Upstream `AudioBoomIE` (`yt_dlp/extractor/audioboom.py`): the post page's
//! `V5DetailPagePlayer` props (HTML-unescaped) with OpenGraph/meta
//! fallbacks, as an untyped result.
use super::{Extractor, download_webpage, search_json_with, search_warning};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct AudioBoom;

const NAME: &str = "AudioBoom";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?audioboom\.com/(?:boos|posts)/(?P<id>[0-9]+)").unwrap()
});
static STORE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"data-react-class="V5DetailPagePlayer"\s*data-react-props=["']"#).unwrap()
});
static UPLOADER_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<div class="avatar flex-shrink-0">\s*<a href="(http[^"]+)""#).unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// Python truthiness of a JSON value.
fn truthy(value: Option<&Value>) -> Option<&Value> {
    value.filter(|value| match value {
        Value::Null | Value::Bool(false) => false,
        Value::String(text) => !text.is_empty(),
        Value::Number(number) => number.as_f64() != Some(0.0),
        Value::Array(items) => !items.is_empty(),
        Value::Object(items) => !items.is_empty(),
        Value::Bool(true) => true,
    })
}

/// `_html_search_meta(names, webpage, display_name)` (non-fatal).
fn meta(webpage: &str, names: &[&str], display: &str) -> Option<String> {
    let found = names
        .iter()
        .find_map(|name| crate::html::html_search_meta(webpage, name));
    if found.is_none() {
        search_warning(NAME, display);
    }
    found
}

impl Extractor for AudioBoom {
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
        let page = Url::parse(&format!("https://audioboom.com/posts/{id}"))?;
        let webpage = download_webpage(client, &page, NAME, &id)?;
        // `_search_json(..., fatal=False, transform_source=unescapeHTML)`.
        let store = match search_json_with(&STORE, &webpage, &crate::html::unescape_html) {
            Some(Some(store)) => Some(store),
            _ => {
                search_warning(NAME, "clip store");
                None
            }
        };
        let empty = serde_json::Map::new();
        let clip = store
            .as_ref()
            .and_then(|store| store.get("clips"))
            .and_then(|clips| clips.get(0))
            .and_then(Value::as_object)
            .unwrap_or(&empty);
        let text = |key: &str| {
            truthy(clip.get(key))
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        let media = match text("clipURLPriorToLoading") {
            Some(media) => media,
            None => crate::html::og_search_property(&webpage, "audio").ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {id}: Unable to extract audio url; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?,
        };
        let title = text("title").or_else(|| {
            meta(
                &webpage,
                &["og:title", "og:audio:title", "audio_title"],
                "og:title",
            )
        });
        let description = text("description")
            .or_else(|| {
                truthy(clip.get("formattedDescription"))
                    .and_then(Value::as_str)
                    .map(crate::html::clean_html)
                    .filter(|text| !text.is_empty())
            })
            .or_else(|| {
                let found = crate::html::og_search_property(&webpage, "description");
                if found.is_none() {
                    search_warning(NAME, "OpenGraph description");
                }
                found
            });
        let duration = match truthy(clip.get("duration")) {
            Some(Value::Number(number)) => number.as_f64(),
            Some(Value::String(text)) => text.trim().parse().ok(),
            _ => meta(&webpage, &["weibo:audio:duration"], "weibo:audio:duration")
                .and_then(|text| text.trim().parse().ok()),
        };
        let uploader = text("author").or_else(|| {
            meta(
                &webpage,
                &[
                    "og:audio:artist",
                    "twitter:audio:artist_name",
                    "audio_artist",
                ],
                "uploader",
            )
        });
        let uploader_url = text("author_url").or_else(|| {
            let found = UPLOADER_URL
                .captures(&webpage)
                .map(|found| crate::html::clean_html(&found[1]));
            if found.is_none() {
                search_warning(NAME, "uploader url");
            }
            found
        });
        let media = page.join(&media)?;
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
        info.metadata.duration = duration.map(crate::info::PyNumber::Float);
        info.extra
            .insert("uploader".into(), uploader.map_or(Value::Null, Value::from));
        match uploader_url {
            Some(uploader_url) => info.metadata.uploader_url = Some(uploader_url),
            None => {
                info.extra.insert("uploader_url".into(), Value::Null);
            }
        }
        Ok(Some(crate::info_order::mark_info(info, "audioboom")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://audioboom.com/posts/7398103-asim-chaudhry"
        ));
        assert!(suitable("https://audioboom.com/boos/2"));
        assert!(!suitable("https://audioboom.com/channels/5128"));
    }
}

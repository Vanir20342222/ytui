//! Upstream `CrooksAndLiarsIE` (`yt_dlp/extractor/crooksandliars.py`): an
//! embed page's `manifest` JSON video flavors, and its embeds.
use super::{Extractor, download_webpage, proto_relative, search_json};
use crate::{Error, Result, VideoFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct CrooksAndLiars;

pub(crate) const NAME: &str = "CrooksAndLiars";
pub(crate) const KEY: &str = "CrooksAndLiars";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://embed\.crooksandliars\.com/(?:embed|v)/(?P<id>[A-Za-z0-9]+)").unwrap()
});
/// `_EMBED_REGEX` (`\1` is one quote character, so the backreference is an
/// alternation of both).
static EMBED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"<(?:iframe[^>]+src|param[^>]+value)=(?:"(?P<a>(?:https?:)?//embed\.crooksandliars\.com/(?:embed|v)/.+?)"|'(?P<b>(?:https?:)?//embed\.crooksandliars\.com/(?:embed|v)/.+?)')"#,
    )
    .unwrap()
});
static MANIFEST: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"var\s+manifest\s*=").unwrap());

/// `qualities(('webm_low', 'mp4_low', 'webm_high', 'mp4_high'))`.
const QUALITIES: [&str; 4] = ["webm_low", "mp4_low", "webm_high", "mp4_high"];

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
        let Some(group) = captures.name("a").or_else(|| captures.name("b")) else {
            continue;
        };
        let Ok(embed) = url.join(&crate::html::unescape_html(group.as_str())) else {
            continue;
        };
        if suitable(embed.as_str()) && !found.contains(&embed) {
            found.push(embed);
        }
    }
    found
}

impl Extractor for CrooksAndLiars {
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
        let page = Url::parse(&format!("http://embed.crooksandliars.com/embed/{id}"))?;
        let webpage = download_webpage(client, &page, NAME, &id)?;
        let manifest = match search_json(&MANIFEST, &webpage, false) {
            Some(Some(manifest)) => manifest,
            Some(None) => {
                return Err(Error::UnsupportedHtml5(
                    "Crooks and Liars manifests that do not parse are not supported natively"
                        .into(),
                ));
            }
            None => {
                return Err(Error::Reported(format!(
                    "[{NAME}] {id}: Unable to extract manifest JSON; {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        let key_error = |name: &str| {
            Error::Reported(format!(
                "{id}: An extractor error has occurred. (caused by KeyError('{name}')); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        };
        let unsupported = |what: &str| {
            Error::UnsupportedHtml5(format!(
                "Crooks and Liars {what} that are not strings are not supported natively"
            ))
        };
        let flavors = match manifest.get("flavors") {
            Some(Value::Array(flavors)) => flavors.clone(),
            Some(_) => return Err(unsupported("flavor lists")),
            None => return Err(key_error("flavors")),
        };
        let mut formats = Vec::new();
        for item in &flavors {
            let mime = item.get("mime").ok_or_else(|| key_error("mime"))?;
            let mime = mime.as_str().ok_or_else(|| unsupported("MIME types"))?;
            if !mime.starts_with("video/") {
                continue;
            }
            let media = item.get("url").ok_or_else(|| key_error("url"))?;
            let media = Url::parse(media.as_str().ok_or_else(|| unsupported("URLs"))?)?;
            let kind = item.get("type").ok_or_else(|| key_error("type"))?;
            let kind = kind.as_str().ok_or_else(|| unsupported("types"))?;
            let quality = QUALITIES
                .iter()
                .position(|quality| *quality == kind)
                .map_or(-1, |index| index as i64);
            let ext = super::direct::determine_ext(media.as_str())
                .unwrap_or_else(|| "unknown_video".into());
            let mut format: VideoFormat = serde_json::from_value(json!({
                "url": media.as_str(),
                "format_id": kind,
                "quality": quality,
                "ext": ext,
            }))
            .map_err(|error| Error::Reported(format!("[{NAME}] {id}: {error}")))?;
            crate::info_order::mark_format(&mut format, "crooksandliars");
            formats.push(format);
        }
        let title = manifest.get("title").ok_or_else(|| key_error("title"))?;
        let title = title
            .as_str()
            .ok_or_else(|| unsupported("titles"))?
            .to_owned();
        let no_formats = formats.is_empty();
        let first = formats
            .first()
            .map_or_else(|| url.clone(), |format| format.url.clone());
        let mut info = super::direct::build_at(first.clone(), &first, String::new());
        info.formats = formats;
        info.direct = None;
        info.id = id;
        info.title = title;
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = KEY.into();
        if no_formats {
            info.extra
                .insert(crate::info::NO_FORMATS.into(), Value::Bool(true));
        }
        let field = |name: &str| manifest.get(name).cloned().unwrap_or(Value::Null);
        info.metadata.description = manifest
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_owned);
        info.metadata.thumbnail = manifest
            .get("poster")
            .and_then(Value::as_str)
            .map(proto_relative);
        let int = |name: &str| super::streamable::int_or_none(manifest.get(name));
        for (name, value) in [("timestamp", int("created")), ("uploader", field("author"))] {
            if value.is_null() {
                info.extra.insert(name.into(), Value::Null);
            } else {
                crate::metadata_parser::set_field(&mut info, name, value);
            }
        }
        // Processing drops a `None` duration.
        let duration = int("duration");
        if !duration.is_null() {
            crate::metadata_parser::set_field(&mut info, "duration", duration);
        }
        Ok(Some(crate::info_order::mark_info(info, "crooksandliars")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://embed.crooksandliars.com/embed/8RUoRhRi"));
        assert!(suitable(
            "https://embed.crooksandliars.com/v/MTE3MjUtMzQ2MzA"
        ));
        assert!(!suitable("https://crooksandliars.com/embed/x"));
    }
}

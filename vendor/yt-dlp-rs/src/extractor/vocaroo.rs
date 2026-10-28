//! Upstream `VocarooIE` (`yt_dlp/extractor/vocaroo.py`): the recording's
//! `mp3` URL (checked with a HEAD request carrying the site's Referer) as an
//! untyped result with an empty title.
use super::Extractor;
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Vocaroo;

const NAME: &str = "Vocaroo";
const REFERER: &str = "https://vocaroo.com/";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?(?:vocaroo\.com|voca\.ro)/(?:embed/)?(?P<id>\w+)").unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for Vocaroo {
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
        let subdomain = if id.len() == 10 || (id.len() == 12 && id.starts_with('1')) {
            "media1"
        } else {
            "media"
        };
        let media = Url::parse(&format!("https://{subdomain}.vocaroo.com/mp3/{id}"))?;
        // `_request_webpage(HEADRequest(url), audio_id, headers=...)`.
        crate::screen::screen(&format!("[{NAME}] {id}: Downloading webpage"));
        crate::screen::before_extraction_request();
        let response = client
            .head(media.clone())
            .header(reqwest::header::REFERER, REFERER)
            .send()
            .inspect(|_| crate::screen::after_request())
            .map_err(Error::Http)
            .and_then(crate::retry::check_response)
            .map_err(|error| match error {
                Error::Http(_) | Error::HttpRetryAfter { .. } => Error::ExtractorRequest {
                    extractor: NAME,
                    id: id.clone(),
                    what: "webpage",
                    source: Box::new(error),
                },
                error => error,
            })?;
        // `float_or_none(header, scale=1000)`.
        let timestamp = response
            .headers()
            .get("x-bz-upload-timestamp")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<f64>().ok())
            .map(|value| value / 1000.0);
        let mut info = super::direct::with_format(
            super::direct::build_at(media.clone(), &media, "mp3".into()),
            "",
        );
        for format in &mut info.formats {
            format.ext = "mp3".into();
            format.vcodec = Some("none".into());
            format.format_id = String::new();
            format.http_headers = Some(serde_json::Map::from_iter([(
                "Referer".to_owned(),
                REFERER.into(),
            )]));
        }
        info.vcodec = Some("none".into());
        info.format_id = None;
        info.id = id;
        // `'title': ''` (a generic title is made in processing).
        info.title = String::new();
        info.webpage_url = url.clone();
        info.direct = None;
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        info.metadata.timestamp = timestamp.map(crate::info::PyNumber::Float);
        info.extra.insert(
            "http_headers".into(),
            serde_json::json!({"Referer": REFERER}),
        );
        if timestamp.is_none() {
            info.extra.insert("timestamp".into(), Value::Null);
        }
        Ok(Some(crate::info_order::mark_info(info, "vocaroo")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://vocaroo.com/1de8yA3LNe77"));
        assert!(suitable("https://voca.ro/embed/12WqtjLnpj6g"));
        assert!(!suitable("https://example.com/1de8yA3LNe77"));
    }
}

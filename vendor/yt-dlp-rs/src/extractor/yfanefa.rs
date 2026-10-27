//! Upstream `YfanefaIE` (`yt_dlp/extractor/yfanefa.py`): the Yorkshire Film
//! Archive player options' signed URL as HLS or MP4 formats.
use super::{Extractor, download_webpage, extract_m3u8_fatal, search_json, search_warning};
use crate::{Error, Result, VideoFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Yfanefa;

const NAME: &str = "yfanefa";
const KEY: &str = "Yfanefa";

static VALID_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://(?:www\.)?yfanefa\.com/(?P<id>[^?#]+)").unwrap());
static OPTIONS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"iwPlayer\.options\["[\w.]+"\]\s*="#).unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for Yfanefa {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &video_id)?;
        let player = match search_json(&OPTIONS, &webpage, false) {
            Some(Some(player)) => player,
            Some(None) => {
                return Err(Error::UnsupportedHtml5(
                    "Yfanefa player options that do not parse are not supported natively".into(),
                ));
            }
            None => {
                return Err(Error::Reported(format!(
                    "[{NAME}] {video_id}: Unable to extract player options; {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        let text = |value: &Value| match value {
            Value::String(text) => Some(text.clone()),
            Value::Null => None,
            _ => Some(value.to_string()),
        };
        let base = player.get("url").ok_or_else(|| {
            Error::Reported(format!(
                "{video_id}: An extractor error has occurred. (caused by KeyError('url')); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
        // `join_nonempty(url, signature, delim='')`.
        let media: String = [Some(base), player.get("signature")]
            .into_iter()
            .flatten()
            .filter_map(text)
            .filter(|part| !part.is_empty())
            .collect();
        let media = Url::parse(&media)?;
        let mut info = if super::direct::determine_ext(media.as_str()).as_deref() == Some("m3u8") {
            extract_m3u8_fatal(client, &media, NAME, &video_id, Some("hls"), false)?
        } else {
            let mut format: VideoFormat = serde_json::from_value(json!({
                "format_id": "",
                "url": media.as_str(),
                "ext": "mp4",
            }))
            .map_err(|error| Error::Reported(format!("[{NAME}] {video_id}: {error}")))?;
            crate::info_order::mark_format(&mut format, "url-ext");
            let mut info = super::direct::build_at(media.clone(), &media, "mp4".into());
            info.formats = vec![format];
            info
        };
        // `_og_search_title(default=None) or remove_end(_html_extract_title(),
        // ' | Yorkshire Film Archive')`.
        let title = crate::html::og_search_property(&webpage, "title")
            .filter(|title| !title.is_empty())
            .or_else(|| {
                let title = crate::html::html_extract_title(&webpage);
                if title.is_none() {
                    search_warning(NAME, "title");
                }
                title.map(|title| {
                    title
                        .strip_suffix(" | Yorkshire Film Archive")
                        .map_or_else(|| title.clone(), str::to_owned)
                })
            });
        info.direct = None;
        info.id = video_id.trim_matches('/').replace('/', "-");
        match title {
            Some(title) => info.title = title,
            None => {
                info.title = String::new();
                info.extra
                    .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
            }
        }
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = KEY.into();
        info.metadata.thumbnail = player
            .get("preview")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none);
        let duration = super::streamable::int_or_none(player.get("duration"));
        if !duration.is_null() {
            crate::metadata_parser::set_field(&mut info, "duration", duration);
        }
        Ok(Some(crate::info_order::mark_info(info, "yfanefa")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://www.yfanefa.com/record/2717"));
        assert!(!suitable("https://www.yfanefa.com/"));
    }
}

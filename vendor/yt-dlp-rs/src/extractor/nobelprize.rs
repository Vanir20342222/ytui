//! Upstream `NobelPrizeIE` (`yt_dlp/extractor/nobelprize.py`): the media
//! player page's JSON-LD with its caption, description and duration.
use super::{Extractor, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct NobelPrize;

const NAME: &str = "NobelPrize";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:(?:mediaplayer|www)\.)?nobelprize\.org/mediaplayer/").unwrap()
});

static DESCRIPTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)<span\b[^>]*?\bitemprop\s*=\s*["']?description["']?[^>]*>(.*?)</span>"#)
        .unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for NobelPrize {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        if !suitable(url.as_str()) {
            return Ok(None);
        }
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        // `traverse_obj(parse_qs(url), (('id', 'qid'), -1, {int_or_none},
        // {str_or_none}, any))`.
        let last = |key: &str| {
            url.query_pairs()
                .filter(|(name, _)| name == key)
                .last()
                .map(|(_, value)| value.into_owned())
        };
        let id = ["id", "qid"]
            .into_iter()
            .filter_map(last)
            .find_map(|value| value.trim().parse::<i64>().ok())
            .map(|id| id.to_string())
            .ok_or_else(|| Error::UnsupportedUrl(url.to_string()))?;
        let mut page = url.clone();
        page.set_host(Some("mediaplayer.nobelprize.org"))?;
        let webpage = download_webpage(client, &page, NAME, &id)?;
        let json_ld = super::json_ld::search(&webpage);
        if json_ld.is_empty() {
            return Err(Error::Reported(format!(
                "[{NAME}] Unable to extract JSON-LD; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        }
        let title = crate::html::html_search_meta(&webpage, "caption");
        let description = DESCRIPTION
            .captures(&webpage)
            .map(|found| crate::html::clean_html(&found[1]));
        let duration = crate::html::html_search_meta(&webpage, "duration")
            .and_then(|duration| crate::parse_duration(&duration));
        let media = json_ld.get("url").and_then(Value::as_str).ok_or_else(|| {
            Error::UnsupportedHtml5(
                "NobelPrize JSON-LD without a media URL is not supported natively".into(),
            )
        })?;
        let media = Url::parse(media)?;
        let ext = json_ld
            .get("ext")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| super::direct::determine_ext(media.as_str()))
            .unwrap_or_else(|| "unknown_video".into());
        let mut info = super::direct::with_format(
            super::direct::build_at(media.clone(), &media, ext.clone()),
            "",
        );
        for format in &mut info.formats {
            format.ext = ext.clone();
            format.format_id = String::new();
        }
        info.format_id = None;
        info.webpage_url = url.clone();
        info.direct = None;
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        // `{**json_ld, 'id': ..., 'title': ..., 'description': ...,
        // 'duration': ...}`.
        let mut keys: Vec<String> = Vec::new();
        for (key, value) in json_ld {
            if matches!(
                key.as_str(),
                "id" | "title" | "description" | "duration" | "url" | "ext"
            ) {
                keys.push(key);
                continue;
            }
            crate::metadata_parser::set_field(&mut info, &key, value);
            keys.push(key);
        }
        for key in ["id", "title", "description", "duration"] {
            if !keys.iter().any(|existing| existing == key) {
                keys.push(key.to_owned());
            }
        }
        info.id = id;
        match title {
            Some(title) => info.title = title,
            None => {
                info.title = String::new();
                info.extra
                    .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
            }
        }
        match description {
            Some(description) => info.metadata.description = Some(description),
            None => {
                info.extra.insert("description".into(), Value::Null);
            }
        }
        match duration {
            Some(duration) => info.metadata.duration = Some(duration),
            None => {
                info.extra.insert("duration".into(), Value::Null);
            }
        }
        let shape = format!("nobelprize;{}", keys.join(","));
        Ok(Some(crate::info_order::mark_info(info, &shape)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://www.nobelprize.org/mediaplayer/?id=2636"));
        assert!(suitable(
            "https://mediaplayer.nobelprize.org/mediaplayer/?qid=12693"
        ));
        assert!(!suitable("https://www.nobelprize.org/prizes/"));
    }
}

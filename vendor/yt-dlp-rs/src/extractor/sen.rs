//! Upstream `SenIE` (`yt_dlp/extractor/sen.py`): the content API's player
//! node HLS stream (else the VOD manifest) and its details node metadata.
use super::{Extractor, download_json, extract_m3u8_fatal};
use crate::{Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Sen;

const NAME: &str = "Sen";

static VALID_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://(?:www\.)?sen\.com/video/(?P<id>[0-9a-f-]+)").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for Sen {
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
        let api = Url::parse(&format!("https://api.sen.com/content/public/video/{id}"))?;
        let data = download_json(client, &api, NAME, &id)?;
        let nodes: Vec<&Value> = data
            .get("data")
            .and_then(|data| data.get("nodes"))
            .and_then(Value::as_array)
            .map(|nodes| nodes.iter().collect())
            .unwrap_or_default();
        let node = |name: &'static str| {
            nodes
                .iter()
                .copied()
                .filter(move |node| node.get("id").and_then(Value::as_str) == Some(name))
        };
        // `('data', 'nodes', lambda _, v: v['id'] == 'player', 'video',
        // 'url', {url_or_none}, any)`.
        let manifest = node("player")
            .filter_map(|node| node.get("video")?.get("url")?.as_str())
            .find_map(super::json_ld::url_or_none)
            .unwrap_or_else(|| format!("https://vod.sen.com/videos/{id}/manifest.m3u8"));
        let manifest = Url::parse(&manifest)?;
        let mut info = extract_m3u8_fatal(client, &manifest, NAME, &id, None, false)?;
        // `('data', 'nodes', lambda _, v: v['id'] == 'details', any,
        // 'content', {...})`.
        let content = node("details").next().and_then(|node| node.get("content"));
        let text = |value: Option<&Value>| {
            value
                .and_then(|value| value.get("text"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        let title = text(content.and_then(|content| content.get("title")));
        let description = text(
            content
                .and_then(|content| content.get("descriptions"))
                .and_then(|descriptions| descriptions.get(0)),
        );
        let tags: Vec<Value> = content
            .and_then(|content| content.get("badges"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|badge| text(Some(badge)).map(Value::from))
            .collect();
        info.id = id;
        // Without one, the template has no `title` key.
        let shape = if title.is_some() {
            "sen"
        } else {
            "sen-untitled"
        };
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
        if !tags.is_empty() {
            crate::metadata_parser::set_field(&mut info, "tags", Value::Array(tags));
        }
        Ok(Some(crate::info_order::mark_info(info, shape)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.sen.com/video/eef46eb1-4d79-4e28-be9d-bd937767f8c4"
        ));
        assert!(!suitable("https://www.sen.com/videos/"));
    }
}

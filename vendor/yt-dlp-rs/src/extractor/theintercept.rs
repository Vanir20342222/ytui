//! Upstream `TheInterceptIE` (`yt_dlp/extractor/theintercept.py`): the
//! page's `initialStoreTree` post as a `url_transparent` result of its
//! `jwplatform:` video, overlaid with the post's fields.
use super::{Extractor, download_webpage};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct TheIntercept;

const NAME: &str = "TheIntercept";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://theintercept\.com/fieldofvision/(?P<id>[^/?#]+)").unwrap()
});

static STORE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"initialStoreTree\s*=\s*(\{.+\})").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn report(display_id: &str, message: &str) -> Error {
    Error::Reported(format!(
        "[{NAME}] {display_id}: {message}; {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

impl Extractor for TheIntercept {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &display_id)?;
        let store = STORE
            .captures(&webpage)
            .map(|found| found[1].to_owned())
            .ok_or_else(|| report(&display_id, "Unable to extract initialStoreTree"))?;
        let store: Value = serde_json::from_str(&store)
            .map_err(|error| report(&display_id, &format!("Failed to parse JSON ({error})")))?;
        let key_error = |key: &str| {
            Error::Reported(format!(
                "{display_id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        };
        let posts = store
            .get("resources")
            .ok_or_else(|| key_error("resources"))?
            .get("posts")
            .ok_or_else(|| key_error("posts"))?
            .as_object()
            .ok_or_else(|| {
                Error::UnsupportedHtml5("non-object posts are not supported natively".into())
            })?;
        for post in posts.values() {
            if post.get("slug").ok_or_else(|| key_error("slug"))?.as_str()
                != Some(display_id.as_str())
            {
                continue;
            }
            let video = post
                .get("fov_videoid")
                .ok_or_else(|| key_error("fov_videoid"))?;
            let video = match video {
                Value::String(text) => text.clone(),
                Value::Number(number) => number.to_string(),
                _ => {
                    return Err(Error::UnsupportedHtml5(
                        "non-string video IDs are not supported natively".into(),
                    ));
                }
            };
            let title = post
                .get("title")
                .cloned()
                .ok_or_else(|| key_error("title"))?;
            post.get("ID").ok_or_else(|| key_error("ID"))?;
            let target = Url::parse(&format!("jwplatform:{video}"))?;
            let Some(ExtractResult::Video(mut info)) =
                super::jwplatform::JwPlatform.extract_result(client, &target)?
            else {
                return Ok(None);
            };
            // `url_transparent`: the post's non-`None` fields other than
            // `_type`, `url`, `id` and the extractor.
            let comment_count = super::streamable::int_or_none(post.get("comments_number"));
            let timestamp = post
                .get("date")
                .and_then(Value::as_str)
                .and_then(crate::dates::parse_iso8601)
                .map_or(Value::Null, Value::from);
            let outer = [
                ("display_id", Value::from(display_id.clone())),
                ("title", title),
                (
                    "description",
                    post.get("excerpt").cloned().unwrap_or(Value::Null),
                ),
                ("timestamp", timestamp),
                ("comment_count", comment_count),
            ];
            let mut keys = Vec::new();
            for (key, value) in outer {
                if value.is_null() {
                    continue;
                }
                keys.push(key);
                match key {
                    "title" => info.title = value.as_str().unwrap_or_default().to_owned(),
                    _ => crate::metadata_parser::set_field(&mut info, key, value),
                }
            }
            info.webpage_url = url.clone();
            let inner_shape = match info.extra.get(crate::info_order::INFO_SHAPE) {
                Some(Value::String(shape)) => shape.clone(),
                _ => String::new(),
            };
            let shape = format!("{inner_shape}|transparent:{}", keys.join(","));
            return Ok(Some(ExtractResult::Video(crate::info_order::mark_info(
                info, &shape,
            ))));
        }
        Err(report(&display_id, "Unable to find the current post"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://theintercept.com/fieldofvision/thisisacoup-episode-four-surrender-or-die/"
        ));
        assert!(!suitable("https://theintercept.com/2015/12/13/x/"));
    }
}

//! Upstream `MastersIE` (`yt_dlp/extractor/masters.py`): the related-content
//! API's HLS stream, title and first image set, dated from the URL.
use super::{Extractor, download_json, extract_m3u8_fatal};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Masters;

const NAME: &str = "Masters";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?masters\.com/en_US/watch/(?P<date>\d{4}-\d{2}-\d{2})/(?P<id>\d+)",
    )
    .unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for Masters {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        let upload_date = crate::dates::unified_strdate(&found["date"], true);
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let api = Url::parse(&format!(
            "https://www.masters.com/relatedcontent/rest/v2/masters_v1/en/content/masters_v1_{id}_en"
        ))?;
        let content = download_json(client, &api, NAME, &id)?;
        // `_extract_m3u8_formats_and_subtitles(traverse_obj(content_resp,
        // ('media', 'm3u8')), video_id, 'mp4')` (fatal).
        let Some(manifest) = content
            .get("media")
            .and_then(|media| media.get("m3u8"))
            .and_then(Value::as_str)
            .filter(|manifest| !manifest.is_empty())
        else {
            return Err(Error::Reported(format!(
                "[{NAME}] {id}: Failed to obtain m3u8 URL; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        };
        let manifest = Url::parse(manifest)?;
        let mut info = extract_m3u8_fatal(client, &manifest, NAME, &id, None, true)?;
        // `traverse_obj(content_resp, ('images', 0), default={}).items()`.
        let images = match content.get("images").and_then(|images| images.get(0)) {
            None | Some(Value::Null) => serde_json::Map::new(),
            Some(Value::Object(images)) => images.clone(),
            Some(_) => {
                return Err(Error::UnsupportedHtml5(
                    "Masters image sets that are not objects are not supported natively".into(),
                ));
            }
        };
        let mut thumbnails = Vec::new();
        for (name, image) in images {
            let Some(image) = image.as_str() else {
                return Err(Error::UnsupportedHtml5(
                    "Masters image URLs that are not strings are not supported natively".into(),
                ));
            };
            thumbnails.push(crate::info::Thumbnail {
                url: image.to_owned(),
                id: Some(name),
                width: None,
                height: None,
                preference: None,
                resolution: None,
                extra: serde_json::Map::new(),
            });
        }
        info.id = id;
        match content.get("title").and_then(Value::as_str) {
            Some(title) if !title.is_empty() => info.title = title.to_owned(),
            _ => {
                info.title = String::new();
                info.extra
                    .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
            }
        }
        info.webpage_url = url.clone();
        info.direct = None;
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        info.metadata.thumbnails = thumbnails;
        match upload_date {
            Some(date) => crate::metadata_parser::set_field(&mut info, "upload_date", date.into()),
            None => {
                info.extra.insert("upload_date".into(), Value::Null);
            }
        }
        Ok(Some(crate::info_order::mark_info(info, "masters")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.masters.com/en_US/watch/2022-04-07/16493755593805191/sungjae_im_thursday_interview_2022.html"
        ));
        assert!(!suitable(
            "https://www.masters.com/en_US/watch/16493755593805191"
        ));
    }
}

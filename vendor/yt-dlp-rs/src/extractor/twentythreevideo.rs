//! Upstream `TwentyThreeVideoIE` (`yt_dlp/extractor/twentythreevideo.py`,
//! `23video`): the player's `api/photo/list` JSON with its audio and video
//! downloads as formats and thumbnails.
use super::{Extractor, download_json_transformed, update_url_query};
use crate::{Error, Result, VideoFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct TwentyThreeVideo;

const NAME: &str = "23video";
const KEY: &str = "TwentyThreeVideo";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?P<domain>[^.]+\.(?:twentythree\.net|23video\.com|filmweb\.no))/v\.ihtml/player\.html\?(?P<query>.*?\bphoto(?:_|%5f)id=(?P<id>\d+).*)",
    )
    .unwrap()
});

static OBJECT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)(\{.+\})").unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for TwentyThreeVideo {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let (domain, query, id) = (&found["domain"], &found["query"], found["id"].to_owned());
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let base = format!("https://{domain}");
        let api = update_url_query(
            &Url::parse(&format!("{base}/api/photo/list?{query}"))?,
            &[("format", "json")],
        );
        let data = download_json_transformed(
            client,
            &api,
            NAME,
            &id,
            "Downloading JSON metadata",
            &|text| {
                OBJECT
                    .captures(text)
                    .map(|found| found[1].to_owned())
                    .ok_or_else(|| {
                        Error::Reported(format!(
                            "[{NAME}] {id}: Unable to extract photo data; {}",
                            crate::BUG_REPORTS_MESSAGE
                        ))
                    })
            },
        )?;
        let key_error = |key: &str| {
            Error::Reported(format!(
                "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        };
        let photo = data.get("photo").ok_or_else(|| key_error("photo"))?;
        let title = photo
            .get("title")
            .ok_or_else(|| key_error("title"))?
            .clone();
        let int = |key: &str| super::streamable::int_or_none(photo.get(key));
        let text = |key: &str| {
            photo
                .get(key)
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
        };
        let mut formats = Vec::new();
        if let Some(path) = text("audio_download") {
            let mut format: VideoFormat = serde_json::from_value(json!({
                "format_id": "audio",
                "url": format!("{base}{path}"),
                "ext": super::direct::determine_ext(path).unwrap_or_else(|| "unknown_video".into()),
                "filesize": int("audio_size"),
                "vcodec": "none",
            }))
            .map_err(|error| Error::Reported(format!("[{NAME}] {id}: {error}")))?;
            crate::info_order::mark_format(&mut format, "23video-audio");
            formats.push(format);
        }
        for name in ["mobile_high", "medium", "hd", "1080p", "4k"] {
            let prefix = format!("video_{name}_");
            let Some(path) = text(&format!("{prefix}download")) else {
                continue;
            };
            let mut format: VideoFormat = serde_json::from_value(json!({
                "format_id": name,
                "url": format!("{base}{path}"),
                "ext": super::direct::determine_ext(path).unwrap_or_else(|| "unknown_video".into()),
                "width": int(&format!("{prefix}width")),
                "height": int(&format!("{prefix}height")),
                "filesize": int(&format!("{prefix}size")),
            }))
            .map_err(|error| Error::Reported(format!("[{NAME}] {id}: {error}")))?;
            crate::info_order::mark_format(&mut format, "23video");
            formats.push(format);
        }
        let mut thumbnails = Vec::new();
        for name in [
            "quad16", "quad50", "quad75", "quad100", "small", "portrait", "standard", "medium",
            "large", "original",
        ] {
            let prefix = format!("{name}_");
            let Some(path) = text(&format!("{prefix}download")) else {
                continue;
            };
            let dimension = |key: &str| {
                int(&format!("{prefix}{key}"))
                    .as_u64()
                    .map(|value| value as u32)
            };
            let mut extra = serde_json::Map::new();
            extra.insert("filesize".into(), int(&format!("{prefix}size")));
            thumbnails.push(crate::info::Thumbnail {
                url: format!("{base}{path}"),
                id: Some(name.to_owned()),
                width: dimension("width"),
                height: dimension("height"),
                preference: None,
                resolution: None,
                extra,
            });
        }
        let first = formats
            .first()
            .map_or_else(|| url.clone(), |format| format.url.clone());
        let mut info = super::direct::build_at(first.clone(), &first, String::new());
        info.formats = formats;
        info.direct = None;
        info.id = id;
        match title.as_str() {
            Some(title) if !title.is_empty() => info.title = title.to_owned(),
            _ => {
                info.title = String::new();
                info.extra
                    .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
            }
        }
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = KEY.into();
        info.metadata.thumbnails = thumbnails;
        for (key, value) in [
            ("timestamp", int("creation_date_epoch")),
            ("duration", int("video_length")),
            ("view_count", int("view_count")),
            ("comment_count", int("number_of_comments")),
            (
                "uploader_id",
                photo.get("user_id").cloned().unwrap_or(Value::Null),
            ),
            (
                "uploader",
                photo.get("display_name").cloned().unwrap_or(Value::Null),
            ),
        ] {
            if value.is_null() {
                info.extra.insert(key.into(), Value::Null);
            } else {
                crate::metadata_parser::set_field(&mut info, key, value);
            }
        }
        Ok(Some(crate::info_order::mark_info(info, "23video")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://video.twentythree.net/v.ihtml/player.html?showDescriptions=0&source=site&photo%5fid=20448876&autoPlay=1"
        ));
        assert!(!suitable(
            "https://video.twentythree.net/v.ihtml/player.html?x=1"
        ));
    }
}

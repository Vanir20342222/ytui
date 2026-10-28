//! Upstream `ReverbNationIE` (`yt_dlp/extractor/reverbnation.py`): a song's
//! API JSON as a single-URL (untyped) result.
use super::{Extractor, download_json_note};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct ReverbNation;

const NAME: &str = "ReverbNation";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?reverbnation\.com/.*?/song/(?P<id>\d+).*?$").unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `str_or_none`.
fn str_or_none(value: Option<&Value>) -> Value {
    match value {
        None | Some(Value::Null) => Value::Null,
        Some(Value::String(text)) => text.clone().into(),
        Some(Value::Bool(value)) => if *value { "True" } else { "False" }.into(),
        Some(other) => other.to_string().into(),
    }
}

impl Extractor for ReverbNation {
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
        let api = Url::parse(&format!("https://api.reverbnation.com/song/{id}"))?;
        let song = download_json_note(
            client,
            &api,
            NAME,
            &id,
            &format!("Downloading information of song {id}"),
        )?;
        let key_error = |key: &str| {
            Error::Reported(format!(
                "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        };
        // `qualities(('thumbnail', 'image'))` of the truthy keys.
        let mut thumbnails = Vec::new();
        for (preference, key) in ["thumbnail", "image"].into_iter().enumerate() {
            let value = song.get(key);
            let truthy = match value {
                None | Some(Value::Null | Value::Bool(false)) => false,
                Some(Value::String(text)) => !text.is_empty(),
                Some(Value::Number(number)) => number.as_f64() != Some(0.0),
                Some(Value::Array(items)) => !items.is_empty(),
                Some(Value::Object(items)) => !items.is_empty(),
                Some(Value::Bool(true)) => true,
            };
            if !truthy {
                continue;
            }
            let Some(Value::String(thumbnail)) = value else {
                return Err(Error::UnsupportedHtml5(
                    "non-string ReverbNation thumbnails are not supported natively".into(),
                ));
            };
            thumbnails.push(crate::info::Thumbnail {
                url: thumbnail.clone(),
                id: None,
                width: None,
                height: None,
                preference: Some(preference as i64),
                resolution: None,
                extra: Default::default(),
            });
        }
        let title = match song.get("name") {
            Some(Value::String(title)) => title.clone(),
            Some(_) => {
                return Err(Error::UnsupportedHtml5(
                    "non-string ReverbNation names are not supported natively".into(),
                ));
            }
            None => return Err(key_error("name")),
        };
        let media = match song.get("url") {
            Some(Value::String(media)) => Url::parse(media)?,
            Some(_) => {
                return Err(Error::UnsupportedHtml5(
                    "non-string ReverbNation URLs are not supported natively".into(),
                ));
            }
            None => return Err(key_error("url")),
        };
        let artist = song.get("artist");
        let mut info = super::direct::with_format(
            super::direct::build_at(media.clone(), &media, "mp3".into()),
            "",
        );
        info.vcodec = Some("none".into());
        for format in &mut info.formats {
            format.vcodec = Some("none".into());
            format.ext = "mp3".into();
            format.format_id = String::new();
        }
        info.format_id = None;
        info.id = id;
        info.title = title;
        info.webpage_url = url.clone();
        info.direct = None;
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        info.metadata.thumbnails = thumbnails;
        for (key, value) in [
            ("uploader", artist.and_then(|artist| artist.get("name"))),
            ("uploader_id", artist.and_then(|artist| artist.get("id"))),
        ] {
            let value = str_or_none(value);
            if !value.is_null() {
                crate::metadata_parser::set_field(&mut info, key, value);
            }
        }
        Ok(Some(crate::info_order::mark_info(info, "reverbnation")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://www.reverbnation.com/alkilados/song/16965047-mona-lisa"
        ));
        assert!(!suitable("https://www.reverbnation.com/alkilados"));
    }
}

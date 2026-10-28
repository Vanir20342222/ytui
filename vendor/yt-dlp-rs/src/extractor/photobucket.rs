//! Upstream `PhotobucketIE` (`yt_dlp/extractor/photobucket.py`): the page's
//! `Pb.Data.Shared.MEDIA` JSON with the embed code's `file=` URL as an
//! untyped result.
use super::{Extractor, download_webpage};
use crate::{Error, Result, VideoInfo};
use percent_encoding::percent_decode_str;
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Photobucket;

const NAME: &str = "Photobucket";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:[a-z0-9]+\.)?photobucket\.com/.*(([\?\&]current=)|_)(?P<id>.*)\.(?P<ext>(flv)|(mp4))",
    )
    .unwrap()
});

static MEDIA: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"Pb\.Data\.Shared\.put\(Pb\.Data\.Shared\.MEDIA, (.*?)\);").unwrap()
});
static FILE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"file=(.+\.mp4)").unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for Photobucket {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        let ext = found["ext"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &id)?;
        crate::screen::screen(&format!("[{NAME}] {id}: Extracting information"));
        let fatal = |what: &str| {
            Error::Reported(format!(
                "[{NAME}] {id}: Unable to extract {what}; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        };
        let media = MEDIA
            .captures(&webpage)
            .map(|found| found[1].to_owned())
            .ok_or_else(|| fatal("info json"))?;
        let info: Value = serde_json::from_str(&media)
            .map_err(|error| Error::Reported(format!("[{NAME}] {id}: {error}")))?;
        let key_error = |key: &str| {
            Error::Reported(format!(
                "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        };
        let get = |key: &str| info.get(key).cloned().ok_or_else(|| key_error(key));
        let embed = info
            .get("linkcodes")
            .ok_or_else(|| key_error("linkcodes"))?
            .get("html")
            .ok_or_else(|| key_error("html"))?
            .as_str()
            .ok_or_else(|| {
                Error::UnsupportedHtml5(
                    "non-string Photobucket embed codes are not supported natively".into(),
                )
            })?
            .to_owned();
        let file = FILE
            .captures(&embed)
            .map(|found| crate::html::clean_html(&found[1]))
            .ok_or_else(|| fatal("url"))?;
        let media = Url::parse(&percent_decode_str(&file).decode_utf8_lossy())?;
        let uploader = get("username")?;
        let timestamp = get("creationDate")?;
        let title = get("title")?;
        let thumbnail = get("thumbUrl")?;
        let mut result = super::direct::with_format(
            super::direct::build_at(media.clone(), &media, ext.clone()),
            "",
        );
        for format in &mut result.formats {
            format.ext = ext.clone();
            format.format_id = String::new();
        }
        result.format_id = None;
        result.id = id;
        match title.as_str() {
            Some(title) if !title.is_empty() => result.title = title.to_owned(),
            _ => {
                result.title = String::new();
                result
                    .extra
                    .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
            }
        }
        result.webpage_url = url.clone();
        result.direct = None;
        result.extractor = NAME.into();
        result.extractor_key = NAME.into();
        result.metadata.thumbnail = thumbnail.as_str().map(str::to_owned);
        if !timestamp.is_null() {
            crate::metadata_parser::set_field(&mut result, "timestamp", timestamp);
        }
        result.extra.insert("uploader".into(), uploader);
        Ok(Some(crate::info_order::mark_info(result, "photobucket")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        let url = "http://media.photobucket.com/user/r/media/Tired_zpsc0c3b9fa.mp4.html?x=1";
        let found = VALID_URL.captures(url).unwrap();
        assert_eq!(&found["id"], "zpsc0c3b9fa");
        assert!(!suitable("http://photobucket.com/images/x.jpg"));
    }
}

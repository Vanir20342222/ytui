//! Upstream `QingTingIE` (`yt_dlp/extractor/qingting.py`): the mobile
//! program page's `window.__initStores` JSON as an untyped `m4a` result.
use super::{Extractor, download_webpage, search_json};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct QingTing;

const NAME: &str = "QingTing";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.|m\.)?(?:qingting\.fm|qtfm\.cn)/v?channels/(?P<channel>\d+)/programs/(?P<id>\d+)",
    )
    .unwrap()
});

static STORES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"window\.__initStores\s*=").unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for QingTing {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let (channel_id, id) = (found["channel"].to_owned(), found["id"].to_owned());
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let page = Url::parse(&format!(
            "https://m.qtfm.cn/vchannels/{channel_id}/programs/{id}/"
        ))?;
        let webpage = download_webpage(client, &page, NAME, &id)?;
        let report = |message: &str| {
            Error::Reported(format!(
                "[{NAME}] {id}: {message}; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        };
        let info = match search_json(&STORES, &webpage, false) {
            None => return Err(report("Unable to extract program info")),
            Some(None) => {
                return Err(report(
                    "Unable to extract program info - Failed to parse JSON",
                ));
            }
            Some(Some(info)) => info,
        };
        // `traverse_obj(info, path)`.
        let path = |keys: &[&str]| -> Option<Value> {
            let mut value = &info;
            for key in keys {
                value = value.get(key)?;
            }
            (!value.is_null()).then(|| value.clone())
        };
        let title = path(&["ProgramStore", "programInfo", "title"]);
        let channel = path(&["ProgramStore", "channelInfo", "title"]);
        let uploader = path(&["ProgramStore", "podcasterInfo", "podcaster", "nickname"]);
        let Some(media) = path(&["ProgramStore", "programInfo", "audioUrl"]) else {
            // `process_video_result`: the title, then no formats.
            if title.is_none() {
                crate::screen::warning(
                    "Extractor failed to obtain \"title\". Creating a generic title instead",
                );
            }
            return Err(report("No video formats found!"));
        };
        let media = Url::parse(media.as_str().ok_or_else(|| {
            Error::UnsupportedHtml5(
                "non-string QingTing audio URLs are not supported natively".into(),
            )
        })?)?;
        let mut info_dict = super::direct::with_format(
            super::direct::build_at(media.clone(), &media, "m4a".into()),
            "",
        );
        for format in &mut info_dict.formats {
            format.ext = "m4a".into();
            format.vcodec = Some("none".into());
            format.acodec = Some("m4a".into());
            format.format_id = String::new();
        }
        info_dict.vcodec = Some("none".into());
        info_dict.acodec = Some("m4a".into());
        info_dict.format_id = None;
        info_dict.id = id;
        match title.as_ref().and_then(Value::as_str) {
            Some(title) => info_dict.title = title.to_owned(),
            None => {
                info_dict.title = String::new();
                info_dict
                    .extra
                    .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
            }
        }
        info_dict.webpage_url = url.clone();
        info_dict.direct = None;
        info_dict.extractor = NAME.into();
        info_dict.extractor_key = NAME.into();
        info_dict
            .extra
            .insert("channel_id".into(), channel_id.into());
        for (key, value) in [("channel", channel), ("uploader", uploader)] {
            if let Some(value) = value {
                crate::metadata_parser::set_field(&mut info_dict, key, value);
            } else {
                info_dict.extra.insert(key.into(), Value::Null);
            }
        }
        Ok(Some(crate::info_order::mark_info(info_dict, "qingting")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.qingting.fm/channels/378005/programs/22257411/"
        ));
        assert!(suitable(
            "https://m.qtfm.cn/vchannels/378005/programs/23023573/"
        ));
        assert!(!suitable("https://www.qingting.fm/channels/378005/"));
    }
}

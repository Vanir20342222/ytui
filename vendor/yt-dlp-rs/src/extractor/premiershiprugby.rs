//! Upstream `PremiershipRugbyIE` (`yt_dlp/extractor/premiershiprugby.py`):
//! the article CMS's hero media HLS stream and metadata.
use super::{Extractor, download_json, extract_m3u8_fatal};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct PremiershipRugby;

const NAME: &str = "PremiershipRugby";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:\w+\.)premiershiprugby\.(?:com)/watch/(?P<id>[\w-]+)").unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `value[key]`'s escaped `KeyError`.
fn key<'a>(value: &'a Value, name: &str, id: &str) -> Result<&'a Value> {
    match value {
        Value::Object(object) => object.get(name).ok_or_else(|| {
            Error::Reported(format!(
                "{id}: An extractor error has occurred. (caused by KeyError('{name}')); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        }),
        _ => Err(Error::UnsupportedHtml5(
            "Premiership Rugby values that are not objects are not supported natively".into(),
        )),
    }
}

/// A string value (other types are not modelled).
fn text<'a>(value: &'a Value, what: &str) -> Result<&'a str> {
    value.as_str().ok_or_else(|| {
        Error::UnsupportedHtml5(format!(
            "Premiership Rugby {what} values that are not strings are not supported natively"
        ))
    })
}

impl Extractor for PremiershipRugby {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let api = Url::parse(&format!(
            "https://article-cms-api.incrowdsports.com/v2/articles/slug/{display_id}?clientId=PRL"
        ))?;
        let response = download_json(client, &api, NAME, &display_id)?;
        let article = key(key(&response, "data", &display_id)?, "article", &display_id)?;
        let hero = key(article, "heroMedia", &display_id)?;
        let content = key(hero, "content", &display_id)?;
        let manifest = Url::parse(text(key(content, "videoLink", &display_id)?, "videoLink")?)?;
        let mut info = extract_m3u8_fatal(client, &manifest, NAME, &display_id, None, true)?;
        super::hls::without_ext(&mut info);
        let id = text(
            key(content, "sourceSystemId", &display_id)?,
            "sourceSystemId",
        )?;
        info.id = id.to_owned();
        match hero.get("title").and_then(Value::as_str) {
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
        crate::metadata_parser::set_field(&mut info, "display_id", display_id.into());
        info.metadata.thumbnail = content
            .get("videoThumbnail")
            .map(|thumbnail| text(thumbnail, "videoThumbnail").map(str::to_owned))
            .transpose()?;
        // `int_or_none(..., scale=1000)`: `int(v) // 1000`.
        let duration = super::streamable::int_or_none(
            content
                .get("metadata")
                .and_then(|metadata| metadata.get("msDuration")),
        );
        if let Some(duration) = duration.as_i64() {
            crate::metadata_parser::set_field(
                &mut info,
                "duration",
                duration.div_euclid(1000).into(),
            );
        }
        // `json_data.get('tags')` as is.
        info.extra.insert(
            "tags".into(),
            article.get("tags").cloned().unwrap_or(Value::Null),
        );
        // `traverse_obj(json_data, ('categories', ..., 'text'))`.
        let categories: Vec<Value> = match article.get("categories") {
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(|item| item.get("text"))
                .filter(|text| !text.is_null())
                .cloned()
                .collect(),
            Some(Value::Object(items)) => items
                .values()
                .filter_map(|item| item.get("text"))
                .filter(|text| !text.is_null())
                .cloned()
                .collect(),
            _ => Vec::new(),
        };
        info.extra
            .insert("categories".into(), Value::Array(categories));
        Ok(Some(crate::info_order::mark_info(info, "premiershiprugby")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.premiershiprugby.com/watch/full-match-harlequins-v-newcastle-falcons"
        ));
        assert!(!suitable("https://premiershiprugby.com/watch/x"));
    }
}

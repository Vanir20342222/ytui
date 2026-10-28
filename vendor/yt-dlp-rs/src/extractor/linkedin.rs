//! Upstream `LinkedInIE` (`yt_dlp/extractor/linkedin.py`): a public post's
//! `<video>` `data-sources` formats and `data-captions-url` track, with the
//! OpenGraph/title metadata, reaction count and JSON-LD author. LinkedIn
//! Learning and events (which need a login) are not supported.
use super::Extractor;
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct LinkedIn;

const NAME: &str = "LinkedIn";

static VALID_URLS: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        Regex::new(
            r"^https?://(?:www\.)?linkedin\.com/posts/[^/?#]+-(?P<id>\d+)-\w{4}/?(?:[?#]|$)",
        )
        .unwrap(),
        Regex::new(r"^https?://(?:www\.)?linkedin\.com/feed/update/urn:li:activity:(?P<id>\d+)")
            .unwrap(),
    ]
});
static VIDEO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(<video[^>]+>)").unwrap());
static REACTIONS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\bdata-num-reactions="(\d+)""#).unwrap());

/// `LinkedInIE.suitable`.
pub fn suitable(url: &str) -> bool {
    VALID_URLS.iter().any(|pattern| pattern.is_match(url))
}

fn fail(id: &str, message: &str) -> Error {
    Error::Reported(format!("[{NAME}] {id}: {message}"))
}

fn bug(id: &str, cause: &str) -> Error {
    fail(
        id,
        &format!(
            "An extractor error has occurred. (caused by {cause}); {}",
            crate::BUG_REPORTS_MESSAGE
        ),
    )
}

impl Extractor for LinkedIn {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, page: &Url) -> Result<Option<ExtractResult>> {
        let Some(video_id) = VALID_URLS
            .iter()
            .find_map(|pattern| pattern.captures(page.as_str()))
            .map(|found| found["id"].to_owned())
        else {
            return Ok(None);
        };
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(page.as_str())
        ));
        let webpage = super::download_webpage(client, page, NAME, &video_id)?;
        let Some(tag) = VIDEO.captures(&webpage) else {
            return Err(fail(
                &video_id,
                &format!("Unable to extract video; {}", crate::BUG_REPORTS_MESSAGE),
            ));
        };
        let attrs = crate::html_elements::extract_attributes(&tag[1]);
        let Some(sources) = attrs.get("data-sources").and_then(Value::as_str) else {
            return Err(bug(&video_id, "KeyError('data-sources')"));
        };
        let sources: Value = serde_json::from_str(sources).map_err(|error| {
            fail(
                &video_id,
                &format!("{video_id}: Failed to parse JSON  (caused by {error})"),
            )
        })?;
        let mut formats = Vec::new();
        for source in sources.as_array().into_iter().flatten() {
            let Some(src) = source.get("src") else {
                return Err(bug(&video_id, "KeyError('src')"));
            };
            let url = src.as_str().unwrap_or_default();
            let ext = source
                .get("type")
                .and_then(Value::as_str)
                .map(|mime| super::direct::mimetype2ext_or(mime, None))
                .or_else(|| super::direct::determine_ext(url))
                .unwrap_or_else(|| "unknown_video".into());
            let tbr = source.get("data-bitrate").and_then(|rate| {
                rate.as_f64()
                    .or_else(|| rate.as_str()?.trim().parse().ok())
                    .map(|rate| rate / 1000.0)
            });
            let mut map = Map::new();
            map.insert("url".into(), url.into());
            map.insert("ext".into(), ext.into());
            if let Some(tbr) = tbr {
                map.insert("tbr".into(), json!(tbr));
            }
            map.insert("format_id".into(), "".into());
            let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(map))
                .map_err(|error| fail(&video_id, &error.to_string()))?;
            crate::info_order::mark_format(&mut format, "dict:url,ext,tbr");
            formats.push(format);
        }
        let mut subtitles = crate::Subtitles::default();
        if let Some(captions) = attrs
            .get("data-captions-url")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
            .and_then(|url| Url::parse(&url).ok())
        {
            let track: crate::SubtitleFormat =
                serde_json::from_value(json!({"url": captions.as_str(), "ext": "vtt"}))
                    .map_err(|error| fail(&video_id, &error.to_string()))?;
            subtitles.entries.push((Some("en".into()), vec![track]));
        }
        let title = crate::html::og_search_property(&webpage, "title")
            .or_else(|| crate::html::html_extract_title(&webpage));
        let uploader = super::json_ld::yield_json_ld(&webpage)
            .into_iter()
            .filter(|item| item.get("@type").and_then(Value::as_str) == Some("SocialMediaPosting"))
            .find_map(|item| {
                item.pointer("/author/name")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            });
        let mut dict = Map::new();
        dict.insert("id".into(), video_id.clone().into());
        dict.insert("formats".into(), json!([]));
        dict.insert(
            "title".into(),
            title.clone().map_or(Value::Null, Value::from),
        );
        dict.insert(
            "like_count".into(),
            REACTIONS
                .captures(&webpage)
                .and_then(|found| found[1].parse::<i64>().ok())
                .into(),
        );
        dict.insert("uploader".into(), uploader.map_or(Value::Null, Value::from));
        // `_og_search_thumbnail(webpage)` (non-fatal, no default).
        let thumbnail = crate::html::og_search_property(&webpage, "image");
        if thumbnail.is_none() {
            super::search_warning(NAME, "thumbnail URL");
        }
        dict.insert(
            "thumbnail".into(),
            thumbnail.map_or(Value::Null, Value::from),
        );
        dict.insert(
            "description".into(),
            crate::html::og_search_property(&webpage, "description")
                .map_or(Value::Null, Value::from),
        );
        dict.insert("subtitles".into(), json!({}));

        let first = formats
            .first()
            .map_or_else(|| page.clone(), |format| format.url.clone());
        let mut info = super::direct::build_at(first.clone(), &first, String::new());
        info.direct = None;
        if formats.is_empty() {
            info.extra
                .insert(crate::info::NO_FORMATS.into(), Value::Bool(true));
        }
        info.formats = formats;
        info.subtitles = subtitles;
        info.webpage_url = page.clone();
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        if title.is_none() {
            info.extra
                .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
        }
        let mut keys = Vec::new();
        for (key, value) in dict {
            keys.push(key.clone());
            match key.as_str() {
                "id" => info.id = value.as_str().unwrap_or_default().to_owned(),
                "title" => info.title = value.as_str().unwrap_or_default().to_owned(),
                "formats" | "subtitles" => {}
                _ if value.is_null() => {
                    info.extra.insert(key, Value::Null);
                }
                _ => crate::metadata_parser::set_field(&mut info, &key, value),
            }
        }
        Ok(Some(ExtractResult::Video(crate::info_order::mark_info(
            info,
            &format!("dict;{};subtitle=url,ext", keys.join(",")),
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_urls() {
        assert!(suitable(
            "https://www.linkedin.com/posts/mishalkhawaja_sendinblueviews-toronto-digitalmarketing-ugcPost-6850898786781339649-mM20"
        ));
        assert!(suitable(
            "https://www.linkedin.com/feed/update/urn:li:activity:7016901149999955968/?utm_source=share"
        ));
        assert!(!suitable(
            "https://www.linkedin.com/learning/some-course/intro"
        ));
    }
}

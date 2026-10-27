//! Upstream `StreamableIE` (`yt_dlp/extractor/streamable.py`): the ajax
//! API's video files as formats.
use super::{Extractor, download_json, proto_relative};
use crate::{Error, Result, VideoFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Streamable;

const NAME: &str = "Streamable";

static VALID_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://streamable\.com/(?:[es]/)?(?P<id>\w+)").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_match_id`'s ID of a suitable URL.
pub(crate) fn temp_id(url: &str) -> Option<String> {
    VALID_URL.captures(url).map(|found| found["id"].to_owned())
}

/// `_EMBED_REGEX` (`q1` is one quote character, so the backreference is
/// an alternation of both).
static EMBED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"<iframe[^>]+\bsrc=(?:"(?P<a>(?:https?:)?//streamable\.com/.+?)"|'(?P<b>(?:https?:)?//streamable\.com/.+?)')"#,
    )
    .unwrap()
});

/// `_extract_embed_urls` under `orderedSet`: the page's suitable iframe
/// URLs, unescaped and joined to `url`, without repeats.
pub(crate) fn embed_urls(url: &Url, webpage: &str) -> Vec<Url> {
    let mut found: Vec<Url> = Vec::new();
    for captures in EMBED.captures_iter(webpage) {
        let Some(group) = captures.name("a").or_else(|| captures.name("b")) else {
            continue;
        };
        let Ok(embed) = url.join(&crate::html::unescape_html(group.as_str())) else {
            continue;
        };
        if suitable(embed.as_str()) && !found.contains(&embed) {
            found.push(embed);
        }
    }
    found
}

/// `int_or_none`.
pub(crate) fn int_or_none(value: Option<&Value>) -> Value {
    match value {
        Some(Value::Number(number)) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value.trunc() as i64))
            .map_or(Value::Null, Value::from),
        Some(Value::String(text)) => text.trim().parse::<i64>().map_or(Value::Null, Value::from),
        _ => Value::Null,
    }
}

/// `float_or_none(value, scale)`.
fn float_or_none(value: Option<&Value>, scale: f64) -> Value {
    let number = match value {
        Some(Value::Number(number)) => number.as_f64(),
        Some(Value::String(text)) => text.trim().parse::<f64>().ok(),
        _ => None,
    };
    number.map_or(Value::Null, |number| Value::from(number / scale))
}

/// `parse_codecs(name).get(key)`.
fn codec(name: Option<&Value>, video: bool) -> Value {
    let codecs = crate::codecs::parse_codecs(name.and_then(Value::as_str), &mut |warning| {
        crate::screen::warning(warning);
    });
    let value = codecs.map(|codecs| if video { codecs.vcodec } else { codecs.acodec });
    match value {
        Some(value) => value.into(),
        None => Value::Null,
    }
}

impl Extractor for Streamable {
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
        let api = Url::parse(&format!("https://ajax.streamable.com/videos/{id}"))?;
        let video = download_json(client, &api, NAME, &id)?;
        if video.get("status").and_then(Value::as_i64) != Some(2) {
            return Err(Error::Reported(format!(
                "[{NAME}] {id}: This video is currently unavailable. It may still be uploading or processing."
            )));
        }
        let title = video
            .get("reddit_title")
            .filter(|title| title.as_str().is_some_and(|title| !title.is_empty()))
            .or_else(|| video.get("title"))
            .and_then(Value::as_str)
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {id}: An extractor error has occurred. (caused by KeyError('title'))"
                ))
            })?
            .to_owned();
        let mut formats = Vec::new();
        for (key, info) in video
            .get("files")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            let Some(file) = info
                .get("url")
                .and_then(Value::as_str)
                .filter(|url| !url.is_empty())
            else {
                continue;
            };
            let file = proto_relative(file);
            let Ok(file_url) = Url::parse(&file) else {
                continue;
            };
            let metadata = info.get("input_metadata");
            let format = serde_json::json!({
                "format_id": key,
                "url": file_url.as_str(),
                "width": int_or_none(info.get("width")),
                "height": int_or_none(info.get("height")),
                "filesize": int_or_none(info.get("size")),
                "fps": int_or_none(info.get("framerate")),
                "vbr": float_or_none(info.get("bitrate"), 1000.0),
                "vcodec": codec(metadata.and_then(|metadata| metadata.get("video_codec_name")), true),
                "acodec": codec(metadata.and_then(|metadata| metadata.get("audio_codec_name")), false),
                "ext": super::direct::determine_ext(file_url.as_str()).unwrap_or_else(|| "unknown_video".into()),
            });
            let mut format: VideoFormat = serde_json::from_value(format)
                .map_err(|error| Error::Reported(format!("[{NAME}] {id}: {error}")))?;
            crate::info_order::mark_format(&mut format, "streamable");
            formats.push(format);
        }
        let first = formats
            .first()
            .map_or(url.clone(), |format| format.url.clone());
        let mut info = super::direct::build_at(first.clone(), &first, "mp4".into());
        info.formats = formats;
        info.id = id;
        info.title = title;
        info.webpage_url = url.clone();
        info.direct = None;
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        let field = |value: Option<&Value>| value.cloned().unwrap_or(Value::Null);
        for (key, value) in [
            ("description", field(video.get("description"))),
            (
                "thumbnail",
                video
                    .get("thumbnail_url")
                    .and_then(Value::as_str)
                    .map_or(Value::Null, |url| proto_relative(url).into()),
            ),
            (
                "uploader",
                field(video.get("owner").and_then(|owner| owner.get("user_name"))),
            ),
            ("timestamp", float_or_none(video.get("date_added"), 1.0)),
            ("duration", float_or_none(video.get("duration"), 1.0)),
            ("view_count", int_or_none(video.get("plays"))),
        ] {
            if !value.is_null() {
                crate::metadata_parser::set_field(&mut info, key, value);
            }
        }
        Ok(Some(crate::info_order::mark_info(info, "streamable")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://streamable.com/dnd1"));
        assert!(suitable("https://streamable.com/e/dnd1"));
        assert!(suitable("https://streamable.com/s/okkqk/drxjds"));
        assert!(!suitable("https://example.com/dnd1"));
    }

    #[test]
    fn embeds_follow_upstream_embed_regex() {
        let page = Url::parse("http://blog.invalid/post").unwrap();
        let html = r#"<iframe src='//streamable.com/e/a1?x=1&amp;y=2'></iframe>
            <iframe width="1" src="https://streamable.com/s/b2/c"></iframe>
            <iframe src="//streamable.com/e/a1?x=1&amp;y=2"></iframe>
            <iframe src="https://streamable.com/"></iframe><a src="https://streamable.com/z">"#;
        let found: Vec<String> = embed_urls(&page, html).iter().map(Url::to_string).collect();
        assert_eq!(
            found,
            [
                "http://streamable.com/e/a1?x=1&y=2",
                "https://streamable.com/s/b2/c"
            ]
        );
    }
}

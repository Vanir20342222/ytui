//! Upstream `RumbleEmbedIE` and `RumbleIE` (`yt_dlp/extractor/rumble.py`):
//! the embed JS API's formats (progressive, HLS, audio and timeline),
//! subtitles and metadata, and video pages as `url_transparent` results of
//! their embed with the page's counts and description.
use super::Extractor;
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct RumbleEmbed;
pub struct Rumble;

const EMBED_NAME: &str = "RumbleEmbed";
const NAME: &str = "Rumble";

static EMBED_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?rumble\.com/embed/(?:[0-9a-z]+\.)?(?P<id>[0-9a-z]+)").unwrap()
});
static VIDEO_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?rumble\.com/(?P<id>v[\w.-]+)[^/]*$").unwrap()
});
static EMBED_IN_PAGE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:<(?:script|iframe)[^>]+\bsrc=|["']embedUrl["']\s*:\s*)["'](?P<url>https?://(?:www\.)?rumble\.com/embed/(?:[0-9a-z]+\.)?[0-9a-z]+)"#).unwrap()
});
static PLAY_SCRIPT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<script>[^<]*\bRumble\(\s*"play"\s*,\s*\{[^}]*['"]?video['"]?\s*:\s*['"](?P<id>[0-9a-z]+)['"]"#)
        .unwrap()
});

/// `RumbleEmbedIE.suitable`.
pub fn embed_suitable(url: &str) -> bool {
    EMBED_URL.is_match(url)
}

/// `RumbleIE.suitable` (`v(?!ideos)`).
pub fn suitable(url: &str) -> bool {
    VIDEO_URL
        .captures(url)
        .is_some_and(|found| !found["id"].starts_with("videos"))
}

fn int(value: Option<&Value>) -> Option<i64> {
    let value = value?;
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|number| number as i64))
        .or_else(|| value.as_str()?.trim().parse().ok())
}

fn fail(name: &str, id: &str, message: &str) -> Error {
    Error::Reported(format!("[{name}] {id}: {message}"))
}

impl Extractor for RumbleEmbed {
    fn key(&self) -> &'static str {
        EMBED_NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = EMBED_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(EMBED_NAME);
        crate::screen::screen(&format!(
            "[{EMBED_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let mut api = Url::parse("https://rumble.com/embedJS/u3/")?;
        api.query_pairs_mut()
            .append_pair("request", "video")
            .append_pair("ver", "2")
            .append_pair("v", &video_id);
        let video = super::download_json(client, &api, EMBED_NAME, &video_id)?;
        if let Some(message) = video
            .pointer("/sys/msg")
            .and_then(Value::as_str)
            .filter(|m| !m.is_empty())
        {
            crate::screen::warning(&format!("[{EMBED_NAME}] {video_id}: {message}"));
        }
        let dvr = video.get("livestream_has_dvr");
        let mut live_status = match video.get("live").and_then(Value::as_i64) {
            Some(0) => Some(if dvr.is_none_or(Value::is_null) {
                "not_live"
            } else {
                "was_live"
            }),
            Some(1) => Some(
                if dvr.is_some_and(|dvr| {
                    dvr.as_bool() == Some(true) || dvr.as_i64().is_some_and(|n| n != 0)
                }) {
                    "is_upcoming"
                } else {
                    "was_live"
                },
            ),
            Some(2) => Some("is_live"),
            _ => None,
        };
        let mut formats = Vec::new();
        for (format_type, format_info) in video
            .get("ua")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            let entries: Vec<Value> = match format_info {
                Value::Object(by_height) => by_height
                    .iter()
                    .map(|(height, info)| {
                        let mut info = info.clone();
                        if int(info.pointer("/meta/h")).is_none_or(|h| h == 0)
                            && let Some(object) = info.as_object_mut()
                        {
                            let meta = object.entry("meta").or_insert_with(|| json!({}));
                            if let Some(meta) = meta.as_object_mut() {
                                meta.insert("h".into(), height.clone().into());
                            }
                        }
                        info
                    })
                    .collect(),
                Value::Array(items) => items.clone(),
                _ => Vec::new(),
            };
            for info in entries {
                let meta = info.get("meta").cloned().unwrap_or_else(|| json!({}));
                let Some(format_url) = info
                    .get("url")
                    .and_then(Value::as_str)
                    .filter(|u| !u.is_empty())
                else {
                    continue;
                };
                if format_type == "tar" {
                    continue;
                }
                if format_type == "hls" {
                    if meta.get("live") == Some(&Value::Bool(true))
                        && video.get("live").and_then(Value::as_i64) == Some(1)
                    {
                        live_status = Some("post_live");
                    }
                    if let Some(stream) = Url::parse(format_url).ok().and_then(|manifest| {
                        super::extract_m3u8_optional(
                            client,
                            &manifest,
                            EMBED_NAME,
                            &video_id,
                            Some("hls"),
                            false,
                        )
                    }) {
                        formats.extend(stream.formats);
                    }
                    continue;
                }
                let timeline = format_type == "timeline";
                let audio = format_type == "audio";
                let height_label = match meta.get("h") {
                    Some(Value::Null) | None => None,
                    Some(Value::String(text)) => Some(format!("{text}p")),
                    Some(other) => Some(format!("{other}p")),
                };
                let mut format = Map::new();
                format.insert(
                    "acodec".into(),
                    if timeline {
                        Value::from("none")
                    } else {
                        Value::Null
                    },
                );
                format.insert(
                    "vcodec".into(),
                    if audio {
                        Value::from("none")
                    } else {
                        Value::Null
                    },
                );
                format.insert("url".into(), format_url.into());
                format.insert(
                    "format_id".into(),
                    match height_label {
                        Some(label) => format!("{format_type}-{label}"),
                        None => format_type.clone(),
                    }
                    .into(),
                );
                format.insert(
                    "format_note".into(),
                    if timeline {
                        Value::from("Timeline")
                    } else {
                        Value::Null
                    },
                );
                format.insert(
                    "fps".into(),
                    if timeline || audio {
                        Value::Null
                    } else {
                        video.get("fps").cloned().unwrap_or(Value::Null)
                    },
                );
                for (key, source) in [
                    ("tbr", "bitrate"),
                    ("filesize", "size"),
                    ("width", "w"),
                    ("height", "h"),
                ] {
                    if let Some(value) = int(meta.get(source)) {
                        format.insert(key.into(), value.into());
                    }
                }
                let keys: Vec<String> = format.keys().cloned().collect();
                for key in ["acodec", "vcodec", "format_note", "fps"] {
                    if format.get(key).is_some_and(Value::is_null) {
                        format.shift_remove(key);
                    }
                }
                format.insert(
                    "ext".into(),
                    super::direct::determine_ext(format_url)
                        .unwrap_or_else(|| "unknown_video".into())
                        .into(),
                );
                let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(format))
                    .map_err(|error| fail(EMBED_NAME, &video_id, &error.to_string()))?;
                crate::info_order::mark_format(&mut format, &format!("dict:{}", keys.join(",")));
                crate::info_order::mark_int_rates(&mut format);
                formats.push(format);
            }
        }
        let mut subtitles = crate::Subtitles::default();
        for (language, info) in video
            .get("cc")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            let Some(path) = info
                .get("path")
                .and_then(Value::as_str)
                .filter(|p| !p.is_empty())
            else {
                continue;
            };
            let name = info
                .get("language")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let ext = super::direct::determine_ext(path).unwrap_or_else(|| "unknown_video".into());
            if let Ok(track) =
                serde_json::from_value(json!({"url": path, "name": name, "ext": ext}))
            {
                subtitles
                    .entries
                    .push((Some(language.clone()), vec![track]));
            }
        }
        let author = video.get("author").cloned().unwrap_or_else(|| json!({}));
        let mut thumbnails: Vec<Value> = video
            .get("t")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|thumb| {
                let mut found = Map::new();
                for (key, source) in [("url", "i"), ("width", "w"), ("height", "h")] {
                    if let Some(value) = thumb.get(source).filter(|v| !v.is_null()) {
                        found.insert(key.into(), value.clone());
                    }
                }
                Value::Object(found)
            })
            .collect();
        if thumbnails.is_empty()
            && let Some(image) = video
                .get("i")
                .and_then(Value::as_str)
                .filter(|i| !i.is_empty())
        {
            thumbnails.push(json!({"url": image}));
        }
        let duration = if matches!(live_status, Some("is_live" | "post_live")) {
            None
        } else {
            int(video.get("duration"))
        };
        let mut dict = Map::new();
        dict.insert("id".into(), video_id.clone().into());
        dict.insert(
            "title".into(),
            video
                .get("title")
                .and_then(Value::as_str)
                .map(crate::html::unescape_html)
                .into(),
        );
        dict.insert("formats".into(), json!([]));
        dict.insert("subtitles".into(), json!({}));
        dict.insert("thumbnails".into(), Value::Array(thumbnails));
        dict.insert(
            "timestamp".into(),
            video
                .get("pubDate")
                .and_then(Value::as_str)
                .and_then(crate::dates::parse_iso8601)
                .into(),
        );
        dict.insert(
            "channel".into(),
            author.get("name").cloned().unwrap_or(Value::Null),
        );
        dict.insert(
            "channel_url".into(),
            author.get("url").cloned().unwrap_or(Value::Null),
        );
        dict.insert("duration".into(), duration.into());
        dict.insert(
            "uploader".into(),
            author.get("name").cloned().unwrap_or(Value::Null),
        );
        dict.insert("live_status".into(), live_status.into());
        let first = formats
            .first()
            .map_or_else(|| url.clone(), |format| format.url.clone());
        let mut info = super::direct::build_at(first.clone(), &first, String::new());
        info.direct = None;
        if formats.is_empty() {
            info.extra
                .insert(crate::info::NO_FORMATS.into(), Value::Bool(true));
        }
        info.formats = formats;
        info.subtitles = subtitles;
        info.webpage_url = url.clone();
        info.extractor = EMBED_NAME.into();
        info.extractor_key = EMBED_NAME.into();
        let mut keys = Vec::new();
        let titled = dict.get("title").is_some_and(|title| !title.is_null());
        if !titled {
            info.extra
                .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
        }
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
        Ok(Some(crate::info_order::mark_info(
            info,
            &format!(
                "dict{};{};subtitle=url,name;thumbnail=url,width,height",
                if titled { "" } else { "-untitled" },
                keys.join(",")
            ),
        )))
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        Ok(self.extract(client, url)?.map(ExtractResult::Video))
    }
}

impl Extractor for Rumble {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        if !suitable(url.as_str()) {
            return Ok(None);
        }
        let page_id = VIDEO_URL
            .captures(url.as_str())
            .map_or_else(String::new, |found| found["id"].to_owned());
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = super::download_webpage(client, url, NAME, &page_id)?;
        let embed = EMBED_IN_PAGE
            .captures(&webpage)
            .map(|found| crate::html::unescape_html(&found["url"]))
            .or_else(|| {
                PLAY_SCRIPT
                    .captures(&webpage)
                    .map(|found| format!("https://rumble.com/embed/{}", &found["id"]))
            });
        let Some(embed) = embed else {
            return Err(Error::Reported(format!("Unsupported URL: {url}")));
        };
        let search = |pattern: &str| {
            Regex::new(pattern)
                .ok()?
                .captures(&webpage)
                .map(|found| found[1].to_owned())
        };
        let count = |pattern: &str| {
            search(pattern).and_then(|text| super::youtube::dict::parse_count(&text))
        };
        let description =
            crate::html_elements::elements_text_and_html_by_class("media-description", &webpage)
                .ok()
                .and_then(|found| found.into_iter().next())
                .map(|(content, _)| crate::html::clean_html(&content));
        let mut fields = Map::new();
        fields.insert(
            "release_timestamp".into(),
            search(r#"(?:Livestream begins|Streamed on):\s+<time datetime="([^"]+)"#)
                .and_then(|date| crate::dates::parse_iso8601(&date))
                .into(),
        );
        fields.insert(
            "view_count".into(),
            search(r#""userInteractionCount"\s*:\s*(\d+)"#)
                .and_then(|views| views.parse::<i64>().ok())
                .into(),
        );
        fields.insert(
            "like_count".into(),
            count(r#"<span data-js="rumbles_up_votes">\s*([\d,.KM]+)"#).into(),
        );
        fields.insert(
            "dislike_count".into(),
            count(r#"<span data-js="rumbles_down_votes">\s*([\d,.KM]+)"#).into(),
        );
        fields.insert("description".into(), description.into());
        let mut follow = super::direct::build_at(url.clone(), url, String::new());
        follow.formats.clear();
        follow.direct = None;
        follow.extra.insert(
            crate::FOLLOW_TRANSPARENT.into(),
            json!({"url": embed, "fields": Value::Object(fields)}),
        );
        Ok(Some(ExtractResult::Video(follow)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(embed_suitable("https://rumble.com/embed/v5pv5f"));
        assert!(embed_suitable("https://rumble.com/embed/ufe9n.v5pv5f"));
        assert!(suitable(
            "https://rumble.com/vdmum1-moose-the-dog-helps-girls-dig-a-snow-fort.html"
        ));
        assert!(suitable("http://www.rumble.com/vDMUM1?key=value"));
        assert!(!suitable("https://rumble.com/videos?page=2"));
        assert!(!suitable("https://rumble.com/c/Example"));
    }
}

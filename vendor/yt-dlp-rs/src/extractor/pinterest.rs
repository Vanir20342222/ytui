//! Upstream `PinterestIE` (`yt_dlp/extractor/pinterest.py`): the Pin
//! resource API, the pin's video list (HLS or progressive) or story pin
//! video, its thumbnails and metadata, and embedded videos as
//! `url_transparent` results.
use super::Extractor;
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Pinterest;

const NAME: &str = "Pinterest";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:[^/]+\.)?pinterest\.(?:com|fr|de|ch|jp|cl|ca|it|co\.uk|nz|ru|com\.au|at|pt|co\.kr|es|com\.mx|dk|ph|th|com\.uy|co|nl|info|kr|ie|vn|com\.vn|ec|mx|in|pe|co\.at|hu|co\.in|co\.nz|id|com\.ec|com\.py|tw|be|uk|com\.bo|com\.pe)/pin/(?:[\w-]+--)?(?P<id>\d+)")
        .unwrap()
});

/// `PinterestIE.suitable`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn int(value: Option<&Value>) -> Option<i64> {
    let value = value?;
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|number| number as i64))
        .or_else(|| value.as_str()?.trim().parse().ok())
}

fn first<'a>(data: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter()
        .find_map(|key| data.get(*key).filter(|value| !value.is_null()))
}

impl Extractor for Pinterest {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, page: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(page.as_str()) else {
            return Ok(None);
        };
        let pin_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(page.as_str())
        ));
        // `_call_api('Pin', id, options)`.
        let options = json!({"options": {"field_set_key": "unauth_react_main_pin", "id": pin_id}});
        let mut api = Url::parse("https://www.pinterest.com/resource/PinResource/get/")?;
        api.query_pairs_mut()
            .append_pair("data", &crate::output::python_json_dumps(&options));
        let response = super::download_json_note_headers(
            client,
            &api,
            NAME,
            &pin_id,
            "Download Pin JSON metadata",
            &[("X-Pinterest-PWS-Handler", "www/[username].js")],
        )?;
        let data = response
            .pointer("/resource_response/data")
            .filter(|data| data.is_object())
            .cloned()
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {pin_id}: An extractor error has occurred. (caused by KeyError('data')); {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let video_id = match data.get("id") {
            Some(Value::String(text)) => text.clone(),
            Some(Value::Number(number)) => number.to_string(),
            _ => pin_id.clone(),
        };
        let mut thumbnails = Vec::new();
        for image in data
            .get("images")
            .and_then(Value::as_object)
            .into_iter()
            .flat_map(|images| images.values())
            .filter(|image| image.is_object())
        {
            let Some(url) = image
                .get("url")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
            else {
                continue;
            };
            thumbnails.push(json!({
                "url": url,
                "width": int(image.get("width")),
                "height": int(image.get("height")),
            }));
        }
        let title = first(&data, &["title", "grid_title"])
            .and_then(Value::as_str)
            .map(|title| title.trim().to_owned())
            .unwrap_or_default();
        let mut info = Map::new();
        info.insert("title".into(), title.clone().into());
        info.insert(
            "description".into(),
            first(&data, &["seo_description", "description"])
                .cloned()
                .unwrap_or(Value::Null),
        );
        info.insert(
            "timestamp".into(),
            data.get("created_at")
                .and_then(Value::as_str)
                .and_then(|date| crate::dates::unified_timestamp(date, true))
                .into(),
        );
        info.insert("thumbnails".into(), Value::Array(thumbnails));
        info.insert(
            "uploader".into(),
            data.pointer("/closeup_attribution/full_name")
                .cloned()
                .unwrap_or(Value::Null),
        );
        info.insert(
            "uploader_id".into(),
            match data.pointer("/closeup_attribution/id") {
                Some(Value::String(text)) => Value::from(text.clone()),
                Some(Value::Number(number)) => Value::from(number.to_string()),
                _ => Value::Null,
            },
        );
        info.insert("repost_count".into(), int(data.get("repin_count")).into());
        info.insert(
            "comment_count".into(),
            int(data.get("comment_count")).into(),
        );
        info.insert(
            "categories".into(),
            data.pointer("/pin_join/visual_annotation")
                .filter(|value| value.is_array())
                .cloned()
                .unwrap_or(Value::Null),
        );
        info.insert(
            "tags".into(),
            data.get("hashtags")
                .filter(|value| value.is_array())
                .cloned()
                .unwrap_or(Value::Null),
        );
        let domain = data
            .get("domain")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if domain.to_lowercase() != "uploaded by user"
            && let Some(embed) = data
                .pointer("/embed/src")
                .and_then(Value::as_str)
                .filter(|src| !src.is_empty())
        {
            // `url_transparent` to the embed with the pin's fields.
            let mut fields = info;
            if title.is_empty() {
                fields.insert("title".into(), Value::Null);
            }
            let mut follow = super::direct::build_at(page.clone(), page, String::new());
            follow.formats.clear();
            follow.direct = None;
            follow.extra.insert(
                crate::FOLLOW_TRANSPARENT.into(),
                json!({"url": embed, "fields": Value::Object(fields)}),
            );
            return Ok(Some(ExtractResult::Video(follow)));
        }
        let video_list = data
            .pointer("/videos/video_list")
            .filter(|value| value.is_object())
            .cloned()
            .or_else(|| {
                data.pointer("/story_pin_data/pages")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|page| page.get("blocks").and_then(Value::as_array))
                    .flatten()
                    .find_map(|block| {
                        block
                            .pointer("/video/video_list")
                            .filter(|value| value.is_object())
                            .cloned()
                    })
            })
            .unwrap_or_else(|| json!({}));
        let mut urls: Vec<String> = Vec::new();
        let mut formats = Vec::new();
        let mut duration: Option<f64> = None;
        for (format_id, entry) in video_list.as_object().into_iter().flatten() {
            let Some(format_url) = entry
                .get("url")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
            else {
                continue;
            };
            if urls.contains(&format_url) {
                continue;
            }
            urls.push(format_url.clone());
            duration = entry
                .get("duration")
                .and_then(|value| value.as_f64().or_else(|| value.as_str()?.parse().ok()))
                .map(|millis| millis / 1000.0);
            let ext = super::direct::determine_ext(&format_url);
            if format_id.to_lowercase().contains("hls") || ext.as_deref() == Some("m3u8") {
                if let Some(stream) = Url::parse(&format_url).ok().and_then(|manifest| {
                    super::extract_m3u8_optional(
                        client,
                        &manifest,
                        NAME,
                        &video_id,
                        Some(format_id),
                        false,
                    )
                }) {
                    formats.extend(stream.formats);
                }
            } else {
                let mut format = Map::new();
                format.insert("url".into(), format_url.clone().into());
                format.insert("format_id".into(), format_id.clone().into());
                format.insert("width".into(), int(entry.get("width")).into());
                format.insert("height".into(), int(entry.get("height")).into());
                format.insert(
                    "duration".into(),
                    duration.map_or(Value::Null, |d| json!(d)),
                );
                let keys: Vec<String> = format.keys().cloned().collect();
                for key in ["width", "height", "duration"] {
                    if format.get(key).is_some_and(Value::is_null) {
                        format.shift_remove(key);
                    }
                }
                format.insert(
                    "ext".into(),
                    ext.unwrap_or_else(|| "unknown_video".into()).into(),
                );
                let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(format))
                    .map_err(|error| Error::Reported(error.to_string()))?;
                crate::info_order::mark_format(&mut format, &format!("dict:{}", keys.join(",")));
                formats.push(format);
            }
        }
        let mut dict = Map::new();
        dict.insert("id".into(), video_id.clone().into());
        dict.insert("formats".into(), json!([]));
        dict.insert(
            "duration".into(),
            duration.map_or(Value::Null, |d| json!(d)),
        );
        dict.insert(
            "webpage_url".into(),
            format!("https://www.pinterest.com/pin/{video_id}/").into(),
        );
        dict.insert("extractor_key".into(), NAME.into());
        dict.insert("extractor".into(), NAME.into());
        dict.extend(info);
        let first_url = formats
            .first()
            .map_or_else(|| page.clone(), |format| format.url.clone());
        let mut result = super::direct::build_at(first_url.clone(), &first_url, String::new());
        result.direct = None;
        if formats.is_empty() {
            result
                .extra
                .insert(crate::info::NO_FORMATS.into(), Value::Bool(true));
        }
        result.formats = formats;
        result.webpage_url = Url::parse(&format!("https://www.pinterest.com/pin/{video_id}/"))?;
        result.extractor = NAME.into();
        result.extractor_key = NAME.into();
        if title.is_empty() {
            result
                .extra
                .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
        }
        let mut keys = Vec::new();
        for (key, value) in dict {
            keys.push(key.clone());
            match key.as_str() {
                "id" => result.id = value.as_str().unwrap_or_default().to_owned(),
                "title" => result.title = value.as_str().unwrap_or_default().to_owned(),
                "formats" | "webpage_url" | "extractor" | "extractor_key" => {}
                _ if value.is_null() => {
                    result.extra.insert(key, Value::Null);
                }
                _ => crate::metadata_parser::set_field(&mut result, &key, value),
            }
        }
        Ok(Some(ExtractResult::Video(crate::info_order::mark_info(
            result,
            &format!(
                "dict{};{};thumbnail=url,width,height",
                if title.is_empty() { "-untitled" } else { "" },
                keys.join(",")
            ),
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.pinterest.com/pin/664281013778109217/"
        ));
        assert!(suitable("https://co.pinterest.com/pin/824721750502199491/"));
        assert!(suitable(
            "https://www.pinterest.co.uk/pin/some-title--664281013778109217/"
        ));
        assert!(!suitable("https://www.pinterest.com/someone/board/"));
    }
}

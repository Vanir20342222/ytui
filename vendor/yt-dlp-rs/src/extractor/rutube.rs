//! Upstream `RutubeIE` and `RutubeEmbedIE` (`yt_dlp/extractor/rutube.py`):
//! the video API's metadata (with description chapters), the play options'
//! balancer HLS/progressive formats, live HLS streams and captions; numeric
//! embeds resolve their `effective_video` first. F4M balancer entries and
//! Rutube's playlist, channel, tag, person and movie pages are unsupported.
use super::Extractor;
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Rutube;
pub struct RutubeEmbed;

const NAME: &str = "rutube";
const KEY: &str = "Rutube";
const EMBED_NAME: &str = "rutube:embed";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://rutube\.ru/(?:(?:live/)?video(?:/private)?|(?:play/)?embed)/(?P<id>[\da-z]{32})")
        .unwrap()
});
static EMBED_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://rutube\.ru/(?:video|play)/embed/(?P<id>[0-9]+)(?:[?#/]|$)").unwrap()
});

/// `RutubeIE.suitable`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `RutubeEmbedIE.suitable`.
pub fn embed_suitable(url: &str) -> bool {
    EMBED_URL.is_match(url)
}

fn int(value: Option<&Value>) -> Option<i64> {
    let value = value?;
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|number| number as i64))
        .or_else(|| value.as_str()?.trim().parse().ok())
}

fn bug(extractor: &str, id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "[{extractor}] {id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `parse_qs(url)` for the API requests (`format=json` added).
fn api_query(page: &Url) -> Vec<(String, String)> {
    let mut query: Vec<(String, String)> = Vec::new();
    for (key, value) in page.query_pairs() {
        if value.is_empty() {
            continue;
        }
        query.push((key.into_owned(), value.into_owned()));
    }
    query.retain(|(key, _)| key != "format");
    query.push(("format".into(), "json".into()));
    query
}

fn api_json(
    client: &Client,
    extractor: &'static str,
    path: &str,
    id: &str,
    note: &str,
    what: &str,
    query: &[(String, String)],
) -> Result<Value> {
    use std::io::Read;
    let mut url = Url::parse(&format!("https://rutube.ru/api/{path}/{id}/"))?;
    url.query_pairs_mut().extend_pairs(query);
    crate::screen::screen(&format!("[{extractor}] {id}: {note}"));
    let mut response = super::direct::request(client, &url, None).map_err(|error| match error {
        Error::Http(_) | Error::HttpRetryAfter { .. } => {
            Error::Reported(super::request_error_text(extractor, id, what, &error))
        }
        error => error,
    })?;
    let mut bytes = Vec::new();
    response
        .read_to_end(&mut bytes)
        .map_err(Error::ExtractionRead)?;
    serde_json::from_slice(&bytes).map_err(|error| {
        Error::Reported(format!(
            "[{extractor}] {id}: Failed to parse JSON  (caused by {error})"
        ))
    })
}

/// `_extract_info(video, video_id)`: the metadata dict in upstream's order.
fn extract_info(extractor: &str, video: &Value, video_id: &str) -> Result<Map<String, Value>> {
    let Some(title) = video.get("title") else {
        return Err(bug(extractor, video_id, "title"));
    };
    let age_limit = video
        .get("is_adult")
        .filter(|adult| !adult.is_null())
        .map(|adult| if adult == &Value::Bool(true) { 18 } else { 0 });
    let description = video.get("description").cloned().unwrap_or(Value::Null);
    let duration = int(video.get("duration"));
    let uploader_id = video.pointer("/author/id").filter(|id| match id {
        Value::Null | Value::Bool(false) => false,
        Value::Number(number) => number.as_f64() != Some(0.0),
        Value::String(text) => !text.is_empty(),
        _ => true,
    });
    let category = video
        .pointer("/category/name")
        .filter(|name| name.as_str().is_some_and(|name| !name.is_empty()));
    let chapters = super::youtube::chapters_from_description(
        description.as_str(),
        duration.and_then(|duration| u64::try_from(duration).ok()),
    );
    let id = match video.get("id") {
        Some(Value::String(text)) if !text.is_empty() => Value::from(text.clone()),
        Some(value @ Value::Number(_)) => value.clone(),
        _ => Value::from(video_id),
    };
    let mut info = Map::new();
    info.insert("id".into(), id);
    info.insert("title".into(), title.clone());
    info.insert("description".into(), description);
    info.insert(
        "thumbnail".into(),
        video.get("thumbnail_url").cloned().unwrap_or(Value::Null),
    );
    info.insert("duration".into(), duration.into());
    info.insert(
        "uploader".into(),
        video
            .pointer("/author/name")
            .cloned()
            .unwrap_or(Value::Null),
    );
    info.insert(
        "uploader_id".into(),
        uploader_id.map_or(Value::Null, |id| match id {
            Value::String(text) => Value::from(text.clone()),
            other => Value::from(other.to_string()),
        }),
    );
    info.insert(
        "timestamp".into(),
        video
            .get("created_ts")
            .and_then(Value::as_str)
            .and_then(|date| crate::dates::unified_timestamp(date, true))
            .into(),
    );
    info.insert(
        "categories".into(),
        category.map_or(Value::Null, |category| json!([category])),
    );
    info.insert("age_limit".into(), age_limit.into());
    info.insert("view_count".into(), int(video.get("hits")).into());
    info.insert(
        "comment_count".into(),
        int(video.get("comments_count")).into(),
    );
    info.insert(
        "is_live".into(),
        video.get("is_livestream").and_then(Value::as_bool).into(),
    );
    info.insert(
        "chapters".into(),
        // `_extract_chapters_helper` is `None` without a duration, else a list.
        if chapters.is_empty() && !duration.is_some_and(|duration| duration != 0) {
            Value::Null
        } else {
            serde_json::to_value(&chapters).unwrap_or(Value::Null)
        },
    );
    Ok(info)
}

/// `_extract_formats_and_subtitles(options, video_id)`.
fn formats_and_subtitles(
    client: &Client,
    extractor: &'static str,
    options: &Value,
    video_id: &str,
) -> Result<(Vec<crate::VideoFormat>, crate::Subtitles)> {
    let Some(balancer) = options.get("video_balancer") else {
        return Err(bug(extractor, video_id, "video_balancer"));
    };
    let mut formats = Vec::new();
    let mut subtitles = crate::Subtitles::default();
    for (format_id, format_url) in balancer.as_object().into_iter().flatten() {
        let format_url = format_url.as_str().unwrap_or_default();
        match super::direct::determine_ext(format_url).as_deref() {
            Some("m3u8") => {
                if let Some(info) = Url::parse(format_url).ok().and_then(|manifest| {
                    super::extract_m3u8_optional(
                        client,
                        &manifest,
                        extractor,
                        video_id,
                        Some(format_id),
                        true,
                    )
                }) {
                    formats.extend(info.formats);
                    super::html5::merge_subtitles(&mut subtitles, info.subtitles);
                }
            }
            Some("f4m") => {
                return Err(Error::UnsupportedHtml5(
                    "Rutube F4M balancer formats are not supported natively".into(),
                ));
            }
            _ => {
                let mut map = Map::new();
                map.insert("url".into(), format_url.into());
                map.insert("format_id".into(), format_id.clone().into());
                map.insert(
                    "ext".into(),
                    super::direct::determine_ext(format_url)
                        .unwrap_or_else(|| "unknown_video".into())
                        .into(),
                );
                let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(map))
                    .map_err(|error| Error::Reported(error.to_string()))?;
                crate::info_order::mark_format(&mut format, "dict:url,format_id");
                formats.push(format);
            }
        }
    }
    for hls in options
        .pointer("/live_streams/hls")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(manifest) = hls
            .get("url")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
            .and_then(|url| Url::parse(&url).ok())
        else {
            continue;
        };
        if let Some(info) =
            super::extract_m3u8_optional(client, &manifest, extractor, video_id, Some("hls"), true)
        {
            formats.extend(info.formats);
            super::html5::merge_subtitles(&mut subtitles, info.subtitles);
        }
    }
    // `subs_list_to_dict(lang='ru')` of the captions.
    let mut captions = crate::Subtitles::default();
    for caption in options
        .get("captions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(url) = caption
            .get("file")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
            .and_then(|url| Url::parse(&url).ok())
        else {
            continue;
        };
        let language = caption
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or("ru")
            .to_owned();
        let mut track = Map::new();
        track.insert("url".into(), url.as_str().into());
        if let Some(name) = caption.get("langTitle").and_then(Value::as_str) {
            track.insert("name".into(), name.into());
        }
        track.insert(
            "ext".into(),
            super::direct::determine_ext(url.as_str())
                .unwrap_or_else(|| "unknown_video".into())
                .into(),
        );
        let track: crate::SubtitleFormat = serde_json::from_value(Value::Object(track))
            .map_err(|error| Error::Reported(error.to_string()))?;
        let mut single = crate::Subtitles::default();
        single.entries.push((Some(language), vec![track]));
        super::html5::merge_subtitles(&mut captions, single);
    }
    super::html5::merge_subtitles(&mut subtitles, captions);
    Ok((formats, subtitles))
}

#[allow(clippy::too_many_arguments)]
fn video(
    page: &Url,
    extractor: &str,
    key: &str,
    mut dict: Map<String, Value>,
    formats: Vec<crate::VideoFormat>,
    subtitles: crate::Subtitles,
    extra_keys: &[&str],
) -> VideoInfo {
    dict.insert("formats".into(), json!([]));
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
    info.extractor = extractor.into();
    info.extractor_key = key.into();
    let titled = dict.get("title").is_some_and(Value::is_string);
    if !titled {
        info.extra
            .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
    }
    let mut keys: Vec<String> = Vec::new();
    for (name, value) in dict {
        keys.push(name.clone());
        match name.as_str() {
            "id" => {
                info.id = match &value {
                    Value::String(text) => text.clone(),
                    other => other.to_string(),
                }
            }
            "title" => info.title = value.as_str().unwrap_or_default().to_owned(),
            "formats" | "subtitles" => {}
            _ if value.is_null() => {
                info.extra.insert(name, Value::Null);
            }
            _ => crate::metadata_parser::set_field(&mut info, &name, value),
        }
    }
    let mut keys: Vec<&str> = keys.iter().map(String::as_str).collect();
    keys.retain(|key| !extra_keys.contains(key));
    keys.extend(extra_keys);
    crate::info_order::mark_info(
        info,
        &format!(
            "dict;{};subtitle=url,name,ext;chapter=start_time,title",
            keys.join(",")
        ),
    )
}

fn start(extractor: &'static str, page: &Url) {
    crate::screen::set_extractor(extractor);
    crate::screen::screen(&format!(
        "[{extractor}] Extracting URL: {}",
        crate::screen::extracting_url(page.as_str())
    ));
}

impl Extractor for Rutube {
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
        let video_id = found["id"].to_owned();
        start(NAME, page);
        let query = api_query(page);
        let api = api_json(
            client,
            NAME,
            "video",
            &video_id,
            "Downloading video JSON",
            "video JSON",
            &query,
        )?;
        let dict = extract_info(NAME, &api, &video_id)?;
        let options = api_json(
            client,
            NAME,
            "play/options",
            &video_id,
            "Downloading options JSON",
            "options JSON",
            &query,
        )?;
        let (formats, subtitles) = formats_and_subtitles(client, NAME, &options, &video_id)?;
        Ok(Some(ExtractResult::Video(video(
            page,
            NAME,
            KEY,
            dict,
            formats,
            subtitles,
            &[],
        ))))
    }
}

impl Extractor for RutubeEmbed {
    fn key(&self) -> &'static str {
        EMBED_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, page: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = EMBED_URL.captures(page.as_str()) else {
            return Ok(None);
        };
        let embed_id = found["id"].to_owned();
        start(EMBED_NAME, page);
        let query = api_query(page);
        let options = api_json(
            client,
            EMBED_NAME,
            "play/options",
            &embed_id,
            "Downloading options JSON",
            "options JSON",
            &query,
        )?;
        let Some(video_id) = options.get("effective_video") else {
            return Err(bug(EMBED_NAME, &embed_id, "effective_video"));
        };
        let video_id = match video_id {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        };
        let (formats, subtitles) = formats_and_subtitles(client, EMBED_NAME, &options, &video_id)?;
        let api = api_json(
            client,
            EMBED_NAME,
            "video",
            &video_id,
            "Downloading video JSON",
            "video JSON",
            &query,
        )?;
        let mut dict = extract_info(EMBED_NAME, &api, &video_id)?;
        // `info.update({'extractor_key': 'Rutube', 'formats', 'subtitles'})`.
        dict.insert("extractor_key".into(), KEY.into());
        let mut info = video(
            page,
            EMBED_NAME,
            KEY,
            dict,
            formats,
            subtitles,
            &["extractor_key", "formats", "subtitles"],
        );
        info.extra.shift_remove("extractor_key");
        Ok(Some(ExtractResult::Video(info)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_urls() {
        assert!(suitable(
            "https://rutube.ru/video/3eac3b4561676c17df9132a9a1e62e3e/"
        ));
        assert!(suitable(
            "https://rutube.ru/play/embed/a10e53b86e8f349080f718582ce4c661"
        ));
        assert!(suitable(
            "https://rutube.ru/video/private/884fb55f07a97ab673c7d654553e0f48/?p=x"
        ));
        assert!(embed_suitable(
            "https://rutube.ru/video/embed/6722881?vk_puid37="
        ));
        assert!(embed_suitable("https://rutube.ru/play/embed/8083783"));
        assert!(!embed_suitable(
            "https://rutube.ru/play/embed/a10e53b86e8f349080f718582ce4c661"
        ));
    }
}

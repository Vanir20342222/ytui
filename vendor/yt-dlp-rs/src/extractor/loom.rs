//! Upstream `LoomIE` (`yt_dlp/extractor/loom.py`): the GraphQL video
//! metadata (password errors), source and chapters, the campaign session
//! `raw-url`/`transcoded-url` answers and the CDN URL as HLS (pre-merged,
//! split video marked silent), DASH or progressive formats with their
//! query as `extra_param_to_segment_url`, description chapters and, when
//! requested, the transcript subtitles. Folders are not supported.
use super::Extractor;
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Loom;

const NAME: &str = "loom";
const KEY: &str = "Loom";
const APOLLO_VERSION: &str = "45a5bd4";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?loom\.com/(?:share|embed)/(?P<id>[\da-f]{32})").unwrap()
});

/// `LoomIE._GRAPHQL_QUERIES` (verbatim).
const QUERIES: [(&str, &str); 4] = [
    (
        "GetVideoSSR",
        "query GetVideoSSR($videoId: ID!, $password: String) {\n  getVideo(id: $videoId, password: $password) {\n    __typename\n    ... on PrivateVideo {\n      id\n      status\n      message\n      __typename\n    }\n    ... on VideoPasswordMissingOrIncorrect {\n      id\n      message\n      __typename\n    }\n    ... on RegularUserVideo {\n      id\n      __typename\n      createdAt\n      description\n      download_enabled\n      folder_id\n      is_protected\n      needs_password\n      owner {\n        display_name\n        __typename\n      }\n      privacy\n      s3_id\n      name\n      video_properties {\n        avgBitRate\n        client\n        camera_enabled\n        client_version\n        duration\n        durationMs\n        format\n        height\n        microphone_enabled\n        os\n        os_version\n        recordingClient\n        recording_type\n        recording_version\n        screen_type\n        tab_audio\n        trim_duration\n        width\n        __typename\n      }\n      playable_duration\n      source_duration\n      visibility\n    }\n  }\n}\n",
    ),
    (
        "GetVideoSource",
        "query GetVideoSource($videoId: ID!, $password: String, $acceptableMimes: [CloudfrontVideoAcceptableMime]) {\n  getVideo(id: $videoId, password: $password) {\n    ... on RegularUserVideo {\n      id\n      nullableRawCdnUrl(acceptableMimes: $acceptableMimes, password: $password) {\n        url\n        credentials {\n          Policy\n          Signature\n          KeyPairId\n          __typename\n        }\n        __typename\n      }\n      __typename\n    }\n    __typename\n  }\n}\n",
    ),
    (
        "FetchVideoTranscript",
        "query FetchVideoTranscript($videoId: ID!, $password: String) {\n  fetchVideoTranscript(videoId: $videoId, password: $password) {\n    ... on VideoTranscriptDetails {\n      id\n      video_id\n      source_url\n      captions_source_url\n      __typename\n    }\n    ... on GenericError {\n      message\n      __typename\n    }\n    __typename\n  }\n}\n",
    ),
    (
        "FetchChapters",
        "query FetchChapters($videoId: ID!, $password: String) {\n  fetchVideoChapters(videoId: $videoId, password: $password) {\n    ... on VideoChapters {\n      video_id\n      content\n      __typename\n    }\n    ... on EmptyChaptersPayload {\n      content\n      __typename\n    }\n    ... on InvalidRequestWarning {\n      message\n      __typename\n    }\n    ... on Error {\n      message\n      __typename\n    }\n    __typename\n  }\n}\n",
    ),
];

/// `LoomIE.suitable`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn fail(id: &str, message: &str) -> Error {
    Error::Reported(format!("[{NAME}] {id}: {message}"))
}

/// A random `uuid.uuid4()`.
fn uuid4() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut bytes = [0u8; 16];
    for (index, chunk) in bytes.chunks_mut(8).enumerate() {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_usize(index);
        hasher.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |time| time.as_nanos()),
        );
        chunk.copy_from_slice(&hasher.finish().to_le_bytes());
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

/// A POST of compact JSON (`_download_json(data=...)`); `Ok(None)` after a
/// non-fatal failure's warning.
#[allow(clippy::too_many_arguments)]
fn post(
    client: &Client,
    url: &Url,
    id: &str,
    note: &str,
    errnote: &str,
    headers: &[(&str, &str)],
    body: &Value,
    fatal: bool,
) -> Result<Option<Value>> {
    use std::io::Read;
    crate::screen::screen(&format!("[{NAME}] {id}: {note}"));
    let failure = |message: String| -> Result<Option<Value>> {
        if fatal {
            return Err(fail(id, &message));
        }
        crate::screen::warning(&format!("[{NAME}] {id}: {message}"));
        Ok(None)
    };
    let response = super::direct::post(
        client,
        url,
        headers,
        serde_json::to_string(body).unwrap_or_default(),
    );
    let mut response = match response {
        Ok(response) => response,
        Err(error @ (Error::Http(_) | Error::HttpRetryAfter { .. })) => {
            let text = super::request_error_text(NAME, id, "X", &error);
            // `report_warning(errmsg)` / `ExtractorError(errmsg)` without the cause.
            let detail = text
                .split_once(": Unable to download X: ")
                .map_or(text.as_str(), |(_, rest)| rest);
            let detail = detail
                .rsplit_once(" (caused by ")
                .map_or(detail, |(head, _)| head);
            return failure(format!("{errnote}: {detail}"));
        }
        Err(error) => return Err(error),
    };
    let mut bytes = Vec::new();
    response
        .read_to_end(&mut bytes)
        .map_err(Error::ExtractionRead)?;
    match serde_json::from_slice(&bytes) {
        Ok(value) => Ok(Some(value)),
        Err(error) => failure(format!("Failed to parse JSON  (caused by {error})")),
    }
}

/// `_call_graphql_api(operation_name, video_id, note, fatal=...)`.
fn graphql(
    client: &Client,
    operation: &str,
    id: &str,
    note: &str,
    fatal: bool,
) -> Result<Option<Value>> {
    let query = QUERIES
        .iter()
        .find(|(name, _)| *name == operation)
        .map_or("", |(_, query)| query);
    let mut variables = Map::new();
    variables.insert("videoId".into(), id.into());
    variables.insert(
        "password".into(),
        super::video_password().map_or(Value::Null, Value::from),
    );
    if operation == "GetVideoSource" {
        variables.insert(
            "acceptableMimes".into(),
            json!(["DASH", "M3U8", "MP4", "WEBM"]),
        );
    }
    let body = json!({
        "operationName": operation,
        "variables": Value::Object(variables),
        "query": query,
    });
    let source = format!("loom_web_{APOLLO_VERSION}");
    post(
        client,
        &Url::parse("https://www.loom.com/graphql")?,
        id,
        note,
        "Failed to download GraphQL JSON",
        &[
            ("Accept", "application/json"),
            ("Content-Type", "application/json"),
            ("x-loom-request-source", &source),
            ("apollographql-client-name", "web"),
            ("apollographql-client-version", APOLLO_VERSION),
            ("graphql-operation-name", operation),
            ("Origin", "https://www.loom.com"),
        ],
        &body,
        fatal,
    )
}

/// `_call_url_api(endpoint, video_id)`: the session's `url`.
fn url_api(client: &Client, endpoint: &str, id: &str) -> Result<Option<String>> {
    let url = Url::parse(&format!(
        "https://www.loom.com/api/campaigns/sessions/{id}/{endpoint}"
    ))?;
    let body = json!({
        "anonID": uuid4(),
        "deviceID": null,
        "force_original": false,
        "password": super::video_password(),
    });
    Ok(post(
        client,
        &url,
        id,
        &format!("Downloading {endpoint} JSON"),
        &format!("Failed to download {endpoint} JSON"),
        &[
            ("Accept", "application/json"),
            ("Content-Type", "application/json"),
        ],
        &body,
        false,
    )?
    .and_then(|response| {
        response
            .get("url")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
    }))
}

/// `update_url(url, query=query)`: the query replaced.
fn with_query(url: &str, query: &str) -> String {
    let base = url.split_once('?').map_or(url, |(base, _)| base);
    if query.is_empty() {
        base.to_owned()
    } else {
        format!("{base}?{query}")
    }
}

/// `get_formats(format_url, format_id, quality)`.
fn get_formats(
    client: &Client,
    id: &str,
    format_url: Option<&str>,
    format_id: &str,
    quality: i64,
    properties: &Map<String, Value>,
) -> Result<Vec<crate::VideoFormat>> {
    let Some(format_url) = format_url.filter(|url| !url.is_empty()) else {
        return Ok(Vec::new());
    };
    let ext = super::direct::determine_ext(format_url);
    let query = format_url
        .split_once('?')
        .map_or("", |(_, query)| query.split('#').next().unwrap_or_default())
        .to_owned();
    let mut formats = Vec::new();
    match ext.as_deref() {
        Some("m3u8") => {
            let merged = format_url.replace("-split.m3u8", ".m3u8");
            let Some(stream) = Url::parse(&merged).ok().and_then(|manifest| {
                super::extract_m3u8_optional(
                    client,
                    &manifest,
                    NAME,
                    id,
                    Some(&format!("hls-{format_id}")),
                    false,
                )
            }) else {
                return Ok(formats);
            };
            let split = stream
                .formats
                .iter()
                .any(|format| format.vcodec.as_deref() == Some("none"));
            for mut format in stream.formats {
                format.quality = Some(quality as f64);
                crate::info_order::mark_int_quality(&mut format);
                if split && format.vcodec.as_deref() != Some("none") {
                    format.acodec = Some("none".into());
                }
                format.url = Url::parse(&with_query(format.url.as_str(), &query))?;
                format
                    .extra
                    .insert("extra_param_to_segment_url".into(), query.clone().into());
                crate::info_order::mark_format_update(&mut format, "extra_param_to_segment_url");
                formats.push(format);
            }
        }
        Some("mpd") => {
            let Some(stream) = Url::parse(format_url).ok().and_then(|manifest| {
                super::extract_mpd_optional_note(
                    client,
                    &manifest,
                    NAME,
                    id,
                    Some(&format!("dash-{format_id}")),
                    "Downloading MPD manifest",
                )
            }) else {
                return Ok(formats);
            };
            for mut format in stream.formats {
                format
                    .extra
                    .insert("extra_param_to_segment_url".into(), query.clone().into());
                crate::info_order::mark_format_update(&mut format, "extra_param_to_segment_url");
                format.quality = Some(quality as f64);
                crate::info_order::mark_format_update(&mut format, "quality");
                crate::info_order::mark_int_quality(&mut format);
                formats.push(format);
            }
        }
        _ => {
            let mut map = Map::new();
            map.insert("url".into(), format_url.into());
            map.insert("ext".into(), ext.clone().map_or(Value::Null, Value::from));
            map.insert("format_id".into(), format!("http-{format_id}").into());
            map.insert("quality".into(), quality.into());
            map.extend(properties.clone());
            let keys: Vec<String> = map.keys().cloned().collect();
            map.retain(|_, value| !value.is_null());
            if !map.contains_key("ext") {
                map.insert("ext".into(), "unknown_video".into());
            }
            let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(map))
                .map_err(|error| fail(id, &error.to_string()))?;
            crate::info_order::mark_format(&mut format, &format!("dict:{}", keys.join(",")));
            crate::info_order::mark_int_quality(&mut format);
            formats.push(format);
        }
    }
    Ok(formats)
}

impl Extractor for Loom {
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
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(page.as_str())
        ));
        let metadata = graphql(
            client,
            "GetVideoSSR",
            &video_id,
            "Downloading GraphQL metadata JSON",
            false,
        )?
        .and_then(|data| {
            data.pointer("/data/getVideo")
                .filter(|v| v.is_object())
                .cloned()
        })
        .unwrap_or_else(|| json!({}));
        if metadata.get("__typename").and_then(Value::as_str)
            == Some("VideoPasswordMissingOrIncorrect")
        {
            return Err(fail(
                &video_id,
                if super::video_password().is_none() {
                    "This video is password-protected, use the --video-password option"
                } else {
                    "Invalid video password"
                },
            ));
        }
        let video_data = graphql(
            client,
            "GetVideoSource",
            &video_id,
            "Downloading GraphQL video JSON",
            true,
        )?
        .unwrap_or(Value::Null);
        let chapter_data = graphql(
            client,
            "FetchChapters",
            &video_id,
            "Downloading GraphQL chapters JSON",
            false,
        )?;
        let duration = metadata
            .pointer("/video_properties/duration")
            .and_then(|value| {
                value
                    .as_i64()
                    .or_else(|| value.as_f64().map(|number| number as i64))
                    .or_else(|| value.as_str()?.trim().parse().ok())
            });
        let chapters = super::youtube::chapters_from_description(
            chapter_data
                .as_ref()
                .and_then(|data| data.pointer("/data/fetchVideoChapters/content"))
                .and_then(Value::as_str),
            duration.and_then(|duration| u64::try_from(duration).ok()),
        );

        // `_extract_formats(video_id, metadata, video_data)`.
        let mut properties = Map::new();
        let property = |key: &str| metadata.pointer(&format!("/video_properties/{key}"));
        for key in ["width", "height"] {
            if let Some(number) = property(key).and_then(|value| {
                value
                    .as_i64()
                    .or_else(|| value.as_str()?.trim().parse().ok())
            }) {
                properties.insert(key.into(), number.into());
            }
        }
        if property("microphone_enabled") == Some(&Value::Bool(false)) {
            properties.insert("acodec".into(), "none".into());
        }
        let mut formats = Vec::new();
        let raw = url_api(client, "raw-url", &video_id)?;
        formats.extend(get_formats(
            client,
            &video_id,
            raw.as_deref(),
            "raw",
            1,
            &properties,
        )?);
        let transcoded = url_api(client, "transcoded-url", &video_id)?;
        formats.extend(get_formats(
            client,
            &video_id,
            transcoded.as_deref(),
            "transcoded",
            -1,
            &properties,
        )?);
        let strip = |url: &str| url.split_once('?').map_or(url, |(base, _)| base).to_owned();
        let valid: Vec<String> = [&raw, &transcoded]
            .into_iter()
            .flatten()
            .map(|url| strip(url))
            .collect();
        if let Some(cdn) = video_data
            .pointer("/data/getVideo/nullableRawCdnUrl/url")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
            && !valid.contains(&strip(&cdn))
        {
            formats.extend(get_formats(
                client,
                &video_id,
                Some(&cdn),
                "cdn",
                0,
                &properties,
            )?);
        }

        let mut subtitles = crate::Subtitles::default();
        if super::subtitles_requested()
            && let Some(data) = graphql(
                client,
                "FetchVideoTranscript",
                &video_id,
                "Downloading GraphQL subtitles JSON",
                false,
            )?
        {
            let tracks: Vec<crate::SubtitleFormat> = ["source_url", "captions_source_url"]
                .iter()
                .filter_map(|key| {
                    data.pointer(&format!("/data/fetchVideoTranscript/{key}"))
                        .and_then(Value::as_str)
                        .and_then(super::json_ld::url_or_none)
                })
                .filter_map(|url| {
                    let ext = super::direct::determine_ext(&url)?;
                    serde_json::from_value(json!({"url": url, "ext": ext})).ok()
                })
                .collect();
            if !tracks.is_empty() {
                subtitles.entries.push((Some("en".into()), tracks));
            }
        }

        let mut dict = Map::new();
        dict.insert("id".into(), video_id.clone().into());
        dict.insert("duration".into(), duration.into());
        dict.insert(
            "chapters".into(),
            if chapters.is_empty() {
                Value::Null
            } else {
                serde_json::to_value(&chapters).unwrap_or(Value::Null)
            },
        );
        dict.insert("formats".into(), json!([]));
        dict.insert("subtitles".into(), json!({}));
        let text = |path: &str| {
            metadata
                .pointer(path)
                .filter(|value| value.is_string())
                .cloned()
        };
        if let Some(title) = text("/name") {
            dict.insert("title".into(), title);
        }
        if let Some(description) = text("/description") {
            dict.insert("description".into(), description);
        }
        if let Some(uploader) = text("/owner/display_name") {
            dict.insert("uploader".into(), uploader);
        }
        if let Some(timestamp) = metadata
            .get("createdAt")
            .and_then(Value::as_str)
            .and_then(crate::dates::parse_iso8601)
        {
            dict.insert("timestamp".into(), timestamp.into());
        }

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
        info.extractor_key = KEY.into();
        let titled = dict.contains_key("title");
        if !titled {
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
            &format!(
                "dict{};{};subtitle=url,ext;chapter=start_time,title",
                if titled { "" } else { "-untitled" },
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
            "https://www.loom.com/share/43d05f362f734614a2e81b4694a3a523"
        ));
        assert!(suitable(
            "https://www.loom.com/embed/ddcf1c1ad21f451ea7468b1e33917e4e"
        ));
        assert!(!suitable(
            "https://www.loom.com/share/folder/997db4db046f43e5912f10dc5f817b5c"
        ));
    }

    #[test]
    fn uuids_have_the_v4_shape() {
        let id = uuid4();
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4");
    }
}

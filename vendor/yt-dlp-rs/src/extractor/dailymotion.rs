//! Upstream `DailymotionIE` (`yt_dlp/extractor/dailymotion.py`): a client
//! token, the GraphQL media object, the player metadata's qualities (HLS
//! with randomized headers, progressive H264), subtitles and metadata.
use super::Extractor;
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Dailymotion;

const NAME: &str = "dailymotion";
const KEY: &str = "Dailymotion";
const COMMON_MEDIA_FIELDS: &str =
    "description\n      geoblockedCountries {\n        allowed\n      }\n      xid";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^(?:https?:)?//(?:dai\.ly/|(?:(?:(?:www|touch|geo)\.)?dailymotion\.[a-z]{2,3}|(?:www\.)?lequipe\.fr)/(?:swf/|(?:(?:crawler|embed|swf)/)?video/|player(?:/[\da-z]+)?\.html\?(?:video|(?P<is_playlist>playlist))=))(?P<id>[^/?_&#]+)(?:[\w-]*\?playlist=(?P<playlist_id>x[0-9a-z]+))?",
    )
    .unwrap()
});
static H264: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"/H264-(\d+)x(\d+)(?:-(60)/)?").unwrap());

/// `_match_valid_url` (`swf/(?!video)` checked apart).
fn match_url(url: &str) -> Option<(String, bool, Option<String>)> {
    let found = VALID_URL.captures(url)?;
    let id = found["id"].to_owned();
    if url.contains("/swf/") && !url.contains("/swf/video/") && id.eq_ignore_ascii_case("video") {
        return None;
    }
    Some((
        id,
        found.name("is_playlist").is_some(),
        found.name("playlist_id").map(|id| id.as_str().to_owned()),
    ))
}

/// `DailymotionIE.suitable`.
pub fn suitable(url: &str) -> bool {
    match_url(url).is_some()
}

fn int(value: Option<&Value>) -> Option<i64> {
    let value = value?;
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|number| number as i64))
        .or_else(|| value.as_str()?.trim().parse().ok())
}

fn fail(id: &str, message: &str) -> Error {
    Error::Reported(format!("[{NAME}] {id}: {message}"))
}

/// `_generate_blockbuster_headers()`: 2–8 random consonant headers.
fn blockbuster_headers() -> Vec<(String, String)> {
    use std::hash::{BuildHasher, Hasher};
    const LETTERS: &[u8] = b"bcdfghjklmnpqrstvwxz";
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |time| time.as_nanos()),
    );
    let mut state = hasher.finish() | 1;
    let mut next = |bound: u64| {
        // xorshift64.
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state % bound
    };
    let mut word = |minimum: u64, maximum: u64| -> String {
        let length = minimum + next(maximum - minimum + 1);
        (0..length)
            .map(|_| LETTERS[next(LETTERS.len() as u64) as usize] as char)
            .collect()
    };
    let count = 2 + (std::process::id() as u64 + 3) % 7;
    (0..count).map(|_| (word(8, 24), word(16, 32))).collect()
}

fn post_json(
    client: &Client,
    url: &Url,
    id: &str,
    note: &str,
    headers: &[(&str, &str)],
    body: String,
) -> Result<Value> {
    use std::io::Read;
    crate::screen::screen(&if id.is_empty() {
        format!("[{NAME}] {note}")
    } else {
        format!("[{NAME}] {id}: {note}")
    });
    let mut response =
        super::direct::post(client, url, headers, body).map_err(|error| match error {
            Error::Http(_) | Error::HttpRetryAfter { .. } => Error::ExtractorRequest {
                extractor: NAME,
                id: id.to_owned(),
                what: "JSON metadata",
                source: Box::new(error),
            },
            error => error,
        })?;
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    serde_json::from_slice(&body).map_err(|error| {
        Error::Reported(format!(
            "[{NAME}] {id}: Failed to parse JSON  (caused by {error})"
        ))
    })
}

/// `_get_token(xid)`: a cookie's token, else a client-credentials token.
fn token(client: &Client) -> Result<String> {
    for name in ["access_token", "client_token"] {
        if let Some(token) = crate::cookies::get_cookie("https://www.dailymotion.com/", name) {
            return Ok(token);
        }
    }
    let url = Url::parse("https://graphql.api.dailymotion.com/oauth/token")?;
    let body = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("client_id", "f1a362d288c1b98099c7")
        .append_pair("client_secret", "eea605b96e01c796ff369935357eca920c5da4c5")
        .append_pair("grant_type", "client_credentials")
        .finish();
    let response = post_json(
        client,
        &url,
        "",
        "Downloading Access Token",
        &[("Content-Type", "application/x-www-form-urlencoded")],
        body,
    )?;
    let token = response
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            Error::Reported(format!(
                "[{NAME}] An extractor error has occurred. (caused by KeyError('access_token')); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?
        .to_owned();
    crate::cookies::set_cookie("www.dailymotion.com", "client_token", &token);
    Ok(token)
}

/// `_call_api('media', xid, fields, note)`.
fn media(client: &Client, xid: &str) -> Result<Value> {
    let token = token(client)?;
    let fields = format!(
        "... on Video {{\n      {COMMON_MEDIA_FIELDS}\n      stats {{\n        likes {{\n          total\n        }}\n        views {{\n          total\n        }}\n      }}\n    }}\n    ... on Live {{\n      {COMMON_MEDIA_FIELDS}\n      audienceCount\n      isOnAir\n    }}"
    );
    let query = format!("{{\n  media(xid: \"{xid}\") {{\n    {fields}\n  }}\n}}");
    let body = crate::output::python_json_dumps(&json!({ "query": query }));
    let authorization = format!("Bearer {token}");
    let response = post_json(
        client,
        &Url::parse("https://graphql.api.dailymotion.com/")?,
        xid,
        "Downloading media JSON metadata",
        &[
            ("Content-Type", "application/json"),
            ("Origin", "https://www.dailymotion.com"),
            ("Authorization", &authorization),
        ],
        body,
    )?;
    match response.pointer("/data/media") {
        Some(media) if !media.is_null() => Ok(media.clone()),
        _ => Err(fail(
            xid,
            response
                .pointer("/errors/0/message")
                .and_then(Value::as_str)
                .unwrap_or("Unknown error"),
        )),
    }
}

/// `_extract_dailymotion_m3u8_formats_and_subtitles`: a request with
/// randomized headers (the impersonated retries are unavailable here).
fn m3u8_formats(
    client: &Client,
    url: &str,
    id: &str,
) -> (Vec<crate::VideoFormat>, crate::Subtitles, Option<String>) {
    let Ok(manifest) = Url::parse(url) else {
        return (Vec::new(), Default::default(), None);
    };
    crate::screen::screen(&format!(
        "[{NAME}] {id}: Downloading m3u8 information with randomized headers"
    ));
    let headers = blockbuster_headers();
    let headers: Vec<(&str, &str)> = headers
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    let parsed = super::direct::request_with_headers(client, &manifest, None, &headers).and_then(
        |response| super::hls::from_response(client, response, &manifest, None, true, Some("hls")),
    );
    match parsed {
        Ok(Some(stream)) => (stream.formats, stream.subtitles, None),
        Ok(None) => (Vec::new(), Default::default(), None),
        Err(error) => {
            crate::screen::debug(&format!(
                "{id}: Unable to download m3u8 information: {}",
                crate::upstream_error_text(&error)
            ));
            // Both impersonated retries fail without an impersonate target.
            for target in ["chrome", "firefox"] {
                let note = if target == "chrome" {
                    "Retrying m3u8 download with Chrome impersonation"
                } else {
                    "Retrying m3u8 download with Firefox impersonation"
                };
                crate::screen::screen(&format!("[{NAME}] {id}: {note}"));
                crate::screen::debug(&format!(
                    "{id}: The extractor is attempting impersonation, but none of these impersonate targets are available: {target}. See  https://github.com/yt-dlp/yt-dlp#impersonation  for information on installing the required dependencies"
                ));
            }
            (
                Vec::new(),
                Default::default(),
                Some("The extractor is attempting impersonation, but none of these impersonate targets are available: firefox. See  https://github.com/yt-dlp/yt-dlp#impersonation  for information on installing the required dependencies".into()),
            )
        }
    }
}

impl Extractor for Dailymotion {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some((mut video_id, is_playlist, mut playlist_id)) = match_url(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(NAME);
        // `_real_initialize`: no `ff` cookie and no age limit turn the family
        // filter off.
        let family = crate::cookies::get_cookie("https://www.dailymotion.com/", "ff");
        let family = family.as_deref() == Some("on");
        crate::cookies::set_cookie(
            "www.dailymotion.com",
            "ff",
            if family { "on" } else { "off" },
        );
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        if is_playlist {
            playlist_id = Some(video_id.clone());
            video_id = String::new();
        }
        // `_yes_playlist(playlist_id, video_id)`.
        if let Some(playlist_id) = &playlist_id {
            let yes = if video_id.is_empty() {
                true
            } else if super::no_playlist() {
                crate::screen::screen(&format!(
                    "[{NAME}] Downloading just the video {video_id} because of --no-playlist"
                ));
                false
            } else {
                crate::screen::screen(&format!(
                    "[{NAME}] Downloading playlist {playlist_id} - add --no-playlist to download just the video {video_id}"
                ));
                true
            };
            if yes {
                return Err(Error::UnsupportedHtml5(
                    "Dailymotion playlists are not supported natively".into(),
                ));
            }
        }
        let media = media(client, &video_id)?;
        let xid = media
            .get("xid")
            .and_then(Value::as_str)
            .unwrap_or(&video_id)
            .to_owned();
        let mut metadata_url = Url::parse(&format!(
            "https://www.dailymotion.com/player/metadata/video/{xid}"
        ))?;
        metadata_url
            .query_pairs_mut()
            .append_pair("app", "com.dailymotion.neon");
        let metadata = super::download_json_note(
            client,
            &metadata_url,
            NAME,
            &xid,
            "Downloading metadata JSON",
        )?;
        if let Some(error) = metadata.get("error").filter(|error| !error.is_null()) {
            let title = error
                .get("title")
                .and_then(Value::as_str)
                .filter(|title| !title.is_empty())
                .or_else(|| error.get("raw_message").and_then(Value::as_str))
                .unwrap_or_default();
            if error.get("code").and_then(Value::as_str) == Some("DM007") {
                // `raise_geo_restricted(msg=title)`.
                return Err(fail(&xid, title));
            }
            return Err(fail(&xid, &format!("{NAME} said: {title}")));
        }
        let title = metadata
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let is_live = media.get("isOnAir").cloned().unwrap_or(Value::Null);
        let mut formats = Vec::new();
        let mut subtitles = crate::Subtitles::default();
        let mut expected_error = None;
        for (quality, media_list) in metadata
            .get("qualities")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            for item in media_list.as_array().into_iter().flatten() {
                let Some(media_url) = item
                    .get("url")
                    .and_then(Value::as_str)
                    .filter(|u| !u.is_empty())
                else {
                    continue;
                };
                let media_type = item.get("type").and_then(Value::as_str);
                if media_type == Some("application/vnd.lumberjack.manifest") {
                    continue;
                }
                if media_type == Some("application/x-mpegURL") {
                    let (found, subs, error) = m3u8_formats(client, media_url, &video_id);
                    formats.extend(found);
                    super::html5::merge_subtitles(&mut subtitles, subs);
                    expected_error = error;
                } else {
                    let mut format = Map::new();
                    format.insert(
                        "url".into(),
                        media_url.split('#').next().unwrap_or(media_url).into(),
                    );
                    format.insert("format_id".into(), format!("http-{quality}").into());
                    let mut keys = vec!["url", "format_id"];
                    if let Some(found) = H264.captures(media_url) {
                        let fps = found
                            .get(3)
                            .and_then(|fps| fps.as_str().parse::<i64>().ok());
                        format.insert("fps".into(), fps.into());
                        format.insert("height".into(), found[2].parse::<i64>().ok().into());
                        format.insert("width".into(), found[1].parse::<i64>().ok().into());
                        keys.extend(["fps", "height", "width"]);
                    }
                    if format.get("fps").is_none_or(Value::is_null) && quality.ends_with("@60") {
                        format.insert("fps".into(), 60.into());
                        if !keys.contains(&"fps") {
                            keys.push("fps");
                        }
                    }
                    if format.get("fps").is_some_and(Value::is_null) {
                        format.shift_remove("fps");
                    }
                    // Processing's `determine_ext` sees the URL without its fragment.
                    format.insert(
                        "ext".into(),
                        super::direct::determine_ext(
                            media_url.split('#').next().unwrap_or(media_url),
                        )
                        .unwrap_or_else(|| "unknown_video".into())
                        .into(),
                    );
                    let mut format: crate::VideoFormat =
                        serde_json::from_value(Value::Object(format))
                            .map_err(|error| fail(&video_id, &error.to_string()))?;
                    crate::info_order::mark_format(
                        &mut format,
                        &format!("dict:{}", keys.join(",")),
                    );
                    formats.push(format);
                }
            }
        }
        if formats.is_empty()
            && let Some(error) = expected_error
        {
            return Err(fail(&video_id, &error));
        }
        for format in &mut formats {
            if let Some((url, _)) = format.url.as_str().split_once('#')
                && let Ok(url) = Url::parse(url)
            {
                format.url = url;
            }
            if format.fps.is_none() && format.format_id.ends_with("@60") {
                format.fps = Some(crate::info::PyNumber::Int(60));
                crate::info_order::mark_format_update(format, "fps");
            }
        }
        for (language, subtitle) in metadata
            .pointer("/subtitles/data")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            let tracks: Vec<crate::SubtitleFormat> = subtitle
                .get("urls")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter_map(|url| {
                    let ext =
                        super::direct::determine_ext(url).unwrap_or_else(|| "unknown_video".into());
                    serde_json::from_value(json!({"url": url, "ext": ext})).ok()
                })
                .collect();
            let language = Some(language.clone());
            match subtitles
                .entries
                .iter_mut()
                .find(|(key, _)| *key == language)
            {
                Some((_, existing)) => *existing = tracks,
                None => subtitles.entries.push((language, tracks)),
            }
        }
        let mut thumbnails = Vec::new();
        for key in ["posters", "thumbnails"] {
            for (size, url) in metadata
                .get(key)
                .and_then(Value::as_object)
                .into_iter()
                .flatten()
            {
                if url.as_str().and_then(super::json_ld::url_or_none).is_none() {
                    continue;
                }
                thumbnails.push(json!({
                    "height": size.parse::<i64>().ok(),
                    "id": size,
                    "url": url,
                }));
            }
        }
        let owner = metadata.get("owner").cloned().unwrap_or_else(|| json!({}));
        let count = |kind: &str| int(media.pointer(&format!("/stats/{kind}s/total")));
        let mut dict = Map::new();
        dict.insert("id".into(), video_id.clone().into());
        dict.insert("title".into(), title.clone().into());
        dict.insert(
            "description".into(),
            media
                .get("description")
                .and_then(Value::as_str)
                .map(crate::html::clean_html)
                .into(),
        );
        dict.insert("thumbnails".into(), Value::Array(thumbnails));
        dict.insert(
            "duration".into(),
            int(metadata.get("duration")).filter(|d| *d != 0).into(),
        );
        dict.insert("timestamp".into(), int(metadata.get("created_time")).into());
        dict.insert(
            "uploader".into(),
            owner.get("screenname").cloned().unwrap_or(Value::Null),
        );
        dict.insert(
            "uploader_id".into(),
            owner
                .get("id")
                .filter(|id| !id.is_null() && *id != "")
                .or_else(|| metadata.get("screenname"))
                .cloned()
                .unwrap_or(Value::Null),
        );
        dict.insert(
            "age_limit".into(),
            if metadata.get("explicit").and_then(Value::as_bool) == Some(true) {
                18
            } else {
                0
            }
            .into(),
        );
        dict.insert(
            "tags".into(),
            metadata.get("tags").cloned().unwrap_or(Value::Null),
        );
        dict.insert(
            "view_count".into(),
            count("view")
                .filter(|views| *views != 0)
                .or_else(|| int(media.get("audienceCount")))
                .into(),
        );
        dict.insert("like_count".into(), count("like").into());
        dict.insert("formats".into(), json!([]));
        dict.insert("subtitles".into(), json!({}));
        dict.insert("is_live".into(), is_live);
        Ok(Some(ExtractResult::Video(video(
            url, dict, formats, subtitles,
        ))))
    }
}

/// A video from upstream's dict (`dict;KEYS` order).
fn video(
    url: &Url,
    dict: Map<String, Value>,
    formats: Vec<crate::VideoFormat>,
    subtitles: crate::Subtitles,
) -> VideoInfo {
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
    info.extractor = NAME.into();
    info.extractor_key = KEY.into();
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
    crate::info_order::mark_info(
        info,
        &format!(
            "dict;{};subtitle=url;thumbnail=height,id,url",
            keys.join(",")
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert_eq!(
            match_url("https://www.dailymotion.com/video/x5kesuj")
                .map(|(id, ..)| id)
                .as_deref(),
            Some("x5kesuj")
        );
        assert!(suitable("https://dai.ly/x5kesuj"));
        assert!(suitable("https://www.dailymotion.com/embed/video/x7tgad0"));
        assert!(suitable(
            "https://geo.dailymotion.com/player/xf7zn.html?video=x8u4owg"
        ));
        assert_eq!(
            match_url("https://www.dailymotion.com/video/x73czzz?playlist=x6hynp"),
            Some(("x73czzz".into(), false, Some("x6hynp".into())))
        );
        assert!(!suitable("https://www.dailymotion.com/user/someone"));
    }
}

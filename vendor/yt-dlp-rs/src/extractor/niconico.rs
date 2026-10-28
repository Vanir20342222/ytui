//! Upstream `NiconicoIE` (`yt_dlp/extractor/niconico.py`): the watch API
//! (`v3_guest`, or `v3` with a `user_session` cookie) with upstream's error
//! and geo-restriction messages, the domand `access-rights/hls` request and
//! its HLS audio/video formats (audio fields from the API, video IDs from
//! the URLs and rates net of the lowest audio bitrate), thumbnails and
//! metadata. The comment "subtitles" (inline JSON data) are unsupported.
use super::Extractor;
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Niconico;

const NAME: &str = "niconico";
const KEY: &str = "Niconico";
const API_BASE: &str = "https://nvapi.nicovideo.jp";
const BASE_URL: &str = "https://www.nicovideo.jp";
const HEADERS: [(&str, &str); 2] = [("X-Frontend-ID", "6"), ("X-Frontend-Version", "0")];
const LOGIN_HINT: &str = "Use --cookies, --cookies-from-browser, --username and --password, --netrc-cmd, or --netrc (niconico) to provide account credentials. See  https://github.com/yt-dlp/yt-dlp/wiki/FAQ#how-do-i-pass-cookies-to-yt-dlp  for how to manually pass cookies";
const THUMB_PREFS: [&str; 6] = ["url", "middleUrl", "largeUrl", "player", "ogp", "short"];

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:(?:embed|sp|www)\.)?nicovideo\.jp/(?:shorts|watch)/(?P<id>(?:[a-z]{2})?\d+)",
    )
    .unwrap()
});

/// `NiconicoIE.suitable`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
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

fn int(value: Option<&Value>) -> Option<i64> {
    let value = value?;
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|number| number as i64))
        .or_else(|| value.as_str()?.trim().parse().ok())
}

fn str_or_none(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

/// `_ERROR_MAP[err_code][reason_code or 'DEFAULT']`.
fn error_message(code: &str, reason: Option<&str>) -> Option<&'static str> {
    let map: &[(&str, &str)] = match code {
        "FORBIDDEN" => &[
            (
                "ADMINISTRATOR_DELETE_VIDEO",
                "Video unavailable, possibly removed by admins",
            ),
            ("CHANNEL_MEMBER_ONLY", "Channel members only"),
            (
                "DELETED_CHANNEL_VIDEO",
                "Video unavailable, channel was closed",
            ),
            (
                "DELETED_COMMUNITY_VIDEO",
                "Video unavailable, community deleted or missing",
            ),
            ("DEFAULT", "Page unavailable, check the URL"),
            ("HARMFUL_VIDEO", "Sensitive content, login required"),
            ("HIDDEN_VIDEO", "Video unavailable, set to private"),
            ("NOT_ALLOWED", "No permission"),
            ("PPV_VIDEO", "PPV video, payment information required"),
            ("PREMIUM_ONLY", "Premium members only"),
        ],
        "INVALID_PARAMETER" => &[("DEFAULT", "Video unavailable, may not exist or was deleted")],
        "MAINTENANCE" => &[("DEFAULT", "Maintenance is in progress")],
        "NOT_FOUND" => &[
            ("DEFAULT", "Video unavailable, may not exist or was deleted"),
            (
                "RIGHT_HOLDER_DELETE_VIDEO",
                "Removed by rights-holder request",
            ),
        ],
        "UNAUTHORIZED" => &[("DEFAULT", "Invalid session, re-login required")],
        _ => &[],
    };
    let find = |key: &str| {
        map.iter()
            .find(|(name, _)| *name == key)
            .map(|(_, message)| *message)
    };
    reason.and_then(find).or_else(|| find("DEFAULT"))
}

fn logged_in() -> bool {
    crate::cookies::get_cookie(BASE_URL, "user_session").is_some_and(|value| !value.is_empty())
}

/// `_download_json(url, data=json, headers)` for the access-rights request.
fn post_json(
    client: &Client,
    url: &Url,
    id: &str,
    headers: &[(&str, &str)],
    body: String,
) -> Result<Value> {
    use std::io::Read;
    crate::screen::screen(&format!("[{NAME}] {id}: Downloading JSON metadata"));
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
    let mut bytes = Vec::new();
    response
        .read_to_end(&mut bytes)
        .map_err(Error::ExtractionRead)?;
    serde_json::from_slice(&bytes)
        .map_err(|error| fail(id, &format!("Failed to parse JSON  (caused by {error})")))
}

/// `_extract_formats(api_data, video_id)`.
fn extract_formats(
    client: &Client,
    api_data: &Value,
    video_id: &str,
) -> Result<Vec<crate::VideoFormat>> {
    let available = |path: &str| -> Vec<Value> {
        api_data
            .pointer(path)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|item| {
                item.get("isAvailable")
                    .is_some_and(|flag| truthy(Some(flag)))
                    && truthy(item.get("id"))
            })
            .cloned()
            .collect()
    };
    let videos = available("/media/domand/videos");
    let audios = available("/media/domand/audios");
    let access_key = api_data
        .pointer("/media/domand/accessRightKey")
        .and_then(Value::as_str)
        .filter(|key| !key.is_empty());
    let track_id = api_data
        .pointer("/client/watchTrackId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty());
    let (Some(access_key), Some(track_id)) = (access_key, track_id) else {
        return Ok(Vec::new());
    };
    if videos.is_empty() || audios.is_empty() {
        return Ok(Vec::new());
    }
    let mut outputs = Vec::new();
    for video in &videos {
        for audio in &audios {
            outputs.push(json!([video["id"], audio["id"]]));
        }
    }
    let mut url = Url::parse(&format!("{API_BASE}/v1/watch/{video_id}/access-rights/hls"))?;
    url.query_pairs_mut().append_pair("actionTrackId", track_id);
    let mut headers = vec![
        ("Accept", "application/json;charset=utf-8"),
        ("Content-Type", "application/json"),
        ("X-Access-Right-Key", access_key),
        ("X-Request-With", BASE_URL),
    ];
    headers.extend(HEADERS);
    let rights = post_json(
        client,
        &url,
        video_id,
        &headers,
        crate::output::python_json_dumps(&json!({"outputs": outputs})),
    )?;
    let Some(data) = rights.get("data") else {
        return Err(bug(video_id, "KeyError('data')"));
    };
    let Some(m3u8_url) = data.get("contentUrl") else {
        return Err(bug(video_id, "KeyError('contentUrl')"));
    };
    let manifest = Url::parse(m3u8_url.as_str().unwrap_or_default())?;
    let raw = super::extract_m3u8_fatal(client, &manifest, NAME, video_id, None, false)?.formats;

    let mut formats = Vec::new();
    for mut format in raw
        .iter()
        .filter(|format| format.vcodec.as_deref() == Some("none"))
        .cloned()
    {
        if let Some(audio) = audios.iter().find(|audio| {
            audio
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| format.format_id.starts_with(id))
        }) {
            if let Some(abr) = audio
                .get("bitRate")
                .and_then(Value::as_f64)
                .map(|rate| rate / 1000.0)
            {
                format.abr = Some(abr);
                crate::info_order::mark_format_update(&mut format, "abr");
            }
            if let Some(asr) = int(audio.get("samplingRate")) {
                format.asr = u32::try_from(asr).ok();
                crate::info_order::mark_format_update(&mut format, "asr");
            }
            if let Some(id) = audio.get("id").and_then(Value::as_str) {
                format.format_id = id.to_owned();
            }
            if let Some(quality) = int(audio.get("qualityLevel")) {
                format.quality = Some(quality as f64);
                crate::info_order::mark_int_quality(&mut format);
            }
        }
        format.acodec = Some("aac".into());
        crate::info_order::mark_format_update(&mut format, "acodec");
        formats.push(format);
    }
    // Sorted by `tbr` (stable), keeping each URL's lowest-`tbr` format.
    let mut video_formats: Vec<crate::VideoFormat> = raw
        .into_iter()
        .filter(|format| format.vcodec.as_deref() != Some("none"))
        .collect();
    video_formats.sort_by(|left, right| {
        left.tbr
            .unwrap_or(0.0)
            .partial_cmp(&right.tbr.unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut seen: Vec<Url> = Vec::new();
    video_formats.retain(|format| {
        if seen.contains(&format.url) {
            false
        } else {
            seen.push(format.url.clone());
            true
        }
    });
    let min_abr = audios
        .iter()
        .filter_map(|audio| audio.get("bitRate").and_then(Value::as_f64))
        .map(|rate| rate / 1000.0)
        .reduce(f64::min)
        .unwrap_or(0.0);
    for mut format in video_formats {
        let basename = format
            .url
            .path_segments()
            .and_then(|mut segments| segments.next_back())
            .unwrap_or_default()
            .to_owned();
        format.format_id = basename
            .rsplit_once('.')
            .map_or(String::new(), |(stem, _)| stem.to_owned());
        let quality = videos
            .iter()
            .find(|video| {
                video.get("id").and_then(Value::as_str) == Some(format.format_id.as_str())
            })
            .and_then(|video| int(video.get("qualityLevel")))
            .filter(|quality| *quality != 0)
            .unwrap_or(-1);
        format.quality = Some(quality as f64);
        crate::info_order::mark_int_quality(&mut format);
        format.tbr = format.tbr.map(|tbr| tbr - min_abr);
        formats.push(format);
    }
    Ok(formats)
}

fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Number(number)) => number.as_f64() != Some(0.0),
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(items)) => !items.is_empty(),
    }
}

/// `parse_resolution(url, lenient=True)`.
fn lenient_resolution(text: &str) -> (Option<u32>, Option<u32>) {
    static SIZE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d+)\s*[xX×,]\s*(\d+)").unwrap());
    static WIDTH: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?:^|[^\d])(\d{2,5})w(?:$|[^a-zA-Z0-9])").unwrap());
    if let Some(found) = SIZE.captures(text) {
        return (found[1].parse().ok(), found[2].parse().ok());
    }
    let strict = super::html5::parse_resolution(text);
    if strict.1.is_some() {
        return strict;
    }
    match WIDTH.captures(text) {
        Some(found) => (found[1].parse().ok(), None),
        None => (None, None),
    }
}

impl Extractor for Niconico {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, page: &Url) -> Result<Option<ExtractResult>> {
        use std::io::Read;
        let Some(found) = VALID_URL.captures(page.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(page.as_str())
        ));
        let path = if logged_in() { "v3" } else { "v3_guest" };
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |time| time.as_millis());
        let mut api = Url::parse(&format!("{BASE_URL}/api/watch/{path}/{video_id}"))?;
        api.query_pairs_mut()
            .append_pair("actionTrackId", &format!("AAAAAAAAAA_{millis}"));
        crate::screen::screen(&format!("[{NAME}] {video_id}: Downloading API JSON"));
        let mut response = super::direct::request_expecting(client, &api, &HEADERS, &[400, 404])
            .map_err(|error| match error {
                Error::Http(_) | Error::HttpRetryAfter { .. } => Error::Reported(
                    super::request_error_text(NAME, &video_id, "API JSON", &error).replacen(
                        ": Unable to download API JSON:",
                        ": Unable to fetch data:",
                        1,
                    ),
                ),
                error => error,
            })?;
        let mut bytes = Vec::new();
        response
            .read_to_end(&mut bytes)
            .map_err(Error::ExtractionRead)?;
        let api_resp: Value = serde_json::from_slice(&bytes).map_err(|error| {
            fail(
                &video_id,
                &format!("Failed to parse JSON  (caused by {error})"),
            )
        })?;
        let api_data = api_resp
            .get("data")
            .filter(|data| data.is_object())
            .cloned()
            .unwrap_or(Value::Null);
        let scheduled = api_data
            .get("publishScheduledAt")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let status = api_resp.pointer("/meta/status").and_then(Value::as_i64);
        if status != Some(200) {
            let code = api_resp
                .pointer("/meta/errorCode")
                .and_then(Value::as_str)
                .map(str::to_uppercase);
            let reason = str_or_none(api_data.get("reasonCode"));
            let mut message = code
                .as_deref()
                .and_then(|code| error_message(code, reason.as_deref()))
                .map(str::to_owned);
            match reason.as_deref() {
                Some("DOMESTIC_VIDEO" | "HIGH_RISK_COUNTRY_VIDEO") => {
                    // `raise_geo_restricted(countries=['JP'])`.
                    return Err(fail(
                        &video_id,
                        "This video is not available from your location due to geo restriction\nThis video is available in Japan.\nYou might want to use a VPN or a proxy server (with --proxy) to workaround.",
                    ));
                }
                Some("HARMFUL_VIDEO")
                    if api_data.pointer("/viewer/allowSensitiveContents")
                        == Some(&Value::Bool(false)) =>
                {
                    message = Some("Sensitive content, adjust display settings to watch".into());
                }
                Some("HIDDEN_VIDEO") if scheduled.is_some() => {
                    message = Some(format!(
                        "This content is scheduled to be released at {}",
                        scheduled.as_deref().unwrap_or_default()
                    ));
                }
                Some(
                    "CHANNEL_MEMBER_ONLY"
                    | "HARMFUL_VIDEO"
                    | "HIDDEN_VIDEO"
                    | "PPV_VIDEO"
                    | "PREMIUM_ONLY",
                ) => {
                    let Some(message) = message else {
                        return Err(bug(
                            &video_id,
                            "TypeError(\"unsupported operand type(s) for +=: 'NoneType' and 'str'\")",
                        ));
                    };
                    return Err(fail(&video_id, &format!("{message}. {LOGIN_HINT}")));
                }
                _ => {}
            }
            if let Some(message) = message {
                return Err(fail(&video_id, &message));
            }
            if status.is_some_and(|status| status >= 500) {
                return Err(fail(&video_id, "Service temporarily unavailable"));
            }
            return Err(fail(
                &video_id,
                &format!(
                    "API returned error status {}; {}",
                    status.map_or("None".into(), |status| status.to_string()),
                    crate::BUG_REPORTS_MESSAGE
                ),
            ));
        }
        // `_availability(needs_auth, needs_subscription, needs_premium)`.
        let payment = api_data.pointer("/payment/video");
        let flag = |key: &str| {
            payment
                .and_then(|payment| payment.get(key))
                .and_then(Value::as_bool)
        };
        let availability = if flag("isPremium") == Some(true) {
            "premium_only"
        } else if flag("isAdmission") == Some(true) {
            "subscriber_only"
        } else if flag("isContinuationBenefit")
            .or_else(|| flag("isPpv"))
            .unwrap_or(false)
        {
            "needs_auth"
        } else {
            "public"
        };
        let formats = extract_formats(client, &api_data, &video_id)?;
        let status_message = match availability {
            "needs_auth" => Some("PPV video, payment information required"),
            "premium_only" => Some("Premium members only"),
            "subscriber_only" => Some("Channel members only"),
            _ => None,
        };
        if formats.is_empty()
            && let Some(message) = status_message
        {
            return Err(fail(&video_id, &format!("{message}. {LOGIN_HINT}")));
        }

        let mut dict = Map::new();
        dict.insert("availability".into(), availability.into());
        dict.insert("display_id".into(), video_id.clone().into());
        dict.insert("formats".into(), json!([]));
        if let Some(genre) = api_data
            .pointer("/genre/label")
            .and_then(Value::as_str)
            .filter(|label| !label.is_empty())
        {
            dict.insert("genres".into(), json!([genre]));
        }
        dict.insert(
            "release_timestamp".into(),
            scheduled
                .as_deref()
                .and_then(crate::dates::parse_iso8601)
                .into(),
        );
        dict.insert(
            "subtitles".into(),
            if super::subtitles_requested() {
                if truthy(api_data.pointer("/comment/nvComment/server")) {
                    return Err(Error::UnsupportedHtml5(
                        "Niconico comments (inline subtitle data) are not supported natively"
                            .into(),
                    ));
                }
                Value::Null
            } else {
                json!({})
            },
        );
        let tags: Vec<Value> = api_data
            .pointer("/tag/items")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|tag| tag.get("name").and_then(Value::as_str))
            .filter(|name| !name.is_empty())
            .map(Value::from)
            .collect();
        if !tags.is_empty() {
            dict.insert("tags".into(), Value::Array(tags));
        }
        let thumbnails: Vec<Value> = api_data
            .pointer("/video/thumbnail")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .filter_map(|(key, url)| {
                let url = url.as_str().and_then(super::json_ld::url_or_none)?;
                let preference = THUMB_PREFS
                    .iter()
                    .position(|name| name == key)
                    .map_or(-1, |index| index as i64);
                let mut thumbnail = Map::new();
                thumbnail.insert("ext".into(), "jpg".into());
                thumbnail.insert("id".into(), key.clone().into());
                thumbnail.insert("preference".into(), preference.into());
                thumbnail.insert("url".into(), url.clone().into());
                let (width, height) = lenient_resolution(&url);
                if let Some(width) = width {
                    thumbnail.insert("width".into(), width.into());
                }
                if let Some(height) = height {
                    thumbnail.insert("height".into(), height.into());
                }
                Some(Value::Object(thumbnail))
            })
            .collect();
        dict.insert("thumbnails".into(), Value::Array(thumbnails));
        if let Some(owner) = ["channel", "owner"]
            .iter()
            .find_map(|key| api_data.get(*key).filter(|owner| !owner.is_null()))
        {
            let name = ["name", "nickname"]
                .iter()
                .find_map(|key| owner.get(*key).and_then(Value::as_str));
            let id = str_or_none(owner.get("id"));
            if let Some(name) = name {
                dict.insert("channel".into(), name.into());
            }
            if let Some(id) = &id {
                dict.insert("channel_id".into(), id.clone().into());
            }
            if let Some(name) = name {
                dict.insert("uploader".into(), name.into());
            }
            if let Some(id) = id {
                dict.insert("uploader_id".into(), id.into());
            }
        }
        let video = api_data.get("video").cloned().unwrap_or(Value::Null);
        if let Some(id) = str_or_none(video.get("id")) {
            dict.insert("id".into(), id.into());
        }
        if let Some(title) = video.get("title").and_then(Value::as_str) {
            dict.insert("title".into(), title.into());
        }
        if let Some(description) = video
            .get("description")
            .and_then(Value::as_str)
            .map(crate::html::clean_html)
            .filter(|text| !text.is_empty())
        {
            dict.insert("description".into(), description.into());
        }
        if let Some(duration) = int(video.get("duration")) {
            dict.insert("duration".into(), duration.into());
        }
        if let Some(timestamp) = video
            .get("registeredAt")
            .and_then(Value::as_str)
            .and_then(crate::dates::parse_iso8601)
        {
            dict.insert("timestamp".into(), timestamp.into());
        }
        for (key, source) in [
            ("comment_count", "comment"),
            ("like_count", "like"),
            ("view_count", "view"),
        ] {
            if let Some(count) = int(video.pointer(&format!("/count/{source}"))) {
                dict.insert(key.into(), count.into());
            }
        }
        if !dict.contains_key("id") {
            return Err(fail(&video_id, "Missing \"id\" field in extractor result"));
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
                "formats" => {}
                _ if value.is_null() => {
                    info.extra.insert(key, Value::Null);
                }
                _ => crate::metadata_parser::set_field(&mut info, &key, value),
            }
        }
        Ok(Some(ExtractResult::Video(crate::info_order::mark_info(
            info,
            &format!(
                "dict{};{};thumbnail=ext,id,preference,url,width,height",
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
        assert!(suitable("https://www.nicovideo.jp/watch/sm9"));
        assert!(suitable(
            "https://sp.nicovideo.jp/watch/sm28964488?ss_pos=1&cp_in=wt_tg"
        ));
        assert!(suitable("https://www.nicovideo.jp/shorts/sm45678901"));
        assert!(suitable("https://www.nicovideo.jp/watch/1173108780"));
        assert!(!suitable("https://www.nicovideo.jp/mylist/27411728"));
    }

    #[test]
    fn errors_follow_the_error_map() {
        assert_eq!(
            error_message("NOT_FOUND", Some("RIGHT_HOLDER_DELETE_VIDEO")),
            Some("Removed by rights-holder request")
        );
        assert_eq!(
            error_message("FORBIDDEN", Some("UNKNOWN")),
            Some("Page unavailable, check the URL")
        );
        assert_eq!(error_message("OTHER", None), None);
    }
}

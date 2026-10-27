//! Upstream `BitChuteIE` (`yt_dlp/extractor/bitchute.py`): the beta API's
//! `video/media` URL (HLS, or a progressive file checked across upstream's
//! seed hosts), the `video` and `channel` metadata (non-fatal) and the API's
//! geo-restriction errors. Channel and playlist pages are not supported.
use super::Extractor;
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct BitChute;

const NAME: &str = "BitChute";
/// `_check_format`'s alternative seed hosts (the URL's own first).
const SEEDS: [&str; 16] = [
    "seed122",
    "seed125",
    "seed126",
    "seed128",
    "seed132",
    "seed150",
    "seed151",
    "seed152",
    "seed153",
    "seed167",
    "seed171",
    "seed177",
    "seed305",
    "seed307",
    "seedp29xb",
    "zb10-7gsop1v78",
];

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:(?:www|old)\.)?bitchute\.com/(?:video|embed|torrent/[^/?#]+)/(?P<id>[^/?#&]+)")
        .unwrap()
});
static SEED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(https?://)(seed\d+)(\.bitchute\.com)").unwrap());

/// `BitChuteIE.suitable`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn fail(id: &str, message: &str) -> Error {
    Error::Reported(format!("[{NAME}] {id}: {message}"))
}

/// `_call_api(endpoint, data, display_id, fatal)`: `Ok(None)` after a
/// non-fatal failure's warning.
fn call_api(
    client: &Client,
    endpoint: &str,
    data: &Value,
    id: &str,
    fatal: bool,
) -> Result<Option<Value>> {
    use std::io::Read;
    let note = endpoint.rsplit('/').next().unwrap_or(endpoint);
    let url = Url::parse(&format!("https://api.bitchute.com/api/beta/{endpoint}"))?;
    crate::screen::screen(&format!("[{NAME}] {id}: Downloading {note} API JSON"));
    let what = format!("{note} API JSON");
    let failure = |message: String| -> Result<Option<Value>> {
        if fatal {
            return Err(fail(id, &message));
        }
        // `report_warning(e.msg)`: the message carries the ID.
        crate::screen::warning(&format!("[{NAME}] {id}: {message}"));
        Ok(None)
    };
    let response = super::direct::post_expecting(
        client,
        &url,
        &[
            ("Accept", "application/json"),
            ("Content-Type", "application/json"),
        ],
        crate::output::python_json_dumps(data),
        &[403],
    );
    let mut response = match response {
        Ok(response) => response,
        Err(error @ (Error::Http(_) | Error::HttpRetryAfter { .. })) => {
            let text = super::request_error_text(NAME, id, &what, &error);
            let message = text
                .split_once(": ")
                .map_or(text.as_str(), |(_, rest)| rest)
                .to_owned();
            return failure(message);
        }
        Err(error) => return Err(error),
    };
    let status = response.status();
    let mut bytes = Vec::new();
    response
        .read_to_end(&mut bytes)
        .map_err(Error::ExtractionRead)?;
    if status.as_u16() == 403 {
        let errors: Vec<String> = serde_json::from_slice::<Value>(&bytes)
            .ok()
            .and_then(|body| body.get("errors").and_then(Value::as_array).cloned())
            .into_iter()
            .flatten()
            .filter(|error| error.get("context").and_then(Value::as_str) == Some("reason"))
            .filter_map(|error| {
                error
                    .get("message")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .collect();
        let errors = errors.join(". ");
        if !errors.is_empty() && errors.contains("location") {
            // `raise_geo_restricted(errors)`.
            return Err(fail(
                id,
                &format!(
                    "{errors}\nYou might want to use a VPN or a proxy server (with --proxy) to workaround."
                ),
            ));
        }
        let reason = status.canonical_reason().unwrap_or("Forbidden");
        return failure(format!(
            "Unable to download {what}: HTTP Error 403: {reason} (caused by <HTTPError 403: {reason}>)"
        ));
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| fail(id, &format!("Failed to parse JSON  (caused by {error})")))
}

/// `_check_format(video_url, video_id)`: the first seed host answering a HEAD.
fn check_format(client: &Client, media_url: &str, id: &str) -> Result<Option<Map<String, Value>>> {
    let mut urls: Vec<String> = vec![media_url.to_owned()];
    for seed in SEEDS {
        let url = SEED
            .replace(media_url, |found: &regex::Captures<'_>| {
                format!("{}{seed}{}", &found[1], &found[3])
            })
            .into_owned();
        if !urls.contains(&url) {
            urls.push(url);
        }
    }
    for url in urls {
        crate::screen::screen(&format!("[{NAME}] {id}: Checking {url}"));
        let Ok(parsed) = Url::parse(&url) else {
            continue;
        };
        match super::direct::head_with_headers(client, &parsed, &[]) {
            Ok(response) => {
                let mut format = Map::new();
                format.insert("url".into(), url.clone().into());
                format.insert(
                    "filesize".into(),
                    response
                        .headers()
                        .get(reqwest::header::CONTENT_LENGTH)
                        .and_then(|value| value.to_str().ok())
                        .and_then(|value| value.trim().parse::<i64>().ok())
                        .into(),
                );
                return Ok(Some(format));
            }
            Err(error @ (Error::Http(_) | Error::HttpRetryAfter { .. })) => {
                crate::screen::screen(&format!(
                    "[{NAME}] {id}: URL is invalid, skipping: {}",
                    crate::upstream_error_text(&error)
                ));
            }
            Err(error) => return Err(error),
        }
    }
    Ok(None)
}

impl Extractor for BitChute {
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
        let data = json!({"video_id": video_id});
        let media = call_api(client, "video/media", &data, &video_id, true)?.unwrap_or_default();
        let Some(media_url) = media.get("media_url") else {
            return Err(fail(
                &video_id,
                &format!(
                    "An extractor error has occurred. (caused by KeyError('media_url')); {}",
                    crate::BUG_REPORTS_MESSAGE
                ),
            ));
        };
        let media_url = media_url.as_str().unwrap_or_default().to_owned();
        let mut formats: Vec<crate::VideoFormat> = Vec::new();
        if super::direct::determine_ext(&media_url).as_deref() == Some("m3u8") {
            let manifest = Url::parse(&media_url)?;
            let mut stream =
                super::extract_m3u8_fatal(client, &manifest, NAME, &video_id, Some("hls"), false)?;
            super::hls::live_format_ids(&mut stream.formats, Some("hls"));
            formats.extend(stream.formats);
        } else {
            let map = if super::check_formats_never() {
                let mut map = Map::new();
                map.insert("url".into(), media_url.clone().into());
                Some(map)
            } else {
                check_format(client, &media_url, &video_id)?
            };
            if let Some(mut map) = map {
                let keys: Vec<String> = map.keys().cloned().collect();
                map.retain(|_, value| !value.is_null());
                let ext = map
                    .get("url")
                    .and_then(Value::as_str)
                    .and_then(super::direct::determine_ext)
                    .unwrap_or_else(|| "unknown_video".into());
                map.insert("ext".into(), ext.into());
                map.insert("format_id".into(), "".into());
                let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(map))
                    .map_err(|error| fail(&video_id, &error.to_string()))?;
                crate::info_order::mark_format(&mut format, &format!("dict:{}", keys.join(",")));
                formats.push(format);
            }
        }
        if formats.is_empty() {
            return Err(fail(
                &video_id,
                "Video is unavailable. Please make sure this video is playable in the browser before reporting this issue.",
            ));
        }
        let video = call_api(client, "video", &data, &video_id, false)?;
        let channel = match video
            .as_ref()
            .and_then(|video| video.pointer("/channel/channel_id"))
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
        {
            Some(channel_id) => call_api(
                client,
                "channel",
                &json!({"channel_id": channel_id}),
                &video_id,
                false,
            )?,
            None => None,
        };
        let video = video.unwrap_or(Value::Null);
        let text = |value: &Value, path: &str| {
            value
                .pointer(path)
                .filter(|value| value.is_string())
                .cloned()
        };
        let uploader_url = |value: &Value| {
            value
                .get("profile_id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .map(|id| Value::from(format!("https://www.bitchute.com/profile/{id}/")))
        };
        let mut dict = Map::new();
        let mut put = |key: &str, value: Option<Value>| {
            if let Some(value) = value {
                dict.insert(key.into(), value);
            }
        };
        put("title", text(&video, "/video_name"));
        put("description", text(&video, "/description"));
        put(
            "thumbnail",
            video
                .get("thumbnail_url")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
                .map(Value::from),
        );
        put("channel", text(&video, "/channel/channel_name"));
        put("channel_id", text(&video, "/channel/channel_id"));
        put(
            "channel_url",
            video
                .pointer("/channel/channel_url")
                .and_then(Value::as_str)
                .filter(|path| !path.is_empty())
                .and_then(|path| {
                    Url::parse("https://www.bitchute.com/")
                        .ok()?
                        .join(path)
                        .ok()
                })
                .map(|url| Value::from(url.as_str())),
        );
        put("uploader_id", text(&video, "/profile_id"));
        put("uploader_url", uploader_url(&video));
        put(
            "timestamp",
            video
                .get("date_published")
                .and_then(Value::as_str)
                .and_then(crate::dates::parse_iso8601)
                .map(Value::from),
        );
        put(
            "duration",
            video
                .get("duration")
                .and_then(Value::as_str)
                .and_then(crate::parse_duration)
                .and_then(|duration| serde_json::to_value(duration).ok()),
        );
        let tags: Vec<Value> = video
            .get("hashtags")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|tag| tag.as_str().is_some_and(|tag| !tag.is_empty()))
            .cloned()
            .collect();
        put("tags", (!tags.is_empty()).then_some(Value::Array(tags)));
        put(
            "view_count",
            video.get("view_count").and_then(|count| {
                count
                    .as_i64()
                    .or_else(|| count.as_f64().map(|count| count as i64))
                    .or_else(|| count.as_str()?.trim().parse().ok())
                    .map(Value::from)
            }),
        );
        put(
            "is_live",
            Some(Value::Bool(
                video.get("state_id").and_then(Value::as_str) == Some("live"),
            )),
        );
        if let Some(channel) = &channel {
            put("channel", text(channel, "/channel_name"));
            put("channel_id", text(channel, "/channel_id"));
            put(
                "channel_url",
                channel
                    .get("url_slug")
                    .and_then(Value::as_str)
                    .filter(|slug| !slug.is_empty())
                    .map(|slug| Value::from(format!("https://www.bitchute.com/channel/{slug}/"))),
            );
            put("uploader", text(channel, "/profile_name"));
            put("uploader_id", text(channel, "/profile_id"));
            put("uploader_url", uploader_url(channel));
        }
        dict.insert("id".into(), video_id.clone().into());
        dict.insert("formats".into(), json!([]));

        let first = formats
            .first()
            .map_or_else(|| page.clone(), |format| format.url.clone());
        let mut info = super::direct::build_at(first.clone(), &first, String::new());
        info.direct = None;
        info.formats = formats;
        info.webpage_url = page.clone();
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
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
                _ => crate::metadata_parser::set_field(&mut info, &key, value),
            }
        }
        Ok(Some(ExtractResult::Video(crate::info_order::mark_info(
            info,
            &format!(
                "dict{};{}",
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
        assert!(suitable("https://www.bitchute.com/video/UGlrF9o9b-Q/"));
        assert!(suitable("https://old.bitchute.com/embed/lbb5G1hjPhw/"));
        assert!(suitable(
            "https://www.bitchute.com/torrent/Zee5BE49045h/szoMrox2JEI.webtorrent"
        ));
        assert!(!suitable("https://www.bitchute.com/channel/livesonnet/"));
    }
}

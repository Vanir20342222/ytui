//! Upstream `VimeoIE` (`yt_dlp/extractor/vimeo.py`) without login or
//! browser impersonation: player pages' `playerConfig` and vimeo.com
//! pages' clip page config through `_parse_config` (progressive, per-CDN
//! HLS/DASH formats, text tracks, thumbnails, chapters and live status).
use super::Extractor;
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Vimeo;

const NAME: &str = "vimeo";
const KEY: &str = "Vimeo";
const QUALITIES: [&str; 3] = ["low", "medium", "high"];

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:(?:www|player)\.)?vimeo\.com/(?:(?P<u>user)|(?:[^?#]*?/)??(?P<q>(?:play_redirect_hls|moogaloop\.swf)\?clip_id=)?(?:videos?/)?)(?P<id>[0-9]+)(?P<rest>[^#]*)(?:#.*)?$",
    )
    .unwrap()
});
static EXCLUDED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:(?:www|player)\.)?vimeo\.com/(?:(?:channels|album|showcase)/[^/?#]+/?(?:$|[?#])|[^/]+/review/|ondemand/|(?:.*?/)??event/)")
        .unwrap()
});
static UNLISTED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^/(?P<hash>[\da-f]{10})/?(?:\?.*)?$").unwrap());
static PLAYER_CONFIG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(?:playerC|c)onfig\s*=").unwrap());
static PAGE_CONFIG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"vimeo\.(?:clip|vod_title)_page_config\s*=\s*(\{.+?\});").unwrap()
});
static VIMEO_CONFIG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"vimeo\.config\s*=\s*(?:(\{.+?\})|_extend\([^,]+,\s+(\{.+?\})\));").unwrap()
});
static SEP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"/sep/video/").unwrap());
static AUDIO_QUALITY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"audio-(low|medium|high)").unwrap());

/// The `(?(u)...)` branch: a `user` URL's trailing name.
fn user_rest(rest: &str) -> bool {
    let Some(name) = rest.strip_prefix('/') else {
        return false;
    };
    let end = name.find(['/', '?', '#']).unwrap_or(name.len());
    let (segment, after) = name.split_at(end);
    !segment.is_empty()
        && !segment.starts_with("videos")
        && !segment.starts_with("likes")
        && (after.is_empty() || after == "/" || after.starts_with('?') || after.starts_with("/?"))
}

/// `VimeoIE._match_valid_url`: the ID and any unlisted hash.
fn match_url(url: &str) -> Option<(String, Option<String>)> {
    if EXCLUDED.is_match(url) {
        return None;
    }
    let found = VALID_URL.captures(url)?;
    let rest = found.name("rest").map_or("", |rest| rest.as_str());
    let id = found["id"].to_owned();
    if found.name("u").is_some() {
        return user_rest(rest).then_some((id, None));
    }
    if found.name("q").is_some() {
        return (rest.is_empty() || rest.starts_with('&')).then_some((id, None));
    }
    if let Some(hash) = UNLISTED.captures(rest) {
        return Some((id, Some(hash["hash"].to_owned())));
    }
    let tail = rest.strip_prefix('/').unwrap_or(rest);
    (tail.is_empty() || tail.starts_with('?')).then_some((id, None))
}

/// `VimeoIE.suitable`.
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

fn impersonation_warning() {
    crate::screen::warning_once(&format!(
        "[{NAME}] The extractor is attempting impersonation, but no impersonate target is available. If you encounter errors, then see  https://github.com/yt-dlp/yt-dlp#impersonation  for information on installing the required dependencies"
    ));
}

/// `_parse_config(config, video_id)`: the info dict and its formats.
fn parse_config(
    client: &Client,
    config: &Value,
    video_id: &str,
) -> Result<(
    Map<String, Value>,
    Vec<crate::VideoFormat>,
    crate::Subtitles,
)> {
    let video = config.get("video").cloned().unwrap_or_else(|| json!({}));
    let live_event = video
        .get("live_event")
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let live_status = match live_event.get("status").and_then(Value::as_str) {
        Some("pending" | "active") => Some("is_upcoming"),
        Some("started") => Some("is_live"),
        Some("ended") => Some("post_live"),
        _ => None,
    };
    let is_live = live_status == Some("is_live");
    let request = config.get("request").cloned().unwrap_or_else(|| json!({}));
    let files = video
        .get("files")
        .filter(|files| files.as_object().is_some_and(|files| !files.is_empty()))
        .or_else(|| request.get("files"))
        .cloned()
        .unwrap_or_else(|| json!({}));
    let mut formats = Vec::new();
    let mut subtitles = crate::Subtitles::default();
    for file in files
        .get("progressive")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(url) = file
            .get("url")
            .and_then(Value::as_str)
            .filter(|url| !url.is_empty())
        else {
            continue;
        };
        let quality = match file.get("quality") {
            Some(Value::String(text)) => text.clone(),
            Some(Value::Null) | None => "None".into(),
            Some(other) => other.to_string(),
        };
        let mut format = Map::new();
        format.insert("url".into(), url.into());
        format.insert("format_id".into(), format!("http-{quality}").into());
        format.insert("source_preference".into(), 10.into());
        for key in ["width", "height", "fps"] {
            format.insert(key.into(), int(file.get(key)).into());
        }
        format.insert("tbr".into(), int(file.get("bitrate")).into());
        let keys: Vec<String> = format.keys().cloned().collect();
        for key in ["width", "height", "fps", "tbr"] {
            if format.get(key).is_some_and(Value::is_null) {
                format.shift_remove(key);
            }
        }
        format.insert(
            "ext".into(),
            super::direct::determine_ext(url)
                .unwrap_or_else(|| "unknown_video".into())
                .into(),
        );
        let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(format))
            .map_err(|error| fail(video_id, &error.to_string()))?;
        crate::info_order::mark_format(&mut format, &format!("dict:{}", keys.join(",")));
        crate::info_order::mark_int_rates(&mut format);
        formats.push(format);
    }
    for files_type in ["hls", "dash"] {
        let Some(cdns) = files
            .get(files_type)
            .and_then(|kind| kind.get("cdns"))
            .and_then(Value::as_object)
        else {
            continue;
        };
        for (cdn_name, cdn) in cdns {
            let Some(manifest_url) = cdn
                .get("url")
                .and_then(Value::as_str)
                .filter(|u| !u.is_empty())
            else {
                continue;
            };
            let format_id = format!("{files_type}-{cdn_name}");
            let urls: Vec<(String, String)> = if SEP.is_match(manifest_url) {
                [("", "video"), ("_sep", "sep/video")]
                    .iter()
                    .map(|(suffix, replacement)| {
                        (
                            format!("{format_id}{suffix}"),
                            SEP.replace_all(manifest_url, format!("/{replacement}/").as_str())
                                .into_owned(),
                        )
                    })
                    .collect()
            } else {
                vec![(format_id.clone(), manifest_url.to_owned())]
            };
            for (f_id, m_url) in urls {
                if files_type == "hls" {
                    let Some(stream) = Url::parse(&m_url).ok().and_then(|manifest| {
                        super::extract_m3u8_optional_note(
                            client,
                            &manifest,
                            NAME,
                            video_id,
                            Some(&f_id),
                            true,
                            is_live,
                            &format!("Downloading {cdn_name} m3u8 information"),
                        )
                    }) else {
                        continue;
                    };
                    for mut format in stream.formats {
                        if let Some(found) = AUDIO_QUALITY.captures(&format.format_id) {
                            let quality = QUALITIES
                                .iter()
                                .position(|quality| *quality == &found[1])
                                .map_or(-1, |index| index as i64);
                            format.quality = Some(quality as f64);
                            crate::info_order::mark_format_update(&mut format, "quality");
                            crate::info_order::mark_int_rates(&mut format);
                        }
                        formats.push(format);
                    }
                    super::html5::merge_subtitles(&mut subtitles, stream.subtitles);
                } else {
                    let mut m_url = m_url;
                    if m_url.contains("json=1")
                        && let Some(real) = Url::parse(&m_url)
                            .ok()
                            .and_then(|json_url| {
                                super::download_json(client, &json_url, NAME, video_id).ok()
                            })
                            .and_then(|data| {
                                data.get("url").and_then(Value::as_str).map(str::to_owned)
                            })
                    {
                        m_url = real;
                    }
                    let Some(stream) = Url::parse(&m_url.replace("/master.json", "/master.mpd"))
                        .ok()
                        .and_then(|manifest| {
                            super::extract_mpd_optional_note(
                                client,
                                &manifest,
                                NAME,
                                video_id,
                                Some(&f_id),
                                &format!("Downloading {cdn_name} MPD information"),
                            )
                        })
                    else {
                        continue;
                    };
                    formats.extend(stream.formats);
                    super::html5::merge_subtitles(&mut subtitles, stream.subtitles);
                }
            }
        }
    }
    if let Some(source) = live_event
        .pointer("/archive/source_url")
        .and_then(Value::as_str)
        .filter(|url| !url.is_empty())
        && live_event
            .pointer("/archive/status")
            .and_then(Value::as_str)
            == Some("done")
    {
        let mut format = Map::new();
        format.insert("format_id".into(), "live-archive-source".into());
        format.insert("url".into(), source.into());
        format.insert("quality".into(), 10.into());
        let keys: Vec<String> = format.keys().cloned().collect();
        format.insert(
            "ext".into(),
            super::direct::determine_ext(source)
                .unwrap_or_else(|| "unknown_video".into())
                .into(),
        );
        let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(format))
            .map_err(|error| fail(video_id, &error.to_string()))?;
        crate::info_order::mark_format(&mut format, &format!("dict:{}", keys.join(",")));
        crate::info_order::mark_int_rates(&mut format);
        formats.push(format);
    }
    for track in request
        .get("text_tracks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let (Some(lang), Some(path)) = (
            track.get("lang").and_then(Value::as_str),
            track.get("url").and_then(Value::as_str),
        ) else {
            continue;
        };
        let url = Url::parse("https://player.vimeo.com/")?.join(path)?;
        let track: crate::SubtitleFormat =
            serde_json::from_value(json!({"ext": "vtt", "url": url.as_str()}))
                .map_err(|error| fail(video_id, &error.to_string()))?;
        let language = Some(lang.to_owned());
        match subtitles
            .entries
            .iter_mut()
            .find(|(key, _)| *key == language)
        {
            Some((_, tracks)) => tracks.push(track),
            None => subtitles.entries.push((language, vec![track])),
        }
    }
    let mut thumbnails = Vec::new();
    if !is_live {
        for (key, thumb) in video
            .get("thumbs")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            thumbnails.push(json!({
                "id": key,
                "width": key.parse::<i64>().ok(),
                "url": thumb,
            }));
        }
        for key in ["thumbnail", "thumbnail_url"] {
            if let Some(url) = video
                .get(key)
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
            {
                thumbnails.push(json!({"url": url}));
            }
        }
    }
    let owner = video.get("owner").cloned().unwrap_or_else(|| json!({}));
    let uploader_url = owner
        .get("url")
        .and_then(Value::as_str)
        .filter(|url| !url.is_empty());
    let mut chapters: Vec<Value> = config
        .pointer("/embed/chapters")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|chapter| int(chapter.get("timecode")).is_some())
        .map(|chapter| {
            let mut found = Map::new();
            if let Some(title) = chapter.get("title").and_then(Value::as_str) {
                found.insert("title".into(), title.into());
            }
            found.insert("start_time".into(), int(chapter.get("timecode")).into());
            Value::Object(found)
        })
        .collect();
    chapters.sort_by_key(|chapter| int(chapter.get("start_time")).unwrap_or(0));
    let release_timestamp = live_event
        .pointer("/ingest/scheduled_start_time")
        .and_then(Value::as_str)
        .and_then(crate::dates::parse_iso8601)
        .or_else(|| int(live_event.pointer("/ingest/start_time")));
    let id = match video.get("id") {
        Some(Value::String(text)) if !text.is_empty() => text.clone(),
        Some(Value::Number(number)) => number.to_string(),
        _ => video_id.to_owned(),
    };
    let mut info = Map::new();
    info.insert("id".into(), id.into());
    info.insert(
        "title".into(),
        video.get("title").cloned().unwrap_or(Value::Null),
    );
    info.insert(
        "uploader".into(),
        owner.get("name").cloned().unwrap_or(Value::Null),
    );
    info.insert(
        "uploader_id".into(),
        uploader_url
            .map(|url| url.rsplit('/').next().unwrap_or_default().to_owned())
            .into(),
    );
    info.insert("uploader_url".into(), uploader_url.into());
    info.insert("thumbnails".into(), Value::Array(thumbnails));
    info.insert("duration".into(), int(video.get("duration")).into());
    info.insert(
        "chapters".into(),
        if chapters.is_empty() {
            Value::Null
        } else {
            Value::Array(chapters)
        },
    );
    info.insert("formats".into(), json!([]));
    info.insert("subtitles".into(), json!({}));
    info.insert("live_status".into(), live_status.into());
    info.insert("release_timestamp".into(), release_timestamp.into());
    info.insert(
        "_format_sort_fields".into(),
        json!(["quality", "res", "fps", "hdr:12", "source"]),
    );
    Ok((info, formats, subtitles))
}

/// `merge_dicts(a, b, ...)`.
fn merge_dicts(dicts: Vec<Map<String, Value>>) -> Map<String, Value> {
    let mut merged = Map::new();
    for dict in dicts {
        for (key, value) in dict {
            let replace = match merged.get(&key) {
                None => !value.is_null(),
                Some(Value::String(existing)) => existing.is_empty() && value.is_string(),
                Some(_) => false,
            };
            if replace {
                merged.insert(key, value);
            }
        }
    }
    merged
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
    let mut titled = false;
    for (key, value) in dict {
        keys.push(key.clone());
        match key.as_str() {
            "id" => info.id = value.as_str().unwrap_or_default().to_owned(),
            "title" => {
                titled = !value.is_null();
                info.title = value.as_str().unwrap_or_default().to_owned();
            }
            "formats" | "subtitles" | "webpage_url" => {}
            "_format_sort_fields" => {
                info.format_sort_fields = value
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect();
            }
            _ if value.is_null() => {
                info.extra.insert(key, Value::Null);
            }
            _ => crate::metadata_parser::set_field(&mut info, &key, value),
        }
    }
    if !titled {
        info.extra
            .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
    }
    crate::info_order::mark_info(
        info,
        &format!(
            "dict{};{};subtitle=ext,url;chapter=title,start_time",
            if titled { "" } else { "-untitled" },
            keys.join(",")
        ),
    )
}

impl Extractor for Vimeo {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some((video_id, unlisted_hash)) = match_url(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let login = |message: &str| {
            fail(
                &video_id,
                &format!(
                    "{message}. Use --cookies, --cookies-from-browser, --username and --password, --netrc-cmd, or --netrc (vimeo) to provide account credentials. See  https://github.com/yt-dlp/yt-dlp/wiki/FAQ#how-do-i-pass-cookies-to-yt-dlp  for how to manually pass cookies"
                ),
            )
        };
        // `_extract_from_api`: the default web client needs a login.
        if unlisted_hash.is_some() {
            return Err(login("The web client only works when logged-in"));
        }
        let mut page = url.clone();
        if ["play_redirect_hls", "moogaloop.swf"]
            .iter()
            .any(|marker| url.as_str().contains(marker))
        {
            page = Url::parse(&format!("https://vimeo.com/{video_id}"))?;
        }
        if page.scheme() == "https" {
            impersonation_warning();
        }
        let referer = url.as_str().to_owned();
        crate::screen::screen(&format!("[{NAME}] {video_id}: Downloading webpage"));
        let response =
            super::direct::request_with_headers(client, &page, None, &[("Referer", &referer)])
                .map_err(|error| match error {
                    Error::Http(_) | Error::HttpRetryAfter { .. } => Error::ExtractorRequest {
                        extractor: NAME,
                        id: video_id.clone(),
                        what: "webpage",
                        source: Box::new(error),
                    },
                    error => error,
                })?;
        let webpage = {
            use std::io::Read;
            let mut response = response;
            let mut body = Vec::new();
            response
                .read_to_end(&mut body)
                .map_err(Error::ExtractionRead)?;
            String::from_utf8_lossy(&body).into_owned()
        };
        if page.host_str() == Some("player.vimeo.com") {
            let config = match super::search_json(&PLAYER_CONFIG, &webpage, false) {
                Some(Some(config)) => config,
                _ => {
                    return Err(fail(
                        &video_id,
                        &format!(
                            "Unable to extract info section; {}",
                            crate::BUG_REPORTS_MESSAGE
                        ),
                    ));
                }
            };
            if config.get("view").and_then(Value::as_i64) == Some(4) {
                return Err(fail(
                    &video_id,
                    "This video is protected by a password, use the --video-password option",
                ));
            }
            let (info, formats, subtitles) = parse_config(client, &config, &video_id)?;
            return Ok(Some(video(url, info, formats, subtitles)));
        }
        if let Some(found) = VIMEO_CONFIG.captures(&webpage)
            && let Some(config) = found
                .get(1)
                .or(found.get(2))
                .and_then(|json| serde_json::from_str::<Value>(json.as_str()).ok())
            && config.pointer("/seed_status/state").and_then(Value::as_str) == Some("failed")
        {
            let title = config
                .pointer("/seed_status/title")
                .and_then(Value::as_str)
                .unwrap_or_default();
            return Err(fail(&video_id, &format!("{NAME} said: {title}")));
        }
        let mut info_dict = Map::new();
        let mut description: Option<String> = None;
        let mut config_url: Option<String> = None;
        static CHANNEL: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"vimeo\.com/channels/([^/?#]+)").unwrap());
        if let Some(found) = CHANNEL.captures(url.as_str()) {
            static CONFIG_URL: LazyLock<Regex> =
                LazyLock::new(|| Regex::new(r#"\bdata-config-url="([^"]+)""#).unwrap());
            config_url = CONFIG_URL
                .captures(&webpage)
                .map(|found| crate::html::unescape_html(&found[1]));
            let channel = found[1].to_owned();
            info_dict.insert("channel_id".into(), channel.clone().into());
            info_dict.insert(
                "channel_url".into(),
                format!("https://vimeo.com/channels/{channel}").into(),
            );
        }
        let mut timestamp: Option<String> = None;
        let mut license: Option<Value> = None;
        if config_url.is_none() {
            let page_config = PAGE_CONFIG
                .captures(&webpage)
                .and_then(|found| serde_json::from_str::<Value>(&found[1]).ok())
                .filter(|config| config.as_object().is_some_and(|config| !config.is_empty()));
            let Some(page_config) = page_config else {
                return Err(login("The web client only works when logged-in"));
            };
            config_url = page_config
                .pointer("/player/config_url")
                .and_then(Value::as_str)
                .map(str::to_owned);
            license = page_config
                .get("cc_license")
                .filter(|v| !v.is_null())
                .cloned();
            timestamp = page_config
                .pointer("/clip/uploaded_on")
                .and_then(Value::as_str)
                .map(str::to_owned);
            description = page_config
                .pointer("/clip/description")
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
                .or_else(|| {
                    page_config
                        .get("description_html_escaped")
                        .and_then(Value::as_str)
                })
                .map(crate::html::clean_html);
        }
        let config_url = Url::parse(config_url.as_deref().unwrap_or_default())?;
        let config = super::download_json(client, &config_url, NAME, &video_id)?;
        if description.as_deref().is_none_or(str::is_empty) {
            static DESCRIPTION: LazyLock<Regex> = LazyLock::new(|| {
                Regex::new(r#"(?s)<div\s+class="[^"]*description[^"]*"[^>]*>(.*?)</div>"#).unwrap()
            });
            description = DESCRIPTION
                .captures(&webpage)
                .map(|found| crate::html::clean_html(&found[1]))
                .filter(|text| !text.is_empty())
                .or_else(|| {
                    ["description", "og:description", "twitter:description"]
                        .iter()
                        .find_map(|name| crate::html::html_search_meta(&webpage, name))
                });
        }
        if description.as_deref().is_none_or(str::is_empty) {
            crate::screen::warning(&format!("[{NAME}] Cannot find video description"));
        }
        if timestamp.is_none() {
            static TIME: LazyLock<Regex> =
                LazyLock::new(|| Regex::new(r#"<time[^>]+datetime="([^"]+)""#).unwrap());
            timestamp = TIME.captures(&webpage).map(|found| found[1].to_owned());
        }
        let count = |pattern: &str| {
            Regex::new(pattern)
                .ok()?
                .captures(&webpage)
                .and_then(|found| found[1].parse::<i64>().ok())
        };
        if license.is_none() {
            static LICENSE: LazyLock<Regex> = LazyLock::new(|| {
                Regex::new(r#"<link[^>]+rel=["']license["'][^>]+href="([^"]+)""#).unwrap()
            });
            license = LICENSE
                .captures(&webpage)
                .map(|found| Value::from(found[1].to_owned()));
        }
        let (config_info, formats, subtitles) = parse_config(client, &config, &video_id)?;
        info_dict.insert(
            "_format_sort_fields".into(),
            config_info["_format_sort_fields"].clone(),
        );
        info_dict.insert("formats".into(), json!([]));
        info_dict.insert(
            "timestamp".into(),
            timestamp
                .as_deref()
                .and_then(|text| crate::dates::unified_timestamp(text, false))
                .into(),
        );
        info_dict.insert("description".into(), description.into());
        info_dict.insert("webpage_url".into(), url.as_str().into());
        info_dict.insert("view_count".into(), count(r"UserPlays:(\d+)").into());
        info_dict.insert("like_count".into(), count(r"UserLikes:(\d+)").into());
        info_dict.insert("comment_count".into(), count(r"UserComments:(\d+)").into());
        info_dict.insert("license".into(), license.unwrap_or(Value::Null));
        let dict = merge_dicts(vec![info_dict, config_info]);
        Ok(Some(video(url, dict, formats, subtitles)))
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        Ok(self.extract(client, url)?.map(ExtractResult::Video))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert_eq!(
            match_url("https://vimeo.com/76979871"),
            Some(("76979871".into(), None))
        );
        assert_eq!(
            match_url("https://player.vimeo.com/video/54469442"),
            Some(("54469442".into(), None))
        );
        assert_eq!(
            match_url("https://vimeo.com/123456789/abcdef0123"),
            Some(("123456789".into(), Some("abcdef0123".into())))
        );
        assert!(suitable("https://vimeo.com/channels/staffpicks/77810431"));
        assert!(suitable("https://vimeo.com/user7108434/my-clip"));
        assert!(!suitable("https://vimeo.com/channels/staffpicks"));
        assert!(!suitable(
            "https://vimeo.com/ondemand/thelastcolony/150274832"
        ));
        assert!(!suitable("https://vimeo.com/user7108434/videos"));
    }
}

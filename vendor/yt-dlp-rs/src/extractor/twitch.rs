//! Upstream `TwitchVodIE` and `TwitchClipsIE` (`yt_dlp/extractor/twitch.py`):
//! GraphQL metadata and playback access tokens, the usher HLS master (with
//! its storyboard) for VODs, and a clip's signed quality URLs.
use super::Extractor;
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct TwitchVod;
pub struct TwitchClips;

const VOD_NAME: &str = "twitch:vod";
const VOD_KEY: &str = "TwitchVod";
const CLIPS_NAME: &str = "twitch:clips";
const CLIPS_KEY: &str = "TwitchClips";
const CLIENT_ID: &str = "ue6666qo983tsx6so1t0vnawi233wa";

static VOD_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:(?:(?:www|go|m)\.)?twitch\.tv/(?:[^/]+/v(?:ideo)?|videos)/|player\.twitch\.tv/\?.*?\bvideo=v?|www\.twitch\.tv/[^/]+/schedule\?vodID=)(?P<id>\d+)")
        .unwrap()
});
static CLIPS_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:clips\.twitch\.tv/(?:embed\?.*?\bclip=|(?:[^/]+/)*)|(?:(?:www|go|m)\.)?twitch\.tv/(?:[^/]+/)?clip/)(?P<id>[^/?#&]+)")
        .unwrap()
});
static THUMBNAIL_SIZE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\d+x\d+(\.\w+)($|[?#])").unwrap());
static OLD_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"%7C(\d+)(?:-\d+)?.mp4").unwrap());

/// `TwitchVodIE.suitable`.
pub fn vod_suitable(url: &str) -> bool {
    VOD_URL.is_match(url)
}

/// `TwitchClipsIE.suitable`.
pub fn clips_suitable(url: &str) -> bool {
    CLIPS_URL.is_match(url)
}

fn operation_hash(name: &str) -> &'static str {
    match name {
        "ShareClipRenderStatus" => {
            "2db6a3b20eabf510bd3cf465ae2408834b59eb6b8af89ca73ab1486cacecfb63"
        }
        "VideoMetadata" => "45111672eea2e507f8ba44d101a61862f9c56b11dee09a15634cb75cb9b9084d",
        "VideoPlayer_ChapterSelectButtonVideo" => {
            "71835d5ef425e154bf282453a926d99b328cdc5e32f36d3a209d0f4778b41203"
        }
        "VideoPlayer_VODSeekbarPreviewVideo" => {
            "07e99e4d56c5a7c67117a154777b0baf85a5ffefa393b213f4bc712ccaf85dd6"
        }
        _ => "",
    }
}

fn int(value: Option<&Value>, scale: i64) -> Option<i64> {
    let value = value?;
    let number = value
        .as_f64()
        .or_else(|| value.as_str()?.trim().parse::<f64>().ok())?;
    Some((number / scale as f64) as i64)
}

fn fail(name: &str, id: &str, message: &str) -> Error {
    Error::Reported(format!("[{name}] {id}: {message}"))
}

/// `_download_base_gql(video_id, ops, note)`.
fn base_gql(
    client: &Client,
    name: &'static str,
    id: &str,
    ops: &Value,
    note: &str,
) -> Result<Value> {
    use std::io::Read;
    crate::screen::screen(&format!("[{name}] {id}: {note}"));
    let mut headers = vec![
        ("Content-Type", "text/plain;charset=UTF-8".to_owned()),
        ("Client-ID", CLIENT_ID.to_owned()),
    ];
    if let Some(token) = crate::cookies::get_cookie("https://gql.twitch.tv", "auth-token") {
        headers.push(("Authorization", format!("OAuth {token}")));
    }
    let headers: Vec<(&str, &str)> = headers
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect();
    let url = Url::parse("https://gql.twitch.tv/gql")?;
    let body = crate::output::python_json_dumps(ops);
    let mut response =
        super::direct::post(client, &url, &headers, body).map_err(|error| match error {
            Error::Http(_) | Error::HttpRetryAfter { .. } => Error::ExtractorRequest {
                extractor: name,
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
            "[{name}] {id}: Failed to parse JSON  (caused by {error})"
        ))
    })
}

/// `_download_gql(video_id, ops, note)`: persisted operations.
fn gql(
    client: &Client,
    name: &'static str,
    id: &str,
    ops: Vec<Value>,
    note: &str,
) -> Result<Value> {
    let ops: Vec<Value> = ops
        .into_iter()
        .map(|mut op| {
            let hash = operation_hash(op["operationName"].as_str().unwrap_or_default());
            if let Some(object) = op.as_object_mut() {
                object.insert(
                    "extensions".into(),
                    json!({"persistedQuery": {"version": 1, "sha256Hash": hash}}),
                );
            }
            op
        })
        .collect();
    base_gql(client, name, id, &Value::Array(ops), note)
}

/// A format from upstream's dict (`dict:KEYS` key order).
fn format_from(mut format: Map<String, Value>, id: &str, name: &str) -> Result<crate::VideoFormat> {
    let keys: Vec<String> = format.keys().cloned().collect();
    for key in ["height", "width", "fps", "aspect_ratio"] {
        if format.get(key).is_some_and(Value::is_null) {
            format.shift_remove(key);
        }
    }
    if !format.contains_key("ext") {
        let ext = format
            .get("url")
            .and_then(Value::as_str)
            .and_then(super::direct::determine_ext)
            .unwrap_or_else(|| "unknown_video".into());
        format.insert("ext".into(), ext.into());
    }
    let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(format))
        .map_err(|error| fail(name, id, &error.to_string()))?;
    crate::info_order::mark_format(&mut format, &format!("dict:{}", keys.join(",")));
    crate::info_order::mark_int_rates(&mut format);
    Ok(format)
}

/// A video from upstream's dict (`dict;KEYS` order).
fn video(
    url: &Url,
    dict: Map<String, Value>,
    formats: Vec<crate::VideoFormat>,
    name: &str,
    key: &str,
    thumbnail_keys: &str,
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
    info.webpage_url = url.clone();
    info.extractor = name.into();
    info.extractor_key = key.into();
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
    crate::info_order::mark_info(
        info,
        &format!(
            "dict;{};chapter=start_time,end_time,title{thumbnail_keys}",
            keys.join(",")
        ),
    )
}

/// `_get_thumbnails(thumbnail)`: the full-size image first.
fn thumbnails(thumbnail: Option<&str>) -> Value {
    match thumbnail {
        Some(thumbnail) => json!([
            {"url": THUMBNAIL_SIZE.replace(thumbnail, "0x0$1$2").into_owned(), "preference": 1},
            {"url": thumbnail},
        ]),
        None => Value::Null,
    }
}

impl Extractor for TwitchVod {
    fn key(&self) -> &'static str {
        VOD_NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VOD_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let vod_id = found["id"].to_owned();
        crate::screen::set_extractor(VOD_NAME);
        crate::screen::screen(&format!(
            "[{VOD_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        // `_download_info`.
        let data = gql(
            client,
            VOD_NAME,
            &vod_id,
            vec![
                json!({"operationName": "VideoMetadata", "variables": {"channelLogin": "", "videoID": vod_id}}),
                json!({"operationName": "VideoPlayer_ChapterSelectButtonVideo", "variables": {"includePrivate": false, "videoID": vod_id}}),
                json!({"operationName": "VideoPlayer_VODSeekbarPreviewVideo", "variables": {"includePrivate": false, "videoID": vod_id}}),
            ],
            "Downloading stream metadata GraphQL",
        )?;
        let responses: Vec<&Value> = data
            .as_array()
            .map(|items| items.iter().collect())
            .unwrap_or_default();
        let video = responses
            .iter()
            .find_map(|item| item.pointer("/data/video").filter(|video| !video.is_null()))
            .cloned()
            .ok_or_else(|| fail(VOD_NAME, &vod_id, &format!("Video {vod_id} does not exist")))?;
        let moments: Vec<Value> = responses
            .iter()
            .filter_map(|item| item.pointer("/data/video/moments/edges"))
            .filter_map(Value::as_array)
            .flatten()
            .filter_map(|edge| edge.get("node").cloned())
            .collect();
        let storyboard = responses
            .iter()
            .filter_map(|item| item.pointer("/data/video/seekPreviewsURL"))
            .filter_map(Value::as_str)
            .find_map(super::json_ld::url_or_none);
        // `_extract_info_gql`.
        let mut id = video
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .unwrap_or(&vod_id)
            .to_owned();
        if !id.starts_with('v') {
            id = format!("v{id}");
        }
        let mut thumbnail = video
            .get("previewThumbnailURL")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none);
        let mut is_live = Value::Null;
        if let Some(found) = &thumbnail {
            static PROCESSING: LazyLock<Regex> =
                LazyLock::new(|| Regex::new(r"/404_processing_[^.?#]+\.png").unwrap());
            if PROCESSING.is_match(found) {
                is_live =
                    (video.get("broadcastType").and_then(Value::as_str) == Some("ARCHIVE")).into();
                thumbnail = None;
            } else {
                is_live = false.into();
            }
        }
        let duration = int(video.get("lengthSeconds"), 1);
        let mut chapters = Vec::new();
        if moments.is_empty() {
            if let Some(game) = video
                .pointer("/game/displayName")
                .filter(|game| !game.is_null())
            {
                chapters.push(json!({"title": game}));
            }
        } else {
            for moment in &moments {
                let start = int(moment.get("positionMilliseconds"), 1000);
                let length = int(moment.get("durationMilliseconds"), 1000);
                let name = moment.get("description").and_then(Value::as_str);
                let (Some(start), Some(length)) = (start, length) else {
                    crate::screen::warning(&format!(
                        "[{VOD_NAME}] {vod_id}: Important chapter information missing for chapter {}",
                        name.unwrap_or("None")
                    ));
                    continue;
                };
                chapters
                    .push(json!({"start_time": start, "end_time": start + length, "title": name}));
            }
        }
        let mut info = Map::new();
        info.insert("id".into(), id.into());
        info.insert(
            "title".into(),
            video
                .get("title")
                .and_then(Value::as_str)
                .filter(|title| !title.is_empty())
                .unwrap_or("Untitled Broadcast")
                .into(),
        );
        info.insert(
            "description".into(),
            video.get("description").cloned().unwrap_or(Value::Null),
        );
        info.insert("duration".into(), duration.into());
        info.insert("thumbnails".into(), thumbnails(thumbnail.as_deref()));
        info.insert(
            "uploader".into(),
            video
                .pointer("/owner/displayName")
                .filter(|v| v.is_string())
                .cloned()
                .unwrap_or(Value::Null),
        );
        info.insert(
            "uploader_id".into(),
            video
                .pointer("/owner/login")
                .filter(|v| v.is_string())
                .cloned()
                .unwrap_or(Value::Null),
        );
        info.insert(
            "timestamp".into(),
            video
                .get("publishedAt")
                .and_then(Value::as_str)
                .and_then(|date| crate::dates::unified_timestamp(date, false))
                .into(),
        );
        info.insert("view_count".into(), int(video.get("viewCount"), 1).into());
        info.insert("chapters".into(), Value::Array(chapters));
        info.insert("is_live".into(), is_live);
        info.insert("was_live".into(), true.into());
        // `_download_access_token(vod_id, 'video', 'id')`.
        let query = format!(
            "{{\n              videoPlaybackAccessToken(\n                id: \"{vod_id}\",\n                params: {{\n                  platform: \"web\",\n                  playerBackend: \"mediaplayer\",\n                  playerType: \"site\"\n                }}\n              )\n              {{\n                value\n                signature\n              }}\n            }}"
        );
        let token = base_gql(
            client,
            VOD_NAME,
            &vod_id,
            &json!({ "query": query }),
            "Downloading video access token GraphQL",
        )?;
        let access = token
            .pointer("/data/videoPlaybackAccessToken")
            .cloned()
            .unwrap_or(Value::Null);
        // `_extract_twitch_m3u8_formats('vod', ...)`.
        let random = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1_000_000, |time| {
                1_000_000 + (time.subsec_nanos() as u64 % 9_000_000)
            });
        let mut usher = Url::parse(&format!("https://usher.ttvnw.net/vod/{vod_id}.m3u8"))?;
        usher
            .query_pairs_mut()
            .append_pair("allow_source", "true")
            .append_pair("allow_audio_only", "true")
            .append_pair("allow_spectre", "true")
            .append_pair("p", &random.to_string())
            .append_pair("platform", "web")
            .append_pair("player", "twitchweb")
            .append_pair("supported_codecs", "av1,h265,h264")
            .append_pair("playlist_include_framerate", "true")
            .append_pair(
                "sig",
                access
                    .get("signature")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            )
            .append_pair(
                "token",
                access
                    .get("value")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            );
        let stream = super::extract_m3u8_fatal(client, &usher, VOD_NAME, &vod_id, None, false)?;
        let mut formats = stream.formats;
        for format in &mut formats {
            if format
                .vcodec
                .as_deref()
                .is_some_and(|codec| codec.starts_with("av01"))
            {
                let options = format
                    .extra
                    .entry("downloader_options")
                    .or_insert_with(|| json!({}));
                if let Some(options) = options.as_object_mut() {
                    options.insert("ffmpeg_args_out".into(), json!(["-f", "mp4"]));
                }
            }
        }
        // `_extract_storyboard`.
        if let (Some(duration), Some(storyboard)) = (duration.filter(|d| *d != 0), &storyboard) {
            let spec = Url::parse(storyboard)
                .ok()
                .and_then(|spec| {
                    super::download_json_note(
                        client,
                        &spec,
                        VOD_NAME,
                        &vod_id,
                        "Downloading storyboard metadata JSON",
                    )
                    .ok()
                })
                .and_then(|spec| spec.as_array().cloned())
                .unwrap_or_default();
            let mut spec = spec;
            spec.sort_by_key(|entry| std::cmp::Reverse(int(entry.get("width"), 1).unwrap_or(0)));
            let base = Url::parse(storyboard)?.join(".")?;
            for (index, entry) in spec.iter().enumerate() {
                let count = int(entry.get("count"), 1).filter(|count| *count != 0);
                let images: Vec<&str> = entry
                    .get("images")
                    .and_then(Value::as_array)
                    .map(|images| images.iter().filter_map(Value::as_str).collect())
                    .unwrap_or_default();
                let (Some(count), false) = (count, images.is_empty()) else {
                    continue;
                };
                let fragment_duration = duration as f64 / images.len() as f64;
                let mut format = Map::new();
                format.insert("format_id".into(), format!("sb{index}").into());
                format.insert("format_note".into(), "storyboard".into());
                format.insert("ext".into(), "mhtml".into());
                format.insert("protocol".into(), "mhtml".into());
                format.insert("acodec".into(), "none".into());
                format.insert("vcodec".into(), "none".into());
                format.insert("url".into(), base.join(images[0])?.as_str().into());
                format.insert("width".into(), int(entry.get("width"), 1).into());
                format.insert("height".into(), int(entry.get("height"), 1).into());
                format.insert("fps".into(), json!(count as f64 / duration as f64));
                format.insert("rows".into(), int(entry.get("rows"), 1).into());
                format.insert("columns".into(), int(entry.get("cols"), 1).into());
                let fragments: Vec<Value> = images
                    .iter()
                    .filter_map(|path| base.join(path).ok())
                    .map(|url| json!({"url": url.as_str(), "duration": fragment_duration}))
                    .collect();
                format.insert("fragments".into(), Value::Array(fragments));
                formats.push(format_from(format, &vod_id, VOD_NAME)?);
            }
        }
        // `_prefer_source`.
        if let Some(source) = formats
            .iter_mut()
            .find(|format| format.format_id == "Source")
        {
            source.quality = Some(10.0);
            crate::info_order::mark_format_update(source, "quality");
            crate::info_order::mark_int_rates(source);
        } else {
            for format in formats
                .iter_mut()
                .filter(|format| format.url.as_str().contains("/chunked/"))
            {
                format.quality = Some(10.0);
                format.format_note = Some("Source".into());
                crate::info_order::mark_format_update(format, "quality");
                crate::info_order::mark_format_update(format, "format_note");
                crate::info_order::mark_int_rates(format);
            }
        }
        info.insert("formats".into(), json!([]));
        if let Some(t) = url
            .query_pairs()
            .find(|(key, value)| key == "t" && !value.is_empty())
            .map(|(_, value)| value.into_owned())
            && let Some(start) = crate::parse_duration(&t)
        {
            info.insert(
                "start_time".into(),
                match start {
                    crate::info::PyNumber::Int(value) => value.into(),
                    crate::info::PyNumber::Float(value) => json!(value),
                },
            );
        }
        Ok(Some(self::video(url, info, formats, VOD_NAME, VOD_KEY, "")))
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        Ok(self.extract(client, url)?.map(ExtractResult::Video))
    }
}

impl Extractor for TwitchClips {
    fn key(&self) -> &'static str {
        CLIPS_NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = CLIPS_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let slug = found["id"].to_owned();
        crate::screen::set_extractor(CLIPS_NAME);
        crate::screen::screen(&format!(
            "[{CLIPS_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let data = gql(
            client,
            CLIPS_NAME,
            &slug,
            vec![json!({"operationName": "ShareClipRenderStatus", "variables": {"slug": slug}})],
            "Downloading clip GraphQL",
        )?;
        let clip = data
            .pointer("/0/data/clip")
            .filter(|clip| !clip.is_null())
            .cloned()
            .ok_or_else(|| fail(CLIPS_NAME, &slug, "This clip is no longer available"))?;
        let signature = clip
            .pointer("/playbackAccessToken/signature")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let token = clip
            .pointer("/playbackAccessToken/value")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let signed = |source: &str| -> Option<String> {
            Url::parse(source).ok().map(|url| {
                super::update_url_query(&url, &[("sig", &signature), ("token", &token)]).to_string()
            })
        };
        let asset = |index: usize| {
            clip.pointer(&format!("/assets/{index}"))
                .filter(|asset| asset.is_object())
                .cloned()
                .unwrap_or_else(|| json!({}))
        };
        let (default, portrait) = (asset(0), asset(1));
        let float = |value: Option<&Value>| {
            value.and_then(|value| value.as_f64().or_else(|| value.as_str()?.parse().ok()))
        };
        let qualities = |asset: &Value| -> Vec<Value> {
            asset
                .get("videoQualities")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|quality| {
                    quality
                        .get("sourceURL")
                        .and_then(Value::as_str)
                        .and_then(super::json_ld::url_or_none)
                        .is_some()
                })
                .cloned()
                .collect()
        };
        let mut formats = Vec::new();
        let default_ratio = float(default.get("aspectRatio"));
        for quality in qualities(&default) {
            let mut format = Map::new();
            format.insert(
                "url".into(),
                signed(quality["sourceURL"].as_str().unwrap_or_default()).into(),
            );
            if let Some(id) = quality.get("quality").and_then(Value::as_str) {
                format.insert("format_id".into(), id.into());
            }
            if let Some(height) = int(quality.get("quality"), 1) {
                format.insert("height".into(), height.into());
            }
            if let Some(fps) = float(quality.get("frameRate")) {
                format.insert("fps".into(), json!(fps));
            }
            if let Some(ratio) = default_ratio {
                format.insert("aspect_ratio".into(), json!(ratio));
            }
            if !format.contains_key("format_id") {
                format.insert("format_id".into(), "".into());
            }
            formats.push(format_from(format, &slug, CLIPS_NAME)?);
        }
        let portrait_ratio = float(portrait.get("aspectRatio"));
        for quality in qualities(&portrait) {
            let mut format = Map::new();
            format.insert(
                "url".into(),
                signed(quality["sourceURL"].as_str().unwrap_or_default()).into(),
            );
            let label = quality
                .get("quality")
                .and_then(Value::as_str)
                .filter(|q| !q.is_empty());
            format.insert(
                "format_id".into(),
                label
                    .map_or_else(
                        || "portrait".to_owned(),
                        |label| format!("portrait-{label}"),
                    )
                    .into(),
            );
            format.insert("height".into(), int(quality.get("quality"), 1).into());
            format.insert(
                "fps".into(),
                float(quality.get("frameRate")).map_or(Value::Null, |fps| json!(fps)),
            );
            format.insert(
                "aspect_ratio".into(),
                portrait_ratio.map_or(Value::Null, |ratio| json!(ratio)),
            );
            format.insert("quality".into(), (-2).into());
            formats.push(format_from(format, &slug, CLIPS_NAME)?);
        }
        let mut thumbnails = Vec::new();
        let default_thumb = default
            .get("thumbnailURL")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none);
        if let Some(url) = &default_thumb {
            thumbnails.push(json!({"id": "default", "url": url, "preference": 0}));
        }
        if let Some(url) = portrait
            .get("thumbnailURL")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
        {
            thumbnails.push(json!({"id": "portrait", "url": url, "preference": -1}));
        }
        if let Some(url) = clip
            .get("thumbnailURL")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
            .filter(|url| Some(url) != default_thumb.as_ref())
        {
            thumbnails.push(json!({"id": "small", "url": url, "preference": -2}));
        }
        let old_id = formats
            .last()
            .and_then(|format| OLD_ID.captures(format.url.as_str()))
            .map(|found| found[1].to_owned());
        let mut info = Map::new();
        info.insert(
            "id".into(),
            clip.get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .unwrap_or(&slug)
                .into(),
        );
        info.insert(
            "_old_archive_ids".into(),
            old_id.map_or(Value::Null, |old| json!([format!("twitchclips {old}")])),
        );
        info.insert("display_id".into(), slug.clone().into());
        info.insert("formats".into(), json!([]));
        info.insert("thumbnails".into(), Value::Array(thumbnails));
        let text = |path: &str| {
            clip.pointer(path)
                .filter(|value| value.is_string())
                .cloned()
        };
        let mut put = |key: &str, value: Option<Value>| {
            if let Some(value) = value.filter(|value| !value.is_null()) {
                info.insert(key.to_owned(), value);
            }
        };
        put("title", text("/title"));
        put(
            "duration",
            int(clip.get("durationSeconds"), 1).map(Value::from),
        );
        put("view_count", int(clip.get("viewCount"), 1).map(Value::from));
        put(
            "timestamp",
            clip.get("createdAt")
                .and_then(Value::as_str)
                .and_then(crate::dates::parse_iso8601)
                .map(Value::from),
        );
        let broadcaster = clip
            .pointer("/broadcaster/displayName")
            .and_then(Value::as_str);
        put(
            "creators",
            Some(json!(
                broadcaster
                    .filter(|name| !name.is_empty())
                    .into_iter()
                    .collect::<Vec<_>>()
            )),
        );
        put("channel", text("/broadcaster/displayName"));
        put("channel_id", text("/broadcaster/id"));
        put(
            "channel_follower_count",
            int(clip.pointer("/broadcaster/followers/totalCount"), 1).map(Value::from),
        );
        put(
            "channel_is_verified",
            clip.pointer("/broadcaster/isPartner")
                .filter(|value| value.is_boolean())
                .cloned(),
        );
        put("uploader", text("/curator/displayName"));
        put("uploader_id", text("/curator/id"));
        let game = clip
            .pointer("/game/displayName")
            .and_then(Value::as_str)
            .filter(|game| !game.is_empty());
        if let Some(game) = game {
            put("categories", Some(json!([game])));
        }
        Ok(Some(video(
            url,
            info,
            formats,
            CLIPS_NAME,
            CLIPS_KEY,
            ";thumbnail=id,url,preference",
        )))
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
        assert!(vod_suitable(
            "http://www.twitch.tv/riotgames/v/6528877?t=5m10s"
        ));
        assert!(vod_suitable("https://www.twitch.tv/videos/6528877"));
        assert!(vod_suitable("https://player.twitch.tv/?video=480452374"));
        assert!(!vod_suitable("https://www.twitch.tv/riotgames"));
        assert!(clips_suitable(
            "https://clips.twitch.tv/FaintLightGullWholeWheat"
        ));
        assert!(clips_suitable(
            "https://www.twitch.tv/sergeynixon/clip/StormyThankfulSproutFutureMan"
        ));
        assert!(clips_suitable(
            "https://clips.twitch.tv/embed?clip=InquisitiveBreakableYogurtJebaited"
        ));
    }

    #[test]
    fn thumbnails_add_the_full_size_image() {
        assert_eq!(
            thumbnails(Some("https://x/a-320x180.jpg")),
            json!([{"url": "https://x/a-0x0.jpg", "preference": 1}, {"url": "https://x/a-320x180.jpg"}])
        );
    }
}

//! Upstream `TikTokIE` and `TikTokVMIE` (`yt_dlp/extractor/tiktok.py`): a
//! video's webpage universal data (solving the page's JS challenge like
//! `_solve_challenge_and_set_cookies`), `_parse_aweme_video_web`'s formats
//! and metadata, and short-link redirects.
use super::Extractor;
use crate::{Error, ExtractResult, Result, VideoInfo};
use base64::Engine;
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct TikTok;
pub struct TikTokVm;

const NAME: &str = "TikTok";
const VM_NAME: &str = "vm.tiktok";
const QUALITIES: [&str; 4] = ["360p", "540p", "720p", "1080p"];

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://www\.tiktokv?\.com/(?:embed|(?:share|@(?P<user_id>[\w\.-]+)?)/video)/(?P<id>\d+)",
    )
    .unwrap()
});
static VM_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:(?:vm|vt)\.tiktok\.com|(?:www\.)tiktok\.com/t)/(?P<id>\w+)").unwrap()
});
static UNIVERSAL_DATA: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<script[^>]+\bid="__UNIVERSAL_DATA_FOR_REHYDRATION__"[^>]*>"#).unwrap()
});
static URL_KEY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"v[^_]+_(?P<id>(?P<codec>[^_]+)_(?P<res>\d+p)_(?P<bitrate>\d+))").unwrap()
});
static ARTISTS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:, | & )").unwrap());

/// `TikTokIE._match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `TikTokVMIE._match_valid_url`.
pub fn vm_suitable(url: &str) -> bool {
    VM_URL.is_match(url)
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

fn login_required(id: &str, message: &str) -> Error {
    Error::Reported(format!(
        "[{NAME}] {id}: {message}. Use --cookies-from-browser or --cookies for the authentication. See  https://github.com/yt-dlp/yt-dlp/wiki/FAQ#how-do-i-pass-cookies-to-yt-dlp  for how to manually pass cookies"
    ))
}

/// `_proto_relative_url(url)`.
fn proto_relative(url: &str) -> String {
    match url.strip_prefix("//") {
        Some(rest) => format!("https://{rest}"),
        None => url.to_owned(),
    }
}

/// SHA-256 (FIPS 180-4), for the webpage's proof-of-work challenge.
fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut state: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut message = data.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for block in message.chunks(64) {
        let mut w = [0u32; 64];
        for (index, word) in block.chunks(4).enumerate() {
            w[index] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for index in 16..64 {
            let s0 = w[index - 15].rotate_right(7)
                ^ w[index - 15].rotate_right(18)
                ^ (w[index - 15] >> 3);
            let s1 = w[index - 2].rotate_right(17)
                ^ w[index - 2].rotate_right(19)
                ^ (w[index - 2] >> 10);
            w[index] = w[index - 16]
                .wrapping_add(s0)
                .wrapping_add(w[index - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
        for index in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[index])
                .wrapping_add(w[index]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, value) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *slot = slot.wrapping_add(value);
        }
    }
    let mut digest = [0u8; 32];
    for (chunk, word) in digest.chunks_mut(4).zip(state) {
        chunk.copy_from_slice(&word.to_be_bytes());
    }
    digest
}

/// `find_element(id=ID, html=True)`'s `class` attribute.
fn element_class(webpage: &str, id: &str) -> Option<String> {
    let element = crate::html_elements::element_html_by_id(id, webpage).ok()??;
    crate::html_elements::extract_attributes(&element)
        .get("class")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// `_solve_challenge_and_set_cookies`: the proof-of-work number whose
/// SHA-256 (after the base) is the expected digest, as the `wci` cookie.
fn solve_challenge(webpage: &str, id: &str) -> Result<Vec<String>> {
    let engine = base64::engine::general_purpose::STANDARD;
    let challenge: Option<Map<String, Value>> = element_class(webpage, "cs")
        .filter(|class| !class.is_empty())
        .and_then(|class| {
            // `f'{x}==='`: Python's decoder ignores the excess padding.
            let trimmed = class.trim_end_matches('=');
            let padded = format!("{trimmed}{}", "=".repeat((4 - trimmed.len() % 4) % 4));
            engine.decode(padded).ok()
        })
        .and_then(|bytes| serde_json::from_slice(&bytes).ok());
    let Some(mut challenge) = challenge else {
        if webpage.contains("Please wait...") {
            return Err(fail(id, "Unable to extract challenge data"));
        }
        return Err(fail(id, "Unexpected response from webpage request"));
    };
    crate::screen::screen(&format!(
        "[{NAME}] Solving JS challenge using native Python implementation"
    ));
    let field = |key: &str| {
        challenge
            .get("v")
            .and_then(|v| v.get(key))
            .and_then(Value::as_str)
            .and_then(|text| engine.decode(text).ok())
    };
    let expected =
        field("c").ok_or_else(|| fail(id, "Unable to extract challenge expected digest"))?;
    let base = field("a").ok_or_else(|| fail(id, "Unable to extract challenge base hash"))?;
    let mut solution = None;
    for number in 0..=1_000_000u32 {
        let mut data = base.clone();
        data.extend_from_slice(number.to_string().as_bytes());
        if sha256(&data)[..] == expected[..] {
            solution = Some(number);
            break;
        }
    }
    let number = solution.ok_or_else(|| fail(id, "Unable to solve JS challenge"))?;
    challenge.insert("d".into(), engine.encode(number.to_string()).into());
    let value = engine.encode(serde_json::to_string(&Value::Object(challenge)).unwrap_or_default());
    let wci = element_class(webpage, "wci")
        .ok_or_else(|| fail(id, "Unable to extract challenge cookie name"))?;
    crate::cookies::set_cookie(".tiktok.com", &wci, &value);
    let mut names = vec![wci];
    if let (Some(rci), Some(rs)) = (element_class(webpage, "rci"), element_class(webpage, "rs")) {
        crate::cookies::set_cookie(".tiktok.com", &rci, &rs);
        names.push(rci);
    }
    Ok(names)
}

/// `_get_universal_data`: `__DEFAULT_SCOPE__` of the rehydration data.
fn universal_data(webpage: &str) -> Option<Value> {
    match super::search_json(&UNIVERSAL_DATA, webpage, false) {
        Some(Some(data)) => data
            .get("__DEFAULT_SCOPE__")
            .filter(|scope| scope.is_object() && truthy(Some(scope)))
            .cloned(),
        _ => None,
    }
}

fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        Some(Value::Number(number)) => number.as_f64() != Some(0.0),
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(object)) => !object.is_empty(),
    }
}

/// `get_webpage(note)` with `impersonate=True` (unavailable here).
fn webpage(client: &Client, url: &Url, id: &str, note: &str) -> Result<String> {
    crate::screen::warning_once(&format!(
        "[{NAME}] The extractor is attempting impersonation, but no impersonate target is available. If you encounter errors, then see  https://github.com/yt-dlp/yt-dlp#impersonation  for information on installing the required dependencies"
    ));
    use std::io::Read;
    crate::screen::screen(&format!("[{NAME}] {id}: {note}"));
    let mut response = super::direct::request(client, url, None).map_err(|error| match error {
        Error::Http(_) | Error::HttpRetryAfter { .. } => Error::ExtractorRequest {
            extractor: NAME,
            id: id.to_owned(),
            what: "webpage",
            source: Box::new(error),
        },
        error => error,
    })?;
    let page_url = response.url().clone();
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    crate::screen::debug(&format!("Webpage size: {} bytes", body.len()));
    crate::screen::debug("Impersonation target: None");
    if page_url.path() == "/login" {
        return Err(login_required(
            id,
            "TikTok is requiring login for access to this content",
        ));
    }
    Ok(String::from_utf8_lossy(&body).into_owned())
}

/// `_extract_web_data_and_status(url, video_id)`.
fn web_data(client: &Client, url: &Url, id: &str) -> Result<(Value, i64)> {
    let page = webpage(client, url, id, "Downloading webpage")?;
    let mut data = universal_data(&page);
    if data.is_none() {
        let names = solve_challenge(&page, id)?;
        let page = webpage(client, url, id, "Downloading webpage with challenge cookie")?;
        for name in names {
            crate::cookies::clear_cookie(".tiktok.com", &name);
        }
        data = universal_data(&page);
    }
    let Some(data) = data else {
        return Err(fail(id, "Unable to extract universal data for rehydration"));
    };
    let detail = data.get("webapp.video-detail");
    let status = detail
        .and_then(|detail| detail.get("statusCode"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let video = detail
        .and_then(|detail| detail.pointer("/itemInfo/itemStruct"))
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or(Value::Null);
    if !video.get("video").is_some_and(Value::is_object)
        && video
            .get("isContentClassified")
            .and_then(Value::as_bool)
            .is_some()
    {
        return Err(login_required(
            id,
            "This post may not be comfortable for some audiences. Log in for access",
        ));
    }
    Ok((video, status))
}

/// `_parse_url_key`: the bitrate rendition's ID, codec, bitrate, quality.
fn url_key(key: &str) -> (Map<String, Value>, Option<String>) {
    let Some(found) = URL_KEY.captures(key) else {
        return (Map::new(), None);
    };
    let mut info = Map::new();
    info.insert("format_id".into(), found["id"].into());
    let codec = &found["codec"];
    info.insert(
        "vcodec".into(),
        if codec == "bytevc1" { "h265" } else { codec }.into(),
    );
    let tbr = found["bitrate"]
        .parse::<i64>()
        .ok()
        .map(|rate| rate / 1000)
        .filter(|rate| *rate != 0);
    info.insert("tbr".into(), tbr.into());
    let res = found["res"].to_owned();
    info.insert(
        "quality".into(),
        QUALITIES
            .iter()
            .position(|quality| *quality == res)
            .map_or(-1, |index| index as i64)
            .into(),
    );
    (info, Some(res))
}

/// `_extract_web_formats`.
fn web_formats(detail: &Value) -> Vec<Map<String, Value>> {
    let mut common = Map::new();
    common.insert("ext".into(), "mp4".into());
    common.insert("vcodec".into(), "h264".into());
    common.insert("acodec".into(), "aac".into());
    let video = detail
        .get("video")
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let play_width = int(video.get("width"));
    let play_height = int(video.get("height"));
    let ratio = match (play_width, play_height) {
        (Some(width), Some(height)) if height != 0 && width != 0 => width as f64 / height as f64,
        _ => 0.5625,
    };
    let mut formats: Vec<Map<String, Value>> = Vec::new();
    for bitrate in video
        .get("bitrateInfo")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|bitrate| {
            bitrate
                .pointer("/PlayAddr/UrlList")
                .is_some_and(|list| truthy(Some(list)))
        })
    {
        let (mut info, res) = url_key(
            bitrate
                .pointer("/PlayAddr/UrlKey")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        );
        let bytevc2 = info.get("vcodec").and_then(Value::as_str) == Some("bytevc2");
        info.insert(
            "format_note".into(),
            if bytevc2 {
                Value::from("UNPLAYABLE")
            } else {
                Value::Null
            },
        );
        info.insert("preference".into(), if bytevc2 { -100 } else { -1 }.into());
        info.insert(
            "filesize".into(),
            int(bitrate.pointer("/PlayAddr/DataSize")).into(),
        );
        if let Some(mut dimension) = res
            .as_deref()
            .and_then(|res| res.strip_suffix('p'))
            .and_then(|res| res.parse::<i64>().ok())
            .filter(|dimension| *dimension != 0)
        {
            if dimension == 540 {
                dimension = 576;
            }
            if ratio < 1.0 {
                let y = (dimension as f64 / ratio) as i64;
                info.insert("width".into(), dimension.into());
                info.insert("height".into(), (y - y % 2).into());
            } else {
                let x = (dimension as f64 * ratio) as i64;
                info.insert("width".into(), (x + x % 2).into());
                info.insert("height".into(), dimension.into());
            }
        }
        for url in bitrate
            .pointer("/PlayAddr/UrlList")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter_map(super::json_ld::url_or_none)
        {
            let mut format = common.clone();
            format.extend(info.clone());
            format.insert("url".into(), proto_relative(&url).into());
            if Url::parse(&url).is_ok_and(|parsed| parsed.path().ends_with("/media-video-hvc1/")) {
                format.insert("acodec".into(), "none".into());
                format.insert("__needs_testing".into(), true.into());
            }
            formats.push(format);
        }
    }
    let play_quality = formats
        .iter()
        .find(|format| int(format.get("width")) == play_width && play_width.is_some())
        .and_then(|format| format.get("quality").cloned())
        .unwrap_or(Value::Null);
    let play_urls: Vec<String> = match video.get("playAddr") {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.get("src").and_then(Value::as_str))
            .filter_map(super::json_ld::url_or_none)
            .collect(),
        Some(Value::String(url)) => super::json_ld::url_or_none(url).into_iter().collect(),
        _ => Vec::new(),
    };
    for url in play_urls {
        let mut format = common.clone();
        format.insert("format_id".into(), "play".into());
        format.insert("url".into(), proto_relative(&url).into());
        format.insert("width".into(), play_width.into());
        format.insert("height".into(), play_height.into());
        format.insert("quality".into(), play_quality.clone());
        formats.push(format);
    }
    for url in [video.get("downloadAddr"), video.pointer("/download/url")]
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(super::json_ld::url_or_none)
    {
        let mut format = common.clone();
        format.insert("format_id".into(), "download".into());
        format.insert("url".into(), proto_relative(&url).into());
        format.insert("format_note".into(), "watermarked".into());
        format.insert("preference".into(), (-2).into());
        format.insert("__needs_testing".into(), true.into());
        formats.push(format);
    }
    // `_remove_duplicate_formats`: the first format of each URL.
    let mut seen = Vec::new();
    formats.retain(|format| {
        let url = format.get("url").cloned();
        if seen.contains(&url) {
            return false;
        }
        seen.push(url);
        true
    });
    if let Some(audio) = detail
        .pointer("/music/playUrl")
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
    {
        let ext = Url::parse(&audio)
            .ok()
            .and_then(|parsed| {
                parsed
                    .query_pairs()
                    .filter(|(key, value)| key == "mime_type" && !value.is_empty())
                    .last()
                    .map(|(_, value)| value.replace('_', "/"))
            })
            .map(|mime| super::direct::mimetype2ext_or(&mime, None))
            .filter(|ext| !ext.is_empty())
            .unwrap_or_else(|| "m4a".into());
        let mut format = Map::new();
        format.insert("format_id".into(), "audio".into());
        format.insert("url".into(), proto_relative(&audio).into());
        format.insert("ext".into(), ext.clone().into());
        format.insert(
            "acodec".into(),
            if ext == "m4a" { "aac".to_owned() } else { ext }.into(),
        );
        format.insert("vcodec".into(), "none".into());
        formats.push(format);
    }
    formats.retain(|format| {
        format
            .get("url")
            .and_then(Value::as_str)
            .and_then(|url| Url::parse(url).ok())
            .is_none_or(|url| url.host_str() != Some("www.tiktok.com"))
    });
    formats
}

/// `_parse_aweme_video_web`'s info dict.
fn aweme_web(detail: &Value, webpage_url: &str, id: &str) -> Map<String, Value> {
    let mut info = Map::new();
    info.insert("id".into(), id.into());
    info.insert("formats".into(), Value::Null);
    info.insert("subtitles".into(), Value::Null);
    info.insert("http_headers".into(), json!({"Referer": webpage_url}));
    // `(('authorInfo', 'author', None), {...})`, the first non-empty match.
    let author = [detail.get("authorInfo"), detail.get("author"), Some(detail)]
        .into_iter()
        .flatten()
        .filter(|source| source.is_object())
        .map(|source| {
            let mut found = Map::new();
            let text = |keys: &[&str]| {
                keys.iter()
                    .find_map(|key| source.get(*key).and_then(Value::as_str))
                    .map(str::to_owned)
            };
            if let Some(value) = text(&["nickname"]) {
                found.insert("channel".into(), value.into());
            }
            if let Some(value) = text(&["authorSecId", "secUid"]) {
                found.insert("channel_id".into(), value.into());
            }
            if let Some(value) = text(&["uniqueId", "author"]) {
                found.insert("uploader".into(), value.into());
            }
            if let Some(value) =
                ["authorId", "uid", "id"]
                    .iter()
                    .find_map(|key| match source.get(*key)? {
                        Value::String(text) => Some(text.clone()),
                        Value::Number(number) => Some(number.to_string()),
                        _ => None,
                    })
            {
                found.insert("uploader_id".into(), value.into());
            }
            found
        })
        .find(|found| !found.is_empty())
        .unwrap_or_default();
    let channel_url = author
        .get("channel_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(|id| format!("https://www.tiktok.com/@{id}"));
    let uploader_url = ["uploader", "uploader_id"]
        .iter()
        .find_map(|key| {
            author
                .get(*key)
                .and_then(Value::as_str)
                .filter(|v| !v.is_empty())
        })
        .map(|name| format!("https://www.tiktok.com/@{name}"));
    info.extend(author);
    info.insert("channel_url".into(), channel_url.into());
    info.insert("uploader_url".into(), uploader_url.into());
    if let Some(music) = detail.get("music").filter(|music| music.is_object()) {
        if let Some(track) = music.get("title").and_then(Value::as_str) {
            info.insert("track".into(), track.into());
        }
        if let Some(album) = music
            .get("album")
            .and_then(Value::as_str)
            .filter(|album| !album.is_empty())
        {
            info.insert("album".into(), album.into());
        }
        if let Some(artists) = music
            .get("authorName")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
        {
            let artists: Vec<Value> = ARTISTS.split(artists).map(Value::from).collect();
            info.insert("artists".into(), Value::Array(artists));
        }
        if let Some(duration) = int(music.get("duration")) {
            info.insert("duration".into(), duration.into());
        }
    }
    if let Some(desc) = detail.get("desc").and_then(Value::as_str) {
        let chars: Vec<char> = desc.chars().collect();
        let title = if chars.len() <= 72 {
            desc.to_owned()
        } else {
            format!("{}...", chars[..69].iter().collect::<String>())
        };
        info.insert("title".into(), title.into());
        info.insert("description".into(), desc.into());
    }
    if let Some(duration) = int(detail.pointer("/video/duration")).filter(|d| *d != 0) {
        info.insert("duration".into(), duration.into());
    }
    if let Some(timestamp) = int(detail.get("createTime")) {
        info.insert("timestamp".into(), timestamp.into());
    }
    if let Some(stats) = detail.get("stats").filter(|stats| stats.is_object()) {
        for (key, source) in [
            ("view_count", "playCount"),
            ("like_count", "diggCount"),
            ("repost_count", "shareCount"),
            ("comment_count", "commentCount"),
            ("save_count", "collectCount"),
        ] {
            if let Some(count) = int(stats.get(source)) {
                info.insert(key.into(), count.into());
            }
        }
    }
    let mut thumbnails = Vec::new();
    for cover in ["thumbnail", "cover", "dynamicCover", "originCover"] {
        for source in [Some(detail), detail.get("video")].into_iter().flatten() {
            if let Some(url) = source
                .get(cover)
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
            {
                thumbnails.push(json!({
                    "id": cover,
                    "url": proto_relative(&url),
                    "preference": if cover == "dynamicCover" { -2 } else { -1 },
                }));
            }
        }
    }
    info.insert("thumbnails".into(), Value::Array(thumbnails));
    info
}

/// The result of upstream's dict (`dict;KEYS` order).
fn video(
    url: &Url,
    dict: Map<String, Value>,
    formats: Vec<Map<String, Value>>,
    id: &str,
) -> Result<VideoInfo> {
    let formats = formats
        .into_iter()
        .map(|format| {
            let keys: Vec<String> = format.keys().cloned().collect();
            let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(format))
                .map_err(|error| fail(id, &error.to_string()))?;
            crate::info_order::mark_format(&mut format, &format!("dict:{}", keys.join(",")));
            crate::info_order::mark_int_rates(&mut format);
            Ok(format)
        })
        .collect::<Result<Vec<_>>>()?;
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
    info.extractor = NAME.into();
    info.extractor_key = NAME.into();
    let mut keys = Vec::new();
    let mut titled = false;
    for (key, value) in dict {
        keys.push(key.clone());
        match key.as_str() {
            "id" => info.id = value.as_str().unwrap_or_default().to_owned(),
            "title" => {
                titled = true;
                info.title = value.as_str().unwrap_or_default().to_owned();
            }
            "formats" | "subtitles" => {}
            _ if value.is_null() => {
                info.extra.insert(key, Value::Null);
            }
            _ => crate::metadata_parser::set_field(&mut info, &key, value),
        }
    }
    if !titled {
        info.title = String::new();
        info.extra
            .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
    }
    Ok(crate::info_order::mark_info(
        info,
        &format!(
            "dict{};{};thumbnail=id,url,preference",
            if titled { "" } else { "-untitled" },
            keys.join(",")
        ),
    ))
}

impl Extractor for TikTok {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        let user = found
            .name("user_id")
            .map_or("_", |user| user.as_str())
            .to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        // `_create_url(user_id, video_id)`.
        let page = Url::parse(&format!("https://www.tiktok.com/@{user}/video/{id}"))?;
        let (detail, status) = web_data(client, &page, &id)?;
        if truthy(Some(&detail)) && status == 0 {
            let dict = aweme_web(&detail, page.as_str(), &id);
            let formats = web_formats(&detail);
            // Processing's `webpage_url` is the input URL.
            return video(url, dict, formats, &id).map(Some);
        }
        Err(match status {
            10216 | 10222 => login_required(
                &id,
                "You do not have permission to view this post. Log into an account that has access",
            ),
            10204 => fail(&id, "Your IP address is blocked from accessing this post"),
            _ => fail(&id, &format!("Video not available, status code {status}")),
        })
    }
}

impl Extractor for TikTokVm {
    fn key(&self) -> &'static str {
        VM_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VM_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        crate::screen::set_extractor(VM_NAME);
        crate::screen::screen(&format!(
            "[{VM_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        // `_request_webpage(HEADRequest(url), headers={'User-Agent':
        // 'facebookexternalhit/1.1'}).url`.
        crate::screen::screen(&format!("[{VM_NAME}] {id}: Downloading webpage"));
        let response = super::direct::head_with_headers(
            client,
            url,
            &[("User-Agent", "facebookexternalhit/1.1")],
        )
        .map_err(|error| match error {
            Error::Http(_) | Error::HttpRetryAfter { .. } => Error::ExtractorRequest {
                extractor: VM_NAME,
                id: id.clone(),
                what: "webpage",
                source: Box::new(error),
            },
            error => error,
        })?;
        let target = response.url().clone();
        if vm_suitable(target.as_str()) {
            return Err(Error::Reported(format!("Unsupported URL: {target}")));
        }
        let mut info = super::direct::build_at(target.clone(), &target, String::new());
        info.formats.clear();
        info.direct = None;
        info.extra
            .insert(crate::FOLLOW_URL.into(), target.as_str().into());
        Ok(Some(ExtractResult::Video(info)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.tiktok.com/@leenabhushan/video/6748451240264420610"
        ));
        assert!(suitable("https://www.tiktok.com/embed/6748451240264420610"));
        assert!(suitable(
            "https://www.tiktokv.com/share/video/6748451240264420610"
        ));
        assert!(!suitable("https://www.tiktok.com/@leenabhushan"));
        assert!(vm_suitable("https://vm.tiktok.com/ZTR45GpSF/"));
        assert!(vm_suitable("https://www.tiktok.com/t/ZTRC5xgJp"));
    }

    #[test]
    fn sha256_matches_known_digests() {
        let hex = |bytes: [u8; 32]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
        assert_eq!(
            hex(sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hex(sha256(&[b'a'; 1000])),
            "41edece42d63e8d9bf515a9ba6932e1c20cbc9f5a5d134645adb5db1b9737ea3"
        );
    }
}

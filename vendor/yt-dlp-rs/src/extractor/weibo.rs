//! Upstream `WeiboIE` and `WeiboVideoIE` (`yt_dlp/extractor/weibo.py`): the
//! `ajax/statuses/show` JSON (after the passport visitor handshake when
//! redirected), its playback list formats (or the media URLs' fallback),
//! mixed-media posts as playlists, and `tv/show` pages resolved through the
//! TV component API to their post.
use super::Extractor;
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Weibo;
pub struct WeiboVideo;

const NAME: &str = "Weibo";
const KEY: &str = "Weibo";
const VIDEO_NAME: &str = "WeiboVideo";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:m\.weibo\.cn/(?:status|detail)|(?:www\.)?weibo\.com/\d+)/(?P<id>[a-zA-Z0-9]+)")
        .unwrap()
});
static VIDEO_URLS: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    let id = r"\d+:(?:[\da-f]{32}|\d{16,})";
    [
        Regex::new(&format!(
            r"^https?://(?:www\.)?weibo\.com/tv/show/(?P<id>{id})"
        ))
        .unwrap(),
        Regex::new(&format!(
            r"^https?://video\.weibo\.com/show/?\?(?:[^#]+&)?fid=(?P<id>{id})"
        ))
        .unwrap(),
    ]
});
static CHROME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"Chrome/(\d+)").unwrap());
static FALLBACK_LABEL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"label=(\w+)&template=(\d+x\d+)").unwrap());

/// `WeiboIE.suitable`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `WeiboVideoIE.suitable`.
pub fn video_suitable(url: &str) -> bool {
    VIDEO_URLS.iter().any(|pattern| pattern.is_match(url))
}

fn fail(extractor: &str, id: &str, message: &str) -> Error {
    Error::Reported(format!("[{extractor}] {id}: {message}"))
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

/// Upstream `strip_jsonp`: the callback's argument.
fn strip_jsonp(code: &str) -> String {
    let trimmed = code.trim_start();
    let body = trimmed.strip_prefix("window.").unwrap_or(trimmed);
    let name_end = body
        .find(|c: char| !(c.is_ascii_alphanumeric() || "_.$".contains(c)))
        .unwrap_or(body.len());
    let rest = body[name_end..].trim_start();
    let rest = match rest.strip_prefix("&&") {
        Some(after) => {
            let after = after.trim_start();
            after
                .strip_prefix(&body[..name_end])
                .unwrap_or(after)
                .trim_start()
        }
        None => rest,
    };
    let Some(inner) = rest.strip_prefix('(') else {
        return code.to_owned();
    };
    let Some(end) = inner.rfind(')') else {
        return code.to_owned();
    };
    inner[..end].trim_start().to_owned()
}

/// A request's page and final URL (`_download_webpage_handle`).
fn fetch(
    client: &Client,
    extractor: &'static str,
    url: &Url,
    id: &str,
    note: &str,
    headers: &[(&str, &str)],
    data: Option<String>,
) -> Result<(String, Url)> {
    use std::io::Read;
    crate::screen::screen(&format!("[{extractor}] {id}: {note}"));
    let response = match data {
        Some(body) => super::direct::post(client, url, headers, body),
        None => super::direct::request_with_headers(client, url, None, headers),
    };
    let mut response = response.map_err(|error| match error {
        Error::Http(_) | Error::HttpRetryAfter { .. } => Error::ExtractorRequest {
            extractor,
            id: id.to_owned(),
            what: "webpage",
            source: Box::new(error),
        },
        error => error,
    })?;
    let final_url = response.url().clone();
    let mut bytes = Vec::new();
    response
        .read_to_end(&mut bytes)
        .map_err(Error::ExtractionRead)?;
    crate::screen::expect_page(id, None, None);
    crate::screen::page_read(&final_url, &bytes)?;
    Ok((String::from_utf8_lossy(&bytes).into_owned(), final_url))
}

/// `_update_visitor_cookies(visitor_url, video_id)`.
fn update_visitor_cookies(
    client: &Client,
    extractor: &'static str,
    visitor_url: &Url,
    id: &str,
) -> Result<()> {
    let agent = super::user_agent();
    let chrome = CHROME
        .captures(&agent)
        .map_or_else(|| "90".to_owned(), |found| found[1].to_owned());
    let fingerprint = serde_json::to_string(&json!({
        "os": "1",
        "browser": format!("Chrome{chrome},0,0,0"),
        "fonts": "undefined",
        "screenInfo": "1920*1080*24",
        "plugins": "",
    }))
    .unwrap_or_default();
    let body = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("cb", "gen_callback")
        .append_pair("fp", &fingerprint)
        .finish();
    let referer = visitor_url.as_str();
    let headers = [
        ("Referer", referer),
        ("Content-Type", "application/x-www-form-urlencoded"),
    ];
    let generate = Url::parse("https://passport.weibo.com/visitor/genvisitor")?;
    let (text, _) = fetch(
        client,
        extractor,
        &generate,
        id,
        "Generating first-visit guest request",
        &headers,
        Some(body),
    )?;
    let visitor: Value = serde_json::from_str(&strip_jsonp(&text)).map_err(|error| {
        fail(
            extractor,
            id,
            &format!("Failed to parse JSON  (caused by {error})"),
        )
    })?;
    let bug = |key: &str| {
        fail(
            extractor,
            id,
            &format!(
                "An extractor error has occurred. (caused by KeyError('{key}')); {}",
                crate::BUG_REPORTS_MESSAGE
            ),
        )
    };
    let data = visitor.get("data").ok_or_else(|| bug("data"))?;
    let tid = str_or_none(data.get("tid")).ok_or_else(|| bug("tid"))?;
    let new_tid = match data.get("new_tid") {
        None | Some(Value::Null) | Some(Value::Bool(false)) => false,
        Some(Value::Number(number)) => number.as_f64() != Some(0.0),
        Some(Value::String(text)) => !text.is_empty(),
        Some(_) => true,
    };
    let confidence = int(data.get("confidence")).unwrap_or(100);
    let random = {
        use std::hash::{BuildHasher, Hasher};
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u64(0);
        (hasher.finish() >> 11) as f64 / (1u64 << 53) as f64
    };
    let callback = super::update_url_query(
        &Url::parse("https://passport.weibo.com/visitor/visitor")?,
        &[
            ("a", "incarnate"),
            ("t", &tid),
            ("w", if new_tid { "3" } else { "2" }),
            ("c", &format!("{confidence:03}")),
            ("gc", ""),
            ("cb", "cross_domain"),
            ("from", "weibo"),
            ("_rand", &random.to_string()),
        ],
    );
    fetch(
        client,
        extractor,
        &callback,
        id,
        "Running first-visit callback to get guest cookies",
        &[("Referer", referer)],
        None,
    )?;
    Ok(())
}

/// `_weibo_download_json(url, video_id, ...)`.
fn weibo_json(
    client: &Client,
    extractor: &'static str,
    url: &Url,
    id: &str,
    referer: Option<&str>,
    data: Option<String>,
) -> Result<Value> {
    let mut headers = vec![("Referer", referer.unwrap_or("https://weibo.com/"))];
    if data.is_some() {
        headers.push(("Content-Type", "application/x-www-form-urlencoded"));
    }
    let note = "Downloading JSON metadata";
    let (mut text, final_url) = fetch(client, extractor, url, id, note, &headers, data.clone())?;
    if final_url.host_str() == Some("passport.weibo.com") {
        update_visitor_cookies(client, extractor, &final_url, id)?;
        text = fetch(client, extractor, url, id, note, &headers, data)?.0;
    }
    serde_json::from_str(&text).map_err(|error| {
        fail(
            extractor,
            id,
            &format!("{id}: Failed to parse JSON  (caused by {error})"),
        )
    })
}

/// `_extract_formats(video_info)`.
fn extract_formats(video_info: &Value, id: &str) -> Result<Vec<crate::VideoFormat>> {
    let media_info = video_info.pointer("/page_info/media_info");
    let mut formats: Vec<Map<String, Value>> = Vec::new();
    for item in media_info
        .and_then(|media| media.get("playback_list"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(play) = item.get("play_info") else {
            continue;
        };
        if play
            .get("url")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
            .is_none()
        {
            continue;
        }
        let mut format = Map::new();
        format.insert("url".into(), play["url"].clone());
        let text = |key: &str| play.get(key).filter(|value| value.is_string()).cloned();
        if let Some(value) = text("quality_desc") {
            format.insert("format".into(), value);
        }
        if let Some(value) = text("label") {
            format.insert("format_id".into(), value);
        }
        if let Some(mime) = play.get("mime").and_then(Value::as_str) {
            format.insert(
                "ext".into(),
                super::direct::mimetype2ext_or(mime, None).into(),
            );
        }
        if let Some(tbr) = int(play.get("bitrate")).filter(|tbr| *tbr != 0) {
            format.insert("tbr".into(), tbr.into());
        }
        if let Some(value) = text("video_codecs") {
            format.insert("vcodec".into(), value);
        }
        for (key, source) in [
            ("fps", "fps"),
            ("width", "width"),
            ("height", "height"),
            ("filesize", "size"),
        ] {
            if let Some(number) = int(play.get(source)) {
                format.insert(key.into(), number.into());
            }
        }
        if let Some(value) = text("audio_codecs") {
            format.insert("acodec".into(), value);
        }
        for (key, source) in [
            ("asr", "audio_sample_rate"),
            ("audio_channels", "audio_channels"),
        ] {
            if let Some(number) = int(play.get(source)) {
                format.insert(key.into(), number.into());
            }
        }
        formats.push(format);
    }
    if formats.is_empty() {
        // Upstream iterates a `set` of the media URLs (unordered).
        let mut urls: Vec<String> = Vec::new();
        for value in media_info
            .and_then(Value::as_object)
            .into_iter()
            .flat_map(|media| media.values())
        {
            if let Some(url) = value.as_str().and_then(super::json_ld::url_or_none)
                && !urls.contains(&url)
            {
                urls.push(url);
            }
        }
        for url in urls.into_iter().filter(|url| url.contains("label=")) {
            let found = FALLBACK_LABEL.captures(&url);
            let format_id = found.as_ref().map(|found| found[1].to_owned());
            let resolution = found.as_ref().map(|found| found[2].to_owned());
            let mut format = Map::new();
            format.insert("url".into(), url.clone().into());
            format.insert(
                "format_id".into(),
                format_id.clone().map_or(Value::Null, Value::from),
            );
            if let Some(resolution) = &resolution {
                let (width, height) = super::html5::parse_resolution(resolution);
                if let Some(width) = width {
                    format.insert("width".into(), width.into());
                }
                if let Some(height) = height {
                    format.insert("height".into(), height.into());
                }
            }
            if let Some(format_id) = &format_id
                && let Some(details) = media_info
                    .and_then(|media| media.get("video_details"))
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .find(|details| {
                        details
                            .get("label")
                            .and_then(Value::as_str)
                            .is_some_and(|label| label.starts_with(format_id.as_str()))
                    })
            {
                if let Some(size) = int(details.get("size")) {
                    format.insert("filesize".into(), size.into());
                }
                if let Some(tbr) = int(details.get("bitrate")) {
                    format.insert("tbr".into(), tbr.into());
                }
            }
            formats.push(format);
        }
    }
    formats
        .into_iter()
        .map(|mut format| {
            let keys: Vec<String> = format.keys().cloned().collect();
            format.retain(|_, value| !value.is_null());
            if !format.contains_key("ext") {
                let ext = format
                    .get("url")
                    .and_then(Value::as_str)
                    .and_then(super::direct::determine_ext)
                    .unwrap_or_else(|| "unknown_video".into());
                format.insert("ext".into(), ext.into());
            }
            if !format.contains_key("format_id") {
                format.insert("format_id".into(), "".into());
            }
            let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(format))
                .map_err(|error| fail(NAME, id, &error.to_string()))?;
            crate::info_order::mark_format(&mut format, &format!("dict:{}", keys.join(",")));
            crate::info_order::mark_int_rates(&mut format);
            Ok(format)
        })
        .collect()
}

/// `truncate_string(s, left=72)`.
fn truncate(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= 72 {
        return text.to_owned();
    }
    format!("{}...", chars[..69].iter().collect::<String>())
}

/// `_parse_video_info(video_info)`.
fn parse_video_info(video_info: &Value, page: &Url, temp_id: &str) -> Result<VideoInfo> {
    let video_id = ["id", "id_str", "mid"]
        .iter()
        .find_map(|key| str_or_none(video_info.get(*key)));
    let formats = extract_formats(video_info, temp_id)?;
    let mut dict = Map::new();
    dict.insert(
        "id".into(),
        video_id.clone().map_or(Value::Null, Value::from),
    );
    dict.insert("extractor_key".into(), KEY.into());
    dict.insert("extractor".into(), KEY.into());
    dict.insert("formats".into(), json!([]));
    dict.insert(
        "http_headers".into(),
        json!({"Referer": "https://weibo.com/"}),
    );
    dict.insert(
        "_old_archive_ids".into(),
        json!([format!(
            "weibomobile {}",
            video_id.as_deref().unwrap_or("None")
        )]),
    );
    let media = video_info.pointer("/page_info/media_info");
    let titles: Vec<&str> = ["video_title", "kol_title", "name"]
        .iter()
        .filter_map(|key| {
            media
                .and_then(|media| media.get(*key))
                .and_then(Value::as_str)
        })
        .collect();
    let mut put = |key: &str, value: Option<Value>| {
        if let Some(value) = value {
            dict.insert(key.into(), value);
        }
    };
    put(
        "display_id",
        str_or_none(video_info.get("mblogid")).map(Value::from),
    );
    put(
        "title",
        titles
            .iter()
            .map(|title| truncate(&title.replace('\n', " ")))
            .find(|title| !title.is_empty())
            .map(Value::from),
    );
    put(
        "alt_title",
        titles
            .iter()
            .find(|title| !title.is_empty())
            .map(|title| Value::from(*title)),
    );
    put(
        "description",
        video_info
            .get("text_raw")
            .filter(|text| text.is_string())
            .cloned(),
    );
    put(
        "duration",
        int(media.and_then(|media| media.get("duration"))).map(Value::from),
    );
    put(
        "timestamp",
        int(media.and_then(|media| media.get("video_publish_time"))).map(Value::from),
    );
    put(
        "thumbnail",
        video_info
            .pointer("/page_info/page_pic")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
            .map(Value::from),
    );
    put(
        "uploader",
        video_info
            .pointer("/user/screen_name")
            .filter(|name| name.is_string())
            .cloned(),
    );
    put(
        "uploader_id",
        ["id", "id_str"]
            .iter()
            .find_map(|key| str_or_none(video_info.pointer(&format!("/user/{key}"))))
            .map(Value::from),
    );
    put(
        "uploader_url",
        video_info
            .pointer("/user/profile_url")
            .and_then(Value::as_str)
            .filter(|path| !path.is_empty())
            .and_then(|path| Url::parse("https://weibo.com/").ok()?.join(path).ok())
            .map(|url| Value::from(url.as_str())),
    );
    put(
        "view_count",
        int(media.and_then(|media| media.get("online_users_number"))).map(Value::from),
    );
    put(
        "like_count",
        int(video_info.get("attitudes_count")).map(Value::from),
    );
    put(
        "repost_count",
        int(video_info.get("reposts_count")).map(Value::from),
    );
    let tags: Vec<Value> = video_info
        .get("topic_struct")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|topic| topic.get("topic_title").filter(|title| title.is_string()))
        .cloned()
        .collect();
    dict.insert(
        "tags".into(),
        if tags.is_empty() {
            Value::Null
        } else {
            Value::Array(tags)
        },
    );

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
    info.extractor = KEY.into();
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
            "formats" | "extractor" | "extractor_key" => {}
            _ if value.is_null() => {
                info.extra.insert(key, Value::Null);
            }
            _ => crate::metadata_parser::set_field(&mut info, &key, value),
        }
    }
    Ok(crate::info_order::mark_info(
        info,
        &format!(
            "dict{};{}",
            if titled { "" } else { "-untitled" },
            keys.join(",")
        ),
    ))
}

fn start(extractor: &'static str, page: &Url) {
    crate::screen::set_extractor(extractor);
    crate::screen::screen(&format!(
        "[{extractor}] Extracting URL: {}",
        crate::screen::extracting_url(page.as_str())
    ));
}

impl Extractor for Weibo {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, page: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(page.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        start(KEY, page);
        let api = super::update_url_query(
            &Url::parse("https://weibo.com/ajax/statuses/show")?,
            &[("id", &video_id)],
        );
        let meta = weibo_json(client, NAME, &api, &video_id, None, None)?;
        let items: Vec<Value> = meta
            .pointer("/mix_media_info/items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if items.is_empty() {
            return Ok(Some(ExtractResult::Video(parse_video_info(
                &meta, page, &video_id,
            )?)));
        }
        let mut entries = Vec::new();
        for item in items
            .iter()
            .filter(|item| item.get("type").and_then(Value::as_str) != Some("pic"))
        {
            let mut entry = Map::new();
            if let Some(id) = item.pointer("/data/object_id") {
                entry.insert("id".into(), id.clone());
            }
            let media = item
                .pointer("/data/media_info")
                .filter(|media| media.is_object())
                .cloned();
            entry.insert(
                "page_info".into(),
                match media {
                    Some(media) => json!({"media_info": media}),
                    None => json!({}),
                },
            );
            entries.push(parse_video_info(&Value::Object(entry), page, &video_id)?);
        }
        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            "playlist-dict;id".into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: video_id,
            title: String::new(),
            webpage_url: page.clone(),
            extractor: KEY.into(),
            extractor_key: KEY.into(),
            entries,
            extra,
        })))
    }
}

impl Extractor for WeiboVideo {
    fn key(&self) -> &'static str {
        VIDEO_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, page: &Url) -> Result<Option<ExtractResult>> {
        let Some(video_id) = VIDEO_URLS
            .iter()
            .find_map(|pattern| pattern.captures(page.as_str()))
            .map(|found| found["id"].to_owned())
        else {
            return Ok(None);
        };
        start(VIDEO_NAME, page);
        let api = super::update_url_query(
            &Url::parse("https://weibo.com/tv/api/component")?,
            &[("page", &format!("/tv/show/{video_id}"))],
        );
        let body = format!(r#"data={{"Component_Play_Playinfo":{{"oid":"{video_id}"}}}}"#);
        let info = weibo_json(
            client,
            VIDEO_NAME,
            &api,
            &video_id,
            Some(page.as_str()),
            Some(body),
        )?;
        let bug = |key: &str| {
            fail(
                VIDEO_NAME,
                &video_id,
                &format!(
                    "An extractor error has occurred. (caused by KeyError('{key}')); {}",
                    crate::BUG_REPORTS_MESSAGE
                ),
            )
        };
        let play = info
            .get("data")
            .ok_or_else(|| bug("data"))?
            .get("Component_Play_Playinfo")
            .ok_or_else(|| bug("Component_Play_Playinfo"))?;
        let mid = str_or_none(play.get("mid")).ok_or_else(|| bug("mid"))?;
        let target = Url::parse(&format!("https://weibo.com/0/{mid}"))?;
        let mut follow = super::direct::build_at(page.clone(), page, String::new());
        follow.formats.clear();
        follow.direct = None;
        follow
            .extra
            .insert(crate::FOLLOW_URL.into(), target.as_str().into());
        Ok(Some(ExtractResult::Video(follow)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_urls() {
        assert!(suitable("https://weibo.com/7827771738/N4xlMvjhI"));
        assert!(suitable("https://m.weibo.cn/status/4189191225395228"));
        assert!(!suitable("https://weibo.com/u/2568101057"));
        assert!(video_suitable(
            "https://weibo.com/tv/show/1034:4797699866951785?from=old_pc_videoshow"
        ));
        assert!(video_suitable(
            "https://video.weibo.com/show?fid=1034:4967272104787984"
        ));
    }

    #[test]
    fn jsonp_is_stripped() {
        assert_eq!(
            strip_jsonp("window.gen_callback && gen_callback({\"a\": 1});"),
            "{\"a\": 1}"
        );
        assert_eq!(strip_jsonp("cb( [1] )"), "[1] ");
    }
}

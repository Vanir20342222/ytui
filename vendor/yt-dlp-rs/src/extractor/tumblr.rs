//! Upstream `TumblrIE` (`yt_dlp/extractor/tumblr.py`): the anonymous API
//! token from the login page, the post page and its permalink API JSON,
//! Tumblr-hosted video (iframe sources or the post's media), external
//! provider embeds as `url` results, and the post metadata.
use super::Extractor;
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::{LazyLock, Mutex};
use url::Url;

pub struct Tumblr;

const NAME: &str = "Tumblr";
const LOGIN_URL: &str = "https://www.tumblr.com/login";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?P<blog1>[^/?#&]+)\.tumblr\.com/(?:post|video|(?P<blog2>[a-zA-Z\d-]+))/(?P<id>[0-9]+)(?:$|[/?#])")
        .unwrap()
});
static API_TOKEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#""API_TOKEN":\s*"(\w+)""#).unwrap());
static TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<title>(?P<title>.*?)(?: \| Tumblr)?</title>").unwrap());

/// `TumblrIE.suitable`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_providers`: the extractor of each known embed provider.
fn provider_key(provider: &str) -> Option<Option<&'static str>> {
    Some(match provider {
        "instagram" => Some("Instagram"),
        "vimeo" => Some("Vimeo"),
        "youtube" => Some("Youtube"),
        "dailymotion" => Some("Dailymotion"),
        "tiktok" => Some("TikTok"),
        "twitch_live" => Some("TwitchStream"),
        "bandcamp" | "soundcloud" => None,
        _ => return None,
    })
}

fn fail(id: &str, message: &str) -> Error {
    Error::Reported(format!("[{NAME}] {id}: {message}"))
}

fn int(value: Option<&Value>) -> Option<i64> {
    let value = value?;
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|number| number as i64))
        .or_else(|| value.as_str()?.trim().parse().ok())
}

/// `_initialize_pre_login`: the login page's anonymous API token, once.
fn access_token(client: &Client) -> Option<String> {
    static TOKEN: Mutex<Option<Option<String>>> = Mutex::new(None);
    if let Some(token) = TOKEN.lock().ok().and_then(|token| token.clone()) {
        return token;
    }
    crate::screen::screen(&format!("[{NAME}] Downloading login page"));
    let token = Url::parse(LOGIN_URL)
        .ok()
        .and_then(|login| super::download_webpage_quiet(client, &login, NAME, "").ok())
        .and_then(|page| {
            let found = API_TOKEN.captures(&page).map(|found| found[1].to_owned());
            if found.is_none() {
                super::search_warning(NAME, "API access token");
            }
            found
        });
    if token.is_none() {
        crate::screen::warning(&format!(
            "[{NAME}] Failed to get access token; metadata will be missing and some videos may not work"
        ));
    }
    if let Ok(mut slot) = TOKEN.lock() {
        *slot = Some(token.clone());
    }
    token
}

/// A `url_result(url, ie)` playlist entry, resolved when processed.
fn url_entry(target: &str, ie: Option<&str>) -> Result<VideoInfo> {
    let url = Url::parse(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    let mut entry = Map::new();
    if let Some(ie) = ie {
        entry.insert("ie_key".into(), ie.into());
    }
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), target.into());
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": ie,
            "extractor": ie.map(str::to_lowercase),
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

fn video(page: &Url, dict: Map<String, Value>, formats: Vec<crate::VideoFormat>) -> VideoInfo {
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
    info.extractor_key = NAME.into();
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
    crate::info_order::mark_info(info, &format!("dict;{}", keys.join(",")))
}

impl Extractor for Tumblr {
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
        let blog = found
            .name("blog2")
            .or(found.name("blog1"))
            .map_or("", |blog| blog.as_str())
            .to_owned();
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        let token = access_token(client);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(page.as_str())
        ));
        let post_url = Url::parse(&format!("http://{blog}.tumblr.com/post/{video_id}"))?;
        crate::screen::screen(&format!("[{NAME}] {video_id}: Downloading webpage"));
        let response = super::direct::request_with_headers(
            client,
            &post_url,
            None,
            &[("User-Agent", "WhatsApp/2.0")],
        )
        .map_err(|error| match error {
            Error::Http(_) | Error::HttpRetryAfter { .. } => Error::ExtractorRequest {
                extractor: NAME,
                id: video_id.clone(),
                what: "webpage",
                source: Box::new(error),
            },
            error => error,
        })?;
        let redirect_url = response.url().to_string();
        let webpage = {
            use std::io::Read;
            let mut response = response;
            let mut body = Vec::new();
            response
                .read_to_end(&mut body)
                .map_err(Error::ExtractionRead)?;
            String::from_utf8_lossy(&body).into_owned()
        };
        static API_ONLY: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"(tumblr.com|^)/(safe-mode|login_required|blog/view)").unwrap()
        });
        let api_only = API_ONLY.is_match(&redirect_url);
        if api_only && token.is_none() {
            return Err(fail(
                &video_id,
                "Cannot get data for dashboard-only post without access token",
            ));
        }
        let mut post = json!({});
        if let Some(token) = &token {
            let api = Url::parse(&format!(
                "https://www.tumblr.com/api/v2/blog/{blog}/posts/{video_id}/permalink"
            ))?;
            let authorization = format!("Bearer {token}");
            match super::download_json_note_headers(
                client,
                &api,
                NAME,
                &video_id,
                "Downloading JSON metadata",
                &[("Authorization", &authorization)],
            ) {
                Ok(value) => {
                    if let Some(element) = value
                        .pointer("/response/timeline/elements/0")
                        .filter(|element| element.is_object())
                    {
                        post = element.clone();
                    }
                }
                Err(error) => crate::screen::warning(&error.to_string()),
            }
        }
        // The trail's first post (the original) else the post itself.
        let content: Vec<Value> = [post.pointer("/trail/0"), Some(&post)]
            .into_iter()
            .flatten()
            .filter_map(|source| source.get("content").and_then(Value::as_array))
            .flatten()
            .filter(|item| item.is_object())
            .cloned()
            .collect();
        let description = if content.is_empty() {
            crate::html::og_search_property(&webpage, "description")
        } else {
            let texts: Vec<&str> = content
                .iter()
                .filter(|item| item.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|item| item.get("text").and_then(Value::as_str))
                .collect();
            Some(texts.join("\n\n")).filter(|text| !text.is_empty())
        };
        let uploader_id = ["reblogged_root_name", "blog_name"]
            .iter()
            .find_map(|key| {
                post.get(*key)
                    .and_then(Value::as_str)
                    .filter(|v| !v.is_empty())
            })
            .map(str::to_owned);
        let title = post
            .get("summary")
            .and_then(Value::as_str)
            .filter(|summary| !summary.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| {
                if api_only {
                    blog.clone()
                } else {
                    TITLE
                        .captures(&webpage)
                        .map(|found| crate::html::clean_html(&found["title"]))
                        .filter(|title| !title.is_empty())
                        .unwrap_or_else(|| blog.clone())
                }
            });
        let mut info = Map::new();
        info.insert("id".into(), video_id.clone().into());
        info.insert("title".into(), title.into());
        info.insert("description".into(), description.into());
        info.insert("uploader_id".into(), uploader_id.clone().into());
        info.insert(
            "uploader_url".into(),
            uploader_id
                .map(|id| format!("https://{id}.tumblr.com/"))
                .into(),
        );
        if let Some(timestamp) = [post.pointer("/trail/0/post"), Some(&post)]
            .into_iter()
            .flatten()
            .find_map(|source| int(source.get("timestamp")))
        {
            info.insert("timestamp".into(), timestamp.into());
        }
        if let Some(likes) = int(post.get("like_count")) {
            info.insert("like_count".into(), likes.into());
        }
        if let Some(reblogs) = int(post.get("reblog_count")) {
            info.insert("repost_count".into(), reblogs.into());
        }
        let tags: Vec<Value> = post
            .get("tags")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|tag| tag.is_string())
            .cloned()
            .collect();
        if !tags.is_empty() {
            info.insert("tags".into(), Value::Array(tags));
        }
        info.insert(
            "age_limit".into(),
            match post.get("is_nsfw") {
                Some(Value::Bool(true)) => 18.into(),
                Some(Value::Bool(false)) => 0.into(),
                _ => Value::Null,
            },
        );
        let video_url = crate::html::og_search_property(&webpage, "video");
        let mut fallback: Option<Map<String, Value>> = None;
        let mut entries: Vec<VideoInfo> = Vec::new();
        let mut ignored: Vec<String> = Vec::new();
        let mut unknown: Vec<String> = Vec::new();
        let mut last_video: Option<Value> = None;
        for item in content.iter().filter(|item| {
            matches!(
                item.get("type").and_then(Value::as_str),
                Some("video" | "audio")
            )
        }) {
            last_video = Some(item.clone());
            let media = item.get("media").cloned().unwrap_or_else(|| json!({}));
            let media_url = media
                .get("url")
                .and_then(Value::as_str)
                .filter(|u| !u.is_empty());
            let item_url = item
                .get("url")
                .and_then(Value::as_str)
                .filter(|u| !u.is_empty());
            if api_only && media_url.is_none() && item_url.is_none() {
                return Err(fail(
                    &video_id,
                    "Failed to find video data for dashboard-only post",
                ));
            }
            let provider = item.get("provider").and_then(Value::as_str);
            if matches!(provider, None | Some("tumblr")) {
                let mut format = Map::new();
                format.insert(
                    "url".into(),
                    media_url
                        .map(str::to_owned)
                        .or_else(|| video_url.clone())
                        .into(),
                );
                let dimension = |key: &str, property: &str| {
                    int(media.get(key)).filter(|value| *value != 0).or_else(|| {
                        crate::html::og_search_property(&webpage, property)
                            .and_then(|value| value.trim().parse().ok())
                    })
                };
                format.insert("width".into(), dimension("width", "video:width").into());
                format.insert("height".into(), dimension("height", "video:height").into());
                fallback = Some(format);
                continue;
            }
            let provider = provider.unwrap_or_default();
            if provider == "spotify" {
                if !ignored.iter().any(|known| known == provider) {
                    ignored.push(provider.to_owned());
                }
                continue;
            }
            let key = provider_key(provider);
            if key.is_none() && !unknown.iter().any(|known| known == provider) {
                unknown.push(provider.to_owned());
            }
            if let Some(item_url) = item_url {
                entries.push(url_entry(item_url, key.flatten())?);
            }
        }
        let mut duration: Option<i64> = None;
        let mut formats: Vec<crate::VideoFormat> = Vec::new();
        let iframe_pattern = Regex::new(&format!(
            r"src='(https?://www\.tumblr\.com/video/{}/{}/[^']+)'",
            regex::escape(&blog),
            regex::escape(&video_id)
        ))
        .map_err(|error| Error::Reported(error.to_string()))?;
        if let Some(found) = iframe_pattern.captures(&webpage) {
            let iframe_url = Url::parse(&found[1])?;
            crate::screen::screen(&format!("[{NAME}] {video_id}: Downloading iframe page"));
            let iframe = super::direct::request_with_headers(
                client,
                &iframe_url,
                None,
                &[("Referer", &redirect_url)],
            )
            .and_then(|response| {
                use std::io::Read;
                let mut response = response;
                let mut body = Vec::new();
                response
                    .read_to_end(&mut body)
                    .map_err(Error::ExtractionRead)?;
                Ok(String::from_utf8_lossy(&body).into_owned())
            })?;
            static OPTIONS: LazyLock<Regex> = LazyLock::new(|| {
                Regex::new(r#"data-crt-options=(?:"(?P<a>.+?)"|'(?P<b>.+?)')"#).unwrap()
            });
            static SD: LazyLock<Regex> = LazyLock::new(|| {
                Regex::new(r#"<source[^>]+src=(?:"(?P<a>.+?)"|'(?P<b>.+?)')"#).unwrap()
            });
            static HEIGHT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"_(\d+)\.\w+$").unwrap());
            let options = OPTIONS
                .captures(&iframe)
                .and_then(|found| {
                    found
                        .name("a")
                        .or(found.name("b"))
                        .map(|value| value.as_str().to_owned())
                })
                .and_then(|text| match serde_json::from_str::<Value>(&text) {
                    Ok(options) => Some(options),
                    Err(error) => {
                        crate::screen::warning(&format!(
                            "[{NAME}] {video_id}: Failed to parse JSON: {error}"
                        ));
                        None
                    }
                })
                .filter(|options| {
                    options
                        .as_object()
                        .is_some_and(|options| !options.is_empty())
                });
            if let Some(options) = options {
                duration = int(options.get("duration"));
                if let Some(hd) = options
                    .get("hdUrl")
                    .and_then(Value::as_str)
                    .filter(|hd| !hd.is_empty())
                {
                    let mut sources: Vec<(String, &str)> = Vec::new();
                    if let Some(sd) = SD.captures(&iframe).and_then(|found| {
                        found
                            .name("a")
                            .or(found.name("b"))
                            .map(|value| value.as_str().to_owned())
                    }) {
                        sources.push((sd, "sd"));
                    }
                    sources.push((hd.to_owned(), "hd"));
                    for (quality, (source, format_id)) in sources.iter().enumerate() {
                        let mut format = Map::new();
                        format.insert("url".into(), source.clone().into());
                        format.insert("format_id".into(), (*format_id).into());
                        let height = HEIGHT
                            .captures(source)
                            .and_then(|found| found[1].parse::<i64>().ok());
                        format.insert("height".into(), height.into());
                        format.insert("quality".into(), quality.into());
                        formats.push(format_from(format, &video_id)?);
                    }
                }
            }
        }
        if formats.is_empty()
            && let Some(fallback) = fallback
        {
            formats.push(format_from(fallback, &video_id)?);
        }
        if !formats.is_empty() {
            let mut dict = info.clone();
            dict.insert("formats".into(), json!([]));
            dict.insert("duration".into(), duration.into());
            dict.insert(
                "thumbnail".into(),
                last_video
                    .as_ref()
                    .and_then(|item| item.pointer("/poster/0/url"))
                    .and_then(Value::as_str)
                    .and_then(super::json_ld::url_or_none)
                    .or_else(|| crate::html::og_search_property(&webpage, "image"))
                    .into(),
            );
            entries.insert(0, video(page, dict, formats));
        }
        if !ignored.is_empty() {
            if entries.is_empty() {
                return Err(fail(
                    &video_id,
                    &format!(
                        "None of embed providers are supported: {}",
                        ignored.join(", ")
                    ),
                ));
            }
            crate::screen::warning(&format!(
                "[{NAME}] {video_id}: Skipped embeds from unsupported providers: {}",
                ignored.join(", ")
            ));
        }
        if !unknown.is_empty() {
            crate::screen::warning(&format!(
                "[{NAME}] {video_id}: Unrecognized providers, please report: {}",
                unknown.join(", ")
            ));
        }
        if entries.is_empty() {
            return Err(fail(&video_id, "No video could be found in this post"));
        }
        if entries.len() == 1 {
            let entry = entries.remove(0);
            // `{**info_dict, **url_result}`: a `url` result, followed.
            if let Some(marker) = entry.extra.get(crate::TRANSPARENT) {
                let target = Url::parse(marker["url"].as_str().unwrap_or_default())?;
                let mut follow = super::direct::build_at(target.clone(), &target, String::new());
                follow.formats.clear();
                follow.direct = None;
                follow
                    .extra
                    .insert(crate::FOLLOW_URL.into(), target.as_str().into());
                return Ok(Some(ExtractResult::Video(follow)));
            }
            return Ok(Some(ExtractResult::Video(entry)));
        }
        // `{**info_dict, '_type': 'playlist', 'entries': entries}`.
        let keys: Vec<String> = info.keys().cloned().collect();
        let mut extra = info;
        let id = extra
            .shift_remove("id")
            .and_then(|id| id.as_str().map(str::to_owned))
            .unwrap_or_default();
        let title = extra
            .shift_remove("title")
            .and_then(|title| title.as_str().map(str::to_owned))
            .unwrap_or_default();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("playlist-dict;{}", keys.join(",")).into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id,
            title,
            webpage_url: page.clone(),
            extractor: NAME.into(),
            extractor_key: NAME.into(),
            entries,
            extra,
        })))
    }
}

fn format_from(mut format: Map<String, Value>, id: &str) -> Result<crate::VideoFormat> {
    let keys: Vec<String> = format.keys().cloned().collect();
    for key in ["width", "height"] {
        if format.get(key).is_some_and(Value::is_null) {
            format.shift_remove(key);
        }
    }
    let ext = format
        .get("url")
        .and_then(Value::as_str)
        .and_then(super::direct::determine_ext)
        .unwrap_or_else(|| "unknown_video".into());
    format.insert("ext".into(), ext.into());
    if !format.contains_key("format_id") {
        format.insert("format_id".into(), "".into());
    }
    let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(format))
        .map_err(|error| fail(id, &error.to_string()))?;
    crate::info_order::mark_format(&mut format, &format!("dict:{}", keys.join(",")));
    crate::info_order::mark_int_rates(&mut format);
    Ok(format)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://tatianamaslanydaily.tumblr.com/post/54196191430/orphan-black-dvd"
        ));
        assert!(suitable("https://www.tumblr.com/anyaboz/1234567890"));
        assert!(!suitable("https://www.tumblr.com/anyaboz"));
    }
}

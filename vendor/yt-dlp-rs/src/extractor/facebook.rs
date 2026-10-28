//! Upstream `FacebookIE` (`yt_dlp/extractor/facebook.py`) through the page's
//! relay prefetched data (`parse_graphql_video`: legacy playable URLs, the
//! video delivery response's DASH manifests, progressive and HLS URLs, and
//! captions), merged with `extract_metadata`'s webpage fields, plus
//! `FacebookReelIE` and `FacebookPluginsVideoIE` redirects.
use super::Extractor;
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Facebook;
pub struct FacebookReel;
pub struct FacebookPluginsVideo;

const NAME: &str = "facebook";
const KEY: &str = "Facebook";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?:https?://(?:[\w-]+\.)?(?:facebook\.com|facebookwkhpilnemxj7asaniu7vnjjbiltxjqhye3mhbshg7kx5tfyd\.onion)/(?:[^#]*?\#!/)?(?:(?:permalink\.php|video/video\.php|photo\.php|video\.php|video/embed|story\.php|watch(?:/live)?/?)\?(?:.*?)(?:v|video_id|story_fbid)=|[^/]+/videos/(?:[^/]+/)?|[^/]+/posts/|events/(?:[^/]+/)?|groups/[^/]+/(?:permalink|posts)/(?:[\da-f]+/)?|watchparty/)|facebook:)(?P<id>pfbid[A-Za-z0-9]+|\d+)",
    )
    .unwrap()
});
static REEL_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://(?:[\w-]+\.)?facebook\.com/reel/(?P<id>\d+)").unwrap());
static PLUGINS_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:[\w-]+\.)?facebook\.com/plugins/video\.php\?.*?\bhref=(?P<id>https.+)",
    )
    .unwrap()
});
static RELAY_VIDEO: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"data-sjs>(\{.*?"(?:dash_manifest|playable_url(?:_quality_hd)?).*?\})</script>"#)
        .unwrap()
});
static SERVER_JS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"data-sjs>(\{.*?ScheduledServerJS.*?\})</script>").unwrap());
static UTIME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<abbr[^>]+data-utime=["'](\d+)"#).unwrap());
static VIEW_COUNT: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        Regex::new(r#"\bviewCount\s*:\s*["']([\d,.]+)"#).unwrap(),
        Regex::new(r#"video_view_count["']\s*:\s*(\d+)"#).unwrap(),
    ]
});
static OWNER_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"ownerName\s*:\s*"([^"]+)""#).unwrap());
static FACEBOOK_SUFFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s*\|\s*Facebook$").unwrap());
static IMAGE_EXT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\.(?:jpg|png)").unwrap());

/// `FacebookIE._match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `FacebookReelIE._match_valid_url`.
pub fn reel_suitable(url: &str) -> bool {
    REEL_URL.is_match(url)
}

/// `FacebookPluginsVideoIE._match_valid_url`.
pub fn plugins_suitable(url: &str) -> bool {
    PLUGINS_URL.is_match(url)
}

fn int(value: Option<&Value>) -> Option<i64> {
    let value = value?;
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|number| number as i64))
        .or_else(|| value.as_str()?.trim().parse().ok())
}

fn float(value: Option<&Value>, scale: f64) -> Option<f64> {
    let value = value?;
    value
        .as_f64()
        .or_else(|| value.as_str()?.trim().parse().ok())
        .map(|number| number / scale)
}

fn str_or_none(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn children(value: &Value) -> Vec<&Value> {
    match value {
        Value::Array(items) => items.iter().collect(),
        Value::Object(object) => object.values().collect(),
        _ => Vec::new(),
    }
}

fn fail(id: &str, message: &str) -> Error {
    Error::Reported(format!("[{NAME}] {id}: {message}"))
}

/// `extract_relay_prefetched_data(filter, target_keys)`: the first
/// prefetched result `data` holding one of the target keys.
fn relay_prefetched_data(webpage: &str) -> Option<Map<String, Value>> {
    const TARGETS: [&str; 5] = ["video", "event", "nodes", "node", "mediaset"];
    for found in RELAY_VIDEO.captures_iter(webpage) {
        let Ok(relay) = serde_json::from_str::<Value>(&found[1]) else {
            continue;
        };
        let Some(require) = relay.get("require") else {
            continue;
        };
        // `(None, (..., ..., ..., '__bbox', 'require'))`.
        let mut lists = vec![require];
        for a in children(require) {
            for b in children(a) {
                for c in children(b) {
                    if let Some(inner) = c.pointer("/__bbox/require") {
                        lists.push(inner);
                    }
                }
            }
        }
        for list in lists {
            for entry in children(list) {
                let prefetched = children(entry).first().is_some_and(|key| {
                    key.as_str()
                        .is_some_and(|key| key.starts_with("RelayPrefetchedStreamCache"))
                });
                if !prefetched {
                    continue;
                }
                for item in children(entry) {
                    for candidate in children(item) {
                        if let Some(data) = candidate
                            .pointer("/__bbox/result/data")
                            .and_then(Value::as_object)
                            .filter(|data| TARGETS.iter().any(|key| data.contains_key(*key)))
                        {
                            return Some(data.clone());
                        }
                    }
                }
            }
        }
    }
    None
}

/// The `post` results of `extract_metadata`: every server JS result's
/// `data` dict.
fn post_data(webpage: &str) -> Vec<Value> {
    let mut post = Vec::new();
    for found in SERVER_JS.captures_iter(webpage) {
        let Ok(data) = serde_json::from_str::<Value>(&found[1]) else {
            continue;
        };
        let Some(require) = data.get("require") else {
            continue;
        };
        for a in children(require) {
            for b in children(a) {
                for c in children(b) {
                    let Some(inner) = c.pointer("/__bbox/require") else {
                        continue;
                    };
                    for d in children(inner) {
                        for e in children(d) {
                            for f in children(e) {
                                if let Some(result) = f
                                    .pointer("/__bbox/result/data")
                                    .filter(|data| data.is_object())
                                {
                                    post.push(result.clone());
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    post
}

/// `_search_regex(patterns, webpage, group='content')` over the title
/// sources `extract_metadata` tries.
fn page_title(webpage: &str) -> Option<String> {
    static HEADER: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"<h2\s+[^>]*class="uiHeaderTitle"[^>]*>(?P<content>[^<]*)</h2>"#).unwrap()
    });
    static CAPTION: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?s)<span class="fbPhotosPhotoCaption".*?id="fbPhotoPageCaption"><span class="hasCaption">(?P<content>.*?)</span>"#).unwrap()
    });
    static TITLE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"<title>(?P<content>.+?)</title>").unwrap());
    let clean = |text: &str| Some(crate::html::clean_html(text)).filter(|text| !text.is_empty());
    HEADER
        .captures(webpage)
        .and_then(|found| clean(&found["content"]))
        .or_else(|| {
            CAPTION
                .captures(webpage)
                .and_then(|found| clean(&found["content"]))
        })
        .or_else(|| crate::html::html_search_meta(webpage, "og:title"))
        .or_else(|| crate::html::html_search_meta(webpage, "twitter:title"))
        .or_else(|| {
            TITLE
                .captures(webpage)
                .and_then(|found| clean(&found["content"]))
        })
}

/// `extract_metadata(webpage)`: the webpage's title, description,
/// uploader, timestamp, thumbnail and counts.
fn metadata(webpage: &str, video_id: &str) -> Map<String, Value> {
    let post = post_data(webpage);
    let media: Vec<&Value> = post
        .iter()
        .filter_map(|data| data.get("attachments"))
        .flat_map(children)
        .filter_map(|attachment| attachment.get("media"))
        .filter(|media| {
            str_or_none(media.get("id")).as_deref() == Some(video_id)
                && media.get("__typename").and_then(Value::as_str) == Some("Video")
        })
        .collect();
    let first_text = |path: &str| {
        media.iter().find_map(|media| {
            media
                .pointer(path)
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
    };
    let title = first_text("/title/text");
    let description = first_text("/creation_story/comet_sections/message/story/message/text");
    let fallback_title = title.clone().or_else(|| page_title(webpage));
    let description = description.or_else(|| {
        ["description", "og:description", "twitter:description"]
            .iter()
            .find_map(|name| crate::html::html_search_meta(webpage, name))
    });
    let named_owner = |owner: Option<&Value>| {
        owner
            .filter(|owner| {
                owner
                    .get("name")
                    .is_some_and(|name| !name.is_null() && name != "")
            })
            .cloned()
    };
    let uploader_data = media
        .iter()
        .find_map(|media| {
            media
                .get("owner")
                .filter(|owner| owner.is_object())
                .cloned()
        })
        .or_else(|| {
            post.iter().find_map(|data| {
                data.pointer("/video/creation_story/attachments")
                    .map(children)
                    .into_iter()
                    .flatten()
                    .find_map(|attachment| named_owner(attachment.pointer("/media/owner")))
            })
        })
        .or_else(|| {
            post.iter().find_map(|data| {
                children(data)
                    .into_iter()
                    .find_map(|value| named_owner(value.pointer("/video/owner")))
            })
        })
        .or_else(|| {
            post.iter().find_map(|data| {
                data.pointer("/node/actors")
                    .map(children)
                    .into_iter()
                    .flatten()
                    .find(|actor| actor.is_object())
                    .cloned()
            })
        })
        .or_else(|| {
            post.iter().find_map(|data| {
                data.pointer("/event/event_creator")
                    .filter(|v| v.is_object())
                    .cloned()
            })
        })
        .or_else(|| {
            post.iter().find_map(|data| {
                data.pointer("/video/creation_story/short_form_video_context/video_owner")
                    .filter(|v| v.is_object())
                    .cloned()
            })
        })
        .unwrap_or_else(|| json!({}));
    let uploader = uploader_data
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            crate::html_elements::element_html_by_id("fbPhotoPageAuthorName", webpage)
                .ok()
                .flatten()
                .map(|element| crate::html::clean_html(&element))
                .filter(|name| !name.is_empty())
        })
        .or_else(|| {
            OWNER_NAME
                .captures(webpage)
                .map(|found| found[1].to_owned())
                .or_else(|| crate::html::og_search_property(webpage, "title"))
                .or_else(|| {
                    super::search_warning(NAME, "uploader");
                    None
                })
        });
    let timestamp = UTIME
        .captures(webpage)
        .and_then(|found| found[1].parse::<i64>().ok());
    let thumbnail = ["og:image", "twitter:image"]
        .iter()
        .find_map(|name| crate::html::html_search_meta(webpage, name))
        .filter(|thumbnail| IMAGE_EXT.is_match(thumbnail));
    let view_count = VIEW_COUNT
        .iter()
        .find_map(|pattern| pattern.captures(webpage))
        .and_then(|found| super::youtube::dict::parse_count(&found[1]));
    let concurrent = post.iter().find_map(|data| {
        let mut sources: Vec<&Value> = data.get("video").into_iter().collect();
        for a in children(data) {
            for b in children(a) {
                if let Some(attachments) = b.get("attachments") {
                    for attachment in children(attachments) {
                        sources.extend(attachment.get("media"));
                    }
                }
            }
        }
        sources
            .into_iter()
            .find_map(|source| int(source.get("liveViewerCount")))
    });
    let mut info = Map::new();
    let mut put = |key: &str, value: Option<Value>| {
        if let Some(value) = value.filter(|value| !value.is_null()) {
            info.insert(key.to_owned(), value);
        }
    };
    // `info_json_ld['title']` first (this port reads no JSON-LD).
    let title_text = FACEBOOK_SUFFIX
        .replace(fallback_title.as_deref().unwrap_or_default(), "")
        .into_owned();
    let title = Some(title_text)
        .filter(|title| !title.is_empty())
        .or_else(|| {
            description
                .as_deref()
                .map(|description| description.replace('\n', " "))
                .filter(|description| !description.is_empty())
        })
        .unwrap_or_else(|| format!("Facebook video #{video_id}"));
    put("title", Some(title.into()));
    put("description", description.map(Value::from));
    put("uploader", uploader.map(Value::from));
    put("uploader_id", uploader_data.get("id").cloned());
    put("timestamp", timestamp.map(Value::from));
    put("thumbnail", thumbnail.map(Value::from));
    put("view_count", view_count.map(Value::from));
    put("concurrent_view_count", concurrent.map(Value::from));
    if let Some(feedback) = post.iter().find_map(|data| {
        data.get("url")
            .and_then(Value::as_str)
            .filter(|url| url.contains(video_id))
            .and(data.get("feedback"))
    }) {
        put(
            "like_count",
            feedback
                .pointer("/likers/count")
                .filter(|count| count.is_i64())
                .cloned(),
        );
        put(
            "comment_count",
            feedback
                .pointer("/total_comment_count")
                .filter(|count| count.is_i64())
                .cloned(),
        );
        put(
            "repost_count",
            feedback
                .pointer("/share_count_reduced")
                .and_then(Value::as_str)
                .and_then(super::youtube::dict::parse_count)
                .map(Value::from),
        );
    }
    info
}

/// `extract_dash_manifest(vid_data, formats, mpd_url)`.
fn dash_manifest(
    data: &Value,
    formats: &mut Vec<crate::VideoFormat>,
    mpd_url: Option<&str>,
    id: &str,
) {
    let Some(manifest) = [
        "dash_manifest",
        "playlist",
        "dash_manifest_xml_string",
        "manifest_xml",
    ]
    .iter()
    .find_map(|key| data.get(*key).and_then(Value::as_str)) else {
        return;
    };
    // `urllib.parse.unquote_plus`.
    let text = percent_encoding::percent_decode_str(&manifest.replace('+', " "))
        .decode_utf8_lossy()
        .into_owned();
    let base = data
        .get("dash_manifest_url")
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
        .or_else(|| mpd_url.map(str::to_owned));
    let Some(base) = base.and_then(|base| Url::parse(&base).ok()) else {
        crate::screen::warning(&format!(
            "[{NAME}] {id}: DASH manifests without a manifest URL are not supported natively"
        ));
        return;
    };
    match crate::dash::parse_with_subtitles(&text, &base) {
        Ok((parsed, subtitles)) => {
            // `_parse_mpd_formats(doc, mpd_url=...)`: no base URL besides
            // the document's own, and the subtitles ignored.
            if !subtitles.is_empty() {
                crate::screen::warning_once(&format!(
                    "[{NAME}] Ignoring subtitle tracks found in the DASH manifest; if any subtitle tracks are missing, {}",
                    crate::BUG_REPORTS_MESSAGE
                ));
            }
            let directory = base.join(".").map(String::from).unwrap_or_default();
            for mut format in parsed {
                if format
                    .extra
                    .get("fragment_base_url")
                    .and_then(Value::as_str)
                    == Some(directory.as_str())
                {
                    format.extra.insert("fragment_base_url".into(), "".into());
                }
                formats.push(format);
            }
        }
        Err(error) => crate::screen::warning(&format!(
            "[{NAME}] {id}: {}",
            crate::upstream_error_text(&error)
        )),
    }
}

fn simple_format(
    format_id: Option<&str>,
    quality: i64,
    url: &str,
    id: &str,
) -> Result<crate::VideoFormat> {
    let mut format = Map::new();
    let mut keys = Vec::new();
    if let Some(format_id) = format_id {
        format.insert("format_id".into(), format_id.into());
        keys.push("format_id");
    }
    format.insert("quality".into(), quality.into());
    format.insert("url".into(), url.into());
    keys.extend(["quality", "url"]);
    format.insert(
        "ext".into(),
        super::direct::determine_ext(url)
            .unwrap_or_else(|| "unknown_video".into())
            .into(),
    );
    if !format.contains_key("format_id") {
        format.insert("format_id".into(), "".into());
    }
    let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(format))
        .map_err(|error| fail(id, &error.to_string()))?;
    crate::info_order::mark_format(&mut format, &format!("dict:{}", keys.join(",")));
    crate::info_order::mark_int_rates(&mut format);
    Ok(format)
}

/// A parsed video's dict and formats.
type Entry = (Map<String, Value>, Vec<crate::VideoFormat>);

/// `parse_graphql_video(video)`: the video's dict and formats, if any.
fn graphql_video(
    client: &Client,
    video: &Value,
    video_id: &str,
    webpage: &str,
) -> Result<Option<Entry>> {
    let v_id = str_or_none(video.get("videoId"))
        .or_else(|| str_or_none(video.get("id")))
        .unwrap_or_else(|| video_id.to_owned());
    let mut video = video.clone();
    if let Some(reel) = video
        .pointer("/creation_story/short_form_video_context/playback_video")
        .filter(|value| value.is_object())
        .cloned()
    {
        let mut story = video
            .get("creation_story")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let owner = story
            .pointer("/short_form_video_context/video_owner")
            .cloned()
            .unwrap_or(Value::Null);
        if let Some(object) = story.as_object_mut() {
            object.insert("owner".into(), owner);
            if let Some(reel) = reel.as_object() {
                object.extend(reel.clone());
            }
        }
        video = story;
    }
    let mut formats = Vec::new();
    let quality = |format_id: Option<&str>| match format_id {
        Some("sd") => 0 - 3,
        Some("hd") => 1 - 3,
        _ => -1 - 3,
    };
    let legacy = video
        .get("videoDeliveryLegacyFields")
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or_else(|| video.clone());
    for (key, format_id) in [
        ("playable_url", "sd"),
        ("playable_url_quality_hd", "hd"),
        ("playable_url_dash", ""),
        ("browser_native_hd_url", "hd"),
        ("browser_native_sd_url", "sd"),
    ] {
        let Some(url) = legacy
            .get(key)
            .and_then(Value::as_str)
            .filter(|url| !url.is_empty())
        else {
            continue;
        };
        if super::direct::determine_ext(url).as_deref() == Some("mpd") {
            if let Some(stream) = Url::parse(url)
                .ok()
                .and_then(|mpd| super::extract_mpd_optional(client, &mpd, NAME, video_id, false))
            {
                formats.extend(stream.formats);
            }
        } else {
            formats.push(simple_format(
                Some(format_id),
                quality(Some(format_id)),
                url,
                video_id,
            )?);
        }
    }
    dash_manifest(&legacy, &mut formats, None, video_id);
    let delivery = video
        .pointer("/videoDeliveryResponseFragment/videoDeliveryResponseResult")
        .cloned()
        .unwrap_or(Value::Null);
    let mpd_urls: Vec<String> = delivery
        .get("dash_manifest_urls")
        .map(children)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("manifest_url").and_then(Value::as_str))
        .filter_map(super::json_ld::url_or_none)
        .collect();
    let manifests: Vec<&Value> = delivery
        .get("dash_manifests")
        .map(children)
        .into_iter()
        .flatten()
        .filter(|manifest| {
            manifest
                .get("manifest_xml")
                .is_some_and(|xml| !xml.is_null() && xml != "")
        })
        .collect();
    for (index, manifest) in manifests.iter().enumerate() {
        dash_manifest(
            manifest,
            &mut formats,
            mpd_urls.get(index).map(String::as_str),
            video_id,
        );
    }
    if manifests.is_empty() {
        for mpd_url in &mpd_urls {
            if let Some(stream) = Url::parse(mpd_url)
                .ok()
                .and_then(|mpd| super::extract_mpd_optional(client, &mpd, NAME, video_id, false))
            {
                formats.extend(stream.formats);
            }
        }
    }
    for progressive in delivery
        .get("progressive_urls")
        .map(children)
        .into_iter()
        .flatten()
    {
        let Some(url) = progressive
            .get("progressive_url")
            .and_then(Value::as_str)
            .filter(|url| !url.is_empty())
        else {
            continue;
        };
        let format_id = progressive
            .pointer("/metadata/quality")
            .and_then(Value::as_str)
            .map(str::to_lowercase);
        formats.push(simple_format(
            format_id.as_deref(),
            quality(format_id.as_deref()),
            url,
            video_id,
        )?);
    }
    for hls in delivery
        .get("hls_playlist_urls")
        .map(children)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("hls_playlist_url").and_then(Value::as_str))
        .filter_map(super::json_ld::url_or_none)
    {
        if let Some(stream) = Url::parse(&hls).ok().and_then(|manifest| {
            super::extract_m3u8_optional(client, &manifest, NAME, video_id, Some("hls"), false)
        }) {
            formats.extend(stream.formats);
        }
    }
    if formats.is_empty() {
        return Ok(None);
    }
    // Captions, by locale.
    let broadcast = video.get("is_video_broadcast").and_then(Value::as_bool) == Some(true);
    let mut automatic = Map::new();
    let mut subtitles = Map::new();
    let mut captions: Vec<&Value> = video
        .get("video_available_captions_locales")
        .and_then(Value::as_array)
        .map(|captions| captions.iter().collect())
        .unwrap_or_default();
    captions.sort_by(|a, b| {
        let locale = |caption: &Value| {
            caption
                .get("locale")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        locale(a).cmp(&locale(b))
    });
    for caption in captions {
        let Some(url) = caption
            .get("captions_url")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
        else {
            continue;
        };
        let lang = caption
            .get("localized_language")
            .and_then(Value::as_str)
            .filter(|lang| !lang.is_empty())
            .unwrap_or("und");
        let name = caption
            .get("localized_country")
            .and_then(Value::as_str)
            .filter(|country| !country.is_empty())
            .map_or_else(|| lang.to_owned(), |country| format!("{lang} ({country})"));
        let locale = caption
            .get("locale")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let target = if truthy(caption.get("localized_creation_method")) || broadcast {
            &mut automatic
        } else {
            &mut subtitles
        };
        if let Value::Array(tracks) = target.entry(locale).or_insert_with(|| json!([])) {
            tracks.push(json!({"url": url, "name": name, "ext": subtitle_ext(&url)}));
        }
    }
    if let Some(url) = video
        .get("captions_url")
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
        && automatic.is_empty()
        && subtitles.is_empty()
    {
        let locale = ["og:locale", "twitter:locale"]
            .iter()
            .find_map(|name| crate::html::html_search_meta(webpage, name))
            .unwrap_or_else(|| "en_US".into());
        let target = if broadcast {
            &mut automatic
        } else {
            &mut subtitles
        };
        target.insert(locale, json!([{"url": url, "ext": subtitle_ext(&url)}]));
    }
    let mut info = Map::new();
    info.insert("id".into(), v_id.clone().into());
    // The formats themselves are returned apart (a placeholder keeps the key).
    info.insert("formats".into(), json!([]));
    info.insert(
        "thumbnail".into(),
        video
            .pointer("/thumbnailImage/uri")
            .or_else(|| video.pointer("/preferred_thumbnail/image/uri"))
            .cloned()
            .unwrap_or(Value::Null),
    );
    info.insert(
        "uploader_id".into(),
        str_or_none(video.pointer("/owner/id")).map_or(Value::Null, Value::from),
    );
    info.insert(
        "timestamp".into(),
        int(video.get("publish_time"))
            .or_else(|| int(video.get("creation_time")))
            .map_or(Value::Null, Value::from),
    );
    info.insert(
        "duration".into(),
        float(video.get("playable_duration_in_ms"), 1000.0)
            .filter(|duration| *duration != 0.0)
            .or_else(|| float(video.get("length_in_second"), 1.0))
            .map_or(Value::Null, |duration| json!(duration)),
    );
    info.insert("automatic_captions".into(), Value::Object(automatic));
    info.insert("subtitles".into(), Value::Object(subtitles));
    let description = video
        .pointer("/savable_description/text")
        .cloned()
        .unwrap_or(Value::Null);
    match video
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
    {
        Some(name) => {
            info.insert("title".into(), name.into());
            info.insert("description".into(), description);
        }
        None => {
            info.insert(
                "title".into(),
                description
                    .as_str()
                    .filter(|text| !text.is_empty())
                    .map_or_else(|| format!("Facebook video #{v_id}"), str::to_owned)
                    .into(),
            );
        }
    }
    // `process_formats`: a non-browser User-Agent and chunked downloads.
    for format in &mut formats {
        crate::info_order::mark_format_update(format, "http_headers");
        crate::info_order::mark_format_update(format, "downloader_options");
        let mut headers = format.http_headers.take().unwrap_or_default();
        headers.insert("User-Agent".into(), "facebookexternalhit/1.1".into());
        format.http_headers = Some(headers);
        let options = format
            .extra
            .entry("downloader_options")
            .or_insert_with(|| json!({}));
        if let Some(options) = options.as_object_mut() {
            options.insert("http_chunk_size".into(), (250_i64 << 20).into());
        }
    }
    Ok(Some((info, formats)))
}

/// Processing's subtitle `ext` (`determine_ext(url)`).
fn subtitle_ext(url: &str) -> String {
    super::direct::determine_ext(url).unwrap_or_else(|| "unknown_video".into())
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

/// `merge_dicts(a, b)`: `a`'s non-`None` values, then `b`'s new ones (or
/// replacing `a`'s empty strings).
fn merge_dicts(first: Map<String, Value>, second: Map<String, Value>) -> Map<String, Value> {
    let mut merged = Map::new();
    for (key, value) in first.into_iter().chain(second) {
        let replace = match merged.get(&key) {
            None => !value.is_null(),
            Some(Value::String(existing)) => existing.is_empty() && value.is_string(),
            Some(_) => false,
        };
        if replace {
            merged.insert(key, value);
        }
    }
    merged
}

/// A video from upstream's dict (`dict;KEYS` order).
fn video(url: &Url, dict: Map<String, Value>, formats: Vec<crate::VideoFormat>) -> VideoInfo {
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
    info.extractor_key = KEY.into();
    let mut keys = Vec::new();
    for (key, value) in dict {
        keys.push(key.clone());
        match key.as_str() {
            "id" => info.id = value.as_str().unwrap_or_default().to_owned(),
            "title" => info.title = value.as_str().unwrap_or_default().to_owned(),
            "formats" => {}
            "subtitles" | "automatic_captions" => {
                let parsed: crate::Subtitles = serde_json::from_value(value).unwrap_or_default();
                if key == "subtitles" {
                    info.subtitles = parsed;
                } else {
                    info.automatic_captions = parsed;
                }
            }
            _ if value.is_null() => {
                info.extra.insert(key, Value::Null);
            }
            _ => crate::metadata_parser::set_field(&mut info, &key, value),
        }
    }
    crate::info_order::mark_info(info, &format!("dict;{};subtitle=url,name", keys.join(",")))
}

/// `url_result(url)`, followed by the registry.
fn follow(target: &str) -> Result<ExtractResult> {
    let target = Url::parse(target)?;
    let mut info = super::direct::build_at(target.clone(), &target, String::new());
    info.formats.clear();
    info.direct = None;
    info.extra
        .insert(crate::FOLLOW_URL.into(), target.as_str().into());
    Ok(ExtractResult::Video(info))
}

fn announce(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

impl Extractor for Facebook {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        announce(NAME, url);
        let page = if url.scheme() == "facebook" {
            Url::parse(&format!(
                "https://www.facebook.com/video/video.php?v={video_id}"
            ))?
        } else {
            Url::parse(
                &url.as_str()
                    .replace("://m.facebook.com/", "://www.facebook.com/"),
            )?
        };
        let webpage = super::download_webpage(client, &page, NAME, &video_id)?;
        let Some(data) = relay_prefetched_data(&webpage) else {
            static INTERSTITIAL: LazyLock<Regex> = LazyLock::new(|| {
                Regex::new(r#"class="[^"]*uiInterstitialContent[^"]*"><div>(.*?)</div>"#).unwrap()
            });
            if let Some(found) = INTERSTITIAL.captures(&webpage) {
                return Err(fail(
                    &video_id,
                    &format!(
                        "The video is not available, Facebook said: \"{}\"",
                        &found[1]
                    ),
                ));
            }
            if [
                ">You must log in to continue",
                "id=\"login_form\"",
                "id=\"loginbutton\"",
            ]
            .iter()
            .any(|marker| webpage.contains(marker))
            {
                return Err(fail(
                    &video_id,
                    "This video is only available for registered users. Use --cookies-from-browser or --cookies for the authentication. See  https://github.com/yt-dlp/yt-dlp/wiki/FAQ#how-do-i-pass-cookies-to-yt-dlp  for how to manually pass cookies",
                ));
            }
            return Err(Error::UnsupportedHtml5(
                "Facebook pages without relay prefetched video data (server JS, pagelets, watch parties, the tahoe player) are not supported natively".into(),
            ));
        };
        let data = Value::Object(data);
        let mut entries: Vec<Entry> = Vec::new();
        let parse_attachment =
            |attachment: &Value, key: &str, entries: &mut Vec<_>| -> Result<()> {
                let media = attachment.get(key).cloned().unwrap_or_else(|| json!({}));
                if media.get("__typename").and_then(Value::as_str) == Some("Video")
                    && let Some(entry) = graphql_video(client, &media, &video_id, &webpage)?
                {
                    entries.push(entry);
                }
                Ok(())
            };
        let nodes: Vec<Value> = match data
            .get("nodes")
            .filter(|v| truthy(Some(v)))
            .or_else(|| data.get("node"))
        {
            Some(Value::Array(items)) => items.clone(),
            Some(Value::Null) | None => Vec::new(),
            Some(other) => vec![other.clone()],
        };
        let mut attachments = Vec::new();
        for node in &nodes {
            let Some(story) = node.pointer("/comet_sections/content/story") else {
                continue;
            };
            for source in [Some(story), story.get("attached_story")]
                .into_iter()
                .flatten()
            {
                for attachment in source
                    .get("attachments")
                    .map(children)
                    .into_iter()
                    .flatten()
                {
                    for style in [
                        attachment.get("styles"),
                        attachment.get("style_type_renderer"),
                        attachment.pointer("/throwbackStyles/attachment_target_renderer"),
                    ]
                    .into_iter()
                    .flatten()
                    {
                        if let Some(found) =
                            style.get("attachment").filter(|value| value.is_object())
                        {
                            attachments.push(found.clone());
                        }
                    }
                }
            }
        }
        for attachment in &attachments {
            let mut subs: Vec<Value> = attachment
                .pointer("/all_subattachments/nodes")
                .map(children)
                .into_iter()
                .flatten()
                .filter(|node| node.is_object())
                .cloned()
                .collect();
            if subs.is_empty() {
                subs = attachment
                    .pointer("/target/attachments")
                    .map(children)
                    .into_iter()
                    .flatten()
                    .filter_map(|item| item.pointer("/styles/attachment"))
                    .filter(|value| value.is_object())
                    .cloned()
                    .collect();
            }
            for sub in &subs {
                parse_attachment(sub, "media", &mut entries)?;
            }
            parse_attachment(attachment, "media", &mut entries)?;
        }
        for edge in data
            .pointer("/mediaset/currMedia/edges")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            parse_attachment(edge, "node", &mut entries)?;
        }
        let cover = data
            .pointer("/event/cover_media_renderer/cover_video")
            .filter(|value| value.is_object())
            .or_else(|| data.get("video").filter(|value| value.is_object()))
            .cloned();
        if let Some(video_data) = cover {
            let story_attachments: Vec<Value> = video_data
                .pointer("/story/attachments")
                .or_else(|| video_data.pointer("/creation_story/attachments"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for attachment in &story_attachments {
                parse_attachment(attachment, "media", &mut entries)?;
            }
            if entries.is_empty()
                && let Some(entry) = graphql_video(client, &video_data, &video_id, &webpage)?
            {
                entries.push(entry);
            }
        }
        if entries.len() > 1 {
            let entries = entries
                .into_iter()
                .map(|(dict, formats)| video(url, dict, formats))
                .collect();
            let mut extra = Map::new();
            extra.insert(
                crate::info_order::INFO_SHAPE.into(),
                "playlist-dict;id".into(),
            );
            return Ok(Some(ExtractResult::Playlist(PlaylistInfo {
                id: video_id,
                title: String::new(),
                webpage_url: url.clone(),
                extractor: NAME.into(),
                extractor_key: KEY.into(),
                entries,
                extra,
            })));
        }
        let (video_info, formats) = entries.pop().unwrap_or_else(|| {
            let mut info = Map::new();
            info.insert("id".into(), video_id.clone().into());
            (info, Vec::new())
        });
        let mut webpage_info = metadata(&webpage, &video_id);
        if let Some(duration) = video_info
            .get("duration")
            .filter(|value| truthy(Some(value)))
        {
            webpage_info.insert("duration".into(), duration.clone());
        }
        if let Some(thumbnail) = video_info
            .get("thumbnail")
            .filter(|value| truthy(Some(value)))
        {
            webpage_info.insert("thumbnail".into(), thumbnail.clone());
        }
        let dict = merge_dicts(webpage_info, video_info);
        Ok(Some(ExtractResult::Video(video(url, dict, formats))))
    }
}

impl Extractor for FacebookReel {
    fn key(&self) -> &'static str {
        "facebook:reel"
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, _client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = REEL_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        announce("facebook:reel", url);
        follow(&format!(
            "https://m.facebook.com/watch/?v={}&_rdr",
            &found["id"]
        ))
        .map(Some)
    }
}

impl Extractor for FacebookPluginsVideo {
    fn key(&self) -> &'static str {
        "FacebookPluginsVideo"
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, _client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = PLUGINS_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        announce("FacebookPluginsVideo", url);
        // `urllib.parse.unquote(self._match_id(url))`.
        let target = percent_encoding::percent_decode_str(&found["id"])
            .decode_utf8_lossy()
            .into_owned();
        follow(&target).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.facebook.com/radiokicksfm/videos/3676516585958356/"
        ));
        assert!(suitable(
            "https://www.facebook.com/watch/?v=647537299265662"
        ));
        assert!(suitable(
            "https://m.facebook.com/story.php?story_fbid=pfbid0abc&id=1"
        ));
        assert!(suitable("facebook:544765982287235"));
        assert!(!suitable("https://www.facebook.com/radiokicksfm/"));
        assert!(reel_suitable(
            "https://www.facebook.com/reel/1195289147628387"
        ));
        assert!(plugins_suitable(
            "https://www.facebook.com/plugins/video.php?href=https%3A%2F%2Fwww.facebook.com%2Fx%2Fvideos%2F1%2F"
        ));
    }

    #[test]
    fn merge_dicts_keeps_first_values_and_fills_empty_strings() {
        let first = json!({"a": "", "b": 1, "c": null})
            .as_object()
            .unwrap()
            .clone();
        let second = json!({"a": "x", "b": 2, "c": 3, "d": 4})
            .as_object()
            .unwrap()
            .clone();
        assert_eq!(
            Value::Object(merge_dicts(first, second)),
            json!({"a": "x", "b": 1, "c": 3, "d": 4})
        );
    }
}

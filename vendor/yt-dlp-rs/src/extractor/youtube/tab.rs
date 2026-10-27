//! Upstream `YoutubeTabIE` (`yt_dlp/extractor/youtube/_tab.py`): playlist
//! and channel tab pages as playlists of `url` entries, their metadata and
//! the browse API continuations.
use super::dict::parse_count;
use super::metadata::text as get_text;
use super::watch;
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub(crate) const NAME: &str = "youtube:tab";
pub(crate) const KEY: &str = "YoutubeTab";

const RESERVED_NAMES: &str = "channel|c|user|playlist|watch|w|v|embed|e|live|watch_popup|clip|shorts|movies|results|search|shared|hashtag|trending|explore|feed|feeds|browse|oembed|get_video_info|iframe_api|s/player|source|storefront|oops|index|account|t/terms|about|upload|signin|logout";

/// `_VALID_URL` (without the Invidious hosts) and `_URL_RE`'s tail.
static URL_RE: LazyLock<Regex> = LazyLock::new(|| {
    // The direct-URL alternative's `(?!reserved\b)` look-ahead is checked
    // separately: the `regex` crate has no look-around.
    Regex::new(
        r"^(?P<pre>https?://(?:\w+\.)?youtube(?:kids)?\.com/(?:(?P<channel_type>channel|c|user|browse)/|(?P<not_channel>feed/|hashtag/|(?:playlist|watch)\?.*?\blist=)|)(?P<id>[^/?#&]+))(?P<rest>.*)$",
    )
    .unwrap()
});
static RESERVED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"^(?:{RESERVED_NAMES})\b")).unwrap());

struct UrlParts {
    pre: String,
    tab: String,
    post: String,
    id: String,
    not_channel: String,
}

/// `_get_url_mobj`: `pre`, the channel `tab` and the `post` remainder.
fn url_parts(url: &str) -> Option<UrlParts> {
    let found = URL_RE.captures(url)?;
    let not_channel = found.name("not_channel").map_or("", |value| value.as_str());
    let channel_type = found.name("channel_type").map(|value| value.as_str());
    let id = &found["id"];
    if channel_type.is_none() && not_channel.is_empty() {
        // The direct URL form (`/@handle`, `/NAME`) must not be reserved.
        let path = url.split_once(".com/").map_or("", |(_, path)| path);
        if RESERVED.is_match(path) {
            return None;
        }
    }
    let rest = found.name("rest").map_or("", |value| value.as_str());
    let (tab, post) = if not_channel.is_empty() {
        // `(?(not_channel)|(?P<tab>/[^?#/]+))?`
        match rest.strip_prefix('/') {
            Some(after) => {
                let end = after.find(['?', '#', '/']).unwrap_or(after.len());
                if end == 0 {
                    (String::new(), rest.to_owned())
                } else {
                    (format!("/{}", &after[..end]), after[end..].to_owned())
                }
            }
            None => (String::new(), rest.to_owned()),
        }
    } else {
        (String::new(), rest.to_owned())
    };
    Some(UrlParts {
        pre: found["pre"].to_owned(),
        tab,
        post,
        id: id.to_owned(),
        not_channel: not_channel.to_owned(),
    })
}

/// `YoutubeTabIE.suitable`: tab URLs that are not single videos.
pub fn suitable(url: &str) -> bool {
    url_parts(url).is_some() && !super::suitable(url)
}

/// Python truthiness of a JSON value.
pub(super) fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        Some(Value::Number(number)) => number.as_f64() != Some(0.0),
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(object)) => !object.is_empty(),
    }
}

pub(super) fn at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = value;
    for key in path {
        current = match current {
            Value::Object(object) => object.get(*key)?,
            Value::Array(items) => items.get(key.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(current)
}

fn text_at(value: &Value, path: &[&str]) -> Option<String> {
    at(value, path).and_then(get_text)
}

/// `_get_count(data, path)`.
fn count_at(value: Option<&Value>) -> Option<i64> {
    let text = value.and_then(get_text).unwrap_or_default();
    if text.to_lowercase().starts_with("no ") {
        return Some(0);
    }
    parse_count(&text).or_else(|| {
        let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        let digits: String = compact
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == ',')
            .filter(char::is_ascii_digit)
            .collect();
        digits.parse().ok()
    })
}

/// `^(UC[\w-]{22})$`.
fn ucid_or_none(value: &str) -> Option<String> {
    let rest = value.strip_prefix("UC")?;
    (rest.chars().count() == 22
        && rest
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-'))
    .then(|| value.to_owned())
}

/// `ucid_from_url`.
fn ucid_from_url(url: &str) -> Option<String> {
    static UCID: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^(?:https?://(?:www\.)?youtube\.com)?/(UC[\w-]{22})").unwrap()
    });
    UCID.captures(url).map(|found| found[1].to_owned())
}

/// `handle_from_url`.
fn handle_from_url(url: &str) -> Option<String> {
    let decoded = percent_encoding::percent_decode_str(url).decode_utf8_lossy();
    static HANDLE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^(?:https?://(?:www\.)?youtube\.com)?/(@[\w.-]{3,30})").unwrap()
    });
    HANDLE.captures(&decoded).map(|found| found[1].to_owned())
}

/// `handle_or_none`.
fn handle_or_none(value: &str) -> Option<String> {
    let decoded = percent_encoding::percent_decode_str(value).decode_utf8_lossy();
    static HANDLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(@[\w.-]{3,30})$").unwrap());
    HANDLE.captures(&decoded).map(|found| found[1].to_owned())
}

/// `_extract_thumbnails(data, path, final_key)`.
fn thumbnails(data: Option<&Value>, final_key: &str) -> Vec<Value> {
    let mut found = Vec::new();
    for thumbnail in data
        .and_then(|data| data.get(final_key))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(url) = thumbnail
            .get("url")
            .and_then(Value::as_str)
            .and_then(super::super::json_ld::url_or_none)
        else {
            continue;
        };
        let url = if url.contains("maxresdefault") {
            url.split('?').next().unwrap_or(&url).to_owned()
        } else {
            url
        };
        let int = |key: &str| {
            thumbnail.get(key).and_then(|value| {
                value
                    .as_i64()
                    .or_else(|| value.as_str()?.trim().parse().ok())
            })
        };
        found.push(json!({"url": url, "height": int("height"), "width": int("width")}));
    }
    found
}

/// A `url_result(url, ie, video_id, video_title, **kwargs)` dict.
fn url_result(
    url: &str,
    ie: Option<&str>,
    id: Option<&str>,
    title: Option<String>,
    kwargs: Map<String, Value>,
) -> Map<String, Value> {
    let mut entry = kwargs;
    if let Some(ie) = ie {
        entry.insert("ie_key".into(), ie.into());
    }
    if let Some(id) = id {
        entry.insert("id".into(), id.into());
    }
    if let Some(title) = title {
        entry.insert("title".into(), title.into());
    }
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), url.into());
    entry
}

/// `_availability` over optional flags.
fn availability(
    is_private: Option<bool>,
    needs_premium: Option<bool>,
    needs_subscription: Option<bool>,
    needs_auth: Option<bool>,
    is_unlisted: Option<bool>,
) -> Option<&'static str> {
    let all_known = [
        is_private,
        needs_premium,
        needs_subscription,
        needs_auth,
        is_unlisted,
    ]
    .iter()
    .all(Option::is_some);
    if is_private == Some(true) {
        Some("private")
    } else if needs_premium == Some(true) {
        Some("premium_only")
    } else if needs_subscription == Some(true) {
        Some("subscriber_only")
    } else if needs_auth == Some(true) {
        Some("needs_auth")
    } else if is_unlisted == Some(true) {
        Some("unlisted")
    } else if all_known {
        Some("public")
    } else {
        None
    }
}

/// `_extract_badges` for the badge kinds tabs use.
fn badge_kinds(list: Option<&Value>) -> Vec<&'static str> {
    let mut found = Vec::new();
    for item in list.and_then(Value::as_array).into_iter().flatten() {
        let Some(object) = item.as_object() else {
            continue;
        };
        for (key, badge) in object {
            if !key.to_lowercase().ends_with("badgerenderer") {
                continue;
            }
            let icon = at(badge, &["icon", "iconType"]).and_then(Value::as_str);
            let kind = match icon {
                Some("PRIVACY_UNLISTED") => Some("unlisted"),
                Some("PRIVACY_PRIVATE") => Some("private"),
                Some("PRIVACY_PUBLIC") => Some("public"),
                Some("CHECK_CIRCLE_THICK" | "OFFICIAL_ARTIST_BADGE" | "CHECK") => Some("verified"),
                _ => None,
            }
            .or(match badge.get("style").and_then(Value::as_str) {
                Some("BADGE_STYLE_TYPE_MEMBERS_ONLY") => Some("subscription"),
                Some("BADGE_STYLE_TYPE_PREMIUM") => Some("premium"),
                Some("BADGE_STYLE_TYPE_LIVE_NOW") => Some("live"),
                Some("BADGE_STYLE_TYPE_VERIFIED" | "BADGE_STYLE_TYPE_VERIFIED_ARTIST") => {
                    Some("verified")
                }
                _ => None,
            });
            if let Some(kind) = kind {
                found.push(kind);
                continue;
            }
            let label = [
                at(badge, &["label"]),
                at(badge, &["accessibilityData", "label"]),
                at(badge, &["tooltip"]),
                at(badge, &["iconTooltip"]),
            ]
            .into_iter()
            .flatten()
            .find_map(Value::as_str)
            .unwrap_or_default()
            .to_lowercase();
            for (needle, kind) in [
                ("unlisted", "unlisted"),
                ("private", "private"),
                ("members only", "subscription"),
                ("live", "live"),
                ("premium", "premium"),
                ("verified", "verified"),
                ("official artist channel", "verified"),
            ] {
                if label.contains(needle) {
                    found.push(kind);
                    break;
                }
            }
        }
    }
    found
}

/// `_extract_video`.
fn extract_video(renderer: &Value) -> Map<String, Value> {
    let video_id = renderer
        .get("videoId")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let reel = at(
        renderer,
        &[
            "navigationEndpoint",
            "reelWatchEndpoint",
            "overlay",
            "reelPlayerOverlayRenderer",
            "reelPlayerHeaderSupportedRenderers",
            "reelPlayerHeaderRenderer",
        ],
    );
    let title = text_at(renderer, &["title"])
        .or_else(|| text_at(renderer, &["headline"]))
        .or_else(|| reel.and_then(|reel| text_at(reel, &["reelTitleText"])));
    let description = text_at(renderer, &["descriptionSnippet"]).or_else(|| {
        renderer
            .get("detailedMetadataSnippets")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find_map(|snippet| text_at(snippet, &["snippetText"]))
    });
    let overlays: Vec<&Value> = renderer
        .get("thumbnailOverlays")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|overlay| overlay.get("thumbnailOverlayTimeStatusRenderer"))
        .collect();
    let mut duration = renderer.get("lengthSeconds").and_then(|value| {
        value
            .as_i64()
            .or_else(|| value.as_str()?.trim().parse().ok())
    });
    if duration.is_none() {
        let text = text_at(renderer, &["lengthText"]).or_else(|| {
            overlays
                .iter()
                .find_map(|overlay| text_at(overlay, &["text"]))
        });
        duration = text
            .and_then(|text| crate::parse_duration(&text))
            .map(|duration| duration.whole_seconds() as i64);
    }
    if duration.is_none() {
        static AGO: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"(?i)ago\s+(?P<duration>[a-z0-9 ,]+?)(?:\s+[\d,]+\s+views)?(?:\s+-\s+play\s+short)?$")
                .unwrap()
        });
        let label = at(
            renderer,
            &["title", "accessibility", "accessibilityData", "label"],
        )
        .and_then(Value::as_str)
        .unwrap_or_default();
        // `(ago)(?!.*\1)`: the last `ago`.
        if let Some(position) = label.to_lowercase().rfind("ago") {
            duration = AGO
                .captures(&label[position..])
                .and_then(|found| crate::parse_duration(&found["duration"]))
                .map(|duration| duration.whole_seconds() as i64);
        }
    }
    let byline_runs: Vec<&Value> = at(renderer, &["shortBylineText", "runs"])
        .and_then(Value::as_array)
        .map(|runs| runs.iter().collect())
        .unwrap_or_default();
    let channel_id = byline_runs
        .iter()
        .find_map(|run| {
            at(run, &["navigationEndpoint", "browseEndpoint", "browseId"]).and_then(Value::as_str)
        })
        .map(str::to_owned)
        .or_else(|| {
            reel.and_then(|reel| {
                at(
                    reel,
                    &["channelNavigationEndpoint", "browseEndpoint", "browseId"],
                )
                .and_then(Value::as_str)
                .map(str::to_owned)
            })
        })
        .and_then(|id| ucid_or_none(&id));
    let overlay_style = overlays
        .iter()
        .find_map(|overlay| overlay.get("style").and_then(Value::as_str));
    let badges = badge_kinds(renderer.get("badges"));
    let owner_badges = badge_kinds(renderer.get("ownerBadges"));
    let navigation_url = at(
        renderer,
        &[
            "navigationEndpoint",
            "commandMetadata",
            "webCommandMetadata",
            "url",
        ],
    )
    .and_then(Value::as_str)
    .and_then(|path| Url::parse("https://www.youtube.com/").ok()?.join(path).ok())
    .map(String::from)
    .unwrap_or_default();
    let url = if overlay_style == Some("SHORTS") || navigation_url.contains("/shorts/") {
        format!("https://www.youtube.com/shorts/{video_id}")
    } else {
        format!("https://www.youtube.com/watch?v={video_id}")
    };
    let time_text = text_at(renderer, &["publishedTimeText"])
        .or_else(|| text_at(renderer, &["videoInfo"]))
        .or_else(|| reel.and_then(|reel| text_at(reel, &["timestampText"])))
        .unwrap_or_default();
    let scheduled = at(renderer, &["upcomingEventData", "startTime"]).and_then(|value| {
        let text = match value {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        };
        text.chars()
            .filter(|c| !matches!(c, ',' | '.' | '+'))
            .collect::<String>()
            .trim()
            .parse::<i64>()
            .ok()
    });
    let live_status = if scheduled.is_some() {
        Some("is_upcoming")
    } else if time_text.to_lowercase().contains("streamed") {
        Some("was_live")
    } else if overlay_style == Some("LIVE") || badges.contains(&"live") {
        Some("is_live")
    } else {
        None
    };
    let view_text = text_at(renderer, &["viewCountText"])
        .or_else(|| text_at(renderer, &["shortViewCountText"]))
        .or_else(|| text_at(renderer, &["videoInfo"]))
        .unwrap_or_default();
    let view_count = if view_text.to_lowercase().contains("no views") {
        Some(0)
    } else {
        count_at(Some(&json!({"simpleText": view_text})))
    };
    let view_field = if matches!(live_status, Some("is_live" | "is_upcoming")) {
        "concurrent_view_count"
    } else {
        "view_count"
    };
    let channel = text_at(renderer, &["ownerText"])
        .or_else(|| text_at(renderer, &["shortBylineText"]))
        .or_else(|| reel.and_then(|reel| text_at(reel, &["channelTitleText"])));
    let handle = byline_runs.iter().find_map(|run| {
        at(
            run,
            &[
                "navigationEndpoint",
                "commandMetadata",
                "webCommandMetadata",
                "url",
            ],
        )
        .or_else(|| {
            at(
                run,
                &["navigationEndpoint", "browseEndpoint", "canonicalBaseUrl"],
            )
        })
        .and_then(Value::as_str)
        .and_then(handle_from_url)
    });
    let mut entry = Map::new();
    entry.insert("_type".into(), "url".into());
    entry.insert("ie_key".into(), "Youtube".into());
    entry.insert("id".into(), video_id.into());
    entry.insert("url".into(), url.into());
    entry.insert("title".into(), title.into());
    entry.insert("description".into(), description.into());
    entry.insert("duration".into(), duration.into());
    entry.insert("channel_id".into(), channel_id.clone().into());
    entry.insert("channel".into(), channel.clone().into());
    entry.insert(
        "channel_url".into(),
        channel_id
            .map(|id| format!("https://www.youtube.com/channel/{id}"))
            .into(),
    );
    entry.insert("uploader".into(), channel.into());
    entry.insert("uploader_id".into(), handle.clone().into());
    entry.insert(
        "uploader_url".into(),
        handle
            .map(|handle| format!("https://www.youtube.com/{handle}"))
            .into(),
    );
    entry.insert(
        "thumbnails".into(),
        Value::Array(thumbnails(renderer.get("thumbnail"), "thumbnails")),
    );
    entry.insert("timestamp".into(), Value::Null);
    entry.insert("release_timestamp".into(), scheduled.into());
    entry.insert(
        "availability".into(),
        if badges.contains(&"public") {
            Some("public")
        } else {
            availability(
                badges.contains(&"private").then_some(true),
                badges.contains(&"premium").then_some(true),
                badges.contains(&"subscription").then_some(true),
                None,
                badges.contains(&"unlisted").then_some(true),
            )
        }
        .into(),
    );
    entry.insert(view_field.into(), view_count.into());
    entry.insert("live_status".into(), live_status.into());
    entry.insert(
        "channel_is_verified".into(),
        if owner_badges.contains(&"verified") {
            Value::Bool(true)
        } else {
            Value::Null
        },
    );
    entry
}

/// `_extract_channel_renderer`.
fn channel_renderer(renderer: &Value) -> Map<String, Value> {
    let channel_id = renderer
        .get("channelId")
        .and_then(Value::as_str)
        .and_then(ucid_or_none);
    let title = text_at(renderer, &["title"]);
    let channel_url = channel_id
        .as_ref()
        .map(|id| format!("https://www.youtube.com/channel/{id}"));
    let handle = at(
        renderer,
        &[
            "navigationEndpoint",
            "commandMetadata",
            "webCommandMetadata",
            "url",
        ],
    )
    .or_else(|| {
        at(
            renderer,
            &["navigationEndpoint", "browseEndpoint", "canonicalBaseUrl"],
        )
    })
    .and_then(Value::as_str)
    .and_then(handle_from_url)
    .or_else(|| text_at(renderer, &["subscriberCountText"]).and_then(|text| handle_or_none(&text)));
    let followers = count_at(renderer.get("subscriberCountText"))
        .or_else(|| count_at(renderer.get("videoCountText")));
    let mut entry = Map::new();
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), channel_url.clone().into());
    entry.insert("id".into(), channel_id.clone().into());
    entry.insert("ie_key".into(), KEY.into());
    entry.insert("channel".into(), title.clone().into());
    entry.insert("uploader".into(), title.clone().into());
    entry.insert("channel_id".into(), channel_id.into());
    entry.insert("channel_url".into(), channel_url.into());
    entry.insert("title".into(), title.into());
    entry.insert("uploader_id".into(), handle.clone().into());
    entry.insert(
        "uploader_url".into(),
        handle
            .map(|handle| format!("https://www.youtube.com/{handle}"))
            .into(),
    );
    entry.insert("channel_follower_count".into(), followers.into());
    entry.insert(
        "thumbnails".into(),
        Value::Array(thumbnails(renderer.get("thumbnail"), "thumbnails")),
    );
    entry.insert(
        "playlist_count".into(),
        if count_at(renderer.get("subscriberCountText")).is_some() {
            count_at(renderer.get("videoCountText")).into()
        } else {
            Value::Null
        },
    );
    entry.insert(
        "description".into(),
        text_at(renderer, &["descriptionSnippet"]).into(),
    );
    entry.insert(
        "channel_is_verified".into(),
        if badge_kinds(renderer.get("ownerBadges")).contains(&"verified") {
            Value::Bool(true)
        } else {
            Value::Null
        },
    );
    entry
}

/// `_extract_lockup_view_model`.
fn lockup_view_model(model: &Value) -> Option<Map<String, Value>> {
    let content_id = model.get("contentId").and_then(Value::as_str)?;
    let (ie, url, thumbnail_path): (&str, String, &[&str]) =
        match model.get("contentType").and_then(Value::as_str) {
            Some("LOCKUP_CONTENT_TYPE_VIDEO") => (
                "Youtube",
                format!("https://www.youtube.com/watch?v={content_id}"),
                &["contentImage", "thumbnailViewModel", "image"],
            ),
            Some("LOCKUP_CONTENT_TYPE_PLAYLIST" | "LOCKUP_CONTENT_TYPE_PODCAST") => (
                KEY,
                format!("https://www.youtube.com/playlist?list={content_id}"),
                &[
                    "contentImage",
                    "collectionThumbnailViewModel",
                    "primaryThumbnail",
                    "thumbnailViewModel",
                    "image",
                ],
            ),
            other => {
                crate::screen::warning_once(&format!(
                    "[youtube:tab] Unsupported lockup view model content type \"{}\"; {}",
                    other.unwrap_or("None"),
                    crate::BUG_REPORTS_MESSAGE
                ));
                return None;
            }
        };
    let lockup = at(model, &["metadata", "lockupMetadataViewModel"]);
    let content = lockup.and_then(|lockup| at(lockup, &["metadata", "contentMetadataViewModel"]));
    let badge_models: Vec<&Value> = at(model, &["contentImage", "thumbnailViewModel", "overlays"])
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|overlay| {
            [
                at(overlay, &["thumbnailBottomOverlayViewModel", "badges"]),
                at(
                    overlay,
                    &["thumbnailOverlayBadgeViewModel", "thumbnailBadges"],
                ),
            ]
        })
        .flatten()
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(|badge| {
            badge
                .get("thumbnailBadgeViewModel")
                .filter(|value| value.is_object())
        })
        .collect();
    let duration_text = badge_models
        .iter()
        .find_map(|badge| badge.get("text").and_then(Value::as_str))
        .map(str::to_lowercase);
    let badge_styles: Vec<&str> = badge_models
        .iter()
        .filter_map(|badge| badge.get("badgeStyle").and_then(Value::as_str))
        .collect();
    let rows: Vec<&Value> = content
        .and_then(|content| content.get("metadataRows"))
        .and_then(Value::as_array)
        .map(|rows| rows.iter().collect())
        .unwrap_or_default();
    let channel_part = rows
        .iter()
        .filter_map(|row| row.get("metadataParts").and_then(Value::as_array))
        .flatten()
        .find(|part| {
            truthy(at(
                part,
                &[
                    "text",
                    "commandRuns",
                    "0",
                    "onTap",
                    "innertubeCommand",
                    "browseEndpoint",
                    "browseId",
                ],
            ))
        })
        .and_then(|part| part.get("text"));
    let collaborators: Vec<&Value> = lockup
        .and_then(|lockup| {
            at(
                lockup,
                &[
                    "image",
                    "avatarStackViewModel",
                    "rendererContext",
                    "commandContext",
                    "onTap",
                    "innertubeCommand",
                    "showDialogCommand",
                    "panelLoadingStrategy",
                    "inlineContent",
                    "dialogViewModel",
                    "customContent",
                    "listViewModel",
                    "listItems",
                ],
            )
        })
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            at(item, &["listItemViewModel", "title"]).filter(|value| value.is_object())
        })
        .collect();
    let channel_source = channel_part.or_else(|| collaborators.first().copied());
    let mut channel_info = Map::new();
    if let Some(source) = channel_source {
        let browse = at(
            source,
            &[
                "commandRuns",
                "0",
                "onTap",
                "innertubeCommand",
                "browseEndpoint",
            ],
        );
        if let Some(name) = source.get("content").and_then(Value::as_str) {
            channel_info.insert("channel".into(), name.into());
        }
        if let Some(id) = browse
            .and_then(|browse| browse.get("browseId"))
            .and_then(Value::as_str)
            .and_then(ucid_or_none)
        {
            channel_info.insert("channel_id".into(), id.into());
        }
        if let Some(name) = source.get("content").and_then(Value::as_str) {
            channel_info.insert("uploader".into(), name.into());
        }
        if let Some(handle) = browse
            .and_then(|browse| browse.get("canonicalBaseUrl"))
            .and_then(Value::as_str)
            .and_then(handle_from_url)
        {
            channel_info.insert("uploader_id".into(), handle.into());
        }
    }
    let views_and_time: Vec<&Value> = rows
        .iter()
        .filter(|row| {
            row.get("metadataParts")
                .and_then(Value::as_array)
                .and_then(|parts| parts.last())
                .and_then(Value::as_object)
                .is_some_and(|part| part.contains_key("accessibilityLabel"))
        })
        .filter_map(|row| row.get("metadataParts").and_then(Value::as_array))
        .flatten()
        .collect();
    let relative = views_and_time
        .last()
        .and_then(|part| at(part, &["text", "content"]))
        .and_then(Value::as_str)
        .map(str::to_lowercase);
    let member_badges: Vec<&str> = rows
        .iter()
        .filter_map(|row| row.get("badges").and_then(Value::as_array))
        .flatten()
        .filter_map(|badge| at(badge, &["badgeViewModel", "badgeStyle"]).and_then(Value::as_str))
        .collect();
    let mut kwargs = Map::new();
    kwargs.insert(
        "title".into(),
        lockup
            .and_then(|lockup| at(lockup, &["title", "content"]))
            .and_then(Value::as_str)
            .into(),
    );
    let mut image = model;
    for key in thumbnail_path {
        image = image.get(*key).unwrap_or(&Value::Null);
    }
    kwargs.insert(
        "thumbnails".into(),
        Value::Array(thumbnails(Some(image), "sources")),
    );
    kwargs.insert(
        "duration".into(),
        duration_text
            .as_deref()
            .and_then(crate::parse_duration)
            .map_or(Value::Null, |duration| match duration {
                crate::info::PyNumber::Int(value) => value.into(),
                crate::info::PyNumber::Float(value) => value.into(),
            }),
    );
    kwargs.insert(
        "view_count".into(),
        if views_and_time.len() == 2 {
            at(views_and_time[0], &["text", "content"])
                .and_then(Value::as_str)
                .and_then(parse_count)
                .into()
        } else {
            Value::Null
        },
    );
    kwargs.insert("timestamp".into(), Value::Null);
    kwargs.insert(
        "live_status".into(),
        if duration_text.as_deref() == Some("upcoming") {
            Some("is_upcoming")
        } else if badge_styles.contains(&"THUMBNAIL_OVERLAY_BADGE_STYLE_LIVE") {
            Some("is_live")
        } else if relative
            .as_deref()
            .is_some_and(|text| text.contains("streamed"))
        {
            Some("was_live")
        } else {
            None
        }
        .into(),
    );
    kwargs.insert(
        "availability".into(),
        availability(
            None,
            None,
            Some(member_badges.contains(&"BADGE_MEMBERS_ONLY")),
            None,
            None,
        )
        .into(),
    );
    kwargs.insert(
        "channel_url".into(),
        channel_info
            .get("channel_id")
            .and_then(Value::as_str)
            .map(|id| format!("https://www.youtube.com/channel/{id}"))
            .into(),
    );
    kwargs.insert(
        "uploader_url".into(),
        channel_info
            .get("uploader_id")
            .and_then(Value::as_str)
            .map(|handle| format!("https://www.youtube.com/{handle}"))
            .into(),
    );
    let creators: Vec<Value> = collaborators
        .iter()
        .filter_map(|collaborator| collaborator.get("content").and_then(Value::as_str))
        .map(Value::from)
        .collect();
    kwargs.insert(
        "creators".into(),
        if creators.is_empty() {
            Value::Null
        } else {
            Value::Array(creators)
        },
    );
    kwargs.extend(channel_info);
    Some(url_result(&url, Some(ie), Some(content_id), None, kwargs))
}

/// `_extract_basic_item_renderer`.
fn basic_item_renderer(item: &Value) -> Option<&Value> {
    for (key, renderer) in item.as_object()? {
        if !renderer.is_object() {
            continue;
        }
        if matches!(
            key.as_str(),
            "playlistRenderer"
                | "videoRenderer"
                | "channelRenderer"
                | "showRenderer"
                | "reelItemRenderer"
        ) || (key.starts_with("grid") && key.ends_with("Renderer"))
        {
            return Some(renderer);
        }
    }
    None
}

/// `_grid_entries`.
fn grid_entries(grid: &Value, entries: &mut Vec<Map<String, Value>>) {
    for item in grid
        .get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if !item.is_object() {
            continue;
        }
        if let Some(model) = item
            .get("lockupViewModel")
            .filter(|value| value.is_object())
        {
            entries.extend(lockup_view_model(model));
            continue;
        }
        let Some(renderer) = basic_item_renderer(item) else {
            continue;
        };
        let title = text_at(renderer, &["title"]);
        if let Some(playlist_id) = renderer
            .get("playlistId")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
        {
            entries.push(url_result(
                &format!("https://www.youtube.com/playlist?list={playlist_id}"),
                Some(KEY),
                Some(playlist_id),
                title,
                Map::new(),
            ));
            continue;
        }
        if truthy(renderer.get("videoId")) {
            entries.push(extract_video(renderer));
            continue;
        }
        if truthy(renderer.get("channelId")) {
            entries.push(channel_renderer(renderer));
            continue;
        }
        if let Some(endpoint) = at(
            renderer,
            &[
                "navigationEndpoint",
                "commandMetadata",
                "webCommandMetadata",
                "url",
            ],
        )
        .and_then(Value::as_str)
        .and_then(|path| Url::parse("https://www.youtube.com/").ok()?.join(path).ok())
        {
            let endpoint = endpoint.to_string();
            let (ie, id) = if suitable(&endpoint) {
                (KEY, url_parts(&endpoint).map(|parts| parts.id))
            } else if super::suitable(&endpoint) {
                ("Youtube", super::temp_id(&endpoint))
            } else {
                continue;
            };
            entries.push(url_result(
                &endpoint,
                Some(ie),
                id.as_deref(),
                title,
                Map::new(),
            ));
        }
    }
}

/// `_playlist_entries`.
fn playlist_entries(list: &Value, entries: &mut Vec<Map<String, Value>>) {
    for content in list
        .get("contents")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(renderer) = content
            .get("playlistVideoRenderer")
            .or_else(|| content.get("playlistPanelVideoRenderer"))
            .filter(|value| value.is_object())
        else {
            continue;
        };
        if !truthy(renderer.get("videoId")) {
            continue;
        }
        entries.push(extract_video(renderer));
    }
}

/// `_rich_entries`.
fn rich_entries(item: &Value, entries: &mut Vec<Map<String, Value>>) {
    if let Some(model) = at(item, &["content", "lockupViewModel"]).filter(|value| value.is_object())
    {
        entries.extend(lockup_view_model(model));
        return;
    }
    let renderer = [
        "videoRenderer",
        "reelItemRenderer",
        "playlistRenderer",
        "shortsLockupViewModel",
    ]
    .iter()
    .find_map(|key| at(item, &["content", key]))
    .unwrap_or(&Value::Null);
    if truthy(renderer.get("videoId")) {
        entries.push(extract_video(renderer));
        return;
    }
    if let Some(playlist_id) = renderer
        .get("playlistId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
    {
        entries.push(url_result(
            &format!("https://www.youtube.com/playlist?list={playlist_id}"),
            Some(KEY),
            Some(playlist_id),
            text_at(renderer, &["title"]),
            Map::new(),
        ));
        return;
    }
    if truthy(renderer.get("entityId")) {
        let Some(video_id) = at(
            renderer,
            &["onTap", "innertubeCommand", "reelWatchEndpoint", "videoId"],
        )
        .and_then(Value::as_str) else {
            return;
        };
        static SHORT: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"^(.+), (?:[\d,.]+(?:[KM]| million)?|No) views? - play Short$").unwrap()
        });
        let title = at(renderer, &["overlayMetadata", "primaryText", "content"])
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| {
                renderer
                    .get("accessibilityText")
                    .and_then(Value::as_str)
                    .and_then(|text| SHORT.captures(text))
                    .map(|found| found[1].to_owned())
            });
        let mut kwargs = Map::new();
        if let Some(title) = title {
            kwargs.insert("title".into(), title.into());
        }
        if let Some(views) = at(renderer, &["overlayMetadata", "secondaryText", "content"])
            .and_then(Value::as_str)
            .and_then(parse_count)
        {
            kwargs.insert("view_count".into(), views.into());
        }
        kwargs.insert(
            "thumbnails".into(),
            Value::Array(thumbnails(
                at(
                    renderer,
                    &["thumbnailViewModel", "thumbnailViewModel", "image"],
                ),
                "sources",
            )),
        );
        entries.push(url_result(
            &format!("https://www.youtube.com/shorts/{video_id}"),
            Some("Youtube"),
            Some(video_id),
            None,
            kwargs,
        ));
    }
}

/// `_build_api_continuation_query`.
fn continuation_query(token: &str, ctp: Option<&str>) -> Map<String, Value> {
    let mut query = Map::new();
    query.insert("continuation".into(), token.into());
    if let Some(ctp) = ctp.filter(|ctp| !ctp.is_empty()) {
        query.insert("clickTracking".into(), json!({"clickTrackingParams": ctp}));
    }
    query
}

/// `_extract_continuation`.
fn extract_continuation(renderer: &Value) -> Option<Map<String, Value>> {
    let next = at(renderer, &["continuations", "0", "nextContinuationData"])
        .or_else(|| at(renderer, &["continuation", "reloadContinuationData"]))
        .filter(|value| value.is_object());
    if let Some(next) = next {
        let token = next.get("continuation").and_then(Value::as_str)?;
        return Some(continuation_query(
            token,
            next.get("clickTrackingParams").and_then(Value::as_str),
        ));
    }
    for key in ["contents", "items", "rows", "subThreads"] {
        for item in renderer
            .get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let endpoints = [
                at(item, &["continuationItemRenderer", "continuationEndpoint"]),
                at(
                    item,
                    &[
                        "continuationItemRenderer",
                        "button",
                        "buttonRenderer",
                        "command",
                    ],
                ),
                at(
                    item,
                    &[
                        "continuationItemViewModel",
                        "continuationCommand",
                        "innertubeCommand",
                    ],
                ),
            ];
            for endpoint in endpoints.into_iter().flatten() {
                let mut commands: Vec<&Value> =
                    at(endpoint, &["commandExecutorCommand", "commands"])
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter(|command| command.is_object())
                        .collect();
                commands.push(endpoint);
                for command in commands {
                    if let Some(token) =
                        at(command, &["continuationCommand", "token"]).and_then(Value::as_str)
                    {
                        return Some(continuation_query(
                            token,
                            command.get("clickTrackingParams").and_then(Value::as_str),
                        ));
                    }
                }
            }
        }
    }
    None
}

/// `_extract_entries`: the entries of a section list or rich grid and its
/// continuation.
pub(super) fn extract_entries(
    parent: &Value,
    entries: &mut Vec<Map<String, Value>>,
) -> Option<Map<String, Value>> {
    let mut continuation: Option<Map<String, Value>> = None;
    for content in parent
        .get("contents")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if !content.is_object() {
            continue;
        }
        let section = [
            "itemSectionRenderer",
            "musicShelfRenderer",
            "musicShelfContinuation",
        ]
        .iter()
        .find_map(|key| content.get(*key).filter(|value| value.is_object()));
        let Some(section) = section else {
            if let Some(item) = content
                .get("richItemRenderer")
                .filter(|value| truthy(Some(value)))
            {
                rich_entries(item, entries);
                continuation = extract_continuation(parent);
            }
            continue;
        };
        for item in section
            .get("contents")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(object) = item.as_object() else {
                continue;
            };
            for (key, renderer) in object {
                let known = match key.as_str() {
                    "playlistVideoListRenderer" => {
                        playlist_entries(renderer, entries);
                        true
                    }
                    "gridRenderer" | "reelShelfRenderer" => {
                        grid_entries(renderer, entries);
                        true
                    }
                    "videoRenderer" => {
                        if truthy(renderer.get("videoId")) {
                            entries.push(extract_video(renderer));
                        }
                        true
                    }
                    "playlistRenderer" => {
                        grid_entries(&json!({"items": [{"playlistRenderer": renderer}]}), entries);
                        true
                    }
                    "channelRenderer" => {
                        grid_entries(&json!({"items": [{"channelRenderer": renderer}]}), entries);
                        true
                    }
                    "richGridRenderer" => {
                        continuation = extract_entries(renderer, entries);
                        true
                    }
                    "lockupViewModel" => {
                        entries.extend(lockup_view_model(renderer));
                        true
                    }
                    "shelfRenderer"
                    | "musicResponsiveListItemRenderer"
                    | "backstagePostThreadRenderer"
                    | "hashtagTileRenderer" => {
                        unsupported_renderer(key);
                        true
                    }
                    _ => false,
                };
                if known {
                    if key != "richGridRenderer" {
                        continuation = extract_continuation(renderer);
                    }
                    break;
                }
            }
        }
        if continuation.is_none() {
            continuation = extract_continuation(section);
        }
    }
    if continuation.is_none() {
        continuation = extract_continuation(parent);
    }
    continuation
}

/// Renderers whose entries this port does not extract yet.
fn unsupported_renderer(key: &str) {
    crate::screen::warning_once(&format!(
        "[youtube:tab] {key} entries are not supported natively and are skipped"
    ));
}

/// A browse API request (`_extract_response(ep='browse')`).
pub(super) fn browse(
    client: &Client,
    item_id: &str,
    ytcfg: &Map<String, Value>,
    visitor: Option<&str>,
    query: Map<String, Value>,
    note: &str,
) -> Result<Value> {
    watch::innertube_api(client, "browse", NAME, item_id, ytcfg, visitor, query, note)
}

/// `_entries`: the selected tab's entries and every continuation page.
fn tab_entries(
    client: &Client,
    tab: &Value,
    item_id: &str,
    ytcfg: &Map<String, Value>,
    mut visitor: Option<String>,
) -> Result<Vec<Map<String, Value>>> {
    let mut entries = Vec::new();
    let Some(content) = tab.get("content").filter(|value| value.is_object()) else {
        return Ok(entries);
    };
    let parent = content
        .get("sectionListRenderer")
        .or_else(|| content.get("richGridRenderer"))
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let mut continuation = extract_entries(&parent, &mut entries);
    let mut seen = std::collections::HashSet::new();
    let mut page = 1;
    while let Some(query) = continuation.take() {
        let token = query
            .get("continuation")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if let Some(token) = &token
            && !seen.insert(token.clone())
        {
            crate::screen::debug("Detected YouTube feed looping - assuming end of feed.");
            break;
        }
        let response = browse(
            client,
            &format!("{item_id} page {page}"),
            ytcfg,
            visitor.as_deref(),
            query,
            "Downloading API JSON",
        )?;
        page += 1;
        if let Some(found) = watch::visitor_from(&response) {
            visitor = Some(found);
        }
        let items = ["onResponseReceivedActions", "onResponseReceivedEndpoints"]
            .iter()
            .filter_map(|key| response.get(*key).and_then(Value::as_array))
            .flatten()
            .find_map(|action| {
                at(
                    action,
                    &["appendContinuationItemsAction", "continuationItems"],
                )
            })
            .or_else(|| response.get("continuationContents"))
            .cloned()
            .unwrap_or(Value::Null);
        let first = items
            .as_array()
            .and_then(|items| items.first())
            .or(Some(&items))
            .filter(|value| value.is_object())
            .cloned()
            .unwrap_or_else(|| json!({}));
        let mut renderer_found = false;
        for key in first
            .as_object()
            .into_iter()
            .flatten()
            .map(|(key, _)| key.clone())
        {
            let parent_key = match key.as_str() {
                "videoRenderer"
                | "gridPlaylistRenderer"
                | "gridVideoRenderer"
                | "gridChannelRenderer"
                | "lockupViewModel" => Some("items"),
                "playlistVideoRenderer" | "itemSectionRenderer" | "richItemRenderer" => {
                    Some("contents")
                }
                "playlistVideoListContinuation"
                | "gridContinuation"
                | "sectionListContinuation" => None,
                _ => continue,
            };
            let wrapped = match parent_key {
                Some(parent_key) => json!({parent_key: items.clone()}),
                None => items.clone(),
            };
            renderer_found = true;
            let found = match key.as_str() {
                "playlistVideoRenderer" | "playlistVideoListContinuation" => {
                    playlist_entries(&wrapped, &mut entries);
                    None
                }
                "itemSectionRenderer" | "richItemRenderer" | "sectionListContinuation" => {
                    extract_entries(&wrapped, &mut entries)
                }
                _ => {
                    grid_entries(&wrapped, &mut entries);
                    None
                }
            };
            continuation = found.or_else(|| extract_continuation(&wrapped));
        }
        if continuation.is_none() {
            continuation = extract_continuation(&json!({"contents": [first]}));
        }
        if continuation.is_none() && !renderer_found {
            break;
        }
    }
    Ok(entries)
}

/// `_extract_tab_renderers`.
fn tab_renderers(data: &Value) -> Vec<&Value> {
    at(
        data,
        &["contents", "twoColumnBrowseResultsRenderer", "tabs"],
    )
    .and_then(Value::as_array)
    .into_iter()
    .flatten()
    .filter_map(|tab| {
        tab.get("tabRenderer")
            .or_else(|| tab.get("expandableTabRenderer"))
            .filter(|value| value.is_object())
    })
    .collect()
}

/// `_extract_sidebar_info_renderer`.
fn sidebar_renderer<'a>(data: &'a Value, name: &str) -> Option<&'a Value> {
    at(data, &["sidebar", "playlistSidebarRenderer", "items"])
        .and_then(Value::as_array)?
        .iter()
        .find_map(|item| item.get(name).filter(|value| truthy(Some(value))))
}

/// `_extract_availability`.
fn tab_availability(data: &Value) -> Option<&'static str> {
    let sidebar = sidebar_renderer(data, "playlistSidebarPrimaryInfoRenderer");
    let header = at(data, &["header", "playlistHeaderRenderer"]);
    let privacy = header
        .and_then(|header| header.get("privacy"))
        .and_then(Value::as_str);
    let badges = badge_kinds(sidebar.and_then(|sidebar| sidebar.get("badges")));
    let icon = [header, sidebar].into_iter().flatten().find_map(|source| {
        at(
            source,
            &[
                "privacyForm",
                "dropdownFormFieldRenderer",
                "dropdown",
                "dropdownRenderer",
                "entries",
            ],
        )?
        .as_array()?
        .iter()
        .find(|entry| {
            at(entry, &["privacyDropdownItemRenderer", "isSelected"]) == Some(&Value::Bool(true))
        })
        .and_then(|entry| at(entry, &["privacyDropdownItemRenderer", "icon", "iconType"]))
        .and_then(Value::as_str)
    });
    let micro = at(data, &["microformat", "microformatDataRenderer"]);
    let is_private = if let Some(privacy) = privacy {
        Some(badges.contains(&"private") || privacy == "PRIVATE")
    } else {
        icon.map(|icon| icon == "PRIVACY_PRIVATE")
    };
    let is_unlisted = if let Some(privacy) = privacy {
        Some(badges.contains(&"unlisted") || privacy == "UNLISTED")
    } else if let Some(icon) = icon {
        Some(icon == "PRIVACY_UNLISTED")
    } else {
        micro
            .and_then(|micro| micro.get("unlisted"))
            .and_then(Value::as_bool)
    };
    if let Some(value) = availability(
        is_private,
        badges.contains(&"premium").then_some(true),
        badges.contains(&"subscription").then_some(true),
        Some(false),
        is_unlisted,
    ) {
        return Some(value);
    }
    if badges.contains(&"public")
        || privacy == Some("PUBLIC")
        || icon == Some("PRIVACY_PUBLIC")
        || micro
            .and_then(|micro| micro.get("noindex"))
            .and_then(Value::as_bool)
            == Some(false)
    {
        return Some("public");
    }
    None
}

/// `_extract_metadata_from_tabs`.
fn tab_metadata(item_id: &str, data: &Value) -> Map<String, Value> {
    let mut info = Map::new();
    info.insert("id".into(), item_id.into());
    let channel_metadata =
        at(data, &["metadata", "channelMetadataRenderer"]).filter(|value| value.is_object());
    let metadata = if let Some(renderer) = channel_metadata {
        let channel_id = renderer
            .get("externalId")
            .and_then(Value::as_str)
            .and_then(ucid_or_none)
            .or_else(|| {
                renderer
                    .get("channelUrl")
                    .and_then(Value::as_str)
                    .and_then(ucid_from_url)
            });
        info.insert(
            "channel".into(),
            renderer.get("title").cloned().unwrap_or(Value::Null),
        );
        info.insert("channel_id".into(), channel_id.clone().into());
        if let Some(channel_id) = channel_id {
            info.insert("id".into(), channel_id.into());
        }
        Some(renderer)
    } else {
        at(data, &["metadata", "playlistMetadataRenderer"]).filter(|value| value.is_object())
    };
    let page_header = at(
        data,
        &[
            "header",
            "pageHeaderRenderer",
            "content",
            "pageHeaderViewModel",
        ],
    )
    .filter(|value| value.is_object());
    let uncropped = |url: &str| {
        super::super::json_ld::url_or_none(&format!("{}=s0", url.split('=').next().unwrap_or(url)))
    };
    let mut avatar = thumbnails(
        metadata.and_then(|metadata| metadata.get("avatar")),
        "thumbnails",
    );
    if let Some(first) = avatar
        .first()
        .and_then(|thumbnail| thumbnail.get("url"))
        .and_then(Value::as_str)
        .and_then(uncropped)
    {
        avatar.push(json!({"url": first, "id": "avatar_uncropped", "preference": 1}));
    }
    let mut banners: Vec<Value> = data
        .get("header")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .flat_map(|(_, header)| {
            ["banner", "mobileBanner", "tvBanner"]
                .into_iter()
                .flat_map(|key| thumbnails(header.get(key), "thumbnails"))
                .collect::<Vec<_>>()
        })
        .collect();
    if banners.is_empty() {
        banners = thumbnails(
            page_header.and_then(|header| at(header, &["banner", "imageBannerViewModel", "image"])),
            "sources",
        );
    }
    for banner in &mut banners {
        if let Some(object) = banner.as_object_mut() {
            object.insert("preference".into(), (-10).into());
        }
    }
    if let Some(first) = banners
        .first()
        .and_then(|thumbnail| thumbnail.get("url"))
        .and_then(Value::as_str)
        .and_then(uncropped)
    {
        banners.push(json!({"url": first, "id": "banner_uncropped", "preference": -5}));
    }
    let primary = sidebar_renderer(data, "playlistSidebarPrimaryInfoRenderer");
    let playlist_header =
        at(data, &["header", "playlistHeaderRenderer"]).filter(|value| value.is_object());
    let primary_thumbnails: Vec<Value> = primary
        .and_then(|primary| primary.get("thumbnailRenderer"))
        .map(|renderer| {
            [
                "playlistVideoThumbnailRenderer",
                "playlistCustomThumbnailRenderer",
            ]
            .into_iter()
            .flat_map(|key| thumbnails(at(renderer, &[key, "thumbnail"]), "thumbnails"))
            .collect()
        })
        .unwrap_or_default();
    let playlist_thumbnails = thumbnails(
        playlist_header.and_then(|header| {
            at(
                header,
                &[
                    "playlistHeaderBanner",
                    "heroPlaylistThumbnailRenderer",
                    "thumbnail",
                ],
            )
        }),
        "thumbnails",
    );
    let title = metadata
        .and_then(|metadata| metadata.get("title"))
        .filter(|title| truthy(Some(title)))
        .cloned()
        .or_else(|| text_at(data, &["header", "hashtagHeaderRenderer", "hashtag"]).map(Value::from))
        .unwrap_or_else(|| info["id"].clone());
    info.insert("title".into(), title);
    info.insert("availability".into(), tab_availability(data).into());
    let followers = data
        .get("header")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .find_map(|(_, header)| header.get("subscriberCountText"))
        .and_then(|value| count_at(Some(value)))
        .filter(|count| *count != 0)
        .or_else(|| {
            page_header
                .and_then(|header| {
                    at(
                        header,
                        &["metadata", "contentMetadataViewModel", "metadataRows"],
                    )
                })
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|row| row.get("metadataParts").and_then(Value::as_array))
                .flatten()
                .filter_map(|part| at(part, &["text", "content"]).and_then(Value::as_str))
                .find(|text| text.contains("subscribers"))
                .and_then(parse_count)
        });
    info.insert("channel_follower_count".into(), followers.into());
    info.insert(
        "description".into(),
        metadata
            .map(|metadata| {
                metadata
                    .get("description")
                    .cloned()
                    .unwrap_or_else(|| "".into())
            })
            .unwrap_or(Value::Null),
    );
    let tags: Vec<Value> = at(data, &["microformat", "microformatDataRenderer", "tags"])
        .and_then(Value::as_array)
        .map(|tags| tags.iter().filter(|tag| tag.is_string()).cloned().collect())
        .filter(|tags: &Vec<Value>| !tags.is_empty())
        .or_else(|| {
            metadata
                .and_then(|metadata| metadata.get("keywords"))
                .and_then(Value::as_str)
                .filter(|keywords| !keywords.is_empty())
                .and_then(|keywords| crate::config::shlex_split(keywords).ok())
                .map(|words| words.into_iter().map(Value::from).collect())
        })
        .unwrap_or_default();
    info.insert("tags".into(), Value::Array(tags));
    let mut all_thumbnails = if primary_thumbnails.is_empty() {
        playlist_thumbnails
    } else {
        primary_thumbnails
    };
    all_thumbnails.extend(avatar);
    all_thumbnails.extend(banners);
    info.insert("thumbnails".into(), Value::Array(all_thumbnails));
    let handle = metadata
        .and_then(|metadata| {
            metadata
                .get("vanityChannelUrl")
                .and_then(Value::as_str)
                .and_then(handle_from_url)
                .or_else(|| {
                    metadata
                        .get("ownerUrls")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .find_map(handle_from_url)
                })
        })
        .or_else(|| {
            data.get("header")
                .and_then(Value::as_object)
                .into_iter()
                .flatten()
                .find_map(|(_, header)| {
                    header
                        .get("channelHandleText")
                        .and_then(get_text)
                        .and_then(|text| handle_or_none(&text))
                })
        });
    if let Some(handle) = &handle {
        info.insert("uploader_id".into(), handle.clone().into());
        info.insert(
            "uploader_url".into(),
            format!("https://www.youtube.com/{handle}").into(),
        );
    }
    let header_badges = data
        .get("header")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .find_map(|(_, header)| header.get("badges"));
    let verified_image = page_header
        .and_then(|header| {
            at(
                header,
                &["title", "dynamicTextViewModel", "text", "attachmentRuns"],
            )
        })
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|run| at(run, &["element", "type", "imageType", "image", "sources"]))
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(|source| at(source, &["clientResource", "imageName"]).and_then(Value::as_str))
        .any(|name| matches!(name, "CHECK_CIRCLE_FILLED" | "AUDIO_BADGE"));
    if badge_kinds(header_badges).contains(&"verified") || verified_image {
        info.insert("channel_is_verified".into(), true.into());
    }
    // Playlist stats: [video count, view count, last updated].
    let stats = [primary, playlist_header]
        .into_iter()
        .flatten()
        .find_map(|source| {
            ["stats", "briefStats", "numVideosText"]
                .iter()
                .find_map(|key| source.get(*key).filter(|value| !value.is_null()))
        });
    let stat = |index: usize| stats.and_then(|stats| stats.get(index));
    let updated = stat(2).and_then(get_text).or_else(|| {
        playlist_header
            .and_then(|header| text_at(header, &["byline", "1", "playlistBylineRenderer", "text"]))
    });
    let modified = updated
        .as_deref()
        .and_then(|text| {
            crate::dates::unified_timestamp(text, true).or_else(|| {
                static TEXT: LazyLock<Regex> = LazyLock::new(|| {
                    Regex::new(r"(?:.+|^)(?:live|premieres|ed|ing)(?:\s*(?:on|for))?\s*(.+\d)")
                        .unwrap()
                });
                let lower = text.to_lowercase();
                TEXT.captures(&lower)
                    .and_then(|found| crate::dates::unified_timestamp(&found[1], true))
            })
        })
        .and_then(crate::dates::strftime_date);
    info.insert("modified_date".into(), modified.into());
    let mut views = count_at(stat(1));
    if views.is_none() {
        views = count_at(playlist_header.and_then(|header| header.get("viewCountText")));
    }
    info.insert("view_count".into(), views.into());
    let mut count = count_at(stat(0));
    if count.is_none() {
        count = count_at(
            playlist_header
                .and_then(|header| at(header, &["byline", "0", "playlistBylineRenderer", "text"])),
        );
    }
    info.insert("playlist_count".into(), count.into());
    if info.get("channel_id").is_none_or(Value::is_null) {
        let owner = playlist_header
            .and_then(|header| header.get("ownerText"))
            .filter(|owner| truthy(Some(owner)))
            .or_else(|| {
                sidebar_renderer(data, "playlistSidebarSecondaryInfoRenderer").and_then(
                    |secondary| at(secondary, &["videoOwner", "videoOwnerRenderer", "title"]),
                )
            });
        let owner_text = owner.and_then(get_text);
        static OTHERS: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"^by (.+) and \d+ others?$").unwrap());
        let channel = owner_text.map(|text| {
            OTHERS
                .captures(&text)
                .map_or(text.clone(), |found| found[1].to_owned())
        });
        let browse = owner.and_then(|owner| {
            at(
                owner,
                &["runs", "0", "navigationEndpoint", "browseEndpoint"],
            )
        });
        info.insert("channel".into(), channel.into());
        info.insert(
            "channel_id".into(),
            browse
                .and_then(|browse| browse.get("browseId"))
                .and_then(Value::as_str)
                .and_then(ucid_or_none)
                .into(),
        );
        info.insert(
            "uploader_id".into(),
            browse
                .and_then(|browse| browse.get("canonicalBaseUrl"))
                .and_then(Value::as_str)
                .and_then(|path| Url::parse("https://www.youtube.com").ok()?.join(path).ok())
                .and_then(|url| handle_from_url(url.as_str()))
                .into(),
        );
    }
    let uploader = info.get("channel").cloned().unwrap_or(Value::Null);
    info.insert("uploader".into(), uploader);
    let channel_url = info
        .get("channel_id")
        .and_then(Value::as_str)
        .map(|id| format!("https://www.youtube.com/channel/{id}"));
    info.insert("channel_url".into(), channel_url.into());
    let uploader_url = info
        .get("uploader_id")
        .and_then(Value::as_str)
        .map(|handle| format!("https://www.youtube.com/{handle}"));
    info.insert("uploader_url".into(), uploader_url.into());
    info
}

/// A `url` entry for this port's playlist (resolved when processed).
pub(super) fn entry_info(entry: Map<String, Value>) -> Result<VideoInfo> {
    let url_text = entry
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let url = Url::parse(&url_text).map_err(|error| Error::Reported(error.to_string()))?;
    let ie_key = entry
        .get("ie_key")
        .and_then(Value::as_str)
        .unwrap_or("Youtube")
        .to_owned();
    let extractor = if ie_key == KEY { NAME } else { "youtube" };
    let mut info = super::super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    info.id = entry
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    info.title = entry
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": url_text,
            "force_videoid": null,
            "fields": {},
            "ie_key": ie_key,
            "extractor": extractor,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

/// `_extract_data`'s webpage: `ytInitialData` and `ytcfg`.
fn page_data(client: &Client, url: &Url, item_id: &str) -> Result<(Value, Map<String, Value>)> {
    let webpage = super::super::download_webpage(client, url, NAME, item_id)?;
    let ytcfg = watch::extract_ytcfg(&webpage);
    static INITIAL_DATA: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?:window\s*\[\s*["']ytInitialData["']\s*\]|ytInitialData)\s*="#).unwrap()
    });
    let data = match super::super::search_json(&INITIAL_DATA, &webpage, false) {
        Some(Some(data)) => data,
        _ => {
            return Err(Error::UnsupportedYoutube(
                "tab pages without embedded initial data are not supported natively".into(),
            ));
        }
    };
    if ![
        "contents",
        "currentVideoEndpoint",
        "onResponseReceivedActions",
    ]
    .iter()
    .any(|key| truthy(data.get(*key)))
    {
        return Err(Error::UnsupportedYoutube(
            "incomplete YouTube initial data".into(),
        ));
    }
    Ok((data, ytcfg))
}

/// `_report_alerts`: warnings, then the last error as an error.
fn report_alerts(data: &Value, item_id: &str) -> Result<()> {
    let mut errors = Vec::new();
    for alert in data
        .get("alerts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .flat_map(|alert| alert.values())
    {
        let Some(kind) = alert
            .get("type")
            .and_then(Value::as_str)
            .filter(|kind| !kind.is_empty())
        else {
            continue;
        };
        let Some(message) = alert.get("text").and_then(get_text) else {
            continue;
        };
        if kind.eq_ignore_ascii_case("error") {
            errors.push((kind.to_owned(), message));
        } else if !matches!(
            message.as_str(),
            "Unavailable videos will be hidden during playback" | "Unavailable videos are hidden"
        ) {
            crate::screen::warning_once(&format!("[youtube:tab] YouTube said: {kind} - {message}"));
        }
    }
    if let Some((_, last)) = errors.pop() {
        for (kind, message) in errors {
            crate::screen::warning_once(&format!("[youtube:tab] YouTube said: {kind} - {message}"));
        }
        return Err(fail(item_id, &format!("YouTube said: {last}")));
    }
    Ok(())
}

pub struct YoutubeTab {
    pub(crate) no_playlist: bool,
}

impl super::super::Extractor for YoutubeTab {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        match self.extract_result(client, url)? {
            Some(ExtractResult::Video(info)) => Ok(Some(info)),
            Some(ExtractResult::Playlist(_)) | None => Ok(None),
        }
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        if !suitable(url.as_str()) {
            return Ok(None);
        }
        let mut url = url.clone();
        let _ = url.set_host(Some("www.youtube.com"));
        let Some(parts) = url_parts(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let item_id = parts.id.clone();
        let is_channel = parts.not_channel.is_empty();
        if !is_channel && !parts.not_channel.starts_with(['p', 'w']) {
            // Feeds and hashtags have no saved-fixture coverage yet.
            return Err(Error::UnsupportedYoutube(
                "feed and hashtag pages are not supported yet".into(),
            ));
        }
        let mut item_id = item_id;
        let original_tab = parts.tab.trim_start_matches('/').to_owned();
        let display_id = format!("{item_id}{}", parts.tab);
        let mut page_url = url.to_string();
        if is_channel && parts.tab.is_empty() {
            page_url = format!("{}/videos{}", parts.pre, parts.post);
        }
        let query: Vec<(String, String)> = url
            .query_pairs()
            .filter(|(_, value)| !value.is_empty())
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect();
        let first = |key: &str| {
            query
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.clone())
        };
        let (video_id, playlist_id) = (first("v"), first("list"));
        if video_id.is_none() && parts.not_channel.starts_with("watch") {
            let Some(playlist_id) = playlist_id else {
                return Err(fail(&parts.id, "A video URL was given without video ID"));
            };
            crate::screen::warning(&format!(
                "[youtube:tab] A video URL was given without video ID. Trying to download playlist {playlist_id}"
            ));
            let target = Url::parse(&format!(
                "https://www.youtube.com/playlist?list={playlist_id}"
            ))?;
            return self.extract_result(client, &target);
        }
        // `_yes_playlist`.
        if let (Some(playlist_id), Some(video_id)) = (&playlist_id, &video_id) {
            if self.no_playlist {
                crate::screen::screen(&format!(
                    "[{NAME}] Downloading just the video {video_id} because of --no-playlist"
                ));
                let target = Url::parse(&format!("https://www.youtube.com/watch?v={video_id}"))?;
                return Ok(Some(follow(&target)));
            }
            crate::screen::screen(&format!(
                "[{NAME}] Downloading playlist {playlist_id} - add --no-playlist to download just the video {video_id}"
            ));
        }
        let fetch_url = Url::parse(&page_url)?;
        let (mut data, mut ytcfg) = page_data(client, &fetch_url, &display_id)?;
        // The URL redirected to the home page (`FEwhat_to_watch`).
        if tab_renderers(&data)
            .iter()
            .find(|tab| truthy(tab.get("selected")))
            .and_then(|tab| tab.get("tabIdentifier"))
            .and_then(Value::as_str)
            == Some("FEwhat_to_watch")
        {
            return Err(fail(
                &parts.id,
                "The channel/playlist does not exist and the URL redirected to youtube.com home page",
            ));
        }
        // A conditional redirect to the regional channel.
        if let Some(redirect) = data
            .get("onResponseReceivedActions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find_map(|action| {
                at(
                    action,
                    &[
                        "navigateAction",
                        "endpoint",
                        "commandMetadata",
                        "webCommandMetadata",
                        "url",
                    ],
                )
                .and_then(Value::as_str)
            })
        {
            let target = Url::parse("https://www.youtube.com")?.join(redirect)?;
            let target = format!("{target}{}{}", parts.tab, parts.post);
            crate::screen::screen(&format!(
                "[{NAME}] This playlist is likely not available in your region. Following conditional redirect to {target}"
            ));
            return self.extract_result(client, &Url::parse(&target)?);
        }
        let tabs = tab_renderers(&data);
        if is_channel && !tabs.is_empty() {
            let selected = tabs
                .iter()
                .find(|tab| truthy(tab.get("selected")))
                .ok_or_else(|| fail(&parts.id, "Unable to find selected tab"))?;
            let (selected_id, selected_name) = tab_id_and_name(selected);
            crate::screen::debug(&format!(
                "Selected tab: '{selected_id}' ({selected_name}), Requested tab: '{original_tab}'"
            ));
            // `/about` is no longer a tab.
            if original_tab == "about" {
                return Ok(Some(empty_playlist(&item_id, &data, &page_url)?));
            }
            if original_tab.is_empty() && !selected_name.is_empty() {
                crate::screen::screen(&format!(
                    "[{NAME}] Downloading all uploads of the channel. To download only the videos in a specific tab, pass the tab's URL"
                ));
                let has = |id: &str| tabs.iter().any(|tab| tab_id_and_name(tab).0 == id);
                if has("streams") || has("shorts") {
                    // Upstream returns a playlist of the tabs' playlists.
                    return Err(Error::UnsupportedYoutube(format!(
                        "channels with live or shorts tabs are downloaded as nested playlists, which are not supported natively; pass a tab's URL (such as {}/videos) or the channel's uploads playlist",
                        parts.pre
                    )));
                }
                if selected_id != "videos" {
                    if !item_id.starts_with("UC") {
                        return Ok(Some(empty_playlist(&item_id, &data, &page_url)?));
                    }
                    // Topic channels have no /videos: use their uploads playlist.
                    let uploads = format!("UU{}", &item_id[2..]);
                    let uploads_url = format!("https://www.youtube.com/playlist?list={uploads}");
                    match page_data(client, &Url::parse(&uploads_url)?, &uploads) {
                        Ok((found, found_ytcfg)) => {
                            data = found;
                            if ytcfg.is_empty() {
                                ytcfg = found_ytcfg;
                            }
                            item_id = uploads.clone();
                            page_url = uploads_url;
                            crate::screen::screen(&format!(
                                "[{NAME}] The channel does not have a videos, shorts, or live tab. Redirecting to playlist {uploads} instead"
                            ));
                        }
                        Err(_) => return Ok(Some(empty_playlist(&item_id, &data, &page_url)?)),
                    }
                }
            } else if (if original_tab.is_empty() {
                "videos"
            } else {
                &original_tab
            }) != selected_id
            {
                if original_tab == "live" {
                    return Err(Error::Reported(format!(
                        "[youtube:tab] {item_id}: The channel is not currently live"
                    )));
                }
                if !selected_name.is_empty() {
                    return Err(fail(
                        &parts.id,
                        &format!("This channel does not have a {original_tab} tab"),
                    ));
                }
                page_url = format!("{}{}", parts.pre, parts.post);
            }
        }
        // `_reload_with_unavailable_videos`.
        let is_playlist = truthy(at(&data, &["metadata", "playlistMetadataRenderer"]))
            || truthy(at(&data, &["header", "playlistHeaderRenderer"]));
        if is_playlist {
            let mut query = Map::new();
            query.insert("params".into(), "wgYCCAA=".into());
            query.insert("browseId".into(), format!("VL{item_id}").into());
            let visitor = watch::visitor_from(&data)
                .or_else(|| watch::visitor_from(&Value::Object(ytcfg.clone())));
            let reloaded = browse(
                client,
                &display_id,
                &ytcfg,
                visitor.as_deref(),
                query,
                "Redownloading playlist API JSON with unavailable videos",
            )?;
            if truthy(reloaded.get("contents")) {
                data = reloaded;
            }
        }
        report_alerts(&data, &parts.id)?;
        let tabs = tab_renderers(&data);
        if !tabs.is_empty() {
            let mut metadata = tab_metadata(&item_id, &data);
            let selected = tabs
                .iter()
                .find(|tab| truthy(tab.get("selected")))
                .ok_or_else(|| fail(&parts.id, "Unable to find selected tab"))?;
            let mut title = metadata
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            for key in ["title", "expandedText"] {
                if let Some(text) = selected
                    .get(key)
                    .and_then(Value::as_str)
                    .filter(|text| !text.is_empty())
                {
                    title.push_str(&format!(" - {text}"));
                }
            }
            metadata.insert("title".into(), title.clone().into());
            let id = metadata
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or(&item_id)
                .to_owned();
            let visitor = watch::visitor_from(&data)
                .or_else(|| watch::visitor_from(&Value::Object(ytcfg.clone())));
            let entries = tab_entries(client, selected, &id, &ytcfg, visitor)?;
            let entries = entries
                .into_iter()
                .map(entry_info)
                .collect::<Result<Vec<_>>>()?;
            return Ok(Some(tab_playlist(metadata, entries, &page_url, true)?));
        }
        // An inline (watch page) playlist that links to its playlist page.
        if let Some(playlist) = at(
            &data,
            &[
                "contents",
                "twoColumnWatchNextResults",
                "playlist",
                "playlist",
            ],
        )
        .filter(|value| value.is_object())
        {
            let target = at(
                playlist,
                &["endpoint", "commandMetadata", "webCommandMetadata", "url"],
            )
            .and_then(Value::as_str)
            .and_then(|path| Url::parse(&page_url).ok()?.join(path).ok());
            let playlist_id = playlist
                .get("playlistId")
                .and_then(Value::as_str)
                .unwrap_or(&item_id);
            static UNVIEWABLE: LazyLock<Regex> =
                LazyLock::new(|| Regex::new(r"^(?:MLCT|RLTD[\w-]{22})$").unwrap());
            if let Some(target) = target
                && target.as_str() != page_url
                && !UNVIEWABLE.is_match(playlist_id)
            {
                return self.extract_result(client, &target);
            }
            return Err(Error::UnsupportedYoutube(
                "inline mix playlists are not supported natively".into(),
            ));
        }
        let current = at(&data, &["currentVideoEndpoint", "watchEndpoint", "videoId"])
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or(video_id);
        if let Some(video_id) = current {
            if parts.tab != "/live" {
                crate::screen::warning(&format!(
                    "[youtube:tab] Unable to recognize playlist. Downloading just video {video_id}"
                ));
            }
            let target = Url::parse(&format!("https://www.youtube.com/watch?v={video_id}"))?;
            return Ok(Some(follow(&target)));
        }
        Err(fail(&parts.id, "Unable to recognize tab page"))
    }
}

/// An `ExtractorError` with `extract`'s temporary ID.
fn fail(item_id: &str, message: &str) -> Error {
    Error::Reported(format!("[{NAME}] {item_id}: {message}"))
}

/// `_extract_tab_id_and_name`: the tab's ID (from its URL, else its
/// identifier, else its name) and lowercase name.
fn tab_id_and_name(tab: &Value) -> (String, String) {
    let name = tab
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_lowercase();
    let url = at(
        tab,
        &["endpoint", "commandMetadata", "webCommandMetadata", "url"],
    )
    .and_then(Value::as_str)
    .and_then(|path| Url::parse("https://www.youtube.com").ok()?.join(path).ok());
    let from_url = url
        .and_then(|url| url_parts(url.as_str()))
        .map(|parts| parts.tab.trim_start_matches('/').to_owned())
        .filter(|tab| !tab.is_empty());
    let id = from_url.or_else(|| {
        tab.get("tabIdentifier")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .map(str::to_owned)
    });
    let id = match id {
        Some(id) if id == "TAB_ID_SPONSORSHIPS" => "membership".into(),
        Some(id) => id,
        None => {
            if !name.is_empty() {
                crate::screen::debug(&format!("Falling back to selected tab name: {name}"));
            }
            match name.as_str() {
                "home" => "featured".into(),
                "live" => "streams".into(),
                _ => name.clone(),
            }
        }
    };
    (id, name)
}

/// `_empty_playlist`: the tab metadata without entries.
fn empty_playlist(item_id: &str, data: &Value, page_url: &str) -> Result<ExtractResult> {
    let mut metadata = tab_metadata(item_id, data);
    // `playlist_result(entries, item_id, **metadata)`.
    metadata.insert("id".into(), item_id.into());
    tab_playlist(metadata, Vec::new(), page_url, false)
}

/// `playlist_result(entries, **metadata)` with upstream's
/// `extractor_key`/`extractor`/`webpage_url` update.
pub(super) fn tab_playlist(
    metadata: Map<String, Value>,
    entries: Vec<VideoInfo>,
    page_url: &str,
    updated: bool,
) -> Result<ExtractResult> {
    let keys: Vec<String> = metadata.keys().cloned().collect();
    let text = |key: &str| {
        metadata
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let (id, title) = (text("id"), text("title"));
    let mut extra = metadata;
    extra.shift_remove("id");
    extra.shift_remove("title");
    extra.insert(
        crate::info_order::INFO_SHAPE.into(),
        format!(
            "{};{}",
            if updated {
                "youtube-tab"
            } else {
                "youtube-tab-plain"
            },
            keys.join(",")
        )
        .into(),
    );
    Ok(ExtractResult::Playlist(PlaylistInfo {
        id,
        title,
        webpage_url: Url::parse(page_url)?,
        extractor: NAME.into(),
        extractor_key: KEY.into(),
        entries,
        extra,
    }))
}

/// `url_result(url, YoutubeIE)`: followed by the downloader.
fn follow(target: &Url) -> ExtractResult {
    let mut info = super::super::direct::build_at(target.clone(), target, String::new());
    info.formats.clear();
    info.direct = None;
    info.extra
        .insert(crate::FOLLOW_URL.into(), target.as_str().into());
    ExtractResult::Video(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_urls_exclude_single_videos_and_reserved_paths() {
        assert!(suitable("https://www.youtube.com/playlist?list=PLx"));
        assert!(suitable(
            "https://www.youtube.com/watch?v=BaW_jenozKc&list=PLx"
        ));
        assert!(suitable("https://www.youtube.com/@handle/videos"));
        assert!(suitable(
            "https://www.youtube.com/channel/UC0123456789abcdefghijkl"
        ));
        assert!(!suitable("https://www.youtube.com/watch?v=BaW_jenozKc"));
        assert!(!suitable("https://www.youtube.com/shorts/BaW_jenozKc"));
        assert!(!suitable("https://www.youtube.com/results?search_query=x"));
        let parts = url_parts("https://www.youtube.com/@handle/shorts?x=1").unwrap();
        assert_eq!(
            (parts.id.as_str(), parts.tab.as_str()),
            ("@handle", "/shorts")
        );
        assert_eq!(parts.post, "?x=1");
    }

    #[test]
    fn video_durations_fall_back_to_the_accessibility_label() {
        let entry = extract_video(&json!({
            "videoId": "BaW_jenozKc",
            "title": {"accessibility": {"accessibilityData": {
                "label": "Title by Someone 3 days ago 2 minutes, 5 seconds 10 views"
            }}},
        }));
        assert_eq!(entry["duration"], json!(125));
    }
}

//! Upstream `YoutubeIE._real_extract`'s info dict after `_list_formats`:
//! metadata from every player response, the watch page's meta tags and
//! `ytInitialData`, in upstream's key order (comments and `mark_watched`
//! are not represented).
use super::metadata::text as get_text;
use regex::Regex;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;

const THUMBNAIL_NAMES: [&str; 19] = [
    "maxresdefault",
    "hq720",
    "sddefault",
    "hqdefault",
    "0",
    "mqdefault",
    "default",
    "sd1",
    "sd2",
    "sd3",
    "hq1",
    "hq2",
    "hq3",
    "mq1",
    "mq2",
    "mq3",
    "1",
    "2",
    "3",
];

/// Python truthiness of a JSON value.
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

/// A path into a JSON value.
fn at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
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

/// `get_first(values, path)`: the first non-`None` value.
fn get_first<'a>(values: &[&'a Value], path: &[&str]) -> Option<&'a Value> {
    values
        .iter()
        .find_map(|value| at(value, path).filter(|value| !value.is_null()))
}

fn int_or_none(value: Option<&Value>) -> Option<i64> {
    match value {
        Some(Value::Number(number)) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value.trunc() as i64)),
        Some(Value::String(text)) => text.trim().parse().ok(),
        Some(Value::Bool(value)) => Some(i64::from(*value)),
        _ => None,
    }
}

fn float_or_none(value: Option<&Value>) -> Option<f64> {
    match value {
        Some(Value::Number(number)) => number.as_f64(),
        Some(Value::String(text)) => text.trim().parse().ok(),
        Some(Value::Bool(value)) => Some(f64::from(u8::from(*value))),
        _ => None,
    }
}

/// `str_to_int`.
fn str_to_int(text: &str) -> Option<i64> {
    text.chars()
        .filter(|c| !matches!(c, ',' | '.' | '+'))
        .collect::<String>()
        .trim()
        .parse()
        .ok()
}

/// `parse_count`.
pub(crate) fn parse_count(text: &str) -> Option<i64> {
    static PREFIX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[^\d]+\s").unwrap());
    static DIGITS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[\d,.]+$").unwrap());
    static UNIT: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^(?P<num>\d+(?:[,.]\d+)?)\s*(?P<unit>kk|KK|k|K|m|M|b|B)\b").unwrap()
    });
    static LEADING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([\d,.]+)(?:$|\s)").unwrap());
    let stripped = PREFIX.replace(text, "");
    let text = stripped.trim();
    if DIGITS.is_match(text) {
        return str_to_int(text);
    }
    if let Some(found) = UNIT.captures(text) {
        let number: f64 = found["num"].replace(',', ".").parse().ok()?;
        let unit = match &found["unit"] {
            "k" | "K" => 1e3,
            "m" | "M" | "kk" | "KK" => 1e6,
            _ => 1e9,
        };
        return Some((number * unit).round() as i64);
    }
    LEADING
        .captures(text)
        .and_then(|found| str_to_int(&found[1]))
}

/// `_get_count(data, *path)`.
fn get_count(value: Option<&Value>) -> Option<i64> {
    let text = value.and_then(get_text).unwrap_or_default();
    if text.to_lowercase().starts_with("no ") {
        return Some(0);
    }
    parse_count(&text).or_else(|| {
        static LEADING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([\d,]+)").unwrap());
        let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        LEADING
            .captures(&compact)
            .and_then(|found| str_to_int(&found[1]))
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

/// `handle_from_url`.
fn handle_from_url(url: &str) -> Option<String> {
    let decoded = percent_encoding::percent_decode_str(url).decode_utf8_lossy();
    static HANDLE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^(?:https?://(?:www\.)?youtube\.com)?/(@[\w.-]{3,30})").unwrap()
    });
    HANDLE.captures(&decoded).map(|found| found[1].to_owned())
}

/// `_extract_badges` badge types.
#[derive(PartialEq, Eq, Clone, Copy)]
enum Badge {
    Unlisted,
    Private,
    Public,
    Premium,
    Subscription,
    LiveNow,
    Verified,
}

fn badges(list: Option<&Value>) -> Vec<Badge> {
    let mut found = Vec::new();
    for item in list.and_then(Value::as_array).into_iter().flatten() {
        let Some(object) = item.as_object() else {
            continue;
        };
        for (key, badge) in object {
            if !(key.ends_with("adgeRenderer")
                && key.len() > "adgeRenderer".len()
                && matches!(
                    key.as_bytes()[key.len() - "adgeRenderer".len() - 1],
                    b'b' | b'B'
                ))
            {
                continue;
            }
            let icon = at(badge, &["icon", "iconType"]).and_then(Value::as_str);
            let by_icon = match icon {
                Some("PRIVACY_UNLISTED") => Some(Badge::Unlisted),
                Some("PRIVACY_PRIVATE") => Some(Badge::Private),
                Some("PRIVACY_PUBLIC") => Some(Badge::Public),
                Some("CHECK_CIRCLE_THICK" | "OFFICIAL_ARTIST_BADGE" | "CHECK") => {
                    Some(Badge::Verified)
                }
                _ => None,
            };
            let by_style = match badge.get("style").and_then(Value::as_str) {
                Some("BADGE_STYLE_TYPE_MEMBERS_ONLY") => Some(Badge::Subscription),
                Some("BADGE_STYLE_TYPE_PREMIUM") => Some(Badge::Premium),
                Some("BADGE_STYLE_TYPE_LIVE_NOW") => Some(Badge::LiveNow),
                Some("BADGE_STYLE_TYPE_VERIFIED" | "BADGE_STYLE_TYPE_VERIFIED_ARTIST") => {
                    Some(Badge::Verified)
                }
                _ => None,
            };
            if let Some(kind) = by_icon.or(by_style) {
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
                ("unlisted", Badge::Unlisted),
                ("private", Badge::Private),
                ("members only", Badge::Subscription),
                ("live", Badge::LiveNow),
                ("premium", Badge::Premium),
                ("verified", Badge::Verified),
                ("official artist channel", Badge::Verified),
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

/// `_availability`.
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

/// `_extract_thumbnails(data, path)` over every response's details and
/// microformat.
fn thumbnails(sources: &[&Value]) -> Vec<Value> {
    let mut found = Vec::new();
    for source in sources {
        for thumbnail in at(source, &["thumbnail", "thumbnails"])
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
            found.push(json!({
                "url": url,
                "height": int_or_none(thumbnail.get("height")),
                "width": int_or_none(thumbnail.get("width")),
            }));
        }
    }
    found
}

/// `_sort_thumbnails`' key.
fn thumbnail_key(thumbnail: &Value) -> (i64, i64, i64, String, String) {
    let int = |key: &str| thumbnail.get(key).and_then(Value::as_i64).unwrap_or(-1);
    (
        int("preference"),
        int("width"),
        int("height"),
        thumbnail
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        thumbnail
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
    )
}

/// `_extract_heatmap`.
fn heatmap(data: &Value) -> Option<Value> {
    let mut markers = Vec::new();
    for mutation in at(
        data,
        &["frameworkUpdates", "entityBatchUpdate", "mutations"],
    )
    .and_then(Value::as_array)
    .into_iter()
    .flatten()
    {
        let list = at(
            mutation,
            &["payload", "macroMarkersListEntity", "markersList"],
        );
        if list
            .and_then(|list| list.get("markerType"))
            .and_then(Value::as_str)
            != Some("MARKER_TYPE_HEATMAP")
        {
            continue;
        }
        for marker in list
            .and_then(|list| list.get("markers"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let start = float_or_none(marker.get("startMillis")).map(|start| start / 1000.0);
            let end = match (
                int_or_none(marker.get("startMillis")),
                int_or_none(marker.get("durationMillis")),
            ) {
                (Some(start), Some(duration)) => Some((start + duration) as f64 / 1000.0),
                _ => None,
            };
            let value = float_or_none(marker.get("intensityScoreNormalized"));
            let mut entry = Map::new();
            if let Some(start) = start {
                entry.insert("start_time".into(), start.into());
            }
            if let Some(end) = end {
                entry.insert("end_time".into(), end.into());
            }
            if let Some(value) = value {
                entry.insert("value".into(), value.into());
            }
            if !entry.is_empty() {
                markers.push(Value::Object(entry));
            }
        }
    }
    (!markers.is_empty()).then_some(Value::Array(markers))
}

/// `_parse_time_text` for absolute dates (relative texts such as
/// `3 days ago` are not resolved).
fn parse_time_text(text: &str) -> Option<i64> {
    static MONTH_DAY_YEAR: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"([a-z]+\s*\d{1,2},?\s*20\d{2})").unwrap());
    static PREFIXED: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?:.+|^)(?:live|premieres|ed|ing)(?:\s*(?:on|for))?\s*(.+\d)").unwrap()
    });
    let timestamp = crate::dates::unified_timestamp(text, true).or_else(|| {
        let lower = text.to_lowercase();
        let found = MONTH_DAY_YEAR
            .captures(&lower)
            .or_else(|| PREFIXED.captures(&lower))?;
        crate::dates::unified_timestamp(&found[1], true)
    });
    if timestamp.is_none() {
        crate::screen::warning_once(&format!(
            "[youtube] Cannot parse localized time text \"{text}\""
        ));
    }
    timestamp
}

/// The inputs `_real_extract` has after `_list_formats`.
pub(super) struct Input<'a> {
    pub video_id: &'a str,
    pub url: &'a url::Url,
    pub webpage: &'a str,
    pub player_responses: &'a [Value],
    pub initial_data: Option<&'a Value>,
    pub title: &'a str,
    pub description: Option<&'a str>,
    pub duration: Option<i64>,
    pub live_status: Option<&'a str>,
    pub keywords: Vec<String>,
    pub chapters: Value,
    pub automatic_captions: Value,
    pub subtitles: Value,
}

/// The info dict (without `formats`, which stay typed) in upstream's order.
pub(super) fn build(input: &Input<'_>) -> Map<String, Value> {
    let search_meta = |name: &str| crate::html::html_search_meta(input.webpage, name);
    let details: Vec<&Value> = input
        .player_responses
        .iter()
        .filter_map(|response| {
            response
                .get("videoDetails")
                .filter(|value| value.is_object())
        })
        .collect();
    let microformats: Vec<&Value> = input
        .player_responses
        .iter()
        .filter_map(|response| {
            at(response, &["microformat", "playerMicroformatRenderer"])
                .filter(|value| value.is_object())
        })
        .collect();
    let statuses: Vec<&Value> = input
        .player_responses
        .iter()
        .filter_map(|response| {
            response
                .get("playabilityStatus")
                .filter(|value| value.is_object())
        })
        .collect();
    let id = input.video_id;

    // Thumbnails: saved ones (and the page's image), then generated names.
    let mut sources = details.clone();
    sources.extend(&microformats);
    let mut thumbnail_list = thumbnails(&sources);
    if let Some(image) = ["og:image", "twitter:image"]
        .into_iter()
        .find_map(&search_meta)
    {
        thumbnail_list.push(json!({"url": image}));
    }
    let mut original = thumbnail_list.clone();
    let live = if input.live_status == Some("is_live") {
        "_live"
    } else {
        ""
    };
    for name in THUMBNAIL_NAMES {
        for ext in ["webp", "jpg"] {
            let webp = if ext == "webp" { "_webp" } else { "" };
            thumbnail_list.push(json!({
                "url": format!("https://i.ytimg.com/vi{webp}/{id}/{name}{live}.{ext}")
            }));
        }
    }
    let preference = |url: &str| {
        let index = THUMBNAIL_NAMES
            .iter()
            .position(|name| url.contains(&format!("/{id}/{name}")))
            .unwrap_or(THUMBNAIL_NAMES.len()) as i64;
        (if url.contains(".webp") { 0 } else { -1 }) - 2 * index
    };
    for thumbnail in thumbnail_list.iter_mut().chain(original.iter_mut()) {
        let url = thumbnail
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        if let Some(object) = thumbnail.as_object_mut() {
            object.insert("preference".into(), preference(&url).into());
        }
    }
    // `_remove_duplicate_formats`: the first of each URL.
    let mut seen = std::collections::HashSet::new();
    thumbnail_list.retain(|thumbnail| {
        seen.insert(
            thumbnail
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        )
    });
    original.sort_by_key(thumbnail_key);

    let category = get_first(&microformats, &["category"])
        .and_then(Value::as_str)
        .filter(|category| !category.is_empty())
        .map(str::to_owned)
        .or_else(|| search_meta("genre"));
    let channel_id = get_first(&details, &["channelId"])
        .or_else(|| get_first(&microformats, &["externalChannelId"]))
        .map(|value| match value {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        })
        .or_else(|| search_meta("channelId"))
        .and_then(|value| ucid_or_none(&value));
    let owner_profile_url = get_first(&microformats, &["ownerProfileUrl"])
        .and_then(Value::as_str)
        .map(str::to_owned);
    let mut channel_handle = owner_profile_url.as_deref().and_then(handle_from_url);
    let live_start_time = get_first(&microformats, &["liveBroadcastDetails", "startTimestamp"])
        .and_then(Value::as_str)
        .and_then(crate::dates::parse_iso8601);

    let view_count = [&details[..], &microformats[..]]
        .concat()
        .iter()
        .find_map(|value| value.get("viewCount").filter(|value| !value.is_null()))
        .and_then(|value| int_or_none(Some(value)))
        .or_else(|| search_meta("interactionCount").and_then(|count| count.trim().parse().ok()));
    let age_limit = if get_first(&microformats, &["isFamilySafe"]) == Some(&Value::Bool(false))
        || search_meta("isFamilyFriendly").as_deref() == Some("false")
        || search_meta("og:restrictions:age").as_deref() == Some("18+")
    {
        18
    } else {
        0
    };
    let media_type = if truthy(get_first(&details, &["isLiveContent"])) {
        "livestream"
    } else if truthy(get_first(&microformats, &["isShortsEligible"])) {
        "short"
    } else {
        "video"
    };

    let mut info = Map::new();
    info.insert("id".into(), id.into());
    info.insert("title".into(), input.title.into());
    info.insert("thumbnails".into(), Value::Array(thumbnail_list));
    info.insert(
        "thumbnail".into(),
        original
            .last()
            .and_then(|thumbnail| thumbnail.get("url"))
            .cloned()
            .unwrap_or(Value::Null),
    );
    info.insert("description".into(), input.description.into());
    info.insert("channel_id".into(), channel_id.clone().into());
    info.insert(
        "channel_url".into(),
        channel_id
            .as_ref()
            .map(|id| format!("https://www.youtube.com/channel/{id}"))
            .into(),
    );
    info.insert("duration".into(), input.duration.into());
    info.insert("view_count".into(), view_count.into());
    info.insert(
        "average_rating".into(),
        float_or_none(get_first(&details, &["averageRating"])).into(),
    );
    info.insert("age_limit".into(), age_limit.into());
    info.insert(
        "webpage_url".into(),
        format!("https://www.youtube.com/watch?v={id}").into(),
    );
    info.insert(
        "categories".into(),
        category
            .map(|category| json!([category]))
            .unwrap_or(Value::Null),
    );
    info.insert(
        "tags".into(),
        Value::Array(input.keywords.iter().cloned().map(Value::from).collect()),
    );
    info.insert(
        "playable_in_embed".into(),
        get_first(&statuses, &["playableInEmbed"])
            .cloned()
            .unwrap_or(Value::Null),
    );
    info.insert("live_status".into(), input.live_status.into());
    info.insert("media_type".into(), media_type.into());
    info.insert("release_timestamp".into(), live_start_time.into());
    info.insert(
        "automatic_captions".into(),
        input.automatic_captions.clone(),
    );
    info.insert("subtitles".into(), input.subtitles.clone());

    // `start_time`/`end_time` from the URL's fragment, then its query.
    for component in [input.url.fragment(), input.url.query()]
        .into_iter()
        .flatten()
    {
        for (key, value) in url::form_urlencoded::parse(component.as_bytes()) {
            for (target, sources) in [
                ("start_time", &["start", "t"][..]),
                ("end_time", &["end"][..]),
            ] {
                if !info.contains_key(target) && sources.contains(&key.as_ref()) {
                    info.insert(
                        target.into(),
                        crate::parse_duration(&value).map_or(
                            Value::Null,
                            |duration| match duration {
                                crate::info::PyNumber::Int(value) => value.into(),
                                crate::info::PyNumber::Float(value) => value.into(),
                            },
                        ),
                    );
                }
            }
        }
    }

    // YouTube Music auto-generated descriptions.
    let description = input.description.unwrap_or_default();
    if description.trim().ends_with("\nAuto-generated by YouTube.") {
        static MUSIC: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(
                r"(?s)(?:\n|^)(?P<track>[^\n·]+) · (?P<artist>[^\n]+)\n+(?P<album>[^\n]+)\n+(?:℗\s*(?P<release_year>\d{4}))?(?:.+?\nReleased on\s*:\s*(?P<release_date>\d{4}-\d{2}-\d{2}))?(?:.+?\nArtist\s*:\s*(?P<clean_artist>[^\n]+)\n)?.+\nAuto-generated by YouTube\.\s*$",
            )
            .unwrap()
        });
        if let Some(found) = MUSIC.captures(description) {
            let mut year = found
                .name("release_year")
                .map(|year| year.as_str().to_owned());
            let date = found
                .name("release_date")
                .map(|date| date.as_str().replace('-', ""));
            if year.is_none() {
                year = date.as_ref().map(|date| date[..4].to_owned());
            }
            let artists: Vec<Value> = match found.name("clean_artist") {
                Some(artist) => vec![artist.as_str().into()],
                None => found["artist"]
                    .split(" · ")
                    .map(|artist| artist.trim().into())
                    .collect(),
            };
            info.insert("album".into(), found["album"].trim().into());
            info.insert("artists".into(), Value::Array(artists));
            info.insert("track".into(), found["track"].trim().into());
            info.insert("release_date".into(), date.into());
            info.insert(
                "release_year".into(),
                year.and_then(|year| year.parse::<i64>().ok()).into(),
            );
        }
    }

    // `comment_count`.
    let data = input.initial_data.unwrap_or(&Value::Null);
    let comment_count = at(
        data,
        &[
            "contents",
            "twoColumnWatchNextResults",
            "results",
            "results",
            "contents",
        ],
    )
    .and_then(Value::as_array)
    .into_iter()
    .flatten()
    .filter_map(|item| at(item, &["itemSectionRenderer", "contents"]))
    .filter_map(Value::as_array)
    .flatten()
    .find_map(|item| {
        get_count(at(
            item,
            &["commentsEntryPointHeaderRenderer", "commentCount"],
        ))
        .filter(|_| at(item, &["commentsEntryPointHeaderRenderer", "commentCount"]).is_some())
    })
    .or_else(|| {
        data.get("engagementPanels")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|panel| {
                matches!(
                    at(
                        panel,
                        &["engagementPanelSectionListRenderer", "panelIdentifier"]
                    )
                    .and_then(Value::as_str),
                    Some("comment-item-section" | "engagement-panel-comments-section")
                )
            })
            .find_map(|panel| {
                let value = at(
                    panel,
                    &[
                        "engagementPanelSectionListRenderer",
                        "header",
                        "engagementPanelTitleHeaderRenderer",
                        "contextualInfo",
                    ],
                )?;
                get_count(Some(value))
            })
    });
    info.insert("comment_count".into(), comment_count.into());

    // Live chat replays.
    if at(
        data,
        &[
            "contents",
            "twoColumnWatchNextResults",
            "conversationBar",
            "liveChatRenderer",
            "continuations",
            "0",
            "reloadContinuationData",
            "continuation",
        ],
    )
    .is_some()
    {
        let protocol = if matches!(input.live_status, Some("is_live" | "is_upcoming")) {
            "youtube_live_chat"
        } else {
            "youtube_live_chat_replay"
        };
        if let Some(Value::Object(subtitles)) = info.get_mut("subtitles") {
            subtitles.insert(
                "live_chat".into(),
                json!([{
                    "url": format!("https://www.youtube.com/watch?v={id}&bpctr=9999999999&has_verified=1"),
                    "video_id": id,
                    "ext": "json",
                    "protocol": protocol,
                }]),
            );
        }
    }

    if input.initial_data.is_some() {
        info.insert("chapters".into(), input.chapters.clone());
        info.insert("heatmap".into(), heatmap(data).unwrap_or(Value::Null));
    }

    let contents: Vec<&Value> = at(
        data,
        &[
            "contents",
            "twoColumnWatchNextResults",
            "results",
            "results",
            "contents",
        ],
    )
    .and_then(Value::as_array)
    .map(|items| items.iter().collect())
    .unwrap_or_default();
    let vpir = get_first(&contents, &["videoPrimaryInfoRenderer"]);
    if let Some(vpir) = vpir {
        if let Some(link) = vpir.get("superTitleLink").filter(|link| truthy(Some(link)))
            && let Some(link) = get_text(link)
        {
            if at(vpir, &["superTitleIcon", "iconType"]).and_then(Value::as_str)
                == Some("LOCATION_PIN")
            {
                info.insert("location".into(), link.into());
            } else {
                static EPISODE: LazyLock<Regex> =
                    LazyLock::new(|| Regex::new(r"(.+?)\s*S(\d+)\s*•?\s*E(\d+)").unwrap());
                if let Some(found) = EPISODE.captures(&link) {
                    info.insert("series".into(), found[1].into());
                    info.insert("season_number".into(), found[2].parse::<i64>().ok().into());
                    info.insert("episode_number".into(), found[3].parse::<i64>().ok().into());
                }
            }
        }
        static LIKE_DEFAULT: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"^(?P<count>[\d,]+)\s*(?P<type>(?:dis)?like)").unwrap());
        static LIKE_OTHERS: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(
                r"^(?P<type>(?:dis)?like) this video along with (?P<count>[\d,]+) other people",
            )
            .unwrap()
        });
        for button in at(vpir, &["videoActions", "menuRenderer", "topLevelButtons"])
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let mut toggles: Vec<&Value> = Vec::new();
            if let Some(toggle) = button.get("toggleButtonRenderer") {
                toggles.push(toggle);
            }
            if let Some(Value::Object(segmented)) = button.get("segmentedLikeDislikeButtonRenderer")
            {
                for value in segmented.values() {
                    if let Some(toggle) = value.get("toggleButtonRenderer") {
                        toggles.push(toggle);
                    }
                }
            }
            for toggle in toggles {
                let candidates = [
                    (
                        at(
                            toggle,
                            &["defaultText", "accessibility", "accessibilityData"],
                        ),
                        &*LIKE_DEFAULT,
                    ),
                    (
                        at(toggle, &["accessibility"])
                            .filter(|value| value.is_object())
                            .or_else(|| at(toggle, &["accessibilityData", "accessibilityData"])),
                        &*LIKE_OTHERS,
                    ),
                ];
                for (container, regex) in candidates {
                    let Some(label) = container
                        .and_then(|container| container.get("label"))
                        .and_then(Value::as_str)
                        .filter(|label| !label.is_empty())
                    else {
                        continue;
                    };
                    if let Some(found) = regex.captures(label) {
                        info.insert(
                            format!("{}_count", &found["type"]),
                            str_to_int(&found["count"]).into(),
                        );
                        break;
                    }
                }
            }
        }
        let like_count = at(vpir, &["videoActions", "menuRenderer", "topLevelButtons"])
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find_map(|button| {
                at(
                    button,
                    &[
                        "segmentedLikeDislikeButtonViewModel",
                        "likeButtonViewModel",
                        "likeButtonViewModel",
                        "toggleButtonViewModel",
                        "toggleButtonViewModel",
                        "defaultButtonViewModel",
                        "buttonViewModel",
                        "accessibilityText",
                    ],
                )
                .and_then(Value::as_str)
                .and_then(parse_count)
            });
        info.insert("like_count".into(), like_count.into());
        if let Some(renderer) = at(vpir, &["viewCount", "videoViewCountRenderer"]) {
            let count = get_count(renderer.get("viewCount"));
            if truthy(renderer.get("isLive")) {
                info.insert("concurrent_view_count".into(), count.into());
            } else if info.get("view_count").is_none_or(Value::is_null) {
                info.insert("view_count".into(), count.into());
            }
        }
    }

    let vsir = get_first(&contents, &["videoSecondaryInfoRenderer"]);
    if let Some(vsir) = vsir {
        let owner = at(vsir, &["owner", "videoOwnerRenderer"]);
        let collaborator_models: Vec<&Value> = owner
            .and_then(|owner| at(owner, &["attributedTitle", "commandRuns"]))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|run| {
                at(
                    run,
                    &[
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
            .filter_map(Value::as_array)
            .flatten()
            .filter_map(|item| {
                item.get("listItemViewModel")
                    .filter(|value| value.is_object())
            })
            .collect();
        let collaborators: Vec<String> = collaborator_models
            .iter()
            .filter_map(|model| at(model, &["title", "content"]).and_then(Value::as_str))
            .map(str::to_owned)
            .collect();
        let channel = owner
            .and_then(|owner| owner.get("title"))
            .and_then(get_text)
            .or_else(|| collaborators.first().cloned());
        let followers = get_count(owner.and_then(|owner| owner.get("subscriberCountText")))
            .filter(|count| *count != 0)
            .or_else(|| {
                collaborator_models.first().and_then(|model| {
                    at(model, &["rendererContext", "accessibilityContext", "label"])
                        .and_then(Value::as_str)
                        .and_then(parse_count)
                })
            });
        info.insert("channel".into(), channel.into());
        info.insert("channel_follower_count".into(), followers.into());
        info.insert(
            "creators".into(),
            if collaborators.is_empty() {
                Value::Null
            } else {
                json!(collaborators)
            },
        );
        if channel_handle.is_none() {
            channel_handle = owner.and_then(|owner| {
                let mut endpoints: Vec<&Value> =
                    owner.get("navigationEndpoint").into_iter().collect();
                endpoints.extend(
                    at(owner, &["title", "runs"])
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(|run| run.get("navigationEndpoint")),
                );
                endpoints.iter().find_map(|endpoint| {
                    at(endpoint, &["commandMetadata", "webCommandMetadata", "url"])
                        .or_else(|| at(endpoint, &["browseEndpoint", "canonicalBaseUrl"]))
                        .and_then(Value::as_str)
                        .and_then(handle_from_url)
                })
            });
        }
        let rows: Vec<&Value> = at(
            vsir,
            &[
                "metadataRowContainer",
                "metadataRowContainerRenderer",
                "rows",
            ],
        )
        .and_then(Value::as_array)
        .map(|rows| rows.iter().collect())
        .unwrap_or_default();
        let multiple_songs = rows.iter().any(|row| {
            at(row, &["metadataRowRenderer", "hasDividerLine"]) == Some(&Value::Bool(true))
        });
        for row in rows {
            let Some(renderer) = row.get("metadataRowRenderer") else {
                continue;
            };
            if !truthy(renderer.get("title")) {
                continue;
            }
            let title = renderer.get("title").and_then(get_text);
            let contents = at(renderer, &["contents", "0"]).and_then(get_text);
            match title.as_deref() {
                Some("License") => {
                    info.insert("license".into(), contents.into());
                }
                Some("Album") if !multiple_songs => {
                    info.insert("album".into(), contents.into());
                }
                Some("Artist") if !multiple_songs => {
                    info.insert(
                        "artists".into(),
                        contents.map_or(Value::Null, |artist| json!([artist])),
                    );
                }
                Some("Song") if !multiple_songs => {
                    info.insert("track".into(), contents.into());
                }
                _ => {}
            }
        }
        if badges(at(vsir, &["owner", "videoOwnerRenderer", "badges"])).contains(&Badge::Verified) {
            info.insert("channel_is_verified".into(), true.into());
        }
    }

    let uploader = info.get("channel").cloned().unwrap_or(Value::Null);
    info.insert("uploader".into(), uploader);
    info.insert("uploader_id".into(), channel_handle.clone().into());
    info.insert(
        "uploader_url".into(),
        channel_handle
            .as_ref()
            .map(|handle| format!("https://www.youtube.com/{handle}"))
            .into(),
    );

    // `parse_iso8601(..., timezone=NO_DEFAULT)`: only with a time and zone.
    let upload_source = get_first(&microformats, &["uploadDate"])
        .and_then(Value::as_str)
        .map(str::to_owned);
    let (date, timestamp) = upload_source
        .as_deref()
        .map(super::metadata::upload_time)
        .filter(|(date, _)| date.is_some())
        .or_else(|| search_meta("uploadDate").map(|meta| super::metadata::upload_time(&meta)))
        .unwrap_or((None, None));
    // Without a timestamp, the (UTC) `dateText` is preferred.
    let mut date = date;
    if date.is_none()
        || (timestamp.is_none() && matches!(input.live_status, None | Some("not_live")))
    {
        let parsed = vpir
            .and_then(|vpir| vpir.get("dateText"))
            .and_then(get_text)
            .and_then(|text| parse_time_text(&text))
            .and_then(crate::dates::strftime_date);
        if parsed.is_some() {
            date = parsed;
        }
    }
    info.insert("upload_date".into(), date.into());
    info.insert("timestamp".into(), timestamp.into());

    for (source, target) in [("artists", "creators"), ("track", "alt_title")] {
        if let Some(value) = info
            .get(source)
            .filter(|value| truthy(Some(value)))
            .cloned()
        {
            info.insert(target.into(), value);
        }
    }

    let page_badges = badges(vpir.and_then(|vpir| vpir.get("badges")));
    let is_private = if page_badges.contains(&Badge::Private) {
        Some(true)
    } else {
        get_first(&details, &["isPrivate"]).and_then(Value::as_bool)
    };
    let known = input.initial_data.is_some() && is_private.is_some();
    let availability = if page_badges.contains(&Badge::Public) {
        Some("public")
    } else {
        availability(
            is_private,
            // `(has_badge(...) or False) if initial_data and is_private is
            // not None else None`.
            known.then(|| page_badges.contains(&Badge::Premium)),
            known.then(|| page_badges.contains(&Badge::Subscription)),
            Some(age_limit >= 18),
            // `has_badge(UNLISTED) or get_first(..., 'isUnlisted')`.
            is_private.and_then(|_| {
                if page_badges.contains(&Badge::Unlisted) {
                    Some(true)
                } else {
                    get_first(&microformats, &["isUnlisted"]).and_then(Value::as_bool)
                }
            }),
        )
    };
    info.insert("availability".into(), availability.into());
    info
}

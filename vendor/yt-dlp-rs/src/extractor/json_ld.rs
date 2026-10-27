//! Upstream `_search_json_ld(html, video_id, default={})` (no expected
//! type): every `application/ld+json` script, and `_json_ld`'s
//! `VideoObject`/`AudioObject`, episode, movie and article fields, in
//! upstream's insertion order with `None` values filtered.
use regex::Regex;
use serde_json::{Map, Value};
use std::sync::LazyLock;

/// `_yield_json_ld`: the dicts of every parseable JSON-LD script.
pub(crate) fn yield_json_ld(html: &str) -> Vec<Value> {
    static SCRIPT: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r#"(?is)<script[^>]+type=(?:"application/ld\+json"|'application/ld\+json'|application/ld\+json)[^>]*>\s*(\{.+?\}|\[.+?\])\s*</script>"#,
        )
        .expect("static pattern")
    });
    let mut items = Vec::new();
    for found in SCRIPT.captures_iter(html) {
        // `strict=False`: raw control characters are accepted in strings.
        let text = escape_controls(&found[1]);
        match serde_json::from_str::<Value>(&text) {
            Ok(Value::Array(values)) => items.extend(values.into_iter().filter(Value::is_object)),
            Ok(value @ Value::Object(_)) => items.push(value),
            _ => {}
        }
    }
    items
}

/// Raw control characters inside JSON strings as `\u00XX` escapes.
fn escape_controls(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_string = false;
    let mut escaped = false;
    for c in text.chars() {
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            } else if (c as u32) < 0x20 {
                out.push_str(&format!("\\u{:04x}", c as u32));
                continue;
            }
        } else if c == '"' {
            in_string = true;
        }
        out.push(c);
    }
    out
}

/// `variadic(traverse_obj(e, '@type'))` contains one of `types`.
fn is_type(value: Option<&Value>, types: &[&str]) -> bool {
    match value.and_then(|value| value.get("@type")) {
        Some(Value::String(kind)) => types.contains(&kind.as_str()),
        Some(Value::Array(kinds)) => kinds
            .iter()
            .filter_map(Value::as_str)
            .any(|kind| types.contains(&kind)),
        _ => false,
    }
}

fn unescape(value: Option<&Value>) -> Value {
    value
        .and_then(Value::as_str)
        .map_or(Value::Null, |text| crate::html::unescape_html(text).into())
}

/// `int_or_none` of a JSON value.
fn int_or_none(value: Option<&Value>) -> Value {
    match value {
        Some(Value::Number(number)) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value.trunc() as i64))
            .map_or(Value::Null, Value::from),
        Some(Value::String(text)) => text.trim().parse::<i64>().map_or(Value::Null, Value::from),
        _ => Value::Null,
    }
}

fn float(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

/// `url_or_none`.
pub(crate) fn url_or_none(text: &str) -> Option<String> {
    static URL: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"^(?:(?:https?|rt(?:m(?:pt?[es]?|fp)|sp[su]?)|s(?:ftp|eek)|wss?|ftps?|mms)?:)?//",
        )
        .expect("static pattern")
    });
    let text = text.trim();
    URL.is_match(text).then(|| text.to_owned())
}

/// `str_to_int`: `,`, `.` and `+` removed.
fn str_to_int(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(number) => number.as_i64(),
        Value::String(text) => text.replace([',', '.', '+'], "").trim().parse().ok(),
        _ => None,
    }
}

fn set(info: &mut Map<String, Value>, key: &str, value: Value) {
    info.insert(key.to_owned(), value);
}

fn video_object(info: &mut Map<String, Value>, e: &Value) {
    let thumbnails: Vec<Value> = ["thumbnailUrl", "thumbnailURL", "thumbnail_url"]
        .iter()
        .filter_map(|key| e.get(*key))
        .flat_map(|value| match value {
            Value::Array(items) => items.iter().collect::<Vec<_>>(),
            other => vec![other],
        })
        .filter_map(Value::as_str)
        .map(crate::html::unescape_html)
        .map(|url| {
            if url.starts_with("//") {
                format!("https:{url}")
            } else {
                url
            }
        })
        .filter_map(|url| url_or_none(&url))
        .map(|url| serde_json::json!({ "url": url }))
        .collect();
    let author = e.get("author");
    set(
        info,
        "url",
        e.get("contentUrl")
            .and_then(Value::as_str)
            .and_then(url_or_none)
            .map_or(Value::Null, Value::from),
    );
    set(
        info,
        "ext",
        e.get("encodingFormat")
            .and_then(Value::as_str)
            .map(|mime| super::direct::mimetype2ext_or(mime, None))
            .filter(|ext| !ext.is_empty())
            .map_or(Value::Null, Value::from),
    );
    set(info, "title", unescape(e.get("name")));
    set(info, "description", unescape(e.get("description")));
    set(info, "thumbnails", Value::Array(thumbnails));
    set(
        info,
        "duration",
        e.get("duration")
            .and_then(Value::as_str)
            .and_then(crate::parse_duration)
            .map_or(Value::Null, |duration| {
                serde_json::to_value(duration).unwrap_or_default()
            }),
    );
    set(
        info,
        "timestamp",
        e.get("uploadDate")
            .and_then(Value::as_str)
            .and_then(|date| crate::dates::unified_timestamp(date, true))
            .map_or(Value::Null, Value::from),
    );
    set(
        info,
        "uploader",
        match author {
            Some(Value::Object(author)) => author.get("name").cloned().unwrap_or(Value::Null),
            Some(Value::String(author)) => author.clone().into(),
            _ => Value::Null,
        },
    );
    set(
        info,
        "artist",
        e.get("byArtist")
            .and_then(|artist| artist.get("name"))
            .filter(|name| name.is_string())
            .cloned()
            .unwrap_or(Value::Null),
    );
    set(
        info,
        "filesize",
        float(e.get("contentSize")).map_or(Value::Null, |size| Value::from(size.trunc() as i64)),
    );
    set(info, "tbr", int_or_none(e.get("bitrate")));
    set(info, "width", int_or_none(e.get("width")));
    set(info, "height", int_or_none(e.get("height")));
    set(info, "view_count", int_or_none(e.get("interactionCount")));
    set(
        info,
        "tags",
        e.get("keywords")
            .and_then(Value::as_str)
            .map_or(Value::Null, |keywords| {
                Value::Array(keywords.split(',').map(Value::from).collect())
            }),
    );
    if is_type(Some(e), &["AudioObject"]) {
        set(info, "vcodec", "none".into());
        set(info, "abr", int_or_none(e.get("bitrate")));
    }
    interaction_statistic(info, e);
    chapters(info, e);
}

fn interaction_statistic(info: &mut Map<String, Value>, e: &Value) {
    let statistics = match e.get("interactionStatistic") {
        Some(Value::Object(_)) => vec![e["interactionStatistic"].clone()],
        Some(Value::Array(items)) => items.clone(),
        _ => return,
    };
    for statistic in &statistics {
        if !is_type(Some(statistic), &["InteractionCounter"]) {
            continue;
        }
        let kind = match statistic.get("interactionType") {
            Some(Value::Object(kind)) => kind.get("@type").and_then(Value::as_str),
            Some(Value::String(kind)) => Some(kind.as_str()),
            _ => None,
        };
        let Some(kind) = kind.filter(|kind| !kind.is_empty()) else {
            continue;
        };
        let Some(count) = str_to_int(statistic.get("userInteractionCount")) else {
            continue;
        };
        let name = match kind.rsplit('/').next().unwrap_or(kind) {
            "CommentAction" => "comment",
            "AgreeAction" | "LikeAction" => "like",
            "DisagreeAction" | "DislikeAction" => "dislike",
            "ListenAction" | "WatchAction" | "ViewAction" => "view",
            _ => continue,
        };
        let key = format!("{name}_count");
        if info.get(&key).is_some_and(|value| !value.is_null()) {
            continue;
        }
        info.insert(key, count.into());
    }
}

/// `extract_chapter_information`: `hasPart` clips (with a warning and no
/// chapters for broken data).
fn chapters(info: &mut Map<String, Value>, e: &Value) {
    let parts: Vec<&Value> = match e.get("hasPart") {
        Some(Value::Array(parts)) => parts.iter().collect(),
        Some(part @ Value::Object(_)) => vec![part],
        _ => Vec::new(),
    };
    let mut chapters: Vec<Map<String, Value>> = parts
        .into_iter()
        .filter(|part| part.get("@type").and_then(Value::as_str) == Some("Clip"))
        .map(|part| {
            let field = |key: &str| part.get(key).cloned().unwrap_or(Value::Null);
            Map::from_iter([
                ("title".to_owned(), field("name")),
                ("start_time".to_owned(), field("startOffset")),
                ("end_time".to_owned(), field("endOffset")),
            ])
        })
        .collect();
    let truthy =
        |value: &Value| !(value.is_null() || value == &Value::from(0) || value == &Value::from(""));
    for index in 0..chapters.len() {
        if index + 1 >= chapters.len() {
            break;
        }
        let previous_end = if index == 0 {
            Value::from(0)
        } else {
            chapters[index - 1]["end_time"].clone()
        };
        let next_start = chapters[index + 1]["start_time"].clone();
        let current = &mut chapters[index];
        if !truthy(&current["end_time"]) {
            current.insert("end_time".into(), next_start);
        }
        if !truthy(&current["start_time"]) {
            current.insert("start_time".into(), previous_end);
        }
        if current.values().any(Value::is_null) {
            crate::screen::warning(&format!(
                "Chapter {index} contains broken data. Not extracting chapters"
            ));
            return;
        }
    }
    if let Some(last) = chapters.last_mut() {
        if !truthy(&last["end_time"]) {
            last.insert(
                "end_time".into(),
                info.get("duration").cloned().unwrap_or(Value::Null),
            );
        }
        info.insert(
            "chapters".into(),
            Value::Array(chapters.into_iter().map(Value::Object).collect()),
        );
    }
}

fn traverse(info: &mut Map<String, Value>, items: &[Value], top: bool) {
    for e in items.iter().filter(|e| e.is_object()) {
        if top && e.get("@context").is_none() {
            continue;
        }
        if top
            && e.as_object().is_some_and(|object| {
                object.len() == 2
                    && object.contains_key("@context")
                    && object.contains_key("@graph")
            })
        {
            let graph = match &e["@graph"] {
                Value::Array(items) => items.clone(),
                other => vec![other.clone()],
            };
            traverse(info, &graph, false);
            continue;
        }
        if let Some(rating) = float(
            e.get("aggregateRating")
                .and_then(|rating| rating.get("ratingValue")),
        ) {
            info.insert("average_rating".into(), rating.into());
        }
        if is_type(Some(e), &["TVEpisode", "Episode", "PodcastEpisode"]) {
            let name = unescape(e.get("name"));
            set(info, "episode", name.clone());
            set(info, "episode_number", int_or_none(e.get("episodeNumber")));
            set(info, "description", unescape(e.get("description")));
            if info
                .get("title")
                .is_none_or(|title| !title.as_str().is_some_and(|t| !t.is_empty()))
                && name.as_str().is_some_and(|name| !name.is_empty())
            {
                set(info, "title", name);
            }
            let season = e.get("partOfSeason");
            if is_type(season, &["TVSeason", "Season", "CreativeWorkSeason"]) {
                let season = season.expect("checked");
                set(info, "season", unescape(season.get("name")));
                set(
                    info,
                    "season_number",
                    int_or_none(season.get("seasonNumber")),
                );
            }
            let series = e
                .get("partOfSeries")
                .filter(|series| !series.is_null())
                .or_else(|| e.get("partOfTVSeries"));
            if is_type(series, &["TVSeries", "Series", "CreativeWorkSeries"]) {
                set(
                    info,
                    "series",
                    unescape(series.and_then(|series| series.get("name"))),
                );
            }
        } else if is_type(Some(e), &["Movie"]) {
            set(info, "title", unescape(e.get("name")));
            set(info, "description", unescape(e.get("description")));
            set(
                info,
                "duration",
                e.get("duration")
                    .and_then(Value::as_str)
                    .and_then(crate::parse_duration)
                    .map_or(Value::Null, |duration| {
                        serde_json::to_value(duration).unwrap_or_default()
                    }),
            );
            set(
                info,
                "timestamp",
                e.get("dateCreated")
                    .and_then(Value::as_str)
                    .and_then(|date| crate::dates::unified_timestamp(date, true))
                    .map_or(Value::Null, Value::from),
            );
        } else if is_type(Some(e), &["Article", "NewsArticle"]) {
            set(
                info,
                "timestamp",
                e.get("datePublished")
                    .and_then(Value::as_str)
                    .and_then(crate::dates::parse_iso8601)
                    .map_or(Value::Null, Value::from),
            );
            set(info, "title", unescape(e.get("headline")));
            let body = e
                .get("articleBody")
                .filter(|body| body.as_str().is_some_and(|body| !body.is_empty()))
                .or_else(|| e.get("description"));
            set(info, "description", unescape(body));
            let first = |key: &str| e.get(key).and_then(|items| items.get(0));
            if is_type(first("video"), &["VideoObject"]) {
                video_object(info, first("video").expect("checked"));
            } else if is_type(first("subjectOf"), &["VideoObject"]) {
                video_object(info, first("subjectOf").expect("checked"));
            }
        } else if is_type(Some(e), &["VideoObject", "AudioObject"]) {
            video_object(info, e);
            continue;
        }
        if is_type(e.get("video"), &["VideoObject"]) {
            video_object(info, &e["video"]);
        }
    }
}

/// `_search_json_ld(webpage, video_id, default={})`.
pub(crate) fn search(html: &str) -> Map<String, Value> {
    let mut info = Map::new();
    traverse(&mut info, &yield_json_ld(html), true);
    info.retain(|_, value| !value.is_null());
    info
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_upstream_results() {
        let cases: Vec<(String, Value)> =
            serde_json::from_str(include_str!("../../tests/fixtures/json_ld/cases.json")).unwrap();
        for (html, expected) in cases {
            assert_eq!(Value::Object(search(&html)), expected, "{html}");
        }
    }

    #[test]
    fn video_objects_follow_upstream_fields() {
        let html = r#"<script type="application/ld+json">{"@context":"https://schema.org","@type":"VideoObject","name":"A &amp; B","contentUrl":"https://e.invalid/a.mp4","thumbnailUrl":["//e.invalid/t.jpg","bad"],"duration":"PT1M2S","uploadDate":"2024-01-02T03:04:05Z","keywords":"a, b","interactionStatistic":{"@type":"InteractionCounter","interactionType":"https://schema.org/WatchAction","userInteractionCount":"1,234"}}</script>"#;
        let info = search(html);
        assert_eq!(
            Value::Object(info),
            serde_json::json!({
                "url": "https://e.invalid/a.mp4", "title": "A & B",
                "thumbnails": [{"url": "https://e.invalid/t.jpg"}], "duration": 62,
                "timestamp": 1704164645, "tags": ["a", " b"], "view_count": 1234,
            })
        );
        assert!(
            search("<script type='application/ld+json'>{\"@type\":\"VideoObject\"}</script>")
                .is_empty()
        );
    }
}

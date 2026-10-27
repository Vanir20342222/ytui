//! Upstream YouTube chapters from `ytInitialData`: the chaptered player bar,
//! then the macro-markers engagement panel, then description timestamps
//! (`_extract_chapters_helper`).
use crate::info::{Chapter, PyNumber};
use regex::Regex;
use serde_json::Value;
use std::sync::LazyLock;

fn float(value: PyNumber) -> f64 {
    value.as_f64()
}

/// Python `str()` of a number.
fn py_str(value: PyNumber) -> String {
    match value {
        PyNumber::Int(value) => value.to_string(),
        PyNumber::Float(value) => crate::screen::py_float(value),
    }
}

/// `_extract_chapters_helper(chapter_list, ..., duration, strict)`.
fn helper(
    list: Vec<(Option<PyNumber>, Option<String>)>,
    duration: Option<u64>,
    strict: bool,
) -> Vec<Chapter> {
    let Some(duration) = duration.filter(|duration| *duration > 0) else {
        return Vec::new();
    };
    let duration = duration as f64;
    let mut list = list;
    if !strict {
        list.sort_by(|a, b| {
            let key = |chapter: &(Option<PyNumber>, Option<String>)| chapter.0.map_or(0.0, float);
            key(a).total_cmp(&key(b))
        });
    }
    // Strict lists warn; description lists only write debug output.
    let warn = |message: String| {
        if strict {
            crate::screen::warning(&format!("[youtube] {message}"));
        }
    };
    let mut chapters: Vec<(Option<PyNumber>, Option<String>)> =
        vec![(Some(PyNumber::Int(0)), None)];
    for (index, chapter) in list.into_iter().enumerate() {
        let Some(start) = chapter.0 else {
            warn(format!("Incomplete chapter {index}"));
            continue;
        };
        let last = chapters.last().and_then(|last| last.0).map_or(0.0, float);
        if last <= float(start) && float(start) <= duration {
            chapters.push(chapter);
        } else if !chapters.contains(&chapter) {
            let issue = if float(start) > duration {
                format!("{} > {}", py_str(start), duration as u64)
            } else {
                format!(
                    "{} < {}",
                    py_str(start),
                    chapters
                        .last()
                        .and_then(|last| last.0)
                        .map_or("0".into(), py_str)
                )
            };
            warn(format!(
                "Invalid start time ({issue}) for chapter \"{}\"",
                chapter.1.as_deref().unwrap_or("None")
            ));
        }
    }
    chapters
        .into_iter()
        .skip(1)
        .map(|(start_time, title)| Chapter {
            start_time,
            title,
            end_time: None,
            extra: Default::default(),
        })
        .collect()
}

/// `float_or_none(value, scale=1000)`.
fn millis(value: &Value) -> Option<PyNumber> {
    let number = match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => crate::batch::py_strip(text).parse().ok(),
        _ => None,
    }?;
    Some(PyNumber::Float(number / 1000.0))
}

fn from_json(data: &Value, duration: Option<u64>) -> Vec<Chapter> {
    let Some(list) = data
        .pointer(
            "/playerOverlays/playerOverlayRenderer/decoratedPlayerBarRenderer/\
             decoratedPlayerBarRenderer/playerBar/chapteredPlayerBarRenderer/chapters",
        )
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    let list = list
        .iter()
        .map(|chapter| {
            let renderer = &chapter["chapterRenderer"];
            (
                millis(&renderer["timeRangeStartMillis"]),
                renderer["title"]["simpleText"].as_str().map(str::to_owned),
            )
        })
        .collect();
    helper(list, duration, true)
}

fn from_engagement_panel(data: &Value, duration: Option<u64>) -> Vec<Chapter> {
    let Some(panels) = data["engagementPanels"].as_array() else {
        return Vec::new();
    };
    panels
        .iter()
        .filter_map(|panel| {
            panel
                .pointer(
                    "/engagementPanelSectionListRenderer/content/macroMarkersListRenderer/contents",
                )
                .and_then(Value::as_array)
        })
        .map(|contents| {
            let list = contents
                .iter()
                .filter_map(|item| item.get("macroMarkersListItemRenderer"))
                .map(|chapter| {
                    let time = super::metadata::text(&chapter["timeDescription"])
                        .and_then(|time| crate::match_filter::parse_duration_number(&time));
                    (time, super::metadata::text(&chapter["title"]))
                })
                .collect();
            helper(list, duration, true)
        })
        .find(|chapters| !chapters.is_empty())
        .unwrap_or_default()
}

static DESCRIPTION_TIME_FIRST: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*((?:\d+:)?\d{1,2}:\d{2})\b\W*\s(.+?)\s*$").unwrap());
static DESCRIPTION_TIME_LAST: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*(.+?)\b\W*\s((?:\d+:)?\d{1,2}:\d{2})\s*$").unwrap());

/// `_extract_chapters_from_description(description, duration)`.
pub(crate) fn from_description(description: Option<&str>, duration: Option<u64>) -> Vec<Chapter> {
    let description = description.unwrap_or("");
    let found = |regex: &Regex, time: usize, title: usize| {
        regex
            .captures_iter(description)
            .map(|captures| {
                (
                    crate::match_filter::parse_duration_number(&captures[time]),
                    Some(captures[title].to_owned()),
                )
            })
            .collect::<Vec<_>>()
    };
    let chapters = helper(found(&DESCRIPTION_TIME_FIRST, 1, 2), duration, false);
    if !chapters.is_empty() {
        return chapters;
    }
    helper(found(&DESCRIPTION_TIME_LAST, 2, 1), duration, false)
}

/// Upstream `info['chapters']`, computed only when the page embeds initial
/// data (the `next` API fallback is not used).
pub(super) fn extract(
    initial_data: Option<&Value>,
    description: Option<&str>,
    duration: Option<u64>,
) -> Vec<Chapter> {
    let Some(data) = initial_data else {
        return Vec::new();
    };
    let chapters = from_json(data, duration);
    if !chapters.is_empty() {
        return chapters;
    }
    let chapters = from_engagement_panel(data, duration);
    if !chapters.is_empty() {
        return chapters;
    }
    from_description(description, duration)
}

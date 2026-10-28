//! Upstream `--download-sections`: `parse_chapters(..., advanced=True)`,
//! `download_range_func` and the per-section info dicts of
//! `process_video_result`, with Python `int`/`float` arithmetic.
use crate::{VideoInfo, info::PyNumber};
use regex::Regex;
use std::sync::LazyLock;

/// One requested time range or matched chapter (`start_time`/`end_time`).
#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub start_time: PyNumber,
    pub end_time: PyNumber,
    pub title: Option<String>,
    pub index: Option<usize>,
}

/// Parsed `--download-sections` values.
#[derive(Debug, Clone, Default)]
pub struct DownloadSections {
    chapters: Vec<Regex>,
    ranges: Vec<(PyNumber, PyNumber)>,
    from_url: bool,
}

static TIMESTAMP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?:(?P<start_sign>-?)(?P<start>[^-]+))?\s*-\s*(?:(?P<end_sign>-?)(?P<end>[^-]+))?$",
    )
    .unwrap()
});

fn float(value: PyNumber) -> f64 {
    value.as_f64()
}

/// Python `a + b` for `int`/`float`.
fn add(a: PyNumber, b: PyNumber) -> PyNumber {
    match (a, b) {
        (PyNumber::Int(a), PyNumber::Int(b)) => a
            .checked_add(b)
            .map_or(PyNumber::Float(a as f64 + b as f64), PyNumber::Int),
        (a, b) => PyNumber::Float(float(a) + float(b)),
    }
}

fn negate(value: PyNumber) -> PyNumber {
    match value {
        PyNumber::Int(value) => PyNumber::Int(-value),
        PyNumber::Float(value) => PyNumber::Float(-value),
    }
}

/// Python `min(a, b)`: the first of equal values.
fn min(a: PyNumber, b: PyNumber) -> PyNumber {
    if float(b) < float(a) { b } else { a }
}

/// Python truthiness of a number.
fn truthy(value: Option<PyNumber>) -> bool {
    value.is_some_and(|value| float(value) != 0.0)
}

/// Chapter regexes, `(start, end)` ranges and whether `*from-url` was given.
pub type ParsedChapters = (Vec<Regex>, Vec<(PyNumber, PyNumber)>, bool);

/// Upstream `parse_chapters(name, value, advanced)`: chapter regexes, time
/// ranges and (`advanced` only) `*from-url`.
pub fn parse_chapters(
    name: &str,
    values: &[String],
    advanced: bool,
) -> Result<ParsedChapters, String> {
    let (mut chapters, mut ranges, mut from_url) = (Vec::new(), Vec::new(), false);
    for value in values {
        if advanced && value == "*from-url" {
            from_url = true;
            continue;
        }
        let Some(spec) = value.strip_prefix('*') else {
            let regex = Regex::new(value)
                .map_err(|error| format!("invalid {name} regex \"{value}\" - {error}"))?;
            chapters.push(regex);
            continue;
        };
        for range in spec.split(',').map(|range| range.trim()) {
            let invalid = |reason: &str| format!("invalid {name} time range \"{value}\". {reason}");
            let timestamp = |text: &str| {
                if matches!(text, "inf" | "infinite") {
                    Some(PyNumber::Float(f64::INFINITY))
                } else {
                    crate::match_filter::parse_duration_number(text)
                }
            };
            let captures = (range != "-").then(|| TIMESTAMP.captures(range)).flatten();
            let parsed = captures.as_ref().and_then(|captures| {
                let part = |name: &str, default: &str| {
                    timestamp(captures.name(name).map_or(default, |m| m.as_str()))
                };
                Some((part("start", "0")?, part("end", "inf")?))
            });
            let (Some(captures), Some((mut start, mut end))) = (captures, parsed) else {
                return Err(invalid("Must be of the form \"*start-end\""));
            };
            let sign = |group: &str| captures.name(group).is_some_and(|m| m.as_str() == "-");
            if !advanced && (sign("start_sign") || sign("end_sign")) {
                return Err(invalid("Negative timestamps are not allowed"));
            }
            if sign("start_sign") {
                start = negate(start);
            }
            if sign("end_sign") {
                end = negate(end);
            }
            if float(end) == f64::NEG_INFINITY {
                return Err(invalid("\"-inf\" is not a valid end"));
            }
            ranges.push((start, end));
        }
    }
    Ok((chapters, ranges, from_url))
}

impl DownloadSections {
    /// `parse_chapters('--download-sections', values, advanced=True)`; the
    /// error text follows upstream (regex messages are this port's).
    pub fn parse(values: &[String]) -> Result<Self, String> {
        let (chapters, ranges, from_url) = parse_chapters("--download-sections", values, true)?;
        Ok(Self {
            chapters,
            ranges,
            from_url,
        })
    }

    /// `download_range_func(info_dict, ydl)`: `None` stands for `{}` (the
    /// whole video). Chapter regexes search the chapter titles.
    pub fn requested(&self, info: &VideoInfo) -> Vec<Option<Section>> {
        let chapters = &info.metadata.chapters;
        let mut requested: Vec<Option<Section>> = Vec::new();
        for regex in &self.chapters {
            for (index, chapter) in chapters.iter().enumerate() {
                if regex.is_match(chapter.title.as_deref().unwrap_or("")) {
                    requested.push(Some(Section {
                        start_time: chapter.start_time.unwrap_or(PyNumber::Int(0)),
                        end_time: chapter.end_time.unwrap_or(PyNumber::Float(f64::INFINITY)),
                        title: chapter.title.clone(),
                        index: Some(index),
                    }));
                }
            }
        }
        if !self.chapters.is_empty() && requested.is_empty() {
            let warning = if chapters.is_empty() {
                "Cannot match chapters since chapter information is unavailable"
            } else {
                "There are no chapters matching the regex"
            };
            crate::screen::screen(&format!("[info] {}: {warning}", info.id));
        }
        let duration = info
            .metadata
            .duration
            .filter(|duration| duration.as_f64() > 0.0);
        // `_handle_negative_timestamp`.
        let resolve = |time: PyNumber| match duration {
            Some(duration) if float(time) < 0.0 => {
                let sum = add(duration, time);
                if float(sum) >= 0.0 {
                    sum
                } else {
                    PyNumber::Int(0)
                }
            }
            _ => time,
        };
        requested.extend(self.ranges.iter().map(|(start, end)| {
            Some(Section {
                start_time: resolve(*start),
                end_time: resolve(*end),
                title: None,
                index: None,
            })
        }));
        let (start, end) = (info.metadata.start_time, info.metadata.end_time);
        if self.from_url && (truthy(start) || truthy(end)) {
            requested.push(Some(Section {
                start_time: start
                    .filter(|start| start.as_f64() != 0.0)
                    .unwrap_or(PyNumber::Int(0)),
                end_time: end
                    .filter(|end| end.as_f64() != 0.0)
                    .unwrap_or(PyNumber::Float(f64::INFINITY)),
                title: None,
                index: None,
            }));
        } else if self.ranges.is_empty() && self.chapters.is_empty() {
            requested.push(None);
        }
        requested
    }

    /// Whether the option was given with anything but the whole video.
    pub fn is_empty(&self) -> bool {
        self.chapters.is_empty() && self.ranges.is_empty() && !self.from_url
    }
}

/// `f'{value:.1f}'`.
fn one_decimal(value: PyNumber) -> String {
    let value = float(value);
    if value.is_infinite() {
        if value > 0.0 { "inf" } else { "-inf" }.into()
    } else {
        format!("{value:.1}")
    }
}

/// The `Downloading N time ranges: START-END, ...` list.
pub fn describe(sections: &[Option<Section>]) -> String {
    sections
        .iter()
        .map(|section| match section {
            Some(section) => format!(
                "{}-{}",
                one_decimal(section.start_time),
                one_decimal(section.end_time)
            ),
            None => "None-None".into(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The per-section info dict of `process_video_result`.
pub fn apply(info: &VideoInfo, section: Option<&Section>) -> VideoInfo {
    let mut new = info.clone();
    let offset = info
        .section_start
        .filter(|start| float(*start) != 0.0)
        .unwrap_or(PyNumber::Int(0));
    let duration = info
        .metadata
        .duration
        .filter(|duration| duration.as_f64() > 0.0)
        .unwrap_or(PyNumber::Float(f64::INFINITY));
    let end = section.map_or(duration, |section| section.end_time);
    let end_time = add(offset, min(end, duration));
    // Durations may be inaccurate, so deviations under a second are allowed.
    let end_time = (float(end_time) != f64::INFINITY
        && float(end_time) <= float(offset) + float(duration) + 1.0)
        .then_some(end_time);
    if section.is_some() || truthy(Some(offset)) {
        new.section_start = Some(add(
            offset,
            section.map_or(PyNumber::Int(0), |section| section.start_time),
        ));
        new.section_end = end_time;
        new.section_title = section.and_then(|section| section.title.clone());
        new.section_number = section.and_then(|section| section.index);
    }
    new
}

/// Whether `info` is a partial download (upstream: `section_start or
/// section_end`), which needs the FFmpeg downloader.
pub fn is_partial(info: &VideoInfo) -> bool {
    truthy(info.section_start) || truthy(info.section_end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn info(duration: Option<u64>) -> VideoInfo {
        let mut info: VideoInfo = serde_json::from_value(json!({"id":"v","title":"V",
            "url":"https://example.test/v.mp4","webpage_url":"https://example.test/v.mp4",
            "ext":"mp4","extractor":"generic","extractor_key":"Generic"}))
        .unwrap();
        info.metadata.duration = duration.map(|duration| PyNumber::Int(duration as i64));
        info
    }

    fn parse(values: &[&str]) -> Result<DownloadSections, String> {
        DownloadSections::parse(&values.iter().map(|v| (*v).to_owned()).collect::<Vec<_>>())
    }

    #[test]
    fn ranges_and_errors_follow_parse_chapters() {
        let sections = parse(&["*2-4,6-7", "*-3-inf", "*-5", "*1:30-2:00.5"]).unwrap();
        assert_eq!(
            describe(&sections.requested(&info(None))),
            "2.0-4.0, 6.0-7.0, -3.0-inf, 0.0-5.0, 90.0-120.5"
        );
        assert_eq!(
            describe(&sections.requested(&info(Some(10)))),
            "2.0-4.0, 6.0-7.0, 7.0-inf, 0.0-5.0, 90.0-120.5"
        );
        for (value, message) in [
            (
                "*a-b",
                "invalid --download-sections time range \"*a-b\". Must be of the form \"*start-end\"",
            ),
            (
                "*1--inf",
                "invalid --download-sections time range \"*1--inf\". \"-inf\" is not a valid end",
            ),
            (
                "*-",
                "invalid --download-sections time range \"*-\". Must be of the form \"*start-end\"",
            ),
        ] {
            assert_eq!(parse(&[value]).unwrap_err(), message);
        }
        assert!(
            parse(&["(bad"])
                .unwrap_err()
                .starts_with("invalid --download-sections regex \"(bad\" - ")
        );
        assert!(parse(&["intro"]).unwrap().requested(&info(None)).is_empty());
        assert_eq!(parse(&[]).unwrap().requested(&info(None)), vec![None]);
    }

    #[test]
    fn from_url_uses_extracted_start_and_end_times() {
        let sections = parse(&["*from-url"]).unwrap();
        let mut video = info(Some(10));
        assert_eq!(sections.requested(&video), vec![None]);
        video.metadata.start_time = Some(PyNumber::Int(3));
        assert_eq!(describe(&sections.requested(&video)), "3.0-inf");
        let applied = apply(&video, sections.requested(&video)[0].as_ref());
        assert_eq!(
            serde_json::to_value((applied.section_start, applied.section_end)).unwrap(),
            json!([3, 10])
        );
        video.metadata.end_time = Some(PyNumber::Float(4.5));
        let with_range = parse(&["*from-url", "*1-2"]).unwrap();
        assert_eq!(describe(&with_range.requested(&video)), "1.0-2.0, 3.0-4.5");
    }

    #[test]
    fn section_fields_follow_process_video_result() {
        let sections = parse(&["*2-4", "*-3-inf", "*0-inf"]).unwrap();
        let unknown = info(None);
        let fields = |info: &VideoInfo| {
            serde_json::to_value((info.section_start, info.section_end)).unwrap()
        };
        let requested = sections.requested(&unknown);
        // Checked against pinned upstream: `2 4`, `-3 NA`, `0 NA`.
        let applied: Vec<_> = requested
            .iter()
            .map(|s| apply(&unknown, s.as_ref()))
            .collect();
        assert_eq!(fields(&applied[0]), json!([2, 4]));
        assert_eq!(fields(&applied[1]), json!([-3, null]));
        assert_eq!(fields(&applied[2]), json!([0, null]));
        assert!(is_partial(&applied[0]) && is_partial(&applied[1]) && !is_partial(&applied[2]));
        let known = info(Some(10));
        let applied = apply(&known, sections.requested(&known)[1].as_ref());
        assert_eq!(fields(&applied), json!([7, 10]));
        assert_eq!(apply(&known, None).section_start, None);
    }
}

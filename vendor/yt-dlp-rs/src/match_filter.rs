//! Upstream `--match-filters` (`match_str`/`_match_one`) over the JSON form of
//! represented metadata. Python regexes are compiled with the Rust `regex`
//! crate; lookaround and backreferences are rejected at parse time.

use crate::{Error, Result, VideoInfo};
use regex::Regex;
use serde_json::Value;
use std::sync::LazyLock;

fn invalid(message: impl Into<String>) -> Error {
    Error::MatchFilter(message.into())
}

/// Upstream `YoutubeDL._format_fields`: keys only known after format selection,
/// treated as incomplete (passing when missing) before selection.
pub const FORMAT_FIELDS: &[&str] = &[
    "url",
    "manifest_url",
    "manifest_stream_number",
    "ext",
    "format",
    "format_id",
    "format_note",
    "available_at",
    "width",
    "height",
    "aspect_ratio",
    "resolution",
    "dynamic_range",
    "tbr",
    "abr",
    "acodec",
    "asr",
    "audio_channels",
    "vbr",
    "fps",
    "vcodec",
    "container",
    "filesize",
    "filesize_approx",
    "rows",
    "columns",
    "hls_media_playlist_data",
    "player_url",
    "protocol",
    "fragment_base_url",
    "fragments",
    "is_from_start",
    "is_dash_periods",
    "request_data",
    "preference",
    "language",
    "language_preference",
    "quality",
    "source_preference",
    "cookies",
    "http_headers",
    "stretched_ratio",
    "no_resume",
    "has_drm",
    "extra_param_to_segment_url",
    "extra_param_to_key_url",
    "hls_aes",
    "downloader_options",
    "impersonate",
    "page_url",
    "app",
    "play_path",
    "tc_url",
    "flash_version",
    "rtmp_live",
    "rtmp_conn",
    "rtmp_protocol",
    "rtmp_real_time",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Contains,
    StartsWith,
    EndsWith,
    Search,
    Le,
    Lt,
    Ge,
    Gt,
    Eq,
}

impl Op {
    // Upstream `COMPARISON_OPERATORS` order: string operators, then "<=" before "<".
    const ALL: [(&'static str, Op); 9] = [
        ("*=", Op::Contains),
        ("^=", Op::StartsWith),
        ("$=", Op::EndsWith),
        ("~=", Op::Search),
        ("<=", Op::Le),
        ("<", Op::Lt),
        (">=", Op::Ge),
        (">", Op::Gt),
        ("=", Op::Eq),
    ];

    fn symbol(self) -> &'static str {
        Self::ALL.iter().find(|(_, op)| *op == self).unwrap().0
    }

    fn string_only(self) -> bool {
        matches!(
            self,
            Op::Contains | Op::StartsWith | Op::EndsWith | Op::Search
        )
    }
}

#[derive(Debug, Clone)]
enum Part {
    Compare {
        key: String,
        negated: bool,
        op: Op,
        none_inclusive: bool,
        value: String,
        regex: Option<Regex>,
    },
    Unary {
        negated: bool,
        key: String,
    },
}

#[derive(Debug, Clone, Copy)]
enum Number {
    Int(i128),
    Float(f64),
}

impl Number {
    fn float(self) -> f64 {
        match self {
            Number::Int(value) => value as f64,
            Number::Float(value) => value,
        }
    }

    fn compare(self, other: Number) -> Option<std::cmp::Ordering> {
        match (self, other) {
            (Number::Int(a), Number::Int(b)) => Some(a.cmp(&b)),
            (a, b) => a.float().partial_cmp(&b.float()),
        }
    }
}

fn py_strip(text: &str) -> &str {
    crate::batch::py_strip(text)
}

fn is_key(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
}

/// One `&`-separated condition.
fn parse_part(part: &str) -> Result<Part> {
    let text = py_strip(part);
    // `.` in the upstream patterns does not match newlines.
    if !text.contains('\n') {
        let key_end = text
            .find(|c: char| !(c.is_ascii_lowercase() || c == '_'))
            .unwrap_or(text.len());
        if key_end > 0 {
            let key = &text[..key_end];
            let mut rest = text[key_end..].trim_start_matches(crate::batch::is_py_space);
            let negated = rest.starts_with('!');
            if negated {
                rest = rest[1..].trim_start_matches(crate::batch::is_py_space);
            }
            if let Some((symbol, op)) = Op::ALL.iter().find(|(symbol, _)| rest.starts_with(symbol))
            {
                let mut rest = &rest[symbol.len()..];
                let after_spaces = rest.trim_start_matches(crate::batch::is_py_space);
                let none_inclusive = after_spaces.starts_with('?');
                if none_inclusive {
                    rest = &after_spaces[1..];
                }
                let rest = rest.trim_start_matches(crate::batch::is_py_space);
                let quote = rest.chars().next().filter(|c| matches!(c, '"' | '\''));
                let value = match quote {
                    Some(quote) if rest.len() >= 3 && rest.ends_with(quote) => {
                        rest[1..rest.len() - 1].replace(&format!("\\{quote}"), &quote.to_string())
                    }
                    _ if !rest.is_empty() => rest.to_owned(),
                    _ => return Err(invalid(format!("Invalid filter part {part:?}"))),
                };
                let regex = (*op == Op::Search)
                    .then(|| {
                        Regex::new(&value).map_err(|_| {
                            invalid(format!("unsupported regular expression in filter {part:?}"))
                        })
                    })
                    .transpose()?;
                return Ok(Part::Compare {
                    key: key.into(),
                    negated,
                    op: *op,
                    none_inclusive,
                    value,
                    regex,
                });
            }
        }
        let (negated, key) = match text.strip_prefix('!') {
            Some(rest) => (true, rest.trim_start_matches(crate::batch::is_py_space)),
            None => (false, text),
        };
        if is_key(key) {
            return Ok(Part::Unary {
                negated,
                key: key.into(),
            });
        }
    }
    Err(invalid(format!("Invalid filter part {part:?}")))
}

/// Python `int(text)` for ASCII digits with optional sign and single underscores.
fn py_int(text: &str) -> Option<i128> {
    let text = py_strip(text);
    let (negative, digits) = match text.as_bytes().first()? {
        b'-' => (true, &text[1..]),
        b'+' => (false, &text[1..]),
        _ => (false, text),
    };
    if digits.is_empty()
        || digits.starts_with('_')
        || digits.ends_with('_')
        || digits.contains("__")
        || !digits.bytes().all(|b| b.is_ascii_digit() || b == b'_')
    {
        return None;
    }
    let value: i128 = digits.replace('_', "").parse().ok()?;
    Some(if negative { -value } else { value })
}

const FILESIZE_UNITS: &[(&str, u32, u32)] = &[
    ("B", 1, 0),
    ("b", 1, 0),
    ("bytes", 1, 0),
    ("KiB", 1024, 1),
    ("KB", 1000, 1),
    ("kB", 1024, 1),
    ("Kb", 1000, 1),
    ("kb", 1000, 1),
    ("kilobytes", 1000, 1),
    ("kibibytes", 1024, 1),
    ("MiB", 1024, 2),
    ("MB", 1000, 2),
    ("mB", 1024, 2),
    ("Mb", 1000, 2),
    ("mb", 1000, 2),
    ("megabytes", 1000, 2),
    ("mebibytes", 1024, 2),
    ("GiB", 1024, 3),
    ("GB", 1000, 3),
    ("gB", 1024, 3),
    ("Gb", 1000, 3),
    ("gb", 1000, 3),
    ("gigabytes", 1000, 3),
    ("gibibytes", 1024, 3),
    ("TiB", 1024, 4),
    ("TB", 1000, 4),
    ("tB", 1024, 4),
    ("Tb", 1000, 4),
    ("tb", 1000, 4),
    ("terabytes", 1000, 4),
    ("tebibytes", 1024, 4),
    ("PiB", 1024, 5),
    ("PB", 1000, 5),
    ("pB", 1024, 5),
    ("Pb", 1000, 5),
    ("pb", 1000, 5),
    ("petabytes", 1000, 5),
    ("pebibytes", 1024, 5),
    ("EiB", 1024, 6),
    ("EB", 1000, 6),
    ("eB", 1024, 6),
    ("Eb", 1000, 6),
    ("eb", 1000, 6),
    ("exabytes", 1000, 6),
    ("exbibytes", 1024, 6),
    ("ZiB", 1024, 7),
    ("ZB", 1000, 7),
    ("zB", 1024, 7),
    ("Zb", 1000, 7),
    ("zb", 1000, 7),
    ("zettabytes", 1000, 7),
    ("zebibytes", 1024, 7),
    ("YiB", 1024, 8),
    ("YB", 1000, 8),
    ("yB", 1024, 8),
    ("Yb", 1000, 8),
    ("yb", 1000, 8),
    ("yottabytes", 1000, 8),
    ("yobibytes", 1024, 8),
];

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Upstream `parse_filesize` (non-strict `lookup_unit_table`, prefix match).
fn parse_filesize(text: &str) -> Option<Number> {
    let digits = text
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(text.len());
    if digits == 0 {
        return None;
    }
    let mut end = digits;
    if let Some(fraction) = text[end..].strip_prefix([',', '.']) {
        let length = fraction
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(fraction.len());
        if length > 0 {
            end += 1 + length;
        }
    }
    let number: f64 = text[..end].replace(',', ".").parse().ok()?;
    let rest = text[end..].trim_start_matches(crate::batch::is_py_space);
    let (_, base, power) = FILESIZE_UNITS.iter().find(|(unit, _, _)| {
        rest.strip_prefix(unit).is_some_and(|after| {
            // `\b` after a unit ending in a word character.
            !after.chars().next().is_some_and(is_word)
        })
    })?;
    let value = (number * f64::from(*base).powi(*power as i32)).round_ties_even();
    Some(if value.abs() < 1e18 {
        Number::Int(value as i128)
    } else {
        Number::Float(value)
    })
}

static CLOCK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?:(?:(?:(?P<days>[0-9]+):)?(?P<hours>[0-9]+):)?(?P<mins>[0-9]+):(?P<secs>[0-9]{1,2})|(?P<bare>[0-9]+))(?P<ms>[.:][0-9]+)?Z?$",
    )
    .unwrap()
});
static UNITS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?ix)^(?:P?
            (?:[0-9]+\s*y(?:ears?)?,?\s*)?
            (?:[0-9]+\s*m(?:onths?)?,?\s*)?
            (?:[0-9]+\s*w(?:eeks?)?,?\s*)?
            (?:(?P<days>[0-9]+)\s*d(?:ays?)?,?\s*)?
            T)?
            (?:(?P<hours>[0-9]+)\s*h(?:(?:ou)?rs?)?,?\s*)?
            (?:(?P<mins>[0-9]+)\s*m(?:in(?:ute)?s?)?,?\s*)?
            (?:(?P<secs>[0-9]+)(?P<ms>\.[0-9]+)?\s*s(?:ec(?:ond)?s?)?\s*)?Z?$",
    )
    .unwrap()
});
static DECIMAL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^(?:(?P<hours>[0-9.]+)\s*(?:hours?)|(?P<mins>[0-9.]+)\s*(?:mins?\.?|minutes?)\s*)Z?$",
    )
    .unwrap()
});

/// Upstream `parse_duration` as a Python `int` or `float`.
pub(crate) fn parse_duration_number(text: &str) -> Option<crate::info::PyNumber> {
    parse_duration(text).map(|number| match number {
        Number::Int(value) => i64::try_from(value).map_or(
            crate::info::PyNumber::Float(value as f64),
            crate::info::PyNumber::Int,
        ),
        Number::Float(value) => crate::info::PyNumber::Float(value),
    })
}

/// Upstream `parse_duration`.
fn parse_duration(text: &str) -> Option<Number> {
    let text = py_strip(text);
    if text.is_empty() {
        return None;
    }
    let group = |captures: &regex::Captures<'_>, name: &str| {
        captures.name(name).map(|m| m.as_str().to_owned())
    };
    let (days, hours, mins, secs, ms) = if let Some(captures) = CLOCK.captures(text) {
        let secs = group(&captures, "secs").or_else(|| group(&captures, "bare"));
        (
            group(&captures, "days"),
            group(&captures, "hours"),
            group(&captures, "mins"),
            secs,
            group(&captures, "ms"),
        )
    } else if let Some(captures) = UNITS.captures(text) {
        (
            group(&captures, "days"),
            group(&captures, "hours"),
            group(&captures, "mins"),
            group(&captures, "secs"),
            group(&captures, "ms"),
        )
    } else {
        let captures = DECIMAL.captures(text)?;
        (
            None,
            group(&captures, "hours"),
            group(&captures, "mins"),
            None,
            None,
        )
    };
    let ms = ms.map(|ms| ms.replace(':', "."));
    let mut total = 0.0;
    for (part, multiplier) in [
        (days, 86400.0),
        (hours, 3600.0),
        (mins, 60.0),
        (secs, 1.0),
        (ms, 1.0),
    ] {
        if let Some(part) = part {
            total += part.parse::<f64>().ok()? * multiplier;
        }
    }
    Some(if total.fract() == 0.0 && total.abs() < 1e18 {
        Number::Int(total as i128)
    } else {
        Number::Float(total)
    })
}

fn numeric(value: &Value) -> Option<Number> {
    match value {
        Value::Bool(value) => Some(Number::Int(i128::from(*value))),
        Value::Number(number) => Some(
            number
                .as_i64()
                .map(|value| Number::Int(value.into()))
                .or_else(|| number.as_u64().map(|value| Number::Int(value.into())))
                .unwrap_or_else(|| Number::Float(number.as_f64().unwrap_or(f64::NAN))),
        ),
        _ => None,
    }
}

fn type_error(op: Op) -> Error {
    invalid(format!(
        "operator {} is not supported for this field type",
        op.symbol()
    ))
}

impl Part {
    fn matches(
        &self,
        dict: &serde_json::Map<String, Value>,
        incomplete: &dyn Fn(&str) -> bool,
    ) -> Result<bool> {
        match self {
            Part::Compare {
                key,
                negated,
                op,
                none_inclusive,
                value,
                regex,
            } => {
                let actual = dict.get(key).filter(|value| !value.is_null());
                let number = match actual.and_then(numeric) {
                    Some(_) => py_int(value)
                        .map(Number::Int)
                        .or_else(|| parse_filesize(value))
                        .or_else(|| parse_filesize(&format!("{value}B")))
                        .or_else(|| parse_duration(value)),
                    None => None,
                };
                if number.is_some() && op.string_only() {
                    return Err(invalid(format!(
                        "Operator {} only supports string values!",
                        op.symbol()
                    )));
                }
                let Some(actual) = actual else {
                    return Ok(incomplete(key) || *none_inclusive);
                };
                let result = match (numeric(actual), number) {
                    (Some(actual), Some(expected)) => {
                        let ordering = actual.compare(expected);
                        match op {
                            Op::Eq => ordering == Some(std::cmp::Ordering::Equal),
                            Op::Lt => ordering == Some(std::cmp::Ordering::Less),
                            Op::Le => ordering.is_some_and(|o| o.is_le()),
                            Op::Gt => ordering == Some(std::cmp::Ordering::Greater),
                            Op::Ge => ordering.is_some_and(|o| o.is_ge()),
                            _ => unreachable!("string operators were rejected"),
                        }
                    }
                    // A number never equals a string; ordering is a TypeError.
                    (Some(_), None) => match op {
                        Op::Eq => false,
                        _ => return Err(type_error(*op)),
                    },
                    (None, _) => match actual {
                        Value::String(actual) => match op {
                            Op::Contains => actual.contains(value.as_str()),
                            Op::StartsWith => actual.starts_with(value.as_str()),
                            Op::EndsWith => actual.ends_with(value.as_str()),
                            Op::Search => {
                                regex.as_ref().expect("compiled at parse").is_match(actual)
                            }
                            Op::Eq => actual == value,
                            Op::Lt => actual.as_str() < value.as_str(),
                            Op::Le => actual.as_str() <= value.as_str(),
                            Op::Gt => actual.as_str() > value.as_str(),
                            Op::Ge => actual.as_str() >= value.as_str(),
                        },
                        // `in` tests list membership; equality with a string is false.
                        Value::Array(items) => match op {
                            Op::Contains => items.iter().any(|item| item.as_str() == Some(value)),
                            Op::Eq => false,
                            _ => return Err(type_error(*op)),
                        },
                        _ => match op {
                            Op::Eq => false,
                            _ => return Err(type_error(*op)),
                        },
                    },
                };
                Ok(result != *negated)
            }
            Part::Unary { negated, key } => {
                let actual = dict.get(key).filter(|value| !value.is_null());
                if actual.is_none() && incomplete(key) {
                    return Ok(true);
                }
                Ok(match (actual, negated) {
                    (Some(Value::Bool(value)), false) => *value,
                    (Some(Value::Bool(value)), true) => !*value,
                    (actual, false) => actual.is_some(),
                    (actual, true) => actual.is_none(),
                })
            }
        }
    }
}

/// One `--match-filters` value: `&`-joined conditions (`\&` is literal).
#[derive(Debug, Clone)]
struct Conjunction {
    text: String,
    parts: Vec<Part>,
}

impl Conjunction {
    fn parse(text: &str) -> Result<Self> {
        let mut parts = Vec::new();
        let mut current = String::new();
        let mut previous = None;
        for c in text.chars() {
            if c == '&' && previous != Some('\\') {
                parts.push(std::mem::take(&mut current));
            } else {
                current.push(c);
            }
            previous = Some(c);
        }
        parts.push(current);
        Ok(Self {
            text: text.into(),
            parts: parts
                .iter()
                .map(|part| parse_part(&part.replace("\\&", "&")))
                .collect::<Result<_>>()?,
        })
    }

    fn matches(
        &self,
        dict: &serde_json::Map<String, Value>,
        incomplete: &dyn Fn(&str) -> bool,
    ) -> Result<bool> {
        for part in &self.parts {
            if !part.matches(dict, incomplete)? {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

/// The `_match_entry` checks preceding the match filters: title patterns,
/// upload date range, view counts and age limit.
#[derive(Debug, Clone, Default)]
pub struct EntrySelection {
    /// `--match-title`: the pattern text and its case-insensitive regex.
    pub match_title: Option<(String, Regex)>,
    /// `--reject-title`.
    pub reject_title: Option<(String, Regex)>,
    pub date_range: Option<crate::dates::DateRange>,
    pub min_views: Option<i64>,
    pub max_views: Option<i64>,
    pub age_limit: Option<i64>,
    /// `--break-on-reject`: any rejection cancels the queue.
    pub break_on_reject: bool,
    /// UNIX seconds for relative dates in `upload_date` values.
    pub now: i64,
}

impl EntrySelection {
    /// A case-insensitive title pattern (Rust regex syntax).
    pub fn title_pattern(pattern: &str) -> Result<(String, Regex)> {
        regex::RegexBuilder::new(pattern)
            .case_insensitive(true)
            .build()
            .map(|regex| (pattern.to_owned(), regex))
            .map_err(|error| invalid(format!("invalid title regex \"{pattern}\": {error}")))
    }

    fn is_empty(&self) -> bool {
        self.match_title.is_none()
            && self.reject_title.is_none()
            && self.date_range.is_none()
            && self.min_views.is_none()
            && self.max_views.is_none()
            && self.age_limit.is_none()
    }

    /// `check_filter` up to the match filter: the rejection reason, if any.
    fn rejection(&self, info: &VideoInfo) -> Result<Option<String>> {
        let title = &info.title;
        if let Some((pattern, regex)) = &self.match_title
            && !regex.is_match(title)
        {
            return Ok(Some(format!(
                "\"{title}\" title did not match pattern \"{pattern}\""
            )));
        }
        if let Some((pattern, regex)) = &self.reject_title
            && regex.is_match(title)
        {
            return Ok(Some(format!(
                "\"{title}\" title matched reject pattern \"{pattern}\""
            )));
        }
        if let (Some(range), Some(date)) = (&self.date_range, &info.metadata.upload_date)
            && let Some(iso) = range
                .reject(date, self.now)
                .map_err(|error| invalid(format!("invalid upload_date \"{date}\": {error}")))?
        {
            return Ok(Some(format!("{iso} upload date is not in range {range}")));
        }
        if let Some(views) = info.metadata.view_count.map(i128::from) {
            if let Some(min) = self.min_views
                && views < i128::from(min)
            {
                return Ok(Some(format!(
                    "Skipping {title}, because it has not reached minimum view count ({views}/{min})"
                )));
            }
            if let Some(max) = self.max_views
                && views > i128::from(max)
            {
                return Ok(Some(format!(
                    "Skipping {title}, because it has exceeded the maximum view count ({views}/{max})"
                )));
            }
        }
        // `age_restricted(content_limit, age_limit)`.
        if let (Some(limit), Some(content)) = (self.age_limit, info.metadata.age_limit)
            && limit < i64::from(content)
        {
            return Ok(Some(format!(
                "Skipping \"{title}\" because it is age restricted"
            )));
        }
        Ok(None)
    }
}

/// Upstream `match_filter_func`: any matching filter passes; breaking filters
/// cancel the queue when they reject.
#[derive(Debug, Clone, Default)]
pub struct MatchFilters {
    filters: Vec<Conjunction>,
    breaking: Vec<Conjunction>,
    selection: EntrySelection,
    /// `--match-filters -`: ask whether to download each passing video.
    interactive: bool,
}

/// The outcome for one video.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatchOutcome {
    Pass,
    /// Skip with upstream's `[download]` reason.
    Reject(String),
    /// A `--break-match-filters` rejection: cancel with this reason.
    Break(String),
    /// `--match-filters -` (upstream `NO_DEFAULT`): ask the user.
    Ask,
}

impl MatchFilters {
    /// Parse repeated `--match-filters` and `--break-match-filters` values.
    /// Like upstream, filters are a set; `-` among the filters asks before
    /// each download (unsupported among the breaking filters).
    pub fn parse(filters: &[String], breaking: &[String]) -> Result<Self> {
        let parse = |values: &[String]| -> Result<Vec<Conjunction>> {
            let mut seen = std::collections::HashSet::new();
            values
                .iter()
                .filter(|value| seen.insert(value.as_str()))
                .map(|value| {
                    if value == "-" {
                        Err(invalid(
                            "interactive --break-match-filters - is unsupported",
                        ))
                    } else {
                        Conjunction::parse(value)
                    }
                })
                .collect()
        };
        let interactive = filters.iter().any(|value| value == "-");
        let filters: Vec<String> = filters
            .iter()
            .filter(|value| *value != "-")
            .cloned()
            .collect();
        Ok(Self {
            filters: parse(&filters)?,
            breaking: parse(breaking)?,
            selection: EntrySelection::default(),
            interactive,
        })
    }

    /// Apply the `_match_entry` checks that precede the filters.
    pub fn with_selection(mut self, selection: EntrySelection) -> Self {
        self.selection = selection;
        self
    }

    pub fn is_empty(&self) -> bool {
        self.filters.is_empty()
            && self.breaking.is_empty()
            && self.selection.is_empty()
            && !self.interactive
    }

    fn rejection(
        filters: &[Conjunction],
        dict: &serde_json::Map<String, Value>,
        incomplete: &dyn Fn(&str) -> bool,
    ) -> Result<Option<String>> {
        if filters.is_empty() {
            return Ok(None);
        }
        for filter in filters {
            if filter.matches(dict, incomplete)? {
                return Ok(None);
            }
        }
        let title = ["title", "id"]
            .iter()
            .find_map(|key| {
                dict.get(*key)
                    .and_then(Value::as_str)
                    .filter(|v| !v.is_empty())
            })
            .unwrap_or("entry");
        let joined = filters
            .iter()
            .map(|filter| py_strip(&filter.text))
            .collect::<Vec<_>>()
            .join(") | (");
        Ok(Some(format!(
            "{title} does not pass filter ({joined}), skipping .."
        )))
    }

    /// Evaluate a video. Before format selection, pass `before_selection` so
    /// format fields are incomplete (missing ones pass).
    pub fn check(&self, info: &VideoInfo, before_selection: bool) -> Result<MatchOutcome> {
        let mut value = serde_json::to_value(info).map_err(|error| invalid(error.to_string()))?;
        let dict = value
            .as_object_mut()
            .expect("video info serializes to an object");
        crate::info_order::strip_markers(dict);
        let dict = &*dict;
        let incomplete = |key: &str| before_selection && FORMAT_FIELDS.contains(&key);
        let reject = |reason| {
            if self.selection.break_on_reject {
                MatchOutcome::Break(reason)
            } else {
                MatchOutcome::Reject(reason)
            }
        };
        if let Some(reason) = self.selection.rejection(info)? {
            return Ok(reject(reason));
        }
        if let Some(reason) = Self::rejection(&self.breaking, dict, &incomplete)? {
            return Ok(MatchOutcome::Break(reason));
        }
        Ok(match Self::rejection(&self.filters, dict, &incomplete)? {
            Some(reason) => reject(reason),
            None if self.interactive && !before_selection => MatchOutcome::Ask,
            None => MatchOutcome::Pass,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dict(value: Value) -> serde_json::Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    fn matches(filter: &str, value: Value) -> Result<bool> {
        Conjunction::parse(filter)?.matches(&dict(value), &|_| false)
    }

    #[test]
    fn saved_upstream_match_str_cases() {
        let fixture: Value =
            serde_json::from_str(include_str!("../tests/fixtures/match_filters/cases.json"))
                .unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let filter = case["filter"].as_str().unwrap();
            let actual = matches(filter, case["dict"].clone());
            match case.get("result") {
                Some(expected) => {
                    assert_eq!(actual.ok(), expected.as_bool(), "{filter}");
                }
                None => assert!(actual.is_err(), "{filter}"),
            }
        }
        // Lookaround is unsupported by the native regex engine.
        assert!(Conjunction::parse("x~=(?<=a)b").is_err());
    }

    #[test]
    fn durations_and_filesizes_follow_upstream_parsers() {
        let int = |value| Some(Number::Int(value));
        for (text, expected) in [
            ("1:30", 90),
            ("1:02:03", 3723),
            ("1:00:00:00", 86400),
            ("90", 90),
            ("1h 2m", 3720),
            ("PT1M30S", 90),
            ("3 min", 180),
            ("2 hours", 7200),
            ("1:2", 62),
        ] {
            assert!(
                matches!(parse_duration(text), Some(Number::Int(v)) if v == expected),
                "{text}"
            );
            let _ = int(expected);
        }
        // The clock form falls back to seconds with a `:` fraction.
        assert!(
            matches!(parse_duration("1:123"), Some(Number::Float(v)) if (v - 1.123).abs() < 1e-9)
        );
        assert!(matches!(
            parse_duration("1.5 hours"),
            Some(Number::Int(5400))
        ));
        assert!(matches!(parse_duration("0.5 min"), Some(Number::Int(30))));
        assert!(matches!(parse_filesize("5 KiB"), Some(Number::Int(5120))));
        assert!(matches!(parse_filesize("5bytes"), Some(Number::Int(5))));
        assert!(matches!(
            parse_filesize("1,5MB"),
            Some(Number::Int(1_500_000))
        ));
        assert!(parse_filesize("5Bx").is_none());
    }
}

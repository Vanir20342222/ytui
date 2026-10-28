//! Upstream date helpers: `unified_timestamp` with the pinned `DATE_FORMATS`,
//! `extract_timezone`, `email.utils.parsedate_tz` and `strftime_or_none`.
//! Python `strptime` is reproduced for the directives these formats use in
//! the C locale. Non-ASCII decimal digits are not interpreted, and inputs for
//! which upstream raises (years outside 1..=9999) return `None`.

use crate::batch::is_py_space;
use regex::Regex;
use std::sync::LazyLock;

const DATE_FORMATS: &[&str] = &[
    "%d %B %Y",
    "%d %b %Y",
    "%B %d %Y",
    "%B %dst %Y",
    "%B %dnd %Y",
    "%B %drd %Y",
    "%B %dth %Y",
    "%b %d %Y",
    "%b %dst %Y",
    "%b %dnd %Y",
    "%b %drd %Y",
    "%b %dth %Y",
    "%b %dst %Y %I:%M",
    "%b %dnd %Y %I:%M",
    "%b %drd %Y %I:%M",
    "%b %dth %Y %I:%M",
    "%Y %m %d",
    "%Y-%m-%d",
    "%Y.%m.%d.",
    "%Y/%m/%d",
    "%Y/%m/%d %H:%M",
    "%Y/%m/%d %H:%M:%S",
    "%Y%m%d%H%M",
    "%Y%m%d%H%M%S",
    "%Y%m%d",
    "%Y-%m-%d %H:%M",
    "%Y-%m-%d %H:%M:%S",
    "%Y-%m-%d %H:%M:%S.%f",
    "%Y-%m-%d %H:%M:%S:%f",
    "%d.%m.%Y %H:%M",
    "%d.%m.%Y %H.%M",
    "%Y-%m-%dT%H:%M:%SZ",
    "%Y-%m-%dT%H:%M:%S.%fZ",
    "%Y-%m-%dT%H:%M:%S.%f0Z",
    "%Y-%m-%dT%H:%M:%S",
    "%Y-%m-%dT%H:%M:%S.%f",
    "%Y-%m-%dT%H:%M",
    "%b %d %Y at %H:%M",
    "%b %d %Y at %H:%M:%S",
    "%B %d %Y at %H:%M",
    "%B %d %Y at %H:%M:%S",
    "%H:%M %d-%b-%Y",
];

const DAY_FIRST: &[&str] = &[
    "%d-%m-%Y",
    "%d.%m.%Y",
    "%d.%m.%y",
    "%d/%m/%Y",
    "%d/%m/%y",
    "%d/%m/%Y %H:%M:%S",
    "%d-%m-%Y %H:%M",
    "%H:%M %d/%m/%Y",
];

const MONTH_FIRST: &[&str] = &[
    "%m-%d-%Y",
    "%m.%d.%Y",
    "%m/%d/%Y",
    "%m/%d/%y",
    "%m/%d/%Y %H:%M:%S",
];

const MONTHS: [&str; 12] = [
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

const TIMEZONE_NAMES: &[(&str, i64)] = &[
    ("UT", 0),
    ("UTC", 0),
    ("GMT", 0),
    ("Z", 0),
    ("AST", -4),
    ("ADT", -3),
    ("EST", -5),
    ("EDT", -4),
    ("CST", -6),
    ("CDT", -5),
    ("MST", -7),
    ("MDT", -6),
    ("PST", -8),
    ("PDT", -7),
];

/// A naive `datetime` without fractions (they never reach `timegm`).
#[derive(Debug, Clone, Copy, PartialEq)]
struct DateTime {
    year: i64,
    month: i64,
    day: i64,
    hour: i64,
    minute: i64,
    second: i64,
}

fn is_leap(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if is_leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days since 1970-01-01 for a proleptic Gregorian date.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_index = (month + 9) % 12;
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// `calendar.timegm`: `datetime.date(year, month, 1)` must be valid; the
/// remaining fields are plain arithmetic.
fn timegm(value: &DateTime) -> Option<i64> {
    if !(1..=9999).contains(&value.year) || !(1..=12).contains(&value.month) {
        return None;
    }
    let days = days_from_civil(value.year, value.month, 1).checked_add(value.day - 1)?;
    days.checked_mul(24)?
        .checked_add(value.hour)?
        .checked_mul(60)?
        .checked_add(value.minute)?
        .checked_mul(60)?
        .checked_add(value.second)
}

/// Naive `datetime + timedelta(seconds=…)`, which overflows outside 1..=9999.
fn shift(value: &DateTime, seconds: i64) -> Option<DateTime> {
    let total = timegm(value)?.checked_add(seconds)?;
    let (year, month, day) = civil_from_days(total.div_euclid(86_400));
    let rest = total.rem_euclid(86_400);
    (1..=9999).contains(&year).then_some(DateTime {
        year,
        month,
        day,
        hour: rest / 3600,
        minute: rest % 3600 / 60,
        second: rest % 60,
    })
}

/// Python `re` `\s+` for `str` patterns.
const SPACE: &str = r"[\s\x1c-\x1f]+";

/// The `_strptime` regex for a format, in the C locale.
fn strptime_regex(format: &str) -> Regex {
    let mut pattern = String::from("^(?i:");
    let mut chars = format.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            let directive = chars.next().expect("pinned formats are well formed");
            pattern.push_str(match directive {
                'd' => "(?P<d>3[0-1]|[1-2][0-9]|0[1-9]|[1-9]| [1-9])",
                'f' => "(?P<f>[0-9]{1,6})",
                'H' => "(?P<H>2[0-3]|[0-1][0-9]|[0-9]| [0-9])",
                'I' => "(?P<I>1[0-2]|0[1-9]|[1-9]| [1-9])",
                'm' => "(?P<m>1[0-2]|0[1-9]|[1-9])",
                'M' => "(?P<M>[0-5][0-9]|[0-9])",
                'S' => "(?P<S>6[0-1]|[0-5][0-9]|[0-9])",
                'y' => "(?P<y>[0-9][0-9])",
                'Y' => "(?P<Y>[0-9][0-9][0-9][0-9])",
                // Longest names first, stable among equal lengths.
                'B' => {
                    "(?P<B>september|february|november|december|january|october|august|march|april|june|july|may)"
                }
                'b' => "(?P<b>jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)",
                other => unreachable!("unsupported directive %{other}"),
            });
        } else if is_py_space(c) {
            while chars.peek().is_some_and(|next| is_py_space(*next)) {
                chars.next();
            }
            pattern.push_str(SPACE);
        } else if c == '\'' {
            pattern.push_str("['\u{2bc}]");
        } else {
            pattern.push_str(&regex::escape(&c.to_string()));
        }
    }
    pattern.push(')');
    Regex::new(&pattern).expect("strptime patterns are valid")
}

static DAY_FIRST_REGEXES: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    DATE_FORMATS
        .iter()
        .chain(DAY_FIRST)
        .map(|format| strptime_regex(format))
        .collect()
});

static MONTH_FIRST_REGEXES: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    DATE_FORMATS
        .iter()
        .chain(MONTH_FIRST)
        .map(|format| strptime_regex(format))
        .collect()
});

/// `datetime.strptime(data, format)` for a compiled pinned format.
fn strptime(regex: &Regex, data: &str) -> Option<DateTime> {
    strptime_checked(regex, data, "").ok()
}

/// `strptime` with Python's `ValueError` messages (datetime range errors use
/// the Python 3.13 wording; later versions word them differently).
fn strptime_checked(regex: &Regex, data: &str, format: &str) -> Result<DateTime, String> {
    let Some(found) = regex.captures(data) else {
        return Err(format!(
            "time data '{data}' does not match format '{format}'"
        ));
    };
    let end = found.get(0).map_or(0, |m| m.end());
    if end != data.len() {
        return Err(format!("unconverted data remains: {}", &data[end..]));
    }
    let invalid = || "invalid date".to_owned();
    let number = |name: &str| {
        found
            .name(name)
            .map(|m| m.as_str().trim_start().parse::<i64>())
    };
    let month_name = |name: &str, length: usize| {
        found.name(name).map(|m| {
            let lower = m.as_str().to_lowercase();
            MONTHS
                .iter()
                .position(|month| month[..length.min(month.len())] == lower)
                .map(|index| index as i64 + 1)
        })
    };
    let mut value = DateTime {
        year: 1900,
        month: 1,
        day: 1,
        hour: 0,
        minute: 0,
        second: 0,
    };
    if let Some(year) = number("y") {
        let year = year.map_err(|_| invalid())?;
        value.year = if year <= 68 { year + 2000 } else { year + 1900 };
    }
    if let Some(year) = number("Y") {
        value.year = year.map_err(|_| invalid())?;
    }
    if let Some(month) = number("m") {
        value.month = month.map_err(|_| invalid())?;
    }
    if let Some(month) = month_name("B", usize::MAX) {
        value.month = month.ok_or_else(invalid)?;
    }
    if let Some(month) = month_name("b", 3) {
        value.month = month.ok_or_else(invalid)?;
    }
    if let Some(day) = number("d") {
        value.day = day.map_err(|_| invalid())?;
    }
    if let Some(hour) = number("H") {
        value.hour = hour.map_err(|_| invalid())?;
    }
    // Without %p, 12 o'clock is midnight.
    if let Some(hour) = number("I") {
        let hour = hour.map_err(|_| invalid())?;
        value.hour = if hour == 12 { 0 } else { hour };
    }
    if let Some(minute) = number("M") {
        value.minute = minute.map_err(|_| invalid())?;
    }
    if let Some(second) = number("S") {
        value.second = second.map_err(|_| invalid())?;
    }
    // `datetime(...)` validation.
    if !(1..=9999).contains(&value.year) {
        return Err(format!("year {} is out of range", value.year));
    }
    if !(1..=days_in_month(value.year, value.month)).contains(&value.day) {
        return Err("day is out of range for month".into());
    }
    if !(0..60).contains(&value.second) {
        return Err("second must be in 0..59".into());
    }
    Ok(value)
}

/// Python `int(text)` for ASCII digits with an optional sign and underscores.
pub(crate) fn py_int(text: &str) -> Option<i64> {
    let text = text.trim_matches(is_py_space);
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
    let value: i64 = digits.replace('_', "").parse().ok()?;
    Some(if negative { -value } else { value })
}

/// `email.utils.parsedate_tz`: the fields and the zone offset in seconds
/// (zero when absent, as the public wrapper reports it).
fn parsedate_tz(data: &str) -> Option<(DateTime, i64)> {
    const MONTH_NAMES: [&str; 24] = [
        "jan",
        "feb",
        "mar",
        "apr",
        "may",
        "jun",
        "jul",
        "aug",
        "sep",
        "oct",
        "nov",
        "dec",
        "january",
        "february",
        "march",
        "april",
        "may",
        "june",
        "july",
        "august",
        "september",
        "october",
        "november",
        "december",
    ];
    const DAY_NAMES: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
    let mut data: Vec<String> = data
        .split(is_py_space)
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect();
    if data.is_empty() {
        return None;
    }
    if data[0].ends_with(',') || DAY_NAMES.contains(&data[0].to_lowercase().as_str()) {
        data.remove(0);
    } else if let Some(index) = data[0].rfind(',') {
        data[0] = data[0][index + 1..].to_owned();
    }
    if data.len() == 3 {
        let stuff: Vec<String> = data[0].split('-').map(str::to_owned).collect();
        if stuff.len() == 3 {
            data = stuff.into_iter().chain(data[1..].iter().cloned()).collect();
        }
    }
    if data.len() == 4 {
        let s = data[3].clone();
        let index = s.find('+').or_else(|| s.find('-'));
        match index {
            Some(index) if index > 0 => {
                data.truncate(3);
                data.push(s[..index].to_owned());
                data.push(s[index..].to_owned());
            }
            _ => data.push(String::new()),
        }
    }
    if data.len() < 5 {
        return None;
    }
    let (mut dd, mut mm, mut yy, mut tm) = (
        data[0].clone(),
        data[1].clone(),
        data[2].clone(),
        data[3].clone(),
    );
    let mut tz = data[4].clone();
    if dd.is_empty() || mm.is_empty() || yy.is_empty() {
        return None;
    }
    mm = mm.to_lowercase();
    if !MONTH_NAMES.contains(&mm.as_str()) {
        (dd, mm) = (mm, dd.to_lowercase());
        if !MONTH_NAMES.contains(&mm.as_str()) {
            return None;
        }
    }
    let mut month = MONTH_NAMES.iter().position(|name| *name == mm)? as i64 + 1;
    if month > 12 {
        month -= 12;
    }
    if dd.ends_with(',') {
        dd.pop();
    }
    if yy.find(':').is_some_and(|index| index > 0) {
        std::mem::swap(&mut yy, &mut tm);
    }
    if yy.ends_with(',') {
        yy.pop();
        if yy.is_empty() {
            return None;
        }
    }
    if !yy.chars().next().is_some_and(char::is_numeric) {
        std::mem::swap(&mut yy, &mut tz);
    }
    if tm.ends_with(',') {
        tm.pop();
    }
    let parts: Vec<&str> = tm.split(':').collect();
    let (hour, minute, second) = match parts.as_slice() {
        [hour, minute] => (*hour, *minute, "0"),
        [hour, minute, second] => (*hour, *minute, *second),
        [single] if single.contains('.') => match single.split('.').collect::<Vec<_>>()[..] {
            [hour, minute] => (hour, minute, "0"),
            [hour, minute, second] => (hour, minute, second),
            _ => return None,
        },
        _ => return None,
    };
    let mut year = py_int(&yy)?;
    let day = py_int(&dd)?;
    let (hour, minute, second) = (py_int(hour)?, py_int(minute)?, py_int(second)?);
    if year < 100 {
        year += if year > 68 { 1900 } else { 2000 };
    }
    const ZONES: &[(&str, i64)] = &[
        ("UT", 0),
        ("UTC", 0),
        ("GMT", 0),
        ("Z", 0),
        ("AST", -400),
        ("ADT", -300),
        ("EST", -500),
        ("EDT", -400),
        ("CST", -600),
        ("CDT", -500),
        ("MST", -700),
        ("MDT", -600),
        ("PST", -800),
        ("PDT", -700),
    ];
    let tz = tz.to_uppercase();
    let offset = ZONES
        .iter()
        .find(|(name, _)| *name == tz)
        .map(|(_, offset)| *offset)
        .or_else(|| py_int(&tz))
        .unwrap_or(0);
    let sign = offset.signum();
    let offset = offset.abs();
    let offset = sign * ((offset / 100) * 3600 + (offset % 100) * 60);
    Some((
        DateTime {
            year,
            month,
            day,
            hour,
            minute,
            second,
        },
        offset,
    ))
}

/// Upstream `timeconvert`: `mktime_tz(parsedate_tz(text))`.
pub fn timeconvert(text: &str) -> Option<i64> {
    let (parsed, offset) = parsedate_tz(text)?;
    timegm(&parsed)?.checked_sub(offset)
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn is_decimal(c: char) -> bool {
    static DIGIT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d$").unwrap());
    let mut buffer = [0; 4];
    DIGIT.is_match(c.encode_utf8(&mut buffer))
}

/// `extract_timezone(date_str, default=None)`: the offset in seconds (zero
/// when absent) and the remaining text.
fn extract_timezone(date: &str) -> (i64, String) {
    let chars: Vec<char> = date.chars().collect();
    let boundary = |index: usize| {
        let before = index.checked_sub(1).and_then(|i| chars.get(i)).copied();
        before.is_some_and(is_word) != chars.get(index).copied().is_some_and(is_word)
    };
    let all = |range: std::ops::Range<usize>, test: fn(char) -> bool| {
        chars[range].iter().copied().all(test)
    };
    // `(?<=.\b\d{4}|\b\d{2}:\d\d)` at `p`.
    let preceded = |p: usize| {
        p >= 5
            && ((chars[p - 5] != '\n' && boundary(p - 4) && all(p - 4..p, is_decimal))
                || (boundary(p - 5)
                    && all(p - 5..p - 3, is_decimal)
                    && chars[p - 3] == ':'
                    && all(p - 2..p, is_decimal)))
    };
    // `(?<!.\b[a-zA-Z]{3}|[a-zA-Z]{4}|..\b\d\d)` at `p`.
    let excluded = |p: usize| {
        (p >= 4
            && chars[p - 4] != '\n'
            && boundary(p - 3)
            && all(p - 3..p, |c| c.is_ascii_alphabetic()))
            || (p >= 4 && all(p - 4..p, |c| c.is_ascii_alphabetic()))
            || (p >= 4
                && chars[p - 4] != '\n'
                && chars[p - 3] != '\n'
                && boundary(p - 2)
                && all(p - 2..p, is_decimal))
    };
    let offset = |p: usize| -> Option<i64> {
        let mut index = p;
        if chars.get(index) == Some(&' ') {
            index += 1;
        }
        let sign = match chars.get(index)? {
            '+' => 1,
            '-' => -1,
            _ => return None,
        };
        index += 1;
        let two = |index: usize| -> Option<i64> {
            let digits = chars.get(index..index + 2)?;
            digits
                .iter()
                .all(char::is_ascii_digit)
                .then(|| digits.iter().collect::<String>().parse().ok())?
        };
        let hours = two(index)?;
        index += 2;
        if chars.get(index) == Some(&':') {
            index += 1;
        }
        let minutes = two(index)?;
        (index + 2 == chars.len()).then_some(sign * (hours * 3600 + minutes * 60))
    };
    for p in 8..chars.len() {
        if chars[p..] == ['Z'] {
            return (0, chars[..p].iter().collect());
        }
        if (preceded(p) || !excluded(p))
            && let Some(seconds) = offset(p)
        {
            return (seconds, chars[..p].iter().collect());
        }
    }
    static NAMED: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\d{1,2}:\d{1,2}(?:\.\d+)?(?P<tz>[\s\x1c-\x1f]*[A-Z]+)$").unwrap()
    });
    if let Some(tz) = NAMED.captures(date).and_then(|found| found.name("tz"))
        && let Some((_, hours)) = TIMEZONE_NAMES
            .iter()
            .find(|(name, _)| *name == tz.as_str().trim_matches(is_py_space))
    {
        return (hours * 3600, date[..tz.start()].to_owned());
    }
    (0, date.to_owned())
}

/// Upstream `unified_timestamp(date_str, day_first)` (no `tz_offset`).
pub fn unified_timestamp(date: &str, day_first: bool) -> Option<i64> {
    static NAMES: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)[,|]|(mon|tues?|wed(nes)?|thu(rs)?|fri|sat(ur)?|sun)(day)?").unwrap()
    });
    static MERIDIEM: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)[\s\x1c-\x1f]*(?:AM|PM)(?:[\s\x1c-\x1f]+[A-Z]+)?").unwrap()
    });
    static PM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)PM").unwrap());
    static UNKNOWN_ZONE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\d{1,2}:\d{1,2}(?:\.\d+)?(?P<tz>[\s\x1c-\x1f]*[A-Z]+)$").unwrap()
    });
    static NANOSECONDS: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"^([0-9]{4,}-[0-9]{1,2}-[0-9]{1,2}T[0-9]{1,2}:[0-9]{1,2}:[0-9]{1,2}\.[0-9]{6})[0-9]+$",
        )
        .unwrap()
    });
    let stripped = NAMES.replace_all(date, "");
    let mut collapsed = String::with_capacity(stripped.len());
    let mut in_space = false;
    for c in stripped.chars() {
        if is_py_space(c) {
            if !in_space {
                collapsed.push(' ');
            }
            in_space = true;
        } else {
            collapsed.push(c);
            in_space = false;
        }
    }
    let pm_delta = if PM.is_match(&collapsed) { 12 } else { 0 };
    let (timezone, rest) = extract_timezone(&collapsed);
    let mut rest = MERIDIEM.replace_all(&rest, "").into_owned();
    if let Some(tz) = UNKNOWN_ZONE
        .captures(&rest)
        .and_then(|found| found.name("tz"))
    {
        rest.truncate(tz.start());
    }
    if let Some(found) = NANOSECONDS.captures(&rest) {
        rest = found[1].to_owned();
    }
    let regexes = if day_first {
        &*DAY_FIRST_REGEXES
    } else {
        &*MONTH_FIRST_REGEXES
    };
    for regex in regexes {
        if let Some(parsed) = strptime(regex, &rest) {
            // A shifted datetime outside 1..=9999 overflows upstream.
            let shifted = shift(&shift(&parsed, -timezone)?, pm_delta * 3600)?;
            return timegm(&shifted);
        }
    }
    let (parsed, _) = parsedate_tz(&rest)?;
    timegm(&parsed)?
        .checked_add(pm_delta * 3600)?
        .checked_sub(timezone)
}

/// Upstream `unified_strdate(date_str, day_first)`: `YYYYMMDD` of the last
/// matching `date_formats` entry (no timezone shift), else `parsedate_tz`.
pub fn unified_strdate(date: &str, day_first: bool) -> Option<String> {
    static MERIDIEM: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)[\s\x1c-\x1f]*(?:AM|PM)(?:[\s\x1c-\x1f]+[A-Z]+)?").unwrap()
    });
    let date = date.replace(',', " ");
    let date = MERIDIEM.replace_all(&date, "");
    let (_, rest) = extract_timezone(&date);
    let regexes = if day_first {
        &*DAY_FIRST_REGEXES
    } else {
        &*MONTH_FIRST_REGEXES
    };
    let valid = |value: &DateTime| {
        timegm(value).map(|_| format!("{:04}{:02}{:02}", value.year, value.month, value.day))
    };
    let mut found = None;
    for regex in regexes {
        if let Some(parsed) = strptime(regex, &rest) {
            found = valid(&parsed).or(found);
        }
    }
    found.or_else(|| valid(&parsedate_tz(&rest)?.0))
}

/// Upstream `parse_iso8601(date)`: fractions removed, `extract_timezone`,
/// then exactly `%Y-%m-%dT%H:%M:%S`.
pub fn parse_iso8601(date: &str) -> Option<i64> {
    static FORMAT: LazyLock<Regex> = LazyLock::new(|| strptime_regex("%Y-%m-%dT%H:%M:%S"));
    parse_iso8601_with(date, &FORMAT)
}

/// Upstream `parse_iso8601(date, delimiter=' ')`.
pub fn parse_iso8601_space(date: &str) -> Option<i64> {
    static FORMAT: LazyLock<Regex> = LazyLock::new(|| strptime_regex("%Y-%m-%d %H:%M:%S"));
    parse_iso8601_with(date, &FORMAT)
}

fn parse_iso8601_with(date: &str, format: &Regex) -> Option<i64> {
    static FRACTION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\.[0-9]+").unwrap());
    let stripped = FRACTION.replace_all(date, "");
    let (timezone, rest) = extract_timezone(&stripped);
    let parsed = strptime(format, &rest)?;
    timegm(&shift(&parsed, -timezone)?)
}

const MIN_DAY: i64 = -719_162; // 0001-01-01
const MAX_DAY: i64 = 2_932_896; // 9999-12-31

fn iso_date(days: i64) -> String {
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}")
}

/// `datetime_from_str(text, precision='microsecond', format='%Y%m%d')` in
/// UNIX seconds (sub-second precision is not represented), with `now` in
/// UNIX seconds.
fn datetime_from_str(text: &str, now: i64) -> Result<i64, String> {
    static RELATIVE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^(?P<start>.+)(?P<sign>[+-])(?P<time>[0-9]+)(?P<unit>microsecond|second|minute|hour|day|week|month|year)s?")
            .unwrap()
    });
    static YMD: LazyLock<Regex> = LazyLock::new(|| strptime_regex("%Y%m%d"));
    let overflow = || "date value out of range".to_owned();
    match text {
        "now" | "today" => return Ok(now),
        "yesterday" => return Ok(now - 86_400),
        _ => {}
    }
    if let Some(found) = RELATIVE.captures(text) {
        let start = datetime_from_str(&found["start"], now)?;
        let time: i64 = found["time"].parse().map_err(|_| overflow())?;
        let time = if &found["sign"] == "-" { -time } else { time };
        let unit_seconds = match &found["unit"] {
            "month" | "year" => None,
            "week" => Some(7 * 86_400),
            "day" => Some(86_400),
            "hour" => Some(3600),
            "minute" => Some(60),
            "second" => Some(1),
            _ => Some(0),
        };
        let shifted = match unit_seconds {
            Some(0) => start.checked_add(time.div_euclid(1_000_000)),
            Some(unit) => time
                .checked_mul(unit)
                .and_then(|delta| start.checked_add(delta)),
            None => {
                let months = if &found["unit"] == "year" {
                    time.checked_mul(12).ok_or_else(overflow)?
                } else {
                    time
                };
                // `datetime_add_months`: clamp the day to the target month.
                let (year, month, day) = civil_from_days(start.div_euclid(86_400));
                let month = (month - 1).checked_add(months).ok_or_else(overflow)?;
                let year = year + month.div_euclid(12);
                let month = month.rem_euclid(12) + 1;
                if !(1..=9999).contains(&year) {
                    return Err(format!("year {year} is out of range"));
                }
                let days = days_from_civil(year, month, day.min(days_in_month(year, month)));
                Some(days * 86_400 + start.rem_euclid(86_400))
            }
        }
        .ok_or_else(overflow)?;
        return if (MIN_DAY * 86_400..(MAX_DAY + 1) * 86_400).contains(&shifted) {
            Ok(shifted)
        } else {
            Err(overflow())
        };
    }
    let parsed = strptime_checked(&YMD, text, "%Y%m%d")?;
    Ok(days_from_civil(parsed.year, parsed.month, parsed.day) * 86_400)
}

/// `date_from_str(text)` as days since the epoch.
fn date_from_str(text: &str, now: i64) -> Result<i64, String> {
    Ok(datetime_from_str(text, now)?.div_euclid(86_400))
}

/// Upstream `DateRange` over UTC dates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateRange {
    start: i64,
    end: i64,
}

impl Default for DateRange {
    fn default() -> Self {
        Self {
            start: MIN_DAY,
            end: MAX_DAY,
        }
    }
}

impl std::fmt::Display for DateRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} to {}", iso_date(self.start), iso_date(self.end))
    }
}

impl DateRange {
    /// `date_from_str(value, strict=True)`.
    fn strict(value: &str, now: i64) -> Result<i64, String> {
        static STRICT: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"^(?:[0-9]{8}|(now|today|yesterday)(-[0-9]+(day|week|month|year)s?)?)$")
                .unwrap()
        });
        if !STRICT.is_match(value) {
            return Err(format!("Invalid date format \"{value}\""));
        }
        date_from_str(value, now)
    }

    /// `DateRange(start, end)`.
    pub fn new(start: Option<&str>, end: Option<&str>, now: i64) -> Result<Self, String> {
        let range = Self {
            start: start
                .map(|v| Self::strict(v, now))
                .transpose()?
                .unwrap_or(MIN_DAY),
            end: end
                .map(|v| Self::strict(v, now))
                .transpose()?
                .unwrap_or(MAX_DAY),
        };
        if range.start > range.end {
            return Err(format!(
                "Date range: \"{range}\" , the start date must be before the end date"
            ));
        }
        Ok(range)
    }

    /// `DateRange.day(day)`.
    pub fn day(day: &str, now: i64) -> Result<Self, String> {
        Self::new(Some(day), Some(day), now)
    }

    /// `date in range` for an `upload_date`, returning its ISO date when
    /// outside the range.
    pub fn reject(&self, upload_date: &str, now: i64) -> Result<Option<String>, String> {
        let day = date_from_str(upload_date, now)?;
        Ok((!(self.start..=self.end).contains(&day)).then(|| iso_date(day)))
    }
}

/// Upstream `strftime_or_none(timestamp)` with the default `%Y%m%d`.
pub fn strftime_date(timestamp: i64) -> Option<String> {
    let (year, month, day) = civil_from_days(timestamp.div_euclid(86_400));
    (1..=9999)
        .contains(&year)
        .then(|| format!("{year:04}{month:02}{day:02}"))
}

/// A `strftime_or_none` input: a Unix timestamp (`int`/`float`) or a
/// `YYYYMMDD` string.
pub enum StrftimeInput<'a> {
    Integer(i64),
    Float(f64),
    Text(&'a str),
}

static YYYYMMDD: LazyLock<Regex> = LazyLock::new(|| strptime_regex("%Y%m%d"));

const WEEKDAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

/// Upstream `strftime_or_none(value, format)`: timestamps become aware UTC
/// datetimes (`%Z` is `UTC`), `YYYYMMDD` strings naive ones. `Ok(None)` is
/// upstream's caught `ValueError`/`OverflowError`; directives the C-locale
/// port does not know, and `%s` of a naive (local-time) date, are errors.
pub fn strftime_or_none(value: StrftimeInput<'_>, format: &str) -> Result<Option<String>, String> {
    // Days since the epoch, microseconds of the day, and whether aware.
    let (days, micros, aware) = match value {
        StrftimeInput::Integer(seconds) => (
            seconds.div_euclid(86_400),
            seconds.rem_euclid(86_400) as i128 * 1_000_000,
            true,
        ),
        StrftimeInput::Float(seconds) => {
            // `timedelta(seconds=…)` rounds to microseconds, half to even.
            let total = (seconds * 1e6).round_ties_even();
            if !total.is_finite() || total.abs() > 1e18 {
                return Ok(None);
            }
            let total = total as i128;
            (
                total.div_euclid(86_400_000_000) as i64,
                total.rem_euclid(86_400_000_000),
                true,
            )
        }
        StrftimeInput::Text(text) => match strptime_checked(&YYYYMMDD, text, "%Y%m%d") {
            Ok(date) => (days_from_civil(date.year, date.month, date.day), 0, false),
            Err(_) => return Ok(None),
        },
    };
    let (year, month, day) = civil_from_days(days);
    if !(1..=9999).contains(&year) {
        return Ok(None);
    }
    let seconds = (micros / 1_000_000) as i64;
    let (hour, minute, second) = (seconds / 3600, seconds % 3600 / 60, seconds % 60);
    let microsecond = micros % 1_000_000;
    // Monday is 0, like `datetime.weekday()`.
    let weekday = (days + 3).rem_euclid(7);
    let year_day = days - days_from_civil(year, 1, 1) + 1;
    // ISO 8601 year and week: the week containing the year's first Thursday.
    let iso = |days: i64| {
        let thursday = days - (days + 3).rem_euclid(7) + 3;
        let (iso_year, _, _) = civil_from_days(thursday);
        (
            iso_year,
            (thursday - days_from_civil(iso_year, 1, 1)) / 7 + 1,
        )
    };
    let (iso_year, iso_week) = iso(days);
    let hour12 = if hour % 12 == 0 { 12 } else { hour % 12 };
    let mut output = String::new();
    let mut chars = format.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            output.push(c);
            continue;
        }
        let Some(mut directive) = chars.next() else {
            output.push('%');
            break;
        };
        let colon = directive == ':';
        if colon {
            directive = chars.next().unwrap_or(':');
            if directive != 'z' {
                return Err(format!("unsupported strftime directive %:{directive}"));
            }
        }
        let text = match directive {
            'a' => WEEKDAYS[weekday as usize][..3].to_owned(),
            'A' => WEEKDAYS[weekday as usize].to_owned(),
            'b' | 'h' => capitalized(&MONTHS[month as usize - 1][..3]),
            'B' => capitalized(MONTHS[month as usize - 1]),
            'c' => format!(
                "{} {} {day:2} {hour:02}:{minute:02}:{second:02} {year:04}",
                &WEEKDAYS[weekday as usize][..3],
                capitalized(&MONTHS[month as usize - 1][..3]),
            ),
            'C' => format!("{:02}", year / 100),
            'd' => format!("{day:02}"),
            'D' | 'x' => format!("{month:02}/{day:02}/{:02}", year % 100),
            'e' => format!("{day:2}"),
            'f' => format!("{microsecond:06}"),
            'F' => format!("{year:04}-{month:02}-{day:02}"),
            'g' => format!("{:02}", iso_year % 100),
            'G' => format!("{iso_year:04}"),
            'H' => format!("{hour:02}"),
            'I' => format!("{hour12:02}"),
            'j' => format!("{year_day:03}"),
            'k' => format!("{hour:2}"),
            'l' => format!("{hour12:2}"),
            'm' => format!("{month:02}"),
            'M' => format!("{minute:02}"),
            'n' => "\n".into(),
            'p' => if hour < 12 { "AM" } else { "PM" }.into(),
            'r' => format!(
                "{hour12:02}:{minute:02}:{second:02} {}",
                if hour < 12 { "AM" } else { "PM" }
            ),
            'R' => format!("{hour:02}:{minute:02}"),
            's' if aware => (days * 86_400 + seconds).to_string(),
            's' => return Err("%s of a naive date needs the local time zone".into()),
            'S' => format!("{second:02}"),
            't' => "\t".into(),
            'T' | 'X' => format!("{hour:02}:{minute:02}:{second:02}"),
            'u' => (weekday + 1).to_string(),
            'U' => format!("{:02}", (year_day + 6 - (weekday + 1) % 7) / 7),
            'V' => format!("{iso_week:02}"),
            'w' => ((weekday + 1) % 7).to_string(),
            'W' => format!("{:02}", (year_day + 6 - weekday) / 7),
            'y' => format!("{:02}", year % 100),
            'Y' => format!("{year:04}"),
            'z' if aware && colon => "+00:00".into(),
            'z' if aware => "+0000".into(),
            'Z' if aware => "UTC".into(),
            'z' | 'Z' => String::new(),
            '%' => "%".into(),
            other => return Err(format!("unsupported strftime directive %{other}")),
        };
        output.push_str(&text);
    }
    Ok(Some(output))
}

fn capitalized(name: &str) -> String {
    name[..1].to_uppercase() + &name[1..]
}

/// `datetime.now().strftime('%Y-%m-%d %H:%M')` in local time (UTC where
/// the C library's `localtime_r` is unavailable).
pub fn local_now_minutes() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64);
    let offset = local_offset(now).unwrap_or(0);
    let local = now + offset;
    let (year, month, day) = civil_from_days(local.div_euclid(86_400));
    let seconds = local.rem_euclid(86_400);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}",
        seconds / 3600,
        seconds % 3600 / 60
    )
}

/// The local UTC offset in seconds at `time` (`tm_gmtoff`).
#[cfg(unix)]
fn local_offset(time: i64) -> Option<i64> {
    #[repr(C)]
    struct Tm {
        tm_sec: std::ffi::c_int,
        tm_min: std::ffi::c_int,
        tm_hour: std::ffi::c_int,
        tm_mday: std::ffi::c_int,
        tm_mon: std::ffi::c_int,
        tm_year: std::ffi::c_int,
        tm_wday: std::ffi::c_int,
        tm_yday: std::ffi::c_int,
        tm_isdst: std::ffi::c_int,
        tm_gmtoff: std::ffi::c_long,
        tm_zone: *const std::ffi::c_char,
    }
    unsafe extern "C" {
        fn localtime_r(time: *const i64, result: *mut Tm) -> *mut Tm;
    }
    let mut tm = std::mem::MaybeUninit::<Tm>::zeroed();
    // SAFETY: `localtime_r` writes the `struct tm` it is given and returns
    // null on failure; both pointers are valid for the call.
    let result = unsafe { localtime_r(&time, tm.as_mut_ptr()) };
    if result.is_null() {
        return None;
    }
    // SAFETY: initialized by the successful call above.
    let offset = unsafe { tm.assume_init() }.tm_gmtoff;
    // `c_long` is `i32` on some targets.
    #[allow(clippy::unnecessary_cast)]
    let offset = offset as i64;
    Some(offset)
}

#[cfg(not(unix))]
fn local_offset(_: i64) -> Option<i64> {
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn unified_strdate_matches_pinned_upstream() {
        // Values from the pinned upstream `unified_strdate`.
        for (input, expected) in [
            ("2021-05-04", Some("20210504")),
            ("May 4, 2021 10:00 PM EST", Some("20210504")),
            ("04.05.2021", Some("20210504")),
            ("Tue, 04 May 2021 10:00:00 +0200", Some("20210504")),
            ("2021/13/40", None),
            ("x", None),
        ] {
            assert_eq!(
                super::unified_strdate(input, true).as_deref(),
                expected,
                "{input}"
            );
        }
    }

    use super::*;

    #[test]
    fn saved_upstream_unified_timestamp_and_strftime_cases() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/dates/cases.json")).unwrap();
        for case in fixture["unified_timestamp"].as_array().unwrap() {
            let input = case["input"].as_str().unwrap();
            for (key, day_first) in [("day_first", true), ("month_first", false)] {
                assert_eq!(
                    unified_timestamp(input, day_first),
                    case[key].as_i64(),
                    "{input:?} {key}"
                );
            }
        }
        for case in fixture["timeconvert"].as_array().unwrap() {
            let input = case["input"].as_str().unwrap();
            assert_eq!(timeconvert(input), case["timestamp"].as_i64(), "{input:?}");
        }
        let ranges = &fixture["ranges"];
        let now = ranges["now"].as_i64().unwrap();
        for case in ranges["cases"].as_array().unwrap() {
            let text = |key: &str| case[key].as_str();
            let result = match text("date").filter(|date| !date.is_empty()) {
                Some(date) => DateRange::day(date, now),
                None => DateRange::new(text("dateafter"), text("datebefore"), now),
            };
            match (result, case.get("error")) {
                (Ok(range), None) => {
                    assert_eq!(range.to_string(), case["range"].as_str().unwrap(), "{case}");
                    for upload in ranges["uploads"].as_array().unwrap() {
                        let upload = upload.as_str().unwrap();
                        assert_eq!(
                            range.reject(upload, now).unwrap().as_deref(),
                            case["rejected"][upload].as_str(),
                            "{case} {upload}"
                        );
                    }
                }
                (Err(error), Some(expected)) => {
                    let expected = expected.as_str().unwrap();
                    assert_eq!(
                        Some(error.as_str()),
                        expected.strip_prefix("ValueError: "),
                        "{case}"
                    );
                }
                (result, _) => panic!("{case}: {result:?}"),
            }
        }
        for case in fixture["strftime"].as_array().unwrap() {
            let timestamp = case["timestamp"].as_i64().unwrap();
            assert_eq!(
                strftime_date(timestamp).as_deref(),
                case["date"].as_str(),
                "{timestamp}"
            );
        }
    }
}

#[cfg(test)]
mod range_tests {
    use super::*;

    #[test]
    fn out_of_range_dates_use_python_3_13_wording() {
        let now = 1_709_164_800;
        assert_eq!(
            DateRange::day("00000101", now).unwrap_err(),
            "year 0 is out of range"
        );
        assert_eq!(
            DateRange::day("20230229", now).unwrap_err(),
            "day is out of range for month"
        );
        assert_eq!(
            DateRange::day("today-9999years", now).unwrap_err(),
            "year -7975 is out of range"
        );
        assert_eq!(
            DateRange::day("today-99999999999999days", now).unwrap_err(),
            "date value out of range"
        );
    }
}

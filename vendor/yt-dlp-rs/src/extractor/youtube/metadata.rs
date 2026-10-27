//! Text and upload-date helpers shared by the YouTube info dict
//! ([`super::dict`]).

use serde_json::Value;

/// Upstream `_get_text`: `simpleText`, else the joined `runs[].text`.
pub(super) fn text(value: &Value) -> Option<String> {
    if let Some(text) = value.get("simpleText").and_then(Value::as_str)
        && !text.is_empty()
    {
        return Some(text.to_owned());
    }
    let runs = value
        .get("runs")
        .and_then(Value::as_array)
        .or_else(|| value.as_array())?;
    let joined: String = runs
        .iter()
        .filter_map(|run| run.get("text").and_then(Value::as_str))
        .collect();
    (!joined.is_empty()).then_some(joined)
}

/// Days since 1970-01-01 for a proleptic Gregorian date.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}

fn valid_date(year: i64, month: i64, day: i64) -> bool {
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    (1..=12).contains(&month) && day >= 1 && day <= days[(month - 1) as usize]
}

/// The microformat `uploadDate` subset: `YYYY-MM-DD[THH:MM:SS[.f+][Z|±HH[:]MM]]`.
/// A timestamp requires both time and zone (`parse_iso8601(timezone=NO_DEFAULT)`);
/// otherwise the date part is the upload date (`unified_strdate`).
pub(super) fn upload_time(value: &str) -> (Option<String>, Option<i64>) {
    let bytes = value.as_bytes();
    let digits = |range: std::ops::Range<usize>| -> Option<i64> {
        let part = value.get(range)?;
        part.bytes()
            .all(|b| b.is_ascii_digit())
            .then(|| part.parse().ok())?
    };
    let (Some(year), Some(month), Some(day)) = (digits(0..4), digits(5..7), digits(8..10)) else {
        return (None, None);
    };
    if bytes.get(4) != Some(&b'-') || bytes.get(7) != Some(&b'-') || !valid_date(year, month, day) {
        return (None, None);
    }
    let date = format!("{year:04}{month:02}{day:02}");
    if bytes.len() == 10 {
        return (Some(date), None);
    }
    let time = (bytes.get(10) == Some(&b'T'))
        .then(|| Some((digits(11..13)?, digits(14..16)?, digits(17..19)?)))
        .flatten()
        .filter(|_| bytes.get(13) == Some(&b':') && bytes.get(16) == Some(&b':'));
    let Some((hour, minute, second)) = time.filter(|(h, m, s)| *h < 24 && *m < 60 && *s < 60)
    else {
        return (None, None);
    };
    let mut rest = &value[19..];
    if let Some(fraction) = rest.strip_prefix('.') {
        let length = fraction.bytes().take_while(u8::is_ascii_digit).count();
        if length == 0 {
            return (None, None);
        }
        rest = &fraction[length..];
    }
    let offset = match rest {
        "" => None,
        "Z" => Some(0),
        zone => {
            let sign = match zone.as_bytes()[0] {
                b'+' => 1,
                b'-' => -1,
                _ => return (None, None),
            };
            let zone = &zone[1..];
            let (hours, minutes) = match zone.len() {
                5 if zone.as_bytes()[2] == b':' => (&zone[..2], &zone[3..]),
                4 => (&zone[..2], &zone[2..]),
                _ => return (None, None),
            };
            let (Ok(hours), Ok(minutes)) = (hours.parse::<i64>(), minutes.parse::<i64>()) else {
                return (None, None);
            };
            Some(sign * (hours * 3600 + minutes * 60))
        }
    };
    let Some(offset) = offset else {
        return (Some(date), None);
    };
    let timestamp =
        days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second - offset;
    let (year, month, day) = civil_from_days(timestamp.div_euclid(86_400));
    (
        Some(format!("{year:04}{month:02}{day:02}")),
        Some(timestamp),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_dates_follow_timezone_and_precision_rules() {
        assert_eq!(
            upload_time("2008-05-29T00:00:00-07:00"),
            (Some("20080529".into()), Some(1_212_044_400))
        );
        assert_eq!(
            upload_time("2008-05-29T23:30:00.5-0100"),
            (Some("20080530".into()), Some(1_212_107_400))
        );
        assert_eq!(upload_time("2008-05-29"), (Some("20080529".into()), None));
        assert_eq!(
            upload_time("2008-05-29T10:00:00"),
            (Some("20080529".into()), None)
        );
        assert_eq!(upload_time("2008-02-30"), (None, None));
    }
}

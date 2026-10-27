use crate::{Error, Result};
use reqwest::{
    blocking::Response,
    header::{HeaderMap, RETRY_AFTER},
};
use std::time::{Duration, SystemTime};

const MAX_RETRY_AFTER: Duration = Duration::from_secs(60);

// Never combine multiple server instructions. Take the clock as an argument so
// date boundaries can be tested without sleeps or dependence on the real date.
fn retry_after(headers: &HeaderMap, now: SystemTime) -> Option<Duration> {
    let mut values = headers.get_all(RETRY_AFTER).iter();
    let value = values.next()?.to_str().ok()?.trim_matches([' ', '\t']);
    if values.next().is_some() || value.is_empty() {
        return None;
    }
    if !value.bytes().all(|b| b.is_ascii_digit()) {
        let date = httpdate::parse_http_date(value).ok()?;
        // Accept the preferred IMF-fixdate form only, excluding obsolete date
        // formats (in particular their ambiguous two-digit years).
        if httpdate::fmt_http_date(date) != value {
            return None;
        }
        return Some(
            date.duration_since(now)
                .unwrap_or_default()
                .min(MAX_RETRY_AFTER),
        );
    }
    // Saturate while parsing so even an overflowing decimal remains bounded.
    let seconds = value.bytes().fold(0_u64, |seconds, digit| {
        seconds
            .saturating_mul(10)
            .saturating_add(u64::from(digit - b'0'))
    });
    Some(Duration::from_secs(seconds).min(MAX_RETRY_AFTER))
}

pub(crate) fn check_response(response: Response) -> Result<Response> {
    if let Err(source) = response.error_for_status_ref() {
        if transient_request(&source)
            && let Some(retry_after) = retry_after(response.headers(), SystemTime::now())
        {
            return Err(Error::HttpRetryAfter {
                source,
                retry_after,
            });
        }
        return Err(Error::Http(source));
    }
    Ok(response)
}

pub(crate) fn failure_delay(error: &Error) -> Option<Duration> {
    match error {
        Error::Webpage { source, .. } => failure_delay(source),
        Error::Http(error) if transient_request(error) => Some(Duration::ZERO),
        Error::HttpRetryAfter {
            source,
            retry_after,
        } if transient_request(source) => Some(*retry_after),
        Error::ExtractionRead(_) => Some(Duration::ZERO),
        _ => None,
    }
}

/// Upstream `HttpFD.establish_connection`: 5xx statuses and transport
/// errors retry; other statuses fail at once.
pub(crate) fn download_failure_delay(error: &Error) -> Option<Duration> {
    let server_error = |error: &reqwest::Error| {
        error
            .status()
            .is_some_and(|status| status.is_server_error())
    };
    match error {
        Error::Http(error) if error.status().is_some() => {
            server_error(error).then_some(Duration::ZERO)
        }
        Error::Http(error) => {
            (!error.is_builder() && !error.is_redirect()).then_some(Duration::ZERO)
        }
        Error::HttpRetryAfter {
            source,
            retry_after,
        } => server_error(source).then_some(*retry_after),
        Error::ExtractionRead(_) => Some(Duration::ZERO),
        _ => None,
    }
}

pub(crate) fn delay(attempt: u32, retry_after: Duration) -> Duration {
    let delay_ms = (100_u64 << attempt.min(4)).min(1000);
    Duration::from_millis(delay_ms).max(retry_after.min(MAX_RETRY_AFTER))
}

pub(crate) fn transient_request(error: &reqwest::Error) -> bool {
    error.is_connect()
        || error.is_timeout()
        || error
            .status()
            .is_some_and(|status| matches!(status.as_u16(), 408 | 429 | 500 | 502 | 503 | 504))
}

/// Upstream `--retry-sleep` expression (`parse_sleep_func`): a number,
/// `linear=START[:END[:STEP]]` or `exp=START[:END[:BASE]]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SleepExpression {
    exponential: bool,
    start: f64,
    limit: f64,
    step: f64,
}

impl SleepExpression {
    /// Parse a stripped expression; `None` when it does not match upstream's
    /// pattern.
    pub fn parse(text: &str) -> Option<Self> {
        static PATTERN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
            let number = r"([0-9]+(?:\.[0-9]+)?)";
            regex::Regex::new(&format!(
                r"^(?:(linear|exp)=)?{number}(?::{number}?)?(?::{number})?$"
            ))
            .unwrap()
        });
        let found = PATTERN.captures(crate::batch::py_strip(text))?;
        let number = |index| {
            found
                .get(index)
                .and_then(|m| m.as_str().parse::<f64>().ok())
        };
        let op = found.get(1).map(|m| m.as_str());
        let start = number(2)?;
        let limit = number(3);
        let step = number(4);
        let exponential = op == Some("exp");
        let step = match (exponential, step) {
            (_, Some(step)) => step,
            (true, None) => 2.0,
            (false, None) if op.is_some() || limit.is_some() => start,
            (false, None) => 0.0,
        };
        Some(Self {
            exponential,
            start,
            limit: limit.unwrap_or(f64::INFINITY),
            step,
        })
    }

    /// The delay before retry `n` (zero-based).
    pub fn delay(&self, n: u32) -> f64 {
        let value = if self.exponential {
            self.start * self.step.powf(f64::from(n))
        } else {
            self.start + self.step * f64::from(n)
        };
        value.min(self.limit)
    }
}

/// Upstream `--retry-sleep` functions for download retries.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RetrySleep {
    pub http: Option<SleepExpression>,
    pub fragment: Option<SleepExpression>,
}

/// `RetryManager.report_retry` for a retry: the `Got error` line, then the
/// configured sleep (announced like upstream) or the native backoff.
pub(crate) fn announce_retry(
    error: &Error,
    attempt: u32,
    retries: u32,
    fragment: Option<u32>,
    sleep: Option<SleepExpression>,
) -> Option<Duration> {
    let retries_text = if retries == u32::MAX {
        "inf".to_owned()
    } else {
        retries.to_string()
    };
    let suffix = fragment.map_or_else(String::new, |index| format!(" fragment {index}"));
    crate::screen::screen(&format!(
        "[download] Got error: {}. Retrying{suffix} ({}/{retries_text})...",
        upstream_error_text(error),
        attempt + 1
    ));
    let delay = sleep?.delay(attempt);
    if delay > 0.0 {
        crate::screen::screen(&format!("Sleeping {delay:.2} seconds ..."));
    }
    Some(Duration::from_secs_f64(delay.clamp(0.0, 86_400.0)))
}

/// The final error of a retryable failure after `retries` retries.
pub(crate) fn exhausted(error: Error, retries: u32) -> Error {
    Error::RetriesExhausted {
        text: upstream_error_text(&error),
        retries,
        source: Box::new(error),
    }
}

/// Upstream-style error text: `HTTP Error CODE: Reason` for HTTP statuses.
pub fn upstream_error_text(error: &Error) -> String {
    let status = match error {
        Error::Http(source) | Error::HttpRetryAfter { source, .. } => source.status(),
        _ => None,
    };
    match status {
        Some(status) => {
            let reason = match error {
                Error::Http(source) | Error::HttpRetryAfter { source, .. } => reason_phrase(source),
                _ => None,
            };
            format!(
                "HTTP Error {}: {}",
                status.as_u16(),
                reason
                    .as_deref()
                    .unwrap_or(status.canonical_reason().unwrap_or(""))
            )
        }
        None => match error {
            Error::Http(source) => urllib_error(source),
            _ => error.to_string(),
        },
    }
}

/// urllib's text for a transport error: `[Errno N] TEXT` from the OS error.
pub(crate) fn urllib_error(error: &reqwest::Error) -> String {
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(error);
    // Without an OS error, the innermost cause says the most (e.g. TLS).
    let mut innermost = error.to_string();
    while let Some(current) = source {
        innermost = current.to_string();
        if let Some(io) = current.downcast_ref::<std::io::Error>()
            && let Some(code) = io.raw_os_error()
        {
            let text = io.to_string();
            let text = text
                .strip_suffix(&format!(" (os error {code})"))
                .unwrap_or(&text);
            // CPython words Windows socket errors as `[WinError N]`, with
            // FormatMessage's trailing dots stripped.
            if cfg!(windows) {
                return format!("[WinError {code}] {}", text.trim_end_matches('.'));
            }
            return format!("[Errno {code}] {text}");
        }
        source = current.source();
    }
    innermost
}

/// The server's reason phrase (like urllib's `HTTPError.reason`), which
/// reqwest keeps only in its status error text: `... (CODE REASON) for url (...)`.
fn reason_phrase(error: &reqwest::Error) -> Option<String> {
    let text = error.to_string();
    let rest = text
        .strip_prefix("HTTP status client error (")
        .or_else(|| text.strip_prefix("HTTP status server error ("))?;
    let inner = match rest.rfind(") for url (") {
        Some(end) => &rest[..end],
        None => rest.strip_suffix(')')?,
    };
    let (_, reason) = inner.split_once(' ')?;
    Some(reason.to_owned())
}

pub(crate) fn backoff(attempt: u32, retry_after: Duration) {
    // Bounded exponential backoff: 100, 200, 400, 800, then 1000 ms.
    std::thread::sleep(delay(attempt, retry_after));
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderValue;

    #[test]
    fn delay_seconds_are_strict_and_bounded() {
        for (value, seconds) in [
            ("0", 0),
            ("0002", 2),
            (" \t12 \t", 12),
            ("60", 60),
            ("61", 60),
            ("184467440737095516160000", 60),
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(RETRY_AFTER, HeaderValue::from_str(value).unwrap());
            assert_eq!(
                retry_after(&headers, SystemTime::UNIX_EPOCH),
                Some(Duration::from_secs(seconds)),
                "{value:?}"
            );
        }
        for value in ["", " \t", "+1", "-1", "1.0", "1, 2", "1 2"] {
            let mut headers = HeaderMap::new();
            headers.insert(RETRY_AFTER, HeaderValue::from_str(value).unwrap());
            assert_eq!(
                retry_after(&headers, SystemTime::UNIX_EPOCH),
                None,
                "{value:?}"
            );
        }
        let mut headers = HeaderMap::new();
        assert_eq!(retry_after(&headers, SystemTime::UNIX_EPOCH), None);
        headers.append(RETRY_AFTER, HeaderValue::from_static("1"));
        headers.append(RETRY_AFTER, HeaderValue::from_static("1"));
        assert_eq!(retry_after(&headers, SystemTime::UNIX_EPOCH), None);
        headers.clear();
        headers.insert(RETRY_AFTER, HeaderValue::from_bytes(&[0xff]).unwrap());
        assert_eq!(retry_after(&headers, SystemTime::UNIX_EPOCH), None);
    }

    #[test]
    fn http_dates_use_the_local_clock_and_remain_bounded() {
        let date = "Wed, 21 Oct 2015 07:28:00 GMT";
        let timestamp = httpdate::parse_http_date(date).unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, HeaderValue::from_static(date));
        for (now, expected) in [
            (
                timestamp - Duration::from_secs(120),
                Duration::from_secs(60),
            ),
            (timestamp - Duration::from_secs(60), Duration::from_secs(60)),
            (
                timestamp - Duration::from_millis(1500),
                Duration::from_millis(1500),
            ),
            (timestamp, Duration::ZERO),
            (timestamp + Duration::from_secs(1), Duration::ZERO),
        ] {
            assert_eq!(retry_after(&headers, now), Some(expected));
        }
        headers.insert(
            RETRY_AFTER,
            HeaderValue::from_static(" \tWed, 21 Oct 2015 07:28:00 GMT\t "),
        );
        assert_eq!(retry_after(&headers, timestamp), Some(Duration::ZERO));
        // Response Date is not a trusted clock: a stale or skewed server date
        // must not turn an expired hint into a long wait.
        headers.insert(
            reqwest::header::DATE,
            HeaderValue::from_static("Tue, 20 Oct 2015 07:28:00 GMT"),
        );
        assert_eq!(retry_after(&headers, timestamp), Some(Duration::ZERO));
        headers.append(RETRY_AFTER, HeaderValue::from_static(date));
        assert_eq!(retry_after(&headers, timestamp), None);
    }

    #[test]
    fn malformed_and_obsolete_dates_use_local_backoff() {
        for value in [
            "Wednesday, 21-Oct-15 07:28:00 GMT",
            "Wed Oct 21 07:28:00 2015",
            "Wed, 21 Oct 2015 07:28:00 UTC",
            "Thu, 21 Oct 2015 07:28:00 GMT",
            "Wed, 32 Oct 2015 07:28:00 GMT",
            "Wed, 21 Oct 2015 25:28:00 GMT",
            "Wed, 21 Oct 2015 07:28:00 GMT, 1",
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(RETRY_AFTER, HeaderValue::from_static(value));
            assert_eq!(
                retry_after(&headers, SystemTime::UNIX_EPOCH),
                None,
                "{value}"
            );
        }
    }

    #[test]
    fn server_hint_extends_backoff_with_a_fixed_cap() {
        for (attempt, milliseconds) in [
            (0, 100),
            (1, 200),
            (2, 400),
            (3, 800),
            (4, 1000),
            (255, 1000),
        ] {
            assert_eq!(
                delay(attempt, Duration::ZERO),
                Duration::from_millis(milliseconds)
            );
            assert_eq!(
                delay(attempt, Duration::from_secs(2)),
                Duration::from_secs(2)
            );
            assert_eq!(delay(attempt, Duration::MAX), MAX_RETRY_AFTER);
        }
    }
}

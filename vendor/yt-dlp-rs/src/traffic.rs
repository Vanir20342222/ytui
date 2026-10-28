//! `--print-traffic`: upstream enables `http.client`'s debug output, which
//! prints each request as `send: b'…'` and each response's status line and
//! headers as `reply: '…'` and `header: NAME: VALUE`. Here reqwest's
//! connection tracing supplies the bytes actually written and read; they
//! are printed on the CLI's thread after each request.
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};

#[derive(Default)]
struct Connection {
    /// Response bytes until the end of the header block.
    pending: Vec<u8>,
    /// Whether the current response's headers were printed.
    replied: bool,
}

#[derive(Default)]
struct State {
    connections: HashMap<String, Connection>,
    lines: Vec<String>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);
static LOGGER: OnceLock<Logger> = OnceLock::new();

struct Logger;

impl log::Log for Logger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.target().starts_with("reqwest::connect::verbose")
    }

    fn log(&self, record: &log::Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let text = record.args().to_string();
        let Some((id, rest)) = text.split_once(' ') else {
            return;
        };
        let (write, escaped) = if let Some(data) = rest.strip_prefix("write: ") {
            (true, data)
        } else if let Some(data) = rest.strip_prefix("write (vectored): ") {
            (true, data)
        } else if let Some(data) = rest.strip_prefix("read: ") {
            (false, data)
        } else {
            return;
        };
        let bytes = unescape(escaped);
        let mut guard = STATE.lock().unwrap();
        let state = guard.get_or_insert_with(State::default);
        let connection = state.connections.entry(id.to_owned()).or_default();
        if write {
            // A new request on the connection: its response follows.
            connection.pending.clear();
            connection.replied = false;
            state
                .lines
                .push(format!("send: {}", crate::batch::py_bytes_repr(&bytes)));
            return;
        }
        if connection.replied {
            return;
        }
        connection.pending.extend_from_slice(&bytes);
        let Some(end) = find(&connection.pending, b"\r\n\r\n") else {
            return;
        };
        connection.replied = true;
        let head = String::from_utf8_lossy(&connection.pending[..end + 2]).into_owned();
        connection.pending.clear();
        let mut lines = head.split_inclusive("\r\n");
        if let Some(status) = lines.next() {
            state
                .lines
                .push(format!("reply: {}", crate::batch::py_repr(status)));
        }
        for line in lines {
            let line = line.trim_end_matches("\r\n");
            if let Some((name, value)) = line.split_once(':') {
                state
                    .lines
                    .push(format!("header: {name}: {}", value.trim_start()));
            }
        }
    }

    fn flush(&self) {}
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Bytes from reqwest's `Escape` debug text: one or more `b"…"` chunks.
fn unescape(text: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != 'b' || chars.peek() != Some(&'"') {
            continue;
        }
        chars.next();
        while let Some(c) = chars.next() {
            match c {
                '"' => break,
                '\\' => match chars.next() {
                    Some('n') => bytes.push(b'\n'),
                    Some('r') => bytes.push(b'\r'),
                    Some('t') => bytes.push(b'\t'),
                    Some('0') => bytes.push(0),
                    Some('x') => {
                        let hex: String = chars.by_ref().take(2).collect();
                        bytes.push(u8::from_str_radix(&hex, 16).unwrap_or(0));
                    }
                    Some(other) => bytes.push(other as u8),
                    None => {}
                },
                c => {
                    let mut buffer = [0; 4];
                    bytes.extend_from_slice(c.encode_utf8(&mut buffer).as_bytes());
                }
            }
        }
    }
    bytes
}

/// Install the trace capture (once).
pub(crate) fn enable() {
    if LOGGER.set(Logger).is_ok() {
        *STATE.lock().unwrap() = Some(State::default());
        let _ = log::set_logger(LOGGER.get().unwrap());
        log::set_max_level(log::LevelFilter::Trace);
    }
}

/// The captured lines since the last call.
pub(crate) fn take_lines() -> Vec<String> {
    STATE
        .lock()
        .unwrap()
        .as_mut()
        .map(|state| std::mem::take(&mut state.lines))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    #[test]
    fn escaped_chunks_round_trip() {
        assert_eq!(
            super::unescape(r#"b"GET / HTTP/1.1\r\nx: \x7f\"\\\0"b"tail""#),
            b"GET / HTTP/1.1\r\nx: \x7f\"\\\0tail"
        );
    }
}

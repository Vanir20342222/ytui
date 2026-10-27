//! Batch-file URL lists, following upstream `read_batch_urls`/`get_urls`.

use std::{
    io::Read,
    path::{Path, PathBuf},
};

/// Python `str.isspace`: Rust's White_Space set plus the ASCII separators
/// U+001C..=U+001F, which Python also treats as whitespace.
pub fn is_py_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// Python `repr(str)`: single quotes unless only double quotes avoid
/// escaping, with backslash escapes for quotes, `\\`, `\t`, `\n`, `\r` and
/// other non-printable characters.
pub fn py_repr(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::from(quote);
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 || (0x7f..=0x9f).contains(&(c as u32)) => {
                out.push_str(&format!("\\x{:02x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// Python `str.strip()` with no arguments.
pub fn py_strip(text: &str) -> &str {
    text.trim_matches(is_py_space)
}

/// Decode UTF-8 like Python's `errors='ignore'`: invalid bytes are dropped.
fn decode_ignoring_errors(bytes: &[u8]) -> String {
    bytes.utf8_chunks().map(|chunk| chunk.valid()).collect()
}

/// Split like Python's universal-newline text iteration (`\r\n`, `\r`, `\n`).
fn universal_lines(text: &str) -> impl Iterator<Item = &str> {
    let mut rest = text;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let Some(end) = rest.find(['\r', '\n']) else {
            return Some(std::mem::take(&mut rest));
        };
        let line = &rest[..end];
        let terminator = if rest[end..].starts_with("\r\n") {
            2
        } else {
            1
        };
        rest = &rest[end + terminator..];
        Some(line)
    })
}

fn fixup(line: &str) -> Option<&str> {
    let mut line = line;
    // Upstream strips a mis-decoded and a decoded BOM from every line, in order.
    for bom in ["\u{ef}\u{bb}\u{bf}", "\u{feff}"] {
        line = line.strip_prefix(bom).unwrap_or(line);
    }
    let line = line.trim_start_matches(is_py_space);
    if line.is_empty() || line.starts_with(['#', ';', ']']) {
        return None;
    }
    // `#` is part of a URI unless it follows whitespace (`\s#`).
    let mut previous_space = false;
    let mut end = line.len();
    for (index, c) in line.char_indices() {
        if c == '#' && previous_space {
            // Exclude the whitespace character before `#`.
            end = line[..index]
                .char_indices()
                .next_back()
                .map_or(0, |(start, _)| start);
            break;
        }
        previous_space = is_py_space(c);
    }
    let url = line[..end].trim_end_matches(is_py_space);
    (!url.is_empty()).then_some(url)
}

/// Parse batch-file bytes into URLs, skipping blank and comment lines.
pub fn read_batch_urls(bytes: &[u8]) -> Vec<String> {
    let text = decode_ignoring_errors(bytes);
    universal_lines(&text)
        .filter_map(fixup)
        .map(str::to_owned)
        .collect()
}

/// Upstream `expand_path`: a leading `~`/`~/` uses `HOME`, then `$NAME` and
/// `${NAME}` (ASCII word names) expand from the environment; unknown
/// variables are kept. `~user` forms and a missing `HOME` leave `~` unchanged.
pub fn expand_path(path: &str) -> PathBuf {
    PathBuf::from(expand_path_text(path))
}

/// [`expand_path`] as text, for templates and `--paths` values.
pub fn expand_path_text(path: &str) -> String {
    let path = match (path.strip_prefix('~'), std::env::var("HOME")) {
        (Some(rest), Ok(home)) if rest.is_empty() || rest.starts_with('/') => {
            let expanded = format!("{}{rest}", home.trim_end_matches('/'));
            if expanded.is_empty() {
                "/".to_owned()
            } else {
                expanded
            }
        }
        _ => path.to_owned(),
    };
    expand_vars(&path)
}

fn expand_vars(path: &str) -> String {
    let is_word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut output = String::with_capacity(path.len());
    let mut rest = path;
    while let Some(index) = rest.find('$') {
        output.push_str(&rest[..index]);
        let after = &rest[index + 1..];
        let (name, length) = if let Some(braced) = after.strip_prefix('{') {
            match braced.find('}') {
                Some(end) => (Some(&braced[..end]), end + 2),
                // `\{[^}]*\}` cannot match; `\w+` cannot match `{` either.
                None => (None, 0),
            }
        } else {
            let end = after.find(|c| !is_word(c)).unwrap_or(after.len());
            ((end > 0).then(|| &after[..end]), end)
        };
        match name.and_then(|name| std::env::var(name).ok()) {
            Some(value) => output.push_str(&value),
            None => output.push_str(&rest[index..index + 1 + length]),
        }
        rest = &after[length..];
    }
    output.push_str(rest);
    output
}

/// Read URLs from `-` (stdin) or an expanded batch-file path.
pub fn load_batch_file(spec: &str) -> std::io::Result<Vec<String>> {
    let mut bytes = Vec::new();
    if spec == "-" {
        std::io::stdin().lock().read_to_end(&mut bytes)?;
    } else {
        bytes = std::fs::read(Path::new(&expand_path(spec)))?;
    }
    Ok(read_batch_urls(&bytes))
}

/// Python `repr(bytes)`.
pub fn py_bytes_repr(bytes: &[u8]) -> String {
    let quote = if bytes.contains(&b'\'') && !bytes.contains(&b'"') {
        '"'
    } else {
        '\''
    };
    let mut text = format!("b{quote}");
    for &byte in bytes {
        match byte {
            b'\\' => text.push_str("\\\\"),
            b'\t' => text.push_str("\\t"),
            b'\n' => text.push_str("\\n"),
            b'\r' => text.push_str("\\r"),
            byte if byte == quote as u8 => {
                text.push('\\');
                text.push(quote);
            }
            0x20..=0x7e => text.push(byte as char),
            byte => text.push_str(&format!("\\x{byte:02x}")),
        }
    }
    text.push(quote);
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_upstream_get_urls_cases() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/batch/cases.json")).unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let hex = case["batch_hex"].as_str().unwrap();
            let bytes: Vec<u8> = (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                .collect();
            let mut urls = read_batch_urls(&bytes);
            urls.extend(
                case["urls"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|url| py_strip(url.as_str().unwrap()).to_owned()),
            );
            let expected: Vec<String> = serde_json::from_value(case["expected"].clone()).unwrap();
            assert_eq!(urls, expected, "{}", case["name"]);
        }
    }

    #[test]
    fn upstream_read_batch_urls_vector() {
        // test_utils.py test_read_batch_urls, with the mis-decoded BOM.
        let text = "\u{ef}\u{bb}\u{bf} foo\n            bar\r\n            baz\n            # More after this line\r\n            ; or after this\n            bam";
        assert_eq!(
            read_batch_urls(text.as_bytes()),
            ["foo", "bar", "baz", "bam"]
        );
    }

    #[test]
    fn comments_bom_newlines_and_invalid_bytes() {
        let bytes = b"\xef\xbb\xbfhttp://a/#frag  # note\r]x\rhttp://b/\xff\xfec\n\x1c\t#c\n;d\nhttp://e/\t#\n  \n";
        assert_eq!(
            read_batch_urls(bytes),
            ["http://a/#frag", "http://b/c", "http://e/"]
        );
        assert_eq!(py_strip("\u{1f} x \u{3000}"), "x");
    }

    #[test]
    fn expands_variables_like_posixpath() {
        // SAFETY: tests in this module do not read these variables concurrently.
        unsafe { std::env::set_var("YTDLP_RS_BATCH_TEST", "v") };
        assert_eq!(
            expand_vars("$YTDLP_RS_BATCH_TEST/${YTDLP_RS_BATCH_TEST}x/$YTDLP_RS_UNSET/${a/$/${"),
            "v/vx/$YTDLP_RS_UNSET/${a/$/${"
        );
    }
}

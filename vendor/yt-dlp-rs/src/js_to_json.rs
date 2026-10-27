//! Upstream `js_to_json(code)` (no `vars`, not strict): JavaScript object
//! literals as JSON text, and `_parse_json(..., ignore_extra=True)`'s
//! `raw_decode(s.lstrip())` of its first value.
//!
//! The final substitution is upstream's single verbose regex; its
//! alternatives (strings, comments, trailing commas, `void 0`, identifiers,
//! hex/octal integers, numeric keys and `!`) are tried in order at each
//! position, as `re.sub` does, with the look-arounds checked by hand.
use regex::Regex;
use serde_json::Value;
use std::sync::LazyLock;

const QUOTES: &[u8] = b"'\"`";

fn regex(pattern: &str) -> Regex {
    Regex::new(pattern).expect("static pattern")
}

/// The length of a `Q(?:\\.|[^\\Q])*Q` string at `index` (dot-all).
fn string_at(bytes: &[u8], index: usize) -> Option<usize> {
    let quote = *bytes.get(index)?;
    if !QUOTES.contains(&quote) {
        return None;
    }
    let mut position = index + 1;
    while position < bytes.len() {
        match bytes[position] {
            b'\\' if position + 1 < bytes.len() => position += 2,
            b'\\' => return None,
            byte if byte == quote => return Some(position + 1 - index),
            _ => position += 1,
        }
    }
    None
}

/// `/\*(?:(?!\*/).)*?\*/|//[^\n]*\n` at `index`.
fn comment_at(bytes: &[u8], index: usize) -> Option<usize> {
    let rest = &bytes[index..];
    if rest.starts_with(b"/*") {
        let end = rest[2..].windows(2).position(|pair| pair == b"*/")?;
        return Some(end + 4);
    }
    if rest.starts_with(b"//") {
        let end = rest.iter().position(|byte| *byte == b'\n')?;
        return Some(end + 1);
    }
    None
}

/// `\s*(?:COMMENT)?\s*`: the length skipped.
fn skip(bytes: &[u8], index: usize) -> usize {
    let mut position = index;
    while position < bytes.len() && bytes[position].is_ascii_whitespace() {
        position += 1;
    }
    if let Some(length) = comment_at(bytes, position) {
        position += length;
        while position < bytes.len() && bytes[position].is_ascii_whitespace() {
            position += 1;
        }
    }
    position - index
}

fn is_word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// The first alternative of the final substitution matching at `index`.
fn token_at(bytes: &[u8], index: usize) -> Option<usize> {
    if let Some(length) = string_at(bytes, index) {
        return Some(length);
    }
    if let Some(length) = comment_at(bytes, index) {
        return Some(length);
    }
    let byte = bytes[index];
    // `,(?=SKIP[\]}])`.
    if byte == b',' {
        let after = index + 1 + skip(bytes, index + 1);
        if matches!(bytes.get(after), Some(b']' | b'}')) {
            return Some(1);
        }
    }
    // `void\s0`.
    if bytes[index..].starts_with(b"void")
        && bytes.get(index + 4).is_some_and(u8::is_ascii_whitespace)
        && bytes.get(index + 5) == Some(&b'0')
    {
        return Some(6);
    }
    // `(?:(?<![0-9])[eE]|[a-df-zA-DF-Z_$])[.a-zA-Z_$0-9]*`.
    let identifier_start = if matches!(byte, b'e' | b'E') {
        index == 0 || !bytes[index - 1].is_ascii_digit()
    } else {
        byte.is_ascii_alphabetic() || byte == b'_' || byte == b'$'
    };
    if identifier_start {
        let mut end = index + 1;
        while end < bytes.len()
            && (bytes[end].is_ascii_alphanumeric() || matches!(bytes[end], b'.' | b'_' | b'$'))
        {
            end += 1;
        }
        return Some(end - index);
    }
    // `\b(?:0[xX][0-9a-fA-F]+|(?<!\.)0+[0-7]+)(?:SKIP:)?`.
    let boundary = index == 0 || !is_word(bytes[index - 1]);
    if boundary && byte == b'0' {
        let number = if matches!(bytes.get(index + 1), Some(b'x' | b'X')) {
            let digits = bytes[index + 2..]
                .iter()
                .take_while(|byte| byte.is_ascii_hexdigit())
                .count();
            (digits > 0).then_some(2 + digits)
        } else if index == 0 || bytes[index - 1] != b'.' {
            // `0+[0-7]+`: at least two digits, all octal.
            let digits = bytes[index..]
                .iter()
                .take_while(|byte| (b'0'..=b'7').contains(byte))
                .count();
            (digits >= 2).then_some(digits)
        } else {
            None
        };
        if let Some(length) = number {
            let after = index + length;
            let skipped = skip(bytes, after);
            if bytes.get(after + skipped) == Some(&b':') {
                return Some(length + skipped + 1);
            }
            return Some(length);
        }
    }
    // `[0-9]+(?=SKIP:)`.
    if byte.is_ascii_digit() {
        let digits = bytes[index..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count();
        let after = index + digits;
        if bytes.get(after + skip(bytes, after)) == Some(&b':') {
            return Some(digits);
        }
    }
    // `!+`.
    if byte == b'!' {
        return Some(bytes[index..].iter().take_while(|b| **b == b'!').count());
    }
    None
}

/// `process_escape` over `(")|\\(.)`, with raw control characters as
/// `\u00XX` (Python's `strict=False` decoder accepts them raw).
fn escape_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => match chars.next() {
                Some(escape) if "\"\\bfnrtu".contains(escape) => {
                    out.push('\\');
                    out.push(escape);
                }
                Some('x') => out.push_str("\\u00"),
                Some('\n') => {}
                Some(other) => push_char(&mut out, other),
                // `\\(.)` needs a character; a lone backslash stays.
                None => out.push('\\'),
            },
            other => push_char(&mut out, other),
        }
    }
    out
}

fn push_char(out: &mut String, c: char) {
    if (c as u32) < 0x20 {
        out.push_str(&format!("\\u{:04x}", c as u32));
    } else {
        out.push(c);
    }
}

/// `fix_kv` for one matched token.
fn fix_kv(token: &str) -> String {
    match token {
        "true" | "false" | "null" => return token.to_owned(),
        "undefined" | "void 0" => return "null".into(),
        "," => return String::new(),
        _ => {}
    }
    if token.starts_with("/*") || token.starts_with("//") || token.starts_with('!') {
        return String::new();
    }
    let first = token.as_bytes()[0];
    if QUOTES.contains(&first) {
        let inner = &token[1..token.len() - 1];
        let inner = if first == b'`' {
            static TEMPLATE: LazyLock<Regex> = LazyLock::new(|| regex(r"(?s)\$\{([^}]+)\}"));
            TEMPLATE
                .replace_all(inner, |captures: &regex::Captures<'_>| {
                    let evaluated = js_to_json(&captures[1]);
                    if evaluated.starts_with('"')
                        && let Ok(Value::String(text)) = serde_json::from_str::<Value>(&evaluated)
                    {
                        return text;
                    }
                    evaluated
                })
                .into_owned()
        } else {
            inner.to_owned()
        };
        return format!("\"{}\"", escape_string(&inner));
    }
    // `INTEGER_TABLE`: hex and octal literals (keys keep their colon).
    let key = token.ends_with(':');
    let number = token.trim_end_matches(':');
    let number = number.trim_end_matches(|c: char| c.is_whitespace());
    let parsed = if let Some(hex) = number
        .strip_prefix("0x")
        .or_else(|| number.strip_prefix("0X"))
    {
        u128::from_str_radix(hex, 16).ok()
    } else if number.len() >= 2
        && number.starts_with('0')
        && number.bytes().all(|b| (b'0'..=b'7').contains(&b))
    {
        u128::from_str_radix(number, 8).ok()
    } else {
        None
    };
    // A comment between the number and its colon only matches upstream's
    // table when `SKIP_RE` consumed it; plain whitespace is the common case.
    if let Some(value) = parsed
        && (number.len() + 1 == token.len()
            || !key
            || token[number.len()..]
                .trim_end_matches(':')
                .trim()
                .is_empty())
    {
        return if key {
            format!("\"{value}\":")
        } else {
            value.to_string()
        };
    }
    format!("\"{token}\"")
}

/// Upstream `js_to_json(code)`.
pub fn js_to_json(code: &str) -> String {
    static ARRAY: LazyLock<Regex> = LazyLock::new(|| regex(r"(?:new\s+)?Array\((.*?)\)"));
    static MAP: LazyLock<Regex> = LazyLock::new(|| regex(r"new Map\((\[.*?\])?\)"));
    static DATE: LazyLock<Regex> = LazyLock::new(|| {
        regex(r#"new Date\(('(?:\\.|[^\\'])*'|"(?:\\.|[^\\"])*"|`(?:\\.|[^\\`])*`)\)"#)
    });
    static NEW: LazyLock<Regex> = LazyLock::new(|| regex(r"new \w+\((.*?)\)"));
    static PARSE_INT: LazyLock<Regex> = LazyLock::new(|| regex(r"parseInt\([^\d]+(\d+)[^\d]+\)"));
    static IIFE: LazyLock<Regex> = LazyLock::new(|| {
        regex(r#"\(function\([^)]*\)\s*\{[^}]*\}\s*\)\s*\(\s*(["'][^)]*["'])\s*\)"#)
    });
    let code = ARRAY.replace_all(code, "[$1]");
    let code = MAP.replace_all(&code, |captures: &regex::Captures<'_>| {
        let pairs = js_to_json(captures.get(1).map_or("[]", |found| found.as_str()));
        let map: serde_json::Map<String, Value> =
            serde_json::from_str::<Vec<(Value, Value)>>(&pairs)
                .unwrap_or_default()
                .into_iter()
                .map(|(key, value)| {
                    let key = match key {
                        Value::String(text) => text,
                        other => other.to_string(),
                    };
                    (key, value)
                })
                .collect();
        crate::output::python_json_dumps(&Value::Object(map))
    });
    let code = DATE.replace_all(&code, "$1");
    let code = NEW.replace_all(&code, |captures: &regex::Captures<'_>| {
        crate::output::python_json_dumps(&Value::String(captures[0].to_owned()))
    });
    let code = PARSE_INT.replace_all(&code, "$1");
    let code = IIFE.replace_all(&code, "$1");
    let bytes = code.as_bytes();
    let mut out = String::with_capacity(code.len());
    let mut index = 0;
    let mut copied = 0;
    while index < bytes.len() {
        match token_at(bytes, index) {
            Some(length) => {
                out.push_str(&code[copied..index]);
                out.push_str(&fix_kv(&code[index..index + length]));
                index += length;
                copied = index;
            }
            None => {
                // Advance by one character (keeping UTF-8 boundaries).
                index += 1;
                while index < bytes.len() && !code.is_char_boundary(index) {
                    index += 1;
                }
            }
        }
    }
    out.push_str(&code[copied..]);
    out
}

/// `_parse_json(text, transform_source=js_to_json, ignore_extra=True)`:
/// the first JSON value of the transformed text.
pub fn parse_js_value(text: &str) -> Option<Value> {
    let json = js_to_json(text);
    serde_json::Deserializer::from_str(json.trim_start())
        .into_iter::<Value>()
        .next()?
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_upstream_conversions() {
        let cases: Vec<(String, String)> =
            serde_json::from_str(include_str!("../tests/fixtures/js_to_json/cases.json")).unwrap();
        for (input, expected) in cases {
            assert_eq!(js_to_json(&input), expected, "{input:?}");
        }
    }
}

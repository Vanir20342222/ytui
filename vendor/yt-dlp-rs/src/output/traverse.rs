//! Upstream `prepare_outtmpl` object traversal (`_traverse_infodict` with
//! `traverse_obj(..., traverse_string=True)`) over the serialized info dict,
//! and Python `str()`/`json.dumps` renderings of the traversed values.
use crate::{Error, Result};
use serde_json::{Map, Value};

fn invalid(message: impl Into<String>) -> Error {
    Error::InvalidTemplate(message.into())
}

#[derive(Debug, Clone, PartialEq)]
enum Step {
    Key(String),
    Index(i64),
    Slice(Option<i64>, Option<i64>, Option<i64>),
    /// `:` (Python `...`): every value.
    All,
    /// `{a,b.c}`: a dict of the named sub-traversals.
    Dict(Vec<(String, Vec<Step>)>),
}

/// Whether `name` is a plain root field (no traversal syntax).
pub(super) fn is_plain(name: &str) -> bool {
    !name.contains(['.', '{', '}', ':'])
}

fn word(text: &str) -> bool {
    text.chars().all(|c| c.is_alphanumeric() || c == '_')
}

fn number(text: &str) -> bool {
    let digits = text.strip_prefix('-').unwrap_or(text);
    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
}

/// `_from_user_input`, validated against upstream `FIELD_INNER_RE`.
fn step(text: &str) -> Result<Step> {
    if text == ":" {
        return Ok(Step::All);
    }
    if text.contains(':') {
        let parts: Vec<&str> = text.split(':').collect();
        if parts.len() > 3 || parts.iter().any(|part| !part.is_empty() && !number(part)) {
            return Err(invalid(format!("unsupported template traversal {text:?}")));
        }
        let int = |index: usize| parts.get(index).and_then(|part| part.parse().ok());
        return Ok(Step::Slice(int(0), int(1), int(2)));
    }
    if number(text) {
        return text
            .parse()
            .map(Step::Index)
            .map_err(|_| invalid("template traversal index is too large"));
    }
    if word(text) && !text.is_empty() {
        return Ok(Step::Key(text.to_owned()));
    }
    Err(invalid(format!("unsupported template traversal {text:?}")))
}

/// Split `fields` like upstream: `.{...}` groups stay whole, other text
/// splits on `.`, and an empty first or last part is dropped.
fn parse(fields: &str) -> Result<Vec<Step>> {
    let mut parts: Vec<String> = Vec::new();
    let mut rest = fields;
    while let Some(start) = rest.find(".{") {
        let end = rest[start..]
            .find('}')
            .map(|end| start + end)
            .ok_or_else(|| invalid("unclosed template traversal brace"))?;
        parts.extend(rest[..start].split('.').map(str::to_owned));
        parts.push(rest[start + 1..=end].to_owned());
        rest = rest[end + 1..]
            .strip_prefix('.')
            .unwrap_or(&rest[end + 1..]);
    }
    parts.extend(rest.split('.').map(str::to_owned));
    if parts.first().is_some_and(String::is_empty) {
        parts.remove(0);
    }
    if parts.last().is_some_and(String::is_empty) {
        parts.pop();
    }
    // An empty path (`%()s`) is the whole info dict.
    parts
        .iter()
        .map(|part| match part.strip_prefix('{') {
            Some(inner) => {
                let inner = inner.strip_suffix('}').unwrap_or(inner);
                inner
                    .split(',')
                    .map(|key| {
                        let path = key.split('.').map(step).collect::<Result<Vec<_>>>()?;
                        Ok((key.to_owned(), path))
                    })
                    .collect::<Result<Vec<_>>>()
                    .map(Step::Dict)
            }
            None => step(part),
        })
        .collect()
}

/// Python `slice.indices` for a sequence of `len` items.
fn slice_indices(
    len: usize,
    start: Option<i64>,
    stop: Option<i64>,
    step: Option<i64>,
) -> Vec<usize> {
    let len = len as i64;
    let step = step.unwrap_or(1);
    if step == 0 {
        return Vec::new();
    }
    let clamp = |value: i64, low: i64, high: i64| value.clamp(low, high);
    let resolve = |value: Option<i64>, default: i64, low: i64, high: i64| match value {
        None => default,
        Some(value) if value < 0 => clamp(value + len, low, high),
        Some(value) => clamp(value, low, high),
    };
    let mut indices = Vec::new();
    if step > 0 {
        let (start, stop) = (resolve(start, 0, 0, len), resolve(stop, len, 0, len));
        let mut index = start;
        while index < stop {
            indices.push(index as usize);
            index += step;
        }
    } else {
        let (start, stop) = (
            resolve(start, len - 1, -1, len - 1),
            resolve(stop, -1, -1, len - 1),
        );
        let mut index = start;
        while index > stop {
            indices.push(index as usize);
            index += step;
        }
    }
    indices
}

fn index(len: usize, index: i64) -> Option<usize> {
    let index = if index < 0 { index + len as i64 } else { index };
    usize::try_from(index).ok().filter(|index| *index < len)
}

/// One traversal step over every current object; branching steps yield all
/// their values. `None`s are dropped as upstream does.
fn walk(object: &Value, steps: &[Step], branched: &mut bool, results: &mut Vec<Value>) {
    let Some((first, rest)) = steps.split_first() else {
        results.push(object.clone());
        return;
    };
    let mut values = Vec::new();
    match (first, object) {
        (Step::Key(key), Value::Object(map)) => values.extend(map.get(key).cloned()),
        (Step::Index(position), Value::Array(items)) => {
            values.extend(index(items.len(), *position).map(|position| items[position].clone()));
        }
        (Step::Index(position), Value::String(text)) => {
            let chars: Vec<char> = text.chars().collect();
            values.extend(
                index(chars.len(), *position).map(|position| Value::String(chars[position].into())),
            );
        }
        (Step::Slice(start, stop, step), Value::Array(items)) => {
            *branched = true;
            values.extend(
                slice_indices(items.len(), *start, *stop, *step)
                    .into_iter()
                    .map(|position| items[position].clone()),
            );
        }
        (Step::Slice(start, stop, step), Value::String(text)) => {
            let chars: Vec<char> = text.chars().collect();
            values.push(Value::String(
                slice_indices(chars.len(), *start, *stop, *step)
                    .into_iter()
                    .map(|position| chars[position])
                    .collect(),
            ));
        }
        (Step::All, Value::Array(items)) => {
            *branched = true;
            values.extend(items.iter().cloned());
        }
        (Step::All, Value::Object(map)) => {
            *branched = true;
            values.extend(map.values().cloned());
        }
        (Step::Dict(keys), _) => {
            let mut dict = Map::new();
            for (key, path) in keys {
                if let Some(value) = traverse_steps(object, path) {
                    dict.insert(key.clone(), value);
                }
            }
            values.push(Value::Object(dict));
        }
        _ => {}
    }
    for value in values.into_iter().filter(|value| !value.is_null()) {
        walk(&value, rest, branched, results);
    }
}

fn traverse_steps(root: &Value, steps: &[Step]) -> Option<Value> {
    let mut branched = false;
    let mut results = Vec::new();
    walk(root, steps, &mut branched, &mut results);
    if branched {
        Some(Value::Array(results))
    } else {
        results.into_iter().next()
    }
}

/// Upstream `_traverse_infodict(fields)`: `None` when nothing is found.
pub(super) fn traverse(root: &Value, fields: &str) -> Result<Option<Value>> {
    Ok(traverse_steps(root, &parse(fields)?))
}

/// Python `repr` of a string.
pub(crate) fn py_repr_str(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut output = String::with_capacity(text.len() + 2);
    output.push(quote);
    for c in text.chars() {
        match c {
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            c if c == quote => {
                output.push('\\');
                output.push(c);
            }
            c if (c as u32) < 0x20 || c == '\u{7f}' => {
                output.push_str(&format!("\\x{:02x}", c as u32));
            }
            c => output.push(c),
        }
    }
    output.push(quote);
    output
}

fn py_number(number: &serde_json::Number) -> String {
    if number.is_f64() {
        crate::screen::py_float(number.as_f64().unwrap_or_default())
    } else {
        number.to_string()
    }
}

/// Python `repr` of a JSON value (nested items in `str()` of containers).
pub(super) fn py_repr(value: &Value) -> String {
    match value {
        Value::String(text) => py_repr_str(text),
        other => py_str(other),
    }
}

/// Python `str()` of a JSON value.
pub(super) fn py_str(value: &Value) -> String {
    match value {
        Value::Null => "None".into(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::Number(number) => py_number(number),
        Value::String(text) => text.clone(),
        Value::Array(items) => format!(
            "[{}]",
            items.iter().map(py_repr).collect::<Vec<_>>().join(", ")
        ),
        Value::Object(map) => format!(
            "{{{}}}",
            map.iter()
                .map(|(key, value)| format!("{}: {}", py_repr_str(key), py_repr(value)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// Python `json.dumps` (default separators) with or without `ensure_ascii`.
pub(crate) fn py_json(value: &Value, ensure_ascii: bool) -> String {
    dumps(value, ensure_ascii, None, 0)
}

/// `json.dumps(value, indent=4)`: `,` item separators and nested indentation.
pub(super) fn py_json_indented(value: &Value, ensure_ascii: bool) -> String {
    dumps(value, ensure_ascii, Some(4), 0)
}

fn dumps(value: &Value, ensure_ascii: bool, indent: Option<usize>, level: usize) -> String {
    let string = |text: &str| super::template::python_json_string(text, ensure_ascii);
    let container = |open: char, close: char, items: Vec<String>| {
        if items.is_empty() {
            return format!("{open}{close}");
        }
        match indent {
            None => format!("{open}{}{close}", items.join(", ")),
            Some(width) => {
                let inner = " ".repeat(width * (level + 1));
                let outer = " ".repeat(width * level);
                format!(
                    "{open}\n{inner}{}\n{outer}{close}",
                    items.join(&format!(",\n{inner}"))
                )
            }
        }
    };
    match value {
        Value::Null => "null".into(),
        Value::Bool(value) => value.to_string(),
        Value::Number(number) => py_number(number),
        Value::String(text) => string(text),
        Value::Array(items) => container(
            '[',
            ']',
            items
                .iter()
                .map(|item| dumps(item, ensure_ascii, indent, level + 1))
                .collect(),
        ),
        Value::Object(map) => container(
            '{',
            '}',
            map.iter()
                .map(|(key, value)| {
                    format!(
                        "{}: {}",
                        string(key),
                        dumps(value, ensure_ascii, indent, level + 1)
                    )
                })
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn traversal_matches_upstream_traverse_obj() {
        // Expected values printed by pinned upstream `evaluate_outtmpl`.
        let root = json!({"id": "x", "title": "abcdef", "f": [{"x": 1, "s": "it's"}, {"x": 2, "s": "q\""}, {"x": 3}],
            "d": {"a": 1, "b": null, "c": "z"}, "e": []});
        let text = |fields: &str| traverse(&root, fields).unwrap().map(|value| py_str(&value));
        assert_eq!(text("f.:.y").as_deref(), Some("[]"));
        assert_eq!(text("f.5:.x").as_deref(), Some("[]"));
        assert_eq!(text("f.0").as_deref(), Some("{'x': 1, 's': \"it's\"}"));
        assert_eq!(text("d.{a,c}").as_deref(), Some("{'a': 1, 'c': 'z'}"));
        assert_eq!(text("d.{a,b}").as_deref(), Some("{'a': 1}"));
        assert_eq!(text("d.{q}").as_deref(), Some("{}"));
        assert_eq!(
            text("f.:.{x}").as_deref(),
            Some("[{'x': 1}, {'x': 2}, {'x': 3}]")
        );
        assert_eq!(text("title.-2:").as_deref(), Some("ef"));
        assert_eq!(text("title.::-1").as_deref(), Some("fedcba"));
        assert_eq!(text("title.0:3").as_deref(), Some("abc"));
        assert_eq!(text("title.1").as_deref(), Some("b"));
        assert_eq!(text("title.10"), None);
        assert_eq!(text("f.-1").as_deref(), Some("{'x': 3}"));
        assert_eq!(text("f.:.s").as_deref(), Some("[\"it's\", 'q\"']"));
        assert_eq!(text("f.1:3.x").as_deref(), Some("[2, 3]"));
        assert_eq!(text("e").as_deref(), Some("[]"));
        assert_eq!(text("e.0"), None);
        assert_eq!(text("d").as_deref(), Some("{'a': 1, 'b': None, 'c': 'z'}"));
        assert_eq!(text("f.x"), None);
        assert_eq!(text("f.0.1"), None);
        assert_eq!(text("d.1"), None);
        assert_eq!(text(".title").as_deref(), Some("abcdef"));
        let value = traverse(&root, "f.:.s").unwrap().unwrap();
        assert_eq!(py_json(&value, false), "[\"it's\", \"q\\\"\"]");
        assert_eq!(
            py_json(&traverse(&root, "f.0").unwrap().unwrap(), true),
            "{\"x\": 1, \"s\": \"it's\"}"
        );
        for fields in ["f.a-b", "f.1:2:3:4", "f.{x", "f.x y"] {
            assert!(traverse(&root, fields).is_err(), "{fields}");
        }
    }
}

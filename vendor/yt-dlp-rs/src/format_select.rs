//! A port of upstream `YoutubeDL.build_format_selector`: the `tokenize`-based
//! parser (with `_remove_unused_ops` and the digit-prefix workaround), format
//! filters (`_build_format_filter`) and selector evaluation over format dicts,
//! including `_merge`.
use crate::info::PyNumber;
use serde_json::Value;

/// Upstream `syntax_error` text, or a filter's `SyntaxError`/`ValueError`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectorError(pub String);

impl std::fmt::Display for SelectorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The length of upstream's random letter prefix before each digit run.
const DIGIT_PREFIX: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Name,
    Op,
    /// Strings and other tokens `_remove_unused_ops` only keeps in brackets.
    Other,
}

#[derive(Debug, Clone)]
struct Token {
    kind: Kind,
    text: String,
    /// Column in the digit-prefixed line, as `tokenize` reports it.
    column: usize,
}

/// Python 3.15's exact operator tokens, longest first.
const OPERATORS: [&str; 47] = [
    "**=", "...", "//=", "<<=", ">>=", "!=", "%=", "&=", "**", "*=", "+=", "-=", "->", "//", "/=",
    ":=", "<<", "<=", "==", ">=", ">>", "@=", "^=", "|=", "!", "%", "&", "(", ")", "*", "+", ",",
    "-", ".", "/", ":", ";", "<", "=", ">", "@", "[", "]", "^", "{", "|", "}",
];

const STRING_PREFIXES: [&str; 11] = ["r", "u", "b", "br", "rb", "f", "fr", "rf", "t", "tr", "rt"];

/// `tokenize.tokenize` over the digit-prefixed spec; `Err` is a `TokenError`.
fn tokenize(spec: &str) -> Result<Vec<Token>, ()> {
    let chars: Vec<char> = spec.chars().collect();
    let mut tokens: Vec<Token> = Vec::new();
    let mut depth: Vec<char> = Vec::new();
    let (mut index, mut column) = (0, 0);
    let mut in_digits = false;
    // Advance one character, accounting for the digit-run prefix.
    let advance = |index: &mut usize, column: &mut usize, in_digits: &mut bool| {
        let c = chars[*index];
        if c.is_ascii_digit() && !*in_digits {
            *column += DIGIT_PREFIX;
            *in_digits = true;
        } else if !(c.is_ascii_digit() || (c == '_' && *in_digits)) {
            *in_digits = false;
        }
        *column += 1;
        *index += 1;
    };
    while index < chars.len() {
        let c = chars[index];
        match c {
            ' ' | '\t' | '\x0c' => advance(&mut index, &mut column, &mut in_digits),
            '\n' | '\r' => {
                index += 1;
                if c == '\r' && chars.get(index) == Some(&'\n') {
                    index += 1;
                }
                column = 0;
                in_digits = false;
            }
            '#' => {
                while index < chars.len() && !matches!(chars[index], '\n' | '\r') {
                    index += 1;
                }
            }
            '\\' => match chars.get(index + 1) {
                Some('\n') => {
                    index += 2;
                    column = 0;
                    in_digits = false;
                }
                Some('\r') => {
                    index += 2;
                    if chars.get(index) == Some(&'\n') {
                        index += 1;
                    }
                    column = 0;
                    in_digits = false;
                }
                _ => return Err(()),
            },
            '\'' | '"' => {
                let start = column;
                let mut text = String::new();
                // A valid prefix name just before the quote belongs to it.
                if let Some(last) = tokens.last()
                    && last.kind == Kind::Name
                    && last.column + last.text.chars().count() == column
                    && STRING_PREFIXES.contains(&last.text.to_lowercase().as_str())
                {
                    let prefix = tokens.pop().unwrap();
                    text = prefix.text;
                }
                let start = if text.is_empty() {
                    start
                } else {
                    start - text.chars().count()
                };
                let triple = chars.get(index + 1) == Some(&c) && chars.get(index + 2) == Some(&c);
                let quote_len = if triple { 3 } else { 1 };
                for _ in 0..quote_len {
                    text.push(c);
                    advance(&mut index, &mut column, &mut in_digits);
                }
                loop {
                    let Some(&next) = chars.get(index) else {
                        return Err(());
                    };
                    if next == '\\' {
                        text.push(next);
                        advance(&mut index, &mut column, &mut in_digits);
                        if let Some(&escaped) = chars.get(index) {
                            text.push(escaped);
                            if matches!(escaped, '\n' | '\r') {
                                index += 1;
                                column = 0;
                            } else {
                                advance(&mut index, &mut column, &mut in_digits);
                            }
                        }
                        continue;
                    }
                    if !triple && matches!(next, '\n' | '\r') {
                        return Err(());
                    }
                    if next == c
                        && (!triple
                            || (chars.get(index + 1) == Some(&c)
                                && chars.get(index + 2) == Some(&c)))
                    {
                        for _ in 0..quote_len {
                            text.push(c);
                            advance(&mut index, &mut column, &mut in_digits);
                        }
                        break;
                    }
                    text.push(next);
                    if matches!(next, '\n' | '\r') {
                        index += 1;
                        column = 0;
                    } else {
                        advance(&mut index, &mut column, &mut in_digits);
                    }
                }
                tokens.push(Token {
                    kind: Kind::Other,
                    text,
                    column: start,
                });
            }
            c if c.is_alphanumeric() || c == '_' || !c.is_ascii() => {
                let start = column;
                let mut text = String::new();
                while let Some(&next) = chars.get(index) {
                    if !(next.is_alphanumeric() || next == '_' || !next.is_ascii())
                        || matches!(next, '\n' | '\r' | ' ' | '\t' | '\x0c')
                    {
                        break;
                    }
                    text.push(next);
                    advance(&mut index, &mut column, &mut in_digits);
                }
                tokens.push(Token {
                    kind: Kind::Name,
                    text,
                    column: start,
                });
            }
            _ => {
                let rest: String = chars[index..chars.len().min(index + 3)].iter().collect();
                let operator = OPERATORS
                    .iter()
                    .find(|op| rest.starts_with(**op))
                    .map_or_else(|| c.to_string(), |op| (*op).to_owned());
                match operator.as_str() {
                    "(" | "[" | "{" => depth.push(c),
                    ")" | "]" | "}" => {
                        depth.pop();
                    }
                    _ => {}
                }
                let start = column;
                for _ in 0..operator.chars().count() {
                    advance(&mut index, &mut column, &mut in_digits);
                }
                tokens.push(Token {
                    kind: Kind::Op,
                    text: operator,
                    column: start,
                });
            }
        }
    }
    if !depth.is_empty() {
        return Err(());
    }
    Ok(tokens)
}

/// `_remove_unused_ops`: join the operands between `/ + , ( )` into names,
/// keeping bracketed filter tokens as they are.
fn remove_unused_ops(tokens: Vec<Token>) -> Vec<Token> {
    let mut output = Vec::new();
    let mut last: Option<Token> = None;
    let mut iter = tokens.into_iter();
    while let Some(token) = iter.next() {
        if token.kind == Kind::Op && token.text == "[" {
            output.extend(last.take());
            output.push(token);
            for inner in iter.by_ref() {
                let end = inner.kind == Kind::Op && inner.text == "]";
                output.push(inner);
                if end {
                    break;
                }
            }
        } else if token.kind == Kind::Op && ["/", "+", ",", "(", ")"].contains(&token.text.as_str())
        {
            output.extend(last.take());
            output.push(token);
        } else if matches!(token.kind, Kind::Name | Kind::Op) {
            match &mut last {
                Some(last) => last.text.push_str(&token.text),
                None => {
                    last = Some(Token {
                        kind: Kind::Name,
                        ..token
                    })
                }
            }
        }
    }
    output.extend(last);
    output
}

/// A parsed `FormatSelector`.
#[derive(Debug, Clone)]
enum Parsed {
    Single(String, Vec<String>),
    PickFirst(Box<Parsed>, Vec<Parsed>, Vec<String>),
    Merge(Box<Parsed>, Vec<Parsed>, Vec<String>),
    Group(Vec<Parsed>, Vec<String>),
}

impl Parsed {
    fn filters(&mut self) -> &mut Vec<String> {
        match self {
            Self::Single(_, filters)
            | Self::PickFirst(_, _, filters)
            | Self::Merge(_, _, filters)
            | Self::Group(_, filters) => filters,
        }
    }
}

struct Parser<'a> {
    spec: &'a str,
    tokens: Vec<Token>,
    position: usize,
}

impl Parser<'_> {
    fn syntax_error(&self, note: &str, column: usize) -> SelectorError {
        SelectorError(format!(
            "Invalid format specification: {note}\n\t{}\n\t{}^",
            self.spec,
            " ".repeat(column)
        ))
    }

    fn next(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.position).cloned();
        if token.is_some() {
            self.position += 1;
        }
        token
    }

    fn restore(&mut self) {
        self.position -= 1;
    }

    /// `_parse_filter`: the tokens up to `]`, joined.
    fn filter(&mut self) -> String {
        let mut text = String::new();
        while let Some(token) = self.next() {
            if token.kind == Kind::Op && token.text == "]" {
                break;
            }
            text.push_str(&token.text);
        }
        text
    }

    /// `_parse_format_selection`.
    fn selection(
        &mut self,
        merge: bool,
        choice: bool,
        group: bool,
    ) -> Result<Vec<Parsed>, SelectorError> {
        let mut selectors = Vec::new();
        let mut current: Option<Parsed> = None;
        while let Some(token) = self.next() {
            if token.kind == Kind::Name {
                current = Some(Parsed::Single(token.text, Vec::new()));
                continue;
            }
            match token.text.as_str() {
                ")" => {
                    if !group {
                        self.restore();
                    }
                    break;
                }
                "/" | "," if merge => {
                    self.restore();
                    break;
                }
                "," if choice => {
                    self.restore();
                    break;
                }
                "," => match current.take() {
                    Some(selector) => selectors.push(selector),
                    None => {
                        return Err(
                            self.syntax_error("\",\" must follow a format selector", token.column)
                        );
                    }
                },
                "/" => {
                    let Some(first) = current.take() else {
                        return Err(
                            self.syntax_error("\"/\" must follow a format selector", token.column)
                        );
                    };
                    let second = self.selection(false, true, false)?;
                    current = Some(Parsed::PickFirst(Box::new(first), second, Vec::new()));
                }
                "[" => {
                    let filter = self.filter();
                    current
                        .get_or_insert_with(|| Parsed::Single("best".into(), Vec::new()))
                        .filters()
                        .push(filter);
                }
                "(" => {
                    if current.is_some() {
                        return Err(self.syntax_error("Unexpected \"(\"", token.column));
                    }
                    let inner = self.selection(false, false, true)?;
                    current = Some(Parsed::Group(inner, Vec::new()));
                }
                "+" => {
                    let Some(first) = current.take() else {
                        return Err(self.syntax_error("Unexpected \"+\"", token.column));
                    };
                    let second = self.selection(true, false, false)?;
                    if second.is_empty() {
                        return Err(self.syntax_error("Expected a selector", token.column));
                    }
                    current = Some(Parsed::Merge(Box::new(first), second, Vec::new()));
                }
                other => {
                    return Err(self.syntax_error(
                        &format!("Operator not recognized: \"{other}\""),
                        token.column,
                    ));
                }
            }
        }
        selectors.extend(current);
        Ok(selectors)
    }
}

/// A compiled `_build_format_filter`.
#[derive(Debug, Clone)]
enum Filter {
    Number {
        key: String,
        op: &'static str,
        value: f64,
        none_inclusive: bool,
    },
    Text {
        key: String,
        op: &'static str,
        negated: bool,
        value: String,
        regex: Option<regex::Regex>,
        none_inclusive: bool,
    },
}

/// Python `type(value).__name__` for a JSON value.
fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "NoneType",
        Value::Bool(_) => "bool",
        Value::Number(number) if number.is_f64() => "float",
        Value::Number(_) => "int",
        Value::String(_) => "str",
        Value::Array(_) => "list",
        Value::Object(_) => "dict",
    }
}

fn number(value: &Value) -> Option<f64> {
    match value {
        Value::Bool(value) => Some(f64::from(u8::from(*value))),
        Value::Number(number) => number.as_f64(),
        _ => None,
    }
}

/// Upstream `parse_filesize` (`lookup_unit_table`, not strict).
fn parse_filesize(text: &str) -> Option<f64> {
    const UNITS: &[(&str, f64)] = &{
        const K: f64 = 1000.0;
        const KI: f64 = 1024.0;
        [
            ("B", 1.0),
            ("b", 1.0),
            ("bytes", 1.0),
            ("KiB", KI),
            ("KB", K),
            ("kB", KI),
            ("Kb", K),
            ("kb", K),
            ("kilobytes", K),
            ("kibibytes", KI),
            ("MiB", KI * KI),
            ("MB", K * K),
            ("mB", KI * KI),
            ("Mb", K * K),
            ("mb", K * K),
            ("megabytes", K * K),
            ("mebibytes", KI * KI),
            ("GiB", KI * KI * KI),
            ("GB", K * K * K),
            ("gB", KI * KI * KI),
            ("Gb", K * K * K),
            ("gb", K * K * K),
            ("gigabytes", K * K * K),
            ("gibibytes", KI * KI * KI),
            ("TiB", KI * KI * KI * KI),
            ("TB", K * K * K * K),
            ("tB", KI * KI * KI * KI),
            ("Tb", K * K * K * K),
            ("tb", K * K * K * K),
            ("terabytes", K * K * K * K),
            ("tebibytes", KI * KI * KI * KI),
            ("PiB", KI * KI * KI * KI * KI),
            ("PB", K * K * K * K * K),
            ("pB", KI * KI * KI * KI * KI),
            ("Pb", K * K * K * K * K),
            ("pb", K * K * K * K * K),
            ("petabytes", K * K * K * K * K),
            ("pebibytes", KI * KI * KI * KI * KI),
            ("EiB", KI * KI * KI * KI * KI * KI),
            ("EB", K * K * K * K * K * K),
            ("eB", KI * KI * KI * KI * KI * KI),
            ("Eb", K * K * K * K * K * K),
            ("eb", K * K * K * K * K * K),
            ("exabytes", K * K * K * K * K * K),
            ("exbibytes", KI * KI * KI * KI * KI * KI),
            ("ZiB", KI * KI * KI * KI * KI * KI * KI),
            ("ZB", K * K * K * K * K * K * K),
            ("zB", KI * KI * KI * KI * KI * KI * KI),
            ("Zb", K * K * K * K * K * K * K),
            ("zb", K * K * K * K * K * K * K),
            ("zettabytes", K * K * K * K * K * K * K),
            ("zebibytes", KI * KI * KI * KI * KI * KI * KI),
            ("YiB", KI * KI * KI * KI * KI * KI * KI * KI),
            ("YB", K * K * K * K * K * K * K * K),
            ("yB", KI * KI * KI * KI * KI * KI * KI * KI),
            ("Yb", K * K * K * K * K * K * K * K),
            ("yb", K * K * K * K * K * K * K * K),
            ("yottabytes", K * K * K * K * K * K * K * K),
            ("yobibytes", KI * KI * KI * KI * KI * KI * KI * KI),
        ]
    };
    static NUMBER: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"^(\d+(?:[,.]\d+)?)\s*").unwrap());
    let found = NUMBER.captures(text)?;
    let rest = &text[found.get(0).unwrap().end()..];
    // `\b` after the unit: the next character is not a word character.
    let (unit, multiplier) = UNITS.iter().find(|(unit, _)| {
        rest.strip_prefix(unit).is_some_and(|after| {
            !after
                .chars()
                .next()
                .is_some_and(|c| c.is_alphanumeric() || c == '_')
        })
    })?;
    let _ = unit;
    let value: f64 = found[1].replace(',', ".").parse().ok()?;
    // Python `round` (half to even) of the product.
    let product = value * multiplier;
    let rounded = product.round();
    Some(
        if (product - product.trunc()).abs() == 0.5 && rounded % 2.0 != 0.0 {
            rounded - product.signum()
        } else {
            rounded
        },
    )
}

fn compile_filter(spec: &str) -> Result<Filter, SelectorError> {
    static NUMERIC: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r"^\s*(?P<key>[\w.-]+)\s*(?P<op><|<=|>|>=|=|!=)(?P<none>\s*\?)?\s*(?P<value>[0-9.]+(?:[kKmMgGtTpPeEzZyY]i?[Bb]?)?)\s*$",
        )
        .unwrap()
    });
    if let Some(found) = NUMERIC.captures(spec) {
        let text = &found["value"];
        let value = match text.parse::<f64>() {
            Ok(value) if !text.is_empty() && text != "." => value,
            _ => parse_filesize(text)
                .or_else(|| parse_filesize(&format!("{text}B")))
                .ok_or_else(|| {
                    SelectorError(format!(
                        "Invalid value {} in format specification {}",
                        crate::batch::py_repr(text),
                        crate::batch::py_repr(spec)
                    ))
                })?,
        };
        let op = match &found["op"] {
            "<" => "<",
            "<=" => "<=",
            ">" => ">",
            ">=" => ">=",
            "=" => "=",
            _ => "!=",
        };
        return Ok(Filter::Number {
            key: found["key"].to_owned(),
            op,
            value,
            none_inclusive: found.name("none").is_some(),
        });
    }
    // The string operator syntax (its quoted value needs a backreference).
    let invalid = || {
        SelectorError(format!(
            "Invalid filter specification {}",
            crate::batch::py_repr(spec)
        ))
    };
    static HEAD: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"^\s*(?P<key>[a-zA-Z0-9._-]+)\s*(?P<neg>!\s*)?(?P<op>=|\^=|\$=|\*=|~=)\s*(?P<none>\?\s*)?")
            .unwrap()
    });
    let head = HEAD.captures(spec).ok_or_else(invalid)?;
    let rest = &spec[head.get(0).unwrap().end()..];
    let (raw, after) = match rest.chars().next() {
        Some(quote @ ('"' | '\'')) => {
            let body = &rest[1..];
            let mut end = None;
            let mut chars = body.char_indices();
            let mut count = 0;
            while let Some((offset, c)) = chars.next() {
                if c == '\\' {
                    if chars.next().is_none() {
                        break;
                    }
                    count += 1;
                } else if c == quote {
                    end = Some(offset);
                    break;
                } else {
                    count += 1;
                }
            }
            let end = end.filter(|_| count > 0).ok_or_else(invalid)?;
            (&body[..end], &body[end + 1..])
        }
        _ => {
            let end = rest
                .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.' || c == '-'))
                .unwrap_or(rest.len());
            if end == 0 {
                return Err(invalid());
            }
            (&rest[..end], &rest[end..])
        }
    };
    if !after.trim_start().is_empty() || after.chars().any(|c| !c.is_whitespace()) {
        return Err(invalid());
    }
    let op = match &head["op"] {
        "=" => "=",
        "^=" => "^=",
        "$=" => "$=",
        "*=" => "*=",
        _ => "~=",
    };
    let (value, regex) = if op == "~=" {
        let regex = regex::Regex::new(raw).map_err(|error| SelectorError(error.to_string()))?;
        (raw.to_owned(), Some(regex))
    } else {
        static ESCAPE: std::sync::LazyLock<regex::Regex> =
            std::sync::LazyLock::new(|| regex::Regex::new(r#"\\([\\"'])"#).unwrap());
        (ESCAPE.replace_all(raw, "$1").into_owned(), None)
    };
    Ok(Filter::Text {
        key: head["key"].to_owned(),
        op,
        negated: head.name("neg").is_some(),
        value,
        regex,
        none_inclusive: head.name("none").is_some(),
    })
}

impl Filter {
    /// `_filter(f)`; `Err` is the Python exception text.
    fn matches(&self, format: &Value) -> Result<bool, SelectorError> {
        let (key, none_inclusive) = match self {
            Self::Number {
                key,
                none_inclusive,
                ..
            }
            | Self::Text {
                key,
                none_inclusive,
                ..
            } => (key, *none_inclusive),
        };
        let actual = match format.get(key) {
            None | Some(Value::Null) => return Ok(none_inclusive),
            Some(actual) => actual,
        };
        match self {
            Self::Number { op, value, .. } => {
                let Some(actual_number) = number(actual) else {
                    return match *op {
                        "=" => Ok(false),
                        "!=" => Ok(true),
                        op => Err(SelectorError(format!(
                            "'{op}' not supported between instances of '{}' and 'float'",
                            type_name(actual)
                        ))),
                    };
                };
                Ok(match *op {
                    "<" => actual_number < *value,
                    "<=" => actual_number <= *value,
                    ">" => actual_number > *value,
                    ">=" => actual_number >= *value,
                    "=" => actual_number == *value,
                    _ => actual_number != *value,
                })
            }
            Self::Text {
                op,
                negated,
                value,
                regex,
                ..
            } => {
                let result = match (*op, actual) {
                    ("=", Value::String(text)) => text == value,
                    ("=", _) => false,
                    ("^=", Value::String(text)) => text.starts_with(value.as_str()),
                    ("$=", Value::String(text)) => text.ends_with(value.as_str()),
                    ("*=", Value::String(text)) => text.contains(value.as_str()),
                    ("*=", Value::Array(items)) => {
                        items.iter().any(|item| item.as_str() == Some(value))
                    }
                    ("*=", Value::Object(map)) => map.contains_key(value),
                    ("~=", Value::String(text)) => {
                        regex.as_ref().is_some_and(|regex| regex.is_match(text))
                    }
                    ("^=", other) => {
                        return Err(SelectorError(format!(
                            "'{}' object has no attribute 'startswith'",
                            type_name(other)
                        )));
                    }
                    ("$=", other) => {
                        return Err(SelectorError(format!(
                            "'{}' object has no attribute 'endswith'",
                            type_name(other)
                        )));
                    }
                    ("*=", other) => {
                        return Err(SelectorError(format!(
                            "argument of type '{}' is not a container or iterable",
                            type_name(other)
                        )));
                    }
                    (_, other) => {
                        return Err(SelectorError(format!(
                            "expected string or bytes-like object, got '{}'",
                            type_name(other)
                        )));
                    }
                };
                Ok(result != *negated)
            }
        }
    }
}

/// A selector node with its compiled filters.
#[derive(Debug, Clone)]
enum Node {
    List(Vec<Node>),
    Group(Box<Node>, Vec<Filter>),
    PickFirst(Vec<Node>, Vec<Filter>),
    Merge(Box<Node>, Box<Node>, Vec<Filter>),
    Single(String, Vec<Filter>),
}

fn build(parsed: &[Parsed]) -> Result<Node, SelectorError> {
    let nodes = parsed
        .iter()
        .map(build_one)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Node::List(nodes))
}

fn build_one(parsed: &Parsed) -> Result<Node, SelectorError> {
    let filters = |filters: &[String]| {
        filters
            .iter()
            .map(|f| compile_filter(f))
            .collect::<Result<Vec<_>, _>>()
    };
    Ok(match parsed {
        Parsed::Group(inner, own) => {
            let inner = build(inner)?;
            Node::Group(Box::new(inner), filters(own)?)
        }
        Parsed::PickFirst(first, second, own) => {
            let first = build_one(first)?;
            let second = build(second)?;
            Node::PickFirst(vec![first, second], filters(own)?)
        }
        Parsed::Merge(first, second, own) => {
            let first = build_one(first)?;
            let second = build(second)?;
            Node::Merge(Box::new(first), Box::new(second), filters(own)?)
        }
        Parsed::Single(name, own) => Node::Single(name.clone(), filters(own)?),
    })
}

/// A compiled format selector.
#[derive(Debug, Clone)]
pub struct FormatSelector {
    root: Node,
}

/// `build_format_selector(format_spec)`.
pub fn parse(spec: &str) -> Result<FormatSelector, SelectorError> {
    let tokens = tokenize(spec).map_err(|_| {
        SelectorError(format!(
            "Invalid format specification: Missing closing/opening brackets or parenthesis\n\t{spec}\n\t{}^",
            " ".repeat(spec.chars().count())
        ))
    })?;
    let mut parser = Parser {
        spec,
        tokens: remove_unused_ops(tokens),
        position: 0,
    };
    let parsed = parser.selection(false, false, false)?;
    Ok(FormatSelector {
        root: build(&parsed)?,
    })
}

/// One selected format, or a merge of several (`requested_formats`).
#[derive(Debug, Clone, PartialEq)]
pub struct Picked {
    /// Indices into the selection's formats.
    pub formats: Vec<usize>,
    /// The `_merge` dict, for merges.
    pub merged: Option<Value>,
}

/// Selection parameters (`allow_multiple_*_streams`, `merge_output_format`,
/// `prefer_free_formats`).
#[derive(Debug, Clone, Default)]
pub struct Options {
    pub merge_output_format: Option<Vec<String>>,
    pub prefer_free: bool,
    pub multiple_video: bool,
    pub multiple_audio: bool,
}

const AUDIO_EXTS: [&str; 9] = [
    "aiff", "alac", "flac", "m4a", "mka", "mp3", "ogg", "opus", "wav",
];
const VIDEO_EXTS: [&str; 7] = ["avi", "flv", "mkv", "mov", "mp4", "webm", "3gp"];

fn codec<'v>(format: &'v Value, key: &str) -> Option<&'v str> {
    format.get(key).and_then(Value::as_str)
}

fn not_none(format: &Value, key: &str) -> bool {
    codec(format, key) != Some("none")
}

struct Evaluator<'a> {
    formats: &'a [Value],
    options: &'a Options,
    has_merged: bool,
    incomplete: bool,
    /// `_check_formats` for `--check-formats` (`selected`): whether a format
    /// works; without one, formats are not tested.
    check: Option<&'a mut dyn FnMut(usize) -> bool>,
}

/// Python truthiness of a JSON value.
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(number) => number.as_f64().is_some_and(|n| n != 0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(map) => !map.is_empty(),
    }
}

/// CPython's `sum()` from 0: exact integers until the first float, then
/// Neumaier-compensated float addition (integers added plainly), with the
/// compensation applied at the end when finite.
fn py_sum(values: impl Iterator<Item = PyNumber>) -> PyNumber {
    let mut total = PyNumber::Int(0);
    let mut compensation = 0.0f64;
    let mut float: Option<f64> = None;
    for value in values {
        match (float, value) {
            (None, PyNumber::Int(_)) => total = add(total, value),
            (None, PyNumber::Float(x)) => {
                float = Some(total.as_f64());
                let f = float.unwrap();
                let t = f + x;
                compensation += if f.abs() >= x.abs() {
                    (f - t) + x
                } else {
                    (x - t) + f
                };
                float = Some(t);
            }
            (Some(f), PyNumber::Float(x)) => {
                let t = f + x;
                compensation += if f.abs() >= x.abs() {
                    (f - t) + x
                } else {
                    (x - t) + f
                };
                float = Some(t);
            }
            (Some(f), PyNumber::Int(n)) => float = Some(f + n as f64),
        }
    }
    match float {
        Some(f) if compensation != 0.0 && compensation.is_finite() => {
            PyNumber::Float(f + compensation)
        }
        Some(f) => PyNumber::Float(f),
        None => total,
    }
}

fn add(a: PyNumber, b: PyNumber) -> PyNumber {
    match (a, b) {
        (PyNumber::Int(a), PyNumber::Int(b)) => a
            .checked_add(b)
            .map_or(PyNumber::Float(a as f64 + b as f64), PyNumber::Int),
        (a, b) => PyNumber::Float(a.as_f64() + b.as_f64()),
    }
}

/// Upstream `determine_protocol`.
fn determine_protocol(format: &Value) -> String {
    if let Some(protocol) = format.get("protocol").and_then(Value::as_str) {
        return protocol.to_owned();
    }
    let url = format.get("url").and_then(Value::as_str).unwrap_or("");
    for scheme in ["rtmp", "mms", "rtsp"] {
        if url.starts_with(scheme) {
            return scheme.into();
        }
    }
    let path = url.split(['?', '#']).next().unwrap_or("");
    match path.rsplit_once('.').map(|(_, ext)| ext) {
        Some("m3u8") => {
            if format.get("is_live").is_some_and(truthy) {
                "m3u8".into()
            } else {
                "m3u8_native".into()
            }
        }
        Some("f4m") => "f4m".into(),
        _ => url::Url::parse(url)
            .map(|url| url.scheme().to_owned())
            .unwrap_or_default(),
    }
}

/// Upstream `format_resolution(format)`.
fn format_resolution(format: &Value) -> Value {
    if codec(format, "vcodec") == Some("none") && not_none(format, "acodec") {
        return "audio only".into();
    }
    if let Some(resolution) = format.get("resolution").filter(|r| !r.is_null()) {
        return resolution.clone();
    }
    let dimension = |key| {
        format
            .get(key)
            .filter(|v| truthy(v))
            .and_then(Value::as_f64)
    };
    match (dimension("width"), dimension("height")) {
        (Some(width), Some(height)) => format!("{}x{}", width as i64, height as i64).into(),
        (None, Some(_)) => format!("{}p", format["height"]).into(),
        (Some(width), None) => format!("{}x?", width as i64).into(),
        (None, None) => "unknown".into(),
    }
}

impl Evaluator<'_> {
    /// `_check_formats(formats)` up to the `wanted`-th working format.
    fn checked(&mut self, candidates: Vec<usize>, wanted: Option<usize>) -> Vec<usize> {
        let Some(check) = self.check.as_mut() else {
            return candidates;
        };
        let mut working = Vec::new();
        for index in candidates {
            if check(index) {
                working.push(index);
                if wanted.is_some_and(|wanted| working.len() >= wanted) {
                    break;
                }
            }
        }
        working
    }

    fn evaluate(&mut self, node: &Node, context: &[usize]) -> Result<Vec<Picked>, SelectorError> {
        let formats = self.formats;
        // `final_selector`: the node's filters narrow the context first.
        let filtered =
            |filters: &[Filter], context: &[usize]| -> Result<Vec<usize>, SelectorError> {
                let mut narrowed = context.to_vec();
                for filter in filters {
                    let mut kept = Vec::new();
                    for index in narrowed {
                        if filter.matches(&formats[index])? {
                            kept.push(index);
                        }
                    }
                    narrowed = kept;
                }
                Ok(narrowed)
            };
        match node {
            Node::List(nodes) => {
                let mut picked = Vec::new();
                for node in nodes {
                    picked.extend(self.evaluate(node, context)?);
                }
                Ok(picked)
            }
            Node::Group(inner, filters) => {
                let context = filtered(filters, context)?;
                self.evaluate(inner, &context)
            }
            Node::PickFirst(choices, filters) => {
                let context = filtered(filters, context)?;
                for choice in choices {
                    let picked = self.evaluate(choice, &context)?;
                    if !picked.is_empty() {
                        return Ok(picked);
                    }
                }
                Ok(Vec::new())
            }
            Node::Merge(first, second, filters) => {
                let context = filtered(filters, context)?;
                let first = self.evaluate(first, &context)?;
                let second = self.evaluate(second, &context)?;
                let mut merged = Vec::new();
                for a in &first {
                    for b in &second {
                        merged.push(self.merge(a, b));
                    }
                }
                Ok(merged)
            }
            Node::Single(name, filters) => {
                let context = filtered(filters, context)?;
                self.single(name, &context)
            }
        }
    }

    fn single(&mut self, name: &str, context: &[usize]) -> Result<Vec<Picked>, SelectorError> {
        let single = |index| Picked {
            formats: vec![index],
            merged: None,
        };
        let formats = self.formats;
        let format = |index: usize| &formats[index];
        if name == "all" {
            let candidates: Vec<usize> = context.iter().rev().copied().collect();
            return Ok(self
                .checked(candidates, None)
                .into_iter()
                .map(single)
                .collect());
        }
        if name == "mergeall" {
            let candidates: Vec<usize> = context
                .iter()
                .copied()
                .filter(|index| {
                    not_none(format(*index), "vcodec") || not_none(format(*index), "acodec")
                })
                .collect();
            let formats = self.checked(candidates, None);
            let Some((last, rest)) = formats.split_last() else {
                return Ok(Vec::new());
            };
            let mut merged = single(*last);
            for index in rest.iter().rev() {
                merged = self.merge(&merged, &single(*index));
            }
            return Ok(vec![merged]);
        }
        static RANKED: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
            regex::Regex::new(r"^(?P<bw>best|worst|b|w)(?P<type>video|audio|v|a)?(?P<mod>\*)?(?:\.(?P<n>[1-9]\d*))?$").unwrap()
        });
        let (mut fallback, mut separate, mut reverse, mut wanted) =
            (false, None::<&str>, true, 1usize);
        type Predicate<'p> = Box<dyn Fn(&Value) -> bool + 'p>;
        let predicate: Predicate<'_> = if let Some(found) = RANKED.captures(name) {
            wanted = found
                .name("n")
                .map_or(1, |n| n.as_str().parse().unwrap_or(usize::MAX));
            reverse = found["bw"].starts_with('b');
            let kind = found.name("type").map(|kind| &kind.as_str()[..1]);
            let modified = found.name("mod").is_some();
            fallback = kind.is_none() && !modified;
            let base: Predicate<'_> = match (kind, modified) {
                (Some(kind), true) => {
                    let key = format!("{kind}codec");
                    Box::new(move |f| not_none(f, &key))
                }
                (Some(kind), false) => {
                    let key = if kind == "v" { "acodec" } else { "vcodec" };
                    Box::new(move |f| codec(f, key) == Some("none"))
                }
                (None, false) => Box::new(|f| not_none(f, "vcodec") && not_none(f, "acodec")),
                (None, true) => Box::new(|_| true),
            };
            Box::new(move |f| base(f) && (not_none(f, "vcodec") || not_none(f, "acodec")))
        } else if AUDIO_EXTS.contains(&name) {
            let name = name.to_owned();
            Box::new(move |f| {
                f.get("ext").and_then(Value::as_str) == Some(name.as_str()) && not_none(f, "acodec")
            })
        } else if VIDEO_EXTS.contains(&name) {
            separate = Some(name);
            let name = name.to_owned();
            Box::new(move |f| {
                f.get("ext").and_then(Value::as_str) == Some(name.as_str())
                    && not_none(f, "acodec")
                    && not_none(f, "vcodec")
            })
        } else if name == "mhtml" {
            Box::new(|f| {
                f.get("ext").and_then(Value::as_str) == Some("mhtml")
                    && codec(f, "acodec") == Some("none")
                    && codec(f, "vcodec") == Some("none")
            })
        } else {
            let name = name.to_owned();
            Box::new(move |f| f.get("format_id").and_then(Value::as_str) == Some(name.as_str()))
        };
        let mut matches: Vec<usize> = context
            .iter()
            .copied()
            .filter(|i| predicate(format(*i)))
            .collect();
        if matches.is_empty() {
            if fallback && self.incomplete {
                matches = context
                    .iter()
                    .copied()
                    .filter(|i| not_none(format(*i), "vcodec") || not_none(format(*i), "acodec"))
                    .collect();
            } else if let Some(ext) = separate.filter(|_| !self.has_merged) {
                matches = context
                    .iter()
                    .copied()
                    .filter(|i| {
                        format(*i).get("ext").and_then(Value::as_str) == Some(ext)
                            && not_none(format(*i), "vcodec")
                    })
                    .collect();
            }
        }
        if reverse {
            matches.reverse();
        }
        let matches = self.checked(matches, Some(wanted));
        Ok(matches
            .get(wanted.saturating_sub(1))
            .copied()
            .map(single)
            .into_iter()
            .collect())
    }

    /// `_merge(formats_pair)`.
    fn merge(&self, a: &Picked, b: &Picked) -> Picked {
        let mut info: Vec<usize> = a.formats.iter().chain(&b.formats).copied().collect();
        let format = |index: usize| &self.formats[index];
        if !self.options.multiple_video || !self.options.multiple_audio {
            let (mut audio_seen, mut video_seen) = (false, false);
            // Python pops while enumerating, skipping the following item.
            let mut position = 0;
            while position < info.len() {
                let f = format(info[position]);
                if codec(f, "acodec") == Some("none") && codec(f, "vcodec") == Some("none") {
                    info.remove(position);
                    position += 1;
                    continue;
                }
                let mut popped = false;
                for (kind, allowed, seen) in [
                    ("acodec", self.options.multiple_audio, &mut audio_seen),
                    ("vcodec", self.options.multiple_video, &mut video_seen),
                ] {
                    if !allowed && not_none(f, kind) {
                        if *seen {
                            popped = true;
                            break;
                        }
                        *seen = true;
                    }
                }
                if popped {
                    info.remove(position);
                }
                position += 1;
            }
        }
        if let [only] = info.as_slice() {
            return Picked {
                formats: vec![*only],
                merged: None,
            };
        }
        let video: Vec<&Value> = info
            .iter()
            .map(|i| format(*i))
            .filter(|f| not_none(f, "vcodec"))
            .collect();
        let audio: Vec<&Value> = info
            .iter()
            .map(|i| format(*i))
            .filter(|f| not_none(f, "acodec"))
            .collect();
        let ext_of = |f: &&Value| {
            f.get("ext")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned()
        };
        let vexts: Vec<String> = video.iter().map(ext_of).collect();
        let aexts: Vec<String> = audio.iter().map(ext_of).collect();
        let preferences: Option<Vec<&str>> = match &self.options.merge_output_format {
            Some(formats) => Some(formats.iter().map(String::as_str).collect()),
            None if self.options.prefer_free => Some(vec!["webm", "mkv"]),
            None => None,
        };
        let ext = crate::codecs::compatible_ext(
            &video.iter().map(|f| codec(f, "vcodec")).collect::<Vec<_>>(),
            &audio.iter().map(|f| codec(f, "acodec")).collect::<Vec<_>>(),
            &vexts.iter().map(String::as_str).collect::<Vec<_>>(),
            &aexts.iter().map(String::as_str).collect::<Vec<_>>(),
            preferences.as_deref(),
        );
        // `filtered(*keys)`: each format's first non-null key, if truthy.
        let filtered = |keys: &[&str]| -> Vec<Value> {
            info.iter()
                .filter_map(|i| {
                    keys.iter()
                        .find_map(|key| format(*i).get(*key).filter(|v| !v.is_null()))
                })
                .filter(|value| truthy(value))
                .cloned()
                .collect()
        };
        let text = |value: &Value| match value {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        };
        let joined = |keys: &[&str], unique: bool| -> Option<String> {
            let mut parts: Vec<String> = filtered(keys).iter().map(text).collect();
            if unique {
                let mut seen = Vec::new();
                parts.retain(|part| {
                    let new = !seen.contains(part);
                    if new {
                        seen.push(part.clone());
                    }
                    new
                });
            }
            Some(parts.join("+"))
        };
        let sum = |keys: &[&str]| {
            py_sum(
                filtered(keys)
                    .iter()
                    .filter_map(|value| serde_json::from_value::<PyNumber>(value.clone()).ok()),
            )
        };
        let mut merged = serde_json::Map::new();
        merged.insert("format".into(), joined(&["format"], false).into());
        merged.insert("format_id".into(), joined(&["format_id"], false).into());
        merged.insert("ext".into(), ext.into());
        merged.insert(
            "protocol".into(),
            info.iter()
                .map(|i| determine_protocol(format(*i)))
                .collect::<Vec<_>>()
                .join("+")
                .into(),
        );
        let or_none = |value: Option<String>| {
            value
                .filter(|v| !v.is_empty())
                .map_or(Value::Null, Value::from)
        };
        merged.insert("language".into(), or_none(joined(&["language"], true)));
        merged.insert(
            "format_note".into(),
            or_none(joined(&["format_note"], true)),
        );
        let size = sum(&["filesize", "filesize_approx"]);
        merged.insert(
            "filesize_approx".into(),
            if size.as_f64() == 0.0 {
                Value::Null
            } else {
                serde_json::to_value(size).unwrap()
            },
        );
        merged.insert(
            "tbr".into(),
            serde_json::to_value(sum(&["tbr", "vbr", "abr"])).unwrap(),
        );
        let get = |f: &Value, key: &str| f.get(key).cloned().unwrap_or(Value::Null);
        if let [video] = video.as_slice() {
            for key in ["width", "height"] {
                merged.insert(key.into(), get(video, key));
            }
            merged.insert("resolution".into(), format_resolution(video));
            for key in [
                "fps",
                "dynamic_range",
                "vcodec",
                "vbr",
                "stretched_ratio",
                "aspect_ratio",
            ] {
                merged.insert(key.into(), get(video, key));
            }
        }
        if let [audio] = audio.as_slice() {
            for key in ["acodec", "abr", "asr", "audio_channels"] {
                merged.insert(key.into(), get(audio, key));
            }
        }
        Picked {
            formats: info,
            merged: Some(Value::Object(merged)),
        }
    }
}

impl FormatSelector {
    /// `format_selector(ctx)` over `formats` (in sorter order, worst first),
    /// with upstream's `has_merged_format` and `incomplete_formats` context.
    pub fn select<'a>(
        &self,
        formats: &'a [Value],
        options: &'a Options,
        check: Option<&'a mut dyn FnMut(usize) -> bool>,
    ) -> Result<Vec<Picked>, SelectorError> {
        let has_video_only =
            |f: &Value| not_none(f, "vcodec") && codec(f, "acodec") == Some("none");
        let has_audio_only =
            |f: &Value| codec(f, "vcodec") == Some("none") && not_none(f, "acodec");
        let mut evaluator = Evaluator {
            formats,
            options,
            has_merged: formats
                .iter()
                .any(|f| codec(f, "acodec") != Some("none") && codec(f, "vcodec") != Some("none")),
            incomplete: formats.iter().all(has_video_only) || formats.iter().all(has_audio_only),
            check,
        };
        let context: Vec<usize> = (0..formats.len()).collect();
        let root = self.root.clone();
        evaluator.evaluate(&root, &context)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn sums_follow_cpython_compensated_float_sum() {
        use super::{PyNumber, py_sum};
        let floats = |values: &[f64]| py_sum(values.iter().map(|v| PyNumber::Float(*v)));
        // `sum([56.115, 69.248, 69.248])`, unlike left-to-right addition.
        assert_eq!(
            floats(&[56.115, 69.248, 69.248]).as_f64(),
            194.61100000000002
        );
        assert_eq!(floats(&[0.1, 0.2, 0.3]).as_f64(), 0.6);
        assert!(matches!(
            py_sum([PyNumber::Int(1), PyNumber::Int(2)].into_iter()),
            PyNumber::Int(3)
        ));
    }

    use super::*;

    fn fixture() -> Value {
        serde_json::from_str(include_str!(
            "../tests/fixtures/selectors/upstream_selection.json"
        ))
        .unwrap()
    }

    /// Every saved spec over every saved format list, against pinned
    /// upstream's selection or error text.
    #[test]
    fn selection_matches_pinned_upstream() {
        let fixture = fixture();
        let mut failures = Vec::new();
        for case in fixture["cases"].as_array().unwrap() {
            let formats = fixture["format_lists"][case["formats"].as_str().unwrap()]
                .as_array()
                .unwrap()
                .clone();
            let spec = case["spec"].as_str().unwrap();
            let actual = parse(spec)
                .and_then(|selector| selector.select(&formats, &Options::default(), None));
            let actual = match actual {
                Ok(picked) => serde_json::json!({"selected": picked.iter().map(|pick| {
                    let mut entry = match &pick.merged {
                        Some(Value::Object(merged)) => merged.clone(),
                        _ => formats[pick.formats[0]].as_object().unwrap().clone(),
                    };
                    let requested: Vec<Value> = if pick.merged.is_some() {
                        pick.formats.iter().map(|i| formats[*i]["format_id"].clone()).collect()
                    } else {
                        Vec::new()
                    };
                    let expected_keys = ["format_id", "ext", "format", "protocol", "language", "format_note", "filesize_approx", "tbr", "width", "height", "vcodec", "acodec", "abr", "vbr"];
                    entry.retain(|key, _| expected_keys.contains(&key.as_str()));
                    entry.insert("requested".into(), requested.into());
                    Value::Object(entry)
                }).collect::<Vec<_>>()}),
                Err(error) => serde_json::json!({"message": error.0}),
            };
            let expected = match case.get("error") {
                Some(_) => serde_json::json!({"message": case["message"]}),
                None => serde_json::json!({"selected": case["selected"]}),
            };
            // Regex dialect messages differ; only their existence is compared.
            let comparable = |value: &Value| value.get("message").is_some();
            if actual != expected
                && !(comparable(&actual) && comparable(&expected) && spec.contains("~="))
            {
                failures.push(format!(
                    "{} {spec:?}\n  expected {expected}\n  actual   {actual}",
                    case["formats"]
                ));
            }
        }
        assert!(
            failures.is_empty(),
            "{} failures:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }
}

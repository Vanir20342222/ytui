//! Upstream `sanitize_filename` (default `is_id`) and forced `sanitize_path`.

use icu_normalizer::ComposingNormalizerBorrowed;
use icu_properties::{
    CodePointMapData,
    props::{GeneralCategory, GeneralCategoryGroup},
};

/// How represented metadata is made safe for one filename component.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FilenameMode {
    /// Upstream default: full-width look-alikes for reserved characters.
    #[default]
    Default,
    /// `--restrict-filenames`: ASCII without spaces or shell punctuation.
    Restricted,
    /// `--no-windows-filenames` outside Windows: only `/` and NUL are replaced.
    Minimal,
}

const ACCENTS: &str = "ÂÃÄÀÁÅÆÇÈÉÊËÌÍÎÏÐÑÒÓÔÕÖŐØŒÙÚÛÜŰÝÞßàáâãäåæçèéêëìíîïðñòóôõöőøœùúûüűýþÿ";
const ACCENT_REPLACEMENTS: [&str; 68] = [
    "A", "A", "A", "A", "A", "A", "AE", "C", "E", "E", "E", "E", "I", "I", "I", "I", "D", "N", "O",
    "O", "O", "O", "O", "O", "O", "OE", "U", "U", "U", "U", "U", "Y", "TH", "ss", "a", "a", "a",
    "a", "a", "a", "ae", "c", "e", "e", "e", "e", "i", "i", "i", "i", "o", "n", "o", "o", "o", "o",
    "o", "o", "o", "oe", "u", "u", "u", "u", "u", "y", "th", "y",
];

fn accent(c: char) -> Option<&'static str> {
    ACCENTS
        .chars()
        .position(|accent| accent == c)
        .map(|index| ACCENT_REPLACEMENTS[index])
}

/// Python `str.isspace`.
pub(crate) fn is_py_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

fn is_control_or_mark(c: char) -> bool {
    let category = CodePointMapData::<GeneralCategory>::new().get(c);
    GeneralCategoryGroup::Other.contains(category) || GeneralCategoryGroup::Mark.contains(category)
}

/// `replace_insane`: `\0` prefixes a substitute character.
fn replace_insane(c: char, restricted: bool, output: &mut String) {
    if restricted && let Some(replacement) = accent(c) {
        output.push_str(replacement);
    } else if !restricted && c == '\n' {
        output.push_str("\0 ");
    } else if !restricted && "\"*:<>?|/\\".contains(c) {
        output.push(match c {
            '/' => '\u{29f8}',
            '\\' => '\u{29f9}',
            c => char::from_u32(c as u32 + 0xfee0).unwrap(),
        });
    } else if c == '?' || (c as u32) < 32 || c as u32 == 127 {
    } else if c == '"' {
        // Only reachable when restricted.
    } else if c == ':' {
        output.push_str("\0_\0-");
    } else if "\\/|*<>".contains(c) {
        output.push_str("\0_");
    } else if restricted && ("!&'()[]{}$;`^,#".contains(c) || is_py_space(c) || c as u32 > 127) {
        if !is_control_or_mark(c) {
            output.push_str("\0_");
        }
    } else {
        output.push(c);
    }
}

/// Replace `:` in `[0-9]+(?::[0-9]+)+` timestamps with `_`.
fn timestamps(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    let mut output = String::with_capacity(value.len());
    let mut index = 0;
    while index < chars.len() {
        if !chars[index].is_ascii_digit() {
            output.push(chars[index]);
            index += 1;
            continue;
        }
        let mut end = index;
        while end < chars.len() && chars[end].is_ascii_digit() {
            end += 1;
        }
        // Extend while `:` is followed by at least one digit.
        let mut matched = end;
        while matched + 1 < chars.len()
            && chars[matched] == ':'
            && chars[matched + 1].is_ascii_digit()
        {
            matched += 1;
            while matched < chars.len() && chars[matched].is_ascii_digit() {
                matched += 1;
            }
        }
        if matched > end {
            output.extend(
                chars[index..matched]
                    .iter()
                    .map(|&c| if c == ':' { '_' } else { c }),
            );
        } else {
            output.extend(&chars[index..end]);
        }
        index = matched;
    }
    output
}

/// Tokens of `(?:\0.|[ _-])`: `\0` pairs, or single strip characters.
fn strip_token(chars: &[char], index: usize) -> Option<(usize, bool)> {
    match chars.get(index)? {
        '\0' if chars.get(index + 1).is_some_and(|&c| c != '\n') => Some((2, true)),
        ' ' | '_' | '-' => Some((1, false)),
        _ => None,
    }
}

/// `re.sub(r'(\0.)(?:(?=\1)..)+', r'\1')` then
/// `re.sub(f'^\0.{STRIP_RE}|{STRIP_RE}\0.$', '')`.
fn collapse_and_strip(value: String) -> String {
    let chars: Vec<char> = value.chars().collect();
    let mut collapsed = Vec::with_capacity(chars.len());
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '\0' && index + 1 < chars.len() && chars[index + 1] != '\n' {
            let pair = [chars[index], chars[index + 1]];
            collapsed.extend(pair);
            index += 2;
            while chars.get(index..index + 2) == Some(&pair[..]) {
                index += 2;
            }
        } else {
            collapsed.push(chars[index]);
            index += 1;
        }
    }
    let chars = collapsed;
    let mut start = 0;
    if let Some((2, true)) = strip_token(&chars, 0) {
        start = 2;
        while let Some((length, _)) = strip_token(&chars, start) {
            start += length;
        }
    }
    // The leftmost suffix that tokenizes completely and ends with a pair.
    let mut end = chars.len();
    for position in start..chars.len() {
        let mut cursor = position;
        let mut last_pair = false;
        while let Some((length, pair)) = strip_token(&chars, cursor) {
            cursor += length;
            last_pair = pair;
        }
        if cursor == chars.len() && last_pair {
            end = position;
            break;
        }
    }
    chars[start..end.max(start)].iter().collect()
}

/// Upstream `sanitize_filename(value, restricted)` with the default `is_id`.
pub fn sanitize_filename(value: &str, restricted: bool) -> String {
    if value.is_empty() {
        return String::new();
    }
    let normalized;
    let value = if restricted {
        normalized = ComposingNormalizerBorrowed::new_nfkc().normalize(value);
        &*normalized
    } else {
        value
    };
    let mut replaced = String::with_capacity(value.len());
    for c in timestamps(value).chars() {
        replace_insane(c, restricted, &mut replaced);
    }
    let result = collapse_and_strip(replaced).replace('\0', "");
    if result.is_empty() {
        "_".into()
    } else {
        result
    }
}

/// Upstream `sanitize_filename(value, restricted, is_id=…)` with an explicit
/// `is_id` (`--compat-options filename-sanitization`): youtube-dl's rules,
/// without full-width look-alikes or substitute-character collapsing; IDs
/// keep their underscores, dots and leading `-`.
pub fn sanitize_filename_legacy(value: &str, restricted: bool, is_id: bool) -> String {
    if value.is_empty() {
        return String::new();
    }
    let normalized;
    let value = if restricted && !is_id {
        normalized = ComposingNormalizerBorrowed::new_nfkc().normalize(value);
        &*normalized
    } else {
        value
    };
    let mut result = String::with_capacity(value.len());
    for c in timestamps(value).chars() {
        if restricted && let Some(replacement) = accent(c) {
            result.push_str(replacement);
        } else if !restricted && c == '\n' {
            result.push(' ');
        } else if c == '?' || (c as u32) < 32 || c as u32 == 127 {
        } else if c == '"' {
            if !restricted {
                result.push('\'');
            }
        } else if c == ':' {
            result.push_str(if restricted { "_-" } else { " -" });
        } else if "\\/|*<>".contains(c) {
            result.push('_');
        } else if restricted && ("!&'()[]{}$;`^,#".contains(c) || is_py_space(c) || c as u32 > 127)
        {
            if !is_control_or_mark(c) {
                result.push('_');
            }
        } else {
            result.push(c);
        }
    }
    let mut result = if result.is_empty() {
        "_".to_owned()
    } else {
        result
    };
    if !is_id {
        while result.contains("__") {
            result = result.replace("__", "_");
        }
        result = result.trim_matches('_').to_owned();
        if restricted && result.starts_with("-_") {
            result.drain(..2);
        }
        if let Some(rest) = result.strip_prefix('-') {
            result = format!("_{rest}");
        }
        result = result.trim_start_matches('.').to_owned();
        if result.is_empty() {
            result = "_".into();
        }
    }
    result
}

/// Upstream `filename_sanitizer(key, value, restricted)`: the default rules,
/// or with `legacy` (`filename-sanitization`) `is_id` from the field name.
pub fn filename_sanitizer(key: &str, value: &str, restricted: bool, legacy: bool) -> String {
    if !legacy {
        return sanitize_filename(value, restricted);
    }
    static ID: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"(^|[_.])id(\.|$)").unwrap());
    sanitize_filename_legacy(value, restricted, ID.is_match(key))
}

/// Upstream non-Windows `sanitize_path(path, force=True)` (`--windows-filenames`):
/// drop empty/`.` parts, fold `..` lexically, and replace reserved characters
/// and a trailing space/dot in each part with `#`.
pub fn force_sanitize_path(path: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|last| last != "..") {
                    parts.pop();
                } else {
                    parts.push("..".into());
                }
            }
            part => {
                let mut chars: Vec<char> = part
                    .chars()
                    .map(|c| if "/<>:\"|\\?*".contains(c) { '#' } else { c })
                    .collect();
                let trailing = |c: char| is_py_space(c) || c == '.';
                // `[\s.]$` also matches before a final newline.
                let length = chars.len();
                if chars.last().is_some_and(|&c| trailing(c)) {
                    if chars[length - 1] == '\n' && length > 1 && trailing(chars[length - 2]) {
                        chars[length - 2] = '#';
                    }
                    chars[length - 1] = '#';
                }
                parts.push(chars.into_iter().collect());
            }
        }
    }
    let root = if path.starts_with('/') { "/" } else { "" };
    let joined = parts.join("/");
    if root.is_empty() && joined.is_empty() {
        ".".into()
    } else {
        format!("{root}{joined}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_upstream_prepare_filename_cases() {
        use crate::output::{TemplateOptions, default_template, render_template_with_options};
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/sanitize/cases.json")).unwrap();
        let mut failures = Vec::new();
        for case in fixture["cases"].as_array().unwrap() {
            let params = &case["params"];
            let restrict = params["restrictfilenames"].as_bool().unwrap_or(false);
            let windows = params["windowsfilenames"].as_bool();
            if cfg!(windows) && windows == Some(false) && !restrict {
                continue; // Upstream minimal sanitization is non-Windows only.
            }
            let mut info = fixture["info"].clone();
            info["title"] = case["title"].clone();
            let info = serde_json::from_value(info).unwrap();
            let options = TemplateOptions {
                filename_mode: TemplateOptions::filename_mode(restrict, windows),
                windows_filenames: windows == Some(true),
                trim_file_name: params["trim_file_name"].as_i64().unwrap_or(0),
                ..Default::default()
            };
            let template = case["output"]
                .as_str()
                .map_or_else(|| default_template(restrict), str::to_owned);
            let actual = render_template_with_options(&template, &info, &options)
                .map(|path| path.to_string_lossy().into_owned());
            if actual.as_deref().ok() != case["expected"].as_str() {
                failures.push(format!(
                    "{}: {actual:?} != {}",
                    case["name"], case["expected"]
                ));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn upstream_default_and_restricted_vectors() {
        for (value, default, restricted) in [
            ("a/b", "a⧸b", "a_b"),
            ("..", "..", ".."),
            ("12:34:56 x: y", "12_34_56 x： y", "12_34_56_x_-_y"),
            ("A? \"b\" <c>|*", "A？ ＂b＂ ＜c＞｜＊", "A_b_c"),
            ("\nx\n", "x", "x"),
            ("Épisode & co!", "Épisode & co!", "Episode_co"),
            ("a - b", "a - b", "a_-_b"),
            ("this: that", "this： that", "this_-_that"),
            (
                "\u{5927}\u{58f0}\u{5e26} - Song",
                "\u{5927}\u{58f0}\u{5e26} - Song",
                "Song",
            ),
            ("xxx/<>\\*|", "xxx⧸＜＞⧹＊｜", "xxx"),
            ("yes? no", "yes？ no", "yes_no"),
            ("-", "-", "-"),
        ] {
            assert_eq!(sanitize_filename(value, false), default, "{value:?}");
            assert_eq!(sanitize_filename(value, true), restricted, "{value:?}");
        }
        assert_eq!(force_sanitize_path("a/./b/../c<d>./e. "), "a/c#d##/e.#");
        assert_eq!(force_sanitize_path("../x"), "../x");
        assert_eq!(force_sanitize_path("/"), "/");
    }
}

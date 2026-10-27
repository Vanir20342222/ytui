//! Upstream regex-based HTML helpers: `unescapeHTML`, OpenGraph property
//! search and `_rta_search`.

use regex::Regex;
use std::{collections::HashMap, sync::LazyLock};

/// Python `re` `\s` for `str` patterns, for use inside a character class.
const SPACE: &str = r"\s\x1c-\x1f";

/// `html.entities.html5` names ending in `;`, overridden by `name2codepoint`
/// (generated from the Python standard library).
static ENTITIES: LazyLock<HashMap<&'static str, String>> = LazyLock::new(|| {
    include_str!("data/html_entities.txt")
        .lines()
        .map(|line| {
            let (name, codepoints) = line
                .split_once('\t')
                .expect("entity lines are tab separated");
            let text = codepoints
                .split(' ')
                .map(|hex| {
                    char::from_u32(u32::from_str_radix(hex, 16).expect("hex codepoint"))
                        .expect("valid codepoint")
                })
                .collect();
            (name, text)
        })
        .collect()
});

/// `_htmlentity_transform` for `&{entity};`.
fn entity(entity: &str) -> String {
    if let Some(text) = ENTITIES.get(entity) {
        return text.clone();
    }
    static NUMERIC: LazyLock<Regex> =
        LazyLock::new(|| Regex::new("^#(x[0-9a-fA-F]+|[0-9]+)").unwrap());
    if let Some(found) = NUMERIC.captures(entity) {
        let number = &found[1];
        let value = match number.strip_prefix('x') {
            Some(hex) => u32::from_str_radix(hex, 16),
            None => number.parse(),
        };
        // Out-of-range values (and, unlike Python, lone surrogates) stay literal.
        if let Some(c) = value.ok().and_then(char::from_u32) {
            return c.into();
        }
    }
    format!("&{entity};")
}

/// Upstream `unescapeHTML`.
pub fn unescape_html(text: &str) -> String {
    static ENTITY: LazyLock<Regex> = LazyLock::new(|| Regex::new("&([^&;]+);").unwrap());
    ENTITY
        .replace_all(text, |found: &regex::Captures| entity(&found[1]))
        .into_owned()
}

/// `_og_regexes(prop)` with the trailing lookaheads expressed without
/// lookaround: the first pattern ends with the content value, and in the
/// second the following `[^>]+?` must start with the lookahead's whitespace.
fn og_regexes(prop: &str) -> [Regex; 2] {
    let prop = regex::escape(prop);
    let sep = "(?:&#x3A;|[:-])";
    let property = format!(
        r#"(?:name|property)=(?:'og{sep}{prop}'|"og{sep}{prop}"|[{SPACE}]*og{sep}{prop}\b)"#
    );
    let unquoted = format!(r#"[{SPACE}]*([^{SPACE}"'=<>`]+?)"#);
    let first = format!(
        r#"(?s)<meta[^>]+?{property}[^>]+?content=(?:"([^"]+?)"|'([^']+?)'|{unquoted}(?:[{SPACE}]|/?>))"#
    );
    let second = format!(
        r#"(?s)<meta[^>]+?content=(?:"([^"]+?)"[^>]+?|'([^']+?)'[^>]+?|[{SPACE}]*([^{SPACE}"'=<>`]+)[{SPACE}][^>]*?){property}"#
    );
    [first, second].map(|pattern| Regex::new(&pattern).expect("OpenGraph patterns are valid"))
}

/// `_og_search_property(prop, html, fatal=False)`.
/// Upstream `clean_html`: tags removed, entities decoded, whitespace
/// collapsed around newlines.
pub fn clean_html(text: &str) -> String {
    // Python's `\s` (for `str`) also matches `\x1c`-`\x1f`.
    static SPACE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"[\s\x1c-\x1f]+").expect("static pattern"));
    static BREAK: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"[\s\x1c-\x1f]?<[\s\x1c-\x1f]?br[\s\x1c-\x1f]?/?[\s\x1c-\x1f]?>[\s\x1c-\x1f]?")
            .expect("static pattern")
    });
    static PARAGRAPH: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"<[\s\x1c-\x1f]?/[\s\x1c-\x1f]?p[\s\x1c-\x1f]?>[\s\x1c-\x1f]?<[\s\x1c-\x1f]?p[^>]*>",
        )
        .expect("static pattern")
    });
    static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<.*?>").expect("static pattern"));
    let text = SPACE.replace_all(text, " ");
    let text = BREAK.replace_all(&text, "\n");
    let text = PARAGRAPH.replace_all(&text, "\n");
    let text = TAG.replace_all(&text, "");
    unescape_html(&text).trim().to_owned()
}

pub fn og_search_property(html: &str, prop: &str) -> Option<String> {
    og_regexes(prop).iter().find_map(|regex| {
        let found = regex.captures(html)?;
        let value = found.iter().skip(1).flatten().next()?;
        Some(unescape_html(value.as_str()))
    })
}

/// Upstream `_html_search_meta(prop, html)` (`_meta_regex`): the first
/// `<meta>` whose `itemprop`/`name`/`property`/`id`/`http-equiv` is `prop`
/// (quoted, or unquoted as a prefix like upstream's empty `\1`) with a later
/// quoted `content`, cleaned like `_html_search_regex`.
pub fn html_search_meta(html: &str, prop: &str) -> Option<String> {
    static CONTENT: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?is)^[^>]+?content=(?:"(.*?)"|'(.*?)')"#).expect("static pattern")
    });
    let lower = html.to_ascii_lowercase();
    let prop_lower = prop.to_ascii_lowercase();
    let mut start = 0;
    while let Some(offset) = lower[start..].find("<meta") {
        let tag_start = start + offset + "<meta".len();
        start = tag_start;
        let tag_end = lower[tag_start..]
            .find('>')
            .map_or(lower.len(), |end| tag_start + end);
        let attributes = &lower[tag_start..tag_end];
        let named = ["itemprop=", "name=", "property=", "id=", "http-equiv="]
            .iter()
            .any(|key| {
                attributes.match_indices(key).any(|(index, _)| {
                    let rest = &attributes[index + key.len()..];
                    [format!("\"{prop_lower}\""), format!("'{prop_lower}'")]
                        .iter()
                        .any(|quoted| rest.starts_with(quoted.as_str()))
                        || rest.starts_with(prop_lower.as_str())
                })
            });
        if !named {
            continue;
        }
        if let Some(found) = CONTENT.captures(&html[tag_start..]) {
            let value = found
                .get(1)
                .or_else(|| found.get(2))
                .map_or("", |value| value.as_str());
            return Some(unescape_html(value).trim().to_owned());
        }
    }
    None
}

/// Upstream `_html_extract_title(html)`: the cleaned `<title>` text.
pub fn html_extract_title(html: &str) -> Option<String> {
    static TITLE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?s)<title\b[^>]*>([^<]+)</title>").unwrap());
    TITLE
        .captures(html)
        .map(|found| clean_html(&found[1]).trim().to_owned())
}

/// `_rta_search(html)`. Ages that do not fit in `u32` are ignored.
pub fn rta_search(html: &str) -> Option<u32> {
    static RTA: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(&format!(
            r#"(?i)<meta[{SPACE}]+name="rating"[{SPACE}]+content="RTA-5042-1996-1400-1577-RTA""#
        ))
        .unwrap()
    });
    static MARKERS: LazyLock<[Regex; 3]> = LazyLock::new(|| {
        [
            r#"Proudly Labeled <a href="http://www\.rtalabel\.org/" title="Restricted to Adults">RTA</a>"#.to_owned(),
            r">[^<]*you acknowledge you are at least ([0-9]+) years old".to_owned(),
            format!(r">[{SPACE}]*(?:18[{SPACE}]+U(?:\.S\.C\.|SC)[{SPACE}]+)?(?:§+[{SPACE}]*)?2257\b"),
        ]
        .map(|pattern| Regex::new(&pattern).unwrap())
    });
    if RTA.is_match(html) {
        return Some(18);
    }
    let mut age_limit = None;
    for marker in MARKERS.iter() {
        if let Some(found) = marker.captures(html) {
            let age = match found.get(1) {
                Some(age) => match age.as_str().parse::<u32>() {
                    Ok(age) => age,
                    Err(_) => continue,
                },
                None => 18,
            };
            age_limit = Some(age_limit.unwrap_or(0).max(age));
        }
    }
    age_limit
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_html_follows_pinned_upstream() {
        // Values from the pinned upstream `clean_html`.
        for (input, expected) in [
            ("Rex Lee\n on  apps", "Rex Lee on apps"),
            ("a<br>b", "a\nb"),
            ("a < br / > b", "a\nb"),
            ("a<BR>b", "ab"),
            ("x</p> <p class=q>y", "x\ny"),
            ("x</P><P>y", "xy"),
            (" t&amp;<b>u</b>\t", "t&u"),
        ] {
            assert_eq!(clean_html(input), expected, "{input:?}");
        }
    }

    #[test]
    fn saved_upstream_html_helper_cases() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/html/cases.json")).unwrap();
        for case in fixture["unescape"].as_array().unwrap() {
            let input = case["input"].as_str().unwrap();
            assert_eq!(
                unescape_html(input),
                case["output"].as_str().unwrap(),
                "{input:?}"
            );
        }
        for case in fixture["og"].as_array().unwrap() {
            let html = case["html"].as_str().unwrap();
            let prop = case["prop"].as_str().unwrap();
            assert_eq!(
                og_search_property(html, prop).as_deref(),
                case["value"].as_str(),
                "{html:?} {prop}"
            );
        }
        for case in fixture["rta"].as_array().unwrap() {
            let html = case["html"].as_str().unwrap();
            assert_eq!(
                rta_search(html).map(u64::from),
                case["age_limit"].as_u64(),
                "{html:?}"
            );
        }
    }
}

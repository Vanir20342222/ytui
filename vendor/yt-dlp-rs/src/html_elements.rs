//! Upstream's `html.parser`-based element helpers (`yt_dlp/utils/_utils.py`):
//! `get_element_text_and_html_by_tag`, `get_elements_text_and_html_by_attribute`
//! and `extract_attributes`, over a tokenizer that follows `HTMLParser`'s
//! start/end tag, comment, declaration and `script`/`style` CDATA handling.
use regex::Regex;

/// Attributes of a start tag: lowercased names and unescaped values (`None`
/// without `=`), in source order.
pub(crate) type Attributes = Vec<(String, Option<String>)>;

/// One `HTMLParser` tag callback and the byte offset just past its source.
#[derive(Debug)]
enum Event {
    Start {
        name: String,
        attrs: Attributes,
        end: usize,
    },
    End {
        name: String,
        end: usize,
    },
}

fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | b'\x0c')
}

/// `tagfind_tolerant`'s name: `[a-zA-Z][^\t\n\r\f />\x00]*`.
fn tag_name(html: &str, start: usize) -> usize {
    let bytes = html.as_bytes();
    let mut end = start + 1;
    while end < bytes.len()
        && !matches!(
            bytes[end],
            b'\t' | b'\n' | b'\r' | b'\x0c' | b' ' | b'/' | b'>' | 0
        )
    {
        end += 1;
    }
    end
}

/// A start tag from `<` at `start`: its name, attributes, whether it is
/// self-closing and the offset after `>` (`None` while incomplete).
fn start_tag(html: &str, start: usize) -> Option<(String, Attributes, bool, usize)> {
    let bytes = html.as_bytes();
    let name_end = tag_name(html, start + 1);
    let name = html[start + 1..name_end].to_ascii_lowercase();
    let mut attrs = Vec::new();
    let mut pos = name_end;
    loop {
        // `(?:\s|/(?!>))*`
        while pos < bytes.len()
            && (is_space(bytes[pos]) || (bytes[pos] == b'/' && bytes.get(pos + 1) != Some(&b'>')))
        {
            pos += 1;
        }
        match bytes.get(pos)? {
            b'>' => return Some((name, attrs, false, pos + 1)),
            b'/' => {
                // `/>`
                return Some((name, attrs, true, pos + 2));
            }
            _ => {}
        }
        // `[^\s/>][^\s/=>]*`
        let attr_start = pos;
        pos += html[pos..].chars().next()?.len_utf8();
        while pos < bytes.len()
            && !is_space(bytes[pos])
            && !matches!(bytes[pos], b'/' | b'=' | b'>')
        {
            pos += 1;
        }
        let attr_name = html[attr_start..pos].to_ascii_lowercase();
        // `(\s*=+\s*(VALUE))?`
        let mut look = pos;
        while look < bytes.len() && is_space(bytes[look]) {
            look += 1;
        }
        if bytes.get(look) == Some(&b'=') {
            while look < bytes.len() && bytes[look] == b'=' {
                look += 1;
            }
            while look < bytes.len() && is_space(bytes[look]) {
                look += 1;
            }
            let value = match bytes.get(look)? {
                quote @ (b'\'' | b'"') => {
                    let close = html[look + 1..].find(*quote as char)? + look + 1;
                    let value = &html[look + 1..close];
                    pos = close + 1;
                    value
                }
                _ => {
                    let value_start = look;
                    while look < bytes.len() && !is_space(bytes[look]) && bytes[look] != b'>' {
                        look += 1;
                    }
                    pos = look;
                    &html[value_start..look]
                }
            };
            attrs.push((attr_name, Some(crate::html::unescape_html(value))));
        } else {
            attrs.push((attr_name, None));
        }
    }
}

/// `HTMLParser.goahead`'s tag callbacks for `html` from its start.
fn events(html: &str) -> Vec<Event> {
    let bytes = html.as_bytes();
    let mut found = Vec::new();
    let mut pos = 0;
    let mut cdata: Option<String> = None;
    while pos < bytes.len() {
        if let Some(element) = cdata.take() {
            // CDATA content ends at `</ELEMENT` (any case).
            let closing = format!("</{element}");
            let lower = html[pos..].to_ascii_lowercase();
            let Some(offset) = lower.find(&closing) else {
                break;
            };
            let Some(close) = html[pos + offset..].find('>') else {
                break;
            };
            pos += offset + close + 1;
            found.push(Event::End {
                name: element,
                end: pos,
            });
            continue;
        }
        let Some(offset) = html[pos..].find('<') else {
            break;
        };
        let at = pos + offset;
        let rest = &html[at..];
        if rest.starts_with("<!--") {
            let Some(close) = html[at + 4..].find("-->") else {
                break;
            };
            pos = at + 4 + close + 3;
        } else if rest.starts_with("</") {
            if bytes.get(at + 2).is_some_and(u8::is_ascii_alphabetic) {
                let name_end = tag_name(html, at + 2);
                let Some(close) = html[name_end..].find('>') else {
                    break;
                };
                pos = name_end + close + 1;
                found.push(Event::End {
                    name: html[at + 2..name_end].to_ascii_lowercase(),
                    end: pos,
                });
            } else {
                let Some(close) = html[at + 2..].find('>') else {
                    break;
                };
                pos = at + 2 + close + 1;
            }
        } else if rest.starts_with("<!") || rest.starts_with("<?") {
            let Some(close) = html[at + 2..].find('>') else {
                break;
            };
            pos = at + 2 + close + 1;
        } else if bytes.get(at + 1).is_some_and(u8::is_ascii_alphabetic) {
            let Some((name, attrs, self_closing, end)) = start_tag(html, at) else {
                break;
            };
            pos = end;
            if self_closing {
                // `handle_startendtag`: the start, then the end.
                found.push(Event::Start {
                    name: name.clone(),
                    attrs,
                    end,
                });
                found.push(Event::End { name, end });
            } else {
                if matches!(name.as_str(), "script" | "style") {
                    cdata = Some(name.clone());
                }
                found.push(Event::Start { name, attrs, end });
            }
        } else {
            pos = at + 1;
        }
    }
    found
}

/// Upstream `get_element_text_and_html_by_tag(tag, html)`: the first
/// `<tag` element's content and whole source, or `compat_HTMLParseError`'s
/// message.
pub(crate) fn element_text_and_html_by_tag(
    tag: &str,
    html: &str,
) -> Result<(String, String), String> {
    let closing = format!("</{tag}>");
    let whole_start = html
        .find(&format!("<{tag}"))
        .ok_or_else(|| format!("opening {tag} tag not found"))?;
    let content_start = html[whole_start..]
        .find('>')
        .ok_or_else(|| format!("malformed opening {tag} tag"))?
        + whole_start
        + 1;
    let opening = events(&html[whole_start..content_start]);
    match opening.first() {
        Some(Event::Start { name, .. }) if name == tag && opening.len() == 1 => {}
        Some(Event::Start { name, .. }) if name == tag => {
            // A self-closing first tag raises `HTMLBreakOnClosingTagException`
            // outside upstream's handler.
            return Err("unsupported self-closing element".into());
        }
        _ => return Err(format!("parser did not match opening {tag} tag")),
    }
    let all = events(&html[whole_start..]);
    let mut stack = vec![tag.to_owned()];
    let mut pending = all.into_iter().skip(1).peekable();
    let mut offset = content_start;
    while offset < html.len() {
        let next = html[offset..]
            .find(&closing)
            .ok_or_else(|| format!("closing {tag} tag not found"))?;
        let chunk_end = offset + next + closing.len();
        while let Some(event) = pending.next_if(|event| {
            let end = match event {
                Event::Start { end, .. } | Event::End { end, .. } => *end,
            };
            whole_start + end <= chunk_end
        }) {
            match event {
                Event::Start { name, .. } => stack.push(name),
                Event::End { name, .. } => {
                    if stack.is_empty() {
                        return Err("no tags in the stack".into());
                    }
                    let mut matched = false;
                    while let Some(inner) = stack.pop() {
                        if inner == name {
                            matched = true;
                            break;
                        }
                    }
                    if !matched {
                        return Err(format!(
                            "matching opening tag for closing {name} tag not found"
                        ));
                    }
                    if stack.is_empty() {
                        return Ok((
                            html[content_start..offset + next].to_owned(),
                            html[whole_start..chunk_end].to_owned(),
                        ));
                    }
                }
            }
        }
        offset = chunk_end;
    }
    Err("unexpected end of html".into())
}

/// Upstream `get_elements_text_and_html_by_attribute(attribute, value, html)`
/// (default `tag`, escaped value): each matching element's unquoted,
/// unescaped content and whole source.
pub(crate) fn elements_text_and_html_by_attribute(
    attribute: &str,
    value: &str,
    html: &str,
) -> Result<Vec<(String, String)>, String> {
    if value.is_empty() {
        return Ok(Vec::new());
    }
    let escaped = regex::escape(value);
    // `(?P<_q>['"]{quote})VALUE(?P=_q)` as one alternative per quote.
    let unquoted = !value
        .chars()
        .next()
        .is_some_and(|first| first.is_whitespace() || "\"'`=<>".contains(first));
    let mut values = vec![format!("\"{escaped}\""), format!("'{escaped}'")];
    if unquoted {
        values.push(escaped);
    }
    elements_matching(attribute, &values, html)
}

/// Upstream `get_elements_text_and_html_by_class(class_name, html)`: the
/// elements whose `class` contains the name as a whole word.
pub(crate) fn elements_text_and_html_by_class(
    class: &str,
    html: &str,
) -> Result<Vec<(String, String)>, String> {
    let escaped = regex::escape(class);
    let values = [
        format!(r#""(?:[^"]*\s)?{escaped}(?:\s[^"]*)?""#),
        format!(r"'(?:[^']*\s)?{escaped}(?:\s[^']*)?'"),
    ];
    elements_matching("class", &values, html)
}

fn elements_matching(
    attribute: &str,
    values: &[String],
    html: &str,
) -> Result<Vec<(String, String)>, String> {
    let pattern = format!(
        r#"<(?P<tag>[\w:.-]+)(?:\s(?:[^>"']|"[^"]*"|'[^']*')*)?\s{}\s*=\s*(?:{})"#,
        regex::escape(attribute),
        values.join("|")
    );
    let element = Regex::new(&pattern).map_err(|error| error.to_string())?;
    let quoted = Regex::new(r#"(?s)^(?:"(.*)"|'(.*)')$"#).unwrap();
    let mut found = Vec::new();
    for matched in element.captures_iter(html) {
        let start = matched.get(0).map_or(0, |whole| whole.start());
        let (content, whole) = element_text_and_html_by_tag(&matched["tag"], &html[start..])?;
        let content = match quoted.captures(&content) {
            Some(inner) => inner
                .get(1)
                .or_else(|| inner.get(2))
                .map_or(content.clone(), |inner| inner.as_str().to_owned()),
            None => content,
        };
        found.push((crate::html::unescape_html(&content), whole));
    }
    Ok(found)
}

/// Upstream `get_element_html_by_id(id, html)`.
pub(crate) fn element_html_by_id(id: &str, html: &str) -> Result<Option<String>, String> {
    Ok(elements_text_and_html_by_attribute("id", id, html)?
        .into_iter()
        .next()
        .map(|(_, whole)| whole))
}

/// Upstream `extract_attributes(html_element)`: the first start tag's
/// attributes (later duplicates win, like `dict(attrs)`).
pub(crate) fn extract_attributes(element: &str) -> serde_json::Map<String, serde_json::Value> {
    let mut attributes = serde_json::Map::new();
    if let Some(Event::Start { attrs, .. }) = events(element)
        .into_iter()
        .find(|event| matches!(event, Event::Start { .. }))
    {
        for (name, value) in attrs {
            attributes.insert(name, value.map_or(serde_json::Value::Null, Into::into));
        }
    }
    attributes
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn attributes_follow_pinned_upstream() {
        // Values from the pinned upstream `extract_attributes`.
        assert_eq!(
            serde_json::Value::Object(extract_attributes(
                r#"<div id="videobox" data-id="a&amp;b" Data-X=1 flag class='c d'>"#
            )),
            json!({"id": "videobox", "data-id": "a&b", "data-x": "1", "flag": null, "class": "c d"})
        );
        assert_eq!(
            serde_json::Value::Object(extract_attributes(r#"<e a="1" a="2" b=x&lt;y c = "sp">"#)),
            json!({"a": "2", "b": "x<y", "c": "sp"})
        );
        assert_eq!(
            serde_json::Value::Object(extract_attributes(r#"<x data-v="&#39;q&#39;" e="">"#)),
            json!({"data-v": "'q'", "e": ""})
        );
    }

    #[test]
    fn elements_by_id_follow_pinned_upstream() {
        // Values from the pinned upstream `get_element_html_by_id`.
        for (html, expected) in [
            (
                r#"<p><div class="a" id="videobox" data-id="z">in<div>x</div></div>"#,
                Ok(Some(
                    r#"<div class="a" id="videobox" data-id="z">in<div>x</div></div>"#,
                )),
            ),
            (
                r#"<p><div id=videoboxes data-id="z">in</div>"#,
                Ok(Some(r#"<div id=videoboxes data-id="z">in</div>"#)),
            ),
            ("<p>none</p>", Ok(None)),
            (
                r#"<div id="videobox">x<span>y</div>"#,
                Ok(Some(r#"<div id="videobox">x<span>y</div>"#)),
            ),
            (
                r#"<div id="videobox"><br>x</div>"#,
                Ok(Some(r#"<div id="videobox"><br>x</div>"#)),
            ),
            (
                r#"<div id="videobox"><script>"</div>"</script></div>z</div>"#,
                Ok(Some(
                    r#"<div id="videobox"><script>"</div>"</script></div>"#,
                )),
            ),
            (
                r#"<div id="videobox"><img/></div>"#,
                Ok(Some(r#"<div id="videobox"><img/></div>"#)),
            ),
            (
                r#"<p><div class="a" id="videobox" data-id="z">in<div>x</div></div><div id=videobox>"#,
                Err("unexpected end of html"),
            ),
        ] {
            assert_eq!(
                element_html_by_id("videobox", html),
                expected
                    .map(|found| found.map(str::to_owned))
                    .map_err(str::to_owned),
                "{html}"
            );
        }
    }
}

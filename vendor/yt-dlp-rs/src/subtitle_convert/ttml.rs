//! Bounded native port of the pinned dfxp2srt text/timing/style converter.
use crate::{Error, Result};
use roxmltree::{Document, Node, ParsingOptions};
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

const TTML: &str = "http://www.w3.org/ns/ttml";
const STYLING: &str = "http://www.w3.org/ns/ttml#styling";
const XML: &str = "http://www.w3.org/XML/1998/namespace";
const PROPERTIES: [&str; 6] = [
    "color",
    "fontFamily",
    "fontSize",
    "fontStyle",
    "fontWeight",
    "textDecoration",
];
const MAX_INPUT: usize = 8 * 1024 * 1024;
const MAX_OUTPUT: usize = 16 * 1024 * 1024;
type Style = BTreeMap<&'static str, Arc<str>>;
fn error(message: &str) -> Error {
    Error::Subtitle(format!("TTML conversion: {message}"))
}
fn append(output: &mut String, value: &str) -> Result<()> {
    if output.len().saturating_add(value.len()) > MAX_OUTPUT {
        return Err(error("output exceeds 16 MiB"));
    }
    output.push_str(value);
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Encoding {
    Utf8,
    Le,
    Be,
}
fn decode(input: &[u8]) -> Result<String> {
    if input.len() > MAX_INPUT {
        return Err(error("input exceeds 8 MiB"));
    }
    let (encoding, bytes) = if let Some(bytes) = input.strip_prefix(&[0xff, 0xfe]) {
        (Encoding::Le, bytes)
    } else if let Some(bytes) = input.strip_prefix(&[0xfe, 0xff]) {
        (Encoding::Be, bytes)
    } else if input.starts_with(&[b'<', 0]) {
        (Encoding::Le, input)
    } else if input.starts_with(&[0, b'<']) {
        (Encoding::Be, input)
    } else {
        (
            Encoding::Utf8,
            input.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(input),
        )
    };
    let mut text = match encoding {
        Encoding::Utf8 => std::str::from_utf8(bytes)
            .map_err(|_| error("invalid UTF-8"))?
            .to_owned(),
        Encoding::Le | Encoding::Be => {
            if bytes.len() % 2 != 0 {
                return Err(error("odd UTF-16 byte count"));
            }
            let units: Vec<_> = bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| {
                    if encoding == Encoding::Le {
                        u16::from_le_bytes([pair[0], pair[1]])
                    } else {
                        u16::from_be_bytes([pair[0], pair[1]])
                    }
                })
                .collect();
            String::from_utf16(&units).map_err(|_| error("invalid UTF-16"))?
        }
    };
    if text.len() > MAX_OUTPUT {
        return Err(error("decoded XML exceeds 16 MiB"));
    }
    // Exact upstream compatibility fix for UTF-8 mislabeled with this spelling.
    if encoding == Encoding::Utf8 {
        text = text.replace("encoding='UTF-16'", "encoding='UTF-8'");
    }
    if text.starts_with("<?xml")
        && let Some(end) = text.find("?>")
    {
        let declaration = &text[..end];
        if let Some(start) = declaration.find("encoding") {
            let value = declaration[start + 8..]
                .trim_start()
                .strip_prefix('=')
                .map(str::trim_start)
                .ok_or_else(|| error("invalid encoding declaration"))?;
            let quote = value
                .chars()
                .next()
                .filter(|quote| matches!(quote, '\'' | '"'))
                .ok_or_else(|| error("invalid encoding declaration"))?;
            let end_value = value[1..]
                .find(quote)
                .ok_or_else(|| error("invalid encoding declaration"))?
                + 1;
            let declared = value[1..end_value].to_ascii_lowercase();
            let supported = match encoding {
                Encoding::Utf8 => {
                    matches!(declared.as_str(), "utf-8" | "utf8")
                        || (matches!(declared.as_str(), "ascii" | "us-ascii") && text.is_ascii())
                }
                Encoding::Le => matches!(declared.as_str(), "utf-16" | "utf-16le"),
                Encoding::Be => matches!(declared.as_str(), "utf-16" | "utf-16be"),
            };
            if !supported {
                return Err(error("unsupported or inconsistent XML encoding"));
            }
            // roxmltree receives already decoded UTF-8; keep its declaration true.
            let value_offset = end - value.len() + 1;
            text.replace_range(value_offset..value_offset + end_value - 1, "UTF-8");
        }
    }
    for old in [
        "http://www.w3.org/2004/11/ttaf1",
        "http://www.w3.org/2006/04/ttaf1",
        "http://www.w3.org/2006/10/ttaf1",
    ] {
        text = text.replace(old, TTML);
    }
    Ok(text.replace("http://www.w3.org/ns/ttml#style", STYLING))
}

fn decimal(value: &str) -> Option<f64> {
    let mut pieces = value.split('.');
    let integer = pieces.next()?;
    if integer.is_empty() || !integer.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    if let Some(fraction) = pieces.next()
        && (fraction.is_empty() || !fraction.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return None;
    }
    if pieces.next().is_some() {
        return None;
    }
    value.parse().ok()
}
fn time(value: Option<&str>) -> Result<Option<f64>> {
    let Some(value) = value.filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let seconds = if let Some(seconds) = decimal(value.strip_suffix('s').unwrap_or(value)) {
        Some(seconds)
    } else {
        let parts: Vec<_> = value.split(':').collect();
        if !(parts.len() == 3 || parts.len() == 4) {
            return Ok(None);
        }
        if parts[0].is_empty()
            || !parts[0].bytes().all(|byte| byte.is_ascii_digit())
            || parts[1].len() != 2
            || !parts[1].bytes().all(|byte| byte.is_ascii_digit())
        {
            return Ok(None);
        }
        let second = if parts.len() == 4 {
            if parts[2].len() != 2
                || !parts[2].bytes().all(|byte| byte.is_ascii_digit())
                || parts[3].is_empty()
                || !parts[3].bytes().all(|byte| byte.is_ascii_digit())
            {
                return Ok(None);
            }
            format!("{}.{}", parts[2], parts[3])
        } else {
            parts[2].to_owned()
        };
        if second
            .split('.')
            .next()
            .is_none_or(|whole| whole.len() != 2)
        {
            return Ok(None);
        }
        match (decimal(parts[0]), decimal(parts[1]), decimal(&second)) {
            (Some(hours), Some(minutes), Some(seconds)) => {
                Some(hours * 3600.0 + minutes * 60.0 + seconds)
            }
            _ => None,
        }
    };
    if seconds
        .is_some_and(|seconds| !seconds.is_finite() || seconds * 1000.0 > (1_u64 << 53) as f64)
    {
        return Err(error("timestamp exceeds finite exact-millisecond range"));
    }
    Ok(seconds)
}
fn timecode(seconds: f64) -> Result<String> {
    if !seconds.is_finite() || seconds < 0.0 || seconds * 1000.0 > (1_u64 << 53) as f64 {
        return Err(error("timestamp exceeds finite exact-millisecond range"));
    }
    let milliseconds = (seconds * 1000.0).floor() as u64;
    Ok(format!(
        "{:02}:{:02}:{:02},{:03}",
        milliseconds / 3_600_000,
        milliseconds / 60_000 % 60,
        milliseconds / 1000 % 60,
        milliseconds % 1000
    ))
}
fn is(node: Node<'_, '_>, tag: &str, namespace: Option<&str>) -> bool {
    node.is_element() && node.tag_name().name() == tag && node.tag_name().namespace() == namespace
}
fn properties(node: Node<'_, '_>, style: &mut Style) {
    for property in PROPERTIES {
        if let Some(value) = node
            .attribute((STYLING, property))
            .filter(|value| !value.is_empty())
        {
            style.insert(property, Arc::from(value));
        }
    }
}
fn styles(root: Node<'_, '_>) -> Result<HashMap<String, Style>> {
    // Cache property strings once, so forward inheritance passes cannot copy a
    // large font property into a fresh allocation on every resolution attempt.
    let definitions: Vec<_> = root
        .descendants()
        .skip(1)
        .filter(|node| is(*node, "style", Some(TTML)))
        .filter_map(|node| {
            let id = node
                .attribute("id")
                .filter(|id| !id.is_empty())
                .or_else(|| node.attribute((XML, "id")))
                .filter(|id| !id.is_empty())?;
            let parent = node.attribute("style").filter(|parent| !parent.is_empty());
            let mut own = Style::new();
            properties(node, &mut own);
            Some((id, parent, own))
        })
        .collect();
    let mut styles: HashMap<String, Style> = HashMap::new();
    let mut work = 0;
    for _ in 0..=definitions.len() {
        let mut repeat = false;
        for (id, parent, own) in &definitions {
            work += 1;
            if work > 1_000_000 {
                return Err(error("style resolution exceeds 1000000 steps"));
            }
            if let Some(parent) = parent {
                let Some(parent) = styles.get(*parent).cloned() else {
                    repeat = true;
                    continue;
                };
                styles.insert((*id).into(), parent);
            }
            if !own.is_empty() {
                styles.entry((*id).into()).or_default().extend(own.clone());
            }
        }
        if !repeat {
            return Ok(styles);
        }
    }
    Err(error("unresolved or cyclic style inheritance"))
}
struct Renderer<'a> {
    styles: &'a HashMap<String, Style>,
    default: Style,
    applied: Vec<Style>,
}
impl Renderer<'_> {
    fn node(&mut self, node: Node<'_, '_>, output: &mut String) -> Result<()> {
        if node.is_text() {
            return append(output, node.text().unwrap_or(""));
        }
        if !node.is_element() {
            return Ok(());
        }
        if is(node, "br", Some(TTML)) || is(node, "br", None) {
            append(output, "\n")?;
            for child in node.children() {
                self.node(child, output)?;
            }
            return Ok(());
        }
        let mut style = self.default.clone();
        if let Some(referenced) = node.attribute("style").and_then(|id| self.styles.get(id)) {
            style.extend(referenced.clone());
        }
        properties(node, &mut style);
        let mut opened = Vec::new();
        let mut font = String::new();
        if !style.is_empty() {
            for (property, value) in &style {
                if self.applied.last().and_then(|style| style.get(property)) == Some(value) {
                    continue;
                }
                match *property {
                    "color" => {
                        append(&mut font, " color=\"")?;
                        append(&mut font, value)?;
                        append(&mut font, "\"")?;
                    }
                    "fontFamily" => {
                        append(&mut font, " face=\"")?;
                        append(&mut font, value)?;
                        append(&mut font, "\"")?;
                    }
                    "fontSize" => {
                        append(&mut font, " size=\"")?;
                        append(&mut font, value)?;
                        append(&mut font, "\"")?;
                    }
                    "fontWeight" if value.as_ref() == "bold" => {
                        append(output, "<b>")?;
                        opened.push("b");
                    }
                    "fontStyle" if value.as_ref() == "italic" => {
                        append(output, "<i>")?;
                        opened.push("i");
                    }
                    "textDecoration" if value.as_ref() == "underline" => {
                        append(output, "<u>")?;
                        opened.push("u");
                    }
                    _ => {}
                }
            }
            if !font.is_empty() {
                append(output, "<font")?;
                append(output, &font)?;
                append(output, ">")?;
                opened.push("font");
            }
            let mut applied = self.applied.last().cloned().unwrap_or_default();
            applied.extend(style);
            self.applied.push(applied);
        }
        for child in node.children() {
            self.node(child, output)?;
        }
        for tag in opened.iter().rev() {
            append(output, "</")?;
            append(output, tag)?;
            append(output, ">")?;
        }
        // Preserve the pinned parser's observable state, including suppression
        // carried across cues when a style opens no new markup tags.
        if !opened.is_empty() {
            self.applied.pop();
        }
        Ok(())
    }
}

pub(crate) fn to_srt(input: &[u8]) -> Result<String> {
    let text = decode(input)?;
    let document = Document::parse_with_options(
        &text,
        ParsingOptions {
            allow_dtd: false,
            nodes_limit: 100_000,
            ..ParsingOptions::default()
        },
    )
    .map_err(|_| error("invalid XML or unsupported DTD/entity"))?;
    let root = document.root_element();
    if document
        .descendants()
        .filter(|node| node.is_element())
        .any(|node| {
            node.ancestors()
                .filter(|ancestor| ancestor.is_element())
                .take(65)
                .count()
                > 64
        })
    {
        return Err(error("XML nesting exceeds 64 levels"));
    }
    let mut paragraphs: Vec<_> = root
        .descendants()
        .skip(1)
        .filter(|node| is(*node, "p", Some(TTML)))
        .collect();
    if paragraphs.is_empty() {
        paragraphs = root
            .descendants()
            .skip(1)
            .filter(|node| is(*node, "p", None))
            .collect();
    }
    if paragraphs.is_empty() {
        return Err(error("document contains no TTML paragraphs"));
    }
    if paragraphs.len() > 10_000 {
        return Err(error("paragraph count exceeds 10000"));
    }
    let styles = styles(root)?;
    let mut default = Style::new();
    for tag in ["body", "div"] {
        let node = root
            .descendants()
            .skip(1)
            .find(|node| is(*node, tag, Some(TTML)))
            .or_else(|| root.descendants().skip(1).find(|node| is(*node, tag, None)));
        if let Some(style) = node
            .and_then(|node| node.attribute("style"))
            .and_then(|id| styles.get(id))
        {
            default.extend(style.clone());
        }
    }
    let mut renderer = Renderer {
        styles: &styles,
        default,
        applied: Vec::new(),
    };
    let mut output = String::new();
    for (index, paragraph) in paragraphs.into_iter().enumerate() {
        let begin = time(paragraph.attribute("begin"))?;
        let end = time(paragraph.attribute("end"))?;
        let duration = time(paragraph.attribute("dur"))?;
        let Some(begin) = begin else { continue };
        let end = match end.filter(|end| *end != 0.0) {
            Some(end) => end,
            None => match duration.filter(|duration| *duration != 0.0) {
                Some(duration) => begin + duration,
                None => continue,
            },
        };
        let mut cue = String::new();
        renderer.node(paragraph, &mut cue)?;
        append(
            &mut output,
            &format!(
                "{}\n{} --> {}\n",
                index + 1,
                timecode(begin)?,
                timecode(end)?
            ),
        )?;
        append(&mut output, cue.trim())?;
        append(&mut output, "\n\n")?;
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saved_upstream_and_native_reference_ttml_vectors_match_exactly() {
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/subtitle_conversion");
        let fixture: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(directory.join("cases.json")).unwrap())
                .unwrap();
        let mut failures = Vec::new();
        for case in fixture["ttml_cases"].as_array().unwrap() {
            let input =
                std::fs::read(directory.join(case["input_file"].as_str().unwrap())).unwrap();
            match to_srt(&input) {
                Ok(actual) if actual == case["expected_srt"].as_str().unwrap() => {}
                actual => failures.push(format!(
                    "{}: {actual:?}, expected {}",
                    case["name"], case["expected_srt"]
                )),
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
    #[test]
    fn timing_grammar_precision_and_skips_preserve_source_indices() {
        for (expression, expected) in [
            ("0.1", Some(0.1)),
            ("0.1s", Some(0.1)),
            ("00:00:01", Some(1.0)),
            ("00:00:01.100", Some(1.1)),
            ("00:00:01:100", Some(1.1)),
            ("1:99:99", Some(9639.0)),
            (".5", None),
            ("-1", None),
            ("2h", None),
            (" 1", None),
            ("1.", None),
            ("1:2:03", None),
            ("1:02:3", None),
            ("1:02:03s", None),
        ] {
            assert_eq!(time(Some(expression)).unwrap(), expected, "{expression}");
        }
        let output=to_srt(b"<tt><p end='1'>skip</p><p begin='0' end='0'>skip</p><p begin='2' end='0' dur='1'>third</p><p begin='3' end='1'>reverse</p></tt>").unwrap();
        assert_eq!(
            output,
            "3\n00:00:02,000 --> 00:00:03,000\nthird\n\n4\n00:00:03,000 --> 00:00:01,000\nreverse\n\n"
        );
        assert!(time(Some(&"9".repeat(400))).is_err());
        assert!(timecode(f64::NAN).is_err());
        assert!(timecode(f64::INFINITY).is_err());
    }
    #[test]
    fn encoding_boms_declarations_surrogates_and_wrong_labels_are_explicit() {
        let xml = "<?xml version=\"1.0\" encoding=\"UTF-16\"?><tt><p begin=\"0\" end=\"1\">影像 𝐀</p></tt>";
        let expected = "1\n00:00:00,000 --> 00:00:01,000\n影像 𝐀\n\n";
        for big in [false, true] {
            let encoded: Vec<_> = xml
                .encode_utf16()
                .flat_map(|unit| {
                    if big {
                        unit.to_be_bytes()
                    } else {
                        unit.to_le_bytes()
                    }
                })
                .collect();
            assert_eq!(to_srt(&encoded).unwrap(), expected);
            let mut with_bom = if big {
                vec![0xfe, 0xff]
            } else {
                vec![0xff, 0xfe]
            };
            with_bom.extend(encoded);
            assert_eq!(to_srt(&with_bom).unwrap(), expected);
        }
        let mut utf8 = vec![0xef, 0xbb, 0xbf];
        utf8.extend_from_slice(b"<tt><p begin='0' end='1'>line</p></tt>");
        assert!(to_srt(&utf8).is_ok());
        assert!(to_srt(xml.as_bytes()).is_err());
        assert!(
            to_srt(
                b"<?xml version='1.0' encoding='UTF-16'?><tt><p begin='0' end='1'>line</p></tt>"
            )
            .is_ok()
        );
        for bytes in [
            vec![0xff],
            vec![0xff, 0xfe, b'<'],
            vec![0xff, 0xfe, 0, 0xd8],
            b"<?xml version='1.0' encoding='ISO-8859-1'?><tt/>".to_vec(),
        ] {
            assert!(to_srt(&bytes).is_err());
        }
    }
    #[test]
    fn namespace_priority_styles_inherit_forward_and_unknown_properties_are_ignored() {
        let xml=b"<tt xmlns:t='http://www.w3.org/ns/ttml' xmlns:s='http://www.w3.org/ns/ttml#styling'><t:head><t:style id='child' style='base' s:fontWeight='bold'/><t:style xml:id='base' s:color='red'/></t:head><body><p begin='0' end='1'>unnamespaced ignored</p><t:p begin='1' end='2' style='child' s:fontStyle='italic' s:textDecoration='underline' s:backgroundColor='blue'>text<t:br/>line</t:p></body></tt>";
        assert_eq!(
            to_srt(xml).unwrap(),
            "1\n00:00:01,000 --> 00:00:02,000\n<i><b><u><font color=\"red\">text\nline</font></u></b></i>\n\n"
        );
    }
    #[test]
    fn invalid_xml_dtd_entities_and_unresolved_style_cycles_fail_boundedly() {
        for xml in [
            "<tt>",
            "<tt/>",
            "<tt><p begin='0' end='1'>&unknown;</p></tt>",
            "<!DOCTYPE tt [<!ENTITY x 'text'>]><tt><p begin='0' end='1'>&x;</p></tt>",
            "<!DOCTYPE tt SYSTEM 'https://example.test/remote'><tt><p begin='0' end='1'>text</p></tt>",
            "<tt xmlns='http://www.w3.org/ns/ttml'><style id='a' style='b'/><style id='b' style='a'/><p begin='0' end='1'>text</p></tt>",
            "<tt xmlns='http://www.w3.org/ns/ttml'><style id='a' style='missing'/><p begin='0' end='1'>text</p></tt>",
        ] {
            assert!(to_srt(xml.as_bytes()).is_err(), "{xml}");
        }
        assert_eq!(
            to_srt(b"<tt><p begin='invalid' end='1'>text</p></tt>").unwrap(),
            ""
        );
    }
    #[test]
    fn input_xml_depth_node_paragraph_and_output_limits_are_enforced() {
        assert!(to_srt(&vec![b' '; MAX_INPUT + 1]).is_err());
        let depth = |count| {
            format!(
                "<tt><p begin='0' end='1'>{}text{}</p></tt>",
                "<span>".repeat(count),
                "</span>".repeat(count)
            )
        };
        assert!(to_srt(depth(62).as_bytes()).is_ok());
        assert!(to_srt(depth(63).as_bytes()).is_err());
        let nodes = format!(
            "<tt>{}<p begin='0' end='1'>text</p></tt>",
            "<span/>".repeat(100_000)
        );
        assert!(to_srt(nodes.as_bytes()).is_err());
        let cues = format!(
            "<tt>{}</tt>",
            "<p begin='0' end='1'>text</p>".repeat(10_001)
        );
        assert!(to_srt(cues.as_bytes()).is_err());
        let mut output = "x".repeat(MAX_OUTPUT);
        assert!(append(&mut output, "x").is_err());
        assert_eq!(output.len(), MAX_OUTPUT);
    }
}

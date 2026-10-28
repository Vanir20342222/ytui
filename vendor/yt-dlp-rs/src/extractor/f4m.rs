//! Upstream `InfoExtractor._parse_f4m_formats` for an F4M manifest:
//! `protocol: f4m` formats of its (unencrypted) media entries.
use serde_json::{Value, json};

const NS1: &str = "http://ns.adobe.com/f4m/1.0";
const NS2: &str = "http://ns.adobe.com/f4m/2.0";

/// A parsed manifest: `None` when the document is not an F4M manifest.
pub(crate) enum Manifest {
    /// The format dicts (in upstream key order).
    Formats(Vec<Value>),
    /// Media URLs that are nested manifests (`.f4m`/`.m3u8`), which
    /// upstream extracts recursively.
    Nested,
}

fn text(node: roxmltree::Node<'_, '_>) -> Option<String> {
    let mut out = String::new();
    for child in node.children() {
        if child.is_element() {
            break;
        }
        out.push_str(child.text().unwrap_or_default());
    }
    (!out.is_empty()).then_some(out)
}

/// `fix_xml_ampersands(manifest).strip()`.
pub(crate) fn fix_ampersands(text: &str) -> String {
    static AMP: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"&(amp;|lt;|gt;|apos;|quot;|#x[0-9a-fA-F]{0,4};|#[0-9]{0,4};)?")
            .expect("static pattern")
    });
    AMP.replace_all(text.trim(), |found: &regex::Captures<'_>| {
        if found.get(1).is_some() {
            found[0].to_owned()
        } else {
            "&amp;".to_owned()
        }
    })
    .into_owned()
}

/// The root tag check of GenericIE (`^{http://ns.adobe.com/f4m/[12].0}manifest$`)
/// and `_parse_f4m_formats(doc, manifest_url, video_id)`.
pub(crate) fn parse(body: &str, manifest_url: &str, f4m_id: Option<&str>) -> Option<Manifest> {
    let body = fix_ampersands(body.trim_start_matches('\u{feff}'));
    let document = roxmltree::Document::parse_with_options(
        &body,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    )
    .ok()?;
    let root = document.root_element();
    if root.tag_name().name() != "manifest"
        || !matches!(root.tag_name().namespace(), Some(NS1 | NS2))
    {
        return None;
    }
    let child = |name: &str, namespaces: &[&str]| {
        namespaces.iter().find_map(|namespace| {
            root.children().find(|node| {
                node.is_element()
                    && node.tag_name().name() == name
                    && node.tag_name().namespace() == Some(namespace)
            })
        })
    };
    // Akamai's player verification challenge cannot be answered.
    if let Some(pv) = child("pv-2.0", &[NS1])
        && let Some(value) = text(pv)
        && let Some((challenge, _)) = value.split_once(';')
        && !challenge.trim().is_empty()
    {
        return Some(Manifest::Formats(Vec::new()));
    }
    let media_of = |namespace: &str| -> Vec<roxmltree::Node<'_, '_>> {
        root.children()
            .filter(|node| {
                node.is_element()
                    && node.tag_name().name() == "media"
                    && node.tag_name().namespace() == Some(namespace)
            })
            .collect()
    };
    let (version, mut media) = match media_of(NS1) {
        nodes if !nodes.is_empty() => ("1.0", nodes),
        _ => ("2.0", media_of(NS2)),
    };
    // `remove_encrypted_media`.
    media.retain(|node| {
        node.attribute("drmAdditionalHeaderId").is_none()
            && node.attribute("drmAdditionalHeaderSetId").is_none()
    });
    let base_url = child("baseURL", &[NS1, NS2])
        .and_then(text)
        .map(|base| base.trim().to_owned())
        .filter(|base| !base.is_empty());
    let bootstrap = child("bootstrapInfo", &[NS1, NS2]).is_some();
    let audio = child("mimeType", &[NS1, NS2])
        .and_then(text)
        .is_some_and(|mime| mime.starts_with("audio/"));
    let int = |value: Option<&str>| value.and_then(|value| value.trim().parse::<i64>().ok());
    let mut formats = Vec::new();
    for (index, node) in media.iter().enumerate() {
        let tbr = int(node.attribute("bitrate"));
        let width = int(node.attribute("width"));
        let height = int(node.attribute("height"));
        // `join_nonempty(f4m_id, tbr or i)`.
        let number = tbr
            .filter(|tbr| *tbr != 0)
            .unwrap_or(index as i64)
            .to_string();
        let format_id = match f4m_id {
            Some(prefix) if !prefix.is_empty() => format!("{prefix}-{number}"),
            _ => number,
        };
        let mut url = manifest_url.to_owned();
        if !bootstrap {
            let media_url = if version == "2.0" {
                node.attribute("href")
            } else {
                None
            }
            .or_else(|| node.attribute("url"));
            let Some(media_url) = media_url.filter(|url| !url.is_empty()) else {
                continue;
            };
            url = if media_url.starts_with("http://") || media_url.starts_with("https://") {
                media_url.to_owned()
            } else {
                let base = base_url.clone().unwrap_or_else(|| {
                    let parts: Vec<&str> = manifest_url.split('/').collect();
                    parts[..parts.len() - 1].join("/")
                });
                format!("{base}/{media_url}")
            };
            if matches!(
                super::direct::determine_ext(&url).as_deref(),
                Some("f4m" | "m3u8")
            ) {
                return Some(Manifest::Nested);
            }
        }
        formats.push(json!({
            "format_id": format_id,
            "url": url,
            "manifest_url": url,
            "ext": if bootstrap { Value::from("flv") } else { Value::Null },
            "protocol": "f4m",
            "tbr": tbr,
            "width": width,
            "height": height,
            "vcodec": if audio { Value::from("none") } else { Value::Null },
            "preference": null,
            "quality": null,
        }));
    }
    Some(Manifest::Formats(formats))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_follow_upstream_parse_f4m_formats() {
        let Some(Manifest::Formats(formats)) = parse(
            r#"<manifest xmlns="http://ns.adobe.com/f4m/2.0"><baseURL> https://h/b </baseURL><media href="a.mp4" bitrate="64"/><media url="x&y.mp4"/><media drmAdditionalHeaderId="1" url="d"/></manifest>"#,
            "https://h/m.f4m",
            None,
        ) else {
            panic!("formats expected");
        };
        assert_eq!(formats.len(), 2);
        assert_eq!(formats[0]["format_id"], "64");
        assert_eq!(formats[0]["url"], "https://h/b/a.mp4");
        assert_eq!(formats[1]["format_id"], "1");
        assert_eq!(formats[1]["url"], "https://h/b/x&y.mp4");
        assert!(parse("<rss/>", "https://h/m.f4m", None).is_none());
    }
}

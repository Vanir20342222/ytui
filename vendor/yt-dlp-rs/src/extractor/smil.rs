//! Upstream `InfoExtractor._parse_smil` for GenericIE's SMIL documents:
//! the parsed head metadata, media, images and text streams.

/// A `video`/`audio`/`media`, `image`, `imagestream` or `textstream`
/// element's attributes.
pub(crate) type Attributes = Vec<(String, String)>;

pub(crate) struct Smil {
    /// `./head/meta` attributes in document order.
    pub meta: Vec<Attributes>,
    /// `.//video`, then `.//audio`, then `.//media`.
    pub media: Vec<Attributes>,
    pub images: Vec<Attributes>,
    pub imagestreams: Vec<Attributes>,
    pub textstreams: Vec<Attributes>,
}

pub(crate) fn get<'a>(attributes: &'a Attributes, name: &str) -> Option<&'a str> {
    attributes
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

/// The elements of a document whose root is `{NS}smil` (or `smil`); the
/// namespace of `_parse_smil_namespace` applies to the searched children.
pub(crate) fn parse(text_body: &str) -> Option<Smil> {
    let document = roxmltree::Document::parse_with_options(
        text_body.trim_start_matches('\u{feff}'),
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    )
    .ok()?;
    let root = document.root_element();
    if root.tag_name().name() != "smil" {
        return None;
    }
    let namespace = root.tag_name().namespace();
    let named = |node: &roxmltree::Node<'_, '_>, name: &str| {
        node.is_element()
            && node.tag_name().name() == name
            && node.tag_name().namespace() == namespace
    };
    let attributes = |node: roxmltree::Node<'_, '_>| -> Attributes {
        node.attributes()
            .filter(|attribute| attribute.namespace().is_none())
            .map(|attribute| (attribute.name().to_owned(), attribute.value().to_owned()))
            .collect()
    };
    let all = |name: &str| -> Vec<Attributes> {
        root.descendants()
            .skip(1)
            .filter(|node| named(node, name))
            .map(attributes)
            .collect()
    };
    let meta = root
        .children()
        .filter(|node| named(node, "head"))
        .flat_map(|head| {
            head.children()
                .filter(|node| named(node, "meta"))
                .collect::<Vec<_>>()
        })
        .map(attributes)
        .collect();
    let mut media = all("video");
    media.extend(all("audio"));
    media.extend(all("media"));
    Some(Smil {
        meta,
        media,
        images: all("image"),
        imagestreams: all("imagestream"),
        textstreams: all("textstream"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elements_follow_upstream_findall_order() {
        let smil = parse(
            r#"<smil xmlns="urn:x"><head><meta name="title" content="T"/></head><body><audio src="a.m4a"/><video src="v.mp4"/><other xmlns="urn:y"><video src="no.mp4"/></other></body></smil>"#,
        )
        .unwrap();
        assert_eq!(get(&smil.meta[0], "content"), Some("T"));
        let sources: Vec<_> = smil.media.iter().map(|m| get(m, "src").unwrap()).collect();
        assert_eq!(sources, ["v.mp4", "a.m4a"]);
        assert!(parse("<rss/>").is_none());
    }
}

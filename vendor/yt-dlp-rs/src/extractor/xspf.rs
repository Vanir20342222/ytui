//! Upstream `InfoExtractor._parse_xspf` for GenericIE's XSPF playlists: one
//! entry per `trackList/track` with its `location`s as formats.
use url::Url;

const XSPF: &str = "http://xspf.org/ns/0/";
const S1: &str = "http://static.streamone.nl/player/ns/0";

pub(crate) struct Track {
    pub title: Option<String>,
    pub description: Option<String>,
    pub thumbnail: Option<String>,
    pub duration: Option<f64>,
    pub locations: Vec<Location>,
}

pub(crate) struct Location {
    pub url: Url,
    pub label: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

fn children<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    name: &'static str,
) -> impl Iterator<Item = roxmltree::Node<'a, 'input>> {
    node.children().filter(move |child| {
        child.is_element()
            && child.tag_name().name() == name
            && child.tag_name().namespace() == Some(XSPF)
    })
}

/// ElementTree's `.text`: the text before the first child element.
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

/// `xpath_text(track, './xspf:NAME')`.
fn child_text(node: roxmltree::Node<'_, '_>, name: &'static str) -> Option<String> {
    children(node, name).next().and_then(text)
}

/// `int_or_none` of an attribute.
fn int(value: Option<&str>) -> Option<u32> {
    value?.trim().parse().ok()
}

/// The tracks of an XSPF document (root `{http://xspf.org/ns/0/}playlist`),
/// with locations joined to `base`.
pub(crate) fn parse(text_body: &str, base: &Url) -> Option<Vec<Track>> {
    let document = roxmltree::Document::parse_with_options(
        text_body.trim_start_matches('\u{feff}'),
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    )
    .ok()?;
    let root = document.root_element();
    if root.tag_name().name() != "playlist" || root.tag_name().namespace() != Some(XSPF) {
        return None;
    }
    let mut tracks = Vec::new();
    for list in children(root, "trackList") {
        for track in children(list, "track") {
            let locations = children(track, "location")
                .filter_map(|location| {
                    // `urljoin(base, None)` is `None`.
                    let url = base.join(&text(location)?).ok()?;
                    Some(Location {
                        url,
                        label: location.attribute((S1, "label")).map(str::to_owned),
                        width: int(location.attribute((S1, "width"))),
                        height: int(location.attribute((S1, "height"))),
                    })
                })
                .collect();
            tracks.push(Track {
                title: child_text(track, "title"),
                description: child_text(track, "annotation"),
                thumbnail: child_text(track, "image"),
                duration: child_text(track, "duration")
                    .and_then(|duration| duration.trim().parse::<f64>().ok())
                    .map(|duration| duration / 1000.0),
                locations,
            });
        }
    }
    Some(tracks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracks_follow_upstream_parse_xspf() {
        let base = Url::parse("https://h.invalid/lists/x.xspf").unwrap();
        let tracks = parse(
            r#"<playlist xmlns="http://xspf.org/ns/0/" xmlns:s1="http://static.streamone.nl/player/ns/0"><trackList><track><title>T</title><duration>1500</duration><location s1:label="Hi" s1:width="640">a.mp4</location><location/></track></trackList></playlist>"#,
            &base,
        )
        .unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].duration, Some(1.5));
        assert_eq!(tracks[0].locations.len(), 1);
        assert_eq!(
            tracks[0].locations[0].url.as_str(),
            "https://h.invalid/lists/a.mp4"
        );
        assert_eq!(tracks[0].locations[0].width, Some(640));
        assert!(parse("<playlist/>", &base).is_none());
    }
}

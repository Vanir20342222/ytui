//! Upstream `GenericIE._extract_rss`: an `<rss>` document's
//! `channel/item`s as `url_transparent` entries (the enclosure URL, else the
//! link, smuggled with the `guid` as `force_videoid`) with their item and
//! iTunes fields.
use serde_json::Value;

const ITUNES: &str = "http://www.itunes.com/dtds/podcast-1.0.dtd";

pub(crate) struct Feed {
    pub title: Option<String>,
    pub description: Option<String>,
    pub items: Vec<Item>,
}

pub(crate) struct Item {
    pub url: String,
    pub guid: Option<String>,
    /// The entry's fields after `_type`/`url`, in upstream order (`null`
    /// for `None`).
    pub fields: Vec<(&'static str, Value)>,
}

fn child<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    name: &str,
    namespace: Option<&str>,
) -> Option<roxmltree::Node<'a, 'input>> {
    node.children().find(|child| {
        child.is_element()
            && child.tag_name().name() == name
            && child.tag_name().namespace() == namespace
    })
}

/// ElementTree's `.text`: the text before the first child element.
fn text(node: Option<roxmltree::Node<'_, '_>>) -> Option<String> {
    let node = node?;
    let mut out = String::new();
    for child in node.children() {
        if child.is_element() {
            break;
        }
        if let Some(text) = child.text() {
            out.push_str(text);
        }
    }
    (!out.is_empty()).then_some(out)
}

/// The feed of an `<rss>` document, if `text` is one.
pub(crate) fn parse(text_body: &str) -> Option<Feed> {
    let document = roxmltree::Document::parse_with_options(
        text_body.trim_start_matches('\u{feff}'),
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    )
    .ok()?;
    let root = document.root_element();
    if root.tag_name().name() != "rss" || root.tag_name().namespace().is_some() {
        return None;
    }
    let channels: Vec<_> = root
        .children()
        .filter(|node| {
            node.is_element()
                && node.tag_name().name() == "channel"
                && node.tag_name().namespace().is_none()
        })
        .collect();
    let mut items = Vec::new();
    for channel in &channels {
        for item in channel.children().filter(|node| {
            node.is_element()
                && node.tag_name().name() == "item"
                && node.tag_name().namespace().is_none()
        }) {
            // `next(enclosure urls, xpath_text(it, 'link'))`: the first
            // enclosure's `url` (even when missing), else the link.
            let enclosure = item.children().find(|node| {
                node.is_element()
                    && node.tag_name().name() == "enclosure"
                    && node.tag_name().namespace().is_none()
            });
            let url = match enclosure {
                Some(enclosure) => enclosure.attribute("url").map(str::to_owned),
                None => text(child(item, "link", None)),
            };
            let Some(url) = url.filter(|url| !url.is_empty()) else {
                continue;
            };
            let guid = text(child(item, "guid", None)).filter(|guid| !guid.is_empty());
            let itunes = |key: &str| text(child(item, key, Some(ITUNES)));
            let string = |value: Option<String>| value.map_or(Value::Null, Value::from);
            let int = |value: Option<String>| {
                value
                    .and_then(|value| value.trim().parse::<i64>().ok())
                    .map_or(Value::Null, Value::from)
            };
            let explicit = itunes("explicit").unwrap_or_default().to_lowercase();
            let fields = vec![
                ("title", string(text(child(item, "title", None)))),
                (
                    "description",
                    string(text(child(item, "description", None))),
                ),
                (
                    "timestamp",
                    text(child(item, "pubDate", None))
                        .and_then(|date| crate::dates::unified_timestamp(&date, true))
                        .map_or(Value::Null, Value::from),
                ),
                (
                    "duration",
                    itunes("duration")
                        .and_then(|duration| crate::parse_duration(&duration))
                        .map_or(Value::Null, |duration| {
                            serde_json::to_value(duration).unwrap_or_default()
                        }),
                ),
                (
                    "thumbnail",
                    child(item, "image", Some(ITUNES))
                        .and_then(|image| image.attribute("href"))
                        .and_then(super::json_ld::url_or_none)
                        .map_or(Value::Null, Value::from),
                ),
                ("episode", string(itunes("title"))),
                ("episode_number", int(itunes("episode"))),
                ("season_number", int(itunes("season"))),
                (
                    "age_limit",
                    match explicit.as_str() {
                        "true" | "yes" => 18.into(),
                        "false" | "no" => 0.into(),
                        _ => Value::Null,
                    },
                ),
            ];
            items.push(Item { url, guid, fields });
        }
    }
    let channel = channels.first().copied();
    Some(Feed {
        title: channel.and_then(|channel| text(child(channel, "title", None))),
        description: channel.and_then(|channel| text(child(channel, "description", None))),
        items,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_follow_upstream_fields() {
        let feed = parse(r#"<?xml version="1.0"?><rss xmlns:itunes="http://www.itunes.com/dtds/podcast-1.0.dtd"><channel><title>F</title><item><title>A</title><enclosure url="https://e.invalid/a.mp3"/><link>https://e.invalid/page</link><guid>g1</guid><itunes:duration>1:02</itunes:duration><itunes:explicit>Yes</itunes:explicit></item><item><link>https://e.invalid/b</link></item><item><title>none</title></item></channel></rss>"#).unwrap();
        assert_eq!(feed.title.as_deref(), Some("F"));
        assert_eq!(feed.items.len(), 2);
        assert_eq!(feed.items[0].url, "https://e.invalid/a.mp3");
        assert_eq!(feed.items[0].guid.as_deref(), Some("g1"));
        assert_eq!(feed.items[0].fields[3], ("duration", 62.into()));
        assert_eq!(feed.items[0].fields[8], ("age_limit", 18.into()));
        assert_eq!(feed.items[1].url, "https://e.invalid/b");
        assert!(parse("<html><body>no</body></html>").is_none());
    }
}

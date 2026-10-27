//! GenericIE `_extract_embeds`' URL patterns after the embed extractors:
//! SWFObject `flashvars`, JW Player `file:` options, `file=`/`source=`
//! parameters, the JW Player JS loader, Flow Player, Cinerama, Twitter
//! `player:stream` cards and Open Graph `og:video`/`og:audio`, then
//! `http-equiv="refresh"` redirects and `twitter:player` iframes.
use regex::Regex;
use std::sync::LazyLock;
use url::Url;

/// What the patterns found.
pub(crate) enum Found {
    /// Media URLs (absolute), with the `report_detected` name.
    Urls(&'static str, Vec<Url>),
    /// `report_following_redirect` or a `twitter:player` iframe: extract
    /// this URL instead.
    Redirect(Url),
}

fn regex(pattern: &str) -> Regex {
    Regex::new(pattern).expect("static pattern")
}

/// Upstream `determine_ext(path, None)` exclusions (`check_video`).
fn check_video(url: &str) -> bool {
    let path = Url::parse(url)
        .map(|url| url.path().to_owned())
        .unwrap_or_else(|_| url.split(['?', '#']).next().unwrap_or(url).to_owned());
    !matches!(
        super::direct::determine_ext(&path).as_deref(),
        None | Some("swf" | "png" | "jpg" | "srt" | "sbv" | "sub" | "vtt" | "ttml" | "js" | "xml")
    )
}

fn find_all(pattern: &Regex, html: &str) -> Vec<String> {
    pattern
        .captures_iter(html)
        .filter_map(|captures| captures.get(1).map(|found| found.as_str().to_owned()))
        .collect()
}

fn filtered(found: Vec<String>) -> Vec<String> {
    found.into_iter().filter(|url| check_video(url)).collect()
}

/// Python `str.encode().decode('unicode-escape')`: backslash escapes are
/// decoded and other non-ASCII bytes read as Latin-1.
fn unicode_escape(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::new();
    let mut index = 0;
    let hex = |slice: &[u8]| -> Option<char> {
        std::str::from_utf8(slice)
            .ok()
            .and_then(|digits| u32::from_str_radix(digits, 16).ok())
            .and_then(char::from_u32)
    };
    while index < bytes.len() {
        let byte = bytes[index];
        if byte != b'\\' || index + 1 >= bytes.len() {
            out.push(char::from(byte));
            index += 1;
            continue;
        }
        let next = bytes[index + 1];
        let (decoded, length) = match next {
            b'\\' => (Some('\\'), 2),
            b'\'' => (Some('\''), 2),
            b'"' => (Some('"'), 2),
            b'n' => (Some('\n'), 2),
            b'r' => (Some('\r'), 2),
            b't' => (Some('\t'), 2),
            b'x' if index + 4 <= bytes.len() => (hex(&bytes[index + 2..index + 4]), 4),
            b'u' if index + 6 <= bytes.len() => (hex(&bytes[index + 2..index + 6]), 6),
            b'U' if index + 10 <= bytes.len() => (hex(&bytes[index + 2..index + 10]), 10),
            _ => (None, 0),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                index += length;
            }
            None => {
                out.push('\\');
                index += 1;
            }
        }
    }
    out
}

/// The patterns' URLs, or a redirect, for a page without HTML5 media.
pub(crate) fn find(page_url: &Url, html: &str) -> Option<Found> {
    static FLASHVARS: LazyLock<Regex> =
        LazyLock::new(|| regex(r#"flashvars: ['"](?:.*&)?file=(http[^'"&]*)"#));
    static GORILLA: LazyLock<Regex> = LazyLock::new(|| {
        regex(
            r#"(?s)(?:jw_plugins|JWPlayerOptions|jwplayer\s*\(\s*["'][^'"]+["']\s*\)\s*\.setup).*?['"]?file['"]?\s*:\s*["'](.*?)["']"#,
        )
    });
    static FILE_PARAMETER: LazyLock<Regex> =
        LazyLock::new(|| regex(r#"[^A-Za-z0-9]?(?:file|source)=(http[^'"&]*)"#));
    static JS_LOADER: LazyLock<Regex> =
        LazyLock::new(|| regex(r#"[^A-Za-z0-9]?(?:file|video_url)["']?:\s*["'](http[^'"]+)["']"#));
    static NUMBERED: LazyLock<Regex> = LazyLock::new(|| regex(r"^http[^'\x22]+\.[0-9]+$"));
    static FLOWPLAYER: LazyLock<Regex> = LazyLock::new(|| {
        regex(
            r#"(?s)flowplayer\("[^"]+",\s*\{[^}]+?\}\s*,\s*\{[^}]+?["']?clip["']?\s*:\s*\{\s*["']?url["']?\s*:\s*["']([^"']+)["']"#,
        )
    });
    static CINERAMA: LazyLock<Regex> =
        LazyLock::new(|| regex(r"cinerama\.embedPlayer\(\s*'[^']+',\s*'([^']+)'"));
    static TWITTER_STREAM: LazyLock<Regex> = LazyLock::new(|| {
        regex(r#"<meta (?:property|name)="twitter:player:stream" (?:content|value)="(.+?)""#)
    });
    static OPEN_GRAPH: LazyLock<Regex> =
        LazyLock::new(|| regex(r#"<meta.*?property="og:(?:video|audio)".*?content="(.*?)""#));
    static REFRESH: LazyLock<Regex> = LazyLock::new(|| {
        regex(
            r#"(?i)<meta\s+(?:[a-z-]+="[^"]+"\s+)*?content="[0-9]{0,2};\s*(?:URL|url)='?([^'"]+)"#,
        )
    });
    static HTTP_EQUIV: LazyLock<Regex> = LazyLock::new(|| regex(r#"(?i)http-equiv="refresh""#));

    type Step<'a> = Box<dyn Fn() -> Vec<String> + 'a>;
    let steps: [(&'static str, Step<'_>); 8] = [
        (
            "JW Player in SFWObject",
            Box::new(|| filtered(find_all(&FLASHVARS, html))),
        ),
        (
            "JW Player embed",
            Box::new(|| filtered(find_all(&GORILLA, html))),
        ),
        (
            "video file",
            Box::new(|| filtered(find_all(&FILE_PARAMETER, html))),
        ),
        (
            "JW Player JS loader",
            // `http(?![^'"]+\.[0-9]+['"])`: no URL ending in `.DIGITS`.
            Box::new(|| {
                filtered(
                    find_all(&JS_LOADER, html)
                        .into_iter()
                        .filter(|url| !NUMBERED.is_match(url))
                        .collect(),
                )
            }),
        ),
        (
            "Flow Player",
            Box::new(|| filtered(find_all(&FLOWPLAYER, html))),
        ),
        ("Cinerama player", Box::new(|| find_all(&CINERAMA, html))),
        (
            "Twitter card",
            Box::new(|| filtered(find_all(&TWITTER_STREAM, html))),
        ),
        (
            "Open Graph video info",
            Box::new(|| filtered(find_all(&OPEN_GRAPH, html))),
        ),
    ];
    for (name, step) in steps {
        let found = step();
        if found.is_empty() {
            continue;
        }
        // `orderedSet`, then the escapes, `\/` and `urljoin`.
        let mut urls: Vec<Url> = Vec::new();
        let mut seen = Vec::new();
        for raw in found {
            if seen.contains(&raw) {
                continue;
            }
            seen.push(raw.clone());
            let text = crate::html::unescape_html(&unicode_escape(&raw)).replace("\\/", "/");
            if let Ok(url) = page_url.join(&text) {
                urls.push(url);
            }
        }
        return Some(Found::Urls(name, urls));
    }
    // A `<meta http-equiv="refresh" content="N; url=...">` redirect.
    if let Some(captures) = REFRESH
        .captures_iter(html)
        .find(|captures| HTTP_EQUIV.is_match(captures.get(0).map_or("", |all| all.as_str())))
        && let Ok(url) = page_url.join(&crate::html::unescape_html(&captures[1]))
        && url != *page_url
    {
        return Some(Found::Redirect(url));
    }
    // `twitter:player` is checked last.
    if let Some(embed) = crate::html::html_search_meta(html, "twitter:player")
        && let Ok(url) = Url::parse(&embed)
        && url != *page_url
    {
        return Some(Found::Redirect(url));
    }
    None
}

/// A Video.js embed: `player.src(...)` sources (URL, lowercased type and
/// extension) and `addRemoteTextTrack` subtitles (as upstream's dicts).
pub(crate) struct VideoJs {
    pub sources: Vec<(Url, Option<String>, String)>,
    /// Language and `{url, name, http_headers}` dicts in page order.
    pub subtitles: Vec<(String, serde_json::Map<String, serde_json::Value>)>,
}

/// Upstream's Video.js step: the first `videojs(...)...VAR.src([...]);`,
/// its sources and `VAR.addRemoteTextTrack({...}, true|false)` tracks.
/// YouTube sources are unsupported.
pub(crate) fn videojs(html: &str, url: &Url, actual: &Url) -> crate::Result<Option<VideoJs>> {
    use serde_json::{Map, Value};
    static PLAYER: LazyLock<Regex> = LazyLock::new(|| {
        regex(r"(?s)\bvideojs\s*\(.+?([a-zA-Z0-9_$]+)\.src\s*\(\s*(\[.+?\]|\{.+?\})\s*\)\s*;")
    });
    let unsupported = |reason: &str| crate::Error::UnsupportedHtml5(reason.into());
    let Some(found) = PLAYER.captures(html) else {
        return Ok(None);
    };
    let variable = found[1].to_owned();
    // `variadic(self._parse_json(..., transform_source=js_to_json, fatal=False) or [])`.
    let parsed: Option<Value> =
        serde_json::from_str(&crate::js_to_json::js_to_json(&found[2])).ok();
    let items = match parsed {
        Some(Value::Array(items)) => items,
        Some(value) if python_truthy(&value) => vec![value],
        _ => Vec::new(),
    };
    let mut sources = Vec::new();
    for source in &items {
        let Some(src) = source
            .get("src")
            .and_then(Value::as_str)
            .filter(|src| !src.is_empty())
        else {
            continue;
        };
        let Ok(src) = url.join(src) else { continue };
        let kind = source
            .get("type")
            .and_then(Value::as_str)
            .map(str::to_lowercase);
        let ext = super::direct::determine_ext(src.as_str())
            .unwrap_or_else(|| "unknown_video".into())
            .to_lowercase();
        if kind.as_deref() == Some("video/youtube") {
            return Err(unsupported(
                "Video.js YouTube sources are not supported natively",
            ));
        }
        sources.push((src, kind, ext));
    }
    let referer = || Map::from_iter([("Referer".to_owned(), Value::from(actual.as_str()))]);
    let track = Regex::new(&format!(
        r"(?s){}\.addRemoteTextTrack\((\{{.+?\}})\s*,\s*(?:true|false)\)",
        regex::escape(&variable)
    ))
    .expect("escaped pattern");
    let mut subtitles = Vec::new();
    for found in track.captures_iter(html) {
        let sub: Value =
            serde_json::from_str(&crate::js_to_json::js_to_json(&found[1])).unwrap_or(Value::Null);
        let Some(src) = sub
            .get("src")
            .and_then(|src| match src {
                Value::String(text) => Some(text.clone()),
                Value::Number(number) => Some(number.to_string()),
                _ => None,
            })
            .filter(|src| !src.is_empty())
        else {
            continue;
        };
        let Ok(src) = url.join(&src) else { continue };
        let language = ["language", "srclang"]
            .iter()
            .find_map(|key| sub.get(*key).filter(|value| python_truthy(value)))
            .and_then(Value::as_str)
            .unwrap_or("und")
            .to_owned();
        let mut entry = Map::new();
        entry.insert("url".into(), src.as_str().into());
        entry.insert(
            "name".into(),
            sub.get("label").cloned().unwrap_or(Value::Null),
        );
        entry.insert("http_headers".into(), Value::Object(referer()));
        subtitles.push((language, entry));
    }
    if sources.is_empty() && subtitles.is_empty() {
        return Ok(None);
    }
    Ok(Some(VideoJs { sources, subtitles }))
}

/// Python truthiness.
fn python_truthy(value: &serde_json::Value) -> bool {
    use serde_json::Value;
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(number) => number.as_f64() != Some(0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(map) => !map.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns_follow_upstream_order_and_filters() {
        let page = Url::parse("http://e.invalid/p/page.html").unwrap();
        let found = |html: &str| match find(&page, html) {
            Some(Found::Urls(name, urls)) => {
                format!(
                    "{name}: {}",
                    urls.iter().map(Url::as_str).collect::<Vec<_>>().join(" ")
                )
            }
            Some(Found::Redirect(url)) => format!("redirect {url}"),
            None => "none".into(),
        };
        assert_eq!(
            found(
                r#"<meta property="og:video" content="v.mp4"><meta property="og:image" content="i.jpg">"#
            ),
            "Open Graph video info: http://e.invalid/p/v.mp4"
        );
        assert_eq!(
            found(
                r#"<meta name="twitter:player:stream" content="s.mp4"><meta property="og:video" content="v.mp4">"#
            ),
            "Twitter card: http://e.invalid/p/s.mp4"
        );
        // Images and subtitles are not videos.
        assert_eq!(
            found(r#"<meta property="og:video" content="i.jpg">"#),
            "none"
        );
        assert_eq!(
            found(r#"x video_url: 'http://cdn.invalid/a.1' file: "http://cdn.invalid/b.mp4""#),
            "JW Player JS loader: http://cdn.invalid/b.mp4"
        );
        assert_eq!(
            found(r#"<meta http-equiv="refresh" content="0; url=/next.html">"#),
            "redirect http://e.invalid/next.html"
        );
        assert_eq!(unicode_escape(r"a/b\\c"), r"a/b\c");
    }
}

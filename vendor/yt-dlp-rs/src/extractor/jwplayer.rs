//! Upstream `_find_jwplayer_data` and `_parse_jwplayer_data` for GenericIE
//! (`require_title=False`, `base_url` the page URL): one playlist item with
//! direct HTTP sources. Manifest, SMIL and RTMP sources, several items and
//! YouTube sources are reported as unsupported.
use crate::{Error, Result};
use regex::Regex;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

/// A manifest source to expand (`_extract_m3u8_formats`/`_extract_mpd_formats`).
pub(crate) const MANIFEST: &str = "__yt_dlp_rs_jwplayer_manifest";

fn unsupported(reason: &str) -> Error {
    Error::UnsupportedHtml5(reason.into())
}

/// `_find_jwplayer_data(webpage, transform_source=js_to_json)`: the first
/// `jwplayer("ID")...setup({...})` (or `.load([{...}])`) on one line, its
/// JSON from the first `{` to the last `}` before `)` (`]`), as
/// `_search_json`'s greedy pattern takes it, parsed with `ignore_extra`.
pub(crate) fn find_data(html: &str) -> Option<Value> {
    static START: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r#"^jwplayer\s*\(\s*(?:"[^"\n][^\n]*"|'[^'\n][^\n]*')\s*\)([^\n]*?)\.\s*(?:setup\s*\(|(load)\s*\(\s*\[)"#,
        )
        .expect("static pattern")
    });
    let bytes = html.as_bytes();
    for (position, _) in html.match_indices("jwplayer") {
        // `(?<!-)\b`.
        if position > 0 {
            let before = bytes[position - 1];
            if before == b'-' || before.is_ascii_alphanumeric() || before == b'_' {
                continue;
            }
        }
        let Some(start) = START.captures(&html[position..]) else {
            continue;
        };
        // `(?:(?!</script>).)*?`.
        if start[1].contains("</script>") {
            continue;
        }
        let load = start.get(2).is_some();
        let rest = &html[position + start.get(0).map_or(0, |all| all.end())..];
        let body = rest.trim_start();
        if !body.starts_with('{') {
            continue;
        }
        let end = if load { ']' } else { ')' };
        // The last `}` followed by `\s*` and the closing bracket.
        let close = body
            .rmatch_indices('}')
            .map(|(index, _)| index)
            .find(|index| body[index + 1..].trim_start().starts_with(end))?;
        return crate::js_to_json::parse_js_value(&body[..=close]);
    }
    None
}

/// Python `int_or_none(value)` (and `scale`): ints, floats truncated and
/// integer strings.
fn int_or_none(value: Option<&Value>, scale: i64) -> Option<i64> {
    let number = match value? {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value.trunc() as i64))?,
        Value::String(text) => text.trim().parse::<i64>().ok()?,
        Value::Bool(value) => i64::from(*value),
        _ => return None,
    };
    Some(number.div_euclid(scale))
}

/// `float_or_none(value)`.
fn float_or_none(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse().ok(),
        Value::Bool(value) => Some(f64::from(u8::from(*value))),
        _ => None,
    }
}

/// `str_or_none`.
fn str_or_none(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Null => None,
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(value) => Some(if *value { "True" } else { "False" }.into()),
        _ => None,
    }
}

/// `clean_html` of a string value.
fn clean_html(value: Option<&Value>) -> Option<String> {
    Some(crate::html::clean_html(value?.as_str()?))
}

/// `urljoin(base_url, self._proto_relative_url(url))` (`//` URLs get
/// `https:`).
fn join(base: &Url, value: Option<&Value>) -> Option<Url> {
    let text = value?.as_str()?.trim();
    if text.is_empty() {
        return None;
    }
    let text = if text.starts_with("//") {
        format!("https:{text}")
    } else {
        text.to_owned()
    };
    base.join(&text).ok()
}

/// `parse_resolution(label)['height']` for labels like `720p` or `1080`.
fn label_height(label: &str) -> Option<i64> {
    static DIMENSIONS: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?:^|[^\w])(\d+)\s*[xX×,]\s*(\d+)(?:[^\w]|$)").expect("static pattern")
    });
    static PROGRESSIVE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)(?:^|[^\w])(\d+)[pi](?:[^\w]|$)").expect("static pattern")
    });
    static K: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)\b([48])[kK]\b").expect("static pattern"));
    if let Some(found) = DIMENSIONS.captures(label) {
        return found[2].parse().ok();
    }
    if let Some(found) = PROGRESSIVE.captures(label) {
        return found[1].parse().ok();
    }
    if let Some(found) = K.captures(label) {
        return Some(if &found[1] == "4" { 2160 } else { 4320 });
    }
    None
}

/// One `_parse_jwplayer_data` entry in upstream key order: the page's
/// `video_id`, then `merge_dicts` drops its `None` values (formats keep
/// theirs). `None` when it has no formats.
pub(crate) fn parse(
    data: &Value,
    video_id: &str,
    base: &Url,
) -> Result<Option<Map<String, Value>>> {
    parse_with(data, video_id, base, false)
}

/// [`parse`] as `_parse_jwplayer_data(..., require_title=True)` returns it
/// to an extractor: `video_data['title']` is required and the entry keeps
/// its `None` values.
pub(crate) fn parse_required(
    data: &Value,
    video_id: &str,
    base: &Url,
) -> Result<Option<Map<String, Value>>> {
    parse_with(data, video_id, base, true)
}

fn parse_with(
    data: &Value,
    video_id: &str,
    base: &Url,
    required: bool,
) -> Result<Option<Map<String, Value>>> {
    let Some(object) = data.as_object() else {
        return Ok(None);
    };
    let items: Vec<&Value> = match object.get("playlist") {
        Some(Value::Array(items)) => items.iter().collect(),
        Some(item) if !item.is_null() && item != &Value::Bool(false) => vec![item],
        _ => vec![data],
    };
    let items: Vec<&Map<String, Value>> =
        items.iter().filter_map(|item| item.as_object()).collect();
    if items.len() > 1 {
        return Err(unsupported(
            "JW Player playlists are not supported natively",
        ));
    }
    let Some(item) = items.first() else {
        return Ok(None);
    };
    // Flattened sources: `video_data['sources'] = [video_data]`; a
    // non-list value iterates as keys (strings), which are skipped.
    let sources: Vec<Value> = match item.get("sources") {
        Some(Value::Array(sources)) => sources.clone(),
        Some(_) => Vec::new(),
        None => vec![Value::Object((*item).clone())],
    };
    let mut seen = Vec::new();
    let mut formats = Vec::new();
    for source in sources.iter().filter_map(Value::as_object) {
        let Some(url) = join(base, source.get("file")) else {
            continue;
        };
        if seen.contains(&url) {
            continue;
        }
        seen.push(url.clone());
        let kind = source.get("type").and_then(Value::as_str).unwrap_or("");
        let ext = super::direct::determine_ext(url.as_str()).or_else(|| {
            Some(super::direct::mimetype2ext_or(kind, None)).filter(|ext| !ext.is_empty())
        });
        // Manifest sources are expanded by the caller (`MANIFEST` marks).
        let hls = kind == "hls"
            || ext.as_deref() == Some("m3u8")
            || url.as_str().contains("format=m3u8-aapl");
        if hls
            || kind == "dash"
            || ext.as_deref() == Some("mpd")
            || url.as_str().contains("format=mpd-time-csf")
        {
            let mut source = Map::new();
            source.insert(MANIFEST.into(), url.as_str().into());
            source.insert("dash".into(), (!hls).into());
            formats.push(Value::Object(source));
            continue;
        }
        if ext.as_deref() == Some("smil") {
            return Err(unsupported(
                "JW Player SMIL sources are not supported natively",
            ));
        }
        if url.scheme().starts_with("rtmp") {
            return Err(unsupported(
                "JW Player RTMP sources are not supported natively",
            ));
        }
        let mut format = Map::new();
        if kind.starts_with("audio")
            || matches!(
                ext.as_deref(),
                Some("oga" | "aac" | "mp3" | "mpeg" | "vorbis")
            )
        {
            format.insert("url".into(), url.as_str().into());
            format.insert("vcodec".into(), "none".into());
            format.insert("ext".into(), ext.map_or(Value::Null, Value::from));
        } else {
            let format_id = str_or_none(source.get("label"));
            let height = int_or_none(source.get("height"), 1)
                .or_else(|| format_id.as_deref().and_then(label_height));
            let number = |value: Option<i64>| value.map_or(Value::Null, Value::from);
            format.insert("url".into(), url.as_str().into());
            format.insert("width".into(), number(int_or_none(source.get("width"), 1)));
            format.insert("height".into(), number(height));
            format.insert(
                "tbr".into(),
                number(int_or_none(source.get("bitrate"), 1000)),
            );
            format.insert(
                "filesize".into(),
                number(int_or_none(source.get("filesize"), 1)),
            );
            format.insert("ext".into(), ext.map_or(Value::Null, Value::from));
            format.insert(
                "format_id".into(),
                format_id.map_or(Value::Null, Value::from),
            );
        }
        formats.push(Value::Object(format));
    }
    if formats.is_empty() {
        return Ok(None);
    }
    // One YouTube source makes a `url_transparent` result.
    static YOUTUBE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^(?:http|//).*(?:youtube\.com|youtu\.be)/.+").expect("static pattern")
    });
    if formats.len() == 1
        && formats[0]
            .get("url")
            .and_then(Value::as_str)
            .is_some_and(|url| YOUTUBE.is_match(url))
    {
        return Err(unsupported(
            "JW Player YouTube sources are not supported natively",
        ));
    }
    let mut subtitles = Map::new();
    if let Some(Value::Array(tracks)) = item.get("tracks") {
        for track in tracks.iter().filter_map(Value::as_object) {
            let kind = track
                .get("kind")
                .and_then(Value::as_str)
                .map(str::to_lowercase);
            if !matches!(kind.as_deref(), Some("captions" | "subtitles")) {
                continue;
            }
            let Some(url) = join(base, track.get("file")) else {
                continue;
            };
            let label = track
                .get("label")
                .and_then(Value::as_str)
                .filter(|label| !label.is_empty())
                .unwrap_or("en");
            let entry = subtitles
                .entry(label.to_owned())
                .or_insert_with(|| Value::Array(Vec::new()));
            if let Value::Array(list) = entry {
                list.push(serde_json::json!({"url": url.as_str()}));
            }
        }
    }
    let text = |key: &str| {
        item.get(key)
            .and_then(Value::as_str)
            .map(crate::html::unescape_html)
    };
    if required && !item.contains_key("title") {
        return Err(Error::Reported(format!(
            "{video_id}: An extractor error has occurred. (caused by KeyError('title')); {}",
            crate::BUG_REPORTS_MESSAGE
        )));
    }
    let mut entry = Map::new();
    let mut put = |key: &str, value: Option<Value>| match value.filter(|value| !value.is_null()) {
        Some(value) => {
            entry.insert(key.to_owned(), value);
        }
        None if required => {
            entry.insert(key.to_owned(), Value::Null);
        }
        None => {}
    };
    put("id", Some(video_id.into()));
    put("title", text("title").map(Value::from));
    put(
        "description",
        clean_html(item.get("description")).map(Value::from),
    );
    put(
        "thumbnail",
        join(base, item.get("image")).map(|url| Value::from(url.as_str())),
    );
    put(
        "timestamp",
        int_or_none(item.get("pubdate"), 1).map(Value::from),
    );
    put(
        "duration",
        float_or_none(
            object
                .get("duration")
                .filter(|value| python_truthy(value))
                .or_else(|| item.get("duration")),
        )
        .map(Value::from),
    );
    put("subtitles", Some(Value::Object(subtitles)));
    put(
        "alt_title",
        clean_html(item.get("subtitle")).map(Value::from),
    );
    put("genre", clean_html(item.get("genre")).map(Value::from));
    put(
        "channel",
        clean_html(
            item.get("category")
                .filter(|value| python_truthy(value))
                .or_else(|| item.get("channel")),
        )
        .map(Value::from),
    );
    put(
        "season_number",
        int_or_none(item.get("season"), 1).map(Value::from),
    );
    put(
        "episode_number",
        int_or_none(item.get("episode"), 1).map(Value::from),
    );
    put(
        "release_year",
        int_or_none(item.get("releasedate"), 1).map(Value::from),
    );
    put(
        "age_limit",
        int_or_none(item.get("age_restriction"), 1).map(Value::from),
    );
    put("formats", Some(Value::Array(formats)));
    Ok(Some(entry))
}

/// Python truthiness.
fn python_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(number) => number.as_f64() != Some(0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(map) => !map.is_empty(),
    }
}

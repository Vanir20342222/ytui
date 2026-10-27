//! Upstream `render_formats_table` (`-F`), including the `--list-formats-old`
//! layout, over the serialized info dict without styles. Pure: never selects
//! or inspects resources.
use crate::VideoInfo;
use serde_json::{Map, Value};

type Dict = Map<String, Value>;

/// Upstream `__list_table` text: `[info] Available formats for ID:` and the
/// table, or `ID has no formats`. Output has a trailing newline.
pub fn render_formats(info: &VideoInfo) -> String {
    render_formats_listing(info, true)
}

/// [`render_formats`] with the `--list-formats-old` layout when `as_table` is false.
pub fn render_formats_listing(info: &VideoInfo, as_table: bool) -> String {
    match render_formats_table(info, as_table) {
        Some(table) => format!(
            "[info] Available formats for {}:\n{table}\n",
            escape(&info.id)
        ),
        None => format!("{} has no formats\n", escape(&info.id)),
    }
}

/// Upstream `render_formats_table`: `None` without formats (or a URL).
/// `as_table` is `listformats_table` (false for `--list-formats-old`).
/// Text fields have terminal controls replaced by visible escapes.
pub fn render_formats_table(info: &VideoInfo, as_table: bool) -> Option<String> {
    let Ok(Value::Object(info)) = serde_json::to_value(info) else {
        return None;
    };
    // `_get_formats`.
    let formats: Vec<&Dict> = match info.get("formats") {
        Some(Value::Array(formats)) => formats.iter().filter_map(Value::as_object).collect(),
        _ if get(&info, "url").is_some_and(truthy) => vec![&info],
        _ => Vec::new(),
    };
    if formats.is_empty() {
        return None;
    }
    if !as_table {
        let rows = formats
            .iter()
            .filter(|f| get(f, "preference").and_then(Value::as_f64).unwrap_or(0.0) >= -1000.0)
            .map(|f| {
                vec![
                    field(f, "format_id", "%s"),
                    field(f, "ext", "%s"),
                    format_resolution(f),
                    format_note(f),
                ]
            })
            .collect();
        let header = ["format code", "extension", "resolution", "note"].map(String::from);
        return Some(render_table(header.to_vec(), rows, None, 1, false));
    }
    let delim = "|".to_owned();
    let duration = get(&info, "duration").and_then(Value::as_f64);
    let rows = formats
        .iter()
        .filter(|f| {
            get(f, "preference")
                .and_then(Value::as_f64)
                .is_none_or(|p| p >= -1000.0)
        })
        .map(|f| {
            let filesize = [
                (get(f, "filesize").and_then(Value::as_f64), " \t"),
                (get(f, "filesize_approx").and_then(Value::as_f64), "≈\t"),
                (
                    // `filesize_from_tbr`.
                    get(f, "tbr")
                        .and_then(Value::as_f64)
                        .zip(duration)
                        .map(|(tbr, duration)| (duration * tbr * (1000.0 / 8.0)).trunc()),
                    "~\t",
                ),
            ]
            .into_iter()
            .find_map(|(value, prefix)| {
                value
                    .filter(|value| *value != 0.0)
                    .map(|value| format!("{prefix}{}", super::progress::format_bytes(Some(value))))
            })
            .unwrap_or_default();
            let more_info = join_nonempty(
                [
                    field(f, "language", "[%s]"),
                    join_nonempty(
                        [
                            if matches!(text_of(f, "ext").as_deref(), Some("f4f" | "f4m")) {
                                "UNSUPPORTED".into()
                            } else {
                                String::new()
                            },
                            match get(f, "has_drm") {
                                Some(Value::String(value)) if value == "maybe" => {
                                    "Maybe DRM".into()
                                }
                                Some(value) if truthy(value) => "DRM".into(),
                                _ => String::new(),
                            },
                            if get(f, "__needs_testing").is_some_and(truthy) {
                                "Untested".into()
                            } else {
                                String::new()
                            },
                            field(f, "format_note", "%s"),
                            match (get(f, "container"), get(f, "ext")) {
                                (Some(container), ext) if Some(container) != ext => {
                                    py_text(container)
                                }
                                _ => String::new(),
                            },
                        ],
                        ", ",
                    ),
                ],
                " ",
            );
            vec![
                field(f, "format_id", "%s"),
                field(f, "ext", "%s"),
                format_resolution(f),
                rounded(f, "fps", "\t", ""),
                match get(f, "dynamic_range") {
                    Some(Value::String(range)) if range == "SDR" => String::new(),
                    Some(range) => py_text(range).replace("HDR", ""),
                    None => String::new(),
                },
                field(f, "audio_channels", "\t%s"),
                delim.clone(),
                filesize,
                rounded(f, "tbr", "\t", "k"),
                text_of(f, "protocol")
                    .map(|protocol| shorten_protocol_name(&protocol))
                    .unwrap_or_default(),
                delim.clone(),
                simplified_codec(f, "vcodec"),
                rounded(f, "vbr", "\t", "k"),
                simplified_codec(f, "acodec"),
                rounded(f, "abr", "\t", "k"),
                get(f, "asr")
                    .filter(|value| truthy(value))
                    .and_then(Value::as_f64)
                    .and_then(|asr| format_decimal_suffix(asr, 1000.0))
                    .map(|asr| format!("\t{asr}"))
                    .unwrap_or_default(),
                more_info,
            ]
        })
        .collect();
    let header = [
        "ID",
        "EXT",
        "RESOLUTION",
        "\tFPS",
        "HDR",
        "CH",
        "|",
        "\tFILESIZE",
        "\tTBR",
        "PROTO",
        "|",
        "VCODEC",
        "\tVBR",
        "ACODEC",
        "\tABR",
        "\tASR",
        "MORE INFO",
    ]
    .map(String::from);
    Some(render_table(header.to_vec(), rows, Some("-"), 0, true))
}

/// Upstream `render_table`: text after a tab is right-aligned.
pub(crate) fn render_table(
    header: Vec<String>,
    data: Vec<Vec<String>>,
    delim: Option<&str>,
    extra_gap: usize,
    hide_empty: bool,
) -> String {
    let width = |text: &str| text.chars().filter(|c| *c != '\t').count();
    let max_lens = |table: &[Vec<String>]| -> Vec<usize> {
        let columns = table.first().map_or(0, Vec::len);
        (0..columns)
            .map(|column| {
                table
                    .iter()
                    .map(|row| width(&row[column]))
                    .max()
                    .unwrap_or(0)
            })
            .collect()
    };
    let (header, data) = if hide_empty {
        let keep: Vec<bool> = max_lens(&data).into_iter().map(|len| len != 0).collect();
        let filter = |row: Vec<String>| -> Vec<String> {
            row.into_iter()
                .enumerate()
                .filter(|(index, _)| keep.get(*index).copied().unwrap_or(true))
                .map(|(_, cell)| cell)
                .collect()
        };
        (filter(header), data.into_iter().map(filter).collect())
    } else {
        (header, data)
    };
    let mut table = vec![header];
    table.extend(data);
    let lens = max_lens(&table);
    let gap = extra_gap + 1;
    if let Some(delim) = delim {
        let mut line: Vec<String> = lens.iter().map(|len| delim.repeat(len + gap)).collect();
        if let Some(last) = line.last_mut() {
            last.truncate(last.len() - gap * delim.len());
        }
        table.insert(1, line);
    }
    table
        .into_iter()
        .map(|row| {
            row.iter()
                .enumerate()
                .map(|(index, text)| {
                    // Python repeats a string zero times for negative counts
                    // (the delimiter row is already gap-wide).
                    let padding = lens[index].saturating_sub(width(text));
                    if text.contains('\t') {
                        format!(
                            "{}{}",
                            text.replace('\t', &" ".repeat(padding)),
                            " ".repeat(gap)
                        )
                    } else {
                        let total = (lens[index] + gap).saturating_sub(width(text));
                        format!("{text}{}", " ".repeat(total))
                    }
                })
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Upstream `_format_note` (`--list-formats-old`).
fn format_note(f: &Dict) -> String {
    let mut res = String::new();
    let separate = |res: &mut String, delim: &str| {
        if !res.is_empty() {
            res.push_str(delim);
        }
    };
    if matches!(text_of(f, "ext").as_deref(), Some("f4f" | "f4m")) {
        res.push_str("(unsupported)");
    }
    if let Some(language) = get(f, "language").filter(|value| truthy(value)) {
        separate(&mut res, " ");
        res.push_str(&format!("[{}]", py_text(language)));
    }
    if let Some(note) = get(f, "format_note") {
        separate(&mut res, " ");
        res.push_str(&py_text(note));
    }
    let number = |key: &str| get(f, key).and_then(Value::as_f64);
    if let Some(tbr) = number("tbr") {
        separate(&mut res, ", ");
        res.push_str(&format!("{:4}k", tbr.trunc() as i64));
    }
    if let Some(container) = get(f, "container") {
        separate(&mut res, ", ");
        res.push_str(&format!("{} container", py_text(container)));
    }
    let vcodec = text_of(f, "vcodec");
    if let Some(vcodec) = vcodec.as_deref().filter(|codec| *codec != "none") {
        separate(&mut res, ", ");
        res.push_str(vcodec);
        if number("vbr").is_some() {
            res.push('@');
        }
    } else if number("vbr").is_some() && number("abr").is_some() {
        res.push_str("video@");
    }
    if let Some(vbr) = number("vbr") {
        res.push_str(&format!("{:4}k", vbr.trunc() as i64));
    }
    if let Some(fps) = get(f, "fps") {
        separate(&mut res, ", ");
        res.push_str(&format!("{}fps", py_text(fps)));
    }
    if let Some(acodec) = text_of(f, "acodec") {
        separate(&mut res, ", ");
        if acodec == "none" {
            res.push_str("video only");
        } else {
            res.push_str(&format!("{acodec:<5}"));
        }
    } else if number("abr").is_some() {
        separate(&mut res, ", ");
        res.push_str("audio");
    }
    if let Some(abr) = number("abr") {
        res.push_str(&format!("@{:3}k", abr.trunc() as i64));
    }
    if let Some(asr) = number("asr") {
        res.push_str(&format!(" ({:5}Hz)", asr.trunc() as i64));
    }
    if let Some(filesize) = number("filesize") {
        separate(&mut res, ", ");
        res.push_str(&super::progress::format_bytes(Some(filesize)));
    } else if let Some(approx) = number("filesize_approx") {
        separate(&mut res, ", ");
        res.push('~');
        res.push_str(&super::progress::format_bytes(Some(approx)));
    }
    res
}

/// Upstream `format_resolution` (default `unknown`).
fn format_resolution(f: &Dict) -> String {
    let vcodec = text_of(f, "vcodec");
    let acodec = text_of(f, "acodec");
    if vcodec.as_deref() == Some("none") && acodec.as_deref() != Some("none") {
        return "audio only".into();
    }
    if let Some(resolution) = get(f, "resolution") {
        return py_text(resolution);
    }
    let dimension = |key: &str| {
        get(f, key)
            .filter(|value| truthy(value))
            .and_then(Value::as_f64)
            .map(|value| value.trunc() as i64)
    };
    match (dimension("width"), dimension("height")) {
        (Some(width), Some(height)) => format!("{width}x{height}"),
        (_, Some(height)) => format!("{height}p"),
        (Some(width), None) => format!("{width}x?"),
        _ => "unknown".into(),
    }
}

/// `simplified_codec` without styles.
fn simplified_codec(f: &Dict, field: &str) -> String {
    let Some(codec) = get(f, field).filter(|value| truthy(value)).map(py_text) else {
        return "unknown".into();
    };
    if codec != "none" {
        return codec.split('.').take(4).collect::<Vec<_>>().join(".");
    }
    let other = text_of(
        f,
        if field == "vcodec" {
            "acodec"
        } else {
            "vcodec"
        },
    );
    if field == "vcodec" && other.as_deref() == Some("none") {
        return "images".into();
    }
    if field == "acodec" && other.as_deref() == Some("none") {
        return String::new();
    }
    if field == "vcodec" {
        "audio only"
    } else {
        "video only"
    }
    .into()
}

/// Upstream `shorten_protocol_name`.
fn shorten_protocol_name(protocol: &str) -> String {
    match protocol {
        "m3u8_native" => "m3u8",
        "m3u8" => "m3u8F",
        "rtmp_ffmpeg" => "rtmpF",
        "http_dash_segments" => "dash",
        "http_dash_segments_generator" => "dashG",
        "websocket_frag" => "WSfrag",
        other => other,
    }
    .into()
}

/// `format_decimal_suffix(num, '%d%s', factor=FACTOR)`.
fn format_decimal_suffix(num: f64, factor: f64) -> Option<String> {
    if num < 0.0 || !num.is_finite() {
        return None;
    }
    const SUFFIXES: [&str; 9] = ["", "k", "M", "G", "T", "P", "E", "Z", "Y"];
    let exponent = if num == 0.0 {
        0
    } else {
        ((num.ln() / factor.ln()) as i64).clamp(0, 8) as usize
    };
    let converted = num / factor.powi(exponent as i32);
    Some(format!(
        "{}{}",
        converted.trunc() as i64,
        SUFFIXES[exponent]
    ))
}

/// `format_field(f, KEY, PREFIX + '%d' + SUFFIX, func=round)`.
fn rounded(f: &Dict, key: &str, prefix: &str, suffix: &str) -> String {
    get(f, key)
        .filter(|value| truthy(value))
        .and_then(Value::as_f64)
        .map(|value| format!("{prefix}{}{suffix}", value.round_ties_even() as i64))
        .unwrap_or_default()
}

/// `format_field(f, KEY, TEMPLATE)` with the default falsy `ignore`.
fn field(f: &Dict, key: &str, template: &str) -> String {
    get(f, key)
        .filter(|value| truthy(value))
        .map(|value| template.replace("%s", &py_text(value)))
        .unwrap_or_default()
}

fn join_nonempty<const N: usize>(values: [String; N], delim: &str) -> String {
    values
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join(delim)
}

fn get<'a>(f: &'a Dict, key: &str) -> Option<&'a Value> {
    f.get(key).filter(|value| !value.is_null())
}

fn text_of(f: &Dict, key: &str) -> Option<String> {
    get(f, key).map(py_text)
}

/// Python truthiness of a JSON value.
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(number) => number.as_f64() != Some(0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(items) => !items.is_empty(),
    }
}

/// Python `str()` of a JSON value, with terminal controls escaped.
fn py_text(value: &Value) -> String {
    match value {
        Value::String(text) => escape(text),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::Number(number) if number.is_f64() => {
            crate::screen::py_float(number.as_f64().unwrap_or_default())
        }
        other => other.to_string(),
    }
}

/// Terminal controls and bidirectional overrides as visible escapes.
pub(crate) fn escape(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for c in value.chars() {
        if c.is_control()
            || matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{2028}' | '\u{2029}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        {
            if (c as u32) <= 255 {
                output.push_str(&format!("\\x{:02x}", c as u32));
            } else {
                output.push_str(&format!("\\u{{{:x}}}", c as u32));
            }
        } else {
            output.push(c);
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn info(value: Value) -> VideoInfo {
        let mut base = json!({"id":"sample", "title":"Sample", "url":"https://example.invalid/a.mp4",
            "webpage_url":"https://example.invalid/watch", "ext":"mp4", "extractor":"test",
            "extractor_key":"Test"});
        base.as_object_mut()
            .unwrap()
            .extend(value.as_object().unwrap().clone());
        serde_json::from_value(base).unwrap()
    }

    #[test]
    fn table_matches_upstream_layout() {
        // Rendered by pinned upstream `render_formats_table` from the same dict.
        let info = info(json!({"duration": 10, "formats": [
            {"format_id":"140", "url":"https://u.invalid/", "ext":"m4a", "vcodec":"none", "acodec":"mp4a.40.2",
             "abr":129.5, "asr":44100, "audio_channels":2, "protocol":"https", "filesize":160000,
             "format_note":"medium", "language":"en", "tbr":129.5},
            {"format_id":"137", "url":"https://u.invalid/", "ext":"mp4", "vcodec":"avc1.640028", "acodec":"none",
             "width":1920, "height":1080, "fps":30, "vbr":4000.0, "tbr":4000.0,
             "protocol":"m3u8_native", "dynamic_range":"HDR10", "container":"mp4_dash"},
        ]}));
        assert_eq!(
            render_formats_table(&info, true).unwrap(),
            concat!(
                "ID  EXT RESOLUTION FPS HDR CH |   FILESIZE   TBR PROTO | VCODEC        VBR ACODEC      ABR ASR MORE INFO\n",
                "----------------------------------------------------------------------------------------------------------\n",
                "140 m4a audio only          2 |  156.25KiB  130k https | audio only        mp4a.40.2  130k 44k [en] medium\n",
                "137 mp4 1920x1080   30 10     | ~  4.77MiB 4000k m3u8  | avc1.640028 4000k video only          mp4_dash"
            )
        );
        assert_eq!(
            render_formats_table(&info, false).unwrap(),
            concat!(
                "format code  extension  resolution  note\n",
                "140          m4a        audio only  [en] medium,  129k, mp4a.40.2@129k (44100Hz), 156.25KiB\n",
                "137          mp4        1920x1080   4000k, mp4_dash container, avc1.640028@4000k, 30fps, video only"
            )
        );
    }

    #[test]
    fn missing_formats_use_the_info_url() {
        let direct = info(json!({}));
        assert_eq!(
            render_formats(&direct),
            "[info] Available formats for sample:\n\
             EXT RESOLUTION | | VCODEC  ACODEC\n\
             ----------------------------------\n\
             mp4 unknown    | | unknown unknown\n"
        );
    }
}

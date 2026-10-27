//! Bounded named printf templates for the represented root metadata fields.
use crate::{Error, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use std::path::PathBuf;
use std::sync::LazyLock;

/// Metadata available to a template: a video, or a playlist with the
/// upstream playlist-file extras (`playlist_index` 0 and selected counts).
#[derive(Clone, Copy)]
pub(super) enum Source<'a> {
    Video(&'a VideoInfo),
    Playlist {
        info: &'a PlaylistInfo,
        playlist_count: usize,
        n_entries: Option<usize>,
        last_playlist_index: usize,
    },
    /// Unsanitized `--print` rendering of a processed video, with upstream's
    /// prepared `filename` and newline-joined selected `urls`.
    Print {
        info: &'a VideoInfo,
        filename: std::result::Result<&'a str, &'a Error>,
        urls: &'a str,
        filepath: Option<&'a str>,
        prepared: Option<&'a str>,
    },
    /// Unsanitized `--exec` rendering with the stage's `filepath` and
    /// `_filename` values (when set).
    Exec {
        info: &'a VideoInfo,
        filepath: Option<&'a str>,
        filename: Option<&'a str>,
    },
    /// Unsanitized rendering over an arbitrary dict (`--progress-template`).
    Json(&'a serde_json::Value),
    /// Sanitized filename rendering over a dict (playlist `pl_video`).
    JsonFile(&'a serde_json::Value),
    /// `--exec` rendering over a dict (`playlist:` commands).
    JsonExec(&'a serde_json::Value),
}

impl Source<'_> {
    /// Unsanitized rendering, as `prepare_outtmpl` without `sanitize`.
    fn print(self) -> bool {
        matches!(
            self,
            Source::Print { .. } | Source::Exec { .. } | Source::Json(_) | Source::JsonExec(_)
        )
    }

    fn exec(self) -> bool {
        matches!(self, Source::Exec { .. } | Source::JsonExec(_))
    }
}

const MAX_SPEC: usize = 16 * 1024;
const MAX_OUTPUT: usize = 64 * 1024;
const MAX_FIELD: usize = 4096;

fn invalid(message: impl Into<String>) -> Error {
    Error::InvalidTemplate(message.into())
}

#[derive(Default, Clone, Copy)]
struct Format {
    alternate: bool,
    zero: bool,
    left: bool,
    plus: bool,
    space: bool,
    width: usize,
    precision: Option<usize>,
    conversion: char,
}

enum Value<'a> {
    Text(&'a str),
    Integer(u64),
    Float(f64),
    Boolean(bool),
    /// A traversed value: owned strings and Python objects (lists, dicts,
    /// negative integers).
    Owned(String),
    Json(serde_json::Value),
}

impl Value<'_> {
    fn from_json(value: serde_json::Value) -> Self {
        match value {
            serde_json::Value::String(text) => Value::Owned(text),
            serde_json::Value::Bool(value) => Value::Boolean(value),
            serde_json::Value::Number(number) if number.is_u64() => {
                Value::Integer(number.as_u64().unwrap_or_default())
            }
            serde_json::Value::Number(number) if number.is_f64() => {
                Value::Float(number.as_f64().unwrap_or_default())
            }
            other => Value::Json(other),
        }
    }

    fn text(&self) -> Option<&str> {
        match self {
            Value::Text(text) => Some(text),
            Value::Owned(text) => Some(text),
            _ => None,
        }
    }

    /// Python `str(value)`.
    fn py_str(&self) -> String {
        match self {
            Value::Text(text) => (*text).to_owned(),
            Value::Owned(text) => text.clone(),
            Value::Integer(n) => n.to_string(),
            Value::Float(n) => crate::screen::py_float(*n),
            Value::Boolean(b) => if *b { "True" } else { "False" }.into(),
            Value::Json(value) => super::traverse::py_str(value),
        }
    }
}

/// Upstream's info dict for traversal: the serialized metadata plus the
/// source's prepared extras.
fn traversal_root(source: Source<'_>, options: &super::TemplateOptions) -> serde_json::Value {
    let insert = |mut value: serde_json::Value, extras: Vec<(&str, serde_json::Value)>| {
        if let Some(object) = value.as_object_mut() {
            for (key, extra) in extras {
                object.insert(key.into(), extra);
            }
        }
        value
    };
    let value = |info: &VideoInfo| crate::info_json::info_value(info).unwrap_or_default();
    let mut root = match source {
        Source::Json(value) | Source::JsonFile(value) | Source::JsonExec(value) => value.clone(),
        Source::Video(info) => value(info),
        Source::Print {
            info,
            filename,
            urls,
            filepath,
            prepared,
        } => insert(
            value(info),
            vec![
                ("filepath", filepath.into()),
                ("_filename", prepared.into()),
                ("filename", filename.ok().into()),
                ("urls", urls.into()),
                // `_forceprint`'s rendered tables.
                (
                    "formats_table",
                    super::formats::render_formats_table(info, !options.list_formats_old).into(),
                ),
                (
                    "thumbnails_table",
                    super::thumbnails::render_thumbnails_table(info).into(),
                ),
                (
                    "subtitles_table",
                    super::subtitles::render_subtitles_table(&info.subtitles).into(),
                ),
                (
                    "automatic_captions_table",
                    super::subtitles::render_subtitles_table(&info.automatic_captions).into(),
                ),
            ],
        ),
        Source::Exec {
            info,
            filepath,
            filename,
        } => insert(
            value(info),
            vec![
                ("filepath", filepath.into()),
                ("_filename", filename.into()),
            ],
        ),
        Source::Playlist {
            info,
            playlist_count,
            n_entries,
            ..
        } => insert(
            crate::info_json::to_shaped_value(info).unwrap_or_default(),
            vec![
                ("playlist_index", 0.into()),
                ("playlist_count", playlist_count.into()),
                ("n_entries", n_entries.into()),
            ],
        ),
    };
    // `prepare_outtmpl`: `info_dict.setdefault('epoch', int(time.time()))`.
    if let Some(object) = root.as_object_mut() {
        object.entry("epoch").or_insert_with(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_secs())
                .into()
        });
    }
    root
}

/// Every represented root field; playlists expose a subset.
const FIELDS: &[&str] = &[
    "id",
    "title",
    "ext",
    "extractor",
    "extractor_key",
    "format_id",
    "url",
    "webpage_url",
    "width",
    "height",
    "filesize",
    "playlist_index",
    "playlist_autonumber",
    "playlist_count",
    "n_entries",
    "__last_playlist_index",
    "direct",
    "description",
    "duration",
    "channel_id",
    "channel_url",
    "uploader_id",
    "uploader_url",
    "view_count",
    "age_limit",
    "playable_in_embed",
    "live_status",
    "media_type",
    "upload_date",
    "timestamp",
    "availability",
    "format",
    "resolution",
    "protocol",
    "vcodec",
    "acodec",
    "dynamic_range",
    "video_ext",
    "audio_ext",
    "duration_string",
    "thumbnail",
    "aspect_ratio",
    "fps",
    "tbr",
    "abr",
    "vbr",
    "asr",
    "audio_channels",
    "fulltitle",
];

/// Whether a represented video field has a value.
pub(crate) fn has_value(info: &VideoInfo, name: &str) -> bool {
    matches!(field(name, Source::Video(info)), Ok(Some(_)))
}

fn field<'a>(name: &str, source: Source<'a>) -> Result<Option<Value<'a>>> {
    use Value::*;
    let info = match source {
        // Dict sources are only traversed.
        Source::Json(_) | Source::JsonFile(_) | Source::JsonExec(_) => return Ok(None),
        Source::Video(info) => info,
        Source::Print {
            info,
            filename,
            urls,
            filepath,
            prepared,
        } => match name {
            "filepath" => return Ok(filepath.map(Text)),
            "_filename" => return Ok(prepared.map(Text)),
            "filename" => {
                return filename
                    .map(|filename| Some(Text(filename)))
                    .map_err(|error| invalid(format!("output filename failed: {error}")));
            }
            "urls" => return Ok(Some(Text(urls))),
            _ => info,
        },
        Source::Exec {
            info,
            filepath,
            filename,
        } => match name {
            "filepath" => return Ok(filepath.map(Text)),
            "_filename" => return Ok(filename.map(Text)),
            _ => info,
        },
        Source::Playlist {
            info,
            playlist_count,
            n_entries,
            ..
        } => {
            // Fields absent from upstream's playlist dict fall back to defaults,
            // while unknown names still fail as they do for videos.
            if !FIELDS.contains(&name) {
                return Err(invalid(format!("unknown or unsupported field {name:?}")));
            }
            return Ok(match name {
                "id" => Some(Text(&info.id)),
                "title" => Some(Text(&info.title)),
                "extractor" => Some(Text(&info.extractor)),
                "extractor_key" => Some(Text(&info.extractor_key)),
                "webpage_url" => Some(Text(info.webpage_url.as_str())),
                "playlist_index" => Some(Integer(0)),
                "playlist_count" => Some(Integer(playlist_count as u64)),
                "n_entries" => n_entries.map(|n| Integer(n as u64)),
                // Other playlist keys (such as the page's timestamp).
                _ => info
                    .extra
                    .get(name)
                    .filter(|value| !value.is_null())
                    .cloned()
                    .map(Value::from_json),
            });
        }
    };
    Ok(match name {
        "id" => Some(Text(&info.id)),
        "title" => Some(Text(&info.title)),
        "ext" => Some(Text(&info.ext)),
        "extractor" => Some(Text(&info.extractor)),
        "extractor_key" => Some(Text(&info.extractor_key)),
        "format_id" => info.format_id.as_deref().map(Text),
        // `_merge`'s dict has no URL.
        "url" if !info.requested_formats.is_empty() => None,
        "url" => Some(Text(info.url.as_str())),
        "webpage_url" => Some(Text(info.webpage_url.as_str())),
        "width" => info.width.map(|n| Integer(n.into())),
        "height" => info.height.map(|n| Integer(n.into())),
        "filesize" => info.filesize.map(Integer),
        "playlist_index" => info.playlist_index.map(|n| Integer(n as u64)),
        "playlist_autonumber" => info.playlist_autonumber.map(|n| Integer(n as u64)),
        "playlist_count" => info.playlist_count.map(|n| Integer(n as u64)),
        "n_entries" => info.n_entries.map(|n| Integer(n as u64)),
        "__last_playlist_index" => info.last_playlist_index.map(|n| Integer(n as u64)),
        // Only generic direct links have a `direct` field upstream.
        "direct" => info.direct.map(Boolean),
        "description" => info.metadata.description.as_deref().map(Text),
        "duration" => info.metadata.duration.map(|duration| match duration {
            crate::info::PyNumber::Int(value) => Integer(value.max(0) as u64),
            crate::info::PyNumber::Float(value) => Float(value),
        }),
        "channel_id" => info.metadata.channel_id.as_deref().map(Text),
        "channel_url" => info.metadata.channel_url.as_deref().map(Text),
        "uploader_id" => info.metadata.uploader_id.as_deref().map(Text),
        "uploader_url" => info.metadata.uploader_url.as_deref().map(Text),
        "view_count" => info.metadata.view_count.map(Integer),
        "age_limit" => info.metadata.age_limit.map(|n| Integer(n.into())),
        "playable_in_embed" => info.metadata.playable_in_embed.map(Boolean),
        "live_status" => info.metadata.live_status.as_deref().map(Text),
        "media_type" => info.metadata.media_type.as_deref().map(Text),
        "upload_date" => info.metadata.upload_date.as_deref().map(Text),
        // Negative timestamps are not representable by the typed renderer.
        "timestamp" => info
            .metadata
            .timestamp
            .and_then(|timestamp| match timestamp {
                crate::info::PyNumber::Int(value) => u64::try_from(value).ok().map(Integer),
                crate::info::PyNumber::Float(value) => Some(Float(value)),
            }),
        "availability" => info.metadata.availability.as_deref().map(Text),
        "format" => info.format.as_deref().map(Text),
        "resolution" => info.resolution.as_deref().map(Text),
        "protocol" => info.protocol.as_deref().map(Text),
        "vcodec" => info.vcodec.as_deref().map(Text),
        "acodec" => info.acodec.as_deref().map(Text),
        "dynamic_range" => info.dynamic_range.as_deref().map(Text),
        "video_ext" => info.video_ext.as_deref().map(Text),
        "audio_ext" => info.audio_ext.as_deref().map(Text),
        "duration_string" => info.metadata.duration_string.as_deref().map(Text),
        "fulltitle" => info.fulltitle.as_deref().map(Text),
        "thumbnail" => info.metadata.thumbnail.as_deref().map(Text),
        "aspect_ratio" => info.aspect_ratio.map(Float),
        "fps" => info.fps.map(|fps| match fps {
            crate::info::PyNumber::Int(n) if n >= 0 => Integer(n as u64),
            other => Float(other.as_f64()),
        }),
        "tbr" => info.tbr.map(Float),
        "abr" => info.abr.map(Float),
        "vbr" => info.vbr.map(Float),
        "asr" => info.asr.map(|n| Integer(n.into())),
        "audio_channels" => info.audio_channels.map(|n| Integer(n.into())),
        _ => return Err(invalid(format!("unknown or unsupported field {name:?}"))),
    })
}

fn digits(input: &str) -> Result<(Option<usize>, &str)> {
    let count = input.bytes().take_while(u8::is_ascii_digit).count();
    if count == 0 {
        return Ok((None, input));
    }
    let value = input[..count]
        .parse::<usize>()
        .map_err(|_| invalid("format width/precision is too large"))?;
    if value > MAX_FIELD {
        return Err(invalid("format width/precision exceeds 4096"));
    }
    Ok((Some(value), &input[count..]))
}

/// Upstream `STR_FORMAT_TYPES` plus its own `ljhqBUDS` conversions.
const CONVERSIONS: &str = "diouxXeEfFgGcrsaljhqBUDS";

fn format_spec(mut input: &str) -> Result<(Format, &str)> {
    let mut format = Format::default();
    loop {
        match input.as_bytes().first() {
            Some(b'#') => format.alternate = true,
            Some(b'0') => format.zero = true,
            Some(b'-') => format.left = true,
            Some(b'+') => format.plus = true,
            Some(b' ') => format.space = true,
            _ => break,
        }
        input = &input[1..];
    }
    let (width, rest) = digits(input)?;
    format.width = width.unwrap_or(0);
    input = rest;
    if let Some(rest) = input.strip_prefix('.') {
        let (precision, rest) = digits(rest)?;
        format.precision =
            Some(precision.ok_or_else(|| invalid("precision requires ASCII digits"))?);
        input = rest;
    }
    // A length modifier precedes a conversion; a bare `l` is the list one.
    if input.starts_with(['h', 'l', 'L']) && input[1..].starts_with(|c| CONVERSIONS.contains(c)) {
        input = &input[1..];
    }
    let Some(conversion) = input.chars().next() else {
        return Err(invalid("missing template conversion"));
    };
    if !CONVERSIONS.contains(conversion) {
        return Err(invalid(format!("unsupported conversion {conversion:?}")));
    }
    format.conversion = conversion;
    Ok((format, &input[conversion.len_utf8()..]))
}

fn padded(mut value: String, format: &Format, numeric: bool) -> String {
    let count = value.chars().count();
    if count >= format.width {
        return value;
    }
    let pad = format.width - count;
    if format.left {
        value.extend(std::iter::repeat_n(' ', pad));
        return value;
    }
    if numeric && format.zero {
        let sign = usize::from(value.starts_with(['+', '-', ' ']));
        value.insert_str(sign, &"0".repeat(pad));
        value
    } else {
        format!("{}{value}", " ".repeat(pad))
    }
}

fn text(value: &str, format: &Format) -> String {
    let value = match format.precision {
        Some(n) => value.chars().take(n).collect(),
        None => value.to_owned(),
    };
    padded(value, format, false)
}

fn number(value: &str) -> Result<Option<f64>> {
    if value.len() > MAX_OUTPUT {
        return Err(invalid("template metadata exceeds 64 KiB"));
    }
    let value = value.trim();
    if !value.contains('_') {
        return Ok(value.parse().ok());
    }
    let bytes = value.as_bytes();
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'_'
            && (index == 0
                || !bytes[index - 1].is_ascii_digit()
                || !bytes.get(index + 1).is_some_and(u8::is_ascii_digit))
        {
            return Ok(None);
        }
    }
    Ok(value.replace('_', "").parse().ok())
}

/// Python `%e` of a finite non-negative number: at least two exponent digits.
fn exponential(magnitude: f64, precision: usize, alternate: bool) -> String {
    let text = format!("{magnitude:.precision$e}");
    let (mantissa, exponent) = text.split_once('e').unwrap_or((&text, "0"));
    let exponent: i32 = exponent.parse().unwrap_or(0);
    let point = if alternate && precision == 0 { "." } else { "" };
    let sign = if exponent < 0 { '-' } else { '+' };
    format!("{mantissa}{point}e{sign}{:02}", exponent.abs())
}

/// Python `%g`: `%e` or `%f` by the rounded exponent, trailing zeros removed
/// unless `#`.
fn general(magnitude: f64, precision: usize, alternate: bool) -> String {
    let precision = precision.max(1);
    let exponent = if magnitude == 0.0 {
        0
    } else {
        let text = format!("{magnitude:.prec$e}", prec = precision - 1);
        text.split_once('e')
            .and_then(|(_, exponent)| exponent.parse::<i64>().ok())
            .unwrap_or(0)
    };
    let mut value = if -4 <= exponent && exponent < precision as i64 {
        let digits = (precision as i64 - 1 - exponent) as usize;
        format!("{magnitude:.digits$}")
    } else {
        exponential(magnitude, precision - 1, false)
    };
    if alternate {
        if !value.contains('.') {
            match value.find('e') {
                Some(index) => value.insert(index, '.'),
                None => value.push('.'),
            }
        }
        return value;
    }
    let (mantissa, exponent) = match value.find('e') {
        Some(index) => value.split_at(index),
        None => (value.as_str(), ""),
    };
    let mantissa = if mantissa.contains('.') {
        mantissa.trim_end_matches('0').trim_end_matches('.')
    } else {
        mantissa
    };
    value = format!("{mantissa}{exponent}");
    value
}

fn numeric(number: f64, format: &Format) -> Result<String> {
    let integer = matches!(format.conversion, 'd' | 'i' | 'u');
    if integer && !number.is_finite() {
        return Err(invalid("nonfinite integer template conversion"));
    }
    let negative =
        number.is_sign_negative() && !number.is_nan() && (!integer || number.trunc() != 0.0);
    let magnitude = number.abs();
    let mut value = if integer {
        let mut value = format!("{:.0}", magnitude.trunc());
        if let Some(precision) = format.precision
            && value.len() < precision
        {
            value = format!("{}{value}", "0".repeat(precision - value.len()));
        }
        value
    } else if magnitude.is_nan() {
        "nan".into()
    } else if magnitude.is_infinite() {
        "inf".into()
    } else if matches!(format.conversion, 'e' | 'E') {
        exponential(magnitude, format.precision.unwrap_or(6), format.alternate)
    } else if matches!(format.conversion, 'g' | 'G') {
        general(magnitude, format.precision.unwrap_or(6), format.alternate)
    } else {
        let precision = format.precision.unwrap_or(6);
        let mut value = format!("{magnitude:.precision$}");
        if format.alternate && precision == 0 {
            value.push('.');
        }
        value
    };
    if matches!(format.conversion, 'F' | 'E' | 'G') {
        value.make_ascii_uppercase();
    }
    if negative {
        value.insert(0, '-');
    } else if format.plus {
        value.insert(0, '+');
    } else if format.space {
        value.insert(0, ' ');
    }
    Ok(padded(value, format, true))
}

/// Upstream `INTERNAL_FORMAT_RE`, except its `(?<!\\)` lookbehind before an
/// alternate, which `parse_key` applies.
static INTERNAL_FORMAT: LazyLock<Regex> = LazyLock::new(|| {
    let inner = r"(?:\w+|-?\d+|(?:-?\d+)?(?::(?:-?\d+)?){1,2})";
    let sub = format!(r"\w*(?:\.{inner})*");
    let field = format!(r"\w*(?:\.(?:{inner}|\{{{sub}(?:,{sub})*\}}))*");
    Regex::new(&format!(
        r"(?s)^(?P<negate>-)?(?P<fields>{field})(?P<maths>(?:[+\-*](?:{field}|-?\d+(?:\.\d+)?))*)(?:>(?P<strf_format>.+?))?(?P<remaining>(?P<alternate>,[^|&)]+)?(?:&(?P<replacement>.*?))?(?:\|(?P<default>.*?))?)$"
    ))
    .expect("the template key pattern is valid")
});

/// Upstream `MATH_FIELD_RE` (field names first, so a number is a field).
static MATH_FIELD: LazyLock<Regex> = LazyLock::new(|| {
    let inner = r"(?:\w+|-?\d+|(?:-?\d+)?(?::(?:-?\d+)?){1,2})";
    let sub = format!(r"\w*(?:\.{inner})*");
    let field = format!(r"\w*(?:\.(?:{inner}|\{{{sub}(?:,{sub})*\}}))*");
    Regex::new(&format!(r"^(?:{field}|-?\d+(?:\.\d+)?)")).expect("the maths pattern is valid")
});

/// The `remaining` group of `INTERNAL_FORMAT_RE`.
static REMAINING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)^(?P<alternate>,[^|&)]+)?(?:&(?P<replacement>.*?))?(?:\|(?P<default>.*?))?$")
        .expect("the template remainder pattern is valid")
});

/// One `INTERNAL_FORMAT_RE` match: a field with its negation, maths and
/// strftime format, then the alternates, replacement and default text.
struct Key<'k> {
    negate: bool,
    fields: &'k str,
    maths: &'k str,
    strf: Option<&'k str>,
    remaining: &'k str,
    alternate: bool,
    replacement: Option<&'k str>,
    default: Option<&'k str>,
}

fn parse_key(key: &str) -> Option<Key<'_>> {
    let found = INTERNAL_FORMAT.captures(key)?;
    let text = |name: &str| found.name(name).map_or("", |m| m.as_str());
    let mut strf = found.name("strf_format").map(|m| (m.start(), m.end()));
    let mut tail = found.name("remaining").map_or(key.len(), |m| m.start());
    let mut rest = REMAINING.captures(&key[tail..])?;
    // `(?<!\\),`: an alternate cannot follow a backslash, so the lazy
    // strftime format extends past an escaped comma.
    if let Some((start, end)) = strf
        && rest.name("alternate").is_some()
        && key[..end].ends_with('\\')
    {
        let (end, captures) = (end + 1..=key.len())
            .filter(|end| key.is_char_boundary(*end))
            .find_map(|end| {
                let captures = REMAINING.captures(&key[end..])?;
                (captures.name("alternate").is_none() || !key[..end].ends_with('\\'))
                    .then_some((end, captures))
            })?;
        strf = Some((start, end));
        tail = end;
        rest = captures;
    }
    Some(Key {
        negate: found.name("negate").is_some(),
        fields: text("fields"),
        maths: text("maths"),
        strf: strf.map(|(start, end)| &key[start..end]),
        remaining: &key[tail..],
        alternate: rest.name("alternate").is_some(),
        replacement: rest.name("replacement").map(|m| m.as_str()),
        default: rest.name("default").map(|m| m.as_str()),
    })
}

/// Field lookups for one substitution: represented root fields, else
/// traversal of the (lazily serialized) info dict.
struct Lookup<'a, 'o> {
    info: Source<'a>,
    options: &'o super::TemplateOptions,
    root: Option<serde_json::Value>,
}

impl<'a> Lookup<'a, '_> {
    fn get(&mut self, name: &str) -> Result<Option<Value<'a>>> {
        let typed = !matches!(
            self.info,
            Source::Json(_) | Source::JsonFile(_) | Source::JsonExec(_)
        ) && super::traverse::is_plain(name)
            && (FIELDS.contains(&name)
                || (matches!(self.info, Source::Print { .. } | Source::Exec { .. })
                    && matches!(name, "filename" | "_filename" | "filepath" | "urls")));
        match name {
            "autonumber" => Ok(self.options.autonumber.get().map(Value::Integer)),
            "video_autonumber" => Ok(self.options.video_autonumber.get().map(Value::Integer)),
            _ if typed => Ok(match field(name, self.info)? {
                Some(value) => Some(value),
                // A represented field holding another type (an integer
                // `uploader_id`) keeps its value in `extra`.
                None => match self.info {
                    Source::Video(info)
                    | Source::Print { info, .. }
                    | Source::Exec { info, .. } => info
                        .extra
                        .get(name)
                        .filter(|value| !value.is_null())
                        .cloned()
                        .map(Value::from_json),
                    _ => None,
                },
            }),
            _ => {
                let (info, options) = (self.info, self.options);
                super::traverse::traverse(
                    self.root
                        .get_or_insert_with(|| traversal_root(info, options)),
                    name,
                )
                .map(|value| value.map(Value::from_json))
            }
        }
    }
}

/// Python `float_or_none`.
fn float_or_none(value: &Value<'_>) -> Result<Option<f64>> {
    Ok(match value {
        Value::Text(s) => number(s)?,
        Value::Owned(s) => number(s)?,
        Value::Integer(n) => Some(*n as f64),
        Value::Float(n) => Some(*n),
        Value::Boolean(b) => Some(f64::from(u8::from(*b))),
        Value::Json(serde_json::Value::Number(n)) => n.as_f64(),
        Value::Json(_) => None,
    })
}

/// Upstream `get_value`: traversal, negation, maths and datetime formatting.
fn get_value<'a>(
    key: &Key<'_>,
    lookup: &mut Lookup<'a, '_>,
    print: bool,
) -> Result<Option<Value<'a>>> {
    let mut value = lookup.get(key.fields)?;
    if key.negate {
        value = match value {
            Some(value) => float_or_none(&value)?.map(|n| Value::Float(-n)),
            None => None,
        };
    }
    if !key.maths.is_empty() {
        let mut current = match &value {
            Some(value) => float_or_none(value)?,
            None => None,
        };
        let mut rest = key.maths;
        while let Some(operator) = rest.chars().next() {
            rest = &rest[1..];
            let item = MATH_FIELD.find(rest).map_or("", |m| m.as_str());
            rest = &rest[item.len()..];
            if item.is_empty() {
                // Upstream fails with an `IndexError`.
                return Err(invalid(
                    "template maths needs a field or number after each operator",
                ));
            }
            let offset = match number(item)? {
                Some(offset) => Some(offset),
                None => match lookup.get(item)? {
                    Some(value) => float_or_none(&value)?,
                    None => None,
                },
            };
            // A missing value or offset is upstream's caught `TypeError`.
            let (Some(left), Some(offset)) = (current, offset) else {
                return Ok(None);
            };
            current = Some(match operator {
                '+' => left + offset,
                '-' => left - offset,
                _ => left * offset,
            });
        }
        value = current.map(Value::Float);
    }
    if let Some(format) = key.strf {
        use crate::dates::StrftimeInput;
        let input = match &value {
            Some(Value::Integer(n)) => i64::try_from(*n).ok().map(StrftimeInput::Integer),
            Some(Value::Float(n)) => Some(StrftimeInput::Float(*n)),
            Some(Value::Boolean(b)) => Some(StrftimeInput::Integer(i64::from(*b))),
            Some(Value::Text(s)) => Some(StrftimeInput::Text(s)),
            Some(Value::Owned(s)) => Some(StrftimeInput::Text(s)),
            Some(Value::Json(serde_json::Value::Number(n))) => match n.as_i64() {
                Some(n) => Some(StrftimeInput::Integer(n)),
                None => n.as_f64().map(StrftimeInput::Float),
            },
            Some(Value::Json(_)) | None => None,
        };
        value = match input {
            Some(input) => crate::dates::strftime_or_none(input, &format.replace("\\,", ","))
                .map_err(invalid)?
                .map(Value::Owned),
            None => None,
        };
    }
    // Filename sanitization turns empty strings into missing values.
    Ok(value.filter(|value| print || value.text() != Some("")))
}

/// Upstream's `_ReplacementFormatter` (`str.format` where every positional
/// field is the value). `Ok(Err(()))` is Python's `ValueError`; format specs
/// are not ported.
fn replacement(template: &str, value: &Value<'_>) -> Result<std::result::Result<String, ()>> {
    let mut output = String::new();
    let mut chars = template.chars().peekable();
    let mut automatic = None;
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                output.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                output.push('}');
            }
            '}' => return Ok(Err(())),
            '{' => {
                let mut field = String::new();
                let mut depth = 0;
                loop {
                    match chars.next() {
                        None => return Ok(Err(())),
                        Some('{') => {
                            depth += 1;
                            field.push('{');
                        }
                        Some('}') if depth == 0 => break,
                        Some('}') => {
                            depth -= 1;
                            field.push('}');
                        }
                        Some(c) => field.push(c),
                    }
                }
                let end = field.find(['!', ':']).unwrap_or(field.len());
                let (name, rest) = field.split_at(end);
                let (conversion, spec) = match rest.strip_prefix('!') {
                    Some(rest) => {
                        let mut rest = rest.chars();
                        let conversion = rest.next();
                        let rest = rest.as_str();
                        match rest.strip_prefix(':') {
                            Some(spec) => (conversion, spec),
                            None if rest.is_empty() => (conversion, ""),
                            None => return Ok(Err(())),
                        }
                    }
                    None => (None, rest.strip_prefix(':').unwrap_or("")),
                };
                let manual = !name.is_empty();
                if automatic.is_some_and(|automatic: bool| automatic == manual)
                    || (manual && !name.chars().all(|c| c.is_ascii_digit()))
                {
                    return Ok(Err(()));
                }
                automatic = Some(!manual);
                let text = match conversion {
                    None | Some('s') => value.py_str(),
                    Some('r') => py_repr(value),
                    Some('a') => py_ascii(&py_repr(value)),
                    Some(_) => return Ok(Err(())),
                };
                if !spec.is_empty() {
                    return Err(invalid(
                        "template replacement format specs are not supported natively",
                    ));
                }
                output.push_str(&text);
            }
            c => output.push(c),
        }
    }
    Ok(Ok(output))
}

/// Python `repr` of a template value.
fn py_repr(value: &Value<'_>) -> String {
    match value {
        Value::Text(text) => super::traverse::py_repr_str(text),
        Value::Owned(text) => super::traverse::py_repr_str(text),
        Value::Json(value) => super::traverse::py_repr(value),
        other => other.py_str(),
    }
}

/// Python `ascii`: `repr` with non-ASCII characters escaped.
fn py_ascii(repr: &str) -> String {
    let mut output = String::with_capacity(repr.len());
    for c in repr.chars() {
        match c as u32 {
            0..0x80 => output.push(c),
            n @ 0x80..0x100 => output.push_str(&format!("\\x{n:02x}")),
            n @ 0x100..0x10000 => output.push_str(&format!("\\u{n:04x}")),
            n => output.push_str(&format!("\\U{n:08x}")),
        }
    }
    output
}

/// Python truthiness.
fn truthy(value: &Value<'_>) -> bool {
    match value {
        Value::Text(text) => !text.is_empty(),
        Value::Owned(text) => !text.is_empty(),
        Value::Integer(n) => *n != 0,
        Value::Float(n) => *n != 0.0,
        Value::Boolean(b) => *b,
        Value::Json(value) => match value {
            serde_json::Value::Null => false,
            serde_json::Value::Bool(b) => *b,
            serde_json::Value::Number(n) => n.as_f64() != Some(0.0),
            serde_json::Value::String(text) => !text.is_empty(),
            serde_json::Value::Array(items) => !items.is_empty(),
            serde_json::Value::Object(map) => !map.is_empty(),
        },
    }
}

/// `bytes` `%s` formatting (width and precision count bytes), decoded as
/// UTF-8 with `errors='ignore'`.
fn bytes_format(value: &str, format: &Format) -> String {
    let mut bytes = value.as_bytes().to_vec();
    if let Some(precision) = format.precision {
        bytes.truncate(precision);
    }
    if bytes.len() < format.width {
        let pad = vec![b' '; format.width - bytes.len()];
        if format.left {
            bytes.extend(pad);
        } else {
            bytes.splice(0..0, pad);
        }
    }
    let mut output = String::new();
    let mut rest = &bytes[..];
    loop {
        match std::str::from_utf8(rest) {
            Ok(text) => {
                output.push_str(text);
                return output;
            }
            Err(error) => {
                let (valid, invalid) = rest.split_at(error.valid_up_to());
                output.push_str(std::str::from_utf8(valid).unwrap_or_default());
                rest = &invalid[error.error_len().unwrap_or(invalid.len())..];
            }
        }
    }
}

/// Upstream `format_decimal_suffix(value, '%{num_fmt}f%s' or '%d%s',
/// factor=1024 if '#' else 1000)`; `None` (negative or non-numeric) is
/// formatted as `'None'`.
fn decimal_suffix(value: &Value<'_>, num_fmt: &str, format: &Format) -> Result<String> {
    let factor: f64 = if format.alternate { 1024.0 } else { 1000.0 };
    let Some(number) = float_or_none(value)?.filter(|n| n.is_nan() || *n >= 0.0) else {
        return Ok("None".into());
    };
    let exponent = if number == 0.0 {
        0
    } else {
        let log = number.ln() / factor.ln();
        if !log.is_finite() {
            return Err(invalid("decimal suffix of a non-finite number"));
        }
        (log.trunc() as i64).min(8)
    };
    const SUFFIXES: [&str; 9] = ["", "k", "M", "G", "T", "P", "E", "Z", "Y"];
    // Python list indexing, including negative exponents for tiny numbers.
    let index = if exponent < 0 { 9 + exponent } else { exponent };
    let Some(suffix) = usize::try_from(index).ok().and_then(|i| SUFFIXES.get(i)) else {
        return Err(invalid("decimal suffix index out of range"));
    };
    let suffix = match (factor == 1024.0, *suffix) {
        (false, suffix) | (true, suffix @ "") => suffix.to_owned(),
        (true, "k") => "Ki".to_owned(),
        (true, suffix) => format!("{suffix}i"),
    };
    let converted = number / factor.powf(exponent as f64);
    let number = if num_fmt.is_empty() {
        format!("{:.0}", converted.trunc())
    } else {
        let format = Format {
            alternate: false,
            conversion: 'f',
            ..*format
        };
        numeric(converted, &format)?
    };
    Ok(format!("{number}{suffix}"))
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn substitution(
    key: &str,
    spec: &str,
    info: Source<'_>,
    options: &super::TemplateOptions,
) -> Result<(String, usize)> {
    let (mut format, remaining) = format_spec(spec)?;
    let consumed = spec.len() - remaining.len();
    let print = info.print();
    let mut lookup = Lookup {
        info,
        options,
        root: None,
    };
    // Upstream `create_key`: alternates are tried while values are missing.
    let mut default = None;
    let mut value = None;
    let mut last_field = "";
    let mut replacement_text = None;
    let mut rest = key;
    while let Some(parsed) = parse_key(rest) {
        if parsed.default.is_some() {
            default = parsed.default;
        }
        value = get_value(&parsed, &mut lookup, print)?;
        last_field = parsed.fields;
        replacement_text = parsed.replacement;
        if value.is_none() && parsed.alternate {
            rest = &parsed.remaining[1..];
        } else {
            break;
        }
    }
    if let (Some(current), Some(template)) = (&value, replacement_text) {
        match replacement(template, current)? {
            Ok(text) => value = Some(Value::Owned(text)),
            Err(()) => (value, default) = (None, None),
        }
    }
    let clean = |text: &str| -> Result<String> {
        if print {
            bounded(text).map(str::to_owned)
        } else {
            sanitize(last_field, text, options)
        }
    };
    let Some(value) = value else {
        return Ok((clean(default.unwrap_or(&options.na_placeholder))?, consumed));
    };
    if &spec[..consumed] == "s" && matches!(value, Value::Integer(_) | Value::Boolean(_)) {
        let size = match last_field {
            "playlist_index" => match info {
                Source::Video(info) | Source::Print { info, .. } | Source::Exec { info, .. } => {
                    Some(info.last_playlist_index.unwrap_or(0))
                }
                Source::Playlist {
                    last_playlist_index,
                    ..
                } => Some(last_playlist_index),
                Source::Json(_) | Source::JsonFile(_) | Source::JsonExec(_) => None,
            },
            "playlist_autonumber" => match info {
                Source::Video(info) | Source::Print { info, .. } | Source::Exec { info, .. } => {
                    Some(info.n_entries.unwrap_or(0))
                }
                Source::Playlist { .. }
                | Source::Json(_)
                | Source::JsonFile(_)
                | Source::JsonExec(_) => None,
            },
            "autonumber" => Some(options.autonumber_size),
            _ => None,
        };
        if let Some(size) = size {
            format.width = if last_field == "autonumber" {
                size
            } else {
                size.to_string().len()
            };
            format.zero = true;
            format.conversion = 'd';
        }
    }
    let result = match format.conversion {
        'q' => {
            // `shell_quote(map(str, variadic(value) if '#' else [value]))`.
            let items = match &value {
                Value::Json(serde_json::Value::Array(items)) if format.alternate => {
                    items.iter().map(super::traverse::py_str).collect()
                }
                other => vec![other.py_str()],
            };
            let quoted = items
                .iter()
                .map(|item| bounded(item).map(super::exec::shell_quote))
                .collect::<Result<Vec<_>>>()?
                .join(" ");
            text(&clean(&quoted)?, &format)
        }
        'j' => {
            let value = match value {
                Value::Text(s) => python_json_string(bounded(s)?, !format.plus),
                Value::Owned(s) => python_json_string(bounded(&s)?, !format.plus),
                Value::Integer(n) => n.to_string(),
                Value::Float(n) => crate::screen::py_float(n),
                Value::Boolean(b) => b.to_string(),
                Value::Json(value) if format.alternate => {
                    super::traverse::py_json_indented(&value, !format.plus)
                }
                Value::Json(value) => super::traverse::py_json(&value, !format.plus),
            };
            text(&clean(&value)?, &format)
        }
        'l' => {
            // `', '.join(map(str, variadic(value)))` (newlines with `#`);
            // dicts iterate over their keys.
            let separator = if format.alternate { "\n" } else { ", " };
            let value = match value {
                Value::Json(serde_json::Value::Array(items)) => items
                    .iter()
                    .map(super::traverse::py_str)
                    .collect::<Vec<_>>()
                    .join(separator),
                Value::Json(serde_json::Value::Object(map)) => {
                    map.keys().cloned().collect::<Vec<_>>().join(separator)
                }
                other => other.py_str(),
            };
            text(&clean(&value)?, &format)
        }
        'h' => text(&clean(&escape_html(bounded(&value.py_str())?))?, &format),
        'B' => clean(&bytes_format(bounded(&value.py_str())?, &format))?,
        'U' => {
            let Some(source) = value.text() else {
                return Err(invalid("%()U needs a string value"));
            };
            let source = bounded(source)?;
            use icu_normalizer::{ComposingNormalizerBorrowed, DecomposingNormalizerBorrowed};
            let normalized = match (format.plus, format.alternate) {
                (false, false) => ComposingNormalizerBorrowed::new_nfc().normalize(source),
                (true, false) => ComposingNormalizerBorrowed::new_nfkc().normalize(source),
                (false, true) => DecomposingNormalizerBorrowed::new_nfd().normalize(source),
                (true, true) => DecomposingNormalizerBorrowed::new_nfkd().normalize(source),
            };
            text(&clean(&normalized)?, &format)
        }
        'D' => {
            let num_fmt = spec[..consumed - 1].replace('#', "");
            clean(&decimal_suffix(&value, &num_fmt, &format)?)?
        }
        'S' => {
            let sanitized = super::sanitize::filename_sanitizer(
                last_field,
                bounded(&value.py_str())?,
                format.alternate,
                options.legacy_sanitization,
            );
            text(&clean(&sanitized)?, &format)
        }
        'c' if truthy(&value) => {
            let first: String = value.py_str().chars().take(1).collect();
            let first = clean(&first)?;
            if first.chars().count() != 1 {
                return Err(invalid("%c requires an int or a unicode character"));
            }
            text(
                &first,
                &Format {
                    precision: None,
                    ..format
                },
            )
        }
        'r' => text(&clean(&py_repr(&value))?, &format),
        'a' => text(&clean(&py_ascii(&py_repr(&value)))?, &format),
        'c' | 's' => text(&clean(bounded(&value.py_str())?)?, &format),
        conversion => match float_or_none(&value)? {
            None => clean(default.unwrap_or(&options.na_placeholder))?,
            Some(_) if matches!(conversion, 'o' | 'x' | 'X') => {
                // Upstream passes a float, which Python rejects.
                return Err(invalid(format!(
                    "%{conversion} requires an integer, not float"
                )));
            }
            Some(number) => numeric(number, &format)?,
        },
    };
    Ok((result, consumed))
}

/// Native containment guard: metadata cannot form a `.`/`..` (or, for Windows,
/// dot/space-only) path component that is not literally in the template.
/// Substitutions cannot contain `/`, so components align with the template's.
fn guard_components(output: &str, template: &str) -> String {
    let literal: Vec<&str> = template.split('/').collect();
    output
        .split('/')
        .enumerate()
        .map(|(index, component)| {
            let alias = !component.is_empty() && component.trim_matches([' ', '.']).is_empty();
            if alias && literal.get(index) != Some(&component) {
                "_"
            } else {
                component
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn bounded(value: &str) -> Result<&str> {
    if value.len() > MAX_OUTPUT {
        return Err(invalid("template metadata/default exceeds 64 KiB"));
    }
    Ok(value)
}

/// Upstream `sanitize(key, value)` for filename templates.
fn sanitize(key: &str, value: &str, options: &super::TemplateOptions) -> Result<String> {
    let value = bounded(value)?;
    Ok(match options.filename_mode {
        super::FilenameMode::Minimal => value.replace('/', "\u{29f8}").replace('\0', ""),
        mode => super::sanitize::filename_sanitizer(
            key,
            value,
            mode == super::FilenameMode::Restricted,
            options.legacy_sanitization,
        ),
    })
}

/// `_prepare_filename` `trim_file_name`: keep up to two dot-separated
/// extensions and slice the remaining text like Python `no_ext[:length]`.
fn trim_file_name(filename: &str, length: i64) -> String {
    if length == 0 {
        return filename.to_owned();
    }
    let mut parts: Vec<&str> = filename.rsplitn(3, '.').collect();
    parts.reverse();
    let (no_ext, extensions) = parts.split_first().unwrap();
    let count = no_ext.chars().count() as i64;
    let keep = if length < 0 {
        (count + length).max(0)
    } else {
        length.min(count)
    };
    let trimmed: String = no_ext.chars().take(keep as usize).collect();
    std::iter::once(trimmed.as_str())
        .chain(extensions.iter().copied())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(".")
}

/// Python `json.dumps` for a string, with or without `ensure_ascii`.
pub(super) fn python_json_string(value: &str, ensure_ascii: bool) -> String {
    let mut output = String::with_capacity(value.len() + 2);
    output.push('"');
    for c in value.chars() {
        match c {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            '\u{8}' => output.push_str("\\b"),
            '\u{c}' => output.push_str("\\f"),
            c if c < ' ' || (ensure_ascii && c > '~') => {
                let mut units = [0; 2];
                for unit in c.encode_utf16(&mut units) {
                    output.push_str(&format!("\\u{unit:04x}"));
                }
            }
            c => output.push(c),
        }
    }
    output.push('"');
    output
}

fn append(output: &mut String, value: &str) -> Result<()> {
    if output.len().saturating_add(value.len()) > MAX_OUTPUT {
        return Err(invalid("expanded output template exceeds 64 KiB"));
    }
    output.push_str(value);
    Ok(())
}

/// Render an explicit filename template (upstream `prepare_filename(outtmpl=...)`).
pub(super) fn render(
    template: &str,
    info: Source<'_>,
    options: &super::TemplateOptions,
) -> Result<PathBuf> {
    render_kind(template, info, options, None)
}

/// Upstream `prepare_filename`: `kind` is an output type (`None` for media or
/// an explicit template). Expands `~`/`$VAR` in the template, renders sanitized
/// metadata, forces the type extension, trims, guards metadata path components,
/// joins `--paths` and optionally forces Windows path sanitization.
pub(super) fn render_kind(
    template: &str,
    info: Source<'_>,
    options: &super::TemplateOptions,
    kind: Option<&str>,
) -> Result<PathBuf> {
    let template = crate::batch::expand_path_text(template);
    let mut output = expand(&template, info, options)?;
    // Media `-` is stdout (the caller handles it); typed files derive from it.
    if output.is_empty() || (output == "-" && kind.is_none()) {
        return Err(invalid("empty paths and stdout output are unsupported"));
    }
    if let Some(ext) = kind.and_then(super::filename::forced_extension) {
        let real = match info {
            Source::Video(info) | Source::Print { info, .. } | Source::Exec { info, .. } => {
                Some(info.ext.as_str())
            }
            Source::Playlist { .. }
            | Source::Json(_)
            | Source::JsonFile(_)
            | Source::JsonExec(_) => None,
        };
        output = super::filename::replace_extension(&output, ext, real);
    }
    let output = guard_components(&trim_file_name(&output, options.trim_file_name), &template);
    if output.is_empty() {
        return Err(invalid("empty paths and stdout output are unsupported"));
    }
    let output = super::filename::output_path(options, kind, &output);
    // Like upstream get_output_path, forced Windows names sanitize the whole path.
    let output = if options.windows_filenames {
        super::sanitize::force_sanitize_path(&output)
    } else {
        output
    };
    Ok(output.into())
}

/// Expand a template to text: unsanitized for `Source::Print`, otherwise
/// with filename-component sanitization.
pub(super) fn expand(
    template: &str,
    info: Source<'_>,
    options: &super::TemplateOptions,
) -> Result<String> {
    if template.len() > MAX_SPEC {
        return Err(invalid("output template exceeds 16 KiB"));
    }
    let mut output = String::new();
    let mut rest = template;
    let mut tokens = 0;
    while let Some(index) = rest.find('%') {
        append(&mut output, &rest[..index])?;
        rest = &rest[index + 1..];
        tokens += 1;
        if tokens > 256 {
            return Err(invalid("output template exceeds 256 tokens"));
        }
        if let Some(tail) = rest.strip_prefix('%') {
            append(&mut output, "%")?;
            rest = tail;
            continue;
        }
        let Some((key, spec)) = rest.strip_prefix('(').and_then(|s| s.split_once(')')) else {
            // `escape_outtmpl` keeps other `%` sequences literal in commands.
            if info.exec() {
                append(&mut output, "%")?;
                continue;
            }
            return Err(invalid("malformed output template token"));
        };
        let (value, consumed) = substitution(key, spec, info, options)?;
        append(&mut output, &value)?;
        rest = &spec[consumed..];
    }
    append(&mut output, rest)?;
    Ok(output)
}

/// Each keyed substitution's conversion character and `|default`, in
/// template order, for `--exec` safety validation. Other `%` sequences are
/// literal, as in command rendering.
pub(super) fn exec_fields(template: &str) -> Vec<(Option<char>, Option<&str>)> {
    let mut fields = Vec::new();
    let mut rest = template;
    while let Some(index) = rest.find('%') {
        rest = &rest[index + 1..];
        if let Some(tail) = rest.strip_prefix('%') {
            rest = tail;
            continue;
        }
        let Some((key, spec)) = rest.strip_prefix('(').and_then(|s| s.split_once(')')) else {
            continue;
        };
        let spec = spec.trim_start_matches(|c: char| "-0123456789.*# +".contains(c));
        let spec = spec.strip_prefix(['h', 'l', 'L']).unwrap_or(spec);
        let conversion = spec.chars().next();
        // The default after trying every alternate of a missing value.
        let mut default = None;
        let mut alternate = key;
        while let Some(parsed) = parse_key(alternate) {
            default = parsed.default.or(default);
            if !parsed.alternate {
                break;
            }
            alternate = &parsed.remaining[1..];
        }
        fields.push((conversion, default));
        rest = &spec[conversion.map_or(0, char::len_utf8)..];
    }
    fields
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn info() -> VideoInfo {
        serde_json::from_value(json!({"id":"42","title":"Épisode 影像","ext":"mp4","extractor":"native","extractor_key":"Native","url":"https://example.com/m.mp4","webpage_url":"https://example.com/watch","direct":true,"filesize":9007199254740993_u64,"width":1280,"height":720,"playlist_index":3,"playlist_autonumber":2,"n_entries":120,"__last_playlist_index":1000})).unwrap()
    }
    fn output(template: &str, info: &VideoInfo) -> String {
        render(
            template,
            Source::Video(info),
            &super::super::TemplateOptions::default(),
        )
        .unwrap()
        .to_string_lossy()
        .into_owned()
    }

    #[test]
    fn typed_values_flags_precision_and_automatic_padding() {
        let info = info();
        assert_eq!(
            output("%(width)+08d-%(height)-6i.%(ext)s", &info),
            "+0001280-720   .mp4"
        );
        assert_eq!(output("%(id)#08.2f-%(height) .0F", &info), "00042.00- 720");
        assert_eq!(
            output("%(filesize)s-%(filesize)d", &info),
            "9007199254740993-9007199254740992"
        );
        assert_eq!(
            output("%(title)12.4s-%(direct)s-%(direct)u", &info),
            "        Épis-True-1"
        );
        assert_eq!(
            output(
                "%(playlist_index)s-%(playlist_autonumber)s-%(playlist_index)1s",
                &info
            ),
            "0003-002-3"
        );
        assert_eq!(output("%(height)#.0f-%(width)ld", &info), "720.-1280");
    }

    #[test]
    fn all_alternates_validate_missing_and_numeric_failures_discard_formatting() {
        let mut info = info();
        info.title.clear();
        assert_eq!(output("%(title,format_id,id|missing)s", &info), "42");
        assert_eq!(output("%(format_id)020.9d", &info), "NA");
        assert_eq!(output("%(extractor|fallback)020d", &info), "fallback");
        assert_eq!(output("%(format_id|)04s-suffix", &info), "-suffix");
        // Unknown fields are missing values, like upstream.
        assert_eq!(output("%(unsupported,id)s", &info), "42");
        assert_eq!(output("%(format_id,playlist_index)s", &info), "0003");
        let options = super::super::TemplateOptions {
            na_placeholder: String::new(),
            ..Default::default()
        };
        assert_eq!(
            render("%(format_id)s-file", Source::Video(&info), &options).unwrap(),
            PathBuf::from("-file")
        );
    }

    #[test]
    fn metadata_and_fallback_are_sanitized_without_reparsing_or_creating_directories() {
        let mut info = info();
        info.title = "../../bad\\name\n%(id)s".into();
        assert_eq!(
            output("directory/%(title)s.%(ext)s", &info),
            "directory/..⧸..⧸bad⧹name %(id)s.mp4"
        );
        assert_eq!(output("%(format_id|../../bad)s", &info), "..⧸..⧸bad");
        info.title = ". .x".into();
        assert_eq!(output("%(title).2s-file", &info), ". -file");
        assert_eq!(output("%(title).2s/%(title).1s/./x", &info), "_/_/./x");
        assert_eq!(output("%(title).0s-file", &info), "-file");
    }

    #[test]
    fn filename_modes_trimming_and_forced_windows_paths() {
        use super::super::{FilenameMode, TemplateOptions};
        let mut info = info();
        info.title = "Épisode: a/b <c>".into();
        let render_with = |template: &str, options: &TemplateOptions| {
            render(template, Source::Video(&info), options)
                .unwrap()
                .to_string_lossy()
                .into_owned()
        };
        let mut options = TemplateOptions {
            filename_mode: FilenameMode::Restricted,
            ..Default::default()
        };
        assert_eq!(
            render_with("%(title)s.%(ext)s", &options),
            "Episode_-_a_b_c.mp4"
        );
        options.filename_mode = FilenameMode::Minimal;
        assert_eq!(
            render_with("%(title)s.%(ext)s", &options),
            "Épisode: a⧸b <c>.mp4"
        );
        options.windows_filenames = true;
        assert_eq!(
            render_with("x/./%(title)s.%(ext)s", &options),
            "x/Épisode# a⧸b #c#.mp4"
        );
        options = TemplateOptions {
            trim_file_name: 3,
            ..Default::default()
        };
        assert_eq!(
            render_with("%(title)s.%(id)s.%(ext)s", &options),
            "Épi.42.mp4"
        );
        options.trim_file_name = -2;
        assert_eq!(render_with("abcd.%(ext)s", &options), "ab.mp4");
        options.trim_file_name = 2;
        assert_eq!(render_with("abcd", &options), "ab");
        assert_eq!(
            super::super::default_template(true),
            "%(title)s-[%(id)s].%(ext)s"
        );
    }

    #[test]
    fn nonfinite_integer_errors_float_strings_and_negative_zero() {
        let mut info = info();
        info.title = "-nan".into();
        assert_eq!(output("%(title)+F", &info), "+NAN");
        assert!(
            render(
                "%(title)d",
                Source::Video(&info),
                &super::super::TemplateOptions::default()
            )
            .is_err()
        );
        info.title = "-0.1".into();
        assert_eq!(output("%(title)d-%(title).1f", &info), "0--0.1");
        info.title = " 1_000.25 ".into();
        assert_eq!(output("%(title).2f", &info), "1000.25");
        info.title = "1__0".into();
        assert_eq!(output("%(title)010d", &info), "NA");
        info.title = "inf".into();
        assert_eq!(output("%(title)08f", &info), "00000inf");
    }

    #[test]
    fn saved_pinned_template_cases_match_actual_upstream() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/templates/cases.json"))
                .unwrap();
        let mut failures = Vec::new();
        for case in fixture["cases"].as_array().unwrap() {
            let info: VideoInfo = serde_json::from_value(case["info"].clone()).unwrap();
            let options = super::super::TemplateOptions {
                na_placeholder: case["na_placeholder"].as_str().unwrap().into(),
                legacy_sanitization: case["compat_opts"]
                    .as_array()
                    .is_some_and(|options| options.iter().any(|o| o == "filename-sanitization")),
                ..Default::default()
            };
            match render(
                case["template"].as_str().unwrap(),
                Source::Video(&info),
                &options,
            ) {
                Ok(actual) if actual.to_string_lossy() == case["expected"].as_str().unwrap() => {}
                actual => failures.push(format!(
                    "{}: {actual:?}, expected {}",
                    case["name"], case["expected"]
                )),
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn malformed_unsupported_and_resource_limits_fail_closed() {
        let info = info();
        let options = super::super::TemplateOptions::default();
        for template in [
            "",
            "-",
            "%",
            "%s",
            "%(id)",
            "%(id).s",
            "%(id).d",
            "%(id)4097s",
            "%(id).4097f",
            "%(id)999999999999999999999999999999d",
            "%(id)x",
            "%(id+)s",
        ] {
            assert!(
                render(template, Source::Video(&info), &options).is_err(),
                "{template}"
            );
        }
        assert_eq!(output("%%-%(id)s", &info), "%-42");
        // Upstream's key grammar: maths makes floats, and keys that do not
        // match `INTERNAL_FORMAT_RE` are missing values.
        assert_eq!(
            output("%(id+1)s-%(id,)s-%(id.a-b)s-%(id.{x)s-%(id)q", &info),
            "43.0-NA-NA-NA-42"
        );
        // `l` is a length modifier before a conversion, else the list one.
        assert_eq!(output("%(id)lls-%(id)ls-%(id)l", &info), "42s-42-42");
        assert!(render(&"x".repeat(MAX_SPEC + 1), Source::Video(&info), &options).is_err());
        assert!(render(&"%%".repeat(257), Source::Video(&info), &options).is_err());
        assert!(render(&"%(width)4096s".repeat(17), Source::Video(&info), &options).is_err());
        let mut huge = info.clone();
        huge.title = "x".repeat(MAX_OUTPUT + 1);
        assert!(render("%(title).1s", Source::Video(&huge), &options).is_err());
    }
}

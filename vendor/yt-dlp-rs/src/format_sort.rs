//! Upstream `FormatSorter`: format preference keys from the default field
//! order, extractor fields and user `-S` strings.
use std::{cmp::Ordering, collections::BTreeMap};

/// A format field value as the sorter sees it.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Num(f64),
    Str(String),
}

/// Format fields by upstream name.
pub type Fields = BTreeMap<String, Value>;

/// One element of a preference tuple.
#[derive(Debug, Clone, PartialEq)]
pub enum Key {
    Num(f64),
    Str(String),
}

impl Key {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Key::Num(a), Key::Num(b)) => a.partial_cmp(b).unwrap_or(Ordering::Equal),
            (Key::Str(a), Key::Str(b)) => a.cmp(b),
            // Categories (the first tuple element) keep types apart.
            (Key::Num(_), Key::Str(_)) => Ordering::Less,
            (Key::Str(_), Key::Num(_)) => Ordering::Greater,
        }
    }
}

/// Lexicographic comparison of two preference keys.
pub fn compare(a: &[Vec<Key>], b: &[Vec<Key>]) -> Ordering {
    for (x, y) in a.iter().zip(b) {
        for (p, q) in x.iter().zip(y) {
            let ordering = p.cmp(q);
            if ordering != Ordering::Equal {
                return ordering;
            }
        }
        let ordering = x.len().cmp(&y.len());
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    a.len().cmp(&b.len())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Type {
    Field,
    Ordered,
    Extractor,
    Boolean,
    Multiple,
    Combined,
    Alias,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Convert {
    Order,
    FloatString,
    String,
    FloatNone,
    Float,
    Bytes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Function {
    AnyNotNone,
    FirstTruthy,
    MinTruthy,
}

#[derive(Debug, Clone)]
struct Setting {
    kind: Type,
    /// Source fields (one for most, several for `multiple`/`combined`).
    fields: Vec<&'static str>,
    convert: Convert,
    regex: bool,
    order: &'static [Option<&'static str>],
    order_free: Option<&'static [Option<&'static str>]>,
    default: Option<f64>,
    max: Option<f64>,
    forced: bool,
    priority: bool,
    not_in_list: &'static [Option<&'static str>],
    function: Option<Function>,
    reverse: bool,
    closest: bool,
    limit: Option<Value>,
    /// The limit text and Python `str` of its resolved value, for
    /// `print_verbose_info`.
    limit_text: Option<(String, String)>,
    visible: bool,
}

impl Setting {
    fn new(kind: Type, name: &'static str) -> Self {
        let fields = match kind {
            Type::Extractor => vec!["preference"],
            _ => vec![name],
        };
        Self {
            kind,
            fields,
            convert: match kind {
                Type::Ordered => Convert::Order,
                _ => Convert::FloatString,
            },
            regex: false,
            order: &[],
            order_free: None,
            default: None,
            max: None,
            forced: false,
            priority: false,
            not_in_list: &[None],
            function: None,
            reverse: false,
            closest: false,
            limit: None,
            limit_text: None,
            visible: true,
        }
    }
}

const VCODEC: &[Option<&str>] = &[
    Some("av0?1"),
    Some(r"vp0?9\.0?2"),
    Some("vp0?9"),
    Some("[hx]265|he?vc?"),
    Some("[hx]264|avc"),
    Some("vp0?8"),
    Some("mp4v|h263"),
    Some("theora"),
    Some(""),
    None,
    Some("none"),
];
const ACODEC: &[Option<&str>] = &[
    Some("[af]lac"),
    Some("wav|aiff"),
    Some("opus"),
    Some("vorbis|ogg"),
    Some("aac"),
    Some("mp?4a?"),
    Some("mp3"),
    Some("ac-?4"),
    Some("e-?a?c-?3"),
    Some("ac-?3"),
    Some("dts"),
    Some(""),
    None,
    Some("none"),
];
const HDR: &[Option<&str>] = &[
    Some("dv"),
    Some("(hdr)?12"),
    Some(r"(hdr)?10\+"),
    Some("(hdr)?10"),
    Some("hlg"),
    Some(""),
    Some("sdr"),
    None,
];
const PROTO: &[Option<&str>] = &[
    Some("(ht|f)tps"),
    Some("(ht|f)tp$"),
    Some("m3u8.*"),
    Some(".*dash"),
    Some("websocket_frag"),
    Some("rtmpe?"),
    Some(""),
    Some("ws|websocket"),
    Some("f4"),
];
const VEXT: &[Option<&str>] = &[
    Some("mp4"),
    Some("mov"),
    Some("webm"),
    Some("flv"),
    Some(""),
    Some("none"),
];
const VEXT_FREE: &[Option<&str>] = &[
    Some("webm"),
    Some("mp4"),
    Some("mov"),
    Some("flv"),
    Some(""),
    Some("none"),
];
const AEXT: &[Option<&str>] = &[
    Some("m4a"),
    Some("aac"),
    Some("mp3"),
    Some("ogg"),
    Some("opus"),
    Some("web[am]"),
    Some(""),
    Some("none"),
];
const AEXT_FREE: &[Option<&str>] = &[
    Some("ogg"),
    Some("opus"),
    Some("web[am]"),
    Some("mp3"),
    Some("m4a"),
    Some("aac"),
    Some(""),
    Some("none"),
];

/// `FormatSorter.default`.
pub const DEFAULT: &[&str] = &[
    "hidden",
    "aud_or_vid",
    "hasvid",
    "ie_pref",
    "lang",
    "quality",
    "res",
    "fps",
    "hdr:12",
    "vcodec",
    "channels",
    "acodec",
    "size",
    "br",
    "asr",
    "proto",
    "ext",
    "hasaud",
    "source",
    "id",
];

/// `FormatSorter._prefer_vp9_sort`.
const PREFER_VP9: &[&str] = &[
    "hidden",
    "aud_or_vid",
    "hasvid",
    "ie_pref",
    "lang",
    "quality",
    "res",
    "fps",
    "hdr:12",
    "vcodec:vp9.2",
    "channels",
    "acodec",
    "size",
    "br",
    "asr",
    "proto",
    "ext",
    "hasaud",
    "source",
    "id",
];

/// `--compat-options prefer-vp9-sort`: `FormatSorter.default` becomes
/// `_prefer_vp9_sort`.
pub static PREFER_VP9_SORT: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Whether `prefer-vp9-sort` is among the compatibility options at all
/// (YouTube's `_format_sort_fields` then prefer `vcodec:vp9.2`).
pub static PREFER_VP9_COMPAT: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// The current `FormatSorter.default`.
fn default_fields() -> &'static [&'static str] {
    if PREFER_VP9_SORT.load(std::sync::atomic::Ordering::Relaxed) {
        PREFER_VP9
    } else {
        DEFAULT
    }
}

fn settings() -> BTreeMap<&'static str, Setting> {
    use Type::*;
    let mut map = BTreeMap::new();
    let mut add = |name: &'static str, setting: Setting| {
        map.insert(name, setting);
    };
    let ordered = |name: &'static str,
                   field: &'static str,
                   regex: bool,
                   order: &'static [Option<&'static str>]| {
        let mut setting = Setting::new(Ordered, name);
        setting.fields = vec![field];
        setting.regex = regex;
        setting.order = order;
        setting
    };
    add("vcodec", ordered("vcodec", "vcodec", true, VCODEC));
    add("acodec", ordered("acodec", "acodec", true, ACODEC));
    add("hdr", ordered("hdr", "dynamic_range", true, HDR));
    add("proto", ordered("proto", "protocol", true, PROTO));
    let mut vext = ordered("vext", "video_ext", false, VEXT);
    vext.order_free = Some(VEXT_FREE);
    add("vext", vext);
    let mut aext = ordered("aext", "audio_ext", true, AEXT);
    aext.order_free = Some(AEXT_FREE);
    add("aext", aext);
    let mut hidden = Setting::new(Extractor, "hidden");
    hidden.forced = true;
    hidden.max = Some(-1000.0);
    hidden.visible = false;
    add("hidden", hidden);
    let mut aud_or_vid = Setting::new(Multiple, "aud_or_vid");
    aud_or_vid.forced = true;
    aud_or_vid.visible = false;
    aud_or_vid.fields = vec!["vcodec", "acodec"];
    aud_or_vid.function = Some(Function::AnyNotNone);
    add("aud_or_vid", aud_or_vid);
    let mut ie_pref = Setting::new(Extractor, "ie_pref");
    ie_pref.priority = true;
    add("ie_pref", ie_pref);
    let boolean = |name: &'static str, field: &'static str, priority: bool| {
        let mut setting = Setting::new(Boolean, name);
        setting.fields = vec![field];
        setting.priority = priority;
        setting.not_in_list = &[Some("none")];
        setting
    };
    add("hasvid", boolean("hasvid", "vcodec", true));
    add("hasaud", boolean("hasaud", "acodec", false));
    let field = |name: &'static str, source: &'static str, convert, default| {
        let mut setting = Setting::new(Field, name);
        setting.fields = vec![source];
        setting.convert = convert;
        setting.default = default;
        setting
    };
    add(
        "lang",
        field("lang", "language_preference", Convert::Float, Some(-1.0)),
    );
    add(
        "quality",
        field("quality", "quality", Convert::Float, Some(-1.0)),
    );
    add(
        "filesize",
        field("filesize", "filesize", Convert::Bytes, None),
    );
    add(
        "fs_approx",
        field("fs_approx", "filesize_approx", Convert::Bytes, None),
    );
    add("id", field("id", "format_id", Convert::String, None));
    for name in ["height", "width", "fps", "tbr", "vbr", "abr", "asr"] {
        add(name, field(name, name, Convert::FloatNone, None));
    }
    add(
        "channels",
        field("channels", "audio_channels", Convert::FloatNone, None),
    );
    add(
        "source",
        field("source", "source_preference", Convert::Float, Some(-1.0)),
    );
    let mut codec = Setting::new(Combined, "codec");
    codec.fields = vec!["vcodec", "acodec"];
    add("codec", codec);
    let multiple = |name: &'static str, fields: Vec<&'static str>, convert, function| {
        let mut setting = Setting::new(Multiple, name);
        setting.fields = fields;
        setting.convert = convert;
        setting.function = Some(function);
        setting
    };
    add(
        "br",
        multiple(
            "br",
            vec!["tbr", "vbr", "abr"],
            Convert::FloatNone,
            Function::FirstTruthy,
        ),
    );
    add(
        "size",
        multiple(
            "size",
            vec!["filesize", "fs_approx"],
            Convert::Bytes,
            Function::FirstTruthy,
        ),
    );
    let mut ext = Setting::new(Combined, "ext");
    ext.fields = vec!["vext", "aext"];
    add("ext", ext);
    add(
        "res",
        multiple(
            "res",
            vec!["height", "width"],
            Convert::FloatString,
            Function::MinTruthy,
        ),
    );
    for (alias, target) in ALIASES {
        let mut setting = Setting::new(Alias, alias);
        setting.fields = vec![target];
        add(alias, setting);
    }
    map
}

/// Upstream alias fields (current and deprecated).
const ALIASES: &[(&str, &str)] = &[
    ("format_id", "id"),
    ("preference", "ie_pref"),
    ("language_preference", "lang"),
    ("source_preference", "source"),
    ("protocol", "proto"),
    ("filesize_approx", "fs_approx"),
    ("audio_channels", "channels"),
    ("dimension", "res"),
    ("resolution", "res"),
    ("extension", "ext"),
    ("bitrate", "br"),
    ("total_bitrate", "tbr"),
    ("video_bitrate", "vbr"),
    ("audio_bitrate", "abr"),
    ("framerate", "fps"),
    ("filesize_estimate", "size"),
    ("samplerate", "asr"),
    ("video_ext", "vext"),
    ("audio_ext", "aext"),
    ("video_codec", "vcodec"),
    ("audio_codec", "acodec"),
    ("video", "hasvid"),
    ("has_video", "hasvid"),
    ("audio", "hasaud"),
    ("has_audio", "hasaud"),
    ("extractor", "ie_pref"),
    ("extractor_preference", "ie_pref"),
];

/// Upstream `parse_bytes`: `NUMBER[kKmMgG…][i][bB]`.
fn parse_bytes(text: &str) -> Option<f64> {
    static PATTERN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?i)^(\d+(?:\.\d+)?)\s*([kmgtpezy]?)(i?)b?$").unwrap()
    });
    let found = PATTERN.captures(text.trim())?;
    let number: f64 = found[1].parse().ok()?;
    let exponent = "kmgtpezy"
        .find(&found[2].to_ascii_lowercase())
        .map_or(0, |index| index as i32 + 1);
    let exponent = if found[2].is_empty() { 0 } else { exponent };
    let base: f64 = if found[3].is_empty() { 1000.0 } else { 1024.0 };
    Some((number * base.powi(exponent)).round())
}

/// Python `float()` of a string, as `float_or_none` uses it.
fn py_float(text: &str) -> Option<f64> {
    let text = text.trim();
    let lowered = text.to_ascii_lowercase();
    match lowered.trim_start_matches(['+', '-']) {
        "inf" | "infinity" | "nan" => return text.parse().ok().or(Some(f64::NAN)),
        _ => {}
    }
    if text.is_empty()
        || !text
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E' | '_'))
    {
        return None;
    }
    text.replace('_', "").parse().ok()
}

/// `re.match(pattern, value)` for a static order pattern, compiled once.
fn order_regex(pattern: &'static str) -> regex::Regex {
    static CACHE: std::sync::LazyLock<
        std::sync::Mutex<std::collections::HashMap<&'static str, regex::Regex>>,
    > = std::sync::LazyLock::new(Default::default);
    CACHE
        .lock()
        .unwrap()
        .entry(pattern)
        .or_insert_with(|| regex::Regex::new(&format!("^(?:{pattern})")).unwrap())
        .clone()
}

/// A configured sorter for one sort-order list.
#[derive(Debug, Clone)]
pub struct FormatSorter {
    settings: BTreeMap<&'static str, Setting>,
    /// Extra settings for arbitrary (unknown) fields from sort strings.
    unknown: BTreeMap<String, Setting>,
    order: Vec<String>,
    free: bool,
    user: Vec<String>,
    extractor: Vec<String>,
}

/// An invalid sort string (upstream `ExtractorError`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidSort(pub String);

impl FormatSorter {
    /// Upstream `evaluate_params` for `-S` user fields, extractor fields,
    /// `--format-sort-force` and `--prefer-free-formats`.
    pub fn new(
        user: &[String],
        extractor: &[&str],
        force: bool,
        free: bool,
    ) -> Result<Self, InvalidSort> {
        let mut sorter = Self {
            settings: settings(),
            unknown: BTreeMap::new(),
            order: Vec::new(),
            free,
            user: user.to_vec(),
            extractor: extractor.iter().map(|field| (*field).to_owned()).collect(),
        };
        let forced: Vec<String> = default_fields()
            .iter()
            .filter(|field| sorter.settings.get(**field).is_some_and(|s| s.forced))
            .map(|field| (*field).to_owned())
            .collect();
        let priority: Vec<String> = if force {
            Vec::new()
        } else {
            default_fields()
                .iter()
                .filter(|field| sorter.settings.get(**field).is_some_and(|s| s.priority))
                .map(|field| (*field).to_owned())
                .collect()
        };
        let items: Vec<String> = forced
            .into_iter()
            .chain(priority)
            .chain(user.iter().cloned())
            .chain(extractor.iter().map(|field| (*field).to_owned()))
            .chain(default_fields().iter().map(|field| (*field).to_owned()))
            .collect();
        static PATTERN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
            regex::Regex::new(
                r"^ *(?:(?P<reverse>\+)?(?P<field>[a-zA-Z0-9_]+)(?:(?P<separator>[~:])(?P<limit>.*?))?)? *$",
            )
            .unwrap()
        });
        for item in items {
            let found = PATTERN
                .captures(&item)
                .ok_or_else(|| InvalidSort(item.clone()))?;
            let Some(field) = found.name("field").map(|m| m.as_str().to_owned()) else {
                continue;
            };
            let field = match sorter.setting(&field) {
                Some(setting) if setting.kind == Type::Alias => setting.fields[0].to_owned(),
                _ => field,
            };
            let reverse = found.name("reverse").is_some();
            let closest = found.name("separator").is_some_and(|m| m.as_str() == "~");
            let limit_text = found.name("limit").map(|m| m.as_str().to_owned());
            let combined = sorter
                .setting(&field)
                .is_some_and(|setting| setting.kind == Type::Combined);
            let fields: Vec<String> = if combined {
                sorter.settings[field.as_str()]
                    .fields
                    .iter()
                    .map(|f| (*f).to_owned())
                    .collect()
            } else {
                vec![field.clone()]
            };
            let limits: Vec<String> = match &limit_text {
                Some(text) if combined => text.split(':').map(str::to_owned).collect(),
                Some(text) => vec![text.clone()],
                None => Vec::new(),
            };
            for (index, name) in fields.iter().enumerate() {
                let limit = limits
                    .get(index)
                    .or(if combined { None } else { limits.first() })
                    .cloned();
                sorter.add_item(name, reverse, closest, limit);
            }
        }
        Ok(sorter)
    }

    /// Upstream `print_verbose_info`.
    pub fn verbose_info(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if !self.user.is_empty() {
            lines.push(format!(
                "Sort order given by user: {}",
                self.user.join(", ")
            ));
        }
        if !self.extractor.is_empty() {
            lines.push(format!(
                "Sort order given by extractor: {}",
                self.extractor.join(", ")
            ));
        }
        let fields: Vec<String> = self
            .order
            .iter()
            .filter_map(|field| {
                let setting = self.setting(field);
                if setting.is_some_and(|setting| !setting.visible) {
                    return None;
                }
                let mut text = String::new();
                if setting.is_some_and(|setting| setting.reverse) {
                    text.push('+');
                }
                text.push_str(field);
                if let Some((limit, shown)) =
                    setting.and_then(|setting| setting.limit_text.as_ref())
                {
                    let closest = setting.is_some_and(|setting| setting.closest);
                    text.push_str(&format!(
                        "{}{limit}({shown})",
                        if closest { '~' } else { ':' }
                    ));
                }
                Some(text)
            })
            .collect();
        lines.push(format!("Formats sorted by: {}", fields.join(", ")));
        lines
    }

    fn setting(&self, field: &str) -> Option<&Setting> {
        self.settings.get(field).or_else(|| self.unknown.get(field))
    }

    fn setting_mut(&mut self, field: &str) -> &mut Setting {
        if let Some(setting) = self.settings.get_mut(field) {
            return setting;
        }
        // `_get_field_setting` for arbitrary fields: an empty field setting.
        self.unknown.entry(field.to_owned()).or_insert_with(|| {
            let mut setting = Setting::new(Type::Field, "");
            setting.fields = Vec::new();
            setting
        })
    }

    fn add_item(&mut self, field: &str, reverse: bool, closest: bool, limit_text: Option<String>) {
        let field = field.to_lowercase();
        if self.order.contains(&field) {
            return;
        }
        self.order.push(field.clone());
        let convert = self
            .setting(&field)
            .map_or(Convert::FloatString, |s| s.convert);
        let limit = limit_text
            .as_deref()
            .and_then(|text| self.resolve(&field, Some(text), false));
        // Python `str` of the resolved limit: `order` and `bytes` give ints.
        let shown = match &limit {
            None => "None".to_owned(),
            Some(Value::Str(text)) => text.clone(),
            Some(Value::Num(number)) if matches!(convert, Convert::Order | Convert::Bytes) => {
                format!("{number:.0}")
            }
            Some(Value::Num(number)) => crate::screen::py_float(*number),
        };
        let setting = self.setting_mut(&field);
        setting.limit_text = limit_text.map(|text| (text, shown));
        if setting.fields.is_empty() && setting.kind == Type::Field {
            // Arbitrary fields read the format key of the same name.
            setting.fields = Vec::new();
        }
        setting.reverse = reverse;
        setting.closest = limit.is_some() && closest;
        setting.limit = limit;
    }

    fn field_names(&self, field: &str) -> Vec<String> {
        match self.setting(field) {
            Some(setting) if !setting.fields.is_empty() => {
                setting.fields.iter().map(|f| (*f).to_owned()).collect()
            }
            _ => vec![field.to_owned()],
        }
    }

    /// `_resolve_field_value`.
    fn resolve(&mut self, field: &str, value: Option<&str>, convert_none: bool) -> Option<Value> {
        let lowered = value.map(str::to_lowercase);
        if lowered.is_none() && !convert_none {
            return None;
        }
        let free = self.free;
        let setting = self.setting(field).cloned();
        let convert = setting.as_ref().map_or(Convert::FloatString, |s| s.convert);
        match convert {
            Convert::String => lowered.map(Value::Str),
            Convert::FloatNone => lowered.as_deref().and_then(py_float).map(Value::Num),
            Convert::Bytes => lowered.as_deref().and_then(parse_bytes).map(Value::Num),
            Convert::Order => {
                let setting = setting.expect("ordered fields have settings");
                let order = if free {
                    setting.order_free.unwrap_or(setting.order)
                } else {
                    setting.order
                };
                let length = order.len() as f64;
                let empty = order
                    .iter()
                    .position(|item| *item == Some(""))
                    .map_or(length + 1.0, |index| index as f64);
                if setting.regex
                    && let Some(value) = &lowered
                {
                    for (index, pattern) in order.iter().enumerate() {
                        if let Some(pattern) = pattern.filter(|p| !p.is_empty())
                            && order_regex(pattern).is_match(value)
                        {
                            return Some(Value::Num(length - index as f64));
                        }
                    }
                    return Some(Value::Num(length - empty));
                }
                let position = order
                    .iter()
                    .position(|item| item.map(str::to_owned) == lowered)
                    .map_or(empty, |index| index as f64);
                Some(Value::Num(length - position))
            }
            Convert::FloatString | Convert::Float => {
                let value = lowered?;
                if !value.is_empty() && value.chars().all(|c| c.is_numeric()) {
                    value.parse().ok().map(Value::Num)
                } else {
                    self.setting_mut(field).convert = Convert::String;
                    Some(Value::Str(value))
                }
            }
        }
    }

    /// `_calculate_field_preference_from_value`.
    fn preference_from_value(&mut self, field: &str, kind: Type, value: Option<Value>) -> Vec<Key> {
        let setting = self.setting(field).cloned().unwrap_or_else(|| {
            let mut setting = Setting::new(Type::Field, "");
            setting.fields = Vec::new();
            setting
        });
        let (reverse, closest, limit) = (setting.reverse, setting.closest, setting.limit.clone());
        let mut value = value;
        match kind {
            Type::Extractor => {
                let below_max = |v: f64| setting.max.is_none_or(|max| v < max);
                value = match value {
                    Some(Value::Num(v)) if below_max(v) => Some(Value::Num(v)),
                    _ => Some(Value::Num(-1.0)),
                };
            }
            Type::Boolean => {
                let text = match &value {
                    Some(Value::Str(text)) => Some(text.as_str()),
                    _ => None,
                };
                let excluded = match &value {
                    None => setting.not_in_list.contains(&None),
                    Some(Value::Str(_)) => setting.not_in_list.contains(&text),
                    Some(Value::Num(_)) => false,
                };
                value = Some(Value::Num(if excluded { -1.0 } else { 0.0 }));
            }
            Type::Ordered => {
                let text = match &value {
                    Some(Value::Str(text)) => Some(text.clone()),
                    Some(Value::Num(number)) => Some(crate::screen::py_float(*number)),
                    None => None,
                };
                value = self.resolve(field, text.as_deref(), true);
            }
            _ => {}
        }
        let convert = self
            .setting(field)
            .map_or(Convert::FloatString, |s| s.convert);
        // `float_or_none(value, default=...)`.
        let number = match &value {
            None => setting.default,
            Some(Value::Num(number)) => Some(*number),
            Some(Value::Str(text)) => py_float(text).or(setting.default),
        };
        let is_num = convert != Convert::String && number.is_some();
        let value = if is_num {
            number.map(Value::Num)
        } else {
            value
        };
        let Some(value) = value else {
            return vec![Key::Num(-10.0), Key::Num(0.0)];
        };
        let Value::Num(value) = value else {
            let Value::Str(text) = value else {
                unreachable!()
            };
            return vec![Key::Num(1.0), Key::Str(text), Key::Num(0.0)];
        };
        if !is_num {
            return vec![
                Key::Num(1.0),
                Key::Str(crate::screen::py_float(value)),
                Key::Num(0.0),
            ];
        }
        let limit = match limit {
            Some(Value::Num(limit)) => Some(limit),
            _ => None,
        };
        let key = |a: f64, b: f64, c: f64| vec![Key::Num(a), Key::Num(b), Key::Num(c)];
        if closest {
            let limit = limit.expect("closest requires a limit");
            return key(
                0.0,
                -(value - limit).abs(),
                if reverse {
                    value - limit
                } else {
                    limit - value
                },
            );
        }
        match limit {
            _ if !reverse && limit.is_none_or(|limit| value <= limit) => key(0.0, value, 0.0),
            None => key(0.0, -value, 0.0),
            Some(limit) if (reverse && value == limit) || value > limit => key(0.0, -value, 0.0),
            Some(_) => key(-1.0, value, 0.0),
        }
    }

    /// `calculate_preference` for a format whose sorting fields are filled.
    pub fn preference(&mut self, format: &Fields) -> Vec<Vec<Key>> {
        let order = self.order.clone();
        order
            .iter()
            .map(|field| self.field_preference(format, field))
            .collect()
    }

    fn field_preference(&mut self, format: &Fields, field: &str) -> Vec<Key> {
        let setting = self.setting(field).cloned();
        let kind = setting.as_ref().map_or(Type::Field, |s| s.kind);
        let get = |sorter: &Self, name: &str| -> Option<Value> {
            let source = sorter.field_names(name);
            format.get(&source[0]).cloned()
        };
        if kind == Type::Multiple {
            let setting = setting.expect("multiple fields have settings");
            let values: Vec<Option<Value>> =
                setting.fields.iter().map(|name| get(self, name)).collect();
            let truthy = |value: &Option<Value>| match value {
                Some(Value::Num(number)) => *number != 0.0 && !number.is_nan() || number.is_nan(),
                Some(Value::Str(text)) => !text.is_empty(),
                None => false,
            };
            let value = match setting.function.expect("multiple fields have a function") {
                Function::AnyNotNone => Some(Value::Num(
                    if values
                        .iter()
                        .any(|value| *value != Some(Value::Str("none".into())))
                    {
                        1.0
                    } else {
                        0.0
                    },
                )),
                Function::FirstTruthy => values.into_iter().find(truthy).flatten(),
                Function::MinTruthy => Some(Value::Num(
                    values
                        .into_iter()
                        .filter(truthy)
                        .filter_map(|value| match value {
                            Some(Value::Num(number)) => Some(number),
                            _ => None,
                        })
                        .fold(None, |min: Option<f64>, number| {
                            Some(min.map_or(number, |min| min.min(number)))
                        })
                        .unwrap_or(0.0),
                )),
            };
            return self.preference_from_value(field, Type::Field, value);
        }
        let value = get(self, field);
        self.preference_from_value(field, kind, value)
    }

    /// Sort formats ascending (worst first) like `formats.sort(key=...)`.
    pub fn sort<T>(&mut self, formats: &mut [T], fields: impl Fn(&T) -> Fields) {
        let mut keyed: Vec<(Vec<Vec<Key>>, usize)> = formats
            .iter()
            .enumerate()
            .map(|(index, format)| (self.preference(&fields(format)), index))
            .collect();
        keyed.sort_by(|a, b| compare(&a.0, &b.0));
        let order: Vec<usize> = keyed.into_iter().map(|(_, index)| index).collect();
        apply_order(formats, &order);
    }
}

/// Reorder `items` so position `i` holds the old `items[order[i]]`.
fn apply_order<T>(items: &mut [T], order: &[usize]) {
    let mut position: Vec<usize> = (0..items.len()).collect();
    let mut location: Vec<usize> = (0..items.len()).collect();
    for (target, &source) in order.iter().enumerate() {
        let current = location[source];
        if current != target {
            items.swap(target, current);
            let displaced = position[target];
            position.swap(target, current);
            location[displaced] = current;
            location[source] = target;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(value: &serde_json::Value) -> Fields {
        value
            .as_object()
            .unwrap()
            .iter()
            .filter_map(|(key, value)| {
                let value = match value {
                    serde_json::Value::Number(number) => Value::Num(number.as_f64()?),
                    serde_json::Value::String(text) => Value::Str(text.clone()),
                    _ => return None,
                };
                Some((key.clone(), value))
            })
            .collect()
    }

    #[test]
    fn apply_order_permutes_in_place() {
        let mut items = vec!['a', 'b', 'c', 'd'];
        apply_order(&mut items, &[2, 0, 3, 1]);
        assert_eq!(items, ['c', 'a', 'd', 'b']);
    }

    #[test]
    fn saved_upstream_format_sorter_cases() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/format_sort/cases.json")).unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let user: Vec<String> = case["sort"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_str().unwrap().to_owned())
                .collect();
            let extractor: Vec<&str> = case["extractor"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_str().unwrap())
                .collect();
            let mut sorter = FormatSorter::new(
                &user,
                &extractor,
                case["force"].as_bool().unwrap(),
                case["free"].as_bool().unwrap(),
            )
            .unwrap();
            let mut formats: Vec<serde_json::Value> = case["filled"].as_array().unwrap().to_owned();
            sorter.sort(&mut formats, fields);
            let ids: Vec<&str> = formats
                .iter()
                .map(|format| format["format_id"].as_str().unwrap())
                .collect();
            assert_eq!(
                ids,
                case["sorted"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|id| id.as_str().unwrap())
                    .collect::<Vec<_>>(),
                "{}",
                case["name"]
            );
        }
    }
}

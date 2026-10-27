//! Upstream info-dict key order and `None` keys for printed and written info
//! (`-j`, `-J`, info JSON). Python dicts keep insertion order, so upstream's
//! order is the order its code paths add keys: the extractor's dict, then
//! `add_extra_info`, `process_video_result` and the selected format's
//! `info_dict.update(format)`. Extractors mark which upstream path built an
//! info or format dict; [`video`] and [`format`] replay that path, keeping
//! this port's values and adding `null` where upstream assigns `None`.
//! Unmarked dicts (YouTube, loaded info JSON) keep this port's order.
use serde_json::{Map, Value};

/// The upstream code path of an info dict (see [`video`]).
pub(crate) const INFO_SHAPE: &str = "__yt_dlp_rs_info_shape";
/// The upstream code path of a format dict (see [`format`]).
pub(crate) const FORMAT_SHAPE: &str = "__yt_dlp_rs_format_shape";

/// How an upstream step sets a key.
#[derive(Clone, Copy)]
enum Set {
    /// Only with a value.
    Take,
    /// Always, `None` without a value.
    Assign,
    /// Always, `{}` without a value.
    Dict,
    /// Always, `[]` without a value.
    List,
    /// `parse_codecs`'s `dynamic_range`: always when codecs are known.
    Codecs,
}
use Set::*;

/// Insertion replay: keys move from this port's dict into upstream order.
struct Replay {
    ours: Map<String, Value>,
    out: Map<String, Value>,
}

impl Replay {
    fn new(ours: Map<String, Value>) -> Self {
        Self {
            ours,
            out: Map::new(),
        }
    }

    /// A key upstream sets when it has a value.
    fn take(&mut self, key: &str) {
        if let Some(value) = self.ours.shift_remove(key) {
            self.out.insert(key.to_owned(), value);
        }
    }

    /// A key upstream always assigns, `None` without a value. An already
    /// placed key keeps its position.
    fn assign(&mut self, key: &str) {
        match self.ours.shift_remove(key) {
            Some(value) => {
                self.out.insert(key.to_owned(), value);
            }
            None if !self.out.contains_key(key) => {
                self.out.insert(key.to_owned(), Value::Null);
            }
            None => {}
        }
    }

    fn set(&mut self, key: &str, how: Set) {
        match how {
            Take => self.take(key),
            Assign => self.assign(key),
            Dict => {
                if !self.ours.contains_key(key) && !self.out.contains_key(key) {
                    self.out.insert(key.to_owned(), Value::Object(Map::new()));
                }
                self.take(key);
            }
            List => {
                if !self.ours.contains_key(key) && !self.out.contains_key(key) {
                    self.out.insert(key.to_owned(), Value::Array(Vec::new()));
                }
                self.take(key);
            }
            Codecs => {
                if self.get("vcodec").is_some() || self.get("acodec").is_some() {
                    self.assign(key);
                }
            }
        }
    }

    /// A loaded info JSON dict's keys in file order: this port's value, or
    /// the file's for keys it does not represent.
    fn loaded(&mut self, original: &Map<String, Value>) {
        for (key, value) in original {
            if self.ours.contains_key(key) {
                self.take(key);
                // Subtitle tracks keep their loaded key order too.
                if matches!(key.as_str(), "subtitles" | "automatic_captions")
                    && let Some(Value::Object(languages)) = self.out.get_mut(key)
                {
                    reorder_tracks(languages, value);
                }
            } else if !self.out.contains_key(key) {
                self.out.insert(key.clone(), value.clone());
            }
        }
    }

    fn steps(&mut self, steps: &[(&str, Set)]) {
        for (key, how) in steps {
            self.set(key, *how);
        }
    }

    /// Remaining keys, except those `held` for later steps.
    fn rest_except(&mut self, held: &dyn Fn(&str) -> bool) {
        let keys: Vec<String> = self.ours.keys().filter(|key| !held(key)).cloned().collect();
        for key in keys {
            self.take(&key);
        }
    }

    fn finish(mut self) -> Map<String, Value> {
        self.rest_except(&|_| false);
        self.out
    }

    fn get(&self, key: &str) -> Option<&Value> {
        self.ours.get(key).or_else(|| self.out.get(key))
    }

    fn text(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Value::as_str)
    }

    /// Python truthiness of a value.
    fn truthy(&self, key: &str) -> bool {
        match self.get(key) {
            None | Some(Value::Null) => false,
            Some(Value::Bool(value)) => *value,
            Some(Value::Number(number)) => number.as_f64() != Some(0.0),
            Some(Value::String(text)) => !text.is_empty(),
            Some(Value::Array(items)) => !items.is_empty(),
            Some(Value::Object(map)) => !map.is_empty(),
        }
    }

    /// `process_video_result`'s per-format steps: `_fill_sorting_fields`,
    /// the audio `acodec`, `resolution`, `dynamic_range`, `aspect_ratio`,
    /// `filesize_approx` and `http_headers`, then `format_id` and `format`.
    fn format_steps(&mut self) {
        self.take("url");
        self.assign("protocol");
        self.assign("ext");
        let audio_only = self.text("vcodec") == Some("none");
        if audio_only {
            self.assign("audio_ext");
            self.assign("video_ext");
        } else {
            self.assign("video_ext");
            self.assign("audio_ext");
        }
        self.take("preference");
        // `vbr`/`abr` are zeroed for a missing stream before the others.
        if audio_only {
            self.assign("vbr");
        }
        if self.text("acodec") == Some("none") {
            self.assign("abr");
        }
        for key in ["vbr", "abr", "tbr"] {
            self.assign(key);
        }
        self.take("acodec");
        self.assign("resolution");
        if !audio_only {
            self.assign("dynamic_range");
        }
        self.assign("aspect_ratio");
        // `filesize_from_tbr` (see `info::fill_format_descriptions`), `None`
        // without a bitrate or duration.
        let manifests =
            crate::info::COMPAT_MANIFEST_FILESIZE_APPROX.load(std::sync::atomic::Ordering::Relaxed);
        if (manifests || self.get("manifest_url").is_none())
            && !self.truthy("filesize")
            && !self.truthy("filesize_approx")
        {
            self.assign("filesize_approx");
        } else {
            self.take("filesize_approx");
        }
        self.assign("http_headers");
        self.assign("format_id");
        self.assign("format");
    }
}

/// Drop the code-path markers from a dict other code reads as upstream's.
pub(crate) fn strip_markers(map: &mut Map<String, Value>) {
    map.shift_remove(INFO_SHAPE);
    map.shift_remove(FORMAT_SHAPE);
    map.shift_remove(INT_QUALITY);
}

/// Record the upstream code path that built an info dict.
pub(crate) fn mark_info(mut info: crate::VideoInfo, shape: &str) -> crate::VideoInfo {
    info.extra.insert(INFO_SHAPE.into(), shape.into());
    info
}

/// Order each loaded subtitle track's keys like the file's track.
fn reorder_tracks(languages: &mut Map<String, Value>, original: &Value) {
    for (language, tracks) in languages.iter_mut() {
        let (Value::Array(tracks), Some(Value::Array(loaded))) = (tracks, original.get(language))
        else {
            continue;
        };
        for (track, loaded) in tracks.iter_mut().zip(loaded) {
            let (Value::Object(track), Value::Object(loaded)) = (track, loaded) else {
                continue;
            };
            let mut ordered = Map::new();
            for key in loaded.keys() {
                if let Some(value) = track.shift_remove(key) {
                    ordered.insert(key.clone(), value);
                }
            }
            ordered.append(track);
            *track = ordered;
        }
    }
}

/// The shape of a dict loaded from info JSON: its sanitized original, whose
/// key order (and unrepresented values) the replay starts from.
const LOADED: &str = "loaded";

/// Mark a dict loaded with `--load-info-json` (`original` without its
/// formats' contents).
pub(crate) fn loaded_shape(original: &Map<String, Value>) -> Value {
    let mut original = original.clone();
    if original.contains_key("formats") {
        original.insert("formats".into(), Value::Null);
    }
    Value::Object(Map::from_iter([(
        LOADED.to_owned(),
        Value::Object(original),
    )]))
}

/// A marker's shape name and, for loaded dicts, the original.
fn shape_of(marker: Option<Value>) -> Option<(String, Option<Map<String, Value>>)> {
    match marker? {
        Value::String(shape) => Some((shape, None)),
        Value::Object(mut map) => match map.shift_remove(LOADED) {
            Some(Value::Object(original)) => Some((LOADED.to_owned(), Some(original))),
            _ => None,
        },
        _ => None,
    }
}

/// Record the upstream code path that built a format dict.
pub(crate) fn mark_format(format: &mut crate::VideoFormat, shape: &str) {
    format.extra.insert(FORMAT_SHAPE.into(), shape.into());
}

/// A caller's `format[key] = ...` after the format was built.
pub(crate) fn mark_format_update(format: &mut crate::VideoFormat, key: &str) {
    let Some(Value::String(shape)) = format.extra.get(FORMAT_SHAPE).cloned() else {
        return;
    };
    let (shape, int_rates) = match shape.strip_suffix("|int-rates") {
        Some(shape) => (shape.to_owned(), true),
        None => (shape, false),
    };
    let mut shape = if shape.contains("|update:") {
        format!("{shape},{key}")
    } else {
        format!("{shape}|update:{key}")
    };
    if int_rates {
        shape.push_str("|int-rates");
    }
    format.extra.insert(FORMAT_SHAPE.into(), shape.into());
}

/// The format's `tbr` (and the `abr`/`vbr` derived from it) is an int.
pub(crate) fn mark_int_rates(format: &mut crate::VideoFormat) {
    if let Some(Value::String(shape)) = format.extra.get(FORMAT_SHAPE).cloned()
        && !shape.ends_with("|int-rates")
    {
        format
            .extra
            .insert(FORMAT_SHAPE.into(), format!("{shape}|int-rates").into());
    }
}

/// A format whose extractor gave an int `quality` (stored as a float).
const INT_QUALITY: &str = "__yt_dlp_rs_int_quality";

/// The format's `quality` is an int.
pub(crate) fn mark_int_quality(format: &mut crate::VideoFormat) {
    format.extra.insert(INT_QUALITY.into(), Value::Bool(true));
}

/// One format dict in upstream order (unmarked formats are unchanged).
pub(crate) fn format(mut map: Map<String, Value>) -> Map<String, Value> {
    if map.shift_remove(INT_QUALITY).is_some()
        && let Some(quality) = map.get("quality").and_then(Value::as_f64)
        && quality.fract() == 0.0
    {
        map.insert("quality".into(), (quality as i64).into());
    }
    let Some((shape, original)) = shape_of(map.shift_remove(FORMAT_SHAPE)) else {
        return map;
    };
    let (shape, int_rates) = match shape.strip_suffix("|int-rates") {
        Some(shape) => (shape.to_owned(), true),
        None => (shape, false),
    };
    // Keys a caller's `format.update(...)` adds after the extractor's.
    let (shape, update) = match shape.split_once("|update:") {
        Some((shape, keys)) => (
            shape.to_owned(),
            keys.split(',').map(str::to_owned).collect::<Vec<_>>(),
        ),
        None => (shape, Vec::new()),
    };
    // `_parse_html5_media_entries` adds the page Referer to every format.
    let (html5, shape) = match shape.strip_prefix("html5+") {
        Some(shape) => (true, shape),
        None => (shape.starts_with("html5-"), shape.as_str()),
    };
    // An extractor's own format dict (`dict:KEYS`), every key set.
    let dict_keys: Vec<(String, Set)> = shape
        .strip_prefix("dict:")
        .map(|keys| {
            keys.split(',')
                .map(|key| {
                    (
                        key.to_owned(),
                        if key.starts_with("__") { Take } else { Assign },
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let dict_steps: Vec<(&str, Set)> = dict_keys
        .iter()
        .map(|(key, how)| (key.as_str(), *how))
        .collect();
    let mut replay = Replay::new(map);
    let extractor: &[(&str, Set)] = match shape {
        _ if shape.starts_with("dict:") => &dict_steps,
        // `FczenitIE`'s quality.
        "fczenit" => &[("format_id", Take), ("url", Take), ("height", Assign)],
        // `CJSWIE`'s audio.
        "cjsw" => &[("url", Take), ("ext", Take), ("vcodec", Take)],
        // `ZhihuIE`'s playlist quality.
        "zhihu" => &[
            ("asr", Assign),
            ("filesize", Assign),
            ("format_id", Take),
            ("fps", Assign),
            ("height", Assign),
            ("tbr", Assign),
            ("url", Take),
            ("width", Assign),
        ],
        // `CrooksAndLiarsIE`'s flavor.
        "crooksandliars" => &[("url", Take), ("format_id", Take), ("quality", Take)],
        // `MaarivIE`'s HTTP stream with `parse_resolution`'s keys.
        "maariv" => &[
            ("url", Take),
            ("format_id", Take),
            ("width", Take),
            ("height", Take),
        ],
        // `YoutubeIE._extract_storyboard`'s format.
        "youtube-storyboard" => &[
            ("format_id", Take),
            ("format_note", Take),
            ("ext", Take),
            ("protocol", Take),
            ("acodec", Take),
            ("vcodec", Take),
            ("url", Take),
            ("width", Take),
            ("height", Take),
            ("fps", Take),
            ("rows", Take),
            ("columns", Take),
            ("fragments", Take),
        ],
        // `YoutubeIE`'s `process_format_stream` dict.
        "youtube-https" => &[
            ("asr", Assign),
            ("filesize", Assign),
            ("format_id", Take),
            ("format_note", Assign),
            ("source_preference", Take),
            ("fps", Assign),
            ("audio_channels", Assign),
            ("height", Assign),
            ("quality", Take),
            ("has_drm", Take),
            ("tbr", Assign),
            ("filesize_approx", Assign),
            ("width", Assign),
            ("language", Assign),
            ("language_preference", Take),
            ("preference", Assign),
            ("ext", Take),
            ("vcodec", Take),
            ("acodec", Take),
            ("dynamic_range", Assign),
            ("container", Take),
            ("url", Take),
            ("available_at", Take),
            ("downloader_options", Take),
        ],
        // `BildIE`'s MP4 source.
        "bild" => &[("url", Take), ("format_id", Take)],
        // `BloggerIE`'s stream.
        "blogger" => &[("ext", Take), ("url", Take), ("format_id", Take)],
        // `TweakersIE`'s progressive source.
        "tweakers" => &[
            ("format_id", Take),
            ("url", Take),
            ("width", Assign),
            ("height", Assign),
            ("ext", Take),
        ],
        // `TwentyThreeVideoIE`'s audio download and video downloads.
        "23video-audio" => &[
            ("format_id", Take),
            ("url", Take),
            ("filesize", Assign),
            ("vcodec", Take),
        ],
        "23video" => &[
            ("format_id", Take),
            ("url", Take),
            ("width", Assign),
            ("height", Assign),
            ("filesize", Assign),
        ],
        // `_parse_ism_formats_and_subtitles`' track.
        "ism" => &[
            ("format_id", Take),
            ("url", Take),
            ("manifest_url", Take),
            ("ext", Take),
            ("width", Assign),
            ("height", Assign),
            ("tbr", Assign),
            ("asr", Assign),
            ("vcodec", Take),
            ("acodec", Take),
            ("protocol", Take),
            ("fragments", Take),
            ("has_drm", Take),
            ("language", Take),
            ("audio_channels", Assign),
            ("_download_params", Take),
        ],
        // Its text track.
        "ism-text" => {
            replay.steps(&[
                ("ext", Take),
                ("protocol", Take),
                ("url", Take),
                ("manifest_url", Take),
                ("fragments", Take),
                ("_download_params", Take),
            ]);
            return replay.finish();
        }
        // `_parse_f4m_formats`' media entry.
        "f4m" => &[
            ("format_id", Take),
            ("url", Take),
            ("manifest_url", Take),
            ("ext", Take),
            ("protocol", Take),
            ("tbr", Assign),
            ("width", Assign),
            ("height", Assign),
            ("vcodec", Assign),
            ("preference", Assign),
            ("quality", Assign),
        ],
        // `_extract_kvs`' format.
        "kvs" => &[
            ("url", Take),
            ("format_id", Take),
            ("ext", Take),
            ("width", Take),
            ("height", Take),
            ("http_headers", Take),
            ("quality", Take),
        ],
        // `_parse_smil_formats_and_subtitles`' HTTP source.
        "smil-http" => &[
            ("url", Take),
            ("ext", Take),
            ("format_id", Take),
            ("tbr", Assign),
            ("filesize", Assign),
            ("width", Assign),
            ("height", Assign),
        ],
        // `_parse_xspf`'s location.
        "xspf" => &[
            ("url", Take),
            ("manifest_url", Take),
            ("format_id", Take),
            ("width", Assign),
            ("height", Assign),
        ],
        // A `{'url': url}` format.
        "url-only" => &[("url", Take)],
        "url-ext" => &[("url", Take), ("ext", Take)],
        // `TwitterBaseIE._extract_variant_formats`' MP4 variant.
        "twitter-http" => &[
            ("url", Take),
            ("format_id", Take),
            ("tbr", Take),
            ("width", Take),
            ("height", Take),
        ],
        // `RedditIE`'s fallback MP4.
        "reddit-fallback" => &[
            ("url", Take),
            ("height", Assign),
            ("width", Assign),
            ("tbr", Assign),
            ("acodec", Take),
            ("vcodec", Take),
            ("ext", Take),
            ("format_id", Take),
            ("format_note", Take),
        ],
        // `StreamableIE`'s files.
        "streamable" => &[
            ("format_id", Take),
            ("url", Take),
            ("width", Assign),
            ("height", Assign),
            ("filesize", Assign),
            ("fps", Assign),
            ("vbr", Assign),
            ("vcodec", Assign),
            ("acodec", Assign),
        ],
        // `_parse_jwplayer_formats`' direct source.
        "jwplayer-source" => &[
            ("url", Take),
            ("width", Assign),
            ("height", Assign),
            ("tbr", Assign),
            ("filesize", Assign),
            ("ext", Assign),
            ("format_id", Assign),
        ],
        "jwplayer-audio" => &[("url", Take), ("vcodec", Assign), ("ext", Assign)],
        // A Video.js source with the page as its Referer.
        "videojs" => &[("url", Take), ("ext", Take), ("http_headers", Take)],
        // GenericIE's direct link: `vcodec` is `none` for audio, else `None`.
        "direct" => &[
            ("format_id", Take),
            ("url", Take),
            ("ext", Take),
            ("vcodec", Assign),
        ],
        // `_media_formats` for a media element's `src`.
        "html5-src" => &[("url", Take), ("vcodec", Assign), ("ext", Take)],
        // `parse_content_type` codecs and `ext`, the `<source>` attributes,
        // then `f.update(formats[0])`.
        "html5-source" => &[
            ("vcodec", Take),
            ("acodec", Take),
            ("dynamic_range", Codecs),
            ("ext", Take),
            ("width", Assign),
            ("height", Assign),
            ("tbr", Assign),
            ("format_id", Assign),
            ("url", Take),
            ("vcodec", Assign),
            ("ext", Take),
        ],
        // `_extract_m3u8_formats` for a media playlist.
        "hls-media" => &[
            ("format_id", Take),
            ("format_index", Assign),
            ("url", Take),
            ("ext", Take),
            ("protocol", Take),
            ("preference", Assign),
            ("quality", Assign),
            ("has_drm", Take),
        ],
        // A master playlist's variant, with `RESOLUTION` and `parse_codecs`.
        "hls-variant" => &[
            ("format_id", Take),
            ("format_index", Assign),
            ("url", Take),
            ("manifest_url", Take),
            ("tbr", Assign),
            ("ext", Take),
            ("fps", Assign),
            ("protocol", Take),
            ("preference", Assign),
            ("quality", Assign),
            ("has_drm", Take),
            ("width", Take),
            ("height", Take),
            ("vcodec", Take),
            ("acodec", Take),
            ("dynamic_range", Codecs),
        ],
        // A master playlist's audio rendition.
        "hls-rendition" => &[
            ("format_id", Take),
            ("format_note", Take),
            ("format_index", Assign),
            ("url", Take),
            ("manifest_url", Take),
            ("language", Assign),
            ("ext", Take),
            ("protocol", Take),
            ("preference", Assign),
            ("quality", Assign),
            ("has_drm", Take),
            ("vcodec", Assign),
            ("source_preference", Assign),
        ],
        // `_parse_mpd_formats` for a text representation, then its URL or
        // fragments.
        "dash-text" => {
            replay.steps(&[
                ("ext", Take),
                ("manifest_url", Take),
                ("filesize", Assign),
                ("url", Take),
                ("fragment_base_url", Take),
                ("fragments", Take),
                ("protocol", Take),
            ]);
            return replay.finish();
        }
        // `_parse_mpd_formats` for a segmented representation.
        "dash" => &[
            ("format_id", Take),
            ("manifest_url", Take),
            ("ext", Take),
            ("width", Assign),
            ("height", Assign),
            ("tbr", Assign),
            ("asr", Assign),
            ("fps", Assign),
            ("language", Assign),
            ("format_note", Take),
            ("filesize", Assign),
            ("container", Take),
            ("vcodec", Take),
            ("acodec", Take),
            ("dynamic_range", Codecs),
            ("url", Take),
            ("fragment_base_url", Take),
            ("fragments", Take),
            ("protocol", Take),
            ("manifest_stream_number", Take),
            ("is_dash_periods", Take),
        ],
        _ => &[],
    };
    replay.steps(extractor);
    for key in &update {
        replay.assign(key);
    }
    if let Some(original) = &original {
        replay.loaded(original);
    }
    if html5 {
        replay.take("http_headers");
    }
    // Keys this port has beyond the extractor's, before processing.
    replay.rest_except(&|key| PROCESSED_FORMAT_KEYS.contains(&key));
    replay.format_steps();
    let mut out = replay.finish();
    // `_check_formats`' `f['__working']` is set after processing.
    if let Some(working) = out.shift_remove("__working") {
        out.insert("__working".into(), working);
    }
    if int_rates {
        for key in ["tbr", "abr", "vbr", "quality"] {
            if let Some(rate) = out.get(key).and_then(Value::as_f64)
                && rate.fract() == 0.0
                && out.get(key).is_some_and(|value| !value.is_i64())
            {
                out.insert(key.into(), (rate as i64).into());
            }
        }
    }
    // `int_or_none(source.get('bitrate'), scale=1000)` is an int.
    if matches!(shape, "jwplayer-source" | "f4m" | "ism" | "reddit-fallback")
        && let Some(tbr) = out.get("tbr").and_then(Value::as_f64)
        && tbr.fract() == 0.0
    {
        out.insert("tbr".into(), (tbr as i64).into());
    }
    // An F4M/ISM entry's `abr`/`vbr` is its int `tbr`.
    if matches!(shape, "f4m" | "ism" | "reddit-fallback")
        && out.get("tbr").is_some_and(Value::is_i64)
    {
        for key in ["abr", "vbr"] {
            if let Some(rate) = out.get(key).and_then(Value::as_f64)
                && rate.fract() == 0.0
            {
                out.insert(key.into(), (rate as i64).into());
            }
        }
    }
    // `_extract_kvs`' `quality = 1` and `qualities()`' indices are ints.
    if matches!(shape, "kvs" | "crooksandliars")
        && let Some(quality) = out.get("quality").and_then(Value::as_f64)
        && quality.fract() == 0.0
    {
        out.insert("quality".into(), (quality as i64).into());
    }
    out
}

/// Keys `process_video_result` adds to formats.
const PROCESSED_FORMAT_KEYS: &[&str] = &[
    "url",
    "protocol",
    "ext",
    "video_ext",
    "audio_ext",
    "vbr",
    "abr",
    "tbr",
    "acodec",
    "resolution",
    "dynamic_range",
    "aspect_ratio",
    "filesize_approx",
    "http_headers",
    "format_id",
    "format",
];

/// Top-level keys added after the extractor's dict.
const PROCESSED_INFO_KEYS: &[&str] = &[
    "webpage_url",
    "original_url",
    "webpage_url_basename",
    "webpage_url_domain",
    "extractor",
    "extractor_key",
    "playlist",
    "playlist_index",
    "playlist_count",
    "playlist_id",
    "playlist_title",
    "playlist_uploader",
    "playlist_uploader_id",
    "playlist_channel",
    "playlist_channel_id",
    "playlist_webpage_url",
    "n_entries",
    "__last_playlist_index",
    "playlist_autonumber",
    "thumbnail",
    "thumbnails",
    "display_id",
    "fulltitle",
    "duration_string",
    "upload_date",
    "release_date",
    "modified_date",
    "release_year",
    "live_status",
    "is_live",
    "was_live",
    "chapter",
    "season",
    "episode",
    "requested_subtitles",
    "_has_drm",
    "epoch",
    "requested_formats",
    "requested_downloads",
    "_filename",
    "filename",
    "__postprocessors",
    "__real_download",
    "__files_to_merge",
    "__finaldir",
    "filepath",
    "__write_download_archive",
    "_type",
    "_version",
];

/// `_merge`'s dict for two selected formats.
const MERGED_KEYS: &[&str] = &[
    "requested_formats",
    "format",
    "format_id",
    "ext",
    "protocol",
    "language",
    "format_note",
    "filesize_approx",
    "tbr",
];
const MERGED_VIDEO_KEYS: &[&str] = &[
    "width",
    "height",
    "resolution",
    "fps",
    "dynamic_range",
    "vcodec",
    "vbr",
    "stretched_ratio",
    "aspect_ratio",
];
const MERGED_AUDIO_KEYS: &[&str] = &["acodec", "abr", "asr", "audio_channels"];

/// One video info dict in upstream order (unmarked dicts are unchanged).
/// Formats must already be ordered by [`format`].
pub(crate) fn video(mut map: Map<String, Value>) -> Map<String, Value> {
    map.shift_remove(FORMAT_SHAPE);
    // The selected format's int `quality`.
    if map.shift_remove(INT_QUALITY).is_some()
        && let Some(quality) = map.get("quality").and_then(Value::as_f64)
        && quality.fract() == 0.0
    {
        map.insert("quality".into(), (quality as i64).into());
    }
    let Some((shape, original)) = shape_of(map.shift_remove(INFO_SHAPE)) else {
        return map;
    };
    // A `url_transparent` result: the outer keys its update adds after the
    // inner extraction's.
    // A playlist entry's (`transparent-entry`) has the playlist's keys next.
    let split = |shape: &str, marker: &str| {
        shape.split_once(marker).map(|(inner, keys)| {
            (
                inner.to_owned(),
                keys.split(',')
                    .filter(|key| !key.is_empty())
                    .map(str::to_owned)
                    .collect::<Vec<_>>(),
            )
        })
    };
    // `_fill_common_fields`' multivalue keys (always the last suffix).
    let (shape, derived) = split(&shape, "|multivalue:").unwrap_or((shape, Vec::new()));
    // A `url_transparent` result, even one without overlaid keys.
    let transparent_result = shape.contains("|transparent:");
    let (shape, transparent, transparent_entry) = match split(&shape, "|transparent-entry:") {
        Some((inner, keys)) => (inner, keys, true),
        None => match split(&shape, "|transparent:") {
            Some((inner, keys)) => (inner, keys, false),
            None => (shape, Vec::new(), false),
        },
    };
    // An `-untitled` shape: upstream's dict has no `title`, so
    // `_fill_common_fields` adds the generic one after `fulltitle`.
    let untitled = shape.ends_with("-untitled");
    let shape = shape.strip_suffix("-untitled").unwrap_or(&shape).to_owned();
    // The selected format's keys, which `info_dict.update(format)` merges.
    let merged = map
        .get("requested_formats")
        .and_then(Value::as_array)
        .is_some_and(|formats| !formats.is_empty());
    // An untyped result (or a loaded one without formats) is its own
    // (only) format.
    let own_format = matches!(
        shape.as_str(),
        "generic-untyped"
            | "generic-found"
            | "generic-found-entry"
            | "reverbnation"
            | "hypem"
            | "historicfilms"
            | "livestreamfails"
            | "ebaumsworld"
            | "glide"
            | "godtube"
            | "behindkink"
            | "qingting"
            | "oneplace"
            | "sztvhu"
            | "thisamericanlife"
            | "photobucket"
            | "vocaroo"
            | "audioboom"
            | "playerfm"
            | "rtrfm"
            | "hitrecord"
            | "cultureunplugged"
            | "jove"
            | "ku6"
            | "fuyintv"
            | "harpodeon"
    ) || shape.starts_with("nobelprize;")
        || original
            .as_ref()
            .is_some_and(|original| !original.contains_key("formats"));
    let selected_format: Map<String, Value> = if merged || own_format {
        Map::new()
    } else {
        let id = map.get("format_id").cloned();
        map.get("formats")
            .and_then(Value::as_array)
            .and_then(|formats| {
                formats
                    .iter()
                    .find(|format| format.get("format_id") == id.as_ref())
            })
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default()
    };
    let selected: Vec<String> = if merged {
        let formats = map["requested_formats"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let streams = |codec: &str| {
            formats
                .iter()
                .filter(|format| format.get(codec).and_then(Value::as_str) != Some("none"))
                .count()
        };
        let mut keys: Vec<&str> = MERGED_KEYS.to_vec();
        if streams("vcodec") == 1 {
            keys.extend(MERGED_VIDEO_KEYS);
        }
        if streams("acodec") == 1 {
            keys.extend(MERGED_AUDIO_KEYS);
        }
        keys.into_iter().map(str::to_owned).collect()
    } else if own_format {
        Vec::new()
    } else {
        let id = map.get("format_id").cloned();
        map.get("formats")
            .and_then(Value::as_array)
            .and_then(|formats| {
                formats
                    .iter()
                    .find(|format| format.get("format_id") == id.as_ref())
            })
            .and_then(Value::as_object)
            .map(|format| format.keys().cloned().collect())
            .unwrap_or_default()
    };
    let untyped = own_format;
    let entry = matches!(
        shape.as_str(),
        "html5-entry"
            | "generic-found-entry"
            | "generic-found-hls-entry"
            | "generic-found-dash-entry"
            | "xspf-entry"
    ) || (shape.starts_with("dict;")
        && map.get("n_entries").is_some_and(|n| !n.is_null()));
    let mut replay = Replay::new(map);
    let extractor: &[(&str, Set)] = match shape.as_str() {
        "generic-direct" => &[
            ("id", Assign),
            ("title", Assign),
            ("timestamp", Assign),
            ("direct", Take),
            ("formats", Take),
            ("subtitles", Dict),
            ("http_headers", Assign),
            ("hls_aes", Assign),
        ],
        // "URL could be a direct video link": the info is its own format.
        "generic-untyped" => &[
            ("id", Assign),
            ("title", Assign),
            ("timestamp", Assign),
            ("direct", Take),
            ("url", Take),
            ("ext", Take),
        ],
        // A media URL found on the page (`_extract_embeds`' patterns), then
        // `merge_dicts(entry, info_dict)`'s page fields.
        // `_extract_kvs`' dict, then `merge_dicts`' page fields.
        "kvs" => &[
            ("id", Take),
            ("display_id", Take),
            ("title", Take),
            ("thumbnail", Take),
            ("formats", Take),
            ("timestamp", Take),
            ("description", Take),
            ("age_limit", Take),
        ],
        // Without a canonical link (`processing` adds `display_id` later).
        "kvs-without-display-id" => &[
            ("id", Take),
            ("title", Take),
            ("thumbnail", Take),
            ("formats", Take),
            ("timestamp", Take),
            ("description", Take),
            ("age_limit", Take),
        ],
        // A found SMIL URL: `{**smil_info, **entry_info_dict}` merged with
        // the page fields (`None` values dropped).
        "generic-found-smil" => &[
            ("id", Take),
            ("title", Take),
            ("description", Take),
            ("upload_date", Take),
            ("thumbnails", List),
            ("formats", Take),
            ("subtitles", Dict),
            ("uploader", Take),
            ("age_limit", Take),
            ("http_headers", Take),
            ("timestamp", Take),
            ("thumbnail", Take),
        ],
        // The same with the page's description (the SMIL file has none).
        "generic-found-smil-page-description" => &[
            ("id", Take),
            ("title", Take),
            ("upload_date", Take),
            ("thumbnails", List),
            ("formats", Take),
            ("subtitles", Dict),
            ("uploader", Take),
            ("age_limit", Take),
            ("http_headers", Take),
            ("timestamp", Take),
            ("description", Take),
            ("thumbnail", Take),
        ],
        "generic-found" => &[
            ("id", Take),
            ("uploader", Take),
            ("title", Take),
            ("age_limit", Take),
            ("http_headers", Take),
            ("url", Take),
            ("timestamp", Take),
            ("description", Take),
            ("thumbnail", Take),
        ],
        // GenericIE's info dict with `_parse_ism_formats_and_subtitles`.
        "generic-ism" => &[
            ("id", Assign),
            ("title", Assign),
            ("timestamp", Assign),
            ("formats", Take),
            ("subtitles", Dict),
        ],
        // GenericIE's info dict with `_parse_f4m_formats`.
        "generic-f4m" => &[
            ("id", Assign),
            ("title", Assign),
            ("timestamp", Assign),
            ("formats", Take),
        ],
        // `_parse_smil`'s dict.
        "smil" => &[
            ("id", Take),
            ("title", Take),
            ("description", Assign),
            ("upload_date", Assign),
            ("thumbnails", List),
            ("formats", Take),
            ("subtitles", Dict),
        ],
        // `_parse_xspf`'s track.
        "xspf-entry" => &[
            ("id", Take),
            ("title", Take),
            ("description", Assign),
            ("thumbnail", Assign),
            ("duration", Take),
            ("formats", Take),
        ],
        // `AudioBoomIE._real_extract`'s dict (its own format).
        "audioboom" => &[
            ("id", Take),
            ("url", Take),
            ("title", Take),
            ("description", Assign),
            ("duration", Take),
            ("uploader", Assign),
            ("uploader_url", Assign),
        ],
        // `VocarooIE._real_extract`'s dict (its own format).
        "vocaroo" => &[
            ("id", Take),
            ("title", Take),
            ("url", Take),
            ("ext", Take),
            ("timestamp", Assign),
            ("vcodec", Take),
            ("http_headers", Take),
        ],
        // `PhotobucketIE._real_extract`'s dict (its own format).
        "photobucket" => &[
            ("id", Take),
            ("url", Take),
            ("uploader", Assign),
            ("timestamp", Assign),
            ("title", Take),
            ("ext", Take),
            ("thumbnail", Assign),
        ],
        // `ThisAmericanLifeIE._real_extract`'s dict (its own format).
        "thisamericanlife" => &[
            ("id", Take),
            ("url", Take),
            ("protocol", Take),
            ("ext", Take),
            ("acodec", Take),
            ("vcodec", Take),
            ("abr", Take),
            ("title", Take),
            ("description", Assign),
            ("thumbnail", Assign),
        ],
        // `SztvHuIE._real_extract`'s dict (its own format).
        "sztvhu" => &[
            ("id", Take),
            ("url", Take),
            ("title", Take),
            ("description", Assign),
            ("thumbnail", Assign),
        ],
        // `OnePlacePodcastIE._real_extract`'s dict (its own format).
        "oneplace" => &[
            ("id", Take),
            ("url", Take),
            ("ext", Take),
            ("vcodec", Take),
            ("title", Take),
            ("description", Assign),
        ],
        // `QingTingIE._real_extract`'s dict (its own format).
        "qingting" => &[
            ("id", Take),
            ("title", Take),
            ("channel_id", Take),
            ("channel", Assign),
            ("uploader", Assign),
            ("url", Take),
            ("vcodec", Take),
            ("acodec", Take),
            ("ext", Take),
        ],
        // `BehindKinkIE._real_extract`'s dict (its own format).
        "behindkink" => &[
            ("id", Take),
            ("display_id", Take),
            ("url", Take),
            ("title", Take),
            ("thumbnail", Assign),
            ("description", Assign),
            ("upload_date", Take),
            ("age_limit", Take),
        ],
        // `FczenitIE._real_extract`'s dict.
        "fczenit" => &[
            ("id", Take),
            ("title", Take),
            ("thumbnail", Assign),
            ("formats", Take),
            ("duration", Take),
            ("timestamp", Assign),
            ("tags", List),
        ],
        // `GodTubeIE._real_extract`'s dict (its own format).
        "godtube" => &[
            ("id", Take),
            ("url", Take),
            ("title", Take),
            ("thumbnail", Assign),
            ("timestamp", Assign),
            ("uploader", Assign),
            ("duration", Take),
        ],
        // `GlideIE._real_extract`'s dict (its own format).
        "glide" => &[
            ("id", Take),
            ("title", Take),
            ("url", Take),
            ("thumbnail", Assign),
        ],
        // `EbaumsWorldIE._real_extract`'s dict (its own format).
        "ebaumsworld" => &[
            ("id", Take),
            ("title", Take),
            ("url", Take),
            ("description", Assign),
            ("thumbnail", Assign),
            ("uploader", Assign),
        ],
        // `LivestreamfailsIE._real_extract`'s dict (its own format).
        "livestreamfails" => &[
            ("id", Take),
            ("display_id", Assign),
            ("timestamp", Assign),
            ("url", Take),
            ("title", Take),
            ("creator", Assign),
            ("thumbnail", Assign),
        ],
        // `HistoricFilmsIE._real_extract`'s dict (its own format).
        "historicfilms" => &[
            ("id", Take),
            ("url", Take),
            ("title", Take),
            ("description", Assign),
            ("thumbnail", Assign),
            ("duration", Take),
        ],
        // `HypemIE._real_extract`'s dict (its own format).
        "hypem" => &[
            ("id", Take),
            ("url", Take),
            ("ext", Take),
            ("title", Take),
            ("uploader", Assign),
            ("duration", Take),
            ("timestamp", Assign),
            ("track", Take),
        ],
        // `ReverbNationIE._real_extract`'s dict (its own format).
        "reverbnation" => &[
            ("id", Take),
            ("title", Take),
            ("url", Take),
            ("uploader", Assign),
            ("uploader_id", Assign),
            ("thumbnails", List),
            ("ext", Take),
            ("vcodec", Take),
        ],
        // `BildIE._real_extract`'s dict.
        "bild" => &[
            ("id", Take),
            ("title", Take),
            ("description", Assign),
            ("formats", Take),
            ("thumbnail", Assign),
            // Processing drops a `None` duration.
            ("duration", Take),
        ],
        // `MastersIE._real_extract`'s dict.
        "masters" => &[
            ("id", Take),
            ("title", Take),
            ("formats", Take),
            ("subtitles", Dict),
            ("upload_date", Assign),
            ("thumbnails", List),
        ],
        // `SenIE._real_extract`'s dict.
        "sen" => &[
            ("id", Take),
            ("formats", Take),
            ("title", Take),
            ("description", Take),
            ("tags", Take),
        ],
        // `KommunetvIE._real_extract`'s dict.
        "kommunetv" => &[("id", Take), ("formats", Take), ("title", Take)],
        // `GraspopIE._real_extract`'s dict.
        "graspop" => &[
            ("id", Take),
            ("formats", Take),
            ("title", Take),
            ("thumbnail", Take),
        ],
        // `CozyTVIE._real_extract`'s dict.
        "cozytv" => &[
            ("id", Take),
            ("title", Take),
            ("uploader", Take),
            ("upload_date", Assign),
            ("was_live", Take),
            ("duration", Take),
            ("formats", Take),
            ("subtitles", Dict),
        ],
        // `PremiershipRugbyIE._real_extract`'s dict.
        "premiershiprugby" => &[
            ("id", Take),
            ("display_id", Take),
            ("title", Take),
            ("formats", Take),
            ("subtitles", Dict),
            ("thumbnail", Assign),
            ("duration", Take),
            ("tags", Assign),
            ("categories", List),
        ],
        // `CJSWIE._real_extract`'s dict.
        "cjsw" => &[
            ("id", Take),
            ("title", Take),
            ("description", Assign),
            ("formats", Take),
            ("series", Take),
            ("episode_id", Take),
        ],
        // `PlayerFmIE._real_extract`'s dict (its own format).
        "playerfm" => &[
            ("display_id", Take),
            ("vcodec", Take),
            ("id", Take),
            ("url", Take),
            ("title", Take),
            ("description", Take),
            ("duration", Take),
            ("thumbnail", Take),
            ("filesize", Take),
            ("timestamp", Take),
            ("creators", Take),
        ],
        // `RTRFMIE._real_extract`'s dict (its own format).
        "rtrfm" => &[
            ("id", Take),
            ("title", Take),
            ("series", Take),
            ("url", Take),
            ("release_date", Take),
            ("description", Assign),
        ],
        // `HitRecordIE._real_extract`'s dict (its own format).
        "hitrecord" => &[
            ("id", Take),
            ("url", Take),
            ("title", Take),
            ("description", Assign),
            ("duration", Take),
            ("timestamp", Assign),
            ("uploader", Assign),
            ("uploader_id", Assign),
            ("view_count", Assign),
            ("like_count", Assign),
            ("comment_count", Assign),
            ("tags", Assign),
        ],
        // `CultureUnpluggedIE._real_extract`'s dict (its own format).
        "cultureunplugged" => &[
            ("id", Take),
            ("display_id", Take),
            ("url", Take),
            ("title", Take),
            ("description", Assign),
            ("creator", Assign),
            ("duration", Take),
            ("view_count", Assign),
            ("thumbnails", List),
        ],
        // `ZhihuIE._real_extract`'s dict.
        "zhihu" => &[
            ("id", Take),
            ("title", Take),
            ("formats", Take),
            ("thumbnail", Assign),
            ("uploader", Assign),
            ("timestamp", Assign),
            ("uploader_id", Assign),
            ("uploader_url", Assign),
            ("duration", Take),
            ("view_count", Assign),
            ("like_count", Assign),
            ("comment_count", Assign),
        ],
        // `CrtvgIE._real_extract`'s dict.
        "crtvg" => &[
            ("id", Take),
            ("_old_archive_ids", Assign),
            ("formats", Take),
            ("title", Take),
            ("description", Assign),
            ("thumbnail", Assign),
        ],
        // `BreitBartIE._real_extract`'s dict.
        "breitbart" => &[
            ("id", Take),
            ("title", Take),
            ("description", Assign),
            ("thumbnail", Assign),
            ("age_limit", Assign),
            ("formats", Take),
        ],
        // `CGTNIE._real_extract`'s dict.
        "cgtn" => &[
            ("id", Take),
            ("title", Take),
            ("description", Assign),
            ("thumbnail", Assign),
            ("formats", Take),
            ("categories", Assign),
            ("creators", Assign),
            ("timestamp", Assign),
        ],
        // `JoveIE._real_extract`'s dict (its own format).
        "jove" => &[
            ("id", Take),
            ("title", Take),
            ("url", Take),
            ("thumbnail", Assign),
            ("description", Assign),
            ("upload_date", Assign),
            ("comment_count", Take),
        ],
        // `ParlviewIE._real_extract`'s dict.
        "parlview" => &[
            ("id", Take),
            ("formats", Take),
            ("subtitles", Dict),
            ("title", Take),
            ("description", Take),
            ("duration", Take),
            ("timestamp", Take),
            ("thumbnail", Take),
        ],
        // `EpochIE._real_extract`'s dict.
        "epoch" => &[
            ("id", Take),
            ("formats", Take),
            ("subtitles", Dict),
            ("title", Take),
        ],
        // `EbayIE._real_extract`'s dict.
        "ebay" => &[("id", Take), ("title", Take), ("formats", Take)],
        // `BitmovinIE._real_extract`'s dict.
        "bitmovin" => &[
            ("id", Take),
            ("formats", Take),
            ("subtitles", Dict),
            ("title", Take),
            ("thumbnail", Take),
        ],
        // `CrooksAndLiarsIE._real_extract`'s dict.
        "crooksandliars" => &[
            ("url", Take),
            ("id", Take),
            ("title", Take),
            ("description", Assign),
            ("thumbnail", Assign),
            ("timestamp", Assign),
            ("uploader", Assign),
            ("duration", Take),
            ("formats", Take),
        ],
        // `HuajiaoIE._real_extract`'s dict.
        "huajiao" => &[
            ("id", Take),
            ("title", Take),
            ("description", Assign),
            ("duration", Take),
            ("thumbnail", Assign),
            ("timestamp", Assign),
            ("uploader", Assign),
            ("uploader_id", Assign),
            ("formats", Take),
        ],
        // `ScreenRecIE._real_extract`'s dict.
        "screenrec" => &[
            ("id", Take),
            ("title", Take),
            ("description", Assign),
            ("thumbnail", Assign),
            ("formats", Take),
        ],
        // `Ku6IE._real_extract`'s dict (its own format).
        "ku6" => &[("id", Take), ("title", Take), ("url", Take)],
        // `FuyinTVIE._real_extract`'s dict (its own format).
        "fuyintv" => &[
            ("id", Take),
            ("title", Take),
            ("url", Take),
            ("ext", Take),
            ("description", Assign),
        ],
        // `MovingImageIE._real_extract`'s dict.
        "movingimage" => &[
            ("id", Take),
            ("formats", Take),
            ("title", Take),
            ("description", Assign),
            ("duration", Take),
            ("thumbnail", Assign),
        ],
        // `FathomIE._real_extract`'s dict.
        "fathom" => &[
            ("id", Take),
            ("formats", Take),
            ("title", Take),
            ("duration", Take),
            ("timestamp", Take),
        ],
        // `MaarivIE._real_extract`'s dict.
        "maariv" => &[
            ("id", Take),
            ("title", Take),
            ("duration", Take),
            ("timestamp", Take),
            ("formats", Take),
        ],
        // `HarpodeonIE._real_extract`'s dict (its own format).
        "harpodeon" => &[
            ("id", Take),
            ("title", Take),
            ("url", Take),
            ("http_headers", Take),
            ("description", Assign),
            ("creator", Assign),
            ("release_year", Assign),
        ],
        // `YfanefaIE._real_extract`'s dict.
        "yfanefa" => &[
            ("id", Take),
            ("title", Take),
            ("formats", Take),
            ("thumbnail", Take),
            ("duration", Take),
        ],
        // `GodResourceIE._real_extract`'s dict.
        "godresource" => &[
            ("id", Take),
            ("formats", Take),
            ("subtitles", Dict),
            ("title", Take),
            ("is_live", Take),
            ("thumbnail", Take),
            ("view_count", Take),
            ("channel", Take),
            ("channel_id", Take),
            ("timestamp", Take),
            ("modified_timestamp", Take),
        ],
        // `PopcornTVIE._real_extract`'s dict.
        "popcorntv" => &[
            ("id", Take),
            ("display_id", Take),
            ("title", Take),
            ("description", Assign),
            ("thumbnail", Assign),
            ("timestamp", Assign),
            ("duration", Take),
            ("view_count", Assign),
            ("formats", Take),
        ],
        // `BloggerIE._real_extract`'s dict.
        "blogger" => &[
            ("id", Take),
            ("title", Take),
            ("formats", Take),
            ("thumbnail", Assign),
            ("duration", Assign),
        ],
        // `TweakersIE._real_extract`'s dict.
        "tweakers" => &[
            ("id", Take),
            ("title", Take),
            ("description", Assign),
            ("thumbnail", Assign),
            ("duration", Assign),
            ("uploader_id", Assign),
            ("formats", Take),
        ],
        // `TwentyThreeVideoIE._real_extract`'s dict.
        "23video" => &[
            ("id", Take),
            ("title", Take),
            ("timestamp", Assign),
            ("duration", Assign),
            ("view_count", Assign),
            ("comment_count", Assign),
            ("uploader_id", Assign),
            ("uploader", Assign),
            ("thumbnails", List),
            ("formats", Take),
        ],
        // `MegaphoneIE._real_extract`'s dict.
        "megaphone" => &[
            ("id", Take),
            ("thumbnail", Assign),
            ("title", Take),
            ("creators", Assign),
            ("duration", Take),
            ("formats", Take),
        ],
        // `StreamableIE._real_extract`'s dict.
        "streamable" => &[
            ("id", Take),
            ("title", Take),
            ("description", Assign),
            ("thumbnail", Assign),
            ("uploader", Assign),
            ("timestamp", Assign),
            ("duration", Assign),
            ("view_count", Assign),
            ("formats", Take),
        ],
        // A Video.js embed's `{'formats', 'subtitles'}`, then the page fields.
        "videojs" => &[
            ("formats", Take),
            ("subtitles", Dict),
            ("extra_param_to_segment_url", Take),
            ("extra_param_to_key_url", Take),
            ("hls_aes", Take),
            ("live_status", Take),
            ("duration", Take),
            ("id", Take),
            ("title", Take),
            ("timestamp", Take),
            ("description", Take),
            ("thumbnail", Take),
            ("age_limit", Take),
        ],
        // A found manifest URL: the entry, the manifest's formats and
        // `_extra_manifest_info`, then the page fields.
        "generic-found-hls" => &[
            ("id", Take),
            ("uploader", Take),
            ("title", Take),
            ("age_limit", Take),
            ("http_headers", Take),
            ("formats", Take),
            ("subtitles", Dict),
            ("extra_param_to_segment_url", Take),
            ("extra_param_to_key_url", Take),
            ("hls_aes", Take),
            ("live_status", Take),
            ("duration", Take),
            ("timestamp", Take),
            ("description", Take),
            ("thumbnail", Take),
        ],
        "generic-found-dash" => &[
            ("id", Take),
            ("uploader", Take),
            ("title", Take),
            ("age_limit", Take),
            ("http_headers", Take),
            ("formats", Take),
            ("subtitles", Dict),
            ("extra_param_to_segment_url", Take),
            ("extra_param_to_key_url", Take),
            ("hls_aes", Take),
            ("timestamp", Take),
            ("description", Take),
            ("thumbnail", Take),
        ],
        // Several found manifest URLs: unmerged, `None` keys stay.
        "generic-found-hls-entry" => &[
            ("id", Take),
            ("uploader", Take),
            ("title", Take),
            ("age_limit", Assign),
            ("http_headers", Take),
            ("formats", Take),
            ("subtitles", Dict),
            ("extra_param_to_segment_url", Take),
            ("extra_param_to_key_url", Take),
            ("hls_aes", Assign),
            ("live_status", Take),
            ("duration", Take),
        ],
        "generic-found-dash-entry" => &[
            ("id", Take),
            ("uploader", Take),
            ("title", Take),
            ("age_limit", Assign),
            ("http_headers", Take),
            ("formats", Take),
            ("subtitles", Dict),
            ("extra_param_to_segment_url", Take),
            ("extra_param_to_key_url", Take),
            ("hls_aes", Assign),
        ],
        // One of several found URLs (a playlist entry keeps `age_limit`).
        "generic-found-entry" => &[
            ("id", Take),
            ("uploader", Take),
            ("title", Take),
            ("age_limit", Assign),
            ("http_headers", Take),
            ("url", Take),
        ],
        // A typed (Content-Type) HLS link; `_extra_manifest_info` adds
        // `hls_aes` and the "Checking m3u8 live status" duration.
        "generic-hls" => &[
            ("id", Assign),
            ("title", Assign),
            ("timestamp", Assign),
            ("formats", Take),
            ("subtitles", Dict),
            ("http_headers", Assign),
            ("extra_param_to_segment_url", Take),
            ("extra_param_to_key_url", Take),
            ("hls_aes", Assign),
            ("live_status", Take),
            ("duration", Take),
        ],
        // An `#EXTM3U` body without an HLS Content-Type.
        "generic-m3u" => &[
            ("id", Assign),
            ("title", Assign),
            ("timestamp", Assign),
            ("formats", Take),
            ("subtitles", Dict),
            ("extra_param_to_segment_url", Take),
            ("extra_param_to_key_url", Take),
            ("hls_aes", Assign),
            ("live_status", Take),
            ("duration", Take),
        ],
        "generic-dash" => &[
            ("id", Assign),
            ("title", Assign),
            ("timestamp", Assign),
            ("formats", Take),
            ("subtitles", Dict),
            ("live_status", Assign),
            ("extra_param_to_segment_url", Take),
            ("extra_param_to_key_url", Take),
            ("hls_aes", Assign),
        ],
        // `merge_dicts(embed, info_dict)` drops the embed's `None` values.
        "html5" => &[
            ("formats", Take),
            ("subtitles", Dict),
            ("thumbnail", Take),
            ("id", Take),
            ("title", Take),
            ("_old_archive_ids", Take),
            ("extractor", Take),
            ("extractor_key", Take),
            ("timestamp", Take),
            ("description", Take),
            ("thumbnail", Take),
            ("age_limit", Take),
        ],
        "html5-entry" => &[
            ("formats", Take),
            ("subtitles", Dict),
            ("thumbnail", Assign),
            ("id", Take),
            ("title", Take),
            ("_old_archive_ids", Take),
            ("extractor", Take),
            ("extractor_key", Take),
        ],
        _ => &[],
    };
    if untyped {
        // Upstream has no `formats`: the info dict is the only format.
        replay.ours.shift_remove("formats");
    }
    if untitled {
        let titled: Vec<(&str, Set)> = extractor
            .iter()
            .filter(|(key, _)| *key != "title")
            .copied()
            .collect();
        replay.steps(&titled);
    } else {
        replay.steps(extractor);
    }
    if let Some(original) = &original {
        replay.loaded(original);
    }
    // `_parse_smil`'s images are `{id, url, width, height}` dicts, 23video's
    // `{id, url, width, height, filesize}`.
    if (shape.contains("smil")
        || matches!(shape.as_str(), "23video" | "masters" | "cultureunplugged"))
        && let Some(Value::Array(thumbnails)) = replay.out.get_mut("thumbnails")
    {
        for thumbnail in thumbnails.iter_mut().filter_map(Value::as_object_mut) {
            let mut ordered = Map::new();
            let keys: &[&str] = if shape == "23video" {
                &["id", "url", "width", "height", "filesize"]
            } else if shape == "masters" {
                // `MastersIE`'s `{id, url}` image dicts.
                &["id", "url"]
            } else if shape == "cultureunplugged" {
                &["url", "id", "preference"]
            } else {
                &["id", "url", "width", "height"]
            };
            for key in keys {
                ordered.insert(
                    (*key).to_owned(),
                    thumbnail.shift_remove(*key).unwrap_or(Value::Null),
                );
            }
            ordered.append(thumbnail);
            *thumbnail = ordered;
        }
    }
    // Video.js tracks are `{url, name, http_headers}`, then `ext`.
    if shape == "videojs"
        && let Some(Value::Object(languages)) = replay.out.get_mut("subtitles")
    {
        for tracks in languages.values_mut().filter_map(Value::as_array_mut) {
            // `addRemoteTextTrack` tracks (manifest tracks keep theirs).
            for track in tracks
                .iter_mut()
                .filter_map(Value::as_object_mut)
                .filter(|track| track.contains_key("http_headers"))
            {
                let mut ordered = Map::new();
                for key in ["url", "name", "http_headers", "ext"] {
                    ordered.insert(
                        key.to_owned(),
                        track.shift_remove(key).unwrap_or(Value::Null),
                    );
                }
                ordered.append(track);
                *track = ordered;
            }
        }
    }
    // `merge_dicts(jwplayer_entry, info_dict)`: the entry's keys, then the
    // page's.
    // `NobelPrizeIE`: `{**json_ld, 'id', 'title', 'description',
    // 'duration'}` (an untyped result).
    if let Some(keys) = shape.strip_prefix("nobelprize;") {
        for key in keys.split(',').filter(|key| !key.is_empty()) {
            replay.set(
                key,
                if matches!(key, "title" | "description" | "duration") {
                    Assign
                } else {
                    Take
                },
            );
        }
    }
    // `AngelIE`: `merge_dicts(info_dict, json_ld)`.
    if let Some(keys) = shape.strip_prefix("angel;") {
        for (key, how) in [
            ("id", Take),
            ("title", Take),
            ("description", Take),
            ("formats", Take),
            ("subtitles", Dict),
            ("thumbnail", Take),
        ] {
            replay.set(key, how);
        }
        for key in keys.split(',').filter(|key| !key.is_empty()) {
            replay.take(key);
        }
    }
    // `JWPlatformIE`: the `_parse_jwplayer_data` entry itself (with its
    // `None` values).
    // `YoutubeIE._real_extract`'s dict, every key set (`None` included).
    if let Some(keys) = shape.strip_prefix("youtube;") {
        for key in keys.split(',') {
            replay.set(
                key,
                match key {
                    "formats" => Take,
                    "subtitles" | "automatic_captions" => Dict,
                    _ => Assign,
                },
            );
        }
        // `_extract_thumbnails`' `{url, height, width}` (only keys present:
        // generated names have just a URL), then `preference` and the ID.
        // `process_language`'s `{ext, url, name, impersonate, client}` tracks.
        for key in ["subtitles", "automatic_captions"] {
            if let Some(Value::Object(languages)) = replay.out.get_mut(key) {
                for tracks in languages.values_mut().filter_map(Value::as_array_mut) {
                    for track in tracks.iter_mut().filter_map(Value::as_object_mut) {
                        let mut ordered = Map::new();
                        for key in ["ext", "url", "name", "impersonate", "__yt_dlp_client"] {
                            if let Some(value) = track.shift_remove(key) {
                                ordered.insert(key.to_owned(), value);
                            }
                        }
                        ordered.append(track);
                        *track = ordered;
                    }
                }
            }
        }
        if let Some(Value::Array(thumbnails)) = replay.out.get_mut("thumbnails") {
            for thumbnail in thumbnails.iter_mut().filter_map(Value::as_object_mut) {
                let mut ordered = Map::new();
                for key in ["url", "height", "width", "preference", "id"] {
                    if let Some(value) = thumbnail.shift_remove(key) {
                        ordered.insert(key.to_owned(), value);
                    }
                }
                ordered.append(thumbnail);
                *thumbnail = ordered;
            }
        }
    }
    // An extractor's own dict (`dict;KEYS[;thumbnail=KEYS]`), every key set.
    if let Some(spec) = shape.strip_prefix("dict;") {
        let mut parts = spec.split(';');
        let keys = parts.next().unwrap_or_default();
        let mut thumbnail_keys = None;
        let mut subtitle_keys = None;
        let mut chapter_keys = None;
        for part in parts {
            if let Some(found) = part.strip_prefix("chapter=") {
                chapter_keys = Some(found);
            }
            if let Some(found) = part.strip_prefix("thumbnail=") {
                thumbnail_keys = Some(found);
            } else if let Some(found) = part.strip_prefix("subtitle=") {
                subtitle_keys = Some(found);
            }
        }
        for key in keys.split(',') {
            replay.set(
                key,
                match key {
                    "formats" => Take,
                    "subtitles" | "automatic_captions" => Dict,
                    _ => Assign,
                },
            );
        }
        // The extractor's chapter keys, then processing's `end_time`.
        if let Some(chapter_keys) = chapter_keys
            && let Some(Value::Array(chapters)) = replay.out.get_mut("chapters")
        {
            for chapter in chapters.iter_mut().filter_map(Value::as_object_mut) {
                let mut ordered = Map::new();
                for key in chapter_keys.split(',') {
                    if let Some(value) = chapter.shift_remove(key) {
                        ordered.insert(key.to_owned(), value);
                    }
                }
                ordered.append(chapter);
                *chapter = ordered;
            }
        }
        // The extractor's subtitle track keys, then processing's `ext`.
        if let Some(subtitle_keys) = subtitle_keys {
            for key in ["subtitles", "automatic_captions"] {
                if let Some(Value::Object(languages)) = replay.out.get_mut(key) {
                    for tracks in languages.values_mut().filter_map(Value::as_array_mut) {
                        // Manifest tracks keep their parser's order.
                        for track in tracks
                            .iter_mut()
                            .filter_map(Value::as_object_mut)
                            .filter(|track| !track.contains_key("manifest_url"))
                        {
                            let mut ordered = Map::new();
                            for key in subtitle_keys.split(',') {
                                if let Some(value) = track.shift_remove(key) {
                                    ordered.insert(key.to_owned(), value);
                                }
                            }
                            ordered.append(track);
                            *track = ordered;
                        }
                    }
                }
            }
        }
        // The extractor's thumbnail keys, then `_sanitize_thumbnails`' `id`
        // and `resolution`.
        if let Some(thumbnail_keys) = thumbnail_keys
            && let Some(Value::Array(thumbnails)) = replay.out.get_mut("thumbnails")
        {
            for thumbnail in thumbnails.iter_mut().filter_map(Value::as_object_mut) {
                let mut ordered = Map::new();
                for key in thumbnail_keys.split(',') {
                    if let Some(value) = thumbnail.shift_remove(key) {
                        ordered.insert(key.to_owned(), value);
                    }
                }
                ordered.append(thumbnail);
                *thumbnail = ordered;
            }
        }
    }
    if let Some(keys) = shape.strip_prefix("jwplatform;") {
        for key in keys.split(',') {
            replay.set(key, if key == "subtitles" { Dict } else { Assign });
        }
    }
    if let Some(keys) = shape.strip_prefix("jwplayer;") {
        for key in keys.split(',') {
            replay.set(key, if key == "subtitles" { Dict } else { Take });
        }
        for key in [
            "title",
            "timestamp",
            "description",
            "thumbnail",
            "age_limit",
        ] {
            replay.take(key);
        }
    }
    let held = |key: &str| {
        (untitled && key == "title")
            || transparent.iter().any(|outer| outer == key)
            || derived.iter().any(|derived| derived == key)
            || PROCESSED_INFO_KEYS.contains(&key)
            || (untyped && PROCESSED_FORMAT_KEYS.contains(&key))
            || selected.iter().any(|selected| selected == key)
    };
    replay.rest_except(&held);
    // `add_extra_info`: the URL fields, or a playlist entry's
    // `ChainMap(entry extras, _playlist_infodict(...))` keys.
    if entry {
        for (key, always) in [
            ("playlist_count", Take),
            ("playlist", Assign),
            ("playlist_id", Assign),
            ("playlist_title", Assign),
            ("playlist_uploader", Assign),
            ("playlist_uploader_id", Assign),
            ("playlist_channel", Assign),
            ("playlist_channel_id", Assign),
            ("playlist_webpage_url", Assign),
            ("n_entries", Take),
            ("webpage_url", Take),
            ("webpage_url_basename", Take),
            ("webpage_url_domain", Take),
            ("playlist_index", Take),
            ("__last_playlist_index", Take),
            ("extractor", Take),
            ("extractor_key", Take),
            ("playlist_autonumber", Take),
        ] {
            replay.set(key, always);
        }
    } else {
        // A `url_result` chain (a followed redirect) passes `original_url`
        // as extra info before `add_default_extra_info`'s keys.
        if !transparent_entry
            && replay.get("original_url").is_some()
            && (transparent_result || replay.get("original_url") != replay.get("webpage_url"))
        {
            replay.take("original_url");
        }
        for key in [
            "webpage_url",
            "original_url",
            "webpage_url_basename",
            "webpage_url_domain",
            "extractor",
            "extractor_key",
        ] {
            replay.take(key);
        }
        for key in &transparent {
            replay.take(key);
        }
        if transparent_entry {
            // The playlist entry's extra info.
            for (key, how) in [
                ("playlist_count", Take),
                ("playlist", Assign),
                ("playlist_id", Assign),
                ("playlist_title", Assign),
                ("playlist_uploader", Assign),
                ("playlist_uploader_id", Assign),
                ("playlist_channel", Assign),
                ("playlist_channel_id", Assign),
                ("playlist_webpage_url", Assign),
                ("n_entries", Take),
                ("playlist_index", Take),
                ("__last_playlist_index", Take),
                ("playlist_autonumber", Take),
            ] {
                replay.set(key, how);
            }
        } else {
            // `process_video_result`: not part of a playlist.
            replay.assign("playlist");
            replay.assign("playlist_index");
        }
    }
    // `process_video_result`: `_sanitize_thumbnails`, then `thumbnail`
    // and `_fill_common_fields`.
    replay.take("thumbnails");
    replay.take("thumbnail");
    replay.assign("display_id");
    replay.assign("fulltitle");
    replay.take("title");
    for key in [
        "duration_string",
        "upload_date",
        "release_date",
        "modified_date",
    ] {
        replay.take(key);
    }
    replay.assign("release_year");
    replay.take("live_status");
    replay.take("is_live");
    replay.take("was_live");
    // Titles generated from `chapter_number`/`season_number`/`episode_number`.
    for key in ["chapter", "season", "episode"] {
        replay.take(key);
    }
    for (old, new) in crate::info::MULTIVALUE_FIELDS {
        replay.take(new);
        replay.take(old);
    }
    replay.assign("requested_subtitles");
    replay.assign("_has_drm");
    if untyped {
        replay.format_steps();
    }
    // The first `prepare_outtmpl` sets `epoch` before format selection.
    replay.take("epoch");
    // `-J`: `requested_downloads`, each without the keys whose values equal
    // the info dict's ("Remove copied info"); the selected format's keys are
    // merged afterwards (`info_dict.update(best_format)`), so the dict still
    // has the extractor's values (`None`) for them.
    if let Some(Value::Array(downloads)) = replay.ours.shift_remove("requested_downloads") {
        // Keys an extractor's own dict set (a format's `duration` too) held
        // that value, not `None`; this port keeps one value per key, the
        // (equal) format's. The info's own `http_headers` always differ from
        // a format's processed headers.
        let own_keys: Vec<&str> = shape
            .strip_prefix("dict;")
            .or_else(|| shape.strip_prefix("dict-untitled;"))
            .and_then(|rest| rest.split(';').next())
            .map(|keys| {
                keys.split(',')
                    .filter(|key| *key != "http_headers")
                    .collect()
            })
            .unwrap_or_default();
        let before: Map<String, Value> = replay
            .out
            .iter()
            .map(|(key, value)| {
                // A loaded dict already held its selected format's values.
                let value = match original.as_ref().and_then(|original| original.get(key)) {
                    Some(loaded) if selected.contains(key) => loaded.clone(),
                    _ if selected.contains(key) && !own_keys.contains(&key.as_str()) => Value::Null,
                    _ => value.clone(),
                };
                (key.clone(), value)
            })
            .collect();
        let downloads = downloads
            .into_iter()
            .map(|download| match download {
                Value::Object(download) => Value::Object(
                    download
                        .into_iter()
                        .filter(|(key, value)| before.get(key).unwrap_or(&Value::Null) != value)
                        .collect(),
                ),
                other => other,
            })
            .collect();
        replay
            .out
            .insert("requested_downloads".into(), Value::Array(downloads));
    }
    // `info_dict.update(format)` (or `_merge`'s dict, whose keys are all
    // assigned): the format's values, including its `None`s.
    for key in &selected {
        match selected_format.get(key) {
            Some(value) if !replay.ours.contains_key(key) => {
                replay.out.insert(key.clone(), value.clone());
            }
            _ if merged => replay.assign(key),
            _ => {
                replay.take(key);
                // The format's int where this port keeps an equal float.
                if let (Some(Value::Number(format)), Some(Value::Number(ours))) =
                    (selected_format.get(key), replay.out.get(key))
                    && format.is_i64()
                    && ours.as_f64() == format.as_f64()
                {
                    replay
                        .out
                        .insert(key.clone(), Value::Number(format.clone()));
                }
            }
        }
    }
    for key in [
        "_filename",
        "filename",
        "__postprocessors",
        "__real_download",
        "__files_to_merge",
        "__finaldir",
        "filepath",
        "__write_download_archive",
        "__last_playlist_index",
        "_type",
        "_version",
    ] {
        replay.take(key);
    }
    let mut out = replay.finish();
    // A merge of int-`tbr` (F4M/ISM) formats sums ints.
    let int_rates = out
        .get("requested_formats")
        .and_then(Value::as_array)
        .is_some_and(|formats| {
            !formats.is_empty()
                // `sum(f.get('tbr') or 0 ...)`: missing rates add an int 0.
                && formats
                    .iter()
                    .any(|format| format.get("tbr").is_some_and(Value::is_i64))
                && formats.iter().all(|format| {
                    format
                        .get("tbr")
                        .is_none_or(|tbr| tbr.is_i64() || tbr.is_null())
                })
        });
    // No rates at all: the sum is the int 0.
    let no_rates = out
        .get("requested_formats")
        .and_then(Value::as_array)
        .is_some_and(|formats| {
            !formats.is_empty()
                && formats
                    .iter()
                    .all(|format| format.get("tbr").is_none_or(Value::is_null))
        });
    if no_rates
        && let Some(tbr) = out.get("tbr").and_then(Value::as_f64)
        && tbr == 0.0
    {
        out.insert("tbr".into(), 0.into());
    }
    // `'abr': 64` (and the `tbr` it gives) of ThisAmericanLifeIE.
    if int_rates || shape == "thisamericanlife" {
        for key in ["tbr", "vbr", "abr"] {
            if let Some(rate) = out.get(key).and_then(Value::as_f64)
                && rate.fract() == 0.0
            {
                out.insert(key.into(), (rate as i64).into());
            }
        }
    }
    // `_merge`'s `vbr`/`abr` are the video/audio format's own (int) values.
    let requested: Vec<Map<String, Value>> = out
        .get("requested_formats")
        .and_then(Value::as_array)
        .map(|formats| {
            formats
                .iter()
                .filter_map(Value::as_object)
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    for (key, codec) in [("vbr", "vcodec"), ("abr", "acodec")] {
        let own = requested.iter().find_map(|format| {
            (format.get(codec).and_then(Value::as_str) != Some("none"))
                .then(|| format.get(key).filter(|rate| rate.is_i64()).cloned())
                .flatten()
        });
        if let Some(own) = own
            && out.get(key).and_then(Value::as_f64) == own.as_f64()
        {
            out.insert(key.into(), own);
        }
    }
    out
}

/// A generic webpage playlist dict (`-J`) in upstream order.
/// `YoutubeTabIE`'s `playlist_result(entries, **metadata)`: the tab
/// metadata's keys (`KEYS`), then the processing fields (`updated`: after
/// `_real_extract`'s `extractor_key`/`extractor`/`webpage_url` update).
/// `KEYS[;after=KEYS]`: the dict's keys before `_type`/`entries`, and those
/// a later `result.update(...)` adds after them.
fn youtube_tab_playlist(
    mut map: Map<String, Value>,
    keys: &str,
    updated: bool,
) -> Map<String, Value> {
    let (keys, after) = keys.split_once(";after=").unwrap_or((keys, ""));
    // `playlist_result(entries, id)` without a title has no `title` key.
    if !keys
        .split(',')
        .chain(after.split(','))
        .any(|key| key == "title")
        && map
            .get("title")
            .is_some_and(|title| title.is_null() || title.as_str() == Some(""))
    {
        map.shift_remove("title");
    }
    let mut replay = Replay::new(map);
    for key in keys.split(',') {
        replay.set(key, Assign);
    }
    replay.take("_type");
    replay.take("entries");
    for key in after.split(',').filter(|key| !key.is_empty()) {
        replay.set(key, Assign);
    }
    let late: &[&str] = if updated {
        &[
            "extractor_key",
            "extractor",
            "webpage_url",
            "original_url",
            "webpage_url_basename",
            "webpage_url_domain",
        ]
    } else {
        &[
            "webpage_url",
            "original_url",
            "webpage_url_basename",
            "webpage_url_domain",
            "extractor",
            "extractor_key",
        ]
    };
    for key in late {
        replay.take(key);
    }
    // `_fill_common_fields(final=False)`.
    replay.take("upload_date");
    replay.assign("release_year");
    for (old, new) in crate::info::MULTIVALUE_FIELDS {
        replay.take(old);
        replay.take(new);
    }
    for key in ["requested_entries", "playlist_count", "epoch"] {
        replay.take(key);
    }
    replay.set("__files_to_move", Dict);
    replay.take("_version");
    replay.finish()
}

pub(crate) fn playlist(mut map: Map<String, Value>) -> Map<String, Value> {
    let Some(Value::String(shape)) = map.shift_remove(INFO_SHAPE) else {
        return map;
    };
    if let Some(keys) = shape.strip_prefix("youtube-tab;") {
        return youtube_tab_playlist(map, keys, true);
    }
    if let Some(keys) = shape
        .strip_prefix("youtube-tab-plain;")
        .or_else(|| shape.strip_prefix("playlist-dict;"))
    {
        return youtube_tab_playlist(map, keys, false);
    }
    if !matches!(
        shape.as_str(),
        "html5-playlist" | "rss-playlist" | "xspf-playlist"
    ) {
        return map;
    }
    let mut replay = Replay::new(map);
    let steps: &[(&str, Set)] = if shape == "xspf-playlist" {
        // `playlist_result(entries, video_id)`: no title.
        replay.ours.shift_remove("title");
        &[("id", Take), ("_type", Take), ("entries", Take)]
    } else if shape == "rss-playlist" {
        // `_extract_rss`'s dict.
        &[
            ("_type", Take),
            ("id", Take),
            ("title", Assign),
            ("description", Assign),
            ("entries", Take),
        ]
    } else {
        // `playlist_result(entries, **info_dict)` of GenericIE's page fields.
        &[
            ("id", Take),
            ("title", Take),
            ("timestamp", Assign),
            ("description", Assign),
            ("thumbnail", Assign),
            ("age_limit", Assign),
            ("_type", Take),
            ("entries", Take),
        ]
    };
    for (key, always) in steps {
        replay.set(key, *always);
    }
    let late = [
        "webpage_url",
        "original_url",
        "webpage_url_basename",
        "webpage_url_domain",
        "extractor",
        "extractor_key",
        "upload_date",
        "release_year",
        "playlist_count",
        "requested_entries",
        "epoch",
        "_version",
    ];
    replay.rest_except(&|key| late.contains(&key));
    for key in [
        "webpage_url",
        "original_url",
        "webpage_url_basename",
        "webpage_url_domain",
        "extractor",
        "extractor_key",
        "upload_date",
    ] {
        replay.take(key);
    }
    // `_fill_common_fields(final=False)`.
    replay.assign("release_year");
    for key in ["playlist_count", "requested_entries", "epoch"] {
        replay.take(key);
    }
    // Upstream's `-J` also prints the playlist's (empty) files to move.
    replay.set("__files_to_move", Dict);
    replay.take("_version");
    replay.finish()
}

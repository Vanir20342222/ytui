//! Info JSON sidecars: upstream's clean `sanitize_info`, `.info.json` naming
//! and atomic writes without overwriting existing files.
use crate::{Error, PlaylistCounts, PlaylistInfo, Result, VideoInfo};
use serde_json::{Map, Value};
use std::{
    io::{self, Write},
    path::{Path, PathBuf},
};

/// Keys removed by upstream's default `clean_infojson`, besides `__` keys and nulls.
const PRIVATE_KEYS: &[&str] = &[
    "requested_downloads",
    "requested_formats",
    "requested_subtitles",
    "requested_entries",
    "entries",
    "filepath",
    "_filename",
    "filename",
    "infojson_filename",
    "original_url",
    "playlist_autonumber",
];

/// Result of an info JSON write; existing files are kept as upstream does by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InfoJsonWrite {
    Written,
    AlreadyPresent,
}

/// Upstream `replace_extension(name, "info.json", ext)`: replace the final
/// extension when it matches `ext` (or any extension when `ext` is absent),
/// otherwise append. Extension splitting follows Python's `os.path.splitext`.
pub fn info_json_path(path: &Path, ext: Option<&str>) -> PathBuf {
    let Some(text) = path.to_str() else {
        let mut name = path.as_os_str().to_owned();
        name.push(".info.json");
        return name.into();
    };
    // Split the text directly so literal template separators are preserved.
    let start = text
        .rfind(std::path::is_separator)
        .map_or(0, |index| index + 1);
    let name = &text[start..];
    let stem = match splitext(name) {
        Some((stem, real)) if ext.is_none_or(|ext| real == ext) => stem,
        _ => name,
    };
    format!("{}{stem}.info.json", &text[..start]).into()
}

/// Python `splitext` on one component: the last dot preceded by a non-dot.
fn splitext(name: &str) -> Option<(&str, &str)> {
    let dot = name.rfind('.')?;
    name[..dot]
        .bytes()
        .any(|byte| byte != b'.')
        .then(|| (&name[..dot], &name[dot + 1..]))
}

/// The cleaned video info dictionary, with upstream's `_type` and `epoch` defaults.
/// `_version` is omitted because this is not an upstream release.
pub fn video_info_json(info: &VideoInfo, epoch: u64) -> Result<Value> {
    video_info_json_with(info, epoch, true)
}

/// [`video_info_json`] with upstream `clean_infojson` (`--no-clean-info-json`
/// keeps `None` values and private keys).
pub fn video_info_json_with(info: &VideoInfo, epoch: u64, clean: bool) -> Result<Value> {
    finish(raw_value(info)?, "video", epoch, clean)
}

/// The cleaned playlist dictionary written before its entries are processed.
pub fn playlist_info_json(
    info: &PlaylistInfo,
    counts: PlaylistCounts,
    epoch: u64,
) -> Result<Value> {
    playlist_info_json_with(info, counts, epoch, true)
}

/// [`playlist_info_json`] with upstream `clean_infojson`.
pub fn playlist_info_json_with(
    info: &PlaylistInfo,
    counts: PlaylistCounts,
    epoch: u64,
    clean: bool,
) -> Result<Value> {
    let mut value = raw_value(info)?;
    if let Value::Object(map) = &mut value {
        map.insert("playlist_count".into(), counts.playlist_count.into());
    }
    finish(value, "playlist", epoch, clean)
}

fn raw_value(value: impl serde::Serialize) -> Result<Value> {
    serde_json::to_value(value).map_err(|error| Error::InfoJson(error.to_string()))
}

fn to_value(value: impl serde::Serialize) -> Result<Value> {
    let mut value = raw_value(value)?;
    upstream_shape(&mut value);
    Ok(value)
}

/// Format keys this port uses internally that upstream's dictionaries lack.
const INTERNAL_FORMAT_KEYS: [&str; 4] = [
    "stream_kind",
    "audio_group",
    "audio_default",
    "audio_autoselect",
];

/// Shape a serialized info dict (video, or playlist with entries) like
/// upstream's: internal format keys are dropped, and HLS formats carry no
/// fragments (HlsFD reads them from the playlist when downloading).
pub fn upstream_shape(value: &mut Value) {
    fn format(map: &mut Map<String, Value>) {
        for key in INTERNAL_FORMAT_KEYS {
            map.shift_remove(key);
        }
        let protocol = map.get("protocol").and_then(Value::as_str).unwrap_or("");
        if protocol.starts_with("m3u8") {
            map.shift_remove("fragments");
        } else if let Some(Value::Array(fragments)) = map.get_mut("fragments") {
            fragments
                .iter_mut()
                .filter_map(Value::as_object_mut)
                .for_each(upstream_fragment);
        }
        // `_fill_sorting_fields` sets `vbr`/`abr` to the integer 0 for a
        // missing stream.
        for key in ["vbr", "abr"] {
            if map.get(key).and_then(Value::as_f64) == Some(0.0) {
                map.insert(key.into(), 0.into());
            }
        }
    }
    // Upstream fragment dicts: `path` (relative to `fragment_base_url`) or
    // `url`, `duration`, and `byte_range` as `start`/`end` (exclusive).
    fn upstream_fragment(fragment: &mut Map<String, Value>) {
        let mut shaped = Map::new();
        match fragment.shift_remove("path") {
            Some(path) => {
                shaped.insert("path".into(), path);
            }
            None => {
                if let Some(url) = fragment.shift_remove("url") {
                    shaped.insert("url".into(), url);
                }
            }
        }
        if let Some(duration) = fragment.shift_remove("duration") {
            shaped.insert("duration".into(), duration);
        }
        match fragment.shift_remove("byte_range") {
            Some(Value::Object(range)) if !range.contains_key("start") => {
                let offset = range.get("offset").and_then(Value::as_u64).unwrap_or(0);
                let length = range.get("length").and_then(Value::as_u64).unwrap_or(0);
                shaped.insert(
                    "byte_range".into(),
                    Value::Object(Map::from_iter([
                        ("start".to_owned(), offset.into()),
                        ("end".to_owned(), offset.saturating_add(length).into()),
                    ])),
                );
            }
            Some(range) => {
                shaped.insert("byte_range".into(), range);
            }
            None => {}
        }
        fragment.shift_remove("url");
        // Other keys (HLS encryption) are not upstream's; DASH has none.
        *fragment = shaped;
    }
    let Some(map) = value.as_object_mut() else {
        return;
    };
    if let Some(Value::Array(entries)) = map.get_mut("entries") {
        entries.iter_mut().for_each(upstream_shape);
        *map = crate::info_order::playlist(std::mem::take(map));
        return;
    }
    // `-J`'s processed downloads are each shaped like a `-j` dict.
    if let Some(Value::Array(downloads)) = map.get_mut("requested_downloads") {
        downloads.iter_mut().for_each(upstream_shape);
    }
    // The top level is the merged selected format.
    format(map);
    for key in ["formats", "requested_formats"] {
        if let Some(Value::Array(items)) = map.get_mut(key) {
            for item in items.iter_mut() {
                if let Value::Object(item) = item {
                    format(item);
                    *item = crate::info_order::format(std::mem::take(item));
                }
            }
        }
    }
    // Subtitle dicts (DASH text tracks have upstream's shape).
    for key in ["subtitles", "automatic_captions", "requested_subtitles"] {
        if let Some(Value::Object(languages)) = map.get_mut(key) {
            for tracks in languages.values_mut() {
                let tracks = match tracks {
                    Value::Array(tracks) => tracks.iter_mut().collect::<Vec<_>>(),
                    track => vec![track],
                };
                for track in tracks {
                    if let Value::Object(track) = track {
                        if let Some(Value::Array(fragments)) = track.get_mut("fragments") {
                            fragments
                                .iter_mut()
                                .filter_map(Value::as_object_mut)
                                .for_each(upstream_fragment);
                        }
                        *track = crate::info_order::format(std::mem::take(track));
                    }
                }
            }
        }
    }
    // `_merge`'s dict has no URL or headers of its own (the info keeps
    // headers its extractor set).
    if map
        .get("requested_formats")
        .and_then(Value::as_array)
        .is_some_and(|formats| !formats.is_empty())
    {
        map.shift_remove("url");
    }
    *map = crate::info_order::video(std::mem::take(map));
}

/// A video's info dict shaped like upstream's ([`upstream_shape`]).
pub fn info_value(info: &VideoInfo) -> Result<Value> {
    to_value(info)
}

/// Any serializable info (a playlist) shaped like upstream's.
pub(crate) fn to_shaped_value(value: impl serde::Serialize) -> Result<Value> {
    to_value(value)
}

fn finish(value: Value, kind: &str, epoch: u64, cleaned: bool) -> Result<Value> {
    let Value::Object(mut map) = value else {
        return Err(Error::InfoJson("info is not a JSON object".into()));
    };
    map.entry("epoch").or_insert(epoch.into());
    map.entry("_type").or_insert(kind.into());
    let mut value = Value::Object(map);
    upstream_shape(&mut value);
    Ok(if cleaned { clean(value) } else { value })
}

fn clean(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .filter(|(key, value)| {
                    !value.is_null()
                        && !key.starts_with("__")
                        && !PRIVATE_KEYS.contains(&key.as_str())
                })
                .map(|(key, value)| (key, clean(value)))
                .collect::<Map<_, _>>(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(clean).collect()),
        value => value,
    }
}

/// Python `json.dump(obj, ensure_ascii=False)` bytes: `", "` and `": "` separators.
pub fn to_python_json(value: &Value) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut serializer = serde_json::Serializer::with_formatter(&mut bytes, PythonFormatter);
    serde::Serialize::serialize(value, &mut serializer)
        .map_err(|error| Error::InfoJson(error.to_string()))?;
    Ok(bytes)
}

struct PythonFormatter;

impl serde_json::ser::Formatter for PythonFormatter {
    fn begin_array_value<W: ?Sized + Write>(
        &mut self,
        writer: &mut W,
        first: bool,
    ) -> io::Result<()> {
        if first {
            Ok(())
        } else {
            writer.write_all(b", ")
        }
    }
    fn begin_object_key<W: ?Sized + Write>(
        &mut self,
        writer: &mut W,
        first: bool,
    ) -> io::Result<()> {
        if first {
            Ok(())
        } else {
            writer.write_all(b", ")
        }
    }
    fn begin_object_value<W: ?Sized + Write>(&mut self, writer: &mut W) -> io::Result<()> {
        writer.write_all(b": ")
    }
}

/// Write `value` to `path` unless a file already exists there. Parent directories
/// are created, data is synced in a sibling temporary file, and publication
/// never replaces a concurrently created destination.
pub fn write_info_json(path: &Path, value: &Value) -> Result<InfoJsonWrite> {
    write_info_json_with(path, value, false)
}

/// Like [`write_info_json`]; with `overwrite`, an existing regular file is
/// atomically replaced (upstream's default `overwrites` for metadata files).
pub fn write_info_json_with(path: &Path, value: &Value, overwrite: bool) -> Result<InfoJsonWrite> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if overwrite && metadata.is_file() => {}
        Ok(_) if std::fs::metadata(path).is_ok_and(|metadata| metadata.is_file()) => {
            return Ok(InfoJsonWrite::AlreadyPresent);
        }
        Ok(_) => {
            return Err(Error::InfoJson(format!(
                "destination is not a regular file: {}",
                path.display()
            )));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let bytes = to_python_json(value)?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut builder = tempfile::Builder::new();
    builder.prefix(".yt-dlp-rs-").suffix(".info.json.tmp");
    // Like upstream, publish with the default 0666 mode reduced by the umask.
    #[cfg(unix)]
    builder.permissions(std::os::unix::fs::PermissionsExt::from_mode(0o666));
    let mut temporary = builder.tempfile_in(parent)?;
    temporary.write_all(&bytes)?;
    temporary.as_file().sync_all()?;
    if overwrite {
        temporary.persist(path).map_err(|error| error.error)?;
        return Ok(InfoJsonWrite::Written);
    }
    match temporary.persist_noclobber(path) {
        Ok(_) => Ok(InfoJsonWrite::Written),
        Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
            Err(Error::OutputExists(path.into()))
        }
        Err(error) => Err(error.error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn info_json_names_follow_replace_extension() {
        let cases = [
            ("a/Title [id].mp4", Some("mp4"), "a/Title [id].info.json"),
            ("Title.webm", Some("mp4"), "Title.webm.info.json"),
            ("Title", Some("mp4"), "Title.info.json"),
            ("Title [x].NA", None, "Title [x].info.json"),
            ("Title.", None, "Title.info.json"),
            ("Title.", Some(""), "Title.info.json"),
            (".hidden", None, ".hidden.info.json"),
            ("..foo", Some("foo"), "..foo.info.json"),
            ("a..mp4", Some("mp4"), "a..info.json"),
            ("dir.d/name", None, "dir.d/name.info.json"),
            ("dir/", None, "dir/.info.json"),
        ];
        for (path, ext, expected) in cases {
            assert_eq!(
                info_json_path(Path::new(path), ext),
                PathBuf::from(expected),
                "{path}"
            );
        }
    }

    #[test]
    fn cleaning_removes_private_keys_and_nulls_recursively_and_adds_defaults() {
        let value = json!({
            "id": "x", "requested_subtitles": {"en": {}}, "__private": 1,
            "playlist_autonumber": 2, "nested": [{"filepath": "p", "keep": null, "ok": 1}],
            "playlist_index": 3,
        });
        let value = finish(value, "video", 7, true).unwrap();
        assert_eq!(
            value,
            json!({"id": "x", "nested": [{"ok": 1}], "playlist_index": 3, "epoch": 7, "_type": "video"})
        );
        let order: Vec<_> = value.as_object().unwrap().keys().cloned().collect();
        assert_eq!(order, ["id", "nested", "playlist_index", "epoch", "_type"]);
    }

    #[test]
    fn python_json_separators_and_unescaped_unicode() {
        let value = json!({"t": "Ép\u{1}\"/", "a": [1, 2.5, true], "o": {}});
        assert_eq!(
            String::from_utf8(to_python_json(&value).unwrap()).unwrap(),
            r#"{"t": "Ép\u0001\"/", "a": [1, 2.5, true], "o": {}}"#
        );
    }

    #[test]
    fn writes_are_atomic_and_never_replace_existing_files() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nested/x.info.json");
        assert_eq!(
            write_info_json(&path, &json!({"a": 1})).unwrap(),
            InfoJsonWrite::Written
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), r#"{"a": 1}"#);
        assert_eq!(
            write_info_json(&path, &json!({"a": 2})).unwrap(),
            InfoJsonWrite::AlreadyPresent
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), r#"{"a": 1}"#);
        let entries: Vec<_> = std::fs::read_dir(path.parent().unwrap()).unwrap().collect();
        assert_eq!(entries.len(), 1);
        std::fs::create_dir(directory.path().join("dir.info.json")).unwrap();
        assert!(write_info_json(&directory.path().join("dir.info.json"), &json!({})).is_err());
    }

    #[test]
    fn saved_upstream_cases_match() {
        let fixture: Value =
            serde_json::from_str(include_str!("../tests/fixtures/info_json/cases.json")).unwrap();
        for case in fixture["filenames"].as_array().unwrap() {
            let path = info_json_path(
                Path::new(case["filename"].as_str().unwrap()),
                case["ext"].as_str(),
            );
            assert_eq!(path.to_str().unwrap(), case["expected"], "{}", case["name"]);
        }
        for case in fixture["playlist_names"].as_array().unwrap() {
            let playlist: PlaylistInfo = serde_json::from_value(json!({
                "entries": [],
                "id": case["playlist"]["id"], "title": case["playlist"]["title"],
                "webpage_url": case["playlist"]["webpage_url"],
                "extractor": case["playlist"]["extractor"],
                "extractor_key": case["playlist"]["extractor_key"],
            }))
            .unwrap();
            let counts = PlaylistCounts {
                playlist_count: case["playlist_count"].as_u64().unwrap() as usize,
                n_entries: Some(case["n_entries"].as_u64().unwrap() as usize),
                last_playlist_index: case["last_playlist_index"].as_u64().unwrap() as usize,
            };
            let options = crate::output::TemplateOptions {
                na_placeholder: case["na_placeholder"].as_str().unwrap().into(),
                ..Default::default()
            };
            let name = crate::output::render_playlist_template_with_options(
                case["template"].as_str().unwrap(),
                &playlist,
                counts,
                &options,
            )
            .unwrap();
            let path = info_json_path(&name, None);
            assert_eq!(path.to_str().unwrap(), case["expected"], "{}", case["name"]);
        }
        for case in fixture["sanitize"].as_array().unwrap() {
            let value = finish(case["info"].clone(), "video", 0, true).unwrap();
            assert_eq!(value, case["expected"], "{}", case["name"]);
            let keys = |value: &Value| {
                value
                    .as_object()
                    .unwrap()
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
            };
            assert_eq!(keys(&value), keys(&case["expected"]), "{}", case["name"]);
        }
        for case in fixture["json_text"].as_array().unwrap() {
            let text = String::from_utf8(to_python_json(&case["value"]).unwrap()).unwrap();
            assert_eq!(text, case["expected"], "{}", case["name"]);
        }
        // Source-only: upstream's _version describes its own release.
        let version = &fixture["source_only"][0];
        let value = finish(version["info"].clone(), "video", 0, true).unwrap();
        assert!(
            value
                .get(version["upstream_extra_key"].as_str().unwrap())
                .is_none()
        );
    }
}

/// Upstream `sanitize_info(info, remove_private_keys=True)`: drop `None`
/// values, `__` keys and the request/file bookkeeping keys, recursively.
fn sanitize_loaded(value: Value) -> Value {
    const PRIVATE: &[&str] = &[
        "requested_downloads",
        "requested_formats",
        "requested_subtitles",
        "requested_entries",
        "entries",
        "filepath",
        "_filename",
        "filename",
        "infojson_filename",
        "original_url",
        "playlist_autonumber",
    ];
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .filter(|(key, value)| {
                    !value.is_null() && !key.starts_with("__") && !PRIVATE.contains(&key.as_str())
                })
                .map(|(key, value)| (key, sanitize_loaded(value)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(sanitize_loaded).collect()),
        other => other,
    }
}

/// Fill fields this port requires from an upstream-written dictionary:
/// `direct` stays absent unless saved, a format's stream kind follows `vcodec`/
/// `acodec` `"none"`, and a missing top-level `url`/`ext` (a merged
/// selection) comes from the last format, to be replaced by format selection.
fn normalize_loaded(map: &mut serde_json::Map<String, Value>) -> Result<()> {
    // The top level holds the selected format's fields.
    loaded_fragments(map)?;
    for key in ["subtitles", "automatic_captions"] {
        if let Some(Value::Object(languages)) = map.get_mut(key) {
            for tracks in languages.values_mut() {
                if let Value::Array(tracks) = tracks {
                    for track in tracks.iter_mut().filter_map(Value::as_object_mut) {
                        loaded_fragments(track)?;
                    }
                }
            }
        }
    }
    if let Some(Value::Array(formats)) = map.get_mut("formats") {
        for format in formats.iter_mut().filter_map(Value::as_object_mut) {
            let protocol = format
                .get("protocol")
                .and_then(Value::as_str)
                .unwrap_or("https");
            // HLS formats read their fragments from the playlist later.
            if !matches!(protocol, "http" | "https" | "m3u8_native")
                && !format.contains_key("fragments")
            {
                return Err(Error::InfoJson(format!(
                    "loaded format protocol {protocol:?} is unsupported"
                )));
            }
            loaded_fragments(format)?;
            if !format.contains_key("stream_kind") {
                let none = |key: &str| format.get(key).and_then(Value::as_str) == Some("none");
                let kind = match (none("vcodec"), none("acodec")) {
                    (false, true) => Some("video_only"),
                    (true, false) => Some("audio_only"),
                    _ => None,
                };
                if let Some(kind) = kind {
                    format.insert("stream_kind".into(), kind.into());
                }
            }
        }
    }
    for key in ["url", "ext"] {
        if !map.contains_key(key)
            && let Some(value) = map
                .get("formats")
                .and_then(Value::as_array)
                .and_then(|formats| formats.last())
                .and_then(|format| format.get(key))
                .cloned()
        {
            map.insert(key.into(), value);
        }
    }
    Ok(())
}

/// Upstream-shaped fragments: `path` resolves against `fragment_base_url`
/// and a `start`/`end` byte range becomes this port's offset and length.
fn loaded_fragments(format: &mut Map<String, Value>) -> Result<()> {
    let base = format
        .get("fragment_base_url")
        .and_then(Value::as_str)
        .map(url::Url::parse)
        .transpose()
        .map_err(|error| Error::InfoJson(format!("invalid fragment_base_url: {error}")))?;
    let Some(Value::Array(fragments)) = format.get_mut("fragments") else {
        return Ok(());
    };
    for fragment in fragments.iter_mut().filter_map(Value::as_object_mut) {
        if !fragment.contains_key("url")
            && let Some(path) = fragment.get("path").and_then(Value::as_str)
        {
            let base = base.as_ref().ok_or_else(|| {
                Error::InfoJson("a fragment path needs the format's fragment_base_url".into())
            })?;
            let url = base
                .join(path)
                .map_err(|error| Error::InfoJson(format!("invalid fragment path: {error}")))?;
            fragment.insert("url".into(), url.as_str().into());
        }
        if let Some(Value::Object(range)) = fragment.get_mut("byte_range")
            && let (Some(start), Some(end)) = (
                range.get("start").and_then(Value::as_u64),
                range.get("end").and_then(Value::as_u64),
            )
        {
            let length = end
                .checked_sub(start)
                .filter(|length| *length > 0)
                .ok_or_else(|| Error::InfoJson("invalid fragment byte_range".into()))?;
            *range = Map::from_iter([
                ("offset".to_owned(), start.into()),
                ("length".to_owned(), length.into()),
            ]);
        }
    }
    Ok(())
}

/// Upstream `download_with_info_file`: one dictionary or a list of them,
/// sanitized like upstream's default `clean_infojson`. Videos keep their
/// formats for reselection; playlists lose their entries.
pub fn load_info_json(path: &std::path::Path) -> Result<Vec<crate::ExtractResult>> {
    let bytes = std::fs::read(path).map_err(|error| Error::InfoJson(error.to_string()))?;
    let text = String::from_utf8(bytes)
        .map_err(|_| Error::InfoJson("info JSON is not valid UTF-8".into()))?;
    let value: Value =
        serde_json::from_str(&text).map_err(|error| Error::InfoJson(error.to_string()))?;
    let items = match value {
        Value::Array(items) => items,
        other => vec![other],
    };
    items
        .into_iter()
        .map(|item| {
            let Value::Object(mut map) = sanitize_loaded(item) else {
                return Err(Error::InfoJson("info is not a JSON object".into()));
            };
            let kind = map
                .get("_type")
                .and_then(Value::as_str)
                .unwrap_or("video")
                .to_owned();
            match kind.as_str() {
                "video" => {
                    // Upstream processes the loaded dict in place: its keys
                    // keep their order, new ones follow.
                    let original = map.clone();
                    normalize_loaded(&mut map)?;
                    let mut info: crate::VideoInfo = serde_json::from_value(Value::Object(map))
                        .map_err(|error| Error::InfoJson(error.to_string()))?;
                    info.extra.insert(
                        crate::info_order::INFO_SHAPE.into(),
                        crate::info_order::loaded_shape(&original),
                    );
                    let formats = original.get("formats").and_then(Value::as_array);
                    for (format, loaded) in
                        info.formats.iter_mut().zip(formats.into_iter().flatten())
                    {
                        if let Value::Object(loaded) = loaded {
                            format.extra.insert(
                                crate::info_order::FORMAT_SHAPE.into(),
                                crate::info_order::loaded_shape(loaded),
                            );
                        }
                    }
                    info.metadata.fill_common_fields();
                    info.metadata.sanitize_thumbnails();
                    Ok(crate::ExtractResult::Video(info))
                }
                "playlist" => {
                    map.insert("entries".into(), Value::Array(Vec::new()));
                    serde_json::from_value(Value::Object(map))
                        .map(crate::ExtractResult::Playlist)
                        .map_err(|error| Error::InfoJson(error.to_string()))
                }
                other => Err(Error::InfoJson(format!(
                    "loading {other:?} info is unsupported"
                ))),
            }
        })
        .collect()
}

#[cfg(test)]
mod load_tests {
    use super::*;
    use serde_json::json;

    fn load(value: Value) -> Result<Vec<crate::ExtractResult>> {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), serde_json::to_vec(&value).unwrap()).unwrap();
        load_info_json(file.path())
    }

    #[test]
    fn upstream_dictionaries_are_sanitized_and_normalized() {
        let loaded = load(json!({
            "id": "v", "title": "T", "ext": "mp4", "webpage_url": "https://e.invalid/w",
            "extractor": "x", "extractor_key": "X", "__private": 1, "filename": "f.mp4",
            "requested_formats": [{"format_id": "nope"}], "description": null,
            "formats": [
                {"format_id": "a", "url": "https://e.invalid/a", "ext": "m4a", "vcodec": "none", "acodec": "mp4a"},
                {"format_id": "v", "url": "https://e.invalid/v", "ext": "mp4", "vcodec": "avc1", "acodec": "none", "protocol": "https"},
            ],
        }))
        .unwrap();
        let crate::ExtractResult::Video(info) = &loaded[0] else {
            panic!("video expected");
        };
        assert!(!info.is_direct());
        assert_eq!(info.url.as_str(), "https://e.invalid/v");
        assert!(info.requested_formats.is_empty());
        assert_eq!(info.formats[0].stream_kind, crate::StreamKind::AudioOnly);
        assert_eq!(info.formats[1].stream_kind, crate::StreamKind::VideoOnly);
        let error = load(json!({
            "id": "v", "title": "T", "ext": "mp4", "webpage_url": "https://e.invalid/w",
            "extractor": "x", "extractor_key": "X",
            "formats": [{"format_id": "h", "url": "https://e.invalid/h.m3u8", "ext": "mp4", "protocol": "m3u8"}],
        }))
        .unwrap_err();
        assert!(error.to_string().contains("\"m3u8\" is unsupported"));
        assert!(load(json!({"_type": "url", "url": "https://e.invalid/"})).is_err());
        assert!(load(json!([1])).is_err());
    }
}

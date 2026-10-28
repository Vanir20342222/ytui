//! Upstream `FFmpegMetadataPP` metadata and chapter options for represented
//! fields; info JSON attachment is not implemented.

use crate::{StreamKind, VideoInfo};
use serde_json::Value;

/// Python `str(value)` for JSON scalars.
fn py_str(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::Null => "None".into(),
        Value::Number(number) => match (number.as_i64(), number.as_u64(), number.as_f64()) {
            (Some(n), _, _) => n.to_string(),
            (_, Some(n), _) => n.to_string(),
            (_, _, Some(f)) if f.fract() == 0.0 && f.abs() < 1e16 => format!("{f:.1}"),
            (_, _, Some(f)) => f.to_string(),
            _ => number.to_string(),
        },
        other => other.to_string(),
    }
}

/// `FFmpegMetadataPP.run` options: `_options(ext)`, then the chapter
/// options (`-map_metadata 1` for the FFMETADATA second input), then
/// `_get_metadata_opts(info)` when `add_metadata`. `None` when there is
/// nothing to add (upstream's `There isn't any metadata to add`).
pub(crate) fn plan_args(
    info: &VideoInfo,
    ext: &str,
    add_metadata: bool,
    chapters: bool,
    infojson: Option<&std::path::Path>,
) -> Option<Vec<String>> {
    if !add_metadata && !chapters && infojson.is_none() {
        return None;
    }
    let all = args(info, ext);
    let copy = if ext == "m4a" { 7 } else { 6 };
    let mut output = all[..copy].to_vec();
    if chapters {
        output.extend(["-map_metadata", "1"].map(String::from));
    }
    if add_metadata {
        output.extend_from_slice(&all[copy..]);
    }
    // `_get_infojson_opts` for a file without attachments: the new
    // attachment is the only `t` stream.
    if let Some(path) = infojson {
        output.push("-attach".into());
        output.push(path.to_string_lossy().into_owned());
        output.extend(
            [
                "-metadata:s:t",
                "mimetype=application/json",
                "-metadata:s:t",
                "filename=info.json",
            ]
            .map(String::from),
        );
    }
    Some(output)
}

/// `_get_chapter_opts`: the FFMETADATA file with millisecond chapters.
pub(crate) fn ffmetadata(chapters: &[crate::info::Chapter]) -> String {
    static ESCAPE: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"([\\=;#\n])").unwrap());
    let millis = |value: Option<crate::PyNumber>| {
        (value.map_or(0.0, |v| v.as_f64()) * 1000.0).trunc() as i64
    };
    let mut content = String::from(";FFMETADATA1\n");
    for chapter in chapters {
        content.push_str("[CHAPTER]\nTIMEBASE=1/1000\n");
        content.push_str(&format!("START={}\n", millis(chapter.start_time)));
        content.push_str(&format!("END={}\n", millis(chapter.end_time)));
        if let Some(title) = chapter.title.as_deref().filter(|title| !title.is_empty()) {
            content.push_str(&format!("title={}\n", ESCAPE.replace_all(title, "\\$1")));
        }
    }
    content
}

/// `--compat-options embed-metadata`: `comment` is the description and
/// `synopsis` is not written.
pub static COMPAT_EMBED_METADATA: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// `FFmpegMetadataPP._options(ext)` followed by `_get_metadata_opts(info)`.
pub(crate) fn args(info: &VideoInfo, ext: &str) -> Vec<String> {
    let value = serde_json::to_value(info).unwrap_or(Value::Null);
    let dict = value.as_object().cloned().unwrap_or_default();
    let mut args: Vec<String> = ["-map", "0", "-dn", "-ignore_unknown"]
        .into_iter()
        .map(Into::into)
        .collect();
    if ext == "m4a" {
        args.extend(["-vn", "-acodec", "copy"].map(String::from));
    } else {
        args.extend(["-c", "copy"].map(String::from));
    }
    let mut common: Vec<(String, String)> = Vec::new();
    let mut add = |names: &[&str], keys: &[&str]| {
        let found = std::iter::once("meta_")
            .chain(keys.iter().copied())
            .find_map(|key| dict.get(key).filter(|value| !value.is_null()));
        // Like upstream, the emptiness check precedes joining lists.
        let Some(found) = found.filter(|value| value.as_str() != Some("")) else {
            return;
        };
        let joined = match found {
            Value::Array(items) => items.iter().map(py_str).collect::<Vec<_>>().join(", "),
            other => py_str(other),
        }
        .replace('\0', "");
        for name in names {
            match common.iter_mut().find(|(existing, _)| existing == name) {
                Some(entry) => entry.1 = joined.clone(),
                None => common.push(((*name).into(), joined.clone())),
            }
        }
    };
    add(&["title"], &["track", "title"]);
    add(&["date"], &["upload_date"]);
    add(&["description", "synopsis"], &["description"]);
    add(&["purl", "comment"], &["webpage_url"]);
    add(&["track"], &["track_number"]);
    add(
        &["artist"],
        &[
            "artist",
            "artists",
            "creator",
            "creators",
            "uploader",
            "uploader_id",
        ],
    );
    add(&["composer"], &["composer", "composers"]);
    add(&["genre"], &["genre", "genres", "categories", "tags"]);
    add(&["album"], &["album", "series"]);
    add(&["album_artist"], &["album_artist", "album_artists"]);
    add(&["disc"], &["disc_number"]);
    add(&["show"], &["series"]);
    add(&["season_number"], &["season_number"]);
    add(&["episode_id"], &["episode", "episode_id"]);
    add(&["episode_sort"], &["episode_number"]);
    if COMPAT_EMBED_METADATA.load(std::sync::atomic::Ordering::Relaxed) {
        add(&["comment"], &["description"]);
        common.retain(|(name, _)| name != "synopsis");
    }
    args.extend(["-write_id3v1", "1"].map(String::from));
    for (name, value) in common {
        args.push("-metadata".into());
        args.push(format!("{name}={value}"));
    }
    // Per-stream languages for the selected streams (two for muxed formats).
    let selected: Vec<(StreamKind, Option<&str>)> = if info.requested_formats.is_empty() {
        let language = info
            .formats
            .iter()
            .find(|format| Some(&format.format_id) == info.format_id.as_ref())
            .and_then(|format| format.language.as_deref());
        vec![(info.stream_kind, language)]
    } else {
        info.requested_formats
            .iter()
            .map(|format| (format.stream_kind, format.language.as_deref()))
            .collect()
    };
    let mut stream = 0;
    for (kind, language) in selected {
        let count = if kind == StreamKind::Standalone { 2 } else { 1 };
        let language = language.unwrap_or("");
        let language = if language.chars().count() == 3 {
            language.to_lowercase()
        } else {
            let lower = language.to_lowercase();
            crate::embed_subtitles::short_to_long(&lower)
                .map_or_else(|| language.to_owned(), str::to_owned)
        };
        for index in stream..stream + count {
            if !language.is_empty() {
                args.push(format!("-metadata:s:{index}"));
                args.push(format!("language={language}"));
            }
        }
        stream += count;
    }
    args
}

/// The output muxer for the processed container.
pub(crate) fn muxer(ext: &str) -> &str {
    match ext {
        "mkv" | "mka" => "matroska",
        "m4a" => "ipod",
        "aac" => "adts",
        "opus" | "ogg" | "oga" => "ogg",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chapter_options_precede_metadata_options() {
        let info: VideoInfo = serde_json::from_value(serde_json::json!({"id":"a","title":"T",
            "url":"https://example.test/a.mp4","webpage_url":"https://example.test/a",
            "ext":"mp4","extractor":"generic","extractor_key":"Generic"}))
        .unwrap();
        assert_eq!(plan_args(&info, "mp4", false, false, None), None);
        assert_eq!(
            plan_args(
                &info,
                "mkv",
                false,
                false,
                Some(std::path::Path::new("a.info.json"))
            )
            .unwrap(),
            [
                "-map",
                "0",
                "-dn",
                "-ignore_unknown",
                "-c",
                "copy",
                "-attach",
                "a.info.json",
                "-metadata:s:t",
                "mimetype=application/json",
                "-metadata:s:t",
                "filename=info.json"
            ]
        );
        assert_eq!(
            plan_args(&info, "mp4", false, true, None).unwrap(),
            [
                "-map",
                "0",
                "-dn",
                "-ignore_unknown",
                "-c",
                "copy",
                "-map_metadata",
                "1"
            ]
        );
        let both = plan_args(&info, "m4a", true, true, None).unwrap();
        assert_eq!(
            &both[..9],
            [
                "-map",
                "0",
                "-dn",
                "-ignore_unknown",
                "-vn",
                "-acodec",
                "copy",
                "-map_metadata",
                "1"
            ]
        );
        assert_eq!(&both[9..11], ["-write_id3v1", "1"]);
    }

    #[test]
    fn chapter_file_follows_get_chapter_opts() {
        use crate::{PyNumber, info::Chapter};
        let chapters = [
            Chapter {
                start_time: Some(PyNumber::Float(0.0)),
                title: Some("Intro; a=b #1\\x".into()),
                end_time: Some(PyNumber::Float(45.5)),
                extra: Default::default(),
            },
            Chapter {
                start_time: Some(PyNumber::Float(45.5)),
                title: Some("Line\nbreak".into()),
                end_time: Some(PyNumber::Int(120)),
                extra: Default::default(),
            },
        ];
        // Rendered by pinned upstream `_get_chapter_opts`.
        assert_eq!(
            ffmetadata(&chapters),
            ";FFMETADATA1\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=0\nEND=45500\ntitle=Intro\\; a\\=b \\#1\\\\x\n\
             [CHAPTER]\nTIMEBASE=1/1000\nSTART=45500\nEND=120000\ntitle=Line\\\nbreak\n"
        );
    }

    #[test]
    fn metadata_options_follow_upstream_key_precedence() {
        let info: VideoInfo = serde_json::from_value(serde_json::json!({
            "id": "v", "title": "Title", "ext": "mp4", "url": "https://e.invalid/v.mp4",
            "webpage_url": "https://e.invalid/watch", "extractor": "x", "extractor_key": "X",
            "direct": false, "upload_date": "20080529", "description": "Desc\u{0}ription",
            "uploader_id": "@Handle", "tags": [], "format_id": "18",
            "formats": [{"format_id": "18", "ext": "mp4", "url": "https://e.invalid/v.mp4", "language": "en"}],
        }))
        .unwrap();
        assert_eq!(
            args(&info, "mp4"),
            [
                "-map",
                "0",
                "-dn",
                "-ignore_unknown",
                "-c",
                "copy",
                "-write_id3v1",
                "1",
                "-metadata",
                "title=Title",
                "-metadata",
                "date=20080529",
                "-metadata",
                "description=Description",
                "-metadata",
                "synopsis=Description",
                "-metadata",
                "purl=https://e.invalid/watch",
                "-metadata",
                "comment=https://e.invalid/watch",
                "-metadata",
                "artist=@Handle",
                "-metadata",
                "genre=",
                "-metadata:s:0",
                "language=eng",
                "-metadata:s:1",
                "language=eng",
            ]
        );
        assert_eq!(&args(&info, "m4a")[4..7], ["-vn", "-acodec", "copy"]);
    }
}

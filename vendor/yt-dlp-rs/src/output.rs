pub mod encoding;
pub(crate) mod exec;
mod filename;
mod formats;
mod links;
mod print;
pub mod progress;
mod sanitize;
mod subtitles;
mod template;
mod thumbnails;
pub(crate) mod traverse;
pub use exec::{ExecCommand, ExecCommands, Stage, shell_quote};
pub use filename::{OUTPUT_TYPES, OutputTemplates, parse_paths, replace_extension};
pub use formats::{render_formats, render_formats_listing, render_formats_table};
pub use links::{iri_to_uri, link_file};
pub use print::{ForcedFields, PrintTemplate, PrintToFile, StagePaths};

/// The case-insensitive `WHEN:` stage name of a `--print` value (`video`
/// when absent), as upstream's `when_prefix` splits it.
pub fn print_stage_name(spec: &str) -> String {
    exec::Stage::split(spec, "video").0
}
pub use sanitize::{FilenameMode, force_sanitize_path, sanitize_filename};
pub use subtitles::{render_automatic_captions, render_subtitles};
pub use thumbnails::render_thumbnails;

use crate::{PlaylistCounts, PlaylistInfo, Result, VideoInfo};
use std::path::PathBuf;

pub const DEFAULT_TEMPLATE: &str = "%(title)s [%(id)s].%(ext)s";

/// Upstream `_parse_outtmpl`: restricted names remove spaces in the default template.
pub fn default_template(restricted: bool) -> String {
    if restricted {
        DEFAULT_TEMPLATE.replace(" - ", " ").replace(' ', "-")
    } else {
        DEFAULT_TEMPLATE.into()
    }
}

pub(crate) fn safe_component(value: &str) -> String {
    let value: String = value
        .chars()
        .map(|c| {
            if c.is_control() || "/\\<>:\"|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let value = value.trim().trim_matches('.');
    if value.is_empty() {
        "video".into()
    } else {
        value.into()
    }
}

/// Options for the supported filename template conversions.
#[derive(Debug, Clone)]
pub struct TemplateOptions {
    /// Used for unavailable metadata or failed numeric conversion.
    pub na_placeholder: String,
    /// Metadata sanitization (`--restrict-filenames`/`--no-windows-filenames`).
    pub filename_mode: FilenameMode,
    /// `--windows-filenames`: force upstream Windows path sanitization.
    pub windows_filenames: bool,
    /// `--trim-filenames`: Python-slice length excluding extensions; 0 disables.
    pub trim_file_name: i64,
    /// `--paths`: `home` and output-type directories joined before names.
    pub paths: std::collections::BTreeMap<String, String>,
    /// Upstream `autonumber` (`autonumber_start - 1 + downloads`) for the
    /// video being processed, set by the caller.
    pub autonumber: std::cell::Cell<Option<u64>>,
    /// `--autonumber-size`: `%(autonumber)s` zero padding (default 5).
    pub autonumber_size: usize,
    /// Upstream `video_autonumber` (videos processed so far).
    pub video_autonumber: std::cell::Cell<Option<u64>>,
    /// `--compat-options filename-sanitization`: youtube-dl's `is_id` rules.
    pub legacy_sanitization: bool,
    /// `--compat-options allow-unsafe-exec-expansion`: skip `ExecPP`'s
    /// template safety validation.
    pub allow_unsafe_exec: bool,
    /// `--list-formats-old` (or `--compat-options list-formats`): the
    /// legacy layout for `--print`'s `formats_table`.
    pub list_formats_old: bool,
}

impl TemplateOptions {
    /// Resolve upstream's restrict/windows filename options for this platform.
    pub fn filename_mode(restrict: bool, windows: Option<bool>) -> FilenameMode {
        if restrict {
            FilenameMode::Restricted
        } else if !cfg!(windows) && windows == Some(false) {
            FilenameMode::Minimal
        } else {
            FilenameMode::Default
        }
    }
}

impl Default for TemplateOptions {
    fn default() -> Self {
        Self {
            na_placeholder: "NA".into(),
            filename_mode: FilenameMode::Default,
            windows_filenames: false,
            trim_file_name: 0,
            paths: Default::default(),
            autonumber: Default::default(),
            autonumber_size: 5,
            video_autonumber: Default::default(),
            legacy_sanitization: false,
            allow_unsafe_exec: false,
            list_formats_old: false,
        }
    }
}

/// Python `json.dumps(value)` (default separators, `ensure_ascii`), as
/// upstream prints `-j`/`-J`.
pub fn python_json_dumps(value: &serde_json::Value) -> String {
    traverse::py_json(value, true)
}

/// Render a supported typed template with the default missing-value placeholder.
/// Only literal template text can introduce directories.
pub fn render_template(template: &str, info: &VideoInfo) -> Result<PathBuf> {
    render_template_with_options(template, info, &TemplateOptions::default())
}

/// Render supported typed metadata with explicit missing-value options.
pub fn render_template_with_options(
    template: &str,
    info: &VideoInfo,
    options: &TemplateOptions,
) -> Result<PathBuf> {
    template::render(template, template::Source::Video(info), options)
}

/// Upstream `prepare_filename(dict, kind)` over an arbitrary dict (the
/// playlist `pl_video` file of `--concat-playlist`).
pub fn render_dict_filename(
    template: &str,
    dict: &serde_json::Value,
    options: &TemplateOptions,
    kind: &str,
) -> Result<PathBuf> {
    template::render_kind(
        template,
        template::Source::JsonFile(dict),
        options,
        Some(kind),
    )
}

/// Upstream `evaluate_outtmpl(template, dict)` over an arbitrary dict, as
/// `--progress-template` renders `{'info': ..., 'progress': ...}`.
pub fn evaluate_dict_template(
    template: &str,
    dict: &serde_json::Value,
    options: &TemplateOptions,
) -> Result<String> {
    template::expand(template, template::Source::Json(dict), options)
}

/// Upstream `evaluate_outtmpl(template, info)`: unsanitized text, as
/// printing renders it (without `filename`/`urls`).
pub fn evaluate_template(
    template: &str,
    info: &VideoInfo,
    options: &TemplateOptions,
) -> Result<String> {
    let unavailable = crate::Error::InvalidTemplate("filename is not available here".into());
    template::expand(
        template,
        template::Source::Print {
            info,
            filename: Err(&unavailable),
            urls: info.url.as_str(),
            filepath: None,
            prepared: None,
        },
        options,
    )
}

/// Render a playlist-level file name, as upstream's playlist info JSON does.
/// Playlist metadata supplies id/title/extractor/webpage URL plus
/// `playlist_index` 0 and the counts; other represented fields are missing.
pub fn render_playlist_template_with_options(
    template: &str,
    info: &PlaylistInfo,
    counts: PlaylistCounts,
    options: &TemplateOptions,
) -> Result<PathBuf> {
    let source = template::Source::Playlist {
        info,
        playlist_count: counts.playlist_count,
        n_entries: counts.n_entries,
        last_playlist_index: counts.last_playlist_index,
    };
    template::render(template, source, options)
}

/// Upstream `_UnsafeExtensionError.ALLOWED_EXTENSIONS`.
pub const ALLOWED_EXTENSIONS: &[&str] = &[
    "3g2",
    "3ga",
    "3gp",
    "aac",
    "ac3",
    "adts",
    "aif",
    "aiff",
    "alac",
    "ape",
    "asf",
    "ass",
    "asx",
    "au",
    "avi",
    "avif",
    "bmp",
    "description",
    "dfxp",
    "divx",
    "dts",
    "f4a",
    "f4b",
    "f4f",
    "f4m",
    "f4v",
    "flac",
    "flv",
    "fs",
    "gif",
    "heic",
    "ico",
    "image",
    "ism",
    "isma",
    "ismt",
    "ismv",
    "it",
    "jfif",
    "jng",
    "jpe",
    "jpeg",
    "jpg",
    "json",
    "json3",
    "jxl",
    "lrc",
    "m2t",
    "m2ts",
    "m2v",
    "m3u",
    "m3u8",
    "m4a",
    "m4b",
    "m4r",
    "m4s",
    "m4v",
    "meta",
    "mhtml",
    "mid",
    "mk3d",
    "mka",
    "mkv",
    "mng",
    "mod",
    "mov",
    "mp1",
    "mp2",
    "mp2v",
    "mp3",
    "mp4",
    "mp4a",
    "mp4v",
    "mpa",
    "mpd",
    "mpe",
    "mpeg",
    "mpeg1",
    "mpeg2",
    "mpeg4",
    "mpg",
    "mpga",
    "mxf",
    "oga",
    "ogg",
    "ogm",
    "ogv",
    "ogx",
    "opus",
    "orig",
    "part",
    "png",
    "qt",
    "ra",
    "rm",
    "sami",
    "sbv",
    "scc",
    "shn",
    "smil",
    "spx",
    "srt",
    "srv1",
    "srv2",
    "srv3",
    "ssa",
    "svg",
    "swf",
    "temp",
    "tif",
    "tiff",
    "ts",
    "tt",
    "ttml",
    "uncut",
    "unknown_video",
    "vid",
    "vob",
    "vorbis",
    "vp9",
    "vtt",
    "wav",
    "wbmp",
    "weba",
    "webm",
    "webp",
    "wma",
    "wmv",
    "xm",
    "xml",
    "ytdl",
];

/// Upstream `_UnsafeExtensionError.sanitize_extension` acceptance: the last
/// dotted part, case-insensitively (`bin` is treated as `unknown_video`).
pub fn allowed_extension(extension: &str) -> bool {
    if extension.contains(['/', '\\']) {
        return false;
    }
    let last = extension
        .rsplit('.')
        .next()
        .unwrap_or(extension)
        .to_lowercase();
    last == "bin" || ALLOWED_EXTENSIONS.contains(&last.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_names_cannot_create_paths() {
        assert_eq!(safe_component("../../escape\\bad\n"), "_.._escape_bad_");
        assert_eq!(safe_component(".."), "video");
    }
}

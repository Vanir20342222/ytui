//! Upstream `FFmpegThumbnailsConvertorPP` and the FFmpeg paths of
//! `EmbedThumbnailPP`, with explicit tools and bounded execution.

use crate::{Error, Result};
use regex::Regex;
use serde_json::Value;
use std::{
    ffi::OsString,
    ops::ControlFlow,
    path::{Path, PathBuf},
    sync::LazyLock,
    time::{Duration, Instant},
};

const DEADLINE: Duration = Duration::from_secs(600);

fn failure(message: impl Into<String>) -> Error {
    Error::Postprocess(message.into())
}

/// `create_mapping_re(MEDIA_EXTENSIONS.thumbnails)` used by `validate_regex`.
static MAPPING: LazyLock<Regex> = LazyLock::new(|| {
    let atom = r"(?:\s*\w+\s*>)?\s*(?:jpg|png|webp)\s*";
    Regex::new(&format!("^{atom}(?:/{atom})*$")).unwrap()
});

/// `--convert-thumbnails`: an upstream mapping like `webp>jpg/png`.
#[derive(Debug, Clone)]
pub struct ThumbnailConversion {
    mapping: String,
}

impl ThumbnailConversion {
    /// `None` for `none`; an error for values upstream's regex rejects.
    pub fn from_cli(format: &str) -> Result<Option<Self>> {
        if format == "none" {
            return Ok(None);
        }
        if !MAPPING.is_match(format) {
            return Err(failure(format!(
                "invalid thumbnail format \"{format}\" given"
            )));
        }
        Ok(Some(Self {
            mapping: format.into(),
        }))
    }

    /// Upstream `resolve_mapping`: the target, or the skip reason.
    pub fn resolve(&self, source: &str) -> std::result::Result<String, String> {
        for pair in self.mapping.to_lowercase().split('/') {
            let (condition, target) = match pair.split_once('>') {
                Some((condition, target)) => (Some(condition.trim()), target.trim()),
                None => (None, pair.trim()),
            };
            if condition.is_none_or(|condition| condition == source) {
                if target == source {
                    return Err(format!("already is in target format {source}"));
                }
                return Ok(target.into());
            }
        }
        Err(format!("could not find a mapping for {source}"))
    }
}

/// `imghdr.what(...) == 'webp'`.
fn is_webp(path: &Path) -> bool {
    let mut header = [0u8; 12];
    std::fs::File::open(path)
        .and_then(|mut file| std::io::Read::read_exact(&mut file, &mut header))
        .is_ok()
        && &header[..4] == b"RIFF"
        && &header[8..] == b"WEBP"
}

fn extension(path: &Path) -> String {
    path.extension()
        .map(|ext| ext.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// `fixup_webp`: rename a WebP file that has another extension.
pub fn fixup_webp(path: &Path) -> Result<PathBuf> {
    let ext = extension(path);
    if ext.is_empty() || ext.eq_ignore_ascii_case("webp") || !is_webp(path) {
        return Ok(path.to_owned());
    }
    let renamed = path.with_extension("webp");
    std::fs::rename(path, &renamed)?;
    Ok(renamed)
}

fn tool(
    path: &Path,
    banner: &[u8],
    started: Instant,
    callback: &mut dyn FnMut() -> ControlFlow<()>,
) -> Result<(PathBuf, PathBuf)> {
    let executable = path.canonicalize()?;
    if !executable.is_file() {
        return Err(failure("thumbnail tools must be regular files"));
    }
    let directory = executable
        .parent()
        .map(Path::to_owned)
        .ok_or_else(|| failure("thumbnail tool has no parent directory"))?;
    let output = crate::merge::execute(
        &executable,
        &["-version".into()],
        &directory,
        started,
        DEADLINE,
        callback,
    )?;
    if !output.starts_with(banner) {
        return Err(failure(
            "thumbnail tool did not report the expected version banner",
        ));
    }
    Ok((executable, directory))
}

/// An image input, with `pp`'s `--postprocessor-args` for input `number`.
fn image_input(path: &Path, pp: &str, number: usize) -> Result<Vec<OsString>> {
    let path = path.canonicalize()?;
    if !path.is_file() {
        return Err(failure("thumbnail input must be a regular file"));
    }
    let mut args: Vec<OsString> = Vec::new();
    // Upstream disables image2 patterns for all formats except GIF.
    if !extension(&path).eq_ignore_ascii_case("gif") {
        args.extend(["-f", "image2", "-pattern_type", "none"].map(OsString::from));
    }
    args.extend(["-protocol_whitelist", "file"].map(OsString::from));
    args.extend(crate::pp_args::input(pp, number));
    args.push("-i".into());
    args.push(path.into_os_string());
    Ok(args)
}

/// `convert_thumbnail`: `-update 1` (and `mjpeg2jpeg` for JPEG) to `output`.
pub fn convert(
    ffmpeg: &Path,
    input: &Path,
    output: &Path,
    target: &str,
    callback: &mut dyn FnMut() -> ControlFlow<()>,
) -> Result<()> {
    let started = Instant::now();
    let (executable, directory) = tool(ffmpeg, b"ffmpeg version ", started, callback)?;
    let mut args: Vec<OsString> = ["-nostdin", "-hide_banner", "-loglevel", "repeat+info", "-y"]
        .map(OsString::from)
        .into();
    args.extend(image_input(input, "ThumbnailsConvertor", 1)?);
    args.extend(["-update", "1"].map(OsString::from));
    if target == "jpg" {
        args.extend(["-bsf:v", "mjpeg2jpeg"].map(OsString::from));
    }
    args.extend(["-movflags", "+faststart"].map(OsString::from));
    args.extend(crate::pp_args::output("ThumbnailsConvertor"));
    args.push(std::path::absolute(output)?.into_os_string());
    crate::merge::execute(&executable, &args, &directory, started, DEADLINE, callback)?;
    if !output.is_file() || output.metadata()?.len() == 0 {
        return Err(failure("FFmpeg produced no converted thumbnail"));
    }
    Ok(())
}

fn media_demuxer(ext: &str) -> Result<&'static str> {
    Ok(match ext {
        "mp3" => "mp3",
        "mkv" | "mka" => "matroska",
        "m4a" | "mp4" | "m4v" | "mov" => "mov",
        "ogg" | "opus" | "flac" => {
            return Err(failure(
                "thumbnail embedding for ogg/opus/flac needs mutagen and is not implemented",
            ));
        }
        _ => {
            return Err(failure(
                "Supported filetypes for thumbnail embedding are: mp3, mkv/mka, ogg/opus/flac, m4a/mp4/m4v/mov",
            ));
        }
    })
}

/// `get_stream_number(path, keys, value)` through `ffprobe -show_streams`.
fn stream_number(
    ffprobe: &Path,
    media: &Path,
    demuxer: &str,
    matches: &dyn Fn(&Value) -> bool,
    started: Instant,
    callback: &mut dyn FnMut() -> ControlFlow<()>,
) -> Result<(Option<usize>, usize)> {
    let (executable, directory) = tool(ffprobe, b"ffprobe version ", started, callback)?;
    let mut args: Vec<OsString> = [
        "-hide_banner",
        "-v",
        "error",
        "-show_format",
        "-show_streams",
        "-print_format",
        "json",
        "-protocol_whitelist",
        "file",
        "-f",
        demuxer,
        "-i",
    ]
    .map(OsString::from)
    .into();
    args.push(media.canonicalize()?.into_os_string());
    let output =
        crate::merge::execute(&executable, &args, &directory, started, DEADLINE, callback)?;
    let probe: Value =
        serde_json::from_slice(&output).map_err(|_| failure("invalid ffprobe JSON"))?;
    let streams = probe["streams"]
        .as_array()
        .ok_or_else(|| failure("ffprobe reported no streams"))?;
    Ok((streams.iter().position(matches), streams.len()))
}

/// Case-insensitive `traverse_obj(stream, (outer, inner), casesense=False)`.
fn nested<'a>(stream: &'a Value, outer: &str, inner: &str) -> Option<&'a Value> {
    let lookup = |value: &'a Value, key: &str| {
        value
            .as_object()?
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(key))
            .map(|(_, value)| value)
    };
    lookup(lookup(stream, outer)?, inner)
}

/// Embed `thumbnail` into `media` (container `ext`, shown as `display`),
/// writing `output`. Non-JPEG/PNG thumbnails are converted to PNG first
/// except for Matroska.
#[allow(clippy::too_many_arguments)]
pub fn embed(
    ffmpeg: &Path,
    ffprobe: &Path,
    media: &Path,
    display: &Path,
    ext: &str,
    thumbnail: &Path,
    workspace: &Path,
    output: &Path,
    callback: &mut dyn FnMut() -> ControlFlow<()>,
) -> Result<()> {
    let demuxer = media_demuxer(ext)?;
    let mut thumbnail = thumbnail.to_owned();
    let mut thumbnail_ext = extension(&thumbnail).to_lowercase();
    if !matches!(ext, "mkv" | "mka") && !matches!(thumbnail_ext.as_str(), "jpg" | "jpeg" | "png") {
        crate::screen::screen(&format!(
            "[ThumbnailsConvertor] Converting thumbnail \"{}\" to png",
            thumbnail.display()
        ));
        let converted = workspace.join("thumbnail.png");
        convert(ffmpeg, &thumbnail, &converted, "png", callback)?;
        thumbnail = converted;
        thumbnail_ext = "png".into();
    }
    let started = Instant::now();
    let (executable, directory) = tool(ffmpeg, b"ffmpeg version ", started, callback)?;
    let media = media.canonicalize()?;
    let mut args: Vec<OsString> = [
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "repeat+info",
        "-y",
        "-f",
        demuxer,
        "-protocol_whitelist",
        "file",
    ]
    .map(OsString::from)
    .into();
    args.extend(crate::pp_args::input("EmbedThumbnail", 1));
    args.push("-i".into());
    args.push(media.clone().into_os_string());
    let copy = ["-map", "0", "-dn", "-ignore_unknown", "-c", "copy"].map(OsString::from);
    match ext {
        "mp3" => {
            args.extend(image_input(&thumbnail, "EmbedThumbnail", 2)?);
            args.extend(
                [
                    "-c",
                    "copy",
                    "-map",
                    "0:0",
                    "-map",
                    "1:0",
                    "-write_id3v1",
                    "1",
                    "-id3v2_version",
                    "3",
                    "-metadata:s:v",
                    "title=Album cover",
                    "-metadata:s:v",
                    "comment=Cover (front)",
                ]
                .map(OsString::from),
            );
        }
        "mkv" | "mka" => {
            let mimetype = format!("image/{}", thumbnail_ext.replace("jpg", "jpeg"));
            let (old, mut new) = stream_number(
                ffprobe,
                &media,
                demuxer,
                &|stream| {
                    nested(stream, "tags", "mimetype").and_then(Value::as_str)
                        == Some(mimetype.as_str())
                },
                started,
                callback,
            )?;
            args.extend(copy);
            if let Some(old) = old {
                args.extend(["-map".into(), format!("-0:{old}").into()]);
                new -= 1;
            }
            args.push("-attach".into());
            args.push(thumbnail.canonicalize()?.into_os_string());
            args.extend([
                format!("-metadata:s:{new}").into(),
                format!("mimetype={mimetype}").into(),
                format!("-metadata:s:{new}").into(),
                format!("filename=cover.{thumbnail_ext}").into(),
            ]);
        }
        _ => {
            // Upstream's FFmpeg fallback (mutagen/AtomicParsley are not used).
            crate::screen::screen(
                "[EmbedThumbnail] Neither mutagen nor AtomicParsley was found. Falling back to ffmpeg",
            );
            let (old, mut new) = stream_number(
                ffprobe,
                &media,
                demuxer,
                &|stream| {
                    nested(stream, "disposition", "attached_pic").and_then(Value::as_i64) == Some(1)
                },
                started,
                callback,
            )?;
            args.extend(image_input(&thumbnail, "EmbedThumbnail", 2)?);
            args.extend(copy);
            args.extend(["-map", "1"].map(OsString::from));
            if let Some(old) = old {
                args.extend(["-map".into(), format!("-0:{old}").into()]);
                new -= 1;
            }
            args.extend([format!("-disposition:{new}").into(), "attached_pic".into()]);
        }
    }
    crate::screen::screen(&format!(
        "[EmbedThumbnail] ffmpeg: Adding thumbnail to \"{}\"",
        display.display()
    ));
    let muxer = match ext {
        "mkv" | "mka" => "matroska",
        "m4a" => "ipod",
        "m4v" => "mp4",
        other => other,
    };
    args.extend(["-movflags", "+faststart", "-f", muxer].map(OsString::from));
    args.extend(crate::pp_args::output("EmbedThumbnail"));
    args.push(std::path::absolute(output)?.into_os_string());
    crate::merge::execute(&executable, &args, &directory, started, DEADLINE, callback)?;
    if !output.is_file() || output.metadata()?.len() == 0 {
        return Err(failure(
            "FFmpeg produced no output with an embedded thumbnail",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mappings_follow_upstream_validation_and_resolution() {
        assert!(ThumbnailConversion::from_cli("none").unwrap().is_none());
        assert!(ThumbnailConversion::from_cli("gif").is_err());
        assert!(ThumbnailConversion::from_cli("JPG").is_err());
        let conversion = ThumbnailConversion::from_cli("webp>jpg / png")
            .unwrap()
            .unwrap();
        assert_eq!(conversion.resolve("webp"), Ok("jpg".into()));
        assert_eq!(conversion.resolve("jpg"), Ok("png".into()));
        assert_eq!(
            conversion.resolve("png"),
            Err("already is in target format png".into())
        );
        let only = ThumbnailConversion::from_cli("webp>png").unwrap().unwrap();
        assert_eq!(
            only.resolve("jpg"),
            Err("could not find a mapping for jpg".into())
        );
    }
}

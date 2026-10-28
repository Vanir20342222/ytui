//! Upstream `FFmpegFixupPostProcessor` remuxes: `FixupM3u8` (MPEG-TS in an
//! MP4-labeled HLS download) and `FixupM4a` (DASH m4a container).
use crate::{Error, Result};
use std::{
    ffi::OsString,
    ops::ControlFlow,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const DEADLINE: Duration = Duration::from_secs(600);

fn failure(message: impl Into<String>) -> Error {
    Error::Postprocess(message.into())
}

/// `prepend_extension(path, 'temp')`: `NAME.temp.EXT`.
fn temp_path(path: &Path) -> PathBuf {
    match (path.file_stem(), path.extension()) {
        (Some(stem), Some(ext)) => path.with_file_name(format!(
            "{}.temp.{}",
            stem.to_string_lossy(),
            ext.to_string_lossy()
        )),
        _ => path.with_file_name(format!(
            "{}.temp",
            path.file_name().unwrap_or_default().to_string_lossy()
        )),
    }
}

fn demuxer(container: &str) -> Result<&'static str> {
    Ok(match container {
        "ts" => "mpegts",
        "mp4" | "m4a" => "mov",
        "aac" => "aac",
        "webm" => "matroska",
        _ => return Err(failure("unsupported fixup input container")),
    })
}

/// Stream-copy `path` into an MP4 container in place (`_fixup`), with
/// `-bsf:a aac_adtstoasc` when requested; `pp` names the postprocessor for
/// `--postprocessor-args` (`FixupM3u8`, `FixupM4a`).
pub(crate) fn remux_to_mp4(
    pp: &str,
    ffmpeg: &Path,
    path: &Path,
    container: &str,
    aac_bsf: bool,
    callback: &mut dyn FnMut() -> ControlFlow<()>,
) -> Result<()> {
    stream_copy(pp, ffmpeg, path, container, "mp4", aac_bsf, callback)
}

/// `FFmpegCopyStreamPP` (`FixupDuplicateMoov`): stream-copy `path` in place
/// into its own container.
pub(crate) fn copy_stream(
    pp: &str,
    ffmpeg: &Path,
    path: &Path,
    container: &str,
    ext: &str,
    callback: &mut dyn FnMut() -> ControlFlow<()>,
) -> Result<()> {
    stream_copy(
        pp,
        ffmpeg,
        path,
        container,
        crate::embed_metadata::muxer(ext),
        false,
        callback,
    )
}

/// `FFmpegFixupStretchedPP`: stream-copy `path` in place with `-aspect`.
pub(crate) fn set_aspect(
    pp: &str,
    ffmpeg: &Path,
    path: &Path,
    container: &str,
    ext: &str,
    ratio: f64,
    callback: &mut dyn FnMut() -> ControlFlow<()>,
) -> Result<()> {
    stream_copy_with(
        pp,
        ffmpeg,
        path,
        container,
        None,
        &["-aspect".into(), format!("{ratio:.6}").into()],
        Some(crate::embed_metadata::muxer(ext)),
        callback,
    )
}

fn stream_copy(
    pp: &str,
    ffmpeg: &Path,
    path: &Path,
    container: &str,
    muxer: &str,
    aac_bsf: bool,
    callback: &mut dyn FnMut() -> ControlFlow<()>,
) -> Result<()> {
    let extra: Vec<OsString> = if aac_bsf {
        vec!["-bsf:a".into(), "aac_adtstoasc".into()]
    } else {
        Vec::new()
    };
    stream_copy_with(
        pp,
        ffmpeg,
        path,
        container,
        Some(muxer),
        &extra,
        None,
        callback,
    )
}

/// `stream_copy_opts()`, then `-f MUXER` and `extra` (or `extra` then the
/// output's own muxer `late_muxer`, as `_fixup` options are ordered).
#[allow(clippy::too_many_arguments)]
fn stream_copy_with(
    pp: &str,
    ffmpeg: &Path,
    path: &Path,
    container: &str,
    muxer: Option<&str>,
    extra: &[OsString],
    late_muxer: Option<&str>,
    callback: &mut dyn FnMut() -> ControlFlow<()>,
) -> Result<()> {
    let started = Instant::now();
    let demuxer = demuxer(container)?;
    let executable = ffmpeg.canonicalize()?;
    let input = path.canonicalize()?;
    if !executable.is_file() || !input.is_file() {
        return Err(failure("ffmpeg and the fixup input must be regular files"));
    }
    let directory = input
        .parent()
        .ok_or_else(|| failure("fixup input has no parent directory"))?
        .to_path_buf();
    let version = crate::merge::execute(
        &executable,
        &["-version".into()],
        &directory,
        started,
        DEADLINE,
        callback,
    )?;
    if !version.starts_with(b"ffmpeg version ") {
        return Err(failure(
            "executable did not report an FFmpeg version banner",
        ));
    }
    let output = temp_path(&input);
    let mut args: Vec<OsString> = [
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "repeat+info",
        "-y",
        "-protocol_whitelist",
        "file",
        "-f",
        demuxer,
    ]
    .into_iter()
    .map(Into::into)
    .collect();
    args.extend(crate::pp_args::input(pp, 1));
    args.push("-i".into());
    args.push(input.clone().into_os_string());
    // `stream_copy_opts()` then `-f mp4` (and the AAC bitstream filter).
    args.extend(
        ["-map", "0", "-dn", "-ignore_unknown", "-c", "copy"]
            .into_iter()
            .map(OsString::from),
    );
    if let Some(muxer) = muxer {
        args.extend(["-f".into(), muxer.into()]);
    }
    args.extend(extra.iter().cloned());
    // The temporary name's extension does not name a muxer.
    if let Some(muxer) = late_muxer {
        args.extend(["-f".into(), muxer.into()]);
    }
    args.extend(crate::pp_args::output(pp));
    args.push(output.clone().into_os_string());
    let oldest = crate::filetime::oldest(std::iter::once(input.as_path()));
    let result = crate::merge::execute(&executable, &args, &directory, started, DEADLINE, callback);
    if let Err(error) = result {
        let _ = std::fs::remove_file(&output);
        return Err(error);
    }
    if std::fs::metadata(&output).map_or(true, |metadata| metadata.len() == 0) {
        let _ = std::fs::remove_file(&output);
        return Err(failure("FFmpeg fixup produced no output"));
    }
    crate::filetime::inherit(&output, oldest);
    std::fs::rename(&output, &input)?;
    Ok(())
}

/// Upstream `FFmpegSplitChaptersPP` for one chapter: input `-ss START -t
/// DURATION`, `stream_copy_opts()` into `output` (via a temporary name).
pub(crate) fn cut(
    ffmpeg: &Path,
    input: &Path,
    output: &Path,
    start: &str,
    duration: &str,
    muxer: &str,
    callback: &mut dyn FnMut() -> ControlFlow<()>,
) -> Result<()> {
    let started = Instant::now();
    let container =
        crate::container::sniff(input).ok_or_else(|| failure("unknown chapter input container"))?;
    let demuxer = demuxer(container)?;
    let executable = ffmpeg.canonicalize()?;
    let input = input.canonicalize()?;
    let directory = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    // `_ensure_dir_exists`.
    std::fs::create_dir_all(directory)?;
    let directory = directory.canonicalize()?;
    let output = directory.join(
        output
            .file_name()
            .ok_or_else(|| failure("invalid chapter output path"))?,
    );
    if output == input {
        return Err(failure("chapter output must differ from its input"));
    }
    let temporary = temp_path(&output);
    let mut args: Vec<OsString> = [
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "repeat+info",
        "-y",
        "-ss",
        start,
        "-t",
        duration,
        "-f",
        demuxer,
        "-protocol_whitelist",
        "file",
    ]
    .into_iter()
    .map(Into::into)
    .collect();
    args.extend(crate::pp_args::input("SplitChapters", 1));
    args.push("-i".into());
    args.push(input.into_os_string());
    args.extend(
        [
            "-map",
            "0",
            "-dn",
            "-ignore_unknown",
            "-c",
            "copy",
            "-f",
            muxer,
        ]
        .into_iter()
        .map(OsString::from),
    );
    args.extend(crate::pp_args::output("SplitChapters"));
    args.push(temporary.clone().into_os_string());
    let result = crate::merge::execute(&executable, &args, &directory, started, DEADLINE, callback);
    if let Err(error) = result {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    if std::fs::metadata(&temporary).map_or(true, |metadata| metadata.len() == 0) {
        let _ = std::fs::remove_file(&temporary);
        return Err(failure("FFmpeg produced no chapter output"));
    }
    // Upstream's FFmpeg overwrites an existing chapter file (`-y`).
    std::fs::rename(&temporary, &output)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temporary_names_follow_prepend_extension() {
        assert_eq!(
            temp_path(Path::new("dir/o.mp4")),
            PathBuf::from("dir/o.temp.mp4")
        );
        assert_eq!(temp_path(Path::new("o")), PathBuf::from("o.temp"));
    }
}

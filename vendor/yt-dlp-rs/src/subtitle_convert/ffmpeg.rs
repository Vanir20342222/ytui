//! Bounded conversion of already-downloaded subtitle files using local FFmpeg.
use crate::{Error, Result};
use std::{
    ffi::OsString,
    fs,
    io::Read,
    ops::ControlFlow,
    path::Path,
    time::{Duration, Instant},
};
const DEADLINE: Duration = Duration::from_secs(60);
const MAX_OUTPUT: u64 = 16 * 1024 * 1024;
fn error(message: &str) -> Error {
    Error::Postprocess(message.into())
}
fn demuxer(extension: &str) -> Result<&'static str> {
    match extension {
        "vtt" => Ok("webvtt"),
        "srt" => Ok("srt"),
        "ass" | "ssa" => Ok("ass"),
        "lrc" => Ok("lrc"),
        _ => Err(error("unsupported subtitle input format for FFmpeg")),
    }
}
fn muxer(extension: &str) -> Result<&'static str> {
    match extension {
        "vtt" => Ok("webvtt"),
        "srt" => Ok("srt"),
        "ass" => Ok("ass"),
        "lrc" => Ok("lrc"),
        _ => Err(error("unsupported subtitle output format for FFmpeg")),
    }
}
pub(crate) fn convert(
    ffmpeg: &Path,
    input: &Path,
    input_ext: &str,
    output: &Path,
    output_ext: &str,
    callback: &mut dyn FnMut() -> ControlFlow<()>,
) -> Result<()> {
    convert_with_deadline(
        ffmpeg, input, input_ext, output, output_ext, callback, DEADLINE,
    )
}
fn convert_with_deadline(
    ffmpeg: &Path,
    input: &Path,
    input_ext: &str,
    output: &Path,
    output_ext: &str,
    callback: &mut dyn FnMut() -> ControlFlow<()>,
    deadline: Duration,
) -> Result<()> {
    let demuxer = demuxer(input_ext)?;
    let muxer = muxer(output_ext)?;
    let started = Instant::now();
    let executable = ffmpeg.canonicalize()?;
    let input = input.canonicalize()?;
    if !executable.is_file() || !input.is_file() {
        return Err(error("subtitle tool and input must be regular files"));
    }
    let directory = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .canonicalize()?;
    let output = directory.join(
        output
            .file_name()
            .ok_or_else(|| error("invalid subtitle output path"))?,
    );
    if input == output {
        return Err(error("subtitle output must differ from input"));
    }
    match fs::symlink_metadata(&output) {
        Ok(_) => return Err(error("subtitle output already exists")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let version = crate::merge::execute(
        &executable,
        &["-version".into()],
        &directory,
        started,
        deadline,
        callback,
    )?;
    if !version.starts_with(b"ffmpeg version ") {
        return Err(error(
            "subtitle tool did not report the expected FFmpeg banner",
        ));
    }
    let args: Vec<OsString> = [
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
    .into_iter()
    .map(Into::into)
    .chain(crate::pp_args::input("SubtitlesConvertor", 1))
    .chain(["-i".into(), input.into_os_string()])
    .chain(
        ["-map", "0:s:0", "-movflags", "+faststart", "-f", muxer]
            .into_iter()
            .map(Into::into),
    )
    .chain(crate::pp_args::output("SubtitlesConvertor"))
    .chain(std::iter::once(output.clone().into_os_string()))
    .collect();
    let mut exceeded_budget = false;
    let result = crate::merge::execute(
        &executable,
        &args,
        &directory,
        started,
        deadline,
        &mut || {
            if fs::symlink_metadata(&output).is_ok_and(|metadata| metadata.len() > MAX_OUTPUT) {
                exceeded_budget = true;
                ControlFlow::Break(())
            } else {
                callback()
            }
        },
    );
    if exceeded_budget {
        return Err(error("converted subtitle exceeds 16 MiB budget"));
    }
    result?;
    let metadata = fs::symlink_metadata(&output)
        .map_err(|_| error("FFmpeg produced no nonempty subtitle output"))?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err(error("FFmpeg produced no nonempty regular subtitle output"));
    }
    if metadata.len() > MAX_OUTPUT {
        return Err(error("converted subtitle exceeds 16 MiB budget"));
    }
    let mut bytes = Vec::new();
    fs::File::open(&output)?
        .take(MAX_OUTPUT + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_OUTPUT {
        return Err(error("converted subtitle exceeds 16 MiB budget"));
    }
    std::str::from_utf8(&bytes).map_err(|_| error("converted subtitle is not UTF-8"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{path::PathBuf, process::Command, sync::OnceLock};
    fn tool() -> &'static Path {
        static TOOL: OnceLock<PathBuf> = OnceLock::new();
        TOOL.get_or_init(|| {
            let directory = tempfile::tempdir().unwrap().keep();
            let tool = directory.join(format!("subtitle-runtime{}", std::env::consts::EXE_SUFFIX));
            assert!(
                Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
                    .arg(
                        Path::new(env!("CARGO_MANIFEST_DIR"))
                            .join("tests/helpers/subtitle_runtime.rs")
                    )
                    .args(["--edition=2024", "-Dwarnings", "-o"])
                    .arg(&tool)
                    .status()
                    .unwrap()
                    .success()
            );
            tool
        })
    }
    fn fixture(mode: &str, extension: &str) -> (tempfile::TempDir, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("mode"), mode).unwrap();
        let input = directory.path().join(format!("input.{extension}"));
        fs::write(&input, b"source subtitle").unwrap();
        (directory, input)
    }
    #[test]
    fn strict_local_formats_map_first_subtitle_and_preserve_input() {
        for input_ext in ["vtt", "srt", "ass", "ssa", "lrc"] {
            for output_ext in ["vtt", "srt", "ass", "lrc"] {
                let (directory, input) = fixture("success", input_ext);
                let output = directory.path().join(format!("output.{output_ext}"));
                convert(tool(), &input, input_ext, &output, output_ext, &mut || {
                    ControlFlow::Continue(())
                })
                .unwrap();
                assert_eq!(fs::read(output).unwrap(), b"converted subtitle");
                assert_eq!(fs::read(input).unwrap(), b"source subtitle");
            }
        }
    }
    #[test]
    fn process_errors_output_validation_and_deadlines_are_bounded() {
        for (mode, expected) in [
            ("bad-version", "banner"),
            ("fail", "exited"),
            ("missing", "nonempty"),
            ("empty", "nonempty"),
            ("stderr", "diagnostic"),
            ("large-stderr", "fixture tail"),
            ("large-stdout", "budget"),
            ("invalid-utf8", "UTF-8"),
            ("large-output", "16 MiB"),
            ("large-output-hang", "16 MiB"),
            ("hang", "deadline"),
            ("version-hang", "deadline"),
        ] {
            let (directory, input) = fixture(mode, "vtt");
            let started = Instant::now();
            let error = convert_with_deadline(
                tool(),
                &input,
                "vtt",
                &directory.path().join("output.srt"),
                "srt",
                &mut || ControlFlow::Continue(()),
                if matches!(mode, "hang" | "version-hang") {
                    Duration::from_millis(200)
                } else {
                    Duration::from_secs(5)
                },
            )
            .unwrap_err()
            .to_string();
            assert!(error.contains(expected), "{mode}: {error}");
            assert!(started.elapsed() < Duration::from_secs(5));
            assert_eq!(fs::read(input).unwrap(), b"source subtitle");
        }
    }
    #[test]
    fn cancellation_interrupts_version_and_conversion() {
        for mode in ["hang", "version-hang"] {
            let (directory, input) = fixture(mode, "srt");
            let started = Instant::now();
            let result = convert(
                tool(),
                &input,
                "srt",
                &directory.path().join("output.vtt"),
                "vtt",
                &mut || {
                    if started.elapsed() > Duration::from_millis(150) {
                        ControlFlow::Break(())
                    } else {
                        ControlFlow::Continue(())
                    }
                },
            );
            assert!(matches!(result, Err(Error::DownloadCancelled)));
            assert!(started.elapsed() < Duration::from_secs(3));
        }
    }
    #[test]
    fn unsupported_formats_and_occupied_paths_reject_without_launch() {
        let (directory, input) = fixture("success", "srt");
        let absent = directory.path().join("no-tool");
        for (source, target) in [
            ("json3", "srt"),
            ("srv1", "vtt"),
            ("ttml", "srt"),
            ("srt", "ssa"),
            ("srt", "json"),
        ] {
            assert!(
                convert(
                    &absent,
                    &input,
                    source,
                    &directory.path().join("output.vtt"),
                    target,
                    &mut || ControlFlow::Continue(())
                )
                .unwrap_err()
                .to_string()
                .contains("unsupported")
            );
        }
        let error = convert(tool(), &input, "srt", &input, "srt", &mut || {
            ControlFlow::Continue(())
        })
        .unwrap_err()
        .to_string();
        assert!(error.contains("differ"));
        let output = directory.path().join("output.vtt");
        fs::write(&output, b"competitor").unwrap();
        assert!(
            convert(tool(), &input, "srt", &output, "vtt", &mut || {
                ControlFlow::Continue(())
            })
            .unwrap_err()
            .to_string()
            .contains("exists")
        );
        assert_eq!(fs::read(output).unwrap(), b"competitor");
    }
    #[cfg(unix)]
    #[test]
    fn dangling_output_symlinks_are_rejected_without_touching_target() {
        let (directory, input) = fixture("success", "srt");
        let output = directory.path().join("output.vtt");
        let target = directory.path().join("absent");
        std::os::unix::fs::symlink(&target, &output).unwrap();
        assert!(
            convert(tool(), &input, "srt", &output, "vtt", &mut || {
                ControlFlow::Continue(())
            })
            .is_err()
        );
        assert!(
            fs::symlink_metadata(output)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(!target.exists());
    }
    #[test]
    fn optional_real_ffmpeg_preserves_cue_text_and_timing_across_four_formats() {
        let Some(ffmpeg) = std::env::var_os("YT_DLP_RS_TEST_FFMPEG") else {
            return;
        };
        let Some(ffprobe) = std::env::var_os("YT_DLP_RS_TEST_FFPROBE") else {
            return;
        };
        for target in ["srt", "vtt", "ass", "lrc"] {
            let directory = tempfile::tempdir().unwrap();
            let input = directory.path().join("input.srt");
            fs::write(
                &input,
                b"1\n00:00:01,250 --> 00:00:03,500\nHello <i>world</i>\n",
            )
            .unwrap();
            let output = directory.path().join(format!("output.{target}"));
            convert(
                Path::new(&ffmpeg),
                &input,
                "srt",
                &output,
                target,
                &mut || ControlFlow::Continue(()),
            )
            .unwrap();
            let text = fs::read_to_string(&output).unwrap();
            assert!(text.contains("Hello"), "{text}");
            assert!(text.contains("world"), "{text}");
            if target == "ass" {
                assert!(text.contains("{\\i1}"), "{text}");
            }
            if target == "srt" || target == "vtt" {
                assert!(text.contains("<i>world</i>"), "{text}");
            }
            if target == "lrc" {
                assert!(text.contains("[00:01.25]"), "{text}");
            }
            if target != "lrc" {
                assert!(
                    text.contains("1.25") || text.contains("01,250") || text.contains("01.250"),
                    "{text}"
                );
            }
            let probe = Command::new(&ffprobe)
                .args([
                    "-v",
                    "error",
                    "-show_entries",
                    "stream=codec_type",
                    "-of",
                    "json",
                ])
                .arg(&output)
                .output()
                .unwrap();
            assert!(
                probe.status.success(),
                "{}",
                String::from_utf8_lossy(&probe.stderr)
            );
            let value: serde_json::Value = serde_json::from_slice(&probe.stdout).unwrap();
            assert_eq!(value["streams"][0]["codec_type"], "subtitle");
        }
    }
}

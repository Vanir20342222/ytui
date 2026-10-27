//! Bounded FFmpeg processing of downloaded local media; no runtime discovery.
use super::VideoPlan;
use crate::{Error, Result};
use std::{
    ffi::OsString,
    ops::ControlFlow,
    path::Path,
    time::{Duration, Instant},
};
const DEADLINE: Duration = Duration::from_secs(600);
fn error(message: &str) -> Error {
    Error::Postprocess(message.into())
}
pub(crate) fn demuxer(extension: &str) -> Result<&'static str> {
    Ok(match extension.to_lowercase().as_str() {
        "mp4" | "m4a" | "mov" => "mov",
        "webm" | "mkv" | "mka" => "matroska",
        "avi" => "avi",
        "flv" => "flv",
        "gif" => "gif",
        "aac" => "aac",
        "aiff" | "aif" => "aiff",
        "flac" => "flac",
        "mp3" => "mp3",
        "ogg" | "opus" | "ogv" => "ogg",
        "wav" => "wav",
        "ts" => "mpegts",
        "wma" | "wmv" => "asf",
        "mpg" | "mpeg" => "mpeg",
        _ => return Err(error("unsupported input container for video processing")),
    })
}
pub(crate) fn convert(
    ffmpeg: &Path,
    input: &Path,
    input_ext: &str,
    output: &Path,
    plan: &VideoPlan,
    callback: &mut dyn FnMut() -> ControlFlow<()>,
) -> Result<()> {
    convert_with_deadline(ffmpeg, input, input_ext, output, plan, callback, DEADLINE)
}
fn convert_with_deadline(
    ffmpeg: &Path,
    input: &Path,
    input_ext: &str,
    output: &Path,
    plan: &VideoPlan,
    callback: &mut dyn FnMut() -> ControlFlow<()>,
    deadline: Duration,
) -> Result<()> {
    if plan.skip {
        return Err(error(
            "skipped video plans must be retained without conversion",
        ));
    }
    if plan.muxer.is_empty() {
        return Err(error(
            "the selected video target has no supported container muxer",
        ));
    }
    let demuxer = demuxer(input_ext)?;
    let started = Instant::now();
    let executable = ffmpeg.canonicalize()?;
    let input = input.canonicalize()?;
    if !executable.is_file() || !input.is_file() {
        return Err(error("video tool and input must be regular files"));
    }
    let directory = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .canonicalize()?;
    let output = directory.join(
        output
            .file_name()
            .ok_or_else(|| error("invalid video output path"))?,
    );
    if input == output {
        return Err(error("video output must differ from input"));
    }
    if let Ok(metadata) = std::fs::symlink_metadata(&output)
        && (!metadata.is_file() || metadata.file_type().is_symlink())
    {
        return Err(error("video output must be a regular file"));
    }
    let banner = crate::merge::execute(
        &executable,
        &["-version".into()],
        &directory,
        started,
        deadline,
        callback,
    )?;
    if !banner.starts_with(b"ffmpeg version ") {
        return Err(error(
            "video tool did not report the expected FFmpeg banner",
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
    .chain(crate::pp_args::input(plan.pp, 1))
    .chain(["-i".into(), input.into_os_string()])
    .chain(plan.metadata_input.iter().flat_map(|path| {
        ["-f", "ffmetadata", "-protocol_whitelist", "file"]
            .into_iter()
            .map(OsString::from)
            .chain(crate::pp_args::input(plan.pp, 2))
            .chain(["-i".into(), path.clone().into_os_string()])
    }))
    .chain(plan.args.iter().map(Into::into))
    .chain(["-movflags", "+faststart"].into_iter().map(Into::into))
    .chain(["-f", plan.muxer.as_str()].into_iter().map(Into::into))
    .chain(crate::pp_args::output(plan.pp))
    .chain(std::iter::once(output.clone().into_os_string()))
    .collect();
    crate::merge::execute(&executable, &args, &directory, started, deadline, callback)?;
    if !output.is_file() || output.metadata()?.len() == 0 {
        return Err(error("FFmpeg produced no nonempty video output"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{VideoMode, VideoOptions};
    use std::{path::PathBuf, process::Command, sync::OnceLock};
    fn tool() -> &'static Path {
        static TOOL: OnceLock<PathBuf> = OnceLock::new();
        TOOL.get_or_init(|| {
            let directory = tempfile::tempdir().unwrap().keep();
            let tool = directory.join(format!("video-runtime{}", std::env::consts::EXE_SUFFIX));
            assert!(
                Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
                    .arg(
                        Path::new(env!("CARGO_MANIFEST_DIR"))
                            .join("tests/helpers/video_runtime.rs")
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
    fn fixture(mode: &str, ext: &str) -> (tempfile::TempDir, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let input = directory.path().join(format!("input.{ext}"));
        std::fs::write(&input, b"source video").unwrap();
        std::fs::write(directory.path().join("mode"), mode).unwrap();
        (directory, input)
    }
    fn plan(mode: &str) -> VideoPlan {
        match mode {
            "recode" => VideoOptions::new("mp4", VideoMode::Recode),
            "avi" => VideoOptions::new("avi", VideoMode::Recode),
            _ => VideoOptions::new("mkv", VideoMode::Remux),
        }
        .unwrap()
        .plan("bin")
        .unwrap()
    }
    #[test]
    fn explicit_local_arguments_preserve_all_streams_data_exclusion_and_avi_codec() {
        for (mode, ext) in [
            ("remux", "mp4"),
            ("recode", "mkv"),
            ("avi", "mp4"),
            ("remux", "webm"),
            ("remux", "aac"),
            ("remux", "mp3"),
            ("remux", "ts"),
            ("remux", "gif"),
            ("remux", "wav"),
            ("remux", "m4a"),
            ("remux", "ogg"),
        ] {
            let (directory, input) = fixture(mode, ext);
            let plan = plan(mode);
            let output = directory.path().join(format!("output.{}", plan.extension));
            convert(tool(), &input, ext, &output, &plan, &mut || {
                ControlFlow::Continue(())
            })
            .unwrap();
            assert_eq!(std::fs::read(output).unwrap(), b"converted video");
            assert_eq!(std::fs::read(input).unwrap(), b"source video");
        }
    }
    #[test]
    fn malformed_tools_outputs_failures_and_deadlines_are_bounded() {
        tool();
        for (mode, message) in [
            ("bad-version", "banner"),
            ("stderr", "diagnostic"),
            ("large-stderr", "fixture tail"),
            ("large-stdout", "budget"),
            ("fail", "exited"),
            ("missing", "nonempty"),
            ("empty", "nonempty"),
            ("hang", "deadline"),
            ("version-hang", "deadline"),
        ] {
            let (directory, input) = fixture(mode, "mp4");
            let started = Instant::now();
            let error = convert_with_deadline(
                tool(),
                &input,
                "mp4",
                &directory.path().join("output.mkv"),
                &plan(mode),
                &mut || ControlFlow::Continue(()),
                if mode.contains("hang") {
                    Duration::from_millis(200)
                } else {
                    Duration::from_secs(5)
                },
            )
            .unwrap_err()
            .to_string();
            assert!(error.contains(message), "{mode}: {error}");
            assert!(started.elapsed() < Duration::from_secs(5));
            assert_eq!(std::fs::read(input).unwrap(), b"source video");
        }
    }
    #[test]
    fn cancellation_interrupts_version_and_conversion_and_invalid_plans_never_launch() {
        tool();
        for mode in ["version-hang", "hang"] {
            let (directory, input) = fixture(mode, "mp4");
            let started = Instant::now();
            let mut callback = || {
                if started.elapsed() > Duration::from_millis(150) {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            };
            assert!(matches!(
                convert(
                    tool(),
                    &input,
                    "mp4",
                    &directory.path().join("output.mkv"),
                    &plan(mode),
                    &mut callback
                ),
                Err(Error::DownloadCancelled)
            ));
            assert!(started.elapsed() < Duration::from_secs(3));
        }
        let absent = Path::new("nonexistent-video-tool");
        for plan in [
            VideoOptions::new("mp4", VideoMode::Remux)
                .unwrap()
                .plan("mp4")
                .unwrap(),
            VideoOptions::new("alac", VideoMode::Remux)
                .unwrap()
                .plan("mp4")
                .unwrap(),
            VideoOptions::new("vorbis", VideoMode::Recode)
                .unwrap()
                .plan("mp4")
                .unwrap(),
        ] {
            assert!(matches!(
                convert(absent, absent, "mp4", absent, &plan, &mut || {
                    ControlFlow::Continue(())
                }),
                Err(Error::Postprocess(_))
            ));
        }
        assert!(matches!(
            convert(absent, absent, "m3u8", absent, &plan("remux"), &mut || {
                ControlFlow::Continue(())
            }),
            Err(Error::Postprocess(_))
        ));
    }
    #[cfg(unix)]
    #[test]
    fn output_symlinks_are_rejected_before_launch_and_preserve_referent() {
        let (directory, input) = fixture("remux", "mp4");
        let target = directory.path().join("target");
        let output = directory.path().join("output.mkv");
        std::fs::write(&target, b"preserve").unwrap();
        std::os::unix::fs::symlink(&target, &output).unwrap();
        assert!(
            convert(tool(), &input, "mp4", &output, &plan("remux"), &mut || {
                ControlFlow::Continue(())
            })
            .is_err()
        );
        assert_eq!(std::fs::read(target).unwrap(), b"preserve");
    }
    #[test]
    fn real_explicit_tools_preserve_tracks_on_remux_and_select_avi_video_codec() {
        let (ffmpeg, ffprobe) = match (
            std::env::var_os("YT_DLP_RS_TEST_FFMPEG"),
            std::env::var_os("YT_DLP_RS_TEST_FFPROBE"),
        ) {
            (None, None) => return,
            (Some(ffmpeg), Some(ffprobe)) => (PathBuf::from(ffmpeg), PathBuf::from(ffprobe)),
            _ => panic!("set both YT_DLP_RS_TEST_FFMPEG and YT_DLP_RS_TEST_FFPROBE"),
        };
        let directory = tempfile::tempdir().unwrap();
        let input = directory.path().join("source.mp4");
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/merge");
        let status = Command::new(&ffmpeg)
            .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(fixtures.join("video.mp4"))
            .arg("-i")
            .arg(fixtures.join("audio.m4a"))
            .args(["-map", "0:v:0", "-map", "1:a:0", "-c", "copy", "-f", "mp4"])
            .arg(&input)
            .status()
            .unwrap();
        assert!(status.success());
        let probe = |path: &Path| {
            let output = Command::new(&ffprobe)
                .args([
                    "-v",
                    "error",
                    "-show_entries",
                    "stream=codec_type,codec_name,codec_tag_string",
                    "-of",
                    "json",
                ])
                .arg(path)
                .output()
                .unwrap();
            assert!(output.status.success());
            let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            value["streams"].as_array().unwrap().clone()
        };
        let original = probe(&input);
        assert_eq!(original.len(), 2);
        let source_bytes = std::fs::read(&input).unwrap();
        let encoders = Command::new(&ffmpeg)
            .args(["-hide_banner", "-encoders"])
            .output()
            .unwrap();
        assert!(encoders.status.success());
        let has_xvid = String::from_utf8_lossy(&encoders.stdout)
            .lines()
            .any(|line| line.split_whitespace().nth(1) == Some("libxvid"));
        for (target, mode) in [
            ("mkv", VideoMode::Remux),
            ("mp4", VideoMode::Recode),
            ("avi", VideoMode::Recode),
        ] {
            let plan = VideoOptions::new(target, mode)
                .unwrap()
                .plan("bin")
                .unwrap();
            let output = directory.path().join(format!("converted.{target}"));
            let result = convert(&ffmpeg, &input, "mp4", &output, &plan, &mut || {
                ControlFlow::Continue(())
            });
            if target == "avi" && !has_xvid {
                let message = result.unwrap_err().to_string();
                assert!(
                    message.contains("libxvid") && message.contains("encoder"),
                    "{message}"
                );
                assert_eq!(std::fs::read(&input).unwrap(), source_bytes);
                continue;
            }
            result.unwrap();
            let streams = probe(&output);
            assert_eq!(streams.len(), 2);
            let video = streams
                .iter()
                .find(|stream| stream["codec_type"] == "video")
                .unwrap();
            assert!(streams.iter().any(|stream| stream["codec_type"] == "audio"));
            if mode == VideoMode::Remux {
                assert_eq!(
                    streams
                        .iter()
                        .map(|stream| (&stream["codec_type"], &stream["codec_name"]))
                        .collect::<Vec<_>>(),
                    original
                        .iter()
                        .map(|stream| (&stream["codec_type"], &stream["codec_name"]))
                        .collect::<Vec<_>>()
                );
            }
            if target == "mp4" {
                let bytes = std::fs::read(&output).unwrap();
                let mut offset = 0;
                let mut moov = None;
                let mut mdat = None;
                while offset + 8 <= bytes.len() {
                    let size =
                        u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
                    match &bytes[offset + 4..offset + 8] {
                        b"moov" => moov = Some(offset),
                        b"mdat" => mdat = Some(offset),
                        _ => {}
                    }
                    if size < 8 {
                        break;
                    }
                    offset += size;
                }
                assert!(
                    moov.unwrap() < mdat.unwrap(),
                    "MP4 faststart must put metadata before media data"
                );
            }
            if target == "avi" {
                assert_eq!(video["codec_name"], "mpeg4");
                assert_eq!(video["codec_tag_string"], "XVID");
            }
        }
    }
}

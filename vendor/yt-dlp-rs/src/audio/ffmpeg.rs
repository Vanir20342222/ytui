//! Explicit local-file probing and bounded audio conversion. No discovery or shell.
use super::AudioPlan;
use crate::{Error, Result};
use serde::Deserialize;
use std::{
    ffi::OsString,
    ops::ControlFlow,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
const PROBE_DEADLINE: Duration = Duration::from_secs(30);
const CONVERT_DEADLINE: Duration = Duration::from_secs(600);
fn failure(message: &str) -> Error {
    Error::Postprocess(message.into())
}
fn demuxer(ext: &str) -> Result<&'static str> {
    Ok(match ext {
        "mp4" | "m4a" | "mov" => "mov",
        "webm" | "mkv" | "mka" => "matroska",
        "mp3" => "mp3",
        "aac" => "aac",
        "ogg" | "opus" => "ogg",
        "wav" => "wav",
        "aiff" | "aif" => "aiff",
        "flac" => "flac",
        "ts" => "mpegts",
        "avi" => "avi",
        "flv" => "flv",
        _ => return Err(failure("unsupported input container for audio processing")),
    })
}
fn paths(executable: &Path, input: &Path) -> Result<(PathBuf, PathBuf)> {
    let executable = executable.canonicalize()?;
    let input = input.canonicalize()?;
    if !executable.is_file() || !input.is_file() {
        return Err(failure("audio tool and input must be regular files"));
    }
    Ok((executable, input))
}
fn version(
    executable: &Path,
    directory: &Path,
    banner: &[u8],
    started: Instant,
    deadline: Duration,
    callback: &mut dyn FnMut() -> ControlFlow<()>,
) -> Result<()> {
    let output = crate::merge::execute(
        executable,
        &["-version".into()],
        directory,
        started,
        deadline,
        callback,
    )?;
    if !output.starts_with(banner) {
        return Err(failure(
            "audio tool did not report the expected version banner",
        ));
    }
    Ok(())
}
#[derive(Deserialize)]
struct Probe {
    streams: Vec<ProbeStream>,
}
#[derive(Deserialize)]
struct ProbeStream {
    codec_type: String,
    codec_name: Option<String>,
}

pub(crate) fn probe(
    ffprobe: &Path,
    input: &Path,
    input_ext: &str,
    callback: &mut dyn FnMut() -> ControlFlow<()>,
) -> Result<String> {
    probe_with_deadline(ffprobe, input, input_ext, callback, PROBE_DEADLINE)
}
fn probe_with_deadline(
    ffprobe: &Path,
    input: &Path,
    input_ext: &str,
    callback: &mut dyn FnMut() -> ControlFlow<()>,
    deadline: Duration,
) -> Result<String> {
    let started = Instant::now();
    let demuxer = demuxer(input_ext)?;
    let (executable, input) = paths(ffprobe, input)?;
    let directory = input
        .parent()
        .ok_or_else(|| failure("audio input has no parent directory"))?;
    version(
        &executable,
        directory,
        b"ffprobe version ",
        started,
        deadline,
        callback,
    )?;
    let args: Vec<OsString> = [
        "-v",
        "error",
        "-show_entries",
        "stream=codec_name,codec_type",
        "-of",
        "json",
        "-protocol_whitelist",
        "file",
        "-f",
        demuxer,
        "-i",
    ]
    .into_iter()
    .map(Into::into)
    .chain(std::iter::once(input.clone().into_os_string()))
    .collect();
    let output = crate::merge::execute(&executable, &args, directory, started, deadline, callback)?;
    let probe: Probe =
        serde_json::from_slice(&output).map_err(|_| failure("invalid audio probe JSON"))?;
    let codec = probe
        .streams
        .into_iter()
        .find(|stream| stream.codec_type == "audio")
        .and_then(|stream| stream.codec_name)
        .ok_or_else(|| failure("input has no identified audio stream"))?;
    if codec.is_empty()
        || codec.len() > 128
        || matches!(codec.as_str(), "unknown" | "none")
        || !codec.as_bytes()[0].is_ascii_alphanumeric()
        || !codec
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_.-".contains(&byte))
    {
        return Err(failure("invalid audio codec from probe"));
    }
    Ok(codec)
}

pub(crate) fn convert(
    ffmpeg: &Path,
    input: &Path,
    input_ext: &str,
    output: &Path,
    plan: &AudioPlan,
    callback: &mut dyn FnMut() -> ControlFlow<()>,
) -> Result<()> {
    convert_with_deadline(
        ffmpeg,
        input,
        input_ext,
        output,
        plan,
        callback,
        CONVERT_DEADLINE,
    )
}
fn convert_with_deadline(
    ffmpeg: &Path,
    input: &Path,
    input_ext: &str,
    output: &Path,
    plan: &AudioPlan,
    callback: &mut dyn FnMut() -> ControlFlow<()>,
    deadline: Duration,
) -> Result<()> {
    let started = Instant::now();
    let demuxer = demuxer(input_ext)?;
    if plan.skip {
        return Err(failure(
            "skipped audio plans must be retained without conversion",
        ));
    }
    let (executable, input) = paths(ffmpeg, input)?;
    let directory = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .canonicalize()?;
    let output = directory.join(
        output
            .file_name()
            .ok_or_else(|| failure("invalid audio output path"))?,
    );
    if output == input {
        return Err(failure("audio output must differ from input"));
    }
    if let Ok(metadata) = std::fs::symlink_metadata(&output)
        && (!metadata.is_file() || metadata.file_type().is_symlink())
    {
        return Err(failure("audio output must be a regular file"));
    }
    version(
        &executable,
        &directory,
        b"ffmpeg version ",
        started,
        deadline,
        callback,
    )?;
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
    .chain(crate::pp_args::input("ExtractAudio", 1))
    .chain(["-i".into(), input.into_os_string()])
    .chain(["-map", "0:a:0", "-vn"].into_iter().map(Into::into))
    .chain(
        plan.encoder
            .iter()
            .flat_map(|encoder| [OsString::from("-acodec"), OsString::from(encoder)]),
    )
    .chain(plan.args.iter().map(Into::into))
    .chain(["-f", plan.muxer.as_str()].into_iter().map(Into::into))
    .chain(crate::pp_args::output("ExtractAudio"))
    .chain(std::iter::once(output.clone().into_os_string()))
    .collect();
    crate::merge::execute(&executable, &args, &directory, started, deadline, callback)?;
    if !output.is_file() || output.metadata()?.len() == 0 {
        return Err(failure("FFmpeg produced no nonempty audio output"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{process::Command, sync::OnceLock};
    fn tools() -> &'static (PathBuf, PathBuf) {
        static TOOLS: OnceLock<(PathBuf, PathBuf)> = OnceLock::new();
        TOOLS.get_or_init(|| {
            let directory = tempfile::tempdir().unwrap().keep();
            let source =
                Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/helpers/audio_runtime.rs");
            let ffmpeg = directory.join(format!("fake-ffmpeg{}", std::env::consts::EXE_SUFFIX));
            let ffprobe = directory.join(format!("fake-ffprobe{}", std::env::consts::EXE_SUFFIX));
            assert!(
                Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
                    .arg(source)
                    .arg("--edition=2024")
                    .arg("-Dwarnings")
                    .arg("-o")
                    .arg(&ffmpeg)
                    .status()
                    .unwrap()
                    .success()
            );
            std::fs::copy(&ffmpeg, &ffprobe).unwrap();
            (ffmpeg, ffprobe)
        })
    }
    fn fixture(mode: &str, ext: &str) -> (tempfile::TempDir, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let input = directory.path().join(format!("input.{ext}"));
        std::fs::write(&input, b"source audio").unwrap();
        std::fs::write(directory.path().join("mode"), mode).unwrap();
        (directory, input)
    }
    fn plan(mode: &str) -> AudioPlan {
        match mode {
            "encode" => AudioPlan {
                extension: "mp3".into(),
                encoder: Some("libmp3lame".into()),
                args: vec!["-q:a".into(), "5".into()],
                muxer: "mp3".into(),
                skip: false,
            },
            "default-codec" => AudioPlan {
                extension: "wav".into(),
                encoder: None,
                args: vec![],
                muxer: "wav".into(),
                skip: false,
            },
            _ => AudioPlan {
                extension: "m4a".into(),
                encoder: Some("copy".into()),
                args: vec![],
                muxer: "ipod".into(),
                skip: false,
            },
        }
    }
    #[test]
    fn probe_checks_fixed_arguments_and_reports_only_first_audio_codec() {
        let tools = tools();
        for ext in [
            "mp4", "m4a", "mov", "webm", "mkv", "mka", "mp3", "aac", "ogg", "opus", "wav", "flac",
            "ts", "avi", "flv", "aiff", "aif",
        ] {
            let (_directory, input) = fixture("copy", ext);
            assert_eq!(
                probe(&tools.1, &input, ext, &mut || ControlFlow::Continue(())).unwrap(),
                "aac"
            );
        }
    }
    #[test]
    fn conversion_checks_fixed_copy_encoding_and_default_codec_arguments() {
        let tools = tools();
        for mode in ["copy", "encode", "default-codec"] {
            let (directory, input) = fixture(mode, "mp4");
            let plan = plan(mode);
            let output = directory.path().join(format!("output.{}", plan.extension));
            convert(&tools.0, &input, "mp4", &output, &plan, &mut || {
                ControlFlow::Continue(())
            })
            .unwrap();
            assert_eq!(std::fs::read(output).unwrap(), b"processed audio");
        }
    }
    #[test]
    fn invalid_probe_outputs_are_errors() {
        for (mode, message) in [
            ("no-audio", "no identified audio"),
            ("invalid-json", "JSON"),
            ("missing-codec", "no identified audio"),
            ("invalid-codec", "invalid audio codec"),
            ("bad-version", "version banner"),
        ] {
            let (_directory, input) = fixture(mode, "mp4");
            let error = probe(&tools().1, &input, "mp4", &mut || ControlFlow::Continue(()))
                .unwrap_err()
                .to_string();
            assert!(error.contains(message), "{mode}: {error}");
        }
    }
    #[test]
    fn shared_runner_bounds_failures_and_reaps_processes() {
        tools();
        for (mode, message) in [
            ("stderr", "diagnostic"),
            ("exit", "exited"),
            ("stdout-flood", "budget"),
            ("stderr-flood", "fixture tail"),
            ("hang", "deadline"),
            ("version-hang", "deadline"),
        ] {
            let (_directory, input) = fixture(mode, "mp4");
            let started = Instant::now();
            let deadline = if mode.contains("hang") {
                Duration::from_millis(150)
            } else {
                Duration::from_secs(3)
            };
            let error = probe_with_deadline(
                &tools().1,
                &input,
                "mp4",
                &mut || ControlFlow::Continue(()),
                deadline,
            )
            .unwrap_err()
            .to_string();
            assert!(error.contains(message), "{mode}: {error}");
            assert!(started.elapsed() < Duration::from_secs(5));
        }
        for mode in ["empty", "missing"] {
            let (directory, input) = fixture(mode, "mp4");
            let error = convert(
                &tools().0,
                &input,
                "mp4",
                &directory.path().join("output.m4a"),
                &plan(mode),
                &mut || ControlFlow::Continue(()),
            )
            .unwrap_err()
            .to_string();
            assert!(error.contains("nonempty"), "{mode}: {error}");
        }
    }
    #[test]
    fn cancellation_interrupts_probe_version_and_conversion() {
        tools();
        for (mode, conversion) in [("version-hang", false), ("hang", false), ("hang", true)] {
            let (directory, input) = fixture(mode, "mp4");
            let started = Instant::now();
            let mut callback = || {
                if started.elapsed() >= Duration::from_millis(150) {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            };
            let result = if conversion {
                convert_with_deadline(
                    &tools().0,
                    &input,
                    "mp4",
                    &directory.path().join("output.m4a"),
                    &plan(mode),
                    &mut callback,
                    Duration::from_secs(3),
                )
                .map(|()| String::new())
            } else {
                probe_with_deadline(
                    &tools().1,
                    &input,
                    "mp4",
                    &mut callback,
                    Duration::from_secs(3),
                )
            };
            assert!(matches!(result, Err(Error::DownloadCancelled)));
            assert!(started.elapsed() < Duration::from_secs(2));
        }
    }
    #[test]
    fn invalid_media_container_and_output_paths_do_not_launch_tools() {
        let (directory, input) = fixture("copy", "mp4");
        let absent = directory.path().join("absent");
        assert!(matches!(
            probe(&absent, &input, "m3u8", &mut || ControlFlow::Continue(())),
            Err(Error::Postprocess(_))
        ));
        assert!(
            convert(
                &tools().0,
                &input,
                "mp4",
                &input,
                &plan("copy"),
                &mut || ControlFlow::Continue(())
            )
            .is_err()
        );
        assert!(
            convert(
                &tools().0,
                &input,
                "mp4",
                directory.path(),
                &plan("copy"),
                &mut || ControlFlow::Continue(())
            )
            .is_err()
        );
        let mut skipped = plan("copy");
        skipped.skip = true;
        assert!(matches!(
            convert(
                &absent,
                &input,
                "mp4",
                &directory.path().join("output.m4a"),
                &skipped,
                &mut || ControlFlow::Continue(())
            ),
            Err(Error::Postprocess(_))
        ));
    }
    #[test]
    fn real_explicit_tools_probe_and_process_supported_audio_targets() {
        let (ffmpeg, ffprobe) = match (
            std::env::var_os("YT_DLP_RS_TEST_FFMPEG"),
            std::env::var_os("YT_DLP_RS_TEST_FFPROBE"),
        ) {
            (None, None) => return,
            (Some(ffmpeg), Some(ffprobe)) => (PathBuf::from(ffmpeg), PathBuf::from(ffprobe)),
            _ => panic!("set both YT_DLP_RS_TEST_FFMPEG and YT_DLP_RS_TEST_FFPROBE"),
        };
        let input = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/merge/audio.m4a");
        assert_eq!(
            probe(&ffprobe, &input, "m4a", &mut || ControlFlow::Continue(())).unwrap(),
            "aac"
        );
        for (target, expected) in [
            ("mp3", "mp3"),
            ("aac", "aac"),
            ("m4a", "aac"),
            ("opus", "opus"),
            ("vorbis", "vorbis"),
            ("flac", "flac"),
            ("alac", "alac"),
            ("wav", "pcm_s16le"),
        ] {
            let options = super::super::AudioOptions::new(target, "5").unwrap();
            let plan = options.plan("mp4", Some("aac")).unwrap();
            let directory = tempfile::tempdir().unwrap();
            let output = directory.path().join(format!("output.{}", plan.extension));
            convert(&ffmpeg, &input, "m4a", &output, &plan, &mut || {
                ControlFlow::Continue(())
            })
            .unwrap();
            // Upstream's AAC-copy plan labels ADTS output m4a; use the explicit
            // actual container when testing the probe instead of guessing by name.
            let input_ext = if plan.muxer == "adts" {
                "aac"
            } else {
                plan.extension.as_str()
            };
            assert_eq!(
                probe(&ffprobe, &output, input_ext, &mut || ControlFlow::Continue(
                    ()
                ))
                .unwrap(),
                expected,
                "{target}"
            );
        }
    }
    #[cfg(unix)]
    #[test]
    fn output_symlinks_are_rejected_without_touching_target() {
        let (directory, input) = fixture("copy", "mp4");
        let target = directory.path().join("target");
        std::fs::write(&target, b"preserve").unwrap();
        let output = directory.path().join("output.m4a");
        std::os::unix::fs::symlink(&target, &output).unwrap();
        assert!(
            convert(
                &tools().0,
                &input,
                "mp4",
                &output,
                &plan("copy"),
                &mut || ControlFlow::Continue(())
            )
            .is_err()
        );
        assert_eq!(std::fs::read(target).unwrap(), b"preserve");
    }
}

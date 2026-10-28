//! Bounded embedding of already downloaded local subtitles, without shell or discovery.
use super::EmbedPlan;
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
fn media_demuxer(extension: &str) -> Result<&'static str> {
    Ok(match extension {
        "mp4" | "mov" | "m4a" => "mov",
        "webm" | "mkv" | "mka" => "matroska",
        _ => return Err(error("unsupported media container for subtitle embedding")),
    })
}
fn subtitle_demuxer(extension: &str) -> Result<&'static str> {
    Ok(match extension {
        "srt" => "srt",
        "vtt" => "webvtt",
        "ass" | "ssa" => "ass",
        "lrc" => "lrc",
        _ => {
            return Err(error(
                "unsupported subtitle container for embedding; convert it first",
            ));
        }
    })
}
pub(crate) fn embed(
    ffmpeg: &Path,
    input: &Path,
    input_ext: &str,
    output: &Path,
    plan: &EmbedPlan,
    callback: &mut dyn FnMut() -> ControlFlow<()>,
) -> Result<()> {
    embed_with_deadline(ffmpeg, input, input_ext, output, plan, callback, DEADLINE)
}
fn embed_with_deadline(
    ffmpeg: &Path,
    input: &Path,
    input_ext: &str,
    output: &Path,
    plan: &EmbedPlan,
    callback: &mut dyn FnMut() -> ControlFlow<()>,
    deadline: Duration,
) -> Result<()> {
    if plan.skip || plan.files.is_empty() {
        return Err(error(
            "skipped subtitle embedding plans must be retained without conversion",
        ));
    }
    let demuxer = media_demuxer(input_ext)?;
    let started = Instant::now();
    let executable = ffmpeg.canonicalize()?;
    let input = input.canonicalize()?;
    if !executable.is_file() || !input.is_file() {
        return Err(error("embedding tool and input must be regular files"));
    }
    let directory = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .canonicalize()?;
    let output = directory.join(
        output
            .file_name()
            .ok_or_else(|| error("invalid embedded output path"))?,
    );
    if output == input {
        return Err(error("embedded output must differ from input"));
    }
    if let Ok(metadata) = std::fs::symlink_metadata(&output)
        && (!metadata.is_file() || metadata.file_type().is_symlink())
    {
        return Err(error("embedded output must be regular"));
    }
    let mut args: Vec<OsString> = ["-nostdin", "-hide_banner", "-loglevel", "repeat+info", "-y"]
        .into_iter()
        .map(Into::into)
        .collect();
    args.extend(
        ["-f", demuxer, "-protocol_whitelist", "file"]
            .into_iter()
            .map(OsString::from),
    );
    args.extend(crate::pp_args::input("EmbedSubtitle", 1));
    args.push("-i".into());
    args.push(input.into_os_string());
    for (index, file) in plan.files.iter().enumerate() {
        let demuxer = subtitle_demuxer(&file.ext)?;
        let path = file.path.canonicalize()?;
        if !path.is_file() {
            return Err(error("subtitle embedding input must be regular"));
        }
        if path == output {
            return Err(error("embedded output must differ from subtitle inputs"));
        }
        args.extend(
            ["-f", demuxer, "-protocol_whitelist", "file"]
                .into_iter()
                .map(OsString::from),
        );
        args.extend(crate::pp_args::input("EmbedSubtitle", index + 2));
        args.push("-i".into());
        args.push(path.into_os_string());
    }
    args.extend(plan.args.iter().map(OsString::from));
    args.extend(
        ["-movflags", "+faststart", "-f", &plan.muxer]
            .into_iter()
            .map(OsString::from),
    );
    args.extend(crate::pp_args::output("EmbedSubtitle"));
    args.push(output.clone().into_os_string());
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
            "embedding tool did not report the expected FFmpeg banner",
        ));
    }
    crate::merge::execute(&executable, &args, &directory, started, deadline, callback)?;
    if !output.is_file() || output.metadata()?.len() == 0 {
        return Err(error("FFmpeg produced no nonempty embedded output"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embed_subtitles::{SubtitleFile, plan};
    use std::{path::PathBuf, process::Command, sync::OnceLock};
    fn tool() -> &'static Path {
        static TOOL: OnceLock<PathBuf> = OnceLock::new();
        TOOL.get_or_init(|| {
            let directory = tempfile::tempdir().unwrap().keep();
            let tool = directory.join(format!("embed-runtime{}", std::env::consts::EXE_SUFFIX));
            assert!(
                Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
                    .arg(
                        Path::new(env!("CARGO_MANIFEST_DIR"))
                            .join("tests/helpers/embed_runtime.rs")
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
    fn fixture(mode: &str, container: &str) -> (tempfile::TempDir, PathBuf, PathBuf, EmbedPlan) {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let input = root.join(format!("input.{container}"));
        let sub = root.join("input.en.vtt");
        let output = root.join(format!("output.{container}"));
        std::fs::write(&input, b"source media").unwrap();
        std::fs::write(&sub, b"subtitle content").unwrap();
        std::fs::write(directory.path().join("mode"), mode).unwrap();
        let plan = plan(
            container,
            &[SubtitleFile {
                path: sub.clone(),
                ext: "vtt".into(),
                language: "en".into(),
                name: Some("English captions".into()),
            }],
        )
        .unwrap();
        let mut expected: Vec<OsString> = [
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "repeat+info",
            "-y",
            "-f",
            if matches!(container, "mp4" | "mov" | "m4a") {
                "mov"
            } else {
                "matroska"
            },
            "-protocol_whitelist",
            "file",
            "-i",
        ]
        .into_iter()
        .map(Into::into)
        .collect();
        expected.push(input.clone().into_os_string());
        expected.extend(
            ["-f", "webvtt", "-protocol_whitelist", "file", "-i"]
                .into_iter()
                .map(OsString::from),
        );
        expected.push(sub.into_os_string());
        expected.extend(plan.args.iter().map(OsString::from));
        expected.extend(
            ["-movflags", "+faststart", "-f", &plan.muxer]
                .into_iter()
                .map(OsString::from),
        );
        expected.push(output.clone().into_os_string());
        std::fs::write(
            directory.path().join("expected"),
            expected
                .iter()
                .map(|arg| arg.to_str().unwrap())
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
        (directory, input, output, plan)
    }
    #[test]
    fn every_source_container_has_explicit_local_inputs_and_metadata_arguments() {
        for container in ["mp4", "mov", "m4a", "webm", "mkv", "mka"] {
            let (_directory, input, output, plan) = fixture("okay", container);
            embed(tool(), &input, container, &output, &plan, &mut || {
                ControlFlow::Continue(())
            })
            .unwrap();
            assert_eq!(std::fs::read(&output).unwrap(), b"embedded media");
            assert_eq!(std::fs::read(&input).unwrap(), b"source media");
            assert_eq!(
                std::fs::read(&plan.files[0].path).unwrap(),
                b"subtitle content"
            );
        }
    }
    #[test]
    fn sidecar_formats_use_actual_demuxer_names_instead_of_codec_names() {
        for (extension, demuxer) in [
            ("srt", "srt"),
            ("vtt", "webvtt"),
            ("ass", "ass"),
            ("ssa", "ass"),
            ("lrc", "lrc"),
        ] {
            let (directory, input, output, mut plan) = fixture("okay", "mp4");
            plan.files[0].ext = extension.into();
            let expected_path = directory.path().join("expected");
            let expected = std::fs::read_to_string(&expected_path)
                .unwrap()
                .replace("webvtt", demuxer);
            std::fs::write(expected_path, expected).unwrap();
            embed(tool(), &input, "mp4", &output, &plan, &mut || {
                ControlFlow::Continue(())
            })
            .unwrap();
        }
    }

    #[test]
    fn shared_process_failures_deadlines_and_empty_results_are_bounded() {
        tool();
        for (mode, message) in [
            ("bad-version", "banner"),
            ("stderr", "diagnostic"),
            ("stdout-flood", "budget"),
            ("stderr-flood", "fixture tail"),
            ("fail", "exited"),
            ("empty", "nonempty"),
            ("missing", "nonempty"),
            ("hang", "deadline"),
            ("version-hang", "deadline"),
        ] {
            let (_directory, input, output, plan) = fixture(mode, "mp4");
            let started = Instant::now();
            let error = embed_with_deadline(
                tool(),
                &input,
                "mp4",
                &output,
                &plan,
                &mut || ControlFlow::Continue(()),
                if mode.contains("hang") {
                    Duration::from_millis(200)
                } else {
                    Duration::from_secs(3)
                },
            )
            .unwrap_err()
            .to_string();
            assert!(error.contains(message), "{mode}: {error}");
            assert!(started.elapsed() < Duration::from_secs(5));
            assert_eq!(std::fs::read(input).unwrap(), b"source media");
        }
    }
    #[test]
    fn cancellation_and_skip_unsupported_formats_and_aliases_never_damage_inputs() {
        tool();
        for mode in ["version-hang", "hang"] {
            let (_directory, input, output, plan) = fixture(mode, "mp4");
            let started = Instant::now();
            let mut callback = || {
                if started.elapsed() > Duration::from_millis(150) {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            };
            assert!(matches!(
                embed(tool(), &input, "mp4", &output, &plan, &mut callback),
                Err(Error::DownloadCancelled)
            ));
        }
        let (_directory, input, output, mut plan) = fixture("okay", "mp4");
        assert!(
            embed(tool(), &input, "mp4", &input, &plan, &mut || {
                ControlFlow::Continue(())
            })
            .is_err()
        );
        plan.files[0].ext = "json3".into();
        assert!(
            embed(tool(), &input, "mp4", &output, &plan, &mut || {
                ControlFlow::Continue(())
            })
            .unwrap_err()
            .to_string()
            .contains("convert it first")
        );
        plan.skip = true;
        assert!(matches!(
            embed(
                Path::new("absent-tool"),
                &input,
                "mp4",
                &output,
                &plan,
                &mut || ControlFlow::Continue(())
            ),
            Err(Error::Postprocess(_))
        ));
    }
    #[test]
    fn real_tools_preserve_media_replace_existing_subtitles_and_set_language_name_codec() {
        let (ffmpeg, ffprobe) = match (
            std::env::var_os("YT_DLP_RS_TEST_FFMPEG"),
            std::env::var_os("YT_DLP_RS_TEST_FFPROBE"),
        ) {
            (None, None) => return,
            (Some(ffmpeg), Some(ffprobe)) => (PathBuf::from(ffmpeg), PathBuf::from(ffprobe)),
            _ => panic!("set both explicit FFmpeg tools"),
        };
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source.mp4");
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/merge");
        let status = Command::new(&ffmpeg)
            .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(fixtures.join("video.mp4"))
            .arg("-i")
            .arg(fixtures.join("audio.m4a"))
            .args(["-map", "0:v:0", "-map", "1:a:0", "-c", "copy", "-f", "mp4"])
            .arg(&source)
            .status()
            .unwrap();
        assert!(status.success());
        let caption = directory.path().join("caption.vtt");
        std::fs::write(
            &caption,
            b"WEBVTT\n\n00:00:00.000 --> 00:00:00.400\nhello captions\n",
        )
        .unwrap();
        let probe = |path: &Path| {
            let result = Command::new(&ffprobe)
                .args([
                    "-v",
                    "error",
                    "-show_entries",
                    "stream=codec_type,codec_name:stream_tags=language,title,handler_name",
                    "-of",
                    "json",
                ])
                .arg(path)
                .output()
                .unwrap();
            assert!(result.status.success());
            let value: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
            value["streams"].as_array().unwrap().clone()
        };
        let srt = directory.path().join("caption.srt");
        std::fs::write(
            &srt,
            b"1\n00:00:00,000 --> 00:00:00,400\nhello captions\n\n",
        )
        .unwrap();
        for (container, codec, extension) in [
            ("mp4", "mov_text", "srt"),
            ("mkv", "subrip", "srt"),
            ("webm", "webvtt", "vtt"),
        ] {
            let input = directory.path().join(format!("input.{container}"));
            if container == "mp4" {
                std::fs::copy(&source, &input).unwrap();
            } else {
                let mut command = Command::new(&ffmpeg);
                command
                    .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y", "-i"])
                    .arg(&source);
                if container == "webm" {
                    command.args(["-c:v", "libvpx-vp9", "-c:a", "libopus"]);
                } else {
                    command.args(["-c", "copy"]);
                }
                assert!(command.arg(&input).status().unwrap().success());
            }
            let original = probe(&input);
            let expected_media: Vec<_> = original
                .iter()
                .filter(|stream| stream["codec_type"] != "subtitle")
                .map(|stream| (stream["codec_type"].clone(), stream["codec_name"].clone()))
                .collect();
            let mut current = input;
            for (index, language, name) in [
                (0, "en-US", "English captions"),
                (1, "fr", "French captions"),
            ] {
                let plan = plan(
                    container,
                    &[SubtitleFile {
                        path: if extension == "srt" {
                            srt.clone()
                        } else {
                            caption.clone()
                        },
                        ext: extension.into(),
                        language: language.into(),
                        name: Some(name.into()),
                    }],
                )
                .unwrap();
                let output = directory
                    .path()
                    .join(format!("embedded{index}.{container}"));
                embed(&ffmpeg, &current, container, &output, &plan, &mut || {
                    ControlFlow::Continue(())
                })
                .unwrap();
                let streams = probe(&output);
                let media: Vec<_> = streams
                    .iter()
                    .filter(|stream| stream["codec_type"] != "subtitle")
                    .map(|stream| (stream["codec_type"].clone(), stream["codec_name"].clone()))
                    .collect();
                assert_eq!(media, expected_media);
                let subtitles: Vec<_> = streams
                    .iter()
                    .filter(|stream| stream["codec_type"] == "subtitle")
                    .collect();
                assert_eq!(subtitles.len(), 1);
                assert_eq!(subtitles[0]["codec_name"], codec);
                let tags = &subtitles[0]["tags"];
                assert_eq!(tags["language"], if index == 0 { "eng" } else { "fra" });
                if container == "mp4" {
                    assert_eq!(tags["handler_name"], name);
                } else {
                    assert_eq!(tags["title"], name);
                }
                current = output;
            }
        }
    }
}

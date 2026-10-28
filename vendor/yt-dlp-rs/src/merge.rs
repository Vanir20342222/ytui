//! Explicit, bounded FFmpeg stream copying of already downloaded local files.
use crate::{Error, Result};
use std::{
    ffi::OsString,
    io::Read,
    ops::ControlFlow,
    path::Path,
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

const DEADLINE: Duration = Duration::from_secs(600);
const MAX_STDOUT: usize = 64 * 1024;
const MAX_STDERR: usize = 256 * 1024;

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn merge(
    executable: &Path,
    video: &Path,
    video_ext: &str,
    audio: &Path,
    audio_ext: &str,
    output: &Path,
    output_ext: &str,
    callback: &mut dyn FnMut() -> ControlFlow<()>,
) -> Result<()> {
    merge_with_deadline(
        executable, video, video_ext, audio, audio_ext, output, output_ext, callback, DEADLINE,
    )
}

/// One merge input: a downloaded stream and which of its streams to map.
pub(crate) struct Input<'a> {
    pub path: &'a Path,
    pub ext: &'a str,
    /// Upstream `acodec != 'none'` / `vcodec != 'none'`.
    pub audio: bool,
    pub video: bool,
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn merge_with_deadline(
    executable: &Path,
    video: &Path,
    video_ext: &str,
    audio: &Path,
    audio_ext: &str,
    output: &Path,
    output_ext: &str,
    callback: &mut dyn FnMut() -> ControlFlow<()>,
    deadline: Duration,
) -> Result<()> {
    // HLS inputs are labeled `mp4` but may hold MPEG-TS or raw AAC.
    let video_ext = crate::container::sniff(video).unwrap_or(video_ext);
    if !matches!(video_ext, "mp4" | "ts" | "webm") {
        return Err(failure("unsupported video container"));
    }
    merge_inputs_with_deadline(
        executable,
        &[
            Input {
                path: video,
                ext: video_ext,
                audio: false,
                video: true,
            },
            Input {
                path: audio,
                ext: audio_ext,
                audio: true,
                video: false,
            },
        ],
        output,
        output_ext,
        callback,
        deadline,
    )
}

/// Upstream `FFmpegMergerPP` over any number of inputs (`--video-multistreams`
/// and `--audio-multistreams`).
pub(crate) fn merge_inputs(
    executable: &Path,
    inputs: &[Input<'_>],
    output: &Path,
    output_ext: &str,
    callback: &mut dyn FnMut() -> ControlFlow<()>,
) -> Result<()> {
    merge_inputs_with_deadline(executable, inputs, output, output_ext, callback, DEADLINE)
}

fn merge_inputs_with_deadline(
    executable: &Path,
    inputs: &[Input<'_>],
    output: &Path,
    output_ext: &str,
    callback: &mut dyn FnMut() -> ControlFlow<()>,
    deadline: Duration,
) -> Result<()> {
    let started = Instant::now();
    let mut demuxers = Vec::with_capacity(inputs.len());
    for input in inputs {
        let ext = crate::container::sniff(input.path).unwrap_or(input.ext);
        demuxers.push(match ext {
            "m4a" | "mp4" => "mov",
            "aac" => "aac",
            "ts" => "mpegts",
            "webm" => "matroska",
            _ => return Err(failure("unsupported input container")),
        });
    }
    // FFmpegMergerPP.SUPPORTED_EXTS, plus `ts` that get_compatible_ext can pick.
    let muxer = match output_ext {
        "mp4" => "mp4",
        "webm" => "webm",
        "mkv" => "matroska",
        "mov" => "mov",
        "avi" => "avi",
        "flv" => "flv",
        "ts" => "mpegts",
        _ => return Err(failure("unsupported output container")),
    };
    let executable = executable.canonicalize()?;
    let paths = inputs
        .iter()
        .map(|input| input.path.canonicalize())
        .collect::<std::io::Result<Vec<_>>>()?;
    if !executable.is_file() || paths.iter().any(|path| !path.is_file()) {
        return Err(failure("executable and inputs must be files"));
    }
    let directory = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .canonicalize()?;
    let output = directory.join(
        output
            .file_name()
            .ok_or_else(|| failure("invalid output path"))?,
    );
    // Existing symlinks must never redirect FFmpeg outside the private directory.
    if let Ok(metadata) = std::fs::symlink_metadata(&output)
        && (!metadata.is_file() || metadata.file_type().is_symlink())
    {
        return Err(failure("output must be a regular file"));
    }
    if paths.contains(&output) {
        return Err(failure("output must differ from inputs"));
    }
    let version = execute(
        &executable,
        &["-version".into()],
        &directory,
        started,
        deadline,
        callback,
    )?;
    if !version.starts_with(b"ffmpeg version ") {
        return Err(failure(
            "executable did not report an FFmpeg version banner",
        ));
    }
    // `--postprocessor-args` go before each `-i` and the output file.
    let mut args: Vec<OsString> = ["-nostdin", "-hide_banner", "-loglevel", "repeat+info", "-y"]
        .into_iter()
        .map(OsString::from)
        .collect();
    for (index, (path, demuxer)) in paths.iter().zip(&demuxers).enumerate() {
        args.extend(
            ["-f", demuxer, "-protocol_whitelist", "file"]
                .into_iter()
                .map(OsString::from),
        );
        args.extend(crate::pp_args::input("Merger", index + 1));
        args.extend(["-i".into(), path.clone().into_os_string()]);
    }
    // Upstream maps each input's audio, then its video.
    for (index, input) in inputs.iter().enumerate() {
        if input.audio {
            args.extend(["-map".into(), format!("{index}:a:0").into()]);
        }
        if input.video {
            args.extend(["-map".into(), format!("{index}:v:0").into()]);
        }
    }
    args.extend(["-c", "copy", "-f", muxer].into_iter().map(OsString::from));
    args.extend(crate::pp_args::output("Merger"));
    args.push(output.clone().into_os_string());
    execute(&executable, &args, &directory, started, deadline, callback)?;
    if !output.is_file() || output.metadata()?.len() == 0 {
        return Err(failure("FFmpeg produced no nonempty output"));
    }
    Ok(())
}

fn failure(message: impl Into<String>) -> Error {
    Error::Postprocess(message.into())
}

/// Read a pipe to its end, keeping only the last `cap` bytes.
fn read_tail(mut pipe: impl Read, cap: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 8192];
    loop {
        let count = pipe.read(&mut buffer)?;
        if count == 0 {
            return Ok(bytes);
        }
        bytes.extend_from_slice(&buffer[..count]);
        if bytes.len() > 2 * cap {
            bytes.drain(..bytes.len() - cap);
        }
    }
}

fn read_pipe(mut pipe: impl Read, cap: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 8192];
    loop {
        let count = pipe.read(&mut buffer)?;
        if count == 0 {
            return Ok(bytes);
        }
        if bytes.len() + count > cap {
            return Err(failure("FFmpeg output budget exceeded"));
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
}

pub(crate) fn execute(
    executable: &Path,
    args: &[OsString],
    directory: &Path,
    started: Instant,
    deadline: Duration,
    callback: &mut dyn FnMut() -> ControlFlow<()>,
) -> Result<Vec<u8>> {
    if callback().is_break() {
        return Err(Error::DownloadCancelled);
    }
    if started.elapsed() >= deadline {
        return Err(failure("FFmpeg wall deadline exceeded"));
    }
    // Version probes are not part of upstream's debug output.
    if args != [OsString::from("-version")] {
        crate::screen::debug_command(executable, args);
    }
    let mut command = Command::new(executable);
    command
        .args(args)
        .env_clear()
        .current_dir(directory)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    for name in ["SystemRoot", "WINDIR"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    let mut child = command.spawn()?;
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let (sender, receiver) = mpsc::channel();
    thread::scope(|scope| {
        let tx = sender.clone();
        if let Err(error) = thread::Builder::new().spawn_scoped(scope, move || {
            let _ = tx.send((false, read_pipe(stdout, MAX_STDOUT)));
        }) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error.into());
        }
        if let Err(error) = thread::Builder::new().spawn_scoped(scope, move || {
            let _ = sender.send((true, read_tail(stderr, MAX_STDERR)));
        }) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error.into());
        }
        let mut output = None;
        let mut errors = None;
        let result = loop {
            if callback().is_break() {
                break Err(Error::DownloadCancelled);
            }
            if started.elapsed() >= deadline {
                break Err(failure("FFmpeg wall deadline exceeded"));
            }
            match receiver.recv_timeout(Duration::from_millis(5)) {
                Ok((_, Err(error))) => break Err(error),
                Ok((false, Ok(bytes))) => output = Some(bytes),
                Ok((true, Ok(bytes))) => errors = Some(bytes),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    thread::sleep(Duration::from_millis(5))
                }
            }
            match child.try_wait() {
                // Upstream checks only the exit status and raises
                // `stderr.strip().splitlines()[-1]`.
                Ok(Some(status)) if !status.success() && errors.is_some() => {
                    let text = String::from_utf8_lossy(errors.as_deref().unwrap_or_default());
                    break Err(failure(match text.trim().lines().last() {
                        Some(last) => last.to_owned(),
                        None => format!("FFmpeg exited with {status}"),
                    }));
                }
                Ok(Some(_)) if output.is_some() && errors.is_some() => {
                    break Ok(output.take().unwrap());
                }
                Ok(_) => {}
                Err(error) => break Err(error.into()),
            }
        };
        if result.is_err() {
            let _ = child.kill();
        }
        let waited = child.wait();
        match (result, waited) {
            (Err(error), _) => Err(error),
            (Ok(output), Ok(_)) => Ok(output),
            (Ok(_), Err(error)) => Err(error.into()),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{path::PathBuf, sync::OnceLock};
    fn fake() -> &'static Path {
        static HELPER: OnceLock<PathBuf> = OnceLock::new();
        HELPER.get_or_init(|| {
            let directory = tempfile::tempdir().unwrap().keep();
            let source =
                Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/helpers/ffmpeg_runtime.rs");
            let binary = directory.join(format!("fake-ffmpeg{}", std::env::consts::EXE_SUFFIX));
            let status = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
                .arg(source)
                .arg("--edition=2024")
                .arg("-Dwarnings")
                .arg("-o")
                .arg(&binary)
                .status()
                .unwrap();
            assert!(status.success());
            binary
        })
    }
    fn run(
        mode: &str,
        deadline: Duration,
        callback: &mut dyn FnMut() -> ControlFlow<()>,
    ) -> Result<Vec<u8>> {
        run_containers(mode, "mp4", "m4a", "mp4", deadline, callback)
    }
    fn run_containers(
        mode: &str,
        video_ext: &str,
        audio_ext: &str,
        output_ext: &str,
        deadline: Duration,
        callback: &mut dyn FnMut() -> ControlFlow<()>,
    ) -> Result<Vec<u8>> {
        let directory = tempfile::tempdir().unwrap();
        let video = directory.path().join(format!("video.{video_ext}"));
        let audio = directory.path().join(format!("audio.{audio_ext}"));
        let output = directory.path().join(format!("output.{output_ext}"));
        std::fs::write(directory.path().join("mode"), mode).unwrap();
        std::fs::write(&video, b"video").unwrap();
        std::fs::write(&audio, b"audio").unwrap();
        merge_with_deadline(
            fake(),
            &video,
            video_ext,
            &audio,
            audio_ext,
            &output,
            output_ext,
            callback,
            deadline,
        )?;
        Ok(std::fs::read(output).unwrap())
    }
    #[test]
    fn explicit_arguments_environment_inputs_and_output() {
        assert_eq!(
            run("success", Duration::from_secs(5), &mut || {
                ControlFlow::Continue(())
            })
            .unwrap(),
            b"merged"
        );
        assert_eq!(
            run("noisy-success", Duration::from_secs(5), &mut || {
                ControlFlow::Continue(())
            })
            .unwrap(),
            b"merged"
        );
    }
    #[test]
    fn hls_containers_use_fixed_local_demuxers_and_explicit_muxers() {
        for (video, audio, output) in [
            ("ts", "aac", "mp4"),
            ("ts", "m4a", "mp4"),
            ("ts", "ts", "mp4"),
            ("mp4", "aac", "mp4"),
            ("webm", "ts", "mkv"),
        ] {
            assert_eq!(
                run_containers(
                    &format!("success-{video}-{audio}"),
                    video,
                    audio,
                    output,
                    Duration::from_secs(5),
                    &mut || ControlFlow::Continue(())
                )
                .unwrap(),
                b"merged"
            );
        }
    }
    #[test]
    fn bounded_failures_join_pipes_and_reap_process() {
        for (mode, expected) in [
            ("stderr", "fixture diagnostic"),
            ("exit", "exited"),
            ("empty", "nonempty"),
            ("missing", "nonempty"),
            ("stdout-flood", "budget"),
            // A failure reports the last line of the retained stderr tail.
            ("stderr-flood", "flood tail"),
            ("hang", "deadline"),
            ("version-hang", "deadline"),
            ("bad-version", "banner"),
        ] {
            let started = Instant::now();
            let timeout = if mode.contains("hang") {
                Duration::from_millis(150)
            } else {
                Duration::from_secs(5)
            };
            let error = run(mode, timeout, &mut || ControlFlow::Continue(()))
                .unwrap_err()
                .to_string();
            assert!(error.contains(expected), "{mode}: {error}");
            assert!(started.elapsed() < Duration::from_secs(10), "{mode}");
        }
    }
    #[test]
    fn cancellation_during_version_and_merge() {
        fake();
        for mode in ["version-hang", "hang"] {
            let started = Instant::now();
            let result = run(mode, Duration::from_secs(5), &mut || {
                if started.elapsed() >= Duration::from_millis(150) {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            });
            assert!(matches!(result, Err(Error::DownloadCancelled)));
            assert!(started.elapsed() < Duration::from_secs(2));
        }
    }
    #[test]
    fn rejects_containers_before_process_launch() {
        assert!(matches!(
            merge(
                Path::new("absent"),
                Path::new("absent"),
                "hls",
                Path::new("absent"),
                "m4a",
                Path::new("absent"),
                "mp4",
                &mut || ControlFlow::Continue(())
            ),
            Err(Error::Postprocess(_))
        ));
    }
}

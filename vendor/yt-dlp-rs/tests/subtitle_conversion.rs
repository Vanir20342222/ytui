//! Local HTTP subtitle conversion and publication, using portable Rust media tools.
use std::{
    io::{Read, Write},
    net::TcpListener,
    ops::ControlFlow,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
use yt_dlp_rs::{DownloadPhase, Error, SubtitleConversion, YoutubeDl};

struct Server {
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Server {
    fn new() -> Self {
        Self::with_body(b"source video")
    }
    fn with_body(body: &'static [u8]) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (record, done) = (requests.clone(), stop.clone());
        let worker = thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                let (mut stream, _) = match listener.accept() {
                    Ok(value) => value,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(error) => panic!("{error}"),
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 2048];
                while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    let count = stream.read(&mut buffer).unwrap();
                    if count == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..count]);
                }
                let line = String::from_utf8(request)
                    .unwrap()
                    .lines()
                    .next()
                    .unwrap()
                    .to_owned();
                let path = line.split_whitespace().nth(1).unwrap();
                let body: &[u8] = match path {
                    "/ttml-page.html" => b"<video src='/media.mp4'><track kind='subtitles' srclang='en' src='/captions.ttml'></video>",
                    "/captions.ttml" => TTML,
                    "/caption-page.html" => b"<video src='/media.mp4'><track kind='subtitles' srclang='en' src='/captions.srt'></video>",
                    "/captions.srt" => SRT,
                    "/audio-page.html" => b"<video src='/master.m3u8' type='application/vnd.apple.mpegurl'></video>",
                    "/master.m3u8" => b"#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360,CODECS=\"avc1.42001e\"\nvideo.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=128000,CODECS=\"mp4a.40.2\"\naudio.m3u8\n",
                    "/video.m3u8" | "/audio.m3u8" => b"#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXTINF:1,\nsegment.ts\n#EXT-X-ENDLIST\n",
                    _ => body,
                };
                let mime = if path.ends_with(".srt") {
                    "text/plain"
                } else if path.ends_with(".html") {
                    "text/html"
                } else if path.ends_with(".m3u8") {
                    "application/vnd.apple.mpegurl"
                } else if path.ends_with(".mp3") {
                    "audio/mpeg"
                } else if path.ends_with(".m4a") {
                    "audio/mp4"
                } else {
                    "video/mp4"
                };
                record.lock().unwrap().push(line);
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(body);
            }
        });
        Self {
            url,
            requests,
            stop,
            worker: Some(worker),
        }
    }
    fn url(&self, name: &str) -> String {
        format!("{}/{name}", self.url)
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
    }
}
fn helper() -> &'static Path {
    static HELPER: OnceLock<PathBuf> = OnceLock::new();
    HELPER.get_or_init(|| {
        let directory = tempfile::tempdir().unwrap().keep();
        let binary = directory.join(format!("subtitle-pipeline{}", std::env::consts::EXE_SUFFIX));
        assert!(
            Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
                .arg(
                    Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("tests/helpers/subtitle_pipeline.rs")
                )
                .args(["--edition=2024", "-Dwarnings", "-o"])
                .arg(&binary)
                .status()
                .unwrap()
                .success()
        );
        binary
    })
}
struct Tools {
    _directory: tempfile::TempDir,
    probe: PathBuf,
    ffmpeg: PathBuf,
}
impl Tools {
    fn new(codec: &str, fail: bool) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let probe = directory
            .path()
            .join(format!("probe-{codec}{}", std::env::consts::EXE_SUFFIX));
        let ffmpeg = directory.path().join(format!(
            "ffmpeg-{}{}",
            if fail { "fail" } else { "ok" },
            std::env::consts::EXE_SUFFIX
        ));
        std::fs::copy(helper(), &probe).unwrap();
        std::fs::copy(helper(), &ffmpeg).unwrap();
        Self {
            _directory: directory,
            probe,
            ffmpeg,
        }
    }
    fn client(&self) -> YoutubeDl {
        YoutubeDl::new()
            .unwrap()
            .with_ffmpeg(&self.ffmpeg)
            .with_ffprobe(&self.probe)
    }
    fn log(&self, path: &Path) -> String {
        std::fs::read_to_string(path.with_extension("log")).unwrap_or_default()
    }
    fn cli(&self, args: &[&str], directory: &Path) -> Output {
        Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .current_dir(directory)
            .args(["--ffmpeg-location"])
            .arg(&self.ffmpeg)
            .arg("--ffprobe-location")
            .arg(&self.probe)
            .args(args)
            .output()
            .unwrap()
    }
}
fn success(output: &Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
fn clean(path: &Path) {
    assert!(!std::fs::read_dir(path).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".yt-dlp-rs-")
    }));
}

const SRT: &[u8] = b"1\n00:00:01,250 --> 00:00:03,500\nHello world\n";
const TTML: &[u8] = b"<tt xmlns='http://www.w3.org/ns/ttml'><body><div><p begin='1.25s' end='3.5s'>Hello world</p></div></body></tt>";
fn conversion(target: &str, keep: bool) -> SubtitleConversion {
    let mut options = SubtitleConversion::new(target).unwrap();
    options.keep_original = keep;
    options
}
#[test]
fn public_conversion_publishes_utf8_and_removes_or_retains_original() {
    for keep in [false, true] {
        let tools = Tools::new("aac", false);
        let client = tools.client();
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("captions.srt");
        std::fs::write(&source, SRT).unwrap();
        let result = client
            .convert_subtitle_with_options(&source, "srt", &conversion("vtt", keep))
            .unwrap();
        assert!(result.converted);
        assert_eq!(result.ext, "vtt");
        assert_eq!(result.path, directory.path().join("captions.vtt"));
        assert_eq!(std::fs::read(&result.path).unwrap(), b"converted subtitle");
        assert_eq!(result.bytes, 18);
        assert_eq!(source.exists(), keep);
        assert_eq!(result.original, keep.then_some(source));
        clean(directory.path());
    }
}
#[test]
fn json_same_extension_and_none_skip_without_tools() {
    for (ext, target) in [("json", "srt"), ("srt", "srt"), ("srt", "none")] {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join(format!("captions.{ext}"));
        std::fs::write(&source, SRT).unwrap();
        let client = YoutubeDl::new()
            .unwrap()
            .with_ffmpeg(directory.path().join("missing"));
        let result = client
            .convert_subtitle_with_options(&source, ext, &conversion(target, false))
            .unwrap();
        assert!(!result.converted);
        assert_eq!(result.path, source);
        assert_eq!(std::fs::read(result.path).unwrap(), SRT);
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}
#[test]
fn ttml_native_srt_requires_no_tools_and_chained_keep_retains_intermediate() {
    for target in ["srt", "vtt"] {
        let tools = Tools::new("aac", false);
        let client = tools.client();
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("captions.ttml");
        std::fs::write(&source, TTML).unwrap();
        let result = client
            .convert_subtitle_with_options(&source, "ttml", &conversion(target, true))
            .unwrap();
        assert!(result.converted);
        assert_eq!(std::fs::read(&source).unwrap(), TTML);
        let srt = std::fs::read_to_string(directory.path().join("captions.srt")).unwrap();
        assert!(srt.contains("Hello world"));
        assert!(srt.contains("00:00:01,250"));
        if target == "srt" {
            assert!(tools.log(&tools.ffmpeg).is_empty());
        } else {
            assert_eq!(std::fs::read(result.path).unwrap(), b"converted subtitle");
            assert_eq!(
                result.intermediates,
                vec![directory.path().join("captions.srt")]
            );
        }
        clean(directory.path());
    }
}
#[test]
fn failed_ffmpeg_preserves_original_and_completed_native_intermediate() {
    for ext in ["srt", "ttml"] {
        let tools = Tools::new("aac", true);
        let client = tools.client();
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join(format!("captions.{ext}"));
        let body = if ext == "srt" { SRT } else { TTML };
        std::fs::write(&source, body).unwrap();
        let result = client.convert_subtitle_with_options(&source, ext, &conversion("vtt", false));
        assert!(result.is_err());
        assert_eq!(std::fs::read(source).unwrap(), body);
        if ext == "ttml" {
            assert!(
                std::fs::read_to_string(directory.path().join("captions.srt"))
                    .unwrap()
                    .contains("Hello world")
            );
        }
        assert!(!directory.path().join("captions.vtt").exists());
        clean(directory.path());
    }
}
#[test]
fn occupied_destination_is_replaced_like_ffmpeg_y() {
    for race in [false, true] {
        let tools = Tools::new("aac", false);
        let client = tools.client();
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("captions.srt");
        let destination = directory.path().join("captions.vtt");
        std::fs::write(&source, SRT).unwrap();
        if !race {
            std::fs::write(&destination, b"competitor").unwrap();
        }
        client
            .convert_subtitle_with_progress(&source, "srt", &conversion("vtt", true), |progress| {
                if race && progress.phase == DownloadPhase::Publishing {
                    std::fs::write(&destination, b"competitor").unwrap();
                }
                ControlFlow::Continue(())
            })
            .unwrap();
        assert_eq!(std::fs::read(destination).unwrap(), b"converted subtitle");
        assert_eq!(std::fs::read(source).unwrap(), SRT);
        clean(directory.path());
    }
}
#[test]
fn conversion_cancellation_leaves_source_and_no_published_output() {
    for phase in [DownloadPhase::Postprocessing, DownloadPhase::Publishing] {
        let tools = Tools::new("aac", false);
        let client = tools.client();
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("captions.srt");
        std::fs::write(&source, SRT).unwrap();
        let result = client.convert_subtitle_with_progress(
            &source,
            "srt",
            &conversion("vtt", false),
            |progress| {
                if progress.phase == phase {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            },
        );
        assert!(matches!(result, Err(Error::DownloadCancelled)));
        assert_eq!(std::fs::read(source).unwrap(), SRT);
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}
#[cfg(unix)]
#[test]
fn symlink_input_and_output_are_refused_without_touching_files() {
    let tools = Tools::new("aac", false);
    let client = tools.client();
    let directory = tempfile::tempdir().unwrap();
    let real = directory.path().join("real.srt");
    std::fs::write(&real, SRT).unwrap();
    let source = directory.path().join("captions.srt");
    std::os::unix::fs::symlink(&real, &source).unwrap();
    assert!(
        client
            .convert_subtitle_with_options(&source, "srt", &conversion("vtt", false))
            .is_err()
    );
    std::fs::remove_file(&source).unwrap();
    std::fs::write(&source, SRT).unwrap();
    assert!(tools.log(&tools.ffmpeg).is_empty());
    // An output symlink is replaced, not followed.
    let output = directory.path().join("captions.vtt");
    let absent = directory.path().join("absent");
    std::os::unix::fs::symlink(&absent, &output).unwrap();
    client
        .convert_subtitle_with_options(&source, "srt", &conversion("vtt", true))
        .unwrap();
    assert!(!absent.exists());
    assert!(
        !std::fs::symlink_metadata(&output)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(std::fs::read(real).unwrap(), SRT);
}

#[test]
fn cli_conversion_requires_write_flag_and_readonly_modes_do_not_fetch_captions() {
    let server = Server::new();
    let tools = Tools::new("aac", false);
    for (mode, write) in [
        ("-j", true),
        ("-s", true),
        ("--list-subs", true),
        ("--skip-download", false),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let url = server.url("caption-page.html");
        let mut args = vec!["--convert-subs", "vtt", mode, "-o", "out.mp4"];
        if write {
            args.push("--write-subs");
        }
        args.push(&url);
        success(&tools.cli(&args, directory.path()));
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }
    assert!(tools.log(&tools.ffmpeg).is_empty());
    assert!(
        !server
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|request| request.contains("/captions.srt "))
    );
}
#[test]
fn cli_skip_media_converts_sidecar_with_aliases_and_keep_original() {
    let server = Server::new();
    let tools = Tools::new("aac", false);
    for (flag, keep) in [
        ("--convert-subs", false),
        ("--convert-sub", true),
        ("--convert-subtitles", false),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let url = server.url("caption-page.html");
        let mut args = vec![
            "--write-subs",
            flag,
            "vtt",
            "--skip-download",
            "--download-archive",
            "archive",
            "-o",
            "out.mp4",
        ];
        if keep {
            args.push("-k");
        }
        args.push(&url);
        let output = tools.cli(&args, directory.path());
        success(&output);
        // Upstream `FFmpegSubtitlesConvertorPP` with `run_pp` deletion.
        let stdout = String::from_utf8_lossy(&output.stdout);
        let expected = if keep {
            "[SubtitlesConvertor] Converting subtitles\n"
        } else {
            "[SubtitlesConvertor] Converting subtitles\nDeleting original file out.en.srt (pass -k to keep)\n"
        };
        assert!(stdout.ends_with(expected), "{stdout}");
        assert_eq!(
            std::fs::read(directory.path().join("out.en.vtt")).unwrap(),
            b"converted subtitle"
        );
        assert_eq!(directory.path().join("out.en.srt").exists(), keep);
        assert!(!directory.path().join("out.mp4").exists());
        assert!(!directory.path().join("archive").exists());
        clean(directory.path());
    }
}

#[test]
fn cli_failed_conversion_is_a_deferred_preprocessing_error() {
    let server = Server::new();
    let tools = Tools::new("aac", true);
    let directory = tempfile::tempdir().unwrap();
    let output = tools.cli(
        &[
            "--write-subs",
            "--convert-subs",
            "vtt",
            "--download-archive",
            "archive",
            "-o",
            "out.mp4",
            &server.url("caption-page.html"),
        ],
        directory.path(),
    );
    assert!(!output.status.success());
    assert_eq!(
        std::fs::read(directory.path().join("out.en.srt")).unwrap(),
        SRT
    );
    assert!(!directory.path().join("out.en.vtt").exists());
    // Upstream defers the error: the media downloads, then the video fails
    // before postprocessing (outside --abort-on-error it is still archived).
    let stderr = String::from_utf8_lossy(&output.stderr);
    let errors: Vec<_> = stderr
        .lines()
        .filter(|line| line.starts_with("ERROR: Preprocessing: "))
        .collect();
    assert_eq!(errors.len(), 2, "{stderr}");
    assert!(directory.path().join("out.mp4").exists());
    assert!(directory.path().join("archive").exists());
    let requests = server.requests.lock().unwrap();
    // The extraction probe and the download.
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.contains("/media.mp4 "))
            .count(),
        2
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.contains("/captions.srt "))
            .count(),
        1
    );
    clean(directory.path());
}

#[test]
fn conflicting_metadata_extension_cannot_convert_over_its_input() {
    let tools = Tools::new("aac", false);
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("captions.vtt");
    std::fs::write(&source, SRT).unwrap();
    assert!(
        tools
            .client()
            .convert_subtitle_with_options(&source, "srt", &conversion("vtt", false))
            .is_err()
    );
    assert_eq!(std::fs::read(&source).unwrap(), SRT);
    assert!(tools.log(&tools.ffmpeg).is_empty());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn cli_reserved_archive_and_cookie_names_reject_target_or_intermediate_before_caption_fetch() {
    let server = Server::new();
    let tools = Tools::new("aac", false);
    for (flag, page, name) in [
        ("--download-archive", "caption-page.html", "out.en.vtt"),
        ("--download-archive", "ttml-page.html", "out.en.srt"),
        ("--cookies", "caption-page.html", "out.en.vtt"),
        ("--cookies", "ttml-page.html", "out.en.srt"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        if flag == "--cookies" {
            std::fs::write(
                directory.path().join(name),
                b"# Netscape HTTP Cookie File\n",
            )
            .unwrap();
        }
        let output = tools.cli(
            &[
                "--write-subs",
                "--convert-subs",
                "vtt",
                "--skip-download",
                flag,
                name,
                "-o",
                "out.mp4",
                &server.url(page),
            ],
            directory.path(),
        );
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("collid"));
        assert!(!directory.path().join("out.mp4").exists());
        assert!(!directory.path().join("out.en.ttml").exists());
        if flag == "--download-archive" {
            assert!(!directory.path().join(name).exists());
        }
        clean(directory.path());
    }
    assert!(
        !server
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|request| request.contains("/captions."))
    );
    assert!(tools.log(&tools.ffmpeg).is_empty());
}

#[test]
fn native_ttml_with_no_valid_cue_times_publishes_empty_srt_without_ffmpeg() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("captions.ttml");
    std::fs::write(&source, b"<tt xmlns='http://www.w3.org/ns/ttml'><body><div><p begin='invalid' end='invalid'>Skipped</p></div></body></tt>").unwrap();
    let client = YoutubeDl::new()
        .unwrap()
        .with_ffmpeg(directory.path().join("missing"));
    let result = client
        .convert_subtitle_with_options(&source, "ttml", &conversion("srt", false))
        .unwrap();
    assert!(result.converted);
    assert_eq!(result.bytes, 0);
    assert_eq!(std::fs::read(result.path).unwrap(), b"");
    assert!(!source.exists());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn source_replaced_at_publication_is_preserved_and_conversion_is_not_published() {
    let tools = Tools::new("aac", false);
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("captions.srt");
    std::fs::write(&source, SRT).unwrap();
    let result = tools.client().convert_subtitle_with_progress(
        &source,
        "srt",
        &conversion("vtt", false),
        |progress| {
            if progress.phase == DownloadPhase::Publishing {
                std::fs::write(&source, b"replacement source with different length").unwrap();
            }
            ControlFlow::Continue(())
        },
    );
    assert!(result.unwrap_err().to_string().contains("source changed"));
    assert_eq!(
        std::fs::read(&source).unwrap(),
        b"replacement source with different length"
    );
    assert!(!directory.path().join("captions.vtt").exists());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}

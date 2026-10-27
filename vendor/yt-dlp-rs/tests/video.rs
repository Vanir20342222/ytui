//! Local HTTP video processing and publication, using portable Rust media tools.
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
use yt_dlp_rs::{
    AudioOptions, DownloadOptions, DownloadPhase, Error, PostprocessOptions, VideoMode,
    VideoOptions, YoutubeDl,
};

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
                    "/audio-page.html" => b"<video src='/master.m3u8' type='application/vnd.apple.mpegurl'></video>",
                    "/master.m3u8" => b"#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360,CODECS=\"avc1.42001e\"\nvideo.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=128000,CODECS=\"mp4a.40.2\"\naudio.m3u8\n",
                    "/video.m3u8" | "/audio.m3u8" => b"#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXTINF:1,\nsegment.ts\n#EXT-X-ENDLIST\n",
                    _ => body,
                };
                let mime = if path.ends_with(".html") {
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
        let binary = directory.join(format!("video-pipeline{}", std::env::consts::EXE_SUFFIX));
        assert!(
            Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
                .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/helpers/video_pipeline.rs"))
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

fn options(format: &str, mode: VideoMode, keep: bool) -> PostprocessOptions {
    PostprocessOptions {
        existing: None,
        no_post_overwrites: false,
        modify_chapters: None,
        ignore_errors: false,
        thumbnail: None,
        metadata: false,
        chapters: false,
        infojson: None,
        fixup: None,
        audio: None,
        video: Some(VideoOptions::new(format, mode).unwrap()),
        keep_video: keep,
        embed: None,
        ..Default::default()
    }
}

#[test]
fn remux_and_recode_cli_use_distinct_codecs_and_publish_selected_container() {
    let server = Server::new();
    for (flag, format, extension) in [
        ("--remux-video", "mkv", "mkv"),
        ("--recode-video", "avi", "avi"),
        ("--recode-video", "webm", "webm"),
    ] {
        let tools = Tools::new("aac", false);
        let directory = tempfile::tempdir().unwrap();
        let output = tools.cli(
            &[flag, format, "-o", "out.mp4", &server.url("source.mp4")],
            directory.path(),
        );
        success(&output);
        assert_eq!(
            std::fs::read(directory.path().join(format!("out.{extension}"))).unwrap(),
            b"converted video"
        );
        assert!(!directory.path().join("out.mp4").exists());
        // Upstream FFmpegVideoRemuxerPP/ConvertorPP and run_pp messages.
        let (name, action) = if flag == "--remux-video" {
            ("VideoRemuxer", "Remuxing")
        } else {
            ("VideoConvertor", "Converting")
        };
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains(&format!(
                "[{name}] {action} video from mp4 to {extension}; Destination: out.{extension}\nDeleting original file out.mp4 (pass -k to keep)\n"
            )),
            "{stdout}"
        );
        let log = tools.log(&tools.ffmpeg);
        if flag == "--remux-video" {
            assert!(log.contains("copy"), "{log}");
        } else {
            assert!(!log.lines().any(|arg| arg == "copy"), "{log}");
        }
        if format == "avi" {
            assert!(log.contains("-c:v\nlibxvid\n-vtag\nXVID"), "{log}");
        }
        assert!(log.contains("-map\n0"), "{log}");
        clean(directory.path());
    }
}

#[test]
fn mappings_and_same_extension_skip_without_any_tools() {
    let server = Server::new();
    for format in ["mp4", "webm>mkv"] {
        let directory = tempfile::tempdir().unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .current_dir(directory.path())
            .args([
                "--remux-video",
                format,
                "--ffmpeg-location",
                "missing",
                "-o",
                "out.mp4",
                &server.url("source.mp4"),
            ])
            .output()
            .unwrap();
        success(&output);
        assert_eq!(
            std::fs::read(directory.path().join("out.mp4")).unwrap(),
            b"source video"
        );
        let reason = if format == "mp4" {
            "already is in target format mp4"
        } else {
            "could not find a mapping for mp4"
        };
        assert!(String::from_utf8_lossy(&output.stdout).contains(&format!(
            "[VideoRemuxer] Not remuxing media file \"out.mp4\"; {reason}\n"
        )));
        clean(directory.path());
    }
}

#[test]
fn readonly_modes_are_lazy_and_invalid_options_preflight() {
    let server = Server::new();
    let tools = Tools::new("aac", false);
    for mode in ["-s", "-j", "-F", "--skip-download"] {
        let directory = tempfile::tempdir().unwrap();
        success(&tools.cli(
            &[
                "--remux-video",
                "mkv",
                mode,
                "-o",
                "out.mp4",
                &server.url("source.mp4"),
            ],
            directory.path(),
        ));
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }
    assert!(tools.log(&tools.ffmpeg).is_empty());
    assert!(tools.log(&tools.probe).is_empty());
    let before = server.requests.lock().unwrap().len();
    for args in [
        vec!["--remux-video", "mkv/bogus"],
        vec!["--recode-video", "mp4>bogus"],
    ] {
        let directory = tempfile::tempdir().unwrap();
        let url = server.url("source.mp4");
        let mut args = args;
        args.push(&url);
        let output = tools.cli(&args, directory.path());
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
    assert_eq!(server.requests.lock().unwrap().len(), before);
}

#[test]
fn keep_source_and_last_no_keep_flag_control_publication() {
    let server = Server::new();
    let tools = Tools::new("aac", false);
    for keep in [true, false] {
        let directory = tempfile::tempdir().unwrap();
        let mut args = vec!["--remux-video", "mkv", "--keep-video", "-o", "out.mp4"];
        if !keep {
            args.push("--no-keep-video");
        }
        let url = server.url("source.mp4");
        args.push(&url);
        success(&tools.cli(&args, directory.path()));
        assert_eq!(
            std::fs::read(directory.path().join("out.mkv")).unwrap(),
            b"converted video"
        );
        assert_eq!(directory.path().join("out.mp4").exists(), keep);
        if keep {
            assert_eq!(
                std::fs::read(directory.path().join("out.mp4")).unwrap(),
                b"source video"
            );
        }
        clean(directory.path());
    }
}

#[test]
fn failed_conversion_preserves_sources_continues_and_never_records() {
    let server = Server::new();
    let tools = Tools::new("aac", true);
    let directory = tempfile::tempdir().unwrap();
    let output = tools.cli(
        &[
            "--recode-video",
            "mkv",
            "--download-archive",
            "archive",
            "-o",
            "%(id)s.%(ext)s",
            &server.url("one.mp4"),
            &server.url("two.mp4"),
        ],
        directory.path(),
    );
    assert!(!output.status.success());
    for id in ["one", "two"] {
        assert_eq!(
            std::fs::read(directory.path().join(format!("{id}.mp4"))).unwrap(),
            b"source video"
        );
        assert!(!directory.path().join(format!("{id}.mkv")).exists());
    }
    assert!(!directory.path().join("archive").exists());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr)
            .matches("ERROR:")
            .count(),
        2
    );
    clean(directory.path());
}

#[test]
fn existing_and_racing_outputs_are_replaced_like_upstream() {
    let server = Server::new();
    let tools = Tools::new("aac", false);
    let client = tools.client();
    let info = client.extract_info(&server.url("source.mp4")).unwrap();
    for race in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let final_path = directory.path().join("out.mkv");
        if !race {
            std::fs::write(&final_path, b"competitor").unwrap();
        }
        let result = client.download_processed_with_progress(
            &info,
            directory.path().join("out.mp4"),
            DownloadOptions::default(),
            &options("mkv", VideoMode::Remux, false),
            |progress| {
                if race && progress.phase == DownloadPhase::Publishing {
                    std::fs::write(&final_path, b"competitor").unwrap();
                }
                ControlFlow::Continue(())
            },
        );
        // Upstream processors overwrite their outputs (`ffmpeg -y`).
        let result = result.unwrap();
        assert_eq!(result.path, final_path);
        assert_ne!(std::fs::read(&final_path).unwrap(), b"competitor");
        assert!(!directory.path().join("out.mp4").exists());
        clean(directory.path());
    }
}

#[test]
fn cancellation_never_publishes_source_or_converted_output() {
    let server = Server::new();
    for phase in [DownloadPhase::Postprocessing, DownloadPhase::Publishing] {
        let tools = Tools::new("aac", false);
        let client = tools.client();
        let info = client.extract_info(&server.url("source.mp4")).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let result = client.download_processed_with_progress(
            &info,
            directory.path().join("out.mp4"),
            DownloadOptions::default(),
            &options("mkv", VideoMode::Remux, true),
            |progress| {
                if progress.phase == phase {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            },
        );
        assert!(matches!(result, Err(Error::DownloadCancelled)));
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }
}

#[test]
fn audio_then_video_uses_updated_extension_mapping_and_keeps_original_source() {
    let server = Server::new();
    let tools = Tools::new("aac", false);
    let client = tools.client();
    let info = client.extract_info(&server.url("source.mp4")).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let mut plan = options("mp3>mkv", VideoMode::Remux, true);
    plan.audio = Some(AudioOptions::new("mp3", "5").unwrap());
    let result = client
        .download_processed_with_options(
            &info,
            directory.path().join("out.mp4"),
            DownloadOptions::default(),
            &plan,
        )
        .unwrap();
    assert_eq!(result.path, directory.path().join("out.mkv"));
    assert_eq!(
        result.kept_files,
        vec![
            directory.path().join("out.mp4"),
            directory.path().join("out.mp3")
        ]
    );
    assert_eq!(std::fs::read(result.path).unwrap(), b"converted video");
    assert_eq!(
        std::fs::read(result.kept_video.unwrap()).unwrap(),
        b"source video"
    );
    assert_eq!(
        std::fs::read(directory.path().join("out.mp3")).unwrap(),
        b"converted audio"
    );
    let log = tools.log(&tools.ffmpeg);
    assert_eq!(log.lines().filter(|arg| *arg == "-i").count(), 2);
    clean(directory.path());
}

#[test]
fn selected_pair_merges_before_video_processing() {
    let server = Server::new();
    let tools = Tools::new("aac", false);
    let client = tools.client();
    let info: yt_dlp_rs::VideoInfo = serde_json::from_value(serde_json::json!({"id":"pair", "title":"pair", "ext":"mp4", "url":server.url("video.mp4"), "webpage_url":server.url("page"), "extractor":"fixture", "extractor_key":"Fixture", "direct":false, "requested_formats":[{"format_id":"v", "url":server.url("video.mp4"), "ext":"mp4", "stream_kind":"video_only"},{"format_id":"a", "url":server.url("audio.m4a"), "ext":"m4a", "stream_kind":"audio_only"}]})).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let mut phases = Vec::new();
    let result = client
        .download_processed_with_progress(
            &info,
            directory.path().join("out.mp4"),
            DownloadOptions::default(),
            &options("mkv", VideoMode::Remux, false),
            |progress| {
                phases.push(progress.phase);
                ControlFlow::Continue(())
            },
        )
        .unwrap();
    assert_eq!(std::fs::read(result.path).unwrap(), b"converted video");
    assert!(
        phases
            .iter()
            .position(|phase| *phase == DownloadPhase::Merging)
            .unwrap()
            < phases
                .iter()
                .position(|phase| *phase == DownloadPhase::Postprocessing)
                .unwrap()
    );
    assert_eq!(
        phases
            .iter()
            .filter(|phase| **phase == DownloadPhase::Publishing)
            .count(),
        1
    );
    assert_eq!(
        tools
            .log(&tools.ffmpeg)
            .lines()
            .filter(|arg| *arg == "-i")
            .count(),
        3
    );
    clean(directory.path());
}

#[test]
fn archive_is_written_after_success_and_cannot_alias_final_output() {
    let server = Server::new();
    let tools = Tools::new("aac", false);
    for archive in ["archive", "out.mkv"] {
        let directory = tempfile::tempdir().unwrap();
        let output = tools.cli(
            &[
                "--remux-video",
                "mkv",
                "--download-archive",
                archive,
                "-o",
                "out.mp4",
                &server.url("source.mp4"),
            ],
            directory.path(),
        );
        assert_eq!(
            output.status.success(),
            archive == "archive",
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            std::fs::read(directory.path().join("out.mkv")).unwrap(),
            b"converted video"
        );
        assert!(!directory.path().join("out.mp4").exists());
        clean(directory.path());
        if archive == "archive" {
            assert!(
                !std::fs::read_to_string(directory.path().join(archive))
                    .unwrap()
                    .is_empty()
            );
        }
    }
}

#[test]
fn recode_wins_over_remux_in_both_argument_orders_with_warning() {
    let server = Server::new();
    let tools = Tools::new("aac", false);
    for flags in [
        ["--remux-video", "mkv", "--recode-video", "avi"],
        ["--recode-video", "avi", "--remux-video", "mkv"],
    ] {
        let directory = tempfile::tempdir().unwrap();
        let url = server.url("source.mp4");
        let mut args = flags.to_vec();
        args.extend(["-o", "out.mp4", &url]);
        let output = tools.cli(&args, directory.path());
        success(&output);
        assert_eq!(
            std::fs::read(directory.path().join("out.avi")).unwrap(),
            b"converted video"
        );
        assert!(!directory.path().join("out.mkv").exists());
        assert!(String::from_utf8_lossy(&output.stderr).contains("WARNING"));
        clean(directory.path());
    }
}

#[test]
fn optional_real_tools_remux_and_recode_saved_video() {
    let Some(ffmpeg) = std::env::var_os("YT_DLP_RS_TEST_FFMPEG") else {
        return;
    };
    let Some(ffprobe) = std::env::var_os("YT_DLP_RS_TEST_FFPROBE") else {
        return;
    };
    let server = Server::with_body(include_bytes!("fixtures/merge/video.mp4"));
    let client = YoutubeDl::new().unwrap().with_ffmpeg(&ffmpeg);
    let info = client.extract_info(&server.url("video.mp4")).unwrap();
    for (format, mode, codec) in [
        ("mkv", VideoMode::Remux, "h264"),
        ("avi", VideoMode::Recode, "mpeg4"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let result = client
            .download_processed_with_options(
                &info,
                directory.path().join("out.mp4"),
                DownloadOptions::default(),
                &options(format, mode, false),
            )
            .unwrap();
        let output = Command::new(&ffprobe)
            .args([
                "-v",
                "error",
                "-show_entries",
                "stream=codec_type,codec_name,width,height",
                "-of",
                "json",
            ])
            .arg(result.path)
            .output()
            .unwrap();
        success(&output);
        let probe: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let video = probe["streams"]
            .as_array()
            .unwrap()
            .iter()
            .find(|stream| stream["codec_type"] == "video")
            .unwrap();
        assert_eq!(video["codec_name"], codec);
        assert!(video["width"].as_u64().unwrap() > 0);
        assert!(video["height"].as_u64().unwrap() > 0);
        clean(directory.path());
    }
}

#[test]
fn second_stage_failure_preserves_latest_audio_and_optional_original() {
    let server = Server::new();
    for keep in [false, true] {
        let tools = Tools::new("aac", true);
        let client = tools.client();
        let info = client.extract_info(&server.url("source.mp4")).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let mut plan = options("mp3>mkv", VideoMode::Remux, keep);
        plan.audio = Some(AudioOptions::new("mp3", "5").unwrap());
        let result = client.download_processed_with_options(
            &info,
            directory.path().join("out.mp4"),
            DownloadOptions::default(),
            &plan,
        );
        assert!(result.is_err());
        assert_eq!(
            std::fs::read(directory.path().join("out.mp3")).unwrap(),
            b"converted audio"
        );
        assert_eq!(directory.path().join("out.mp4").exists(), keep);
        if keep {
            assert_eq!(
                std::fs::read(directory.path().join("out.mp4")).unwrap(),
                b"source video"
            );
        }
        assert!(!directory.path().join("out.mkv").exists());
        clean(directory.path());
    }
}

#[test]
fn archive_cannot_append_to_retained_audio_intermediate() {
    let server = Server::new();
    let tools = Tools::new("aac", false);
    let directory = tempfile::tempdir().unwrap();
    let output = tools.cli(
        &[
            "-x",
            "--audio-format",
            "mp3",
            "--remux-video",
            "mka",
            "-k",
            "--download-archive",
            "out.mp3",
            "-o",
            "out.mp4",
            &server.url("source.mp4"),
        ],
        directory.path(),
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("archive"));
    for (name, bytes) in [
        ("out.mp4", b"source video".as_slice()),
        ("out.mp3", b"converted audio".as_slice()),
        ("out.mka", b"converted video".as_slice()),
    ] {
        assert_eq!(std::fs::read(directory.path().join(name)).unwrap(), bytes);
    }
    clean(directory.path());
}

#[test]
fn uppercase_source_extension_preserves_literal_suffix_during_container_change() {
    let server = Server::new();
    let tools = Tools::new("aac", false);
    let client = tools.client();
    let mut info = client.extract_info(&server.url("source.mp4")).unwrap();
    info.ext = "MP4".into();
    let directory = tempfile::tempdir().unwrap();
    let result = client
        .download_processed_with_options(
            &info,
            directory.path().join("out.MP4"),
            DownloadOptions::default(),
            &options("mkv", VideoMode::Remux, false),
        )
        .unwrap();
    assert_eq!(result.path, directory.path().join("out.MP4.mkv"));
    assert_eq!(std::fs::read(&result.path).unwrap(), b"converted video");
    assert!(!directory.path().join("out.MP4").exists());
    assert!(
        tools
            .log(&tools.ffmpeg)
            .contains("-f\nmov\n-protocol_whitelist")
    );
    clean(directory.path());
}

#[test]
fn chain_returning_to_original_filename_keeps_source_under_orig_name() {
    let server = Server::new();
    let tools = Tools::new("aac", false);
    let client = tools.client();
    let info = client.extract_info(&server.url("source.mp4")).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let mut plan = options("mp4", VideoMode::Remux, true);
    plan.audio = Some(AudioOptions::new("m4a", "5").unwrap());
    let result = client
        .download_processed_with_options(
            &info,
            directory.path().join("out.mp4"),
            DownloadOptions::default(),
            &plan,
        )
        .unwrap();
    assert_eq!(result.path, directory.path().join("out.mp4"));
    assert_eq!(
        result.kept_video,
        Some(directory.path().join("out.orig.mp4"))
    );
    assert_eq!(
        result.kept_files,
        vec![
            directory.path().join("out.orig.mp4"),
            directory.path().join("out.m4a")
        ]
    );
    for (name, bytes) in [
        ("out.orig.mp4", b"source video".as_slice()),
        ("out.m4a", b"converted audio".as_slice()),
        ("out.mp4", b"converted video".as_slice()),
    ] {
        assert_eq!(std::fs::read(directory.path().join(name)).unwrap(), bytes);
    }
    clean(directory.path());
}

#[test]
fn kept_original_replaces_a_stale_orig_and_completed_audio_survives_later_failure() {
    let server = Server::new();
    let tools = Tools::new("aac", false);
    let client = tools.client();
    let info = client.extract_info(&server.url("source.mp3")).unwrap();
    assert_eq!(info.ext, "mp3");
    let directory = tempfile::tempdir().unwrap();
    let competitor = directory.path().join("movie.orig.mp3");
    std::fs::write(&competitor, b"competitor").unwrap();
    let mut plan = options("alac", VideoMode::Remux, true);
    plan.audio = Some(AudioOptions::new("mp3", "5").unwrap());
    let result = client.download_processed_with_options(
        &info,
        directory.path().join("movie.mp3"),
        DownloadOptions::default(),
        &plan,
    );
    assert!(result.is_err());
    // The kept original replaces the stale `.orig` file (`os.replace`).
    assert_eq!(std::fs::read(competitor).unwrap(), b"source video");
    assert_eq!(
        std::fs::read(directory.path().join("movie.mp3")).unwrap(),
        b"converted audio"
    );
    assert!(!directory.path().join("movie.alac").exists());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 2);
    clean(directory.path());
}

#[test]
fn embed_metadata_rewrites_the_final_file_with_upstream_metadata_arguments() {
    let server = Server::new();
    let tools = Tools::new("aac", false);
    let directory = tempfile::tempdir().unwrap();
    let url = server.url("source.mp4");
    let output = tools.cli(
        &["--embed-metadata", "-o", "out.mp4", &url],
        directory.path(),
    );
    success(&output);
    assert_eq!(
        std::fs::read(directory.path().join("out.mp4")).unwrap(),
        b"converted video"
    );
    let log = tools.log(&tools.ffmpeg);
    assert!(
        log.contains(&format!(
            "-map\n0\n-dn\n-ignore_unknown\n-c\ncopy\n-write_id3v1\n1\n-metadata\ntitle=source\n-metadata\npurl={url}\n-metadata\ncomment={url}\n-movflags\n+faststart\n-f\nmp4"
        )),
        "{log}"
    );
    clean(directory.path());
    // --no-add-metadata (last flag) skips the stage and needs no tools.
    let plain = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .current_dir(plain.path())
        .args(["--add-metadata", "--no-add-metadata", "-o", "out.mp4", &url])
        .output()
        .unwrap();
    success(&output);
    assert_eq!(
        std::fs::read(plain.path().join("out.mp4")).unwrap(),
        b"source video"
    );
}

#[test]
fn ignore_errors_skips_failed_processors_and_abort_on_error_stops_the_queue() {
    let server = Server::new();
    let tools = Tools::new("aac", true);
    let directory = tempfile::tempdir().unwrap();
    let url = server.url("source.mp4");
    // -i: the failed remux is reported, the download still counts.
    let output = tools.cli(
        &[
            "-i",
            "--remux-video",
            "mkv",
            "--download-archive",
            "archive.txt",
            "-o",
            "out.%(ext)s",
            &url,
        ],
        directory.path(),
    );
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("ERROR: "), "{stderr}");
    assert_eq!(
        std::fs::read(directory.path().join("out.mp4")).unwrap(),
        b"source video"
    );
    assert!(!directory.path().join("out.mkv").exists());
    assert_eq!(
        std::fs::read_to_string(directory.path().join("archive.txt")).unwrap(),
        "generic source\n"
    );
    clean(directory.path());
    // Without -i (or after a later --abort-on-error) nothing is archived.
    let strict = tempfile::tempdir().unwrap();
    let output = tools.cli(
        &[
            "-i",
            "--abort-on-error",
            "--remux-video",
            "mkv",
            "--download-archive",
            "archive.txt",
            "-o",
            "out.%(ext)s",
            &url,
        ],
        strict.path(),
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(!strict.path().join("archive.txt").exists());
    // --abort-on-error stops at the first failing input; the default continues.
    let unreachable = "http://127.0.0.1:9/missing.mp4";
    for (args, downloads) in [
        (vec!["--abort-on-error"], false),
        (vec!["--no-ignore-errors", "--no-abort-on-error"], true),
        (vec![], true),
    ] {
        let queue = tempfile::tempdir().unwrap();
        let mut all = args.clone();
        all.extend(["--proxy", "", "-o", "next.%(ext)s", unreachable, &url]);
        let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .current_dir(queue.path())
            .args(&all)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{args:?}");
        assert_eq!(
            queue.path().join("next.mp4").exists(),
            downloads,
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn ffmpeg_outputs_inherit_the_downloaded_modification_time() {
    let server = Server::new();
    let tools = Tools::new("aac", false);
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.mp4");
    let info = tools
        .client()
        .extract_info(&server.url("source.mp4"))
        .unwrap();
    let result = tools
        .client()
        .download_processed_with_options(
            &info,
            &source,
            DownloadOptions::default(),
            &options("mkv", VideoMode::Remux, true),
        )
        .unwrap();
    let kept = result.kept_video.unwrap();
    let times = |path: &Path| std::fs::metadata(path).unwrap().modified().unwrap();
    // Like upstream `run_ffmpeg_multiple_files`, the remuxed output takes
    // the oldest input modification time rather than its creation time.
    assert_eq!(times(&result.path), times(&kept));
}

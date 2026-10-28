//! Local HTTP audio extraction and publication, using portable Rust media tools.
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
use yt_dlp_rs::{AudioOptions, DownloadOptions, DownloadPhase, Error, YoutubeDl};

struct Server {
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Server {
    fn new() -> Self {
        Self::with_body(b"source audio")
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
        let binary = directory.join(format!("audio-pipeline{}", std::env::consts::EXE_SUFFIX));
        assert!(
            Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
                .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/helpers/audio_pipeline.rs"))
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

#[test]
fn remux_aac_and_encode_targets_publish_bytes_and_pass_quality() {
    let server = Server::new();
    for (target, codec, ext, encoder, muxer, quality, quality_args) in [
        (
            "m4a",
            "aac",
            "m4a",
            "copy",
            "ipod",
            "5",
            "-bsf:a\naac_adtstoasc",
        ),
        (
            "mp3",
            "aac",
            "mp3",
            "libmp3lame",
            "mp3",
            "192",
            "-b:a\n192.0k",
        ),
        ("wav", "aac", "wav", "", "wav", "5", "-vn"),
    ] {
        let tools = Tools::new(codec, false);
        let directory = tempfile::tempdir().unwrap();
        let client = tools.client();
        let info = client.extract_info(&server.url("source.mp4")).unwrap();
        let destination = directory.path().join("out.mp4");
        let result = client
            .download_audio_with_options(
                &info,
                &destination,
                DownloadOptions::default(),
                &AudioOptions::new(target, quality).unwrap(),
            )
            .unwrap();
        assert_eq!(result.path, directory.path().join(format!("out.{ext}")));
        assert_eq!(std::fs::read(&result.path).unwrap(), b"converted audio");
        assert_eq!(result.bytes, 15);
        assert_eq!(result.kept_video, None);
        assert!(!destination.exists());
        let args = tools.log(&tools.ffmpeg);
        if !encoder.is_empty() {
            assert!(args.contains(&format!("-acodec\n{encoder}")), "{args}");
        }
        assert!(args.contains(&format!("-f\n{muxer}\n")), "{args}");
        assert!(args.contains(quality_args), "{args}");
        clean(directory.path());
    }
}

#[test]
fn keep_video_publishes_source_and_same_extension_uses_orig() {
    let server = Server::new();
    for (source, target, original, final_name) in [
        ("source.mp4", "mp3", "out.mp4", "out.mp3"),
        ("source.m4a", "alac", "out.orig.m4a", "out.m4a"),
    ] {
        let tools = Tools::new("aac", false);
        let client = tools.client();
        let info = client.extract_info(&server.url(source)).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join(if source.ends_with("m4a") {
            "out.m4a"
        } else {
            "out.mp4"
        });
        let mut options = AudioOptions::new(target, "5").unwrap();
        options.keep_video = true;
        let result = client
            .download_audio_with_options(&info, destination, DownloadOptions::default(), &options)
            .unwrap();
        assert_eq!(result.path, directory.path().join(final_name));
        assert_eq!(result.kept_video, Some(directory.path().join(original)));
        assert_eq!(
            std::fs::read(directory.path().join(original)).unwrap(),
            b"source audio"
        );
        assert_eq!(std::fs::read(result.path).unwrap(), b"converted audio");
        clean(directory.path());
    }
}

#[test]
fn common_best_unmatched_mapping_and_same_codec_copy_are_lazy() {
    let server = Server::new();
    for (name, format) in [("source.mp3", "best"), ("source.mp4", "webm>mp3")] {
        let directory = tempfile::tempdir().unwrap();
        let client = YoutubeDl::new()
            .unwrap()
            .with_ffmpeg(directory.path().join("missing-ffmpeg"))
            .with_ffprobe(directory.path().join("missing-probe"));
        let info = client.extract_info(&server.url(name)).unwrap();
        let destination = directory.path().join(name);
        let result = client
            .download_audio_with_options(
                &info,
                &destination,
                DownloadOptions::default(),
                &AudioOptions::new(format, "5").unwrap(),
            )
            .unwrap();
        assert_eq!(result.path, destination);
        assert_eq!(std::fs::read(result.path).unwrap(), b"source audio");
        clean(directory.path());
    }
    let tools = Tools::new("aac", false);
    let client = tools.client();
    let info = client.extract_info(&server.url("source.m4a")).unwrap();
    let directory = tempfile::tempdir().unwrap();
    client
        .download_audio_with_options(
            &info,
            directory.path().join("out.m4a"),
            DownloadOptions::default(),
            &AudioOptions::new("m4a", "5").unwrap(),
        )
        .unwrap();
    assert!(!tools.log(&tools.probe).is_empty());
    assert!(tools.log(&tools.ffmpeg).is_empty());
}

#[test]
fn simulation_listing_json_and_skip_download_never_start_tools() {
    let server = Server::new();
    let tools = Tools::new("aac", false);
    for mode in ["-s", "-j", "-F", "--skip-download"] {
        let directory = tempfile::tempdir().unwrap();
        let output = tools.cli(
            &[
                "-x",
                "--audio-format",
                "mp3",
                mode,
                "-o",
                "out.mp4",
                &server.url("source.mp4"),
            ],
            directory.path(),
        );
        success(&output);
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }
    assert!(tools.log(&tools.probe).is_empty());
    assert!(tools.log(&tools.ffmpeg).is_empty());
    assert_eq!(server.requests.lock().unwrap().len(), 4);
}

#[test]
fn invalid_audio_options_fail_before_http_or_files() {
    let server = Server::new();
    let tools = Tools::new("aac", false);
    for (flag, value) in [
        ("--audio-format", "flac/bogus"),
        ("--audio-quality", "NaN"),
        ("--audio-quality", "-1"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let output = tools.cli(
            &["-x", flag, value, &server.url("source.mp4")],
            directory.path(),
        );
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }
    assert!(server.requests.lock().unwrap().is_empty());
    // Like upstream, -k alone is accepted and changes nothing for one stream.
    let directory = tempfile::tempdir().unwrap();
    let output = tools.cli(
        &["-k", "-o", "out.%(ext)s", &server.url("source.mp4")],
        directory.path(),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn process_failure_preserves_sources_continues_urls_and_does_not_record_archive() {
    let server = Server::new();
    let tools = Tools::new("aac", true);
    let directory = tempfile::tempdir().unwrap();
    let output = tools.cli(
        &[
            "-x",
            "--audio-format",
            "mp3",
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
            b"source audio"
        );
        assert!(!directory.path().join(format!("{id}.mp3")).exists());
    }
    assert!(!directory.path().join("archive").exists());
    clean(directory.path());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr)
            .matches("ERROR:")
            .count(),
        2
    );
}

#[test]
fn existing_final_extension_is_processed_again_without_downloading() {
    let server = Server::new();
    let tools = Tools::new("aac", false);
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("out.mp3"), b"existing").unwrap();
    let output = tools.cli(
        &[
            "-x",
            "--audio-format",
            "mp3",
            "--download-archive",
            "archive",
            "-o",
            "out.mp4",
            &server.url("source.mp4"),
        ],
        directory.path(),
    );
    // Upstream `existing_video_file` finds the `final_ext` file and runs
    // the processors on it instead of downloading.
    success(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.ends_with(
        "[download] out.mp3 has already been downloaded\n[ExtractAudio] Destination: out.mp3\nDeleting original file out.orig.mp3 (pass -k to keep)\n"
    ));
    // The (AAC-probed) existing file is converted in place.
    assert_eq!(
        std::fs::read(directory.path().join("out.mp3")).unwrap(),
        b"converted audio"
    );
    assert!(!directory.path().join("out.mp4").exists());
    assert!(!directory.path().join("out.orig.mp3").exists());
    assert!(
        std::fs::read_to_string(directory.path().join("archive"))
            .unwrap()
            .contains("source")
    );
    clean(directory.path());
}

#[test]
fn cancellation_during_processing_or_publication_removes_private_source_and_conversion() {
    let server = Server::new();
    for phase in [DownloadPhase::Postprocessing, DownloadPhase::Publishing] {
        let tools = Tools::new("aac", false);
        let client = tools.client();
        let info = client.extract_info(&server.url("source.mp4")).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let result = client.download_audio_with_progress(
            &info,
            directory.path().join("out.mp4"),
            DownloadOptions::default(),
            &AudioOptions::new("mp3", "5").unwrap(),
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
        if phase == DownloadPhase::Publishing {
            assert!(!tools.log(&tools.ffmpeg).is_empty());
        }
    }
}

#[test]
fn successful_audio_records_archive_only_after_converted_output_is_published() {
    let server = Server::new();
    let tools = Tools::new("aac", false);
    let directory = tempfile::tempdir().unwrap();
    let args = [
        "-x",
        "--audio-format",
        "mp3",
        "--download-archive",
        "archive",
        "-o",
        "out.mp4",
    ];
    let mut command = args.to_vec();
    let url = server.url("source.mp4");
    command.push(&url);
    success(&tools.cli(&command, directory.path()));
    assert_eq!(
        std::fs::read(directory.path().join("out.mp3")).unwrap(),
        b"converted audio"
    );
    assert!(!directory.path().join("out.mp4").exists());
    assert!(
        !std::fs::read_to_string(directory.path().join("archive"))
            .unwrap()
            .is_empty()
    );
    let before = server.requests.lock().unwrap().len();
    success(&tools.cli(&command, directory.path()));
    assert_eq!(server.requests.lock().unwrap().len(), before + 1);
}

#[test]
fn archive_cannot_append_to_published_audio_or_kept_source() {
    let server = Server::new();
    let tools = Tools::new("aac", false);
    for (keep, archive) in [(false, "out.mp3"), (true, "out.mp4")] {
        let directory = tempfile::tempdir().unwrap();
        let mut args = vec![
            "-x",
            "--audio-format",
            "mp3",
            "--download-archive",
            archive,
            "-o",
            "out.mp4",
        ];
        if keep {
            args.push("--keep-video");
        }
        let url = server.url("source.mp4");
        args.push(&url);
        let output = tools.cli(&args, directory.path());
        assert!(!output.status.success());
        assert_eq!(
            std::fs::read(directory.path().join("out.mp3")).unwrap(),
            b"converted audio"
        );
        if keep {
            assert_eq!(
                std::fs::read(directory.path().join("out.mp4")).unwrap(),
                b"source audio"
            );
        } else {
            assert!(!directory.path().join("out.mp4").exists());
        }
        assert!(String::from_utf8_lossy(&output.stderr).contains("archive"));
        clean(directory.path());
    }
}

#[test]
fn missing_probe_preserves_source_and_publication_replaces_outputs_like_upstream() {
    let server = Server::new();
    let tools = Tools::new("aac", false);
    let directory = tempfile::tempdir().unwrap();
    let client = YoutubeDl::new()
        .unwrap()
        .with_ffmpeg(&tools.ffmpeg)
        .with_ffprobe(directory.path().join("missing-probe"));
    let info = client.extract_info(&server.url("source.mp4")).unwrap();
    let result = client.download_audio_with_options(
        &info,
        directory.path().join("missing.mp4"),
        DownloadOptions::default(),
        &AudioOptions::new("mp3", "5").unwrap(),
    );
    assert!(result.is_err());
    assert_eq!(
        std::fs::read(directory.path().join("missing.mp4")).unwrap(),
        b"source audio"
    );
    let client = tools.client();
    let final_path = directory.path().join("race.mp3");
    let result = client.download_audio_with_progress(
        &info,
        directory.path().join("race.mp4"),
        DownloadOptions::default(),
        &AudioOptions::new("mp3", "5").unwrap(),
        |progress| {
            if progress.phase == DownloadPhase::Publishing {
                std::fs::write(&final_path, b"competitor").unwrap();
            }
            ControlFlow::Continue(())
        },
    );
    // Upstream processors overwrite their outputs (`os.replace`).
    assert!(result.is_ok());
    assert_eq!(std::fs::read(final_path).unwrap(), b"converted audio");
    assert!(!directory.path().join("race.mp4").exists());
    clean(directory.path());
}

#[test]
fn selected_pair_merges_then_probes_and_converts_with_original_retention() {
    let server = Server::new();
    let tools = Tools::new("aac", false);
    let client = tools.client();
    let info: yt_dlp_rs::VideoInfo = serde_json::from_value(serde_json::json!({
        "id":"pair", "title":"pair", "ext":"mp4", "url":server.url("video.mp4"),
        "webpage_url":server.url("page"), "extractor":"fixture", "extractor_key":"Fixture", "direct":false,
        "requested_formats":[
            {"format_id":"v", "url":server.url("video.mp4"), "ext":"mp4", "stream_kind":"video_only"},
            {"format_id":"a", "url":server.url("audio.m4a"), "ext":"m4a", "stream_kind":"audio_only"}
        ]
    })).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let mut options = AudioOptions::new("mp3", "5").unwrap();
    options.keep_video = true;
    let mut phases = Vec::new();
    let result = client
        .download_audio_with_progress(
            &info,
            directory.path().join("pair.mp4"),
            DownloadOptions::default(),
            &options,
            |progress| {
                phases.push(progress.phase);
                ControlFlow::Continue(())
            },
        )
        .unwrap();
    assert_eq!(std::fs::read(&result.path).unwrap(), b"converted audio");
    assert_eq!(
        std::fs::read(result.kept_video.unwrap()).unwrap(),
        b"source audio"
    );
    let merge = phases
        .iter()
        .position(|phase| *phase == DownloadPhase::Merging)
        .unwrap();
    let processing = phases
        .iter()
        .position(|phase| *phase == DownloadPhase::Postprocessing)
        .unwrap();
    assert!(merge < processing);
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
    assert_eq!(server.requests.lock().unwrap().len(), 2);
    clean(directory.path());
}

#[test]
fn optional_real_tools_encode_mp3_wav_and_alac_as_audio_only() {
    let (Some(ffmpeg), Some(ffprobe)) = (
        std::env::var_os("YT_DLP_RS_TEST_FFMPEG"),
        std::env::var_os("YT_DLP_RS_TEST_FFPROBE"),
    ) else {
        return;
    };
    let server = Server::with_body(include_bytes!("fixtures/merge/audio.m4a"));
    let client = YoutubeDl::new()
        .unwrap()
        .with_ffmpeg(&ffmpeg)
        .with_ffprobe(&ffprobe);
    let info = client.extract_info(&server.url("audio.m4a")).unwrap();
    for (target, expected) in [("mp3", "mp3"), ("wav", "pcm_s16le"), ("alac", "alac")] {
        let directory = tempfile::tempdir().unwrap();
        let result = client
            .download_audio_with_options(
                &info,
                directory.path().join("audio.m4a"),
                DownloadOptions::default(),
                &AudioOptions::new(target, "5").unwrap(),
            )
            .unwrap();
        let output = Command::new(&ffprobe)
            .args([
                "-v",
                "error",
                "-show_entries",
                "stream=codec_type,codec_name,channels",
                "-of",
                "json",
            ])
            .arg(&result.path)
            .output()
            .unwrap();
        success(&output);
        let probe: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let streams = probe["streams"].as_array().unwrap();
        assert_eq!(streams.len(), 1);
        assert_eq!(streams[0]["codec_type"], "audio");
        assert_eq!(streams[0]["codec_name"], expected);
        assert!(streams[0]["channels"].as_u64().unwrap() > 0);
        clean(directory.path());
    }
}

#[test]
fn cli_audio_default_prefers_audio_but_keep_and_explicit_selector_preserve_selection() {
    let server = Server::new();
    let tools = Tools::new("aac", false);
    for (extra, expected_pair) in [
        (vec![], false),
        (vec!["-k"], true),
        (vec!["-f", "bestvideo+bestaudio"], true),
        (vec!["-k", "--no-keep-video"], false),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let url = server.url("audio-page.html");
        let mut args = vec!["-x", "-j"];
        args.extend(extra);
        args.push(&url);
        let output = tools.cli(&args, directory.path());
        success(&output);
        let info: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            info.get("requested_formats")
                .is_some_and(|value| value.as_array().is_some_and(|values| !values.is_empty())),
            expected_pair,
            "{info}"
        );
        if !expected_pair {
            assert!(
                info["format_id"].as_str().unwrap().contains("hls-1"),
                "{info}"
            );
        }
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }
    assert!(tools.log(&tools.probe).is_empty());
    assert!(tools.log(&tools.ffmpeg).is_empty());
    assert!(
        !server
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|line| line.contains("segment.ts"))
    );
}

#[test]
fn post_overwrites_replace_existing_audio_unless_disabled() {
    let server = Server::new();
    let tools = Tools::new("aac", false);
    for (flag, kept) in [
        (None, false),
        (Some("--no-post-overwrites"), true),
        (Some("--post-overwrites"), false),
    ] {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("out.m4a"), b"existing").unwrap();
        let mut args = vec!["-x", "--no-progress", "-o", "out.mp4"];
        args.extend(flag);
        let url = server.url("source.mp4");
        args.push(&url);
        let output = tools.cli(&args, directory.path());
        success(&output);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let audio = std::fs::read(directory.path().join("out.m4a")).unwrap();
        if kept {
            // Upstream keeps both files when the output exists.
            assert!(
                stdout.ends_with("[ExtractAudio] Post-process file out.m4a exists, skipping\n")
            );
            assert_eq!(audio, b"existing");
            assert!(directory.path().join("out.mp4").exists());
        } else {
            assert!(stdout.ends_with(
                "[ExtractAudio] Destination: out.m4a\nDeleting original file out.mp4 (pass -k to keep)\n"
            ));
            assert_ne!(audio, b"existing");
            assert!(!directory.path().join("out.mp4").exists());
        }
        clean(directory.path());
    }
}

use std::{
    io::{Read, Write},
    net::TcpListener,
    ops::ControlFlow,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
use yt_dlp_rs::{DownloadOptions, DownloadPhase, Error, VideoInfo, YoutubeDl};

struct Server {
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Server {
    fn new() -> Self {
        Self::with_bodies(b"video-bytes", b"audio-bytes", "video/mp4", "audio/mp4")
    }
    fn with_bodies(
        video: &'static [u8],
        audio: &'static [u8],
        video_mime: &'static str,
        audio_mime: &'static str,
    ) -> Self {
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
                    .set_read_timeout(Some(Duration::from_secs(2)))
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
                let first = String::from_utf8(request)
                    .unwrap()
                    .lines()
                    .next()
                    .unwrap()
                    .to_owned();
                let (body, mime) = if first.contains("/video ") {
                    (video, video_mime)
                } else {
                    (audio, audio_mime)
                };
                record.lock().unwrap().push(first);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
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
    fn info(&self, reversed: bool) -> VideoInfo {
        let mut streams = vec![
            serde_json::json!({"format_id":"v", "url":format!("{}/video",self.url), "ext":"mp4", "stream_kind":"video_only"}),
            serde_json::json!({"format_id":"a", "url":format!("{}/audio",self.url), "ext":"m4a", "stream_kind":"audio_only"}),
        ];
        if reversed {
            streams.reverse();
        }
        serde_json::from_value(serde_json::json!({"id":"fixture", "title":"fixture", "ext":"mp4", "url":format!("{}/video",self.url), "webpage_url":self.url, "extractor":"fixture", "extractor_key":"Fixture", "direct":false, "requested_formats":streams})).unwrap()
    }
    fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
    }
}

// Compile a native executable so process tests also run without a shell on Windows.
fn helper() -> &'static Path {
    static HELPER: OnceLock<PathBuf> = OnceLock::new();
    HELPER.get_or_init(|| {
        let directory = tempfile::tempdir().unwrap().keep();
        let source = directory.join("fake.rs");
        std::fs::write(&source, r#"
fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args == ["-version"] { println!("ffmpeg version merge-fixture"); return; }
    let inputs: Vec<_> = args.windows(2).filter(|pair| pair[0] == "-i").map(|pair| &pair[1]).collect();
    assert_eq!(inputs.len(),2);
    // FFmpegMergerPP: inputs in selection order, each input's streams mapped.
    let first = std::fs::read(inputs[0]).unwrap();
    let maps: Vec<_> = args.windows(2).filter(|pair| pair[0] == "-map").map(|pair| &pair[1]).collect();
    if first == b"video-bytes" {
        assert_eq!(std::fs::read(inputs[1]).unwrap(),b"audio-bytes");
        assert_eq!(maps,["0:v:0","1:a:0"]);
    } else {
        assert_eq!(first,b"audio-bytes");
        assert_eq!(std::fs::read(inputs[1]).unwrap(),b"video-bytes");
        assert_eq!(maps,["0:a:0","1:v:0"]);
    }
    let output = args.last().unwrap();
    std::fs::write(output,b"merged-video-audio").unwrap();
    if std::env::current_exe().unwrap().file_stem().unwrap() == "failure" { eprintln!("fixture failed after writing output"); std::process::exit(1); }
}
"#).unwrap();
        let binary = directory.join(format!("fake-ffmpeg{}", std::env::consts::EXE_SUFFIX));
        assert!(Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into())).arg(source).arg("--edition=2024").arg("-Dwarnings").arg("-o").arg(&binary).status().unwrap().success());
        binary
    })
}
fn clean(directory: &Path, expected: &[&str]) {
    let mut names: Vec<_> = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, expected);
}

#[test]
fn selected_order_and_bytes_are_preserved_with_ffmpeg_inputs_in_selection_order() {
    for reversed in [false, true] {
        let server = Server::new();
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("out.mp4");
        let mut progress = Vec::new();
        let bytes = YoutubeDl::new()
            .unwrap()
            .with_ffmpeg(helper())
            .download_with_progress(
                &server.info(reversed),
                &output,
                DownloadOptions::default(),
                |event| {
                    progress.push(event);
                    ControlFlow::Continue(())
                },
            )
            .unwrap();
        assert_eq!(bytes, b"merged-video-audio".len() as u64);
        assert_eq!(std::fs::read(&output).unwrap(), b"merged-video-audio");
        assert_eq!(
            server.requests(),
            if reversed {
                vec!["GET /audio HTTP/1.1", "GET /video HTTP/1.1"]
            } else {
                vec!["GET /video HTTP/1.1", "GET /audio HTTP/1.1"]
            }
        );
        for index in [1, 2] {
            assert!(
                progress
                    .iter()
                    .any(|event| event.stream_index == Some(index)
                        && event.stream_count == Some(2)
                        && event.phase == DownloadPhase::Downloading)
            );
        }
        assert!(
            progress
                .iter()
                .any(|event| event.phase == DownloadPhase::Merging)
        );
        assert_eq!(progress.last().unwrap().phase, DownloadPhase::Publishing);
        clean(directory.path(), &["out.mp4"]);
    }
}

#[test]
fn cancellation_at_each_public_phase_cleans_every_temporary_file() {
    for (phase, index) in [
        (DownloadPhase::Requesting, Some(1)),
        (DownloadPhase::Downloading, Some(1)),
        (DownloadPhase::Requesting, Some(2)),
        (DownloadPhase::Downloading, Some(2)),
        (DownloadPhase::Merging, None),
        (DownloadPhase::Publishing, None),
    ] {
        let server = Server::new();
        let directory = tempfile::tempdir().unwrap();
        let result = YoutubeDl::new()
            .unwrap()
            .with_ffmpeg(helper())
            .download_with_progress(
                &server.info(false),
                directory.path().join("out.mp4"),
                DownloadOptions::default(),
                |event| {
                    if event.phase == phase && event.stream_index == index {
                        ControlFlow::Break(())
                    } else {
                        ControlFlow::Continue(())
                    }
                },
            );
        assert!(
            matches!(result, Err(Error::DownloadCancelled)),
            "{phase:?}/{index:?}: {result:?}"
        );
        clean(directory.path(), &[]);
        if phase == DownloadPhase::Requesting && index == Some(1) {
            assert!(server.requests().is_empty());
        }
    }
}

#[test]
fn missing_tool_and_nonexistent_tool_fail_before_media_requests() {
    for configured in [false, true] {
        let server = Server::new();
        let directory = tempfile::tempdir().unwrap();
        let downloader = YoutubeDl::new().unwrap();
        let downloader = if configured {
            downloader.with_ffmpeg(directory.path().join("does-not-exist"))
        } else {
            downloader
        };
        let error = downloader
            .download(&server.info(false), directory.path().join("out.mp4"))
            .unwrap_err();
        assert!(matches!(error, Error::Postprocess(_)), "{error}");
        assert!(server.requests().is_empty());
        clean(directory.path(), &[]);
    }
}

#[test]
fn failure_after_output_creation_removes_workspace_and_output() {
    let tool_directory = tempfile::tempdir().unwrap();
    let tool = tool_directory
        .path()
        .join(format!("failure{}", std::env::consts::EXE_SUFFIX));
    std::fs::copy(helper(), &tool).unwrap();
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let error = YoutubeDl::new()
        .unwrap()
        .with_ffmpeg(tool)
        .download(&server.info(false), directory.path().join("out.mp4"))
        .unwrap_err();
    assert!(matches!(error, Error::Postprocess(_)), "{error}");
    clean(directory.path(), &[]);
}

#[test]
fn existing_destination_and_publication_race_preserve_other_file() {
    for race in [false, true] {
        let server = Server::new();
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("out.mp4");
        if !race {
            std::fs::write(&output, b"other file").unwrap();
        }
        let result = YoutubeDl::new()
            .unwrap()
            .with_ffmpeg(helper())
            .download_with_progress(
                &server.info(false),
                &output,
                DownloadOptions::default(),
                |event| {
                    if race && event.phase == DownloadPhase::Publishing {
                        std::fs::write(&output, b"other file").unwrap();
                    }
                    ControlFlow::Continue(())
                },
            );
        assert!(matches!(result, Err(Error::OutputExists(_))), "{result:?}");
        assert_eq!(std::fs::read(&output).unwrap(), b"other file");
        if !race {
            assert!(server.requests().is_empty());
        }
        clean(directory.path(), &["out.mp4"]);
    }
}

#[test]
fn standalone_download_ignores_invalid_ffmpeg_path() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let mut info = server.info(false);
    info.requested_formats.clear();
    let output = directory.path().join("out.mp4");
    YoutubeDl::new()
        .unwrap()
        .with_ffmpeg(directory.path().join("missing"))
        .download(&info, &output)
        .unwrap();
    assert_eq!(std::fs::read(output).unwrap(), b"video-bytes");
    clean(directory.path(), &["out.mp4"]);
}

#[test]
fn real_ffmpeg_stream_copy_preserves_video_and_audio() {
    let ffmpeg = std::env::var_os("YT_DLP_RS_TEST_FFMPEG");
    let ffprobe = std::env::var_os("YT_DLP_RS_TEST_FFPROBE");
    let (ffmpeg, ffprobe) = match (ffmpeg, ffprobe) {
        (None, None) => return,
        (Some(ffmpeg), Some(ffprobe)) => (ffmpeg, ffprobe),
        _ => panic!("set both YT_DLP_RS_TEST_FFMPEG and YT_DLP_RS_TEST_FFPROBE"),
    };
    let mp4_video: &'static [u8] = include_bytes!("fixtures/merge/video.mp4");
    let webm_video: &'static [u8] = include_bytes!("fixtures/merge/video.webm");
    let m4a_audio: &'static [u8] = include_bytes!("fixtures/merge/audio.m4a");
    let webm_audio: &'static [u8] = include_bytes!("fixtures/merge/audio.webm");
    let ts_video: &'static [u8] = include_bytes!("fixtures/hls/audio-video/video/segment-0.ts");
    let aac_audio: &'static [u8] = include_bytes!("fixtures/hls/audio-video/audio/segment-0.aac");
    let ts_audio: &'static [u8] = include_bytes!("fixtures/hls/audio-video/audio/track.ts");
    for (video, audio, video_ext, audio_ext, output_ext, video_codec, audio_codec) in [
        (mp4_video, m4a_audio, "mp4", "m4a", "mp4", "h264", "aac"),
        (
            webm_video, webm_audio, "webm", "webm", "webm", "vp9", "opus",
        ),
        (mp4_video, webm_audio, "mp4", "webm", "mkv", "h264", "opus"),
        (ts_video, aac_audio, "ts", "aac", "mp4", "h264", "aac"),
        (ts_video, m4a_audio, "ts", "m4a", "mp4", "h264", "aac"),
        (ts_video, ts_audio, "ts", "ts", "mp4", "h264", "aac"),
    ] {
        let video_mime = match video_ext {
            "webm" => "video/webm",
            "ts" => "video/mp2t",
            _ => "video/mp4",
        };
        let audio_mime = match audio_ext {
            "webm" => "audio/webm",
            "ts" => "video/mp2t",
            "aac" => "audio/aac",
            _ => "audio/mp4",
        };
        let server = Server::with_bodies(video, audio, video_mime, audio_mime);
        let mut info = server.info(true);
        info.ext = output_ext.into();
        // A reversed (audio first) selection: upstream's FFmpegMergerPP maps
        // each requested format's streams in selection order.
        info.requested_formats[0].ext = audio_ext.into();
        info.requested_formats[1].ext = video_ext.into();
        // TS files exercise the fragment route used by HLS extraction.
        for format in &mut info.requested_formats {
            if format.ext == "ts" {
                format.fragments = vec![yt_dlp_rs::Fragment {
                    url: format.url.clone(),
                    byte_range: None,
                    encryption: None,
                    path: None,
                    duration: None,
                }];
            }
        }
        let directory = tempfile::tempdir().unwrap();
        let name = format!("out.{output_ext}");
        let output = directory.path().join(&name);
        let bytes = YoutubeDl::new()
            .unwrap()
            .with_ffmpeg(&ffmpeg)
            .download(&info, &output)
            .unwrap();
        assert_eq!(bytes, output.metadata().unwrap().len());
        assert!(bytes > 0);
        let probe = Command::new(Path::new(&ffprobe).canonicalize().unwrap())
            .args([
                "-v",
                "error",
                "-show_entries",
                "stream=codec_name,codec_type,extradata_size:format=format_name",
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
        assert!(probe.stderr.is_empty());
        let info: serde_json::Value = serde_json::from_slice(&probe.stdout).unwrap();
        let streams = info["streams"].as_array().unwrap();
        assert_eq!(streams.len(), 2);
        assert_eq!(streams[0]["codec_type"], "audio");
        assert_eq!(streams[0]["codec_name"], audio_codec);
        assert_eq!(streams[1]["codec_type"], "video");
        assert_eq!(streams[1]["codec_name"], video_codec);
        if output_ext == "mp4" && audio_codec == "aac" {
            // FFmpeg's MP4 muxer automatically inserts aac_adtstoasc for ADTS
            // AAC copied from elementary AAC or MPEG-TS inputs.
            assert!(
                streams[0]["extradata_size"]
                    .as_u64()
                    .is_some_and(|size| size > 0)
            );
        }
        let format = info["format"]["format_name"].as_str().unwrap();
        assert!(
            if output_ext == "mp4" {
                format.split(',').any(|name| name == "mp4")
            } else {
                format.split(',').any(|name| name == "matroska")
            },
            "unexpected output container: {format}"
        );
        clean(directory.path(), &[&name]);
    }
}

#[test]
fn upstream_merge_files_are_named_reused_and_deleted_unless_kept() {
    use yt_dlp_rs::MergeFiles;
    for keep in [false, true] {
        let server = Server::new();
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("out.mp4");
        // An existing complete input is reused, like upstream FileDownloader.
        std::fs::write(directory.path().join("out.fv.mp4"), b"video-bytes").unwrap();
        YoutubeDl::new()
            .unwrap()
            .with_ffmpeg(helper())
            .download_with_options(
                &server.info(false),
                &output,
                DownloadOptions {
                    merge_files: MergeFiles::Upstream { keep },
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(server.requests(), ["GET /audio HTTP/1.1"]);
        assert_eq!(std::fs::read(&output).unwrap(), b"merged-video-audio");
        if keep {
            assert_eq!(
                std::fs::read(directory.path().join("out.fa.m4a")).unwrap(),
                b"audio-bytes"
            );
            clean(directory.path(), &["out.fa.m4a", "out.fv.mp4", "out.mp4"]);
        } else {
            clean(directory.path(), &["out.mp4"]);
        }
    }
}

#[test]
fn cli_merge_prints_upstream_format_destinations_merger_and_deletions() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let info = serde_json::json!({
        "id": "fixture", "title": "fixture", "ext": "mp4",
        "url": format!("{}/video", server.url),
        "webpage_url": server.url, "extractor": "fixture", "extractor_key": "Fixture",
        "formats": [
            {"format_id": "v", "url": format!("{}/video", server.url), "ext": "mp4", "vcodec": "avc1", "acodec": "none", "protocol": "http"},
            {"format_id": "a", "url": format!("{}/audio", server.url), "ext": "m4a", "vcodec": "none", "acodec": "mp4a", "protocol": "http"},
        ],
    });
    let path = directory.path().join("info.json");
    std::fs::write(&path, info.to_string()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--no-progress", "-f", "v+a", "-o", "out.%(ext)s"])
        .arg("--ffmpeg-location")
        .arg(helper())
        .arg("--load-info-json")
        .arg(&path)
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<_> = stdout
        .lines()
        .filter(|line| !line.starts_with("[info]"))
        .collect();
    assert_eq!(
        lines,
        [
            "[download] Destination: out.fv.mp4",
            "[download] Download completed",
            "[download] Destination: out.fa.m4a",
            "[download] Download completed",
            "[Merger] Merging formats into \"out.mp4\"",
            "Deleting original file out.fv.mp4 (pass -k to keep)",
            "Deleting original file out.fa.m4a (pass -k to keep)",
        ]
    );
    clean(directory.path(), &["info.json", "out.mp4"]);
}

#[test]
fn cli_merge_output_format_prefers_containers_and_validates_like_upstream() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let info = serde_json::json!({
        "id": "fixture", "title": "fixture", "ext": "mp4",
        "url": format!("{}/video", server.url),
        "webpage_url": server.url, "extractor": "fixture", "extractor_key": "Fixture",
        "formats": [
            {"format_id": "v", "url": format!("{}/video", server.url), "ext": "mp4", "vcodec": "avc1", "acodec": "none"},
            {"format_id": "a", "url": format!("{}/audio", server.url), "ext": "m4a", "vcodec": "none", "acodec": "mp4a"},
        ],
    });
    let path = directory.path().join("info.json");
    std::fs::write(&path, info.to_string()).unwrap();
    let run = |format: &str| {
        Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .args(["--no-progress", "-f", "v+a", "-o", "out.%(ext)s"])
            .args(["--merge-output-format", format])
            .arg("--ffmpeg-location")
            .arg(helper())
            .arg("--load-info-json")
            .arg(&path)
            .current_dir(directory.path())
            .output()
            .unwrap()
    };
    // avc1+mp4a would merge to mp4; the first preference wins via get_compatible_ext.
    let output = run("mkv/mp4");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("[Merger] Merging formats into \"out.mkv\"")
    );
    assert_eq!(
        std::fs::read(directory.path().join("out.mkv")).unwrap(),
        b"merged-video-audio"
    );
    // `re.match` accepts a valid prefix; other values are usage errors.
    let output = run("bogus");
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .ends_with("yt-dlp-rs: error: invalid merge output format \"bogus\" given\n")
    );
}

#[test]
fn cli_without_ffmpeg_keeps_format_files_unmerged_or_aborts_like_upstream() {
    let server = Server::new();
    let empty = tempfile::tempdir().unwrap();
    for abort in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let info = serde_json::json!({
            "id": "fixture", "title": "fixture", "ext": "mp4",
            "url": format!("{}/video", server.url),
            "webpage_url": server.url, "extractor": "fixture", "extractor_key": "Fixture",
            "formats": [
                {"format_id": "v", "url": format!("{}/video", server.url), "ext": "mp4", "vcodec": "avc1", "acodec": "none"},
                {"format_id": "a", "url": format!("{}/audio", server.url), "ext": "m4a", "vcodec": "none", "acodec": "mp4a"},
            ],
        });
        let path = directory.path().join("info.json");
        std::fs::write(&path, info.to_string()).unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"));
        // No ffmpeg on PATH and no --ffmpeg-location.
        command
            .env("PATH", empty.path())
            .args(["--no-progress", "-o", "out.%(ext)s", "--load-info-json"])
            .arg(&path)
            .current_dir(directory.path());
        if abort {
            command.arg("--abort-on-error");
        }
        let output = command.output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        let message = "You have requested merging of multiple formats but ffmpeg is not installed";
        if abort {
            // The raised error makes `download_with_info_file` retry the
            // webpage URL, whose single format needs no merge.
            assert!(output.status.success(), "{stderr}");
            assert!(
                stderr.starts_with(&format!(
                    "ERROR: {message}. Aborting due to --abort-on-error\n\r\n\
                     WARNING: The info failed to download: ERROR: {message}. Aborting due to --abort-on-error; trying with URL {}/\n",
                    server.url
                )),
                "{stderr}"
            );
            assert!(!directory.path().join("out.fv.mp4").exists());
        } else {
            assert!(output.status.success(), "{stderr}");
            assert!(
                stderr.contains(&format!("WARNING: {message}. The formats won't be merged")),
                "{stderr}"
            );
            assert_eq!(
                std::fs::read(directory.path().join("out.fv.mp4")).unwrap(),
                b"video-bytes"
            );
            assert_eq!(
                std::fs::read(directory.path().join("out.fa.m4a")).unwrap(),
                b"audio-bytes"
            );
            clean(directory.path(), &["info.json", "out.fa.m4a", "out.fv.mp4"]);
        }
    }
}

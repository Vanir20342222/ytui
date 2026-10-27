use std::{
    io::{Read, Write},
    net::TcpListener,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
use yt_dlp_rs::YoutubeDl;

struct Server {
    origin: String,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Server {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let worker = thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                let (mut stream, _) = match listener.accept() {
                    Ok(pair) => pair,
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
                let request = String::from_utf8(request).unwrap();
                let path = request
                    .split_whitespace()
                    .nth(1)
                    .unwrap()
                    .trim_start_matches('/');
                assert!(path.split('/').all(|part| !part.is_empty() && part != ".."));
                let body = std::fs::read(
                    concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/hls/").to_owned() + path,
                )
                .unwrap();
                let mime = if path.ends_with("m3u8") {
                    "application/vnd.apple.mpegurl"
                } else if path.ends_with("key") {
                    "application/octet-stream"
                } else if path.ends_with("aac") {
                    "audio/aac"
                } else if path.ends_with("ts") {
                    "video/mp2t"
                } else {
                    "video/mp4"
                };
                if let Some(range) = request.lines().find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("range: bytes=")
                        .map(str::to_owned)
                }) {
                    let (start, end) = range.split_once('-').unwrap();
                    let start: usize = start.parse().unwrap();
                    let end: usize = end.parse().unwrap();
                    let selected = &body[start..=end];
                    let _ = write!(
                        stream,
                        "HTTP/1.1 206 Partial Content\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nContent-Range: bytes {start}-{end}/{}\r\nConnection: close\r\n\r\n",
                        selected.len(),
                        body.len()
                    );
                    let _ = stream.write_all(selected);
                } else {
                    let _ = write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(&body);
                }
            }
        });
        Self {
            origin,
            stop,
            worker: Some(worker),
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
    }
}

#[test]
fn saved_transport_stream_and_fragmented_mp4_assemble_exact_media() {
    let server = Server::new();
    let dl = YoutubeDl::new().unwrap();
    for (kind, ext, count) in [
        ("ts", "ts", 3),
        ("fmp4", "mp4", 4),
        ("range", "mp4", 4),
        ("aes-ts", "ts", 3),
        ("aes-fmp4", "mp4", 4),
        ("aes-range", "mp4", 4),
    ] {
        let info = dl
            .extract_info(&format!("{}/{kind}/index.m3u8", server.origin))
            .unwrap();
        // Upstream labels HLS `mp4`; the assembled bytes keep their container.
        assert_eq!(info.ext, "mp4");
        assert_eq!(info.fragments.len(), count);
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join(format!("media.{ext}"));
        let bytes = dl.download(&info, &output).unwrap();
        assert_eq!(yt_dlp_rs::container::sniff(&output), Some(ext), "{kind}");
        let actual = std::fs::read(&output).unwrap();
        let mut expected = Vec::new();
        for (index, fragment) in info.fragments.iter().enumerate() {
            if kind.starts_with("aes-") {
                let plain_kind = if kind == "aes-ts" { "ts" } else { "fmp4" };
                let name = if plain_kind == "fmp4" && index == 0 {
                    "init.mp4".to_owned()
                } else {
                    let number = if plain_kind == "fmp4" {
                        index - 1
                    } else {
                        index
                    };
                    let suffix = if plain_kind == "fmp4" { "m4s" } else { "ts" };
                    format!("segment-{number}.{suffix}")
                };
                expected.extend(
                    std::fs::read(format!(
                        "{}/tests/fixtures/hls/{plain_kind}/{name}",
                        env!("CARGO_MANIFEST_DIR")
                    ))
                    .unwrap(),
                );
            } else {
                let path = fragment.url.path();
                let body = std::fs::read(
                    concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/hls").to_owned() + path,
                )
                .unwrap();
                if let Some(range) = &fragment.byte_range {
                    expected.extend_from_slice(
                        &body[range.offset as usize..(range.offset + range.length) as usize],
                    );
                } else {
                    expected.extend(body);
                }
            }
        }
        assert_eq!(actual, expected);
        assert_eq!(bytes, actual.len() as u64);
        assert_ne!(&actual[..7], b"#EXTM3U");
        if let Some(probe) = std::env::var_os("YT_DLP_RS_TEST_FFPROBE") {
            let result = Command::new(probe)
                .args(["-v", "error", "-show_streams", "-of", "json"])
                .arg(&output)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            let data: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
            let tracks = data["streams"].as_array().unwrap();
            assert_eq!(tracks.len(), 2);
            assert!(
                tracks
                    .iter()
                    .any(|track| track["codec_type"] == "video" && track["codec_name"] == "h264")
            );
            assert!(
                tracks
                    .iter()
                    .any(|track| track["codec_type"] == "audio" && track["codec_name"] == "aac")
            );
        }
    }
}

#[test]
fn real_external_audio_rendition_merges_complete_native_fragment_downloads() {
    let (ffmpeg, ffprobe) = match (
        std::env::var_os("YT_DLP_RS_TEST_FFMPEG"),
        std::env::var_os("YT_DLP_RS_TEST_FFPROBE"),
    ) {
        (None, _) => return,
        (Some(ffmpeg), Some(ffprobe)) => (ffmpeg, ffprobe),
        _ => panic!("YT_DLP_RS_TEST_FFMPEG requires YT_DLP_RS_TEST_FFPROBE"),
    };
    let server = Server::new();
    let dl = YoutubeDl::new().unwrap().with_ffmpeg(ffmpeg);
    let info = dl
        .extract_info(&format!("{}/audio-video/master.m3u8", server.origin))
        .unwrap();
    assert_eq!(info.format_id.as_deref(), Some("140+main-English"));
    assert_eq!(info.ext, "mp4");
    assert_eq!(
        info.requested_formats[0].audio_group.as_deref(),
        Some("main")
    );
    assert_eq!(
        info.requested_formats[1].audio_group.as_deref(),
        Some("main")
    );
    assert_eq!(info.requested_formats[1].language.as_deref(), Some("en"));
    assert_eq!(info.requested_formats[0].fragments.len(), 3);
    assert_eq!(info.requested_formats[1].fragments.len(), 4);
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("grouped.mp4");
    let mut events = Vec::new();
    let bytes = dl
        .download_with_progress(
            &info,
            &output,
            yt_dlp_rs::DownloadOptions::default(),
            |event| {
                events.push(event);
                std::ops::ControlFlow::Continue(())
            },
        )
        .unwrap();
    assert_eq!(bytes, std::fs::metadata(&output).unwrap().len());
    for index in [1, 2] {
        assert!(
            events
                .iter()
                .any(|event| event.stream_index == Some(index) && event.fragment_index.is_some())
        );
    }
    assert_eq!(
        events
            .iter()
            .filter(|event| event.phase == yt_dlp_rs::DownloadPhase::Publishing)
            .count(),
        1
    );
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    let result = Command::new(ffprobe)
        .args(["-v", "error", "-show_streams", "-of", "json"])
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let data: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    let tracks = data["streams"].as_array().unwrap();
    assert_eq!(tracks.len(), 2);
    assert!(
        tracks
            .iter()
            .any(|track| track["codec_type"] == "video" && track["codec_name"] == "h264")
    );
    assert!(
        tracks
            .iter()
            .any(|track| track["codec_type"] == "audio" && track["codec_name"] == "aac")
    );
}

/// A compiled fake ffmpeg: `-version` prints a banner; otherwise it logs its
/// arguments beside itself and writes `fixed` to the last argument.
fn fake_ffmpeg(directory: &std::path::Path) -> std::path::PathBuf {
    let source = directory.join("fake.rs");
    std::fs::write(
        &source,
        r#"
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args == ["-version"] { println!("ffmpeg version fixup-fixture"); return; }
    let log = std::env::current_exe().unwrap().with_extension("log");
    std::fs::write(log, args.join("\n")).unwrap();
    std::fs::write(args.last().unwrap(), b"fixed").unwrap();
}
"#,
    )
    .unwrap();
    let binary = directory.join(format!("ffmpeg{}", std::env::consts::EXE_SUFFIX));
    assert!(
        Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
            .arg(&source)
            .args(["--edition=2024", "-o"])
            .arg(&binary)
            .status()
            .unwrap()
            .success()
    );
    binary
}

#[test]
fn transport_stream_downloads_get_upstream_fixup_m3u8_or_warnings() {
    let server = Server::new();
    let tools = tempfile::tempdir().unwrap();
    let ffmpeg = fake_ffmpeg(tools.path());
    let empty = tempfile::tempdir().unwrap();
    let url = format!("{}/ts/index.m3u8", server.origin);
    let run = |args: &[&str], with_ffmpeg: bool| {
        let directory = tempfile::tempdir().unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"));
        command
            .env("PATH", empty.path())
            .args(["--no-progress", "-o", "out.%(ext)s"])
            .args(args)
            .arg(&url)
            .current_dir(directory.path());
        if with_ffmpeg {
            command.arg("--ffmpeg-location").arg(&ffmpeg);
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let bytes = std::fs::read(directory.path().join("out.mp4")).unwrap();
        (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
            bytes,
        )
    };
    // Checked with pinned upstream on a real TS stream: the MP4-labeled TS is
    // remuxed with a stream copy (no ffprobe here, so no AAC filter).
    let (stdout, _, bytes) = run(&[], true);
    assert!(
        stdout.contains("[FixupM3u8] Fixing MPEG-TS in MP4 container of \"out.mp4\""),
        "{stdout}"
    );
    assert_eq!(bytes, b"fixed");
    let log = std::fs::read_to_string(ffmpeg.with_extension("log")).unwrap();
    let args: Vec<&str> = log.lines().collect();
    let input = args.iter().position(|arg| *arg == "-i").unwrap();
    assert_eq!(args[input - 1], "mpegts");
    assert_eq!(
        &args[input + 2..args.len() - 1],
        [
            "-map",
            "0",
            "-dn",
            "-ignore_unknown",
            "-c",
            "copy",
            "-f",
            "mp4"
        ]
    );
    assert!(args.last().unwrap().ends_with("out.temp.mp4"));
    let message = "WARNING: index: Possible MPEG-TS in MP4 container or malformed AAC timestamps";
    let (_, stderr, bytes) = run(&[], false);
    assert!(
        stderr.contains(&format!(
            "{message}. Install ffmpeg to fix this automatically"
        )),
        "{stderr}"
    );
    assert_eq!(yt_dlp_rs::container::sniff_bytes(&bytes), Some("ts"));
    let (stdout, stderr, bytes) = run(&["--fixup", "warn"], true);
    assert!(stderr.contains(&format!("{message}\n")), "{stderr}");
    assert!(!stdout.contains("FixupM3u8"));
    assert_ne!(bytes, b"fixed");
    for args in [&["--fixup", "never"][..], &["--hls-use-mpegts"]] {
        let (stdout, stderr, bytes) = run(args, true);
        assert!(
            !stdout.contains("FixupM3u8") && !stderr.contains("MPEG-TS"),
            "{args:?}"
        );
        assert_ne!(bytes, b"fixed");
    }
}

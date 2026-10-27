use std::{
    io::{Read, Write},
    net::TcpListener,
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
use yt_dlp_rs::{StreamKind, YoutubeDl};

struct Server {
    origin: String,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Server {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (done, recorded) = (stop.clone(), requests.clone());
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
                while !request.windows(4).any(|part| part == b"\r\n\r\n") {
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
                recorded.lock().unwrap().push(path.to_owned());
                let body = std::fs::read(format!(
                    "{}/tests/fixtures/dash/{path}",
                    env!("CARGO_MANIFEST_DIR")
                ))
                .unwrap();
                let mime = if path.ends_with("mpd") {
                    "application/dash+xml"
                } else {
                    "video/mp4"
                };
                if let Some(range) = request.lines().find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("range: bytes=")
                        .map(str::to_owned)
                }) {
                    let (start, end) = range.split_once('-').unwrap();
                    let (start, end): (usize, usize) =
                        (start.parse().unwrap(), end.parse().unwrap());
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
            requests,
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
fn probe(path: &std::path::Path, kinds: &[&str]) {
    let Some(executable) = std::env::var_os("YT_DLP_RS_TEST_FFPROBE") else {
        return;
    };
    let output = Command::new(executable)
        .args(["-v", "error", "-show_streams", "-of", "json"])
        .arg(path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let streams = value["streams"].as_array().unwrap();
    assert_eq!(streams.len(), kinds.len());
    for kind in kinds {
        let codec = if *kind == "video" { "h264" } else { "aac" };
        assert!(
            streams
                .iter()
                .any(|stream| stream["codec_type"] == *kind && stream["codec_name"] == codec)
        );
    }
}
#[test]
fn saved_timeline_number_list_and_range_plans_download_exact_media() {
    for manifest in ["manifest.mpd", "number.mpd", "list.mpd", "ranges.mpd"] {
        let server = Server::new();
        let dl = YoutubeDl::new().unwrap();
        let url = format!("{}/media/{manifest}", server.origin);
        for (id, kind, ext) in [
            ("0", StreamKind::VideoOnly, "mp4"),
            ("1", StreamKind::AudioOnly, "m4a"),
        ] {
            let before = server.requests.lock().unwrap().len();
            let info = dl.extract_info_with_format(&url, Some(id)).unwrap();
            assert_eq!(
                &server.requests.lock().unwrap()[before..],
                [format!("media/{manifest}")]
            );
            assert_eq!(info.stream_kind, kind);
            assert_eq!(info.ext, ext);
            let directory = tempfile::tempdir().unwrap();
            let output = directory.path().join(format!("media.{ext}"));
            let bytes = dl.download(&info, &output).unwrap();
            let resource = if kind == StreamKind::VideoOnly {
                "video-combined.mp4"
            } else {
                "audio-combined.mp4"
            };
            let expected = std::fs::read(format!(
                "{}/tests/fixtures/dash/media/{resource}",
                env!("CARGO_MANIFEST_DIR")
            ))
            .unwrap();
            assert_eq!(std::fs::read(&output).unwrap(), expected);
            assert_eq!(bytes, expected.len() as u64);
            assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
            probe(
                &output,
                &[if kind == StreamKind::VideoOnly {
                    "video"
                } else {
                    "audio"
                }],
            );
        }
    }
}
#[test]
fn real_static_dash_downloads_both_representations_and_merges_tracks() {
    let Some(ffmpeg) = std::env::var_os("YT_DLP_RS_TEST_FFMPEG") else {
        return;
    };
    assert!(std::env::var_os("YT_DLP_RS_TEST_FFPROBE").is_some());
    for manifest in ["manifest.mpd", "number.mpd", "list.mpd", "ranges.mpd"] {
        let server = Server::new();
        let dl = YoutubeDl::new().unwrap().with_ffmpeg(&ffmpeg);
        let info = dl
            .extract_info(&format!("{}/media/{manifest}", server.origin))
            .unwrap();
        assert_eq!(info.format_id.as_deref(), Some("0+1"));
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("merged.mp4");
        let bytes = dl.download(&info, &output).unwrap();
        assert_eq!(bytes, std::fs::metadata(&output).unwrap().len());
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
        probe(&output, &["video", "audio"]);
    }
}

#[test]
fn info_json_keeps_upstream_fragment_shape_and_reloads_exact_media() {
    for manifest in ["manifest.mpd", "number.mpd", "list.mpd", "ranges.mpd"] {
        let server = Server::new();
        let directory = tempfile::tempdir().unwrap();
        let cli = |args: &[&str]| {
            Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
                .args([
                    "--no-progress",
                    "--fixup",
                    "never",
                    "-f",
                    "0",
                    "-o",
                    "o.%(ext)s",
                ])
                .args(args)
                .current_dir(directory.path())
                .output()
                .unwrap()
        };
        let url = format!("{}/media/{manifest}", server.origin);
        let output = cli(&["--skip-download", "--write-info-json", &url]);
        assert!(output.status.success(), "{manifest}");
        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(directory.path().join("o.info.json")).unwrap())
                .unwrap();
        // Upstream's DASH format dict: the MPD as manifest_url, relative
        // fragment paths against fragment_base_url, durations and
        // `is_dash_periods`.
        assert_eq!(json["manifest_url"], url, "{manifest}");
        assert_eq!(json["is_dash_periods"], true, "{manifest}");
        let base = json["fragment_base_url"].as_str().unwrap();
        assert!(base.starts_with(&server.origin), "{manifest}");
        let fragments = json["fragments"].as_array().unwrap();
        // Byte ranges of the base resource have no relative path.
        let key = if manifest == "ranges.mpd" {
            "url"
        } else {
            "path"
        };
        assert!(
            fragments
                .iter()
                .all(|fragment| fragment[key].is_string()
                    && fragment.as_object().unwrap().len() <= 3),
            "{manifest}: {fragments:?}"
        );
        // Timeline and template durations; a SegmentList without
        // `duration` has none, as upstream.
        assert!(fragments[0].get("duration").is_none(), "{manifest}");
        assert_eq!(
            fragments[1]["duration"].is_f64(),
            matches!(manifest, "manifest.mpd" | "number.mpd"),
            "{manifest}"
        );
        if manifest == "ranges.mpd" {
            let range = &fragments[1]["byte_range"];
            assert!(range["end"].as_u64().unwrap() > range["start"].as_u64().unwrap());
        }
        let output = cli(&["--load-info-json", "o.info.json"]);
        assert!(
            output.status.success(),
            "{manifest}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let expected = std::fs::read(format!(
            "{}/tests/fixtures/dash/media/video-combined.mp4",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        assert!(
            std::fs::read(directory.path().join("o.mp4")).unwrap() == expected,
            "{manifest}"
        );
    }
}

//! Format listing is extraction-only: local probes and manifests, no downloads.
use serde_json::Value;
use std::{
    io::{Read, Write},
    net::TcpListener,
    process::{Command, Output},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
use yt_dlp_rs::{ExtractionOptions, StreamKind, YoutubeDl};

const PAGE: &str = "<title>Listing fixture</title><video><source src='/low.mp4' type='video/mp4' width='640' height='360'><source src='/high.mp4' type='video/mp4' width='1920' height='1080'></video>";
const MASTER: &str = "#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"audio\",NAME=\"English\",LANGUAGE=\"en\",DEFAULT=YES,URI=\"audio.m3u8\"\n#EXT-X-STREAM-INF:BANDWIDTH=1000000,RESOLUTION=1920x1080,CODECS=\"avc1.4,mp4a.40.2\",AUDIO=\"audio\"\nvideo.m3u8\n";
const MEDIA: &str = "#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\",IV=0x1\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:1,\nsegment.m4s\n#EXT-X-ENDLIST\n";

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
        let (done, record) = (stop.clone(), requests.clone());
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
                    match stream.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => request.extend_from_slice(&buffer[..n]),
                    }
                }
                if !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    continue;
                }
                let request = String::from_utf8(request).unwrap();
                let path = request.split_whitespace().nth(1).unwrap().to_owned();
                let count = {
                    let mut requests = record.lock().unwrap();
                    requests.push(path.clone());
                    requests.iter().filter(|value| **value == path).count()
                };
                let (status, mime, body, extra) = match path.as_str() {
                    "/retry.html" if count == 1 => {
                        (503, "text/plain", "retry", "Retry-After: 0\r\n")
                    }
                    "/page.html" | "/retry.html" => (200, "text/html", PAGE, ""),
                    "/playlist.html" => (
                        200,
                        "text/html",
                        "<title>Playlist listing</title><video><source src='/low.mp4' width='640' height='360'></video><video><source src='/high.mp4' width='1920' height='1080'></video>",
                        "",
                    ),
                    "/invalid-playlist.html" => (
                        200,
                        "text/html",
                        "<video src='/low.mp4'></video><video src='/missing.mp4'></video>",
                        "",
                    ),
                    "/low.mp4" => (200, "video/mp4", "low", ""),
                    "/high.mp4" => (200, "video/mp4", "higher", ""),
                    "/manifest.mpd" => (
                        200,
                        "application/dash+xml",
                        include_str!("fixtures/dash/media/manifest.mpd"),
                        "",
                    ),
                    "/master.m3u8" => (200, "application/vnd.apple.mpegurl", MASTER, ""),
                    "/video.m3u8" | "/audio.m3u8" => {
                        (200, "application/vnd.apple.mpegurl", MEDIA, "")
                    }
                    _ => (404, "text/plain", "missing", ""),
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} Fixture\r\nContent-Type: {mime}\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(body.as_bytes());
            }
        });
        Self {
            origin,
            requests,
            stop,
            worker: Some(worker),
        }
    }
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.origin)
    }
    fn count(&self, path: &str) -> usize {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|value| value.as_str() == path)
            .count()
    }
    fn cli(&self, path: &str, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .args(args)
            .arg(self.url(path))
            .output()
            .unwrap()
    }
    fn assert_no_resources(&self) {
        for path in ["/key.bin", "/init.mp4", "/segment.m4s"] {
            assert_eq!(self.count(path), 0, "{path}");
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
    }
}
fn success(output: &Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone()).unwrap()
}
fn row<'a>(table: &'a str, id: &str) -> &'a str {
    table
        .lines()
        .find(|line| line.split_whitespace().next() == Some(id))
        .unwrap_or_else(|| panic!("missing format row {id}: {table}"))
}
fn assert_header(table: &str) {
    assert!(
        table.lines().any(|line| {
            let columns: Vec<_> = line.split_whitespace().collect();
            columns.contains(&"ID") && columns.contains(&"EXT")
        }),
        "{table}"
    );
}

#[test]
fn listing_and_simulation_list_all_formats_without_selection_or_template_rendering() {
    for args in [
        vec!["-F"],
        vec!["-F", "-s"],
        vec!["--list-formats", "-f", "999"],
    ] {
        let server = Server::new();
        let directory = tempfile::tempdir().unwrap();
        let mut command = args;
        command.extend(["-o", "%(unsupported)s"]);
        let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .args(command)
            .arg(server.url("/page.html"))
            .current_dir(directory.path())
            .output()
            .unwrap();
        let table = success(&output);
        assert_header(&table);
        assert!(row(&table, "0").contains("640x360"));
        assert!(row(&table, "1").contains("1920x1080"));
        assert_eq!(server.count("/low.mp4"), 1);
        assert_eq!(server.count("/high.mp4"), 1);
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }
}

#[test]
fn json_with_listing_follows_table_and_retains_all_formats_after_explicit_selection() {
    let server = Server::new();
    let stdout = success(&server.cli("/page.html", &["-F", "-j", "-f", "1"]));
    let json_line = stdout.lines().find(|line| line.starts_with('{')).unwrap();
    let json_position = stdout.find(json_line).unwrap();
    assert_header(&stdout[..json_position]);
    assert!(stdout[..json_position].contains(row(&stdout, "0")));
    let info: Value = serde_json::from_str(json_line).unwrap();
    assert_eq!(info["formats"].as_array().unwrap().len(), 2);
    assert_eq!(info["format_id"], "1");
    assert!(
        info["requested_formats"]
            .as_array()
            .is_none_or(Vec::is_empty)
    );
    assert!(
        !stdout.contains("[info] Available formats for "),
        "{stdout}"
    );
    assert_eq!(server.count("/high.mp4"), 1);
    for mode in ["-j", "-s"] {
        let output = server.cli("/page.html", &["-F", mode, "-f", "999"]);
        assert!(!output.status.success());
        // Upstream lists formats before selection fails; no JSON follows.
        // The caption is a screen message, suppressed by -j's quiet mode.
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_header(&stdout);
        assert!(row(&stdout, "1").contains("1920x1080"), "{stdout}");
        assert_eq!(
            stdout.contains("[info] Available formats for page-1:"),
            mode == "-s",
            "{stdout}"
        );
        assert!(!stdout.contains('{'), "{stdout}");
        assert!(
            String::from_utf8_lossy(&output.stderr)
                .contains(": Requested format is not available. Use --list-formats")
        );
    }
}

#[test]
fn separate_hls_listing_never_inspects_ffmpeg_or_fetches_keys_initialization_or_segments() {
    let server = Server::new();
    let stdout = success(&server.cli(
        "/master.m3u8",
        &[
            "-F",
            "--ffmpeg-location",
            "/nonexistent-list-formats-ffmpeg",
        ],
    ));
    assert_header(&stdout);
    assert!(row(&stdout, "1000").contains("1920x1080"));
    assert!(row(&stdout, "audio-English").contains("en"));
    // Extraction reads the master and both media playlists; upstream's
    // generic "Checking m3u8 live status" then refetches the first
    // `m3u8_native` format (audio renditions precede variants).
    let extraction = ["/master.m3u8", "/video.m3u8", "/audio.m3u8", "/audio.m3u8"];
    assert_eq!(*server.requests.lock().unwrap(), extraction);
    server.assert_no_resources();
    let combined = success(&server.cli(
        "/master.m3u8",
        &[
            "-F",
            "-j",
            "-f",
            "bv+ba",
            "--ffmpeg-location",
            "/nonexistent-list-formats-ffmpeg",
        ],
    ));
    let info: Value =
        serde_json::from_str(combined.lines().find(|line| line.starts_with('{')).unwrap()).unwrap();
    assert_eq!(info["format_id"], "1000+audio-English");
    assert_eq!(info["requested_formats"].as_array().unwrap().len(), 2);
    assert_eq!(*server.requests.lock().unwrap(), extraction.repeat(2));
    server.assert_no_resources();
}

#[test]
fn separate_dash_listing_reads_only_the_saved_mpd_and_combined_json_keeps_fragment_plans() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args([
            "-F",
            "--ffmpeg-location",
            "/nonexistent-dash-listing-ffmpeg",
            "-o",
            "%(unsupported)s",
        ])
        .arg(server.url("/manifest.mpd"))
        .current_dir(directory.path())
        .output()
        .unwrap();
    let table = success(&output);
    assert_header(&table);
    let video = row(&table, "0");
    assert!(
        video.contains("mp4") && video.contains("32x32") && video.contains("video only"),
        "{video}"
    );
    let audio = row(&table, "1");
    assert!(
        audio.contains("m4a") && audio.contains("audio only"),
        "{audio}"
    );
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    assert_eq!(*server.requests.lock().unwrap(), vec!["/manifest.mpd"]);

    let combined = success(&server.cli(
        "/manifest.mpd",
        &[
            "-F",
            "-j",
            "-f",
            "0+1",
            "--ffmpeg-location",
            "/nonexistent-dash-listing-ffmpeg",
        ],
    ));
    let info: Value =
        serde_json::from_str(combined.lines().find(|line| line.starts_with('{')).unwrap()).unwrap();
    assert_eq!(info["format_id"], "0+1");
    assert_eq!(info["requested_formats"].as_array().unwrap().len(), 2);
    assert_eq!(
        info["requested_formats"][0]["fragments"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    assert_eq!(
        info["requested_formats"][1]["fragments"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
    assert_eq!(
        *server.requests.lock().unwrap(),
        vec!["/manifest.mpd", "/manifest.mpd"]
    );
}

#[test]
fn library_unselected_extraction_retains_separate_streams_before_default_selection() {
    let server = Server::new();
    let dl = YoutubeDl::new().unwrap();
    let info = dl
        .extract_result_unselected(&server.url("/master.m3u8"))
        .unwrap()
        .into_video()
        .unwrap();
    // Upstream lists audio renditions before variants.
    assert_eq!(info.formats.len(), 2);
    assert_eq!(info.formats[0].stream_kind, StreamKind::AudioOnly);
    assert_eq!(info.formats[1].stream_kind, StreamKind::VideoOnly);
    assert!(info.format_id.is_none());
    assert!(info.requested_formats.is_empty());
    // Upstream's no-merger default `best/bestvideo+bestaudio` selects the pair.
    let selected = dl.extract_info(&server.url("/master.m3u8")).unwrap();
    assert_eq!(selected.requested_formats.len(), 2);
    server.assert_no_resources();
}

#[test]
fn playlist_listing_is_ordered_and_failed_later_entries_produce_no_output() {
    let server = Server::new();
    let stdout = success(&server.cli("/playlist.html", &["-F"]));
    assert_eq!(
        stdout
            .lines()
            .filter(|line| line.split_whitespace().next() == Some("ID"))
            .count(),
        2
    );
    assert!(stdout.find("640x360").unwrap() < stdout.find("1920x1080").unwrap());
    assert_eq!(server.count("/low.mp4"), 1);
    assert_eq!(server.count("/high.mp4"), 1);
    let failed = server.cli("/invalid-playlist.html", &["-F", "-j"]);
    assert!(!failed.status.success());
    assert!(failed.stdout.is_empty());
    assert_eq!(server.count("/missing.mp4"), 1);
}

#[test]
fn malformed_selector_fails_before_any_network_or_listing() {
    let server = Server::new();
    let output = server.cli("/page.html", &["-F", "-f", "best/best[height>abc]"]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(server.requests.lock().unwrap().is_empty());
}

#[test]
fn listing_continues_to_later_urls_after_an_extraction_failure() {
    let server = Server::new();
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .arg("-F")
        .arg(server.url("/missing.html"))
        .arg(server.url("/page.html"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_header(&stdout);
    assert!(row(&stdout, "1").contains("1920x1080"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("ERROR:"));
    assert_eq!(server.count("/missing.html"), 1);
    assert_eq!(server.count("/high.mp4"), 1);
}

#[test]
fn direct_results_list_their_content_type_format_id() {
    let server = Server::new();
    let stdout = success(&server.cli("/low.mp4", &["-F", "-j"]));
    assert_header(&stdout);
    // Upstream names the single direct format by its Content-Type subtype.
    assert!(row(&stdout, "mp4").contains("mp4"));
    let info: Value =
        serde_json::from_str(stdout.lines().find(|line| line.starts_with('{')).unwrap()).unwrap();
    assert_eq!(info["format_id"], "mp4");
    assert_eq!(info["formats"].as_array().map(Vec::len), Some(1));
    assert_eq!(server.count("/low.mp4"), 1);
}

#[test]
fn transient_listing_retries_complete_extraction_before_printing_once() {
    let server = Server::new();
    let stdout = success(&server.cli("/retry.html", &["-F", "--extractor-retries", "1"]));
    assert_header(&stdout);
    assert_eq!(
        stdout
            .lines()
            .filter(|line| line.split_whitespace().next() == Some("ID"))
            .count(),
        1
    );
    assert_eq!(server.count("/retry.html"), 2);
    assert_eq!(server.count("/low.mp4"), 1);
    assert_eq!(server.count("/high.mp4"), 1);
    let library_server = Server::new();
    let info = YoutubeDl::new()
        .unwrap()
        .extract_result_unselected_with_options(
            &library_server.url("/retry.html"),
            ExtractionOptions { retries: 1 },
        )
        .unwrap()
        .into_video()
        .unwrap();
    assert_eq!(info.formats.len(), 2);
    assert!(info.format_id.is_none());
    assert_eq!(library_server.count("/retry.html"), 2);
}

#[test]
fn shared_upstream_listing_fixture_runs_through_unselected_orchestration_and_native_renderer() {
    struct SavedFixture;
    impl yt_dlp_rs::extractor::Extractor for SavedFixture {
        fn key(&self) -> &'static str {
            "SavedListingFixture"
        }
        fn extract(
            &self,
            _client: &reqwest::blocking::Client,
            url: &url::Url,
        ) -> yt_dlp_rs::Result<Option<yt_dlp_rs::VideoInfo>> {
            assert_eq!(url.as_str(), "https://listing.invalid/saved");
            Ok(Some(
                serde_json::from_str(include_str!("fixtures/listing/input.json")).unwrap(),
            ))
        }
    }
    let expected: Value =
        serde_json::from_str(include_str!("fixtures/listing/expected.json")).unwrap();
    let mut dl = YoutubeDl::new().unwrap();
    dl.register_extractor(Box::new(SavedFixture));
    let info = dl
        .extract_result_unselected("https://listing.invalid/saved")
        .unwrap()
        .into_video()
        .unwrap();
    assert!(info.format_id.is_none());
    assert!(info.requested_formats.is_empty());
    let ids: Vec<_> = info
        .formats
        .iter()
        .map(|format| format.format_id.as_str())
        .collect();
    // Processing sorts the declaration-order fixture like pinned upstream
    // `process_ie_result` does (a128 lacks a filesize, so it ranks below a64).
    let sorted_ids = ["a128", "a64", "v360", "v720"];
    assert_eq!(ids, sorted_ids);
    let mut declaration_ids: Vec<_> = expected["declaration_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| id.as_str().unwrap())
        .collect();
    declaration_ids.sort_unstable();
    let mut sorted = sorted_ids.to_vec();
    sorted.sort_unstable();
    assert_eq!(sorted, declaration_ids);
    let actual = serde_json::to_value(&info).unwrap();
    assert_eq!(
        info.formats.len(),
        expected["formats"].as_array().unwrap().len()
    );
    for expected in expected["formats"].as_array().unwrap() {
        let actual = actual["formats"]
            .as_array()
            .unwrap()
            .iter()
            .find(|format| format["format_id"] == expected["format_id"])
            .unwrap();
        for (field, value) in expected.as_object().unwrap() {
            if value.is_number() {
                assert_eq!(
                    actual[field].as_f64(),
                    value.as_f64(),
                    "{} {field}",
                    actual["format_id"]
                );
            } else {
                assert_eq!(&actual[field], value, "{} {field}", actual["format_id"]);
            }
        }
    }
    // `table.txt` is the exact pinned upstream `-F` output for `input.json`
    // (full `process_ie_result` with `listformats`; see the fixture README).
    let table = yt_dlp_rs::output::render_formats(&info);
    let upstream_table = include_str!("fixtures/listing/table.txt");
    assert_eq!(
        table,
        format!("[info] Available formats for listing-fixture:\n{upstream_table}")
    );
    let rendered_ids: Vec<_> = upstream_table
        .lines()
        .skip(2)
        .map(|line| line.split_whitespace().next().unwrap())
        .collect();
    assert_eq!(rendered_ids, sorted_ids);
    // Without a merger the default falls back to the best pair in sorted
    // order (a64 is the last audio-only format).
    assert_eq!(
        dl.extract_info("https://listing.invalid/saved")
            .unwrap()
            .format_id
            .as_deref(),
        Some("v720+a64")
    );
    let selected = dl
        .extract_info_with_format("https://listing.invalid/saved", Some("v720+a64"))
        .unwrap();
    assert_eq!(selected.format_id.as_deref(), Some("v720+a64"));
    assert_eq!(selected.requested_formats.len(), 2);
    assert_eq!(selected.height, Some(720));
    assert_eq!(selected.ext, "mp4");
    assert_eq!(selected.formats.len(), info.formats.len());
}

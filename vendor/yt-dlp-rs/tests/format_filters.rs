//! Local HTTP and CLI coverage for native format predicate selection.
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
use yt_dlp_rs::YoutubeDl;

const FORMATS: &str = "<title>Filters</title><video><source src='/low.mp4' type='video/mp4' width='640' height='360'><source src='/medium.mp4' type='video/mp4' width='1280' height='720'><source src='/high.mp4' type='video/mp4' width='1920' height='1080'><source src='/unknown.mp4' type='video/mp4'><source src='/clip.webm' type='video/webm' width='1280' height='720'></video>";
const MASTER: &str = "#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"main\",NAME=\"English/+Main\",LANGUAGE=\"en\",DEFAULT=YES,AUTOSELECT=YES,URI=\"english.m3u8\"\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"main\",NAME=\"Norwegian\",LANGUAGE=\"nb\",URI=\"norwegian.m3u8\"\n#EXT-X-STREAM-INF:BANDWIDTH=500000,RESOLUTION=1280x720,CODECS=\"avc1.4,mp4a.40.2\",AUDIO=\"main\"\nvideo.m3u8\n";
/// Extracting `MASTER` reads every media playlist, then upstream generic's
/// "Checking m3u8 live status" refetches the first `m3u8_native` format
/// (audio renditions precede variants). Segments and keys are never fetched.
const MASTER_EXTRACTION: [&str; 5] = [
    "/master.m3u8",
    "/video.m3u8",
    "/english.m3u8",
    "/norwegian.m3u8",
    "/english.m3u8",
];
const MEDIA: &str =
    "#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:1,\nsegment.m4s\n#EXT-X-ENDLIST\n";

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
                        Ok(length) => request.extend_from_slice(&buffer[..length]),
                    }
                }
                if !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    continue;
                }
                let request = String::from_utf8(request).unwrap();
                let path = request.split_whitespace().nth(1).unwrap().to_owned();
                record.lock().unwrap().push(path.clone());
                let (status, mime, body) = match path.as_str() {
                    "/formats.html" => (200, "text/html", FORMATS),
                    "/playlist.html" => (
                        200,
                        "text/html",
                        "<title>Atomic filters</title><video><source src='/low.mp4' height='360'><source src='/medium.mp4' height='720'></video><video><source src='/high.mp4' height='1080'></video>",
                    ),
                    "/low.mp4" => (200, "video/mp4", "low"),
                    "/medium.mp4" => (200, "video/mp4", "medium"),
                    "/high.mp4" => (200, "video/mp4", "high"),
                    "/unknown.mp4" => (200, "video/mp4", "unknown"),
                    "/clip.webm" => (200, "video/webm", "webm"),
                    "/master.m3u8" => (200, "application/vnd.apple.mpegurl", MASTER),
                    "/video.m3u8" | "/english.m3u8" | "/norwegian.m3u8" => {
                        (200, "application/vnd.apple.mpegurl", MEDIA)
                    }
                    _ => (404, "text/plain", "unexpected request"),
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} Fixture\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
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
    fn json(&self, path: &str, selector: &str) -> Value {
        let output = self.cli(path, &["-j", "-s", "-f", selector]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
    fn assert_one_probe_per_progressive_source(&self) {
        for path in [
            "/low.mp4",
            "/medium.mp4",
            "/high.mp4",
            "/unknown.mp4",
            "/clip.webm",
        ] {
            assert_eq!(self.count(path), 1, "{path}");
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
    }
}

/// Without `-j`, stdout holds only bracketed screen lines: no JSON was printed.
fn only_screen_lines(output: &Output) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout
            .lines()
            .all(|line| line.trim_start_matches('\r').starts_with('[')),
        "{stdout}"
    );
}

/// Runs `selector` against the two-entry playlist with `-j -s` and as a
/// download. Like upstream `process_video_result`, entries are selected one
/// at a time: playlist-1 resolves to `format_id` (served from `path`) and is
/// printed/downloaded, playlist-2 (only 1080p) fails with upstream's error,
/// and the run exits 1.
fn assert_second_playlist_entry_fails_alone(
    server: &Server,
    selector: &str,
    format_id: &str,
    height: u64,
    path: &str,
) {
    let directory = tempfile::tempdir().unwrap();
    let template = directory.path().join("%(id)s.%(ext)s");
    for mode in [vec!["-j", "-s"], Vec::new()] {
        let json = !mode.is_empty();
        let mut args = mode;
        args.extend(["-f", selector, "-o", template.to_str().unwrap()]);
        let output = server.cli("/playlist.html", &args);
        assert_eq!(output.status.code(), Some(1), "{selector}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(
            stderr
                .lines()
                .filter(|line| line.starts_with("ERROR:"))
                .collect::<Vec<_>>(),
            [
                "ERROR: [html5] playlist-2: Requested format is not available. Use --list-formats for a list of available formats"
            ],
            "{selector}: {stderr}"
        );
        assert!(!stderr.contains("format selection failed"), "{stderr}");
        if json {
            let lines: Vec<Value> = String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            assert_eq!(lines.len(), 1, "{selector}");
            assert_eq!(lines[0]["id"], "playlist-1");
            assert_eq!(lines[0]["playlist_index"], 1);
            assert_eq!(lines[0]["format_id"], format_id);
            assert_eq!(lines[0]["height"], height);
            assert_eq!(lines[0]["url"], server.url(path));
            assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
        } else {
            only_screen_lines(&output);
            let body = path.trim_start_matches('/').trim_end_matches(".mp4");
            assert_eq!(
                std::fs::read(directory.path().join("playlist-1.mp4")).unwrap(),
                body.as_bytes(),
                "{selector}"
            );
            assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
        }
    }
    // Two probes per source (one per run) plus the single playlist-1 download.
    for source in ["/low.mp4", "/medium.mp4", "/high.mp4"] {
        let expected = 2 + usize::from(source == path);
        assert_eq!(server.count(source), expected, "{selector}: {source}");
    }
}

#[test]
fn cli_numeric_and_extension_filters_select_without_downloading() {
    let server = Server::new();
    // Upstream numbers HTML5 sources after FormatSorter order (worst first).
    let info = server.json("/formats.html", "best[height<=720][width>=1000][ext=mp4]");
    assert_eq!(info["format_id"], "3");
    assert_eq!(info["height"], 720);
    assert_eq!(info["width"], 1280);
    assert!(info["url"].as_str().unwrap().ends_with("/medium.mp4"));
    assert_eq!(info["formats"].as_array().unwrap().len(), 5);
    server.assert_one_probe_per_progressive_source();
}

#[test]
fn actual_download_uses_only_the_filtered_source_after_all_probes() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("chosen.mp4");
    let output = server.cli(
        "/formats.html",
        &[
            "-f",
            "best[height<=720][ext=mp4]",
            "-o",
            target.to_str().unwrap(),
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(std::fs::read(target).unwrap(), b"medium");
    assert_eq!(server.count("/medium.mp4"), 2);
    for path in ["/low.mp4", "/high.mp4", "/unknown.mp4", "/clip.webm"] {
        assert_eq!(
            server.count(path),
            1,
            "unselected source {path} was downloaded"
        );
    }
}

#[test]
fn unknown_numeric_values_require_explicit_inclusion() {
    let server = Server::new();
    let info = server.json("/formats.html", "best[height<=?100][ext=mp4]");
    assert_eq!(info["format_id"], "0");
    // Upstream's `<source>` formats carry `height: None`.
    assert!(info["height"].is_null());
    let output = server.cli(
        "/formats.html",
        &["-j", "-s", "-f", "best[height<=100][ext=mp4]"],
    );
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}

#[test]
fn quoted_filter_delimiters_do_not_split_hls_pair_or_fallback_expression() {
    let server = Server::new();
    let info = server.json(
        "/master.m3u8",
        "bestvideo[height<=720]+bestaudio[format_note='English/+Main'][language=en]/best",
    );
    assert_eq!(info["format_id"], "500+main-English__Main");
    assert_eq!(info["requested_formats"][1]["language"], "en");
    assert_eq!(info["requested_formats"][1]["format_note"], "English/+Main");
    // Like upstream, HLS formats carry no fragments.
    assert!(info["requested_formats"][0].get("fragments").is_none());
    assert_eq!(server.count("/segment.m4s"), 0);
    assert_eq!(*server.requests.lock().unwrap(), MASTER_EXTRACTION);
}

#[test]
fn invalid_predicates_fail_even_after_a_valid_fallback_branch() {
    let server = Server::new();
    // Upstream builds every filter at startup; unknown fields only match
    // nothing, and evaluation errors need a reached, mistyped value.
    for selector in [
        "best/best[height<=]",
        "best/best[height>abc]",
        "best/best[ext='mp4]",
    ] {
        let output = server.cli("/formats.html", &["-j", "-s", "-f", selector]);
        assert!(!output.status.success(), "{selector}");
        assert!(output.stdout.is_empty(), "{selector}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("Invalid "),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn playlist_filter_failure_is_per_entry_for_json_and_downloads() {
    assert_second_playlist_entry_fails_alone(
        &Server::new(),
        "best[height<=720]",
        "1",
        720,
        "/medium.mp4",
    );
}

#[test]
fn direct_result_filters_select_the_single_upstream_format() {
    let server = Server::new();
    let info = YoutubeDl::new()
        .unwrap()
        .extract_info_with_format(
            &server.url("/medium.mp4"),
            Some("best[filesize=?6][ext=mp4]"),
        )
        .unwrap();
    // Upstream names the direct format by its Content-Type subtype and, like
    // upstream, leaves its filesize unknown at extraction time.
    assert_eq!(info.formats.len(), 1);
    assert_eq!(info.format_id.as_deref(), Some("mp4"));
    assert_eq!(info.filesize, None);
    // Checked against pinned upstream: an unknown size fails a plain size
    // comparison, and only the `?` suffix lets the fallback branch match.
    let fallback = server.json(
        "/medium.mp4",
        "best[filesize<6]/worst[filesize>=?6][ext=mp4]",
    );
    assert_eq!(fallback["format_id"], "mp4");
    assert!(fallback["filesize"].is_null());
    for selector in [
        "best[filesize=6][ext=mp4]",
        "best[filesize<6]/worst[filesize>=6][ext=mp4]",
    ] {
        let output = server.cli("/medium.mp4", &["-j", "-s", "-f", selector]);
        assert!(!output.status.success(), "{selector}");
        assert!(output.stdout.is_empty());
    }
    let output = server.cli("/medium.mp4", &["-j", "-s", "-f", "best[ext=webm]"]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}

#[test]
fn text_predicates_chain_and_literal_id_filters_can_fall_back() {
    let server = Server::new();
    let info = server.json(
        "/formats.html",
        "2[height<720]/best[ext^=m][ext$=4][ext*=p][ext!=webm][height<1080]",
    );
    assert_eq!(info["format_id"], "3");
    assert_eq!(info["ext"], "mp4");
    server.assert_one_probe_per_progressive_source();
}

#[test]
fn grouped_filters_narrow_candidates_before_nested_fallbacks_and_ranking() {
    let server = Server::new();
    for selector in [
        "(b/0)[height<=720][ext=mp4]",
        "((4/(3/0)))[height<=720][ext=mp4]",
    ] {
        let info = server.json("/formats.html", selector);
        assert_eq!(info["format_id"], "3", "{selector}");
        assert_eq!(info["height"], 720);
    }
    for path in [
        "/low.mp4",
        "/medium.mp4",
        "/high.mp4",
        "/unknown.mp4",
        "/clip.webm",
    ] {
        assert_eq!(
            server.count(path),
            2,
            "only extraction probes expected for {path}"
        );
    }
}

#[test]
fn bare_filter_download_and_short_alias_json_use_existing_media_selection() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("bare.mp4");
    let output = server.cli(
        "/formats.html",
        &[
            "-f",
            "[height<=720][ext=mp4]",
            "-o",
            target.to_str().unwrap(),
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(std::fs::read(target).unwrap(), b"medium");
    assert_eq!(server.count("/medium.mp4"), 2);
    let best = server.json("/formats.html", "b[ext=mp4]");
    assert_eq!(best["format_id"], "4");
    let worst = server.json("/formats.html", "w[height>=360][ext=mp4]");
    assert_eq!(worst["format_id"], "1");
    assert_eq!(server.count("/medium.mp4"), 4);
    assert_eq!(server.count("/high.mp4"), 3);
}

#[test]
fn nested_hls_pair_groups_filter_audio_before_fallback_without_launching_ffmpeg() {
    let server = Server::new();
    let output = server.cli(
        "/master.m3u8",
        &[
            "-j",
            "-s",
            "--ffmpeg-location",
            "/nonexistent-filter-test-ffmpeg",
            "-f",
            "((bv[height<100]/bv)+(ba[language=xx]/wa)[format_note='English/+Main'])/b",
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let info: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(info["format_id"], "500+main-English__Main");
    assert_eq!(info["requested_formats"][1]["language"], "en");
    assert_eq!(info["requested_formats"][0]["height"], 720);
    assert!(info["requested_formats"][1].get("fragments").is_none());
    assert_eq!(*server.requests.lock().unwrap(), MASTER_EXTRACTION);
    assert_eq!(server.count("/init.mp4"), 0);
    assert_eq!(server.count("/segment.m4s"), 0);
}

#[test]
fn malformed_grouped_later_branches_fail_before_json_or_downloads() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("never.mp4");
    // Unbalanced groups fail tokenization; upstream accepts `()`, `(w/)`,
    // multi-stream merges and `,` inside groups.
    for selector in ["b/(w[ext=mp4]", "b/(w", "b/)(", "b/w["] {
        let output = server.cli("/formats.html", &["-j", "-s", "-f", selector]);
        assert!(!output.status.success(), "{selector}");
        assert!(output.stdout.is_empty(), "{selector}");
    }
    let output = server.cli(
        "/formats.html",
        &["-f", "b/(w[ext=mp4]", "-o", target.to_str().unwrap()],
    );
    assert!(!output.status.success());
    assert!(!target.exists());
}

#[test]
fn grouped_playlist_selection_fails_per_entry() {
    // `0` names the only (1080p) format of playlist-2, which the outer
    // filter still rejects.
    assert_second_playlist_entry_fails_alone(
        &Server::new(),
        "((b/0)[height<=720])",
        "1",
        720,
        "/medium.mp4",
    );
}

#[test]
fn grouped_bare_filters_preserve_direct_metadata_and_format() {
    let server = Server::new();
    // Checked against pinned upstream: the direct format's filesize is
    // unknown, so size filters match it only with the `?` suffix.
    let info = server.json("/medium.mp4", "(b[ext=webm]/[filesize>=?6])[ext=mp4]");
    assert_eq!(info["format_id"], "mp4");
    assert!(info["filesize"].is_null());
    assert_eq!(info["ext"], "mp4");
    assert_eq!(info["formats"].as_array().map(Vec::len), Some(1));
    let output = server.json("/medium.mp4", "(b/w)[filesize<?6]");
    assert_eq!(output["format_id"], "mp4");
    for selector in ["(b[ext=webm]/[filesize>=6])[ext=mp4]", "(b/w)[filesize<6]"] {
        let output = server.cli("/medium.mp4", &["-j", "-s", "-f", selector]);
        assert!(!output.status.success(), "{selector}");
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn indexed_best_and_worst_rank_after_filters_and_download_the_selected_bytes() {
    let server = Server::new();
    let second_best = server.json("/formats.html", "best.2[height>=360][ext=mp4]");
    assert_eq!(second_best["format_id"], "3");
    assert_eq!(second_best["height"], 720);
    let second_worst = server.json("/formats.html", "worst.2[height>=360][ext=mp4]");
    assert_eq!(second_worst["format_id"], "3");
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("second.mp4");
    let output = server.cli(
        "/formats.html",
        &[
            "-f",
            "b.2[height>=360][ext=mp4]",
            "-o",
            target.to_str().unwrap(),
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(std::fs::read(target).unwrap(), b"medium");
    assert_eq!(server.count("/medium.mp4"), 4);
    assert_eq!(server.count("/low.mp4"), 3);
    assert_eq!(server.count("/high.mp4"), 3);
}

#[test]
fn indexed_hls_audio_follows_upstream_sort_order_without_merger_launch() {
    let server = Server::new();
    // Checked with pinned upstream: the renditions tie until the format ID,
    // so `main-Norwegian` sorts last (best) despite DEFAULT=YES on English.
    for (index, expected_id, language) in
        [(1, "main-Norwegian", "nb"), (2, "main-English__Main", "en")]
    {
        let selector = format!("bv.1+ba.{index}");
        let output = server.cli(
            "/master.m3u8",
            &[
                "-j",
                "-s",
                "--ffmpeg-location",
                "/nonexistent-index-test-ffmpeg",
                "-f",
                &selector,
            ],
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let info: Value = serde_json::from_slice(&output.stdout).unwrap();
        // Upstream sanitizes `/` and `+` in rendition names to `_`.
        assert_eq!(info["format_id"], format!("500+{expected_id}"));
        assert_eq!(info["requested_formats"][1]["language"], language);
    }
    assert_eq!(server.count("/init.mp4"), 0);
    assert_eq!(server.count("/segment.m4s"), 0);
    assert_eq!(
        *server.requests.lock().unwrap(),
        MASTER_EXTRACTION.repeat(2)
    );
}

#[test]
fn indexed_out_of_range_can_fall_back_inside_filtered_groups() {
    let server = Server::new();
    let info = server.json("/formats.html", "(b.4/b.2)[height>=360][ext=mp4]");
    assert_eq!(info["format_id"], "3");
    let unavailable = server.cli(
        "/formats.html",
        &["-j", "-s", "-f", "b.4[height>=360][ext=mp4]"],
    );
    assert!(!unavailable.status.success());
    assert!(unavailable.stdout.is_empty());
    let filtered = server.json("/formats.html", "(b.2/b.1)[height<=360][ext=mp4]");
    assert_eq!(filtered["format_id"], "1");
}

#[test]
fn indexed_playlist_failure_is_per_entry_for_json_and_downloads() {
    // playlist-1 has a second-best format; playlist-2 has neither `b.2` nor `w.2`.
    assert_second_playlist_entry_fails_alone(
        &Server::new(),
        "(b.2/w.2)[height>=360]",
        "0",
        360,
        "/low.mp4",
    );
}

#[test]
fn direct_metadata_exposes_only_the_first_index_of_its_single_format() {
    let server = Server::new();
    // Checked against pinned upstream, including the unknown-size branch.
    for selector in ["b.1[ext=mp4]", "b.2/w.1", "(b.2/[filesize=?6])"] {
        let info = server.json("/medium.mp4", selector);
        assert!(info["filesize"].is_null());
        assert_eq!(info["format_id"], "mp4");
    }
    for selector in ["b.2", "(b.2/[filesize=6])"] {
        let output = server.cli("/medium.mp4", &["-j", "-s", "-f", selector]);
        assert!(!output.status.success(), "{selector}");
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn format_sort_options_reorder_formats_like_upstream() {
    let server = Server::new();
    // Each expected order was checked with pinned upstream (FormatSorter).
    let order = |args: &[&str]| -> Vec<String> {
        let output = server.cli("/formats.html", &[&["-j"], args].concat());
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let info: Value = serde_json::from_slice(&output.stdout).unwrap();
        info["formats"]
            .as_array()
            .unwrap()
            .iter()
            .map(|format| {
                format["url"]
                    .as_str()
                    .unwrap()
                    .rsplit('/')
                    .next()
                    .unwrap()
                    .to_owned()
            })
            .collect()
    };
    assert_eq!(
        order(&[]),
        [
            "unknown.mp4",
            "low.mp4",
            "clip.webm",
            "medium.mp4",
            "high.mp4"
        ]
    );
    assert_eq!(
        order(&["-S", "+res"]),
        [
            "high.mp4",
            "clip.webm",
            "medium.mp4",
            "low.mp4",
            "unknown.mp4"
        ]
    );
    assert_eq!(
        order(&["-S", "res~700"]),
        [
            "unknown.mp4",
            "high.mp4",
            "low.mp4",
            "clip.webm",
            "medium.mp4"
        ]
    );
    // Later -S values are prepended: `ext` wins over `res,+size`.
    assert_eq!(
        order(&["-S", "res,+size", "-S", "ext"]),
        [
            "clip.webm",
            "unknown.mp4",
            "low.mp4",
            "medium.mp4",
            "high.mp4"
        ]
    );
    assert_eq!(
        order(&["-S", "ext", "--format-sort-reset", "-S", "+res"]),
        [
            "high.mp4",
            "clip.webm",
            "medium.mp4",
            "low.mp4",
            "unknown.mp4"
        ]
    );
    let output = server.cli("/formats.html", &["-j", "-S", "res!"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .ends_with("error: invalid format sorting \"res!\" given\n")
    );
}

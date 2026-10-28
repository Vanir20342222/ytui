//! Native HTML track discovery, subtitle selection and lazy sidecar HTTP transport.
use serde_json::Value;
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    process::{Command, Output},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
const VTT: &str = "WEBVTT\n\n00:00.000 --> 00:01.000\nCaption é\n";
const MASTER: &str = "#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"Main\",URI=\"audio.m3u8\"\n#EXT-X-STREAM-INF:BANDWIDTH=100000,RESOLUTION=640x360,CODECS=\"avc1.4,mp4a.40.2\",AUDIO=\"a\"\nvideo.m3u8\n";
const MEDIA: &str = "#EXTM3U\n#EXTINF:1,\nsegment.ts\n#EXT-X-ENDLIST\n";
struct Server {
    origin: String,
    requests: Arc<Mutex<Vec<(String, String)>>>,
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
                    Ok(v) => v,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(e) => panic!("{e}"),
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 2048];
                while !request.windows(4).any(|v| v == b"\r\n\r\n") {
                    match stream.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => request.extend_from_slice(&buffer[..n]),
                    }
                }
                if !request.windows(4).any(|v| v == b"\r\n\r\n") {
                    continue;
                }
                let text = String::from_utf8(request).unwrap();
                let path = text.split_whitespace().nth(1).unwrap().to_owned();
                let count = {
                    let mut requests = record.lock().unwrap();
                    requests.push((path.clone(), text));
                    requests.iter().filter(|(p, _)| *p == path).count()
                };
                let (status, mime, body, extra) = match path.as_str() {
                    "/tracks.html" => (
                        200,
                        "text/html",
                        "<base href='/base/'><video src='/media.mp4'><track src=' one.VTT?x=1&amp;y=2 ' srclang='en' label='English'><track kind='captions' src='two.srt' lang='nb'><track kind='' src='three.ass' label='French'><track kind='metadata' src='ignored.vtt'><track kind='Captions' src='case.vtt'><div><track src='nested.vtt'></div></video>",
                        "",
                    ),
                    "/select.html" => (
                        200,
                        "text/html",
                        "<video src='/media.mp4'><track src='/en.vtt' srclang='en'><track src='/en.srt' srclang='en'><track src='/en-us.vtt' srclang='en-US'><track src='/nb.vtt' srclang='nb'><track src='/fr.vtt' srclang='fr'></video>",
                        "",
                    ),
                    "/prefix.html" => (
                        200,
                        "text/html",
                        "<video src='/media.mp4'><track src='/nb.vtt' srclang='nb'><track src='/en-us.vtt' srclang='en-US'></video>",
                        "",
                    ),
                    "/first.html" => (
                        200,
                        "text/html",
                        "<video src='/media.mp4'><track src='/nb.vtt' srclang='nb'><track src='/fr.vtt' srclang='fr'></video>",
                        "",
                    ),
                    "/playlist.html" => (
                        200,
                        "text/html",
                        "<video src='/media.mp4'><track src='/en.vtt' srclang='en'></video><video src='/media.mp4'><track src='/nb.vtt' srclang='nb'></video>",
                        "",
                    ),
                    "/separate.html" => (
                        200,
                        "text/html",
                        "<video src='/master.m3u8'><track src='/en.vtt' srclang='en'></video>",
                        "",
                    ),
                    "/retry.html" => (
                        200,
                        "text/html",
                        "<video src='/media.mp4'><track src='/retry.vtt' srclang='en'></video>",
                        "",
                    ),
                    "/ok.html" => (
                        200,
                        "text/html",
                        "<video src='/media.mp4'><track src='/nb.vtt' srclang='nb'></video>",
                        "",
                    ),
                    "/failing-subs.html" => (
                        200,
                        "text/html",
                        "<video src='/media.mp4'><track src='/en.vtt' srclang='en'><track src='/missing.vtt' srclang='nb'></video>",
                        "",
                    ),
                    "/compressed.html" => (
                        200,
                        "text/html",
                        "<video src='/media.mp4'><track src='/en.vtt' srclang='en'><track src='/compressed.vtt' srclang='nb'></video>",
                        "",
                    ),
                    "/changing.html" => (
                        200,
                        "text/html",
                        "<video src='/changing.mp4'><track src='/en.vtt' srclang='en'></video>",
                        "",
                    ),
                    "/unknown.html" => (
                        200,
                        "text/html",
                        "<video src='/media.mp4'><track src='/en.vtt'></video>",
                        "",
                    ),
                    "/collision.html" => (
                        200,
                        "text/html",
                        "<video src='/media.mp4'><track src='/en.vtt' srclang='en/us'><track src='/nb.vtt' srclang='en?us'></video>",
                        "",
                    ),
                    "/unsafe-lang.html" => (
                        200,
                        "text/html",
                        "<video src='/media.mp4'><track src='/en.vtt' srclang='../x'></video>",
                        "",
                    ),
                    "/opaque.html" => (
                        200,
                        "text/html",
                        "<video src='/media.mp4'><track src='/opaque' srclang='en'></video>",
                        "",
                    ),
                    "/invalid-track.html" => (
                        200,
                        "text/html",
                        "<video src='/media.mp4'><track src='/en.vtt' srclang='en'><track src='blob:https://example.invalid/sub' srclang='nb'></video>",
                        "",
                    ),
                    "/master.m3u8" => (200, "application/vnd.apple.mpegurl", MASTER, ""),
                    "/video.m3u8" | "/audio.m3u8" => {
                        (200, "application/vnd.apple.mpegurl", MEDIA, "")
                    }
                    "/media.mp4" => (200, "video/mp4", "media", ""),
                    "/changing.mp4" if count == 1 => (200, "video/mp4", "probe", ""),
                    "/retry.vtt" if count == 1 => {
                        (503, "text/plain", "retry", "Retry-After: 0\r\n")
                    }
                    "/compressed.vtt" => (200, "text/vtt", VTT, "Content-Encoding: gzip\r\n"),
                    "/en.srt" => (
                        200,
                        "application/x-subrip",
                        "1\n00:00:00,000 --> 00:00:01,000\nSRT\n",
                        "",
                    ),
                    "/en.vtt" | "/en-us.vtt" | "/nb.vtt" | "/fr.vtt" | "/retry.vtt" | "/opaque" => {
                        (200, "text/vtt", VTT, "")
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
    fn cli(&self, paths: &[&str], args: &[&str], directory: &Path) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"));
        command.args(args).current_dir(directory);
        for path in paths {
            command.arg(self.url(path));
        }
        command.output().unwrap()
    }
    fn count(&self, path: &str) -> usize {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(p, _)| p.as_str() == path)
            .count()
    }
    fn assert_no_subtitle_requests(&self) {
        for (path, _) in self.requests.lock().unwrap().iter() {
            assert!(
                path.ends_with(".html") || path.ends_with(".mp4") || path.ends_with(".m3u8"),
                "{path}"
            );
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
fn json(output: &Output) -> Value {
    serde_json::from_str(&success(output)).unwrap()
}

#[test]
fn direct_child_track_metadata_resolves_base_entities_extensions_and_playlist_context_without_fetches()
 {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let info = json(&server.cli(&["/tracks.html"], &["-j"], directory.path()));
    let subtitles = &info["subtitles"];
    assert_eq!(subtitles.as_object().unwrap().len(), 3);
    assert_eq!(subtitles["en"][0]["ext"], "vtt");
    assert_eq!(
        subtitles["en"][0]["url"],
        server.url("/base/one.VTT?x=1&y=2")
    );
    assert_eq!(subtitles["nb"][0]["ext"], "srt");
    assert_eq!(subtitles["French"][0]["ext"], "ass");
    assert!(info.get("requested_subtitles").is_none_or(Value::is_null));
    let selected = json(&server.cli(&["/playlist.html"], &["-j", "-I", "2"], directory.path()));
    assert!(selected["subtitles"].get("nb").is_some());
    assert!(selected["subtitles"].get("en").is_none());
    server.assert_no_subtitle_requests();
}

#[test]
fn language_and_extension_selection_is_visible_in_json_but_never_fetches_in_readonly_modes() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    for (path, language, ext) in [
        ("/select.html", "en", "srt"),
        ("/prefix.html", "en-US", "vtt"),
        ("/first.html", "nb", "vtt"),
    ] {
        let info = json(&server.cli(&[path], &["-j", "--write-subs"], directory.path()));
        assert_eq!(info["requested_subtitles"].as_object().unwrap().len(), 1);
        assert_eq!(info["requested_subtitles"][language]["ext"], ext);
    }
    let info = json(&server.cli(
        &["/select.html"],
        &[
            "-j",
            "--write-srt",
            "--sub-langs",
            " all , -nb , NB ",
            "--sub-format",
            "vtt/srt",
        ],
        directory.path(),
    ));
    assert_eq!(info["requested_subtitles"].as_object().unwrap().len(), 4);
    assert_eq!(info["requested_subtitles"]["en"]["ext"], "vtt");
    assert!(info["requested_subtitles"].get("nb").is_some());
    let repeated = success(&server.cli(
        &["/select.html"],
        &[
            "-j",
            "--write-subs",
            "--sub-langs",
            " fr ",
            "--srt-langs",
            "en",
        ],
        directory.path(),
    ));
    let info: Value = serde_json::from_str(&repeated).unwrap();
    assert_eq!(info["requested_subtitles"].as_object().unwrap().len(), 2);
    assert_eq!(info["requested_subtitles"]["fr"]["ext"], "vtt");
    assert_eq!(info["requested_subtitles"]["en"]["ext"], "srt");
    let requested = repeated.split_once("\"requested_subtitles\":").unwrap().1;
    assert!(requested.find("\"fr\":").unwrap() < requested.find("\"en\":").unwrap());
    let disabled = json(&server.cli(
        &["/select.html"],
        &["-j", "--write-subs", "--no-write-subs"],
        directory.path(),
    ));
    assert!(
        disabled
            .get("requested_subtitles")
            .is_none_or(Value::is_null)
    );
    let stdout = success(&server.cli(&["/select.html"], &["-s", "--write-subs"], directory.path()));
    // Upstream `process_subtitles` names the selected languages.
    assert!(stdout.contains(": Downloading subtitles: "), "{stdout}");
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    server.assert_no_subtitle_requests();
}

#[test]
fn list_subs_handles_separate_only_hls_without_selection_and_combined_modes_remain_lazy() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let table = success(&server.cli(
        &["/separate.html"],
        &["--list-subs", "-f", "999"],
        directory.path(),
    ));
    assert!(table.contains("en") && table.contains("vtt"), "{table}");
    let combined = success(&server.cli(
        &["/separate.html"],
        &["--list-subs", "-j", "-f", "bv+ba"],
        directory.path(),
    ));
    let info: Value =
        serde_json::from_str(combined.lines().find(|line| line.starts_with('{')).unwrap()).unwrap();
    assert_eq!(info["requested_formats"].as_array().unwrap().len(), 2);
    assert!(info["subtitles"].get("en").is_some());
    server.assert_no_subtitle_requests();
    assert_eq!(server.count("/segment.ts"), 0);
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn actual_sidecar_preserves_bytes_referrer_identity_retry_and_precedes_media_and_archive() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let archive = directory.path().join("archive.txt");
    success(&server.cli(
        &["/retry.html"],
        &[
            "--write-subs",
            "--retries",
            "1",
            "--download-archive",
            archive.to_str().unwrap(),
            "-o",
            "video.mp4",
        ],
        directory.path(),
    ));
    assert_eq!(
        std::fs::read(directory.path().join("video.en.vtt")).unwrap(),
        VTT.as_bytes()
    );
    assert_eq!(
        std::fs::read(directory.path().join("video.mp4")).unwrap(),
        b"media"
    );
    assert_eq!(
        std::fs::read_to_string(archive).unwrap(),
        "html5mediaembed retry-1\n"
    );
    let requests = server.requests.lock().unwrap();
    assert_eq!(
        requests.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>(),
        [
            "/retry.html",
            "/media.mp4",
            "/retry.vtt",
            "/retry.vtt",
            "/media.mp4"
        ]
    );
    for (_, headers) in requests.iter().filter(|(p, _)| p == "/retry.vtt") {
        let lower = headers.to_ascii_lowercase();
        assert!(lower.contains("accept-encoding: identity\r\n"));
        assert!(lower.contains(&format!("referer: {}\r\n", server.url("/retry.html"))));
    }
}

#[test]
fn skip_download_aliases_write_sidecars_without_media_download_or_archive() {
    for flag in ["--skip-download", "--no-download"] {
        let server = Server::new();
        let directory = tempfile::tempdir().unwrap();
        let archive = directory.path().join("archive.txt");
        success(&server.cli(
            &["/ok.html"],
            &[
                "--write-subs",
                flag,
                "--download-archive",
                archive.to_str().unwrap(),
                "-o",
                "video.mp4",
            ],
            directory.path(),
        ));
        assert_eq!(
            std::fs::read(directory.path().join("video.nb.vtt")).unwrap(),
            VTT.as_bytes()
        );
        assert!(!directory.path().join("video.mp4").exists());
        assert!(!archive.exists());
        assert_eq!(server.count("/media.mp4"), 1);
    }
}

#[test]
fn existing_sidecars_follow_overwrite_modes_and_existing_media_is_already_downloaded() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("video.custom.nb.vtt"), b"keep").unwrap();
    success(&server.cli(
        &["/ok.html"],
        &["--write-subs", "-w", "-o", "video.custom"],
        directory.path(),
    ));
    assert_eq!(
        std::fs::read(directory.path().join("video.custom.nb.vtt")).unwrap(),
        b"keep"
    );
    assert_eq!(
        std::fs::read(directory.path().join("video.custom")).unwrap(),
        b"media"
    );
    assert_eq!(server.count("/nb.vtt"), 0);
    // By default related files are overwritten.
    success(&server.cli(
        &["/ok.html"],
        &["--write-subs", "-o", "video.custom"],
        directory.path(),
    ));
    assert_eq!(
        std::fs::read(directory.path().join("video.custom.nb.vtt")).unwrap(),
        VTT.as_bytes()
    );
    assert_eq!(server.count("/nb.vtt"), 1);
    // Existing media is kept as already downloaded after sidecars are written.
    let existing = Server::new();
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("video.mp4"), b"existing").unwrap();
    let output = existing.cli(
        &["/ok.html"],
        &["--write-subs", "-o", "video.mp4"],
        directory.path(),
    );
    success(&output);
    assert!(String::from_utf8_lossy(&output.stdout).contains("has already been downloaded"));
    assert_eq!(
        std::fs::read(directory.path().join("video.mp4")).unwrap(),
        b"existing"
    );
    assert_eq!(
        std::fs::read(directory.path().join("video.nb.vtt")).unwrap(),
        VTT.as_bytes()
    );
    // --force-overwrites replaces the media.
    let output = existing.cli(
        &["/ok.html"],
        &["--force-overwrites", "-o", "video.mp4"],
        directory.path(),
    );
    success(&output);
    assert!(String::from_utf8_lossy(&output.stdout).contains("Deleting existing file video.mp4"));
    assert_eq!(
        std::fs::read(directory.path().join("video.mp4")).unwrap(),
        b"media"
    );
}

#[test]
fn sidecar_failure_aborts_media_and_archive_keeps_earlier_complete_sidecars_and_cleans_temps() {
    for path in ["/failing-subs.html", "/compressed.html"] {
        let server = Server::new();
        let directory = tempfile::tempdir().unwrap();
        let archive = directory.path().join("archive.txt");
        let output = server.cli(
            &[path],
            &[
                "--write-subs",
                "--sub-langs",
                "all",
                "--retries",
                "0",
                "--download-archive",
                archive.to_str().unwrap(),
                "-o",
                "video.mp4",
            ],
            directory.path(),
        );
        assert!(!output.status.success());
        if path == "/compressed.html" {
            let stderr = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
            assert!(
                stderr.contains("compressed") || stderr.contains("encoding"),
                "{stderr}"
            );
        }
        assert_eq!(
            std::fs::read(directory.path().join("video.en.vtt")).unwrap(),
            VTT.as_bytes()
        );
        assert!(!directory.path().join("video.mp4").exists());
        assert!(!archive.exists());
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
        assert_eq!(server.count("/media.mp4"), 1);
    }
}

#[test]
fn media_failure_keeps_subtitles_and_later_url_continues_without_archiving_failure() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let archive = directory.path().join("archive.txt");
    let output = server.cli(
        &["/changing.html", "/ok.html"],
        &[
            "--write-subs",
            "--retries",
            "0",
            "--download-archive",
            archive.to_str().unwrap(),
            "-o",
            "%(id)s.%(ext)s",
        ],
        directory.path(),
    );
    assert!(!output.status.success());
    assert_eq!(
        std::fs::read(directory.path().join("changing-1.en.vtt")).unwrap(),
        VTT.as_bytes()
    );
    assert!(!directory.path().join("changing-1.mp4").exists());
    assert_eq!(
        std::fs::read(directory.path().join("ok-1.nb.vtt")).unwrap(),
        VTT.as_bytes()
    );
    assert_eq!(
        std::fs::read(directory.path().join("ok-1.mp4")).unwrap(),
        b"media"
    );
    assert_eq!(
        std::fs::read_to_string(archive).unwrap(),
        "html5mediaembed ok-1\n"
    );
    // Like upstream, the media transfer failed before receiving data: no part file.
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 4);
}

#[test]
fn missing_language_is_preserved_as_null_metadata_but_cannot_be_written_under_a_guessed_tag() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let info = json(&server.cli(&["/unknown.html"], &["-j"], directory.path()));
    assert_eq!(info["subtitles"]["null"][0]["ext"], "vtt");
    for langs in [None, Some("all")] {
        let mut args = vec!["--write-subs", "--skip-download", "-o", "video.mp4"];
        if let Some(langs) = langs {
            args.extend(["--sub-langs", langs]);
        }
        let output = server.cli(&["/unknown.html"], &args, directory.path());
        assert!(!output.status.success());
    }
    assert_eq!(server.count("/en.vtt"), 0);
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn unsafe_language_paths_are_contained_and_colliding_sidecars_fail_before_any_io() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let output = server.cli(
        &["/collision.html"],
        &[
            "--write-subs",
            "--sub-langs",
            "all",
            "--skip-download",
            "-o",
            "video.mp4",
        ],
        directory.path(),
    );
    assert!(!output.status.success());
    assert_eq!(server.count("/en.vtt"), 0);
    assert_eq!(server.count("/nb.vtt"), 0);
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    success(&server.cli(
        &["/unsafe-lang.html"],
        &[
            "--write-subs",
            "--sub-langs",
            "all",
            "--skip-download",
            "-o",
            "video.mp4",
        ],
        directory.path(),
    ));
    assert_eq!(
        std::fs::read(directory.path().join("video._x.vtt")).unwrap(),
        VTT.as_bytes()
    );
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn unknown_extension_uses_unknown_video_and_unavailable_format_falls_back_with_warning() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let info = json(&server.cli(&["/opaque.html"], &["-j"], directory.path()));
    assert_eq!(info["subtitles"]["en"][0]["ext"], "unknown_video");
    let output = server.cli(
        &["/opaque.html"],
        &[
            "--write-subs",
            "--skip-download",
            "--sub-format",
            "ass/vtt",
            "-o",
            "video.mp4",
        ],
        directory.path(),
    );
    success(&output);
    assert!(String::from_utf8_lossy(&output.stderr).contains("WARNING:"));
    assert_eq!(
        std::fs::read(directory.path().join("video.en.unknown_video")).unwrap(),
        VTT.as_bytes()
    );
}

#[test]
fn unsupported_track_urls_and_language_regex_fail_explicitly_without_sidecar_or_download() {
    for (path, args) in [
        ("/invalid-track.html", vec!["--write-subs"]),
        ("/select.html", vec!["--write-subs", "--sub-langs", "en.*"]),
    ] {
        let server = Server::new();
        let directory = tempfile::tempdir().unwrap();
        let mut args = args;
        args.extend(["-o", "video.mp4"]);
        let output = server.cli(&[path], &args, directory.path());
        assert!(!output.status.success());
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
        server.assert_no_subtitle_requests();
        assert!(server.count("/media.mp4") <= 1);
    }
}

#[test]
fn library_subtitle_cancellation_at_request_or_publication_never_leaves_final_or_temporary_files() {
    use std::ops::ControlFlow;
    use yt_dlp_rs::{DownloadOptions, DownloadPhase, Error, YoutubeDl};
    for cancel_at in [DownloadPhase::Requesting, DownloadPhase::Publishing] {
        let server = Server::new();
        let directory = tempfile::tempdir().unwrap();
        let dl = YoutubeDl::new().unwrap();
        let info = dl
            .extract_result_unselected(&server.url("/ok.html"))
            .unwrap()
            .into_video()
            .unwrap();
        let subtitle = &info.subtitles.entries[0].1[0];
        let target = directory.path().join("cancelled.nb.vtt");
        let mut checkpoints = Vec::new();
        let result = dl.download_subtitle_with_progress(
            &info,
            subtitle,
            &target,
            DownloadOptions {
                retries: 3,
                ..Default::default()
            },
            |progress| {
                checkpoints.push(progress.phase);
                if progress.phase == cancel_at {
                    if cancel_at == DownloadPhase::Publishing {
                        assert_eq!(progress.downloaded_bytes, VTT.len() as u64);
                        assert_eq!(progress.total_bytes, Some(VTT.len() as u64));
                    }
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            },
        );
        assert!(matches!(result, Err(Error::DownloadCancelled)));
        assert!(checkpoints.contains(&cancel_at));
        assert!(!target.exists());
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
        assert_eq!(
            server.count("/media.mp4"),
            1,
            "only the initial extraction probe is allowed"
        );
        assert_eq!(
            server.count("/nb.vtt"),
            usize::from(cancel_at == DownloadPhase::Publishing)
        );
        for (_, request) in server
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(path, _)| path == "/nb.vtt")
        {
            let headers = request.to_ascii_lowercase();
            assert!(headers.contains("accept-encoding: identity\r\n"));
            assert!(headers.contains(&format!("referer: {}\r\n", server.url("/ok.html"))));
        }
    }
}

#[cfg(unix)]
#[test]
fn dangling_media_or_sidecar_symlinks_are_preserved_and_refused_before_caption_requests() {
    use std::os::unix::fs::symlink;
    for existing in ["video.mp4", "video.nb.vtt"] {
        let server = Server::new();
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join(existing);
        symlink("missing-target", &target).unwrap();
        let output = server.cli(
            &["/ok.html"],
            &["--write-subs", "-o", "video.mp4"],
            directory.path(),
        );
        assert!(!output.status.success(), "{existing}");
        assert_eq!(
            std::fs::read_link(&target).unwrap(),
            Path::new("missing-target")
        );
        assert!(
            std::fs::symlink_metadata(&target)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(!directory.path().join("missing-target").exists());
        assert_eq!(
            server.count("/nb.vtt"),
            0,
            "refusal must precede subtitle requests"
        );
        assert_eq!(
            server.count("/media.mp4"),
            1,
            "only the extraction probe is allowed"
        );
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}

#[test]
fn automatic_flags_are_independent_last_wins_and_never_select_normal_tracks_alone() {
    for (flags, writes) in [
        (vec!["--write-auto-subs"], false),
        (vec!["--write-automatic-subs", "--write-subs"], true),
        (
            vec!["--write-subs", "--write-auto-subs", "--no-write-subs"],
            false,
        ),
        (
            vec![
                "--write-subs",
                "--write-automatic-subs",
                "--no-write-automatic-subs",
            ],
            true,
        ),
        (vec!["--no-write-auto-subs", "--write-auto-subs"], false),
    ] {
        let server = Server::new();
        let directory = tempfile::tempdir().unwrap();
        let mut flags = flags;
        flags.extend(["--skip-download", "-o", "video.mp4"]);
        success(&server.cli(&["/ok.html"], &flags, directory.path()));
        assert_eq!(
            directory.path().join("video.nb.vtt").exists(),
            writes,
            "{flags:?}"
        );
        assert_eq!(server.count("/nb.vtt"), usize::from(writes));
        assert_eq!(server.count("/media.mp4"), 1);
        assert!(!directory.path().join("video.mp4").exists());
    }
}

#[test]
fn saved_upstream_overwrite_modes_match_published_files() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/overwrites/cases.json")).unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let server = Server::new();
        let directory = tempfile::tempdir().unwrap();
        for name in case["existing"].as_array().unwrap() {
            std::fs::write(directory.path().join(name.as_str().unwrap()), b"old").unwrap();
        }
        let archive = directory.path().join("archive.txt");
        let mut args: Vec<&str> = case["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|arg| arg.as_str().unwrap())
            .collect();
        args.extend([
            "--write-subs",
            "--write-info-json",
            "--download-archive",
            archive.to_str().unwrap(),
            "-o",
            "video.%(ext)s",
        ]);
        success(&server.cli(&["/ok.html"], &args, directory.path()));
        let name = case["name"].as_str().unwrap();
        assert_eq!(
            server.count("/media.mp4") == 2,
            case["media_downloaded"].as_bool().unwrap(),
            "{name}"
        );
        for (file, state) in case["files"].as_object().unwrap() {
            let bytes = std::fs::read(directory.path().join(file)).unwrap();
            assert_eq!(bytes == b"old", state == "old", "{name} {file}");
        }
        assert_eq!(
            archive.exists(),
            case["archived"].as_bool().unwrap(),
            "{name}"
        );
    }
}

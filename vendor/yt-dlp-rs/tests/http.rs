use std::{
    io::{Read, Write},
    net::TcpListener,
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use yt_dlp_rs::{Error, ExtractResult, YoutubeDl, YoutubeOptions, output::render_template};

fn html_fixture(path: &str) -> Option<&'static str> {
    Some(match path {
        "/dimension-boundaries.html" => {
            "<video src='/clip.mp4' width='3840' height='2160'><source src='/clip.mp4' width='-1' height='0' res='480'><source src='/clip.mp4' width='4294967296' height='720p' label='1080p'><source src='/clip.mp4' width=' 1280 ' height='+720'></video>"
        }
        "/ranked.html" => include_str!("fixtures/html5/ranked.html"),
        "/playlist.html" => include_str!("fixtures/html5/playlist.html"),
        "/playlist-formats.html" => include_str!("fixtures/html5/playlist-formats.html"),
        "/playlist-one.html" => {
            "<video></video><audio src='/audio'></audio><video src=' '></video>"
        }
        "/playlist-invalid.html" => "<video src='/clip.mp4'></video><audio src='/missing'></audio>",
        "/playlist-manifest.html" => {
            "<video src='/clip.mp4'></video><audio src='/playlist.m3u8'></audio>"
        }
        "/playlist-mixed-formats.html" => {
            "<video><source src='/clip.mp4'><source src='/audio'></video><audio src='/audio'></audio>"
        }
        "/playlist-changing.html" => {
            "<video src='/changing.mp4'></video><audio src='/audio'></audio>"
        }
        "/formats-progressive.html" => include_str!("fixtures/html5/formats.html"),
        "/video.html" => include_str!("fixtures/html5/video.html"),
        "/audio.html" => include_str!("fixtures/html5/audio.html"),
        "/source.html" => include_str!("fixtures/html5/source.html"),
        "/nested/relative.html" => "<title> Relative  title </title><video src='clip.mp4'></video>",
        "/base.html" => "<base href='/media/'><title>Base</title><video src='clip.mp4'></video>",
        "/og.html" => {
            "<title>Fallback</title><meta property='og:title' content='Chosen &amp; title'><video src='/clip.mp4'></video>"
        }
        "/untitled.html" => "<audio src='/audio'></audio>",
        "/referrer.html" => "<video src='/referrer.mp4'></video>",
        "/changing.html" => "<video src='/changing.mp4'></video>",
        "/formats-redirect.html" => {
            "<video><source src='/clip.mp4'><source src='/media-redirect'></video>"
        }
        "/source-redirect.html" => "<video src='/media-redirect'></video>",
        "/manifest-source.html" => "<video src='/stream'></video>",
        "/multiple.html" => "<video src='/clip.mp4'></video><audio src='/audio'></audio>",
        "/formats.html" => "<video><source src='/clip.mp4'><source src='/playlist.m3u8'></video>",
        "/src-and-source.html" => "<video src='/clip.mp4'><source src='/audio'></video>",
        "/invalid-alternative.html" => {
            "<video><source src='/clip.mp4'><source src='/page.mp4'></video>"
        }
        "/blob-alternative.html" => {
            "<video><source src='/clip.mp4'><source src='blob:https://example.invalid/id'></video>"
        }
        "/manifest.html" => "<video src='/playlist.m3u8'></video>",
        "/codecs.html" => {
            "<video src='/clip.mp4' type='video/mp4; codecs=\"avc1, mp4a\"'><source src='/clip.mp4' type='video/mp4; codecs=\"avc1.42E01E, mp4a.40.2\"'><source src='/clip.mp4' type='video/mp4; codecs=\"newcodec\"'></video><audio><source src='/audio'></audio>"
        }
        "/manifest-hint.html" => "<video src='/clip.mp4' type='application/dash+xml'></video>",
        "/blob.html" => "<video src='blob:https://example.invalid/id'></video>",
        "/source-is-html.html" => "<video src='/page.mp4'></video>",
        "/source-is-partial.html" => "<video src='/partial.mp4'></video>",
        "/source-is-missing.html" => "<video src='/missing'></video>",
        "/empty.html" => "<video src=' '><source src=''></video>",
        "/fake.html" => {
            "<!-- <video src='/clip.mp4'></video> --><script>var x = '<video src=\"/clip.mp4\"></video>';</script><p>&lt;video src='/clip.mp4'&gt;</p>"
        }
        _ => return None,
    })
}

// Local-only fixture: no live sites, Python installation, or external test services.
struct Server {
    base: String,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
    requests: Arc<Mutex<Vec<String>>>,
}

impl Server {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let shutdown = stop.clone();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        let worker = thread::spawn(move || {
            while !shutdown.load(Ordering::Relaxed) {
                let (mut stream, _) = match listener.accept() {
                    Ok(connection) => connection,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(e) => panic!("fixture accept failed: {e}"),
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 1024];
                let deadline = Instant::now() + Duration::from_secs(2);
                while !request.windows(4).any(|w| w == b"\r\n\r\n") && Instant::now() < deadline {
                    match stream.read(&mut buffer) {
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                        Ok(0) | Err(_) => break,
                        Ok(n) => request.extend_from_slice(&buffer[..n]),
                    }
                }
                // A client may abandon a speculative connection. Never answer an
                // empty/incomplete request with a response for a fabricated path.
                if !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    continue;
                }
                let request = String::from_utf8_lossy(&request);
                recorded.lock().unwrap().push(request.to_string());
                let path = request.split_whitespace().nth(1).unwrap_or("/");
                let changing_count = recorded
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|request| request.starts_with("GET /changing.mp4 "))
                    .count();
                let oversized = if path.starts_with("/large") {
                    " ".repeat(2 * 1024 * 1024 + 1)
                } else {
                    String::new()
                };
                let (status, mime, body, extra, length) = match path {
                    "/playlist-redirect" => (
                        "302 Found",
                        "text/plain",
                        "",
                        "Location: /playlist.html\r\n",
                        0,
                    ),
                    "/compressed.html" => {
                        ("200 OK", "text/html", "", "Content-Encoding: gzip\r\n", 0)
                    }
                    "/invalid-utf8.html" => ("200 OK", "text/html", "x", "", 1),
                    "/stream" => ("200 OK", "application/dash+xml", "<MPD/>", "", 6),
                    "/media-redirect" => (
                        "302 Found",
                        "text/plain",
                        "",
                        "Location: /referrer.mp4\r\n",
                        0,
                    ),
                    "/changing.mp4" if changing_count == 1 => {
                        ("200 OK", "video/mp4", "media bytes", "", 11)
                    }
                    "/changing.mp4" => ("200 OK", "text/html", "<html>login</html>", "", 18),
                    "/page-redirect" => (
                        "302 Found",
                        "text/plain",
                        "",
                        "Location: /nested/relative.html\r\n",
                        0,
                    ),
                    "/media/clip.mp4?one=1&two=2"
                    | "/media/clip.mp4"
                    | "/media/clip.mp4?portrait=1"
                    | "/media/clip.mp4?low=1"
                    | "/nested/clip.mp4" => ("200 OK", "video/mp4", "media bytes", "", 11),
                    "/media/clip.webm" => ("200 OK", "video/webm", "webm", "", 4),
                    "/media/tone.mp3" => ("200 OK", "audio/mpeg", "audio", "", 5),
                    "/referrer.mp4"
                        if request.contains(&format!(
                            "referer: http://{}/referrer.html\r\n",
                            stream.local_addr().unwrap()
                        )) || request.contains(&format!(
                            "referer: http://{}/source-redirect.html\r\n",
                            stream.local_addr().unwrap()
                        )) || request.contains(&format!(
                            "referer: http://{}/formats-redirect.html\r\n",
                            stream.local_addr().unwrap()
                        )) =>
                    {
                        ("200 OK", "video/mp4", "media bytes", "", 11)
                    }
                    "/large.html" | "/large-stream.html" => (
                        "200 OK",
                        "text/html",
                        oversized.as_str(),
                        "",
                        oversized.len(),
                    ),
                    "/latin.html" => (
                        "200 OK",
                        "text/html; charset=iso-8859-1",
                        "<video src='/clip.mp4'></video>",
                        "",
                        30,
                    ),
                    "/redirect" => (
                        "302 Found",
                        "text/plain",
                        "",
                        "Location: /hello%20world.mp4?token=secret\r\n",
                        0,
                    ),
                    "/hello%20world.mp4?token=secret" | "/clip.mp4" => {
                        ("200 OK", "video/mp4", "media bytes", "", 11)
                    }
                    "/audio" => ("200 OK", "audio/mpeg; charset=binary", "audio", "", 5),
                    "/opaque.webm" => ("200 OK", "application/octet-stream", "webm", "", 4),
                    "/latm.m4a" => ("200 OK", "audio/mp4a-latm", "audio", "", 5),
                    "/clip.weird" => ("200 OK", "video/x-weird", "media", "", 5),
                    "/unnamed" => ("200 OK", "video/mp2t", "media", "", 5),
                    "/blob" => ("200 OK", "application/octet-stream", "media", "", 5),
                    "/named" => (
                        "200 OK",
                        "audio/mpeg",
                        "media",
                        "Content-Disposition: attachment; filename=\"song.ogg\"\r\n",
                        5,
                    ),
                    "/truncated.mp4" => ("200 OK", "video/mp4", "short", "", 100),
                    "/partial.mp4" => (
                        "206 Partial Content",
                        "video/mp4",
                        "short",
                        "Content-Range: bytes 0-4/100\r\n",
                        5,
                    ),
                    "/page.mp4" => ("200 OK", "text/html", "<html>not media</html>", "", 22),
                    "/playlist.m3u8" => {
                        ("200 OK", "application/vnd.apple.mpegurl", "#EXTM3U", "", 7)
                    }
                    _ => match html_fixture(path) {
                        Some(body) => ("200 OK", "text/html; charset=utf-8", body, "", body.len()),
                        None => ("404 Not Found", "text/plain", "missing", "", 7),
                    },
                };
                let content_length = if path == "/large-stream.html" {
                    String::new()
                } else {
                    format!("Content-Length: {length}\r\n")
                };
                let mut response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\n{content_length}{extra}Connection: close\r\n\r\n{body}"
                ).into_bytes();
                if path == "/invalid-utf8.html" {
                    *response.last_mut().unwrap() = 0xff;
                }
                // Extraction intentionally closes a response after inspecting headers.
                let _ = stream.write_all(&response);
            }
        });
        Self {
            base,
            stop,
            worker: Some(worker),
            requests,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
    }
}

#[test]
fn extracts_redirected_media_and_downloads_exact_bytes() {
    let server = Server::start();
    let dl = YoutubeDl::new().unwrap();
    let info = dl.extract_info(&server.url("/redirect")).unwrap();
    assert_eq!(info.title, "hello world");
    assert_eq!(info.id, "hello world");
    assert_eq!(info.ext, "mp4");
    // Upstream's direct format carries no extraction-time filesize; the size
    // is only checked through the downloaded byte count below.
    assert_eq!(info.filesize, None);
    assert_eq!(info.webpage_url.as_str(), server.url("/redirect"));
    assert!(info.url.as_str().ends_with("?token=secret"));
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("nested/video.mp4");
    assert_eq!(dl.download(&info, &output).unwrap(), 11);
    assert_eq!(std::fs::read(&output).unwrap(), b"media bytes");
    assert!(matches!(
        dl.download(&info, &output),
        Err(Error::OutputExists(_))
    ));
    assert_eq!(std::fs::read(&output).unwrap(), b"media bytes");
}

#[test]
fn recognizes_mime_and_binary_extension_fallback() {
    let server = Server::start();
    let dl = YoutubeDl::new().unwrap();
    assert_eq!(dl.extract_info(&server.url("/audio")).unwrap().ext, "mp3");
    assert_eq!(
        dl.extract_info(&server.url("/opaque.webm")).unwrap().ext,
        "webm"
    );
}

#[test]
fn extracts_saved_html5_fixtures_and_downloads_media() {
    let server = Server::start();
    let dl = YoutubeDl::new().unwrap();
    for (page, title, ext, source, body) in [
        (
            "video",
            "A & B",
            "mp4",
            "/media/clip.mp4?one=1&two=2",
            b"media bytes".as_slice(),
        ),
        (
            "audio",
            "Audio fixture",
            "mp3",
            "/media/tone.mp3",
            b"audio".as_slice(),
        ),
        (
            "source",
            "Source fixture",
            "mp4",
            "/media/clip.mp4",
            b"media bytes".as_slice(),
        ),
    ] {
        let page_url = server.url(&format!("/{page}.html"));
        let info = dl.extract_info(&page_url).unwrap();
        assert_eq!(info.format_id.as_deref(), Some("0"));
        assert_eq!(info.formats.len(), 1);
        assert_eq!(info.id, format!("{page}-1"));
        assert_eq!(info.title, format!("{title} (1)"));
        assert_eq!(info.ext, ext);
        assert_eq!(info.url.as_str(), server.url(source));
        assert_eq!(info.webpage_url.as_str(), page_url);
        // Upstream HTML5 formats have no extraction-time filesize.
        assert_eq!(info.filesize, None);
        assert!(!info.is_direct());
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join(format!("media.{ext}"));
        assert_eq!(dl.download(&info, &destination).unwrap(), body.len() as u64);
        assert_eq!(std::fs::read(destination).unwrap(), body);
    }
}

#[test]
fn html5_resolves_redirects_base_urls_and_titles() {
    let server = Server::start();
    let dl = YoutubeDl::new().unwrap();
    let redirected = dl.extract_info(&server.url("/page-redirect")).unwrap();
    assert_eq!(redirected.url.as_str(), server.url("/nested/clip.mp4"));
    assert_eq!(
        redirected.webpage_url.as_str(),
        server.url("/nested/relative.html")
    );
    assert_eq!(redirected.id, "relative-1");
    assert_eq!(redirected.title, "Relative title (1)");
    let base = dl.extract_info(&server.url("/base.html")).unwrap();
    assert_eq!(base.url.as_str(), server.url("/media/clip.mp4"));
    assert_eq!(
        dl.extract_info(&server.url("/og.html")).unwrap().title,
        "Chosen & title (1)"
    );
    assert_eq!(
        dl.extract_info(&server.url("/untitled.html"))
            .unwrap()
            .title,
        "untitled (1)"
    );
}

#[test]
fn html5_preserves_referrer_for_probe_and_download() {
    let server = Server::start();
    let dl = YoutubeDl::new().unwrap();
    let info = dl.extract_info(&server.url("/referrer.html")).unwrap();
    let directory = tempfile::tempdir().unwrap();
    assert_eq!(
        dl.download(&info, directory.path().join("media.mp4"))
            .unwrap(),
        11
    );
    let redirected = dl
        .extract_info(&server.url("/source-redirect.html"))
        .unwrap();
    assert_eq!(redirected.url.as_str(), server.url("/referrer.mp4"));
    assert_eq!(
        redirected.webpage_url.as_str(),
        server.url("/source-redirect.html")
    );
    assert_eq!(
        dl.download(&redirected, directory.path().join("redirect.mp4"))
            .unwrap(),
        11
    );
}

#[test]
fn html5_source_changing_to_html_cannot_be_published_as_media() {
    let server = Server::start();
    let dl = YoutubeDl::new().unwrap();
    let info = dl.extract_info(&server.url("/changing.html")).unwrap();
    let directory = tempfile::tempdir().unwrap();
    assert!(matches!(
        dl.download(&info, directory.path().join("media.mp4")),
        Err(Error::UnsupportedHtml5(_))
    ));
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn html5_rejects_unsupported_sources() {
    let server = Server::start();
    let dl = YoutubeDl::new().unwrap();
    for path in [
        "/formats.html",
        "/manifest.html",
        "/manifest-hint.html",
        "/source-is-html.html",
        "/latin.html",
        "/compressed.html",
        "/invalid-utf8.html",
        "/manifest-source.html",
    ] {
        assert!(
            matches!(
                dl.extract_info(&server.url(path)),
                Err(Error::UnsupportedHtml5(_) | Error::UnsupportedManifest(_))
            ),
            "{path}"
        );
    }
    for path in ["/empty.html", "/fake.html"] {
        assert!(
            matches!(
                dl.extract_info(&server.url(path)),
                Err(Error::UnsupportedUrl(_))
            ),
            "{path}"
        );
    }
    assert!(matches!(
        dl.extract_info(&server.url("/blob.html")),
        Err(Error::UnsupportedScheme(_))
    ));
    assert!(matches!(
        dl.extract_info(&server.url("/source-is-partial.html")),
        Err(Error::PartialResponse)
    ));
    assert!(matches!(
        dl.extract_info(&server.url("/source-is-missing.html")),
        Err(Error::Http(_))
    ));
    for path in ["/large.html", "/large-stream.html"] {
        let result = dl.extract_info(&server.url(path));
        assert!(
            matches!(result, Err(Error::HtmlTooLarge(_))),
            "{path}: {result:?}"
        );
    }
}

#[test]
fn direct_media_uses_one_extraction_request() {
    let server = Server::start();
    YoutubeDl::new()
        .unwrap()
        .extract_info(&server.url("/clip.mp4"))
        .unwrap();
    assert_eq!(server.requests.lock().unwrap().len(), 1);
}

#[test]
fn cli_html5_json_and_simulation_do_not_download() {
    let server = Server::start();
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("video.mp4");
    for flag in ["--dump-json", "--simulate"] {
        let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .args([
                flag,
                "-o",
                target.to_str().unwrap(),
                &server.url("/video.html"),
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!target.exists());
        if flag == "--dump-json" {
            let info: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            // Like upstream, only generic direct links have `direct`.
            assert!(info.get("direct").is_none());
            assert_eq!(info["title"], "A & B (1)");
            assert_eq!(info["protocol"], "http");
            assert_eq!(info["formats"][0]["protocol"], "http");
        }
    }
    assert_eq!(server.requests.lock().unwrap().len(), 4);
}

#[test]
fn rejects_pages_manifests_errors_and_non_http_urls() {
    let server = Server::start();
    let dl = YoutubeDl::new().unwrap();
    for path in ["/page.mp4", "/playlist.m3u8"] {
        assert!(matches!(
            dl.extract_info(&server.url(path)),
            Err(Error::UnsupportedUrl(_) | Error::UnsupportedManifest(_))
        ));
    }
    assert!(matches!(
        dl.extract_info(&server.url("/missing")),
        Err(error @ Error::Webpage { .. }) if error.to_string().contains("Unable to download webpage: HTTP Error 404")
    ));
    // Upstream refuses file URLs unless --enable-file-urls.
    assert!(matches!(
        dl.extract_info("file:///tmp/video.mp4"),
        Err(Error::Reported(message)) if message.starts_with("file:// URLs are disabled by default")
    ));
    assert!(matches!(
        dl.extract_info("ftp://example.invalid/video.mp4"),
        Err(Error::UnsupportedScheme(_))
    ));
    // Upstream GenericIE: an input without a scheme is not a valid URL.
    assert!(matches!(
        dl.extract_info("not a url"),
        Err(Error::Reported(message)) if message == "[generic] 'not a url' is not a valid URL"
    ));
}

#[test]
fn truncated_transfers_do_not_publish_or_leave_temporary_files() {
    let server = Server::start();
    let dl = YoutubeDl::new().unwrap();
    let info = dl.extract_info(&server.url("/truncated.mp4")).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("video.mp4");
    assert!(dl.download(&info, &output).is_err());
    assert!(!output.exists());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn unsolicited_partial_responses_cannot_be_published_as_complete_media() {
    let server = Server::start();
    let dl = YoutubeDl::new().unwrap();
    assert!(matches!(
        dl.extract_info(&server.url("/partial.mp4")),
        Err(Error::PartialResponse)
    ));
    let mut info = dl.extract_info(&server.url("/clip.mp4")).unwrap();
    info.url = server.url("/partial.mp4").parse().unwrap();
    let directory = tempfile::tempdir().unwrap();
    assert!(matches!(
        dl.download(&info, directory.path().join("clip.mp4")),
        Err(Error::PartialResponse)
    ));
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn templates_are_explicit_and_metadata_cannot_inject_paths_or_fields() {
    let server = Server::start();
    let mut info = YoutubeDl::new()
        .unwrap()
        .extract_info(&server.url("/clip.mp4"))
        .unwrap();
    info.title = "../folder/%(id)s".into();
    assert_eq!(
        render_template("downloads/%%-%(title)s.%(ext)s", &info)
            .unwrap()
            .to_str()
            .unwrap(),
        "downloads/%-..⧸folder⧸%(id)s.mp4"
    );
    assert_eq!(
        render_template("%(title)03d", &info).unwrap(),
        std::path::PathBuf::from("NA")
    );
    // Unknown fields are missing values, like upstream.
    assert_eq!(
        render_template("%(unknown)s", &info).unwrap(),
        std::path::PathBuf::from("NA")
    );
    for invalid in ["%(id)z", "%(title+)s", "%", "", "-"] {
        assert!(render_template(invalid, &info).is_err(), "{invalid}");
    }
}

#[test]
fn cli_json_continues_after_errors_and_returns_failure() {
    let server = Server::start();
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args([
            "--dump-json",
            &server.url("/missing"),
            &server.url("/clip.mp4"),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert_eq!(text.lines().count(), 1);
    let info: serde_json::Value = serde_json::from_str(text.trim()).unwrap();
    assert_eq!(info["extractor_key"], "Generic");
    // Upstream's direct format has no extraction-time filesize.
    assert!(info["filesize"].is_null());
    assert_eq!(info["url"], server.url("/clip.mp4"));
    assert!(String::from_utf8(output.stderr).unwrap().contains("404"));
}

#[test]
fn cli_simulate_and_download() {
    let server = Server::start();
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("clip.mp4");
    let args = ["-o", target.to_str().unwrap(), &server.url("/clip.mp4")];
    let simulated = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .arg("--simulate")
        .args(args)
        .output()
        .unwrap();
    assert!(simulated.status.success());
    assert!(!target.exists());
    let downloaded = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(args)
        .output()
        .unwrap();
    assert!(downloaded.status.success());
    assert_eq!(std::fs::read(target).unwrap(), b"media bytes");
}

/// Without `-j`/`-J`/`--print`, stdout holds only bracketed screen lines.
fn only_screen_lines(stdout: &[u8]) {
    let stdout = String::from_utf8_lossy(stdout);
    assert!(
        stdout
            .lines()
            .all(|line| line.trim_start_matches('\r').starts_with('[')),
        "{stdout}"
    );
}

#[test]
fn cli_screen_lines_use_stdout_and_warnings_use_stderr() {
    let server = Server::start();
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("video.mp4");
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["-o", target.to_str().unwrap(), &server.url("/video.html")])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert!(target.exists());
    // Upstream `to_screen` writes to stdout unless the output template is `-`.
    let destination = format!("[download] Destination: {}", target.display());
    assert!(stdout.contains(&destination), "{stdout}");
    assert!(stdout.contains("[generic] Extracting URL: "), "{stdout}");
    assert!(!stderr.contains("[download]"), "{stderr}");
    assert!(!stderr.contains("[generic] Extracting URL: "), "{stderr}");
    // `report_warning` always writes to stderr.
    let warning = "WARNING: [generic] Falling back on generic information extractor";
    assert!(stderr.contains(warning), "{stderr}");
    assert!(!stdout.contains("WARNING:"), "{stdout}");
}

#[test]
fn youtube_runtime_configuration_is_lazy_for_generic_extraction() {
    let server = Server::start();
    let directory = tempfile::tempdir().unwrap();
    let runtime = directory.path().join("nonexistent-deno");
    let dl = YoutubeDl::with_youtube_options(YoutubeOptions {
        include_translated_subtitles: false,
        no_playlist: false,
        js_runtime: Some(runtime.clone()),
        playback_format: None,
    })
    .unwrap();
    for (path, extractor) in [("/clip.mp4", "generic"), ("/video.html", "html5")] {
        let info = dl.extract_info(&server.url(path)).unwrap();
        // Neither upstream direct nor HTML5 formats carry a filesize.
        assert_eq!(info.filesize, None);
        assert_eq!(info.extractor, extractor);
    }
    assert!(!runtime.exists());
}

#[test]
fn cli_explicit_youtube_runtime_preserves_generic_json_simulation_and_download() {
    let server = Server::start();
    let directory = tempfile::tempdir().unwrap();
    let runtime = directory.path().join("nonexistent-deno");
    for (index, path) in ["/clip.mp4", "/video.html"].into_iter().enumerate() {
        let url = server.url(path);
        let target = directory.path().join(format!("output-{index}.mp4"));
        for mode in [Some("-j"), Some("-s"), None] {
            let mut command = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"));
            command.arg("--youtube-js-runtime").arg(&runtime);
            if let Some(mode) = mode {
                command.arg(mode);
            }
            let output = command.arg("-o").arg(&target).arg(&url).output().unwrap();
            assert!(
                output.status.success(),
                "{path} {mode:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            match mode {
                Some("-j") => {
                    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
                    // Upstream direct and HTML5 formats carry no filesize.
                    assert!(json["filesize"].is_null());
                    assert!(json["url"].as_str().unwrap().contains("clip.mp4"));
                    assert!(!target.exists());
                }
                Some("-s") => {
                    only_screen_lines(&output.stdout);
                    assert!(!target.exists());
                }
                None => assert_eq!(std::fs::read(&target).unwrap(), b"media bytes"),
                _ => unreachable!(),
            }
        }
    }
    assert!(!runtime.exists());
}

#[test]
fn cli_youtube_runtime_requires_a_path() {
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["http://example.invalid/video.mp4", "--youtube-js-runtime"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("--youtube-js-runtime <PATH>"), "{error}");
    assert!(error.contains("a value is required"), "{error}");
}

#[test]
fn html5_formats_select_automatically_and_download_explicit_sources() {
    let server = Server::start();
    let dl = YoutubeDl::new().unwrap();
    let page = server.url("/formats-progressive.html");
    // Pinned upstream sorts the labeled sources as ["1", "0"] (webm below
    // mp4) and selects "0" by default.
    assert_eq!(
        dl.extract_info(&page).unwrap().format_id.as_deref(),
        Some("0")
    );
    for (id, ext, path, bytes) in [
        (
            "0",
            "mp4",
            "/media/clip.mp4?one=1&two=2",
            b"media bytes".as_slice(),
        ),
        ("1", "webm", "/media/clip.webm", b"webm".as_slice()),
    ] {
        let info = dl.extract_info_with_format(&page, Some(id)).unwrap();
        assert_eq!(info.formats.len(), 2);
        assert_eq!(info.formats[0].format_id, "1");
        assert_eq!(info.formats[1].format_id, "0");
        assert_eq!(info.format_id.as_deref(), Some(id));
        assert_eq!(info.ext, ext);
        // Upstream HTML5 formats have no extraction-time filesize; the
        // downloaded byte count below checks the transfer size instead.
        assert_eq!(info.filesize, None);
        assert_eq!(info.url.as_str(), server.url(path));
        assert_eq!(info.title, "Format fixture (1)");
        let directory = tempfile::tempdir().unwrap();
        let destination = render_template(
            directory.path().join("%(title)s.%(ext)s").to_str().unwrap(),
            &info,
        )
        .unwrap();
        assert_eq!(
            dl.download(&info, &destination).unwrap(),
            bytes.len() as u64
        );
        assert_eq!(std::fs::read(destination).unwrap(), bytes);
    }
    // Every probe and transfer uses the page referrer, including the second source.
    for request in server
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.starts_with("GET /media/"))
    {
        assert!(request.contains(&format!("referer: {page}\r\n")));
    }
}

#[test]
fn format_ids_are_literal_and_src_precedes_child_sources() {
    let server = Server::start();
    let dl = YoutubeDl::new().unwrap();
    let page = server.url("/src-and-source.html");
    assert_eq!(
        dl.extract_info(&page).unwrap().format_id.as_deref(),
        Some("1")
    );
    let info = dl.extract_info_with_format(&page, Some("1")).unwrap();
    assert_eq!(info.ext, "mp3");
    assert_eq!(info.url.as_str(), server.url("/audio"));
    // Like upstream `_merge`, a second complete source is dropped.
    assert_eq!(
        dl.extract_info_with_format(&page, Some("0+1"))
            .unwrap()
            .format_id
            .as_deref(),
        Some("0")
    );
    for id in ["", "2", "01", "bestvideo", "0//1"] {
        assert!(
            matches!(
                dl.extract_info_with_format(&page, Some(id)),
                Err(Error::FormatSelection(_))
            ),
            "{id}"
        );
    }
    assert!(
        dl.extract_info_with_format(&server.url("/video.html"), Some("0"))
            .is_ok()
    );
    assert!(matches!(
        dl.extract_info_with_format(&server.url("/clip.mp4"), Some("0")),
        Err(Error::FormatSelection(_))
    ));
}

#[test]
fn selecting_supported_source_does_not_hide_unsupported_alternatives() {
    let server = Server::start();
    let dl = YoutubeDl::new().unwrap();
    for page in ["/formats.html", "/invalid-alternative.html"] {
        assert!(
            matches!(
                dl.extract_info_with_format(&server.url(page), Some("0")),
                Err(Error::UnsupportedHtml5(_) | Error::UnsupportedManifest(_))
            ),
            "{page}"
        );
    }
    assert!(matches!(
        dl.extract_info_with_format(&server.url("/blob-alternative.html"), Some("0")),
        Err(Error::UnsupportedScheme(_))
    ));
}

#[test]
fn html5_playlist_metadata_order_and_legacy_boundary() {
    let server = Server::start();
    let dl = YoutubeDl::new().unwrap();
    let page = server.url("/playlist.html");
    let result = dl.extract_result(&page).unwrap();
    let serialized = serde_json::to_value(&result).unwrap();
    assert_eq!(serialized["_type"], "playlist");
    let decoded: ExtractResult = serde_json::from_value(serialized).unwrap();
    let ExtractResult::Playlist(playlist) = decoded else {
        panic!("expected playlist");
    };
    assert_eq!(playlist.id, "playlist");
    assert_eq!(playlist.title, "Playlist fixture");
    assert_eq!(playlist.webpage_url.as_str(), page);
    assert_eq!(playlist.extractor, "generic");
    assert_eq!(playlist.extractor_key, "Generic");
    assert_eq!(playlist.entries.len(), 2);
    let redirected = dl
        .extract_result(&server.url("/playlist-redirect"))
        .unwrap();
    let ExtractResult::Playlist(redirected) = redirected else {
        panic!("expected redirected playlist");
    };
    assert_eq!(redirected.webpage_url.as_str(), page);
    assert_eq!(redirected.id, "playlist");
    for (index, (entry, ext)) in playlist.entries.iter().zip(["mp4", "mp3"]).enumerate() {
        assert_eq!(entry.id, format!("playlist-{}", index + 1));
        assert_eq!(entry.title, format!("Playlist fixture ({})", index + 1));
        assert_eq!(entry.ext, ext);
        assert_eq!(entry.webpage_url.as_str(), page);
        assert_eq!(entry.extractor, "html5");
        assert_eq!(entry.extractor_key, "HTML5MediaEmbed");
        assert_eq!(entry.format_id.as_deref(), Some("0"));
        assert!(!entry.is_direct());
    }
    assert!(matches!(
        dl.extract_info(&page),
        Err(Error::PlaylistRequiresResult)
    ));
    assert!(matches!(
        dl.extract_info_with_format(&page, Some("0")),
        Err(Error::PlaylistRequiresResult)
    ));
    let generic = yt_dlp_rs::extractor::GenericHttp;
    assert!(matches!(
        yt_dlp_rs::extractor::Extractor::extract(
            &generic,
            &reqwest::blocking::Client::new(),
            &page.parse().unwrap()
        ),
        Err(Error::PlaylistRequiresResult)
    ));
    for request in server
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.starts_with("GET /media/"))
    {
        assert!(request.contains(&format!("referer: {page}\r\n")));
    }
}

#[test]
fn html5_result_cardinality_and_title_fallback() {
    let server = Server::start();
    let dl = YoutubeDl::new().unwrap();
    let ExtractResult::Video(info) = dl
        .extract_result(&server.url("/playlist-one.html"))
        .unwrap()
    else {
        panic!("empty elements should not force a playlist");
    };
    assert_eq!(info.id, "playlist-one-1");
    assert_eq!(info.title, "playlist-one (1)");
    let serialized = serde_json::to_value(ExtractResult::Video(info)).unwrap();
    assert_eq!(serialized["_type"], "video");
    let decoded: ExtractResult = serde_json::from_value(serialized).unwrap();
    assert_eq!(decoded.into_video().unwrap().id, "playlist-one-1");
    assert!(dl.extract_info(&server.url("/playlist-one.html")).is_ok());
    assert!(matches!(
        dl.extract_result(&server.url("/empty.html")),
        Err(Error::UnsupportedUrl(_))
    ));
    let ExtractResult::Playlist(info) = dl.extract_result(&server.url("/multiple.html")).unwrap()
    else {
        panic!("expected playlist");
    };
    assert_eq!(info.id, "multiple");
    assert_eq!(info.title, "video");
    assert_eq!(info.entries[0].title, "multiple (1)");
}

#[test]
fn html5_playlist_selection_is_atomic_in_the_library_and_per_entry_in_the_cli() {
    let server = Server::start();
    let dl = YoutubeDl::new().unwrap();
    let page = server.url("/playlist-formats.html");
    // Pinned upstream selects the mp4 source ("0") for each entry by default.
    for entry in dl.extract_result(&page).unwrap().into_entries() {
        assert_eq!(entry.format_id.as_deref(), Some("0"));
    }
    for id in ["0", "1"] {
        let entries = dl
            .extract_result_with_format(&page, Some(id))
            .unwrap()
            .into_entries();
        assert_eq!(entries.len(), 2);
        for entry in entries {
            assert_eq!(entry.formats.len(), 2);
            assert_eq!(entry.format_id.as_deref(), Some(id));
            assert_eq!(entry.ext, if id == "0" { "mp4" } else { "webm" });
        }
    }
    let error = dl
        .extract_result_with_format(&server.url("/playlist-mixed-formats.html"), Some("1"))
        .unwrap_err();
    assert!(matches!(error, Error::FormatSelection(_)));
    assert!(error.to_string().contains("playlist-mixed-formats-2"));
    assert!(matches!(
        dl.extract_result(&server.url("/playlist-invalid.html")),
        Err(Error::Http(_))
    ));
    assert!(matches!(
        dl.extract_result_with_format(&server.url("/playlist-manifest.html"), Some("0")),
        Err(Error::UnsupportedManifest(_))
    ));
    // The library stays atomic, but the CLI selects per entry like upstream
    // `process_video_result`: entry 1 downloads its audio source, entry 2
    // (no format "1") fails on its own and the run exits 1.
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .current_dir(directory.path())
        .args(["-f", "1", &server.url("/playlist-mixed-formats.html")])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    only_screen_lines(&output.stdout);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        stderr
            .lines()
            .filter(|line| line.starts_with("ERROR:"))
            .collect::<Vec<_>>(),
        [
            "ERROR: [html5] playlist-mixed-formats-2: Requested format is not available. Use --list-formats for a list of available formats"
        ],
        "{stderr}"
    );
    assert!(!stderr.contains("format selection failed"), "{stderr}");
    let item_two = stdout.find("[download] Downloading item 2 of 2").unwrap();
    assert!(
        stdout[..item_two].contains("[info] playlist-mixed-formats-1: Downloading 1 format(s): 1")
    );
    assert!(!stdout[item_two..].contains("[info]"), "{stdout}");
    assert!(stdout.contains("[download] Finished downloading playlist: video"));
    let downloaded = directory
        .path()
        .join("playlist-mixed-formats (1) [playlist-mixed-formats-1].mp3");
    assert_eq!(std::fs::read(&downloaded).unwrap(), b"audio");
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    std::fs::remove_file(downloaded).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .current_dir(directory.path())
        .args(["-f", "1", "-o", "%(id)s.%(ext)s", &page])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for id in ["playlist-formats-1", "playlist-formats-2"] {
        assert_eq!(
            std::fs::read(directory.path().join(format!("{id}.webm"))).unwrap(),
            b"webm"
        );
    }
}

#[test]
fn cli_playlist_json_simulation_and_downloads() {
    let server = Server::start();
    let directory = tempfile::tempdir().unwrap();
    let page = server.url("/playlist.html");
    for flag in ["-j", "-s"] {
        let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .current_dir(directory.path())
            .args([flag, &page])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
        if flag == "-j" {
            let entries: Vec<serde_json::Value> = String::from_utf8(output.stdout)
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            assert_eq!(entries.len(), 2);
            assert_eq!(entries[0]["id"], "playlist-1");
            assert_eq!(entries[1]["id"], "playlist-2");
        } else {
            only_screen_lines(&output.stdout);
        }
    }
    // Only page requests and header probes, no transfer requests.
    assert_eq!(server.requests.lock().unwrap().len(), 6);
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .current_dir(directory.path())
        .arg(&page)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read(
            directory
                .path()
                .join("Playlist fixture (1) [playlist-1].mp4")
        )
        .unwrap(),
        b"media bytes"
    );
    assert_eq!(
        std::fs::read(
            directory
                .path()
                .join("Playlist fixture (2) [playlist-2].mp3")
        )
        .unwrap(),
        b"audio"
    );
    for request in server
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.starts_with("GET /media/"))
    {
        assert!(request.contains(&format!("referer: {page}\r\n")));
    }
}

#[test]
fn cli_playlist_download_failures_continue_to_later_entries_and_urls() {
    let server = Server::start();
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .current_dir(directory.path())
        .args([
            "-o",
            "%(id)s.%(ext)s",
            &server.url("/playlist-changing.html"),
            &server.url("/clip.mp4"),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("playlist-changing-1"));
    assert!(!directory.path().join("playlist-changing-1.mp4").exists());
    assert_eq!(
        std::fs::read(directory.path().join("playlist-changing-2.mp3")).unwrap(),
        b"audio"
    );
    assert_eq!(
        std::fs::read(directory.path().join("clip.mp4")).unwrap(),
        b"media bytes"
    );
    // Like upstream, a transfer that fails before receiving data leaves no part file.
    assert!(
        !directory
            .path()
            .join("playlist-changing-1.mp4.part")
            .exists()
    );
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 2);
}

#[test]
fn existing_video_extractors_use_the_default_typed_adapter() {
    use yt_dlp_rs::{
        Result, VideoInfo,
        extractor::{DirectHttp, Extractor},
    };
    struct Custom;
    impl Extractor for Custom {
        fn key(&self) -> &'static str {
            "Custom"
        }
        fn extract(
            &self,
            client: &reqwest::blocking::Client,
            url: &url::Url,
        ) -> Result<Option<VideoInfo>> {
            let mut info = DirectHttp.extract(client, url)?.unwrap();
            info.extractor = "custom".into();
            Ok(Some(info))
        }
    }
    let server = Server::start();
    let mut dl = YoutubeDl::new().unwrap();
    dl.register_extractor(Box::new(Custom));
    let ExtractResult::Video(info) = dl.extract_result(&server.url("/clip.mp4")).unwrap() else {
        panic!("expected video")
    };
    assert_eq!(info.extractor, "custom");
    assert_eq!(server.requests.lock().unwrap().len(), 1);
}

#[test]
fn cli_format_selection_json_download_and_failure() {
    let server = Server::start();
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("selected.%(ext)s");
    let page = server.url("/formats-progressive.html");
    for flag in ["--dump-json", "--simulate"] {
        let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .args([flag, "--format", "1", "-o", target.to_str().unwrap(), &page])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
        if flag == "--dump-json" {
            let info: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(info["format_id"], "1");
            assert_eq!(info["formats"].as_array().unwrap().len(), 2);
            assert_eq!(info["ext"], "webm");
        }
    }
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["-f", "1", "-o", target.to_str().unwrap(), &page])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read(directory.path().join("selected.webm")).unwrap(),
        b"webm"
    );
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["-j", "-f", "missing/best", &page])
        .output()
        .unwrap();
    assert!(output.status.success());
    let info: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    // Pinned upstream `-f missing/best` selects "0" (mp4) here.
    assert_eq!(info["format_id"], "0");
}

#[test]
fn redirected_alternative_records_final_url_and_preserves_referrer() {
    let server = Server::start();
    let dl = YoutubeDl::new().unwrap();
    let info = dl
        .extract_info_with_format(&server.url("/formats-redirect.html"), Some("1"))
        .unwrap();
    assert_eq!(info.url.as_str(), server.url("/referrer.mp4"));
    assert_eq!(info.formats[1].url, info.url);
    let directory = tempfile::tempdir().unwrap();
    assert_eq!(
        dl.download(&info, directory.path().join("media.mp4"))
            .unwrap(),
        11
    );
}

#[test]
fn cli_continues_after_a_format_selection_error() {
    let server = Server::start();
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args([
            "-j",
            "-f",
            "1",
            &server.url("/clip.mp4"),
            &server.url("/formats-progressive.html"),
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let text = String::from_utf8(output.stdout).unwrap();
    assert_eq!(text.lines().count(), 1);
    let info: serde_json::Value = serde_json::from_str(text.trim()).unwrap();
    assert_eq!(info["id"], "formats-progressive-1");
    assert_eq!(info["format_id"], "1");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        stderr
            .lines()
            .filter(|line| line.starts_with("ERROR:"))
            .collect::<Vec<_>>(),
        [
            "ERROR: [generic] clip: Requested format is not available. Use --list-formats for a list of available formats"
        ],
        "{stderr}"
    );
    assert!(!stderr.contains("format selection failed"), "{stderr}");
    // With -J, a single video whose selection fails prints `null`.
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args([
            "-J",
            "-f",
            "1",
            &server.url("/clip.mp4"),
            &server.url("/formats-progressive.html"),
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let text = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<serde_json::Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines.len(), 2, "{text}");
    assert!(lines[0].is_null());
    assert_eq!(lines[1]["id"], "formats-progressive-1");
    assert_eq!(lines[1]["format_id"], "1");
    assert_eq!(
        String::from_utf8_lossy(&output.stderr)
            .lines()
            .filter(|line| line.starts_with("ERROR:"))
            .collect::<Vec<_>>(),
        [
            "ERROR: [generic] clip: Requested format is not available. Use --list-formats for a list of available formats"
        ]
    );
}

#[test]
fn automatic_resolution_selection_reaches_cli_json_and_downloads() {
    let server = Server::start();
    let dl = YoutubeDl::new().unwrap();
    let page = server.url("/ranked.html");
    for (selection, id, width, height, path) in [
        (None, "0", Some(1920), Some(1080), "/media/clip.mp4"),
        (Some("best"), "0", Some(1920), Some(1080), "/media/clip.mp4"),
        (Some("worst"), "2", None, Some(360), "/media/clip.mp4?low=1"),
        (
            Some("1"),
            "1",
            Some(720),
            Some(1280),
            "/media/clip.mp4?portrait=1",
        ),
    ] {
        let info = dl.extract_info_with_format(&page, selection).unwrap();
        assert_eq!(info.format_id.as_deref(), Some(id));
        assert_eq!((info.width, info.height), (width, height));
        assert_eq!(info.url.as_str(), server.url(path));
        // Upstream HTML5 formats have no extraction-time filesize.
        assert_eq!(info.filesize, None);
        let mut command = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"));
        command.arg("-j");
        if let Some(selection) = selection {
            command.args(["-f", selection]);
        }
        let output = command.arg(&page).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(json["format_id"], id);
        assert_eq!(json["height"], height.unwrap());
        assert_eq!(json["formats"][1]["width"], 720);
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("selected.mp4");
        let mut command = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"));
        command.args(["-o", target.to_str().unwrap()]);
        if let Some(selection) = selection {
            command.args(["-f", selection]);
        }
        let output = command.arg(&page).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(std::fs::read(target).unwrap(), b"media bytes");
        assert!(
            server
                .requests
                .lock()
                .unwrap()
                .last()
                .unwrap()
                .starts_with(&format!("GET {path} HTTP/1.1"))
        );
    }
    for selector in [None, Some("best"), Some("worst")] {
        let direct = dl
            .extract_info_with_format(&server.url("/clip.mp4"), selector)
            .unwrap();
        assert_eq!(direct.format_id.as_deref(), Some("mp4"));
        assert!(direct.width.is_none());
        let entries = dl
            .extract_result_with_format(&server.url("/playlist-mixed-formats.html"), selector)
            .unwrap()
            .into_entries();
        assert_eq!(
            entries[0].format_id.as_deref(),
            Some(if selector == Some("worst") { "0" } else { "1" })
        );
        assert_eq!(entries[1].format_id.as_deref(), Some("0"));
    }
}

#[test]
fn source_dimensions_follow_upstream_parsing_labels_and_player_size() {
    // Keep this parsing boundary covered through real extraction and JSON metadata.
    // Compared with pinned upstream. Upstream keeps the widths -1 and
    // 4294967296, which native u32 fields leave unknown; its `res` of -1 then
    // sorts the 480 source first (ID 0) and the player source second, the
    // reverse of the native order below. Known divergence for such values.
    let server = Server::start();
    let mut info = YoutubeDl::new()
        .unwrap()
        .extract_info(&server.url("/dimension-boundaries.html"))
        .unwrap();
    assert_eq!(info.format_id.as_deref(), Some("1080p"));
    let dimensions: Vec<_> = info
        .formats
        .iter()
        .map(|f| (f.format_id.as_str(), f.width, f.height))
        .collect();
    assert_eq!(
        dimensions,
        vec![
            // The media element's own player size is not a source dimension.
            ("0", None, None),
            // width='-1' height='0' res='480': zero height falls back to res.
            ("1", None, Some(480)),
            // ' 1280 ' and '+720' parse like Python int().
            ("2", Some(1280), Some(720)),
            // height='720p' is invalid; the label supplies 1080.
            ("1080p", None, Some(1080)),
        ]
    );
    info.select_format(Some("worst")).unwrap();
    assert_eq!(info.format_id.as_deref(), Some("0"));
    info.select_format(Some("1")).unwrap();
    assert_eq!((info.width, info.height), (None, Some(480)));
    info.select_format(None).unwrap();
    assert_eq!((info.width, info.height), (None, Some(1080)));
}

#[test]
fn html5_formats_carry_upstream_content_type_codecs() {
    let server = Server::start();
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["-J", &server.url("/codecs.html")])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let info: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let codecs = |entry: usize| -> Vec<_> {
        info["entries"][entry]["formats"]
            .as_array()
            .unwrap()
            .iter()
            .map(|format| {
                (
                    format.get("vcodec").cloned(),
                    format.get("acodec").cloned(),
                    format["dynamic_range"].clone(),
                )
            })
            .collect()
    };
    let text = |value: &str| Some(serde_json::json!(value));
    // The element's own src ignores type codecs; <source> codecs keep their
    // acodec, but vcodec is reset to `None` like upstream's
    // `f.update(formats[0])`.
    // Sorted like pinned upstream: the source with a known acodec ranks best.
    assert_eq!(
        codecs(0),
        [
            (
                Some(serde_json::Value::Null),
                None,
                serde_json::json!("SDR")
            ),
            (
                Some(serde_json::Value::Null),
                None,
                serde_json::json!("SDR")
            ),
            (
                Some(serde_json::Value::Null),
                text("mp4a.40.2"),
                serde_json::json!("SDR")
            ),
        ]
    );
    // Inside <audio>, vcodec is `none` (no dynamic range), and the mp3
    // extension fills acodec like process_video_result.
    assert_eq!(
        codecs(1),
        [(text("none"), text("mp3"), serde_json::Value::Null)]
    );
    // Unknown codecs are reported like upstream's write_string warning.
    assert!(String::from_utf8_lossy(&output.stderr).contains("WARNING: Unknown codec newcodec\n"));
}

#[test]
fn generic_direct_links_follow_upstream_content_type_and_extension_rules() {
    let server = Server::start();
    let dl = YoutubeDl::new().unwrap();
    // Checked with pinned upstream: any audio/video type is a direct link,
    // `ext` comes from the URL, then Content-Disposition or mimetype2ext.
    for (path, ext, format_id) in [
        ("/latm.m4a", "m4a", "mp4a-latm"),
        ("/clip.weird", "weird", "x-weird"),
        ("/unnamed", "ts", "mp2t"),
        ("/named", "ogg", "mpeg"),
        // Untyped binary responses: an allowed URL extension or unknown_video.
        ("/opaque.webm", "webm", "0"),
        ("/blob", "unknown_video", "0"),
    ] {
        let info = dl.extract_info(&server.url(path)).unwrap();
        assert_eq!(
            (info.ext.as_str(), info.format_id.as_deref()),
            (ext, Some(format_id)),
            "{path}"
        );
        assert!(info.is_direct());
    }
    // Downloads refuse unusual extensions like upstream's _UnsafeExtensionError.
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["-o", "o.%(ext)s", &server.url("/clip.weird")])
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains(
        "The extracted extension ('weird') is unusual and will be skipped for safety reasons"
    ));
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["-o", "o.%(ext)s", &server.url("/blob")])
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read(directory.path().join("o.unknown_video")).unwrap(),
        b"media"
    );
}

#[test]
fn output_template_dash_streams_media_to_stdout_like_upstream() {
    let server = Server::start();
    let directory = tempfile::tempdir().unwrap();
    // Checked with pinned upstream: media goes to stdout, screen lines to
    // stderr, postprocessors are skipped and sidecars derive from `-`.
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--no-progress", "-o", "-", "--write-info-json", "-x"])
        .arg(server.url("/clip.mp4"))
        .current_dir(directory.path())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert_eq!(output.stdout, b"media bytes");
    assert!(stderr.contains("[info] Writing video metadata as JSON to: -.info.json\n[download] Destination: -\n[download] Download completed\n"), "{stderr}");
    assert!(!stderr.contains("ExtractAudio"));
    let mut names: Vec<_> = std::fs::read_dir(directory.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, ["-.info.json"]);
    // JSON output follows the screen to stderr.
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["-o", "-", "-j"])
        .arg(server.url("/clip.mp4"))
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("\"id\": \"clip\""));
}

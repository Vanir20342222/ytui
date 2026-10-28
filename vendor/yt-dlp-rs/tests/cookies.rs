//! Scoped response cookies and explicit Netscape-file persistence on loopback.
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
use yt_dlp_rs::{CookieJar, HttpOptions, YoutubeDl, YoutubeOptions};
#[derive(Debug, Clone)]
struct Request {
    path: String,
    cookie: String,
}
impl Request {
    fn has(&self, name: &str, value: &str) -> bool {
        self.cookie
            .split(';')
            .map(str::trim)
            .any(|pair| pair == format!("{name}={value}"))
    }
}
struct Server {
    origin: String,
    requests: Arc<Mutex<Vec<Request>>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Server {
    fn new() -> Self {
        Self::with_cross(None)
    }
    fn with_cross(cross: Option<String>) -> Self {
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
                let cookie = text
                    .lines()
                    .filter_map(|line| line.split_once(':'))
                    .filter(|(name, _)| name.eq_ignore_ascii_case("cookie"))
                    .map(|(_, value)| value.trim())
                    .collect::<Vec<_>>()
                    .join("; ");
                record.lock().unwrap().push(Request {
                    path: path.clone(),
                    cookie,
                });
                let (status, mime, body, extra) = match path.as_str() {
                    "/page.html" => (
                        200,
                        "text/html",
                        b"<video src='/media.mp4'><track src='/captions.vtt' srclang='en'></video>"
                            .to_vec(),
                        "Set-Cookie: page=learned; Path=/; HttpOnly\r\n".into(),
                    ),
                    "/media.mp4" => (
                        200,
                        "video/mp4",
                        b"media".to_vec(),
                        "Set-Cookie: probe=seen; Path=/media.mp4\r\n".into(),
                    ),
                    "/captions.vtt" => (
                        200,
                        "text/vtt",
                        b"WEBVTT\n\n".to_vec(),
                        "Set-Cookie: caption=written; Path=/\r\n".into(),
                    ),
                    "/set-only.mp4" => (
                        200,
                        "video/mp4",
                        b"seeded".to_vec(),
                        "Set-Cookie: shared=yes; Path=/\r\n".into(),
                    ),
                    "/outside.mp4" | "/scoped/media.mp4" => {
                        (200, "video/mp4", b"media".to_vec(), String::new())
                    }
                    "/redirect" => (
                        302,
                        "text/plain",
                        Vec::new(),
                        "Set-Cookie: hop=yes; Path=/scoped\r\nLocation: /scoped/media.mp4\r\n"
                            .into(),
                    ),
                    "/cross" => (
                        302,
                        "text/plain",
                        Vec::new(),
                        format!(
                            "Set-Cookie: private=yes; Path=/\r\nLocation: {}\r\n",
                            cross.as_deref().expect("cross host target")
                        ),
                    ),
                    "/attributes.mp4" => (
                        200,
                        "video/mp4",
                        b"media".to_vec(),
                        concat!(
                            "Set-Cookie: alive=ok; Path=/\r\n",
                            "Set-Cookie: expired=bad; Max-Age=0; Path=/\r\n",
                            "Set-Cookie: secure=hidden; Secure; Path=/\r\n",
                            "Set-Cookie: foreign=no; Domain=example.invalid; Path=/\r\n",
                        )
                        .into(),
                    ),
                    "/failure.mp4" => (
                        404,
                        "text/plain",
                        b"missing".to_vec(),
                        "Set-Cookie: failure=learned; Path=/\r\n".into(),
                    ),
                    "/hls.html" => (
                        200,
                        "text/html",
                        b"<video src='/hls/aes-fmp4/index.m3u8'></video>".to_vec(),
                        "Set-Cookie: manifest=hls; Path=/\r\n".into(),
                    ),
                    "/dash.html" => (
                        200,
                        "text/html",
                        b"<video src='/dash/media/manifest.mpd'></video>".to_vec(),
                        "Set-Cookie: manifest=dash; Path=/\r\n".into(),
                    ),
                    _ if path.starts_with("/hls/aes-") || path.starts_with("/dash/media/") => {
                        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
                            .join("tests/fixtures")
                            .join(path.trim_start_matches('/'));
                        let body = std::fs::read(&fixture)
                            .unwrap_or_else(|error| panic!("{fixture:?}: {error}"));
                        let mime = if path.ends_with(".m3u8") {
                            "application/vnd.apple.mpegurl"
                        } else if path.ends_with(".mpd") {
                            "application/dash+xml"
                        } else {
                            "application/octet-stream"
                        };
                        (200, mime, body, String::new())
                    }
                    _ => (404, "text/plain", b"missing".to_vec(), String::new()),
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} Fixture\r\nContent-Type: {mime}\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(&body);
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
    fn recorded(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
    }
}
fn success(output: &Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
fn seed(path: &Path) {
    std::fs::write(
        path,
        "# Netscape HTTP Cookie File\n127.0.0.1\tFALSE\t/\tFALSE\t0\tseed\tloaded\n",
    )
    .unwrap();
}
fn saved(path: &Path, name: &str, value: &str) -> bool {
    std::fs::read_to_string(path).unwrap().lines().any(|line| {
        let parts: Vec<_> = line.split('\t').collect();
        parts.len() == 7 && parts[5] == name && parts[6] == value
    })
}

#[test]
fn default_in_memory_cookie_jar_learns_page_probe_and_caption_cookies_before_media_download() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    success(&server.cli(
        &["/page.html"],
        &["--write-subs", "-o", "out.mp4"],
        directory.path(),
    ));
    assert_eq!(
        std::fs::read(directory.path().join("out.mp4")).unwrap(),
        b"media"
    );
    assert_eq!(
        std::fs::read(directory.path().join("out.en.vtt")).unwrap(),
        b"WEBVTT\n\n"
    );
    let requests = server.recorded();
    assert_eq!(
        requests.iter().map(|r| r.path.as_str()).collect::<Vec<_>>(),
        ["/page.html", "/media.mp4", "/captions.vtt", "/media.mp4"]
    );
    assert!(requests[0].cookie.is_empty());
    assert!(requests[1].has("page", "learned"));
    assert!(requests[2].has("page", "learned"));
    assert!(!requests[2].has("probe", "seen"));
    assert!(requests[3].has("probe", "seen") && requests[3].has("caption", "written"));
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 2);
}

#[test]
fn cookie_files_load_before_metadata_requests_and_save_learned_session_cookies_in_all_readonly_modes()
 {
    for mode in ["-j", "-s", "-F"] {
        let server = Server::new();
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("cookies.txt");
        seed(&file);
        success(&server.cli(
            &["/set-only.mp4"],
            &[mode, "--cookies", file.to_str().unwrap()],
            directory.path(),
        ));
        assert!(server.recorded()[0].has("seed", "loaded"));
        assert!(saved(&file, "seed", "loaded"));
        assert!(saved(&file, "shared", "yes"));
        let text = std::fs::read_to_string(&file).unwrap();
        let session = text
            .lines()
            .find(|line| line.ends_with("\tshared\tyes"))
            .unwrap();
        assert_eq!(session.split('\t').nth(4), Some("0"));
    }
}

#[test]
fn same_client_shares_cookies_across_inputs_and_redirect_cookie_path_scope_is_enforced() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    success(&server.cli(
        &["/set-only.mp4", "/redirect", "/outside.mp4"],
        &["-j"],
        directory.path(),
    ));
    let requests = server.recorded();
    assert_eq!(requests.len(), 4);
    assert!(requests[1].has("shared", "yes"));
    assert!(requests[2].has("hop", "yes") && requests[2].has("shared", "yes"));
    assert!(!requests[3].has("hop", "yes"));
    assert!(requests[3].has("shared", "yes"));
}

#[test]
fn cookies_reject_foreign_domains_expiration_insecure_secure_use_and_cross_host_leaks() {
    use reqwest::{cookie::CookieStore, header::HeaderValue};
    let target = Server::new();
    let cross = Server::with_cross(Some(
        target.url("/outside.mp4").replace("127.0.0.1", "localhost"),
    ));
    let directory = tempfile::tempdir().unwrap();
    success(&cross.cli(
        &["/attributes.mp4", "/outside.mp4", "/cross"],
        &["-j"],
        directory.path(),
    ));
    let requests = cross.recorded();
    assert!(requests[1].has("alive", "ok"));
    for request in requests.iter().skip(1) {
        assert!(!request.has("expired", "bad"));
        assert!(!request.has("secure", "hidden"));
        assert!(!request.has("foreign", "no"));
    }
    let requests = target.recorded();
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0].cookie.is_empty(),
        "host-only cookie leaked to localhost: {requests:?}"
    );
    // Check ordinary secure/insecure origins through the exact reqwest trait too.
    let jar = CookieJar::default();
    let secure_url = url::Url::parse("https://cookies.example.test/set").unwrap();
    let insecure_url = url::Url::parse("http://cookies.example.test/set").unwrap();
    let header = HeaderValue::from_static("secure=hidden; Secure; Path=/");
    jar.set_cookies(&mut [&header].into_iter(), &secure_url);
    assert_eq!(jar.cookies(&secure_url).unwrap(), "secure=hidden");
    assert!(jar.cookies(&insecure_url).is_none());
}

#[test]
fn extraction_failure_still_flushes_response_cookies_and_returns_failure() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("new-cookies.txt");
    let output = server.cli(
        &["/failure.mp4"],
        &["-j", "--cookies", file.to_str().unwrap()],
        directory.path(),
    );
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(saved(&file, "failure", "learned"));
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn no_cookies_overrides_file_loading_and_saving_but_keeps_the_in_memory_jar_and_last_flag_wins() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let invalid = directory.path().join("invalid.txt");
    std::fs::write(&invalid, [0xff]).unwrap();
    success(&server.cli(
        &["/page.html"],
        &["-j", "--cookies", invalid.to_str().unwrap(), "--no-cookies"],
        directory.path(),
    ));
    assert!(server.recorded()[1].has("page", "learned"));
    assert_eq!(std::fs::read(&invalid).unwrap(), [0xff]);
    let file = directory.path().join("valid.txt");
    seed(&file);
    success(&server.cli(
        &["/set-only.mp4"],
        &["-j", "--no-cookies", "--cookies", file.to_str().unwrap()],
        directory.path(),
    ));
    assert!(server.recorded().last().unwrap().has("seed", "loaded"));
    assert!(saved(&file, "shared", "yes"));
}

#[test]
fn page_cookies_reach_hls_encryption_keys_and_dash_manifest_and_selected_fragment_requests() {
    for (page, kind, selector) in [
        ("/hls.html", "hls", None),
        ("/dash.html", "dash", Some("0")),
    ] {
        let server = Server::new();
        let directory = tempfile::tempdir().unwrap();
        let mut args = vec!["-o", "out.mp4"];
        if let Some(selector) = selector {
            args.extend(["-f", selector]);
        }
        success(&server.cli(&[page], &args, directory.path()));
        assert!(
            !std::fs::read(directory.path().join("out.mp4"))
                .unwrap()
                .is_empty()
        );
        let requests = server.recorded();
        for request in requests.iter().skip(1) {
            assert!(request.has("manifest", kind), "{request:?}");
        }
        assert!(
            requests
                .iter()
                .any(|r| r.path.ends_with(".mpd") || r.path.ends_with(".m3u8"))
        );
        assert!(requests.iter().any(|r| r.path.ends_with(".m4s")));
        if kind == "hls" {
            assert!(requests.iter().any(|r| r.path.ends_with("0.key")));
            assert!(requests.iter().any(|r| r.path.ends_with("1.key")));
        }
    }
}

#[test]
fn explicit_library_arc_jar_loads_learns_across_requests_and_saves_on_demand() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("input.txt");
    let output = directory.path().join("output.txt");
    seed(&input);
    let jar = Arc::new(CookieJar::load(&input).unwrap());
    let dl = YoutubeDl::with_options(
        YoutubeOptions::default(),
        HttpOptions {
            cookies: Some(jar.clone()),
            ..Default::default()
        },
    )
    .unwrap();
    let info = dl.extract_info(&server.url("/set-only.mp4")).unwrap();
    let observer = YoutubeDl::with_options(
        YoutubeOptions::default(),
        HttpOptions {
            cookies: Some(jar.clone()),
            ..Default::default()
        },
    )
    .unwrap();
    observer.extract_info(&server.url("/outside.mp4")).unwrap();
    dl.download(&info, directory.path().join("out.mp4"))
        .unwrap();
    jar.save(&output).unwrap();
    assert_eq!(
        std::fs::read(directory.path().join("out.mp4")).unwrap(),
        b"seeded"
    );
    assert!(saved(&output, "seed", "loaded"));
    assert!(saved(&output, "shared", "yes"));
    assert!(!saved(&input, "shared", "yes"));
    let requests = server.recorded();
    assert_eq!(requests.len(), 3);
    assert!(requests[0].has("seed", "loaded"));
    for request in requests.iter().skip(1) {
        assert!(request.has("seed", "loaded") && request.has("shared", "yes"));
    }
}

#[test]
fn invalid_cookie_files_symlinks_nonregular_paths_and_raw_cookie_headers_fail_before_http() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let invalid = directory.path().join("invalid.txt");
    std::fs::write(&invalid, [0xff]).unwrap();
    let paths = vec![invalid.clone(), directory.path().to_path_buf()];
    #[cfg(unix)]
    let paths = {
        let mut paths = paths;
        use std::os::unix::fs::symlink;
        let dangling = directory.path().join("dangling");
        symlink("missing", &dangling).unwrap();
        paths.push(dangling);
        let regular = directory.path().join("regular-link");
        symlink(&invalid, &regular).unwrap();
        paths.push(regular);
        paths
    };
    for path in paths {
        let output = server.cli(
            &["/set-only.mp4"],
            &["-j", "--cookies", path.to_str().unwrap()],
            directory.path(),
        );
        assert!(!output.status.success(), "{path:?}");
        assert!(output.stdout.is_empty());
    }
    let output = server.cli(
        &["/set-only.mp4"],
        &["-j", "--add-headers", "Cookie:raw=value"],
        directory.path(),
    );
    assert!(!output.status.success());
    assert!(server.recorded().is_empty());
}

#[test]
fn missing_cookie_path_that_becomes_media_output_cannot_overwrite_the_download_at_exit() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let output = server.cli(
        &["/set-only.mp4"],
        &["--cookies", "out.mp4", "-o", "out.mp4"],
        directory.path(),
    );
    assert!(!output.status.success());
    assert_eq!(
        std::fs::read(directory.path().join("out.mp4")).unwrap(),
        b"seeded"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .to_ascii_lowercase()
            .contains("cookie")
    );
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    assert_eq!(server.recorded().len(), 2);
}

#[test]
fn repeated_cookie_file_options_load_and_save_only_the_last_path() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let old = directory.path().join("old-invalid.txt");
    let selected = directory.path().join("selected.txt");
    std::fs::write(&old, [0xff]).unwrap();
    seed(&selected);
    success(&server.cli(
        &["/set-only.mp4"],
        &[
            "-j",
            "--cookies",
            old.to_str().unwrap(),
            "--cookies",
            selected.to_str().unwrap(),
        ],
        directory.path(),
    ));
    assert!(server.recorded()[0].has("seed", "loaded"));
    assert!(saved(&selected, "shared", "yes"));
    assert_eq!(std::fs::read(old).unwrap(), [0xff]);
}

#[test]
fn cookie_file_cannot_alias_the_download_archive() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.txt");
    let bytes = b"# Netscape HTTP Cookie File\n";
    std::fs::write(&path, bytes).unwrap();
    let output = server.cli(
        &["/media.mp4"],
        &[
            "--cookies",
            "state.txt",
            "--download-archive",
            "./state.txt",
        ],
        directory.path(),
    );
    assert!(!output.status.success());
    assert!(server.recorded().is_empty());
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}

//! Request defaults through the CLI/library and the native HTTP resource paths.
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
use yt_dlp_rs::{HttpOptions, YoutubeDl, YoutubeOptions};
#[derive(Debug, Clone)]
struct Request {
    path: String,
    headers: Vec<(String, String)>,
}
impl Request {
    fn values(&self, name: &str) -> Vec<&str> {
        self.headers
            .iter()
            .filter(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
            .collect()
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
        Self::redirecting_to(None)
    }
    fn redirecting_to(cross: Option<String>) -> Self {
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
                let headers = text
                    .lines()
                    .skip(1)
                    .filter_map(|line| line.split_once(':'))
                    .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_owned()))
                    .collect();
                let count = {
                    let mut requests = record.lock().unwrap();
                    requests.push(Request {
                        path: path.clone(),
                        headers,
                    });
                    requests
                        .iter()
                        .filter(|request| request.path == path)
                        .count()
                };
                let (status, mime, body, extra) = match path.as_str() {
                    "/retry.html" if count == 1 => (
                        503,
                        "text/plain",
                        b"retry".to_vec(),
                        "Retry-After: 0\r\n".into(),
                    ),
                    "/page.html" | "/retry.html" => (
                        200,
                        "text/html",
                        b"<video src='/media.mp4'><track src='/captions.vtt' srclang='en'></video>"
                            .to_vec(),
                        String::new(),
                    ),
                    "/retry.mp4" if count == 2 => (
                        503,
                        "text/plain",
                        b"retry".to_vec(),
                        "Retry-After: 0\r\n".into(),
                    ),
                    "/media.mp4" | "/retry.mp4" => {
                        (200, "video/mp4", b"media".to_vec(), String::new())
                    }
                    "/captions.vtt" => (200, "text/vtt", b"WEBVTT\n\n".to_vec(), String::new()),
                    "/redirect" => (
                        302,
                        "text/plain",
                        Vec::new(),
                        "Location: /media.mp4\r\n".into(),
                    ),
                    "/cross" => (
                        302,
                        "text/plain",
                        Vec::new(),
                        format!(
                            "Location: {}\r\n",
                            cross.as_deref().expect("cross-origin target")
                        ),
                    ),
                    _ if path.starts_with("/hls/aes-") || path.starts_with("/dash/media/") => {
                        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
                            .join("tests/fixtures")
                            .join(path.trim_start_matches('/'));
                        let body =
                            std::fs::read(&fixture).unwrap_or_else(|e| panic!("{fixture:?}: {e}"));
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
    fn cli(&self, path: &str, args: &[&str], directory: &Path) -> Output {
        Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .args(args)
            .arg(self.url(path))
            .current_dir(directory)
            .output()
            .unwrap()
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
fn configured() -> YoutubeDl {
    YoutubeDl::with_options(
        YoutubeOptions::default(),
        HttpOptions {
            proxy: None,
            cookies: None,
            headers: vec![
                ("X-Token".into(), "local-value".into()),
                ("Accept-Encoding".into(), "gzip".into()),
            ],
            user_agent: Some("header-fixture/1".into()),
            referer: Some("https://referrer.invalid/start".into()),
            ..Default::default()
        },
    )
    .unwrap()
}
fn assert_defaults(requests: &[Request]) {
    for request in requests {
        assert_eq!(request.values("x-token"), ["local-value"], "{request:?}");
        assert_eq!(
            request.values("user-agent"),
            ["header-fixture/1"],
            "{request:?}"
        );
        assert_eq!(
            request.values("accept-encoding"),
            ["identity"],
            "{request:?}"
        );
    }
}

#[test]
fn cli_headers_ua_duplicates_first_colon_and_referrer_rules_cover_page_media_and_subtitles() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    success(&server.cli(
        "/page.html",
        &[
            "--add-headers",
            "X-Trace: first:part ",
            "--add-headers",
            "x-trace: final:part ",
            "--add-headers",
            "X-Empty:",
            "--add-headers",
            "Accept-Encoding:gzip",
            "--user-agent",
            "cli-header-fixture/1",
            "--referer",
            "https://referrer.invalid/start",
            "--write-subs",
            "-o",
            "out.mp4",
        ],
        directory.path(),
    ));
    assert_eq!(
        std::fs::read(directory.path().join("out.en.vtt")).unwrap(),
        b"WEBVTT\n\n"
    );
    assert_eq!(
        std::fs::read(directory.path().join("out.mp4")).unwrap(),
        b"media"
    );
    let requests = server.recorded();
    assert_eq!(
        requests.iter().map(|r| r.path.as_str()).collect::<Vec<_>>(),
        ["/page.html", "/media.mp4", "/captions.vtt", "/media.mp4"]
    );
    for request in &requests {
        assert_eq!(request.values("x-trace"), ["final:part"]);
        assert_eq!(request.values("x-empty"), [""]);
        assert_eq!(request.values("user-agent"), ["cli-header-fixture/1"]);
        assert_eq!(request.values("accept-encoding"), ["identity"]);
        let referer = if request.path == "/page.html" {
            "https://referrer.invalid/start".into()
        } else {
            server.url("/page.html")
        };
        assert_eq!(request.values("referer"), [referer]);
    }
}

#[test]
fn library_defaults_support_http_token_names_and_tabs_for_direct_probe_and_download() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let dl = YoutubeDl::with_options(
        YoutubeOptions::default(),
        HttpOptions {
            proxy: None,
            cookies: None,
            headers: vec![("X!Token".into(), "one\ttwo".into())],
            user_agent: Some("library-fixture/1".into()),
            referer: Some("https://referrer.invalid/direct".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let info = dl.extract_info(&server.url("/media.mp4")).unwrap();
    dl.download(&info, directory.path().join("media.mp4"))
        .unwrap();
    let requests = server.recorded();
    assert_eq!(requests.len(), 2);
    for request in requests {
        assert_eq!(request.values("x!token"), ["one\ttwo"]);
        assert_eq!(request.values("user-agent"), ["library-fixture/1"]);
        assert_eq!(
            request.values("referer"),
            ["https://referrer.invalid/direct"]
        );
        assert_eq!(request.values("accept-encoding"), ["identity"]);
    }
}

#[test]
fn existing_constructors_keep_default_user_agent_and_direct_requests_have_no_invented_referrer() {
    let server = Server::new();
    YoutubeDl::new()
        .unwrap()
        .extract_info(&server.url("/media.mp4"))
        .unwrap();
    let requests = server.recorded();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].values("user-agent"),
        [concat!("yt-dlp-rs/", env!("CARGO_PKG_VERSION"))]
    );
    assert!(requests[0].values("referer").is_empty());
}

#[test]
fn headers_reach_hls_manifest_rotating_keys_initialization_and_encrypted_fragments() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let dl = configured();
    let url = server.url("/hls/aes-fmp4/index.m3u8");
    let info = dl.extract_info(&url).unwrap();
    let target = directory.path().join("out.mp4");
    dl.download(&info, &target).unwrap();
    let mut expected = include_bytes!("fixtures/hls/fmp4/init.mp4").to_vec();
    expected.extend_from_slice(include_bytes!("fixtures/hls/fmp4/segment-0.m4s"));
    expected.extend_from_slice(include_bytes!("fixtures/hls/fmp4/segment-1.m4s"));
    expected.extend_from_slice(include_bytes!("fixtures/hls/fmp4/segment-2.m4s"));
    assert_eq!(std::fs::read(target).unwrap(), expected);
    let requests = server.recorded();
    assert_defaults(&requests);
    assert!(requests.iter().any(|r| r.path.ends_with("0.key")));
    assert!(requests.iter().any(|r| r.path.ends_with("1.key")));
    assert!(requests.iter().any(|r| r.path.ends_with("init.mp4")));
    for request in &requests {
        let expected = if request.path.ends_with("index.m3u8") {
            "https://referrer.invalid/start"
        } else {
            &url
        };
        assert_eq!(request.values("referer"), [expected]);
    }
}

#[test]
fn headers_reach_dash_manifest_and_selected_stream_init_and_fragments() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let dl = configured();
    let url = server.url("/dash/media/manifest.mpd");
    let info = dl.extract_info_with_format(&url, Some("0")).unwrap();
    let target = directory.path().join("video.mp4");
    dl.download(&info, &target).unwrap();
    assert_eq!(
        std::fs::read(target).unwrap(),
        include_bytes!("fixtures/dash/media/video-combined.mp4")
    );
    let requests = server.recorded();
    assert_eq!(requests.len(), 5);
    assert_defaults(&requests);
    for request in &requests {
        let expected = if request.path.ends_with(".mpd") {
            "https://referrer.invalid/start"
        } else {
            &url
        };
        assert_eq!(request.values("referer"), [expected]);
    }
}

#[test]
fn extractor_and_download_retries_keep_configured_headers() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let flags = [
        "--add-headers",
        "X-Token:local-value",
        "--user-agent",
        "header-fixture/1",
        "--referer",
        "https://referrer.invalid/start",
    ];
    let mut args = flags.to_vec();
    args.extend(["--extractor-retries", "1", "-o", "page.mp4"]);
    success(&server.cli("/retry.html", &args, directory.path()));
    let mut args = flags.to_vec();
    args.extend(["--retries", "1", "-o", "direct.mp4"]);
    success(&server.cli("/retry.mp4", &args, directory.path()));
    let requests = server.recorded();
    assert_eq!(
        requests.iter().filter(|r| r.path == "/retry.html").count(),
        2
    );
    assert_eq!(
        requests.iter().filter(|r| r.path == "/retry.mp4").count(),
        3
    );
    assert_defaults(&requests);
}

#[test]
fn redirects_keep_custom_headers_but_strip_authorization_across_origins() {
    let target = Server::new();
    let server = Server::redirecting_to(Some(target.url("/media.mp4")));
    let directory = tempfile::tempdir().unwrap();
    let args = [
        "-j",
        "--add-headers",
        "X-Custom:visible",
        "--add-headers",
        "Authorization:Bearer local-test-token",
    ];
    success(&server.cli("/redirect", &args, directory.path()));
    success(&server.cli("/cross", &args, directory.path()));
    for request in server.recorded() {
        assert_eq!(request.values("x-custom"), ["visible"]);
        assert_eq!(request.values("authorization"), ["Bearer local-test-token"]);
    }
    let target_requests = target.recorded();
    assert_eq!(target_requests.len(), 1);
    assert_eq!(target_requests[0].values("x-custom"), ["visible"]);
    assert!(target_requests[0].values("authorization").is_empty());
}

#[test]
fn invalid_cli_header_configuration_is_rejected_before_http_or_output() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let oversized = format!("X-Long:{}", "x".repeat(16 * 1024 + 1));
    for args in [
        vec!["-j", "--add-headers", "missing-colon"],
        vec!["-j", "--add-headers", ":empty-name"],
        vec!["-j", "--add-headers", "X!Token:value"],
        vec!["-j", "--add-headers", "X-Trace:a\r\nInjected: yes"],
        vec!["-j", "--add-headers", "Cookie:"],
        vec!["-j", "--user-agent", "bad\nagent"],
        vec!["-j", "--referer", "bad\rreferrer"],
        vec!["-j", "--add-headers", oversized.as_str()],
    ] {
        let output = server.cli("/media.mp4", &args, directory.path());
        assert!(!output.status.success(), "{args:?}");
        assert!(output.stdout.is_empty());
    }
    assert!(server.recorded().is_empty());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn library_header_controls_cookie_and_resource_limits_fail_during_construction() {
    let server = Server::new();
    let cases = vec![
        vec![("Bad Name".into(), "value".into())],
        vec![("X-Control".into(), "a\u{7f}b".into())],
        vec![("cOoKiE".into(), String::new())],
        vec![("X-Long".into(), "x".repeat(16 * 1024 + 1))],
        (0..257)
            .map(|n| (format!("X-{n}"), "value".into()))
            .collect(),
        (0..240)
            .map(|n| (format!("X-{n}"), "x".repeat(300)))
            .collect(),
    ];
    for headers in cases {
        assert!(
            YoutubeDl::with_options(
                YoutubeOptions::default(),
                HttpOptions {
                    proxy: None,
                    cookies: None,
                    headers,
                    ..Default::default()
                }
            )
            .is_err()
        );
    }
    assert!(server.recorded().is_empty());
}

#[test]
fn shared_pinned_header_option_cases_reach_the_actual_cli_http_request() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/headers/cases.json")).unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let server = Server::new();
        let directory = tempfile::tempdir().unwrap();
        let mut args: Vec<_> = case["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|arg| arg.as_str().unwrap())
            .collect();
        args.push("-j");
        let output = server.cli("/media.mp4", &args, directory.path());
        assert!(
            output.status.success(),
            "{}: {}",
            case["name"],
            String::from_utf8_lossy(&output.stderr)
        );
        let requests = server.recorded();
        assert_eq!(requests.len(), 1, "{}", case["name"]);
        for (name, value) in case["expected_headers"].as_object().unwrap() {
            assert_eq!(
                requests[0].values(name),
                [value.as_str().unwrap()],
                "{}: {name}",
                case["name"]
            );
        }
    }
    for case in fixture["invalid_cases"].as_array().unwrap() {
        let server = Server::new();
        let directory = tempfile::tempdir().unwrap();
        let mut args: Vec<_> = case["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|arg| arg.as_str().unwrap())
            .collect();
        args.push("-j");
        let output = server.cli("/media.mp4", &args, directory.path());
        assert!(!output.status.success(), "{}", case["name"]);
        assert!(output.stdout.is_empty());
        assert!(server.recorded().is_empty(), "{}", case["name"]);
    }
}

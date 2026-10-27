//! Local HTTP and SOCKS proxy fixtures: no destination DNS or external network.
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
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
    method: String,
    target: String,
    headers: Vec<(String, String)>,
}
impl Request {
    fn value(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}
fn read_request(stream: &mut TcpStream) -> Request {
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut bytes = Vec::new();
    let mut byte = [0];
    while !bytes.ends_with(b"\r\n\r\n") {
        assert!(bytes.len() < 64 * 1024, "oversized local request");
        assert_eq!(stream.read(&mut byte).unwrap(), 1);
        bytes.push(byte[0]);
    }
    let text = String::from_utf8(bytes).unwrap();
    let mut lines = text.lines();
    let mut first = lines.next().unwrap().split_whitespace();
    let method = first.next().unwrap().to_owned();
    let target = first.next().unwrap().to_owned();
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_owned()))
        .collect();
    Request {
        method,
        target,
        headers,
    }
}
fn send(stream: &mut TcpStream, status: u16, mime: &str, body: &[u8], extra: &str) {
    let _ = write!(
        stream,
        "HTTP/1.1 {status} Fixture\r\nContent-Type: {mime}\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(body);
}
struct Proxy {
    url: String,
    requests: Arc<Mutex<Vec<Request>>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Proxy {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
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
                let request = read_request(&mut stream);
                let path = if request.method == "CONNECT" {
                    String::new()
                } else {
                    url::Url::parse(&request.target)
                        .map(|url| url.path().to_owned())
                        .unwrap_or_else(|_| request.target.clone())
                };
                let count = {
                    let mut requests = record.lock().unwrap();
                    requests.push(request.clone());
                    requests
                        .iter()
                        .filter(|r| r.target == request.target)
                        .count()
                };
                if request.method == "CONNECT" {
                    send(
                        &mut stream,
                        407,
                        "text/plain",
                        b"proxy auth required",
                        "Proxy-Authenticate: Basic realm=\"fixture\"\r\n",
                    );
                    continue;
                }
                let (status, mime, body, extra) = match path.as_str() {
                    "/page.html" => (
                        200,
                        "text/html",
                        b"<video src='/media.mp4'><track src='/captions.vtt' srclang='en'></video>"
                            .to_vec(),
                        "Set-Cookie: proxy=learned; Path=/\r\n".into(),
                    ),
                    "/media.mp4" => (200, "video/mp4", b"proxy-media".to_vec(), String::new()),
                    "/captions.vtt" => (200, "text/vtt", b"WEBVTT\n\n".to_vec(), String::new()),
                    "/redirect" => (
                        302,
                        "text/plain",
                        Vec::new(),
                        "Location: http://other.invalid/media.mp4\r\n".into(),
                    ),
                    "/retry.mp4" if count == 1 => (
                        503,
                        "text/plain",
                        b"retry".to_vec(),
                        "Retry-After: 0\r\n".into(),
                    ),
                    "/retry.mp4" => (200, "video/mp4", b"retried".to_vec(), String::new()),
                    "/hls.html" => (
                        200,
                        "text/html",
                        b"<video src='/hls/aes-fmp4/index.m3u8'></video>".to_vec(),
                        "Set-Cookie: proxy=learned; Path=/\r\n".into(),
                    ),
                    "/dash.html" => (
                        200,
                        "text/html",
                        b"<video src='/dash/media/manifest.mpd'></video>".to_vec(),
                        "Set-Cookie: proxy=learned; Path=/\r\n".into(),
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
                send(&mut stream, status, mime, &body, &extra);
            }
        });
        Self {
            url,
            requests,
            stop,
            worker: Some(worker),
        }
    }
    fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }
    fn origin(&self, path: &str) -> String {
        format!("{}{path}", self.url)
    }
}
impl Drop for Proxy {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
    }
}
fn command(url: &str, args: &[&str], directory: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"));
    command.args(args).arg(url).current_dir(directory);
    for name in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "NO_PROXY",
        "no_proxy",
    ] {
        command.env_remove(name);
    }
    command
}
fn success(output: &Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
fn through(proxy: &Proxy, url: &str, args: &[&str], directory: &Path) -> Output {
    let mut flags = vec!["--proxy", proxy.url.as_str()];
    flags.extend_from_slice(args);
    command(url, &flags, directory)
        .env("NO_PROXY", "*")
        .output()
        .unwrap()
}

#[test]
fn explicit_proxy_overrides_no_proxy_for_page_probes_subtitles_and_media_with_cookies_and_referrer()
{
    let proxy = Proxy::new();
    let directory = tempfile::tempdir().unwrap();
    success(&through(
        &proxy,
        "http://destination.invalid/page.html",
        &[
            "--write-subs",
            "--add-headers",
            "X-Token:proxy-test",
            "-o",
            "out.mp4",
        ],
        directory.path(),
    ));
    assert_eq!(
        std::fs::read(directory.path().join("out.mp4")).unwrap(),
        b"proxy-media"
    );
    assert_eq!(
        std::fs::read(directory.path().join("out.en.vtt")).unwrap(),
        b"WEBVTT\n\n"
    );
    let requests = proxy.requests();
    assert_eq!(requests.len(), 4);
    for (index, request) in requests.iter().enumerate() {
        assert!(
            request.target.starts_with("http://destination.invalid/"),
            "{request:?}"
        );
        assert_eq!(request.value("host"), Some("destination.invalid"));
        assert_eq!(request.value("x-token"), Some("proxy-test"));
        assert_eq!(request.value("accept-encoding"), Some("identity"));
        if index > 0 {
            assert_eq!(
                request.value("referer"),
                Some("http://destination.invalid/page.html")
            );
            assert!(request.value("cookie").unwrap().contains("proxy=learned"));
        }
    }
}

#[test]
fn library_http_proxy_uses_absolute_targets_and_percent_decoded_proxy_credentials() {
    let proxy = Proxy::new();
    let directory = tempfile::tempdir().unwrap();
    let uri = proxy.url.replace("http://", "http://us%65r:p%40ss@");
    let dl = YoutubeDl::with_options(
        YoutubeOptions::default(),
        HttpOptions {
            proxy: Some(uri),
            ..Default::default()
        },
    )
    .unwrap();
    let info = dl
        .extract_info("http://destination.invalid/media.mp4")
        .unwrap();
    dl.download(&info, directory.path().join("out.mp4"))
        .unwrap();
    for request in proxy.requests() {
        assert_eq!(request.target, "http://destination.invalid/media.mp4");
        assert_eq!(
            request.value("proxy-authorization"),
            Some("Basic dXNlcjpwQHNz")
        );
        assert!(request.value("authorization").is_none());
    }
}

#[test]
fn environment_default_explicit_direct_and_repeated_last_proxy_options_are_isolated_per_subprocess()
{
    let proxy = Proxy::new();
    let directory = tempfile::tempdir().unwrap();
    success(
        &command(
            "http://destination.invalid/media.mp4",
            &["-j"],
            directory.path(),
        )
        .env("HTTP_PROXY", &proxy.url)
        .env("NO_PROXY", "")
        .output()
        .unwrap(),
    );
    assert_eq!(proxy.requests().len(), 1);
    let direct = Proxy::new();
    success(
        &command(
            &direct.origin("/media.mp4"),
            &["-j", "--proxy", ""],
            directory.path(),
        )
        .env("HTTP_PROXY", "http://127.0.0.1:1")
        .env("ALL_PROXY", "http://127.0.0.1:1")
        .env("NO_PROXY", "")
        .output()
        .unwrap(),
    );
    assert_eq!(direct.requests()[0].target, "/media.mp4");
    success(
        &command(
            "http://destination.invalid/media.mp4",
            &[
                "-j",
                "--proxy",
                "invalid-old-scheme://unused",
                "--proxy",
                proxy.url.as_str(),
            ],
            directory.path(),
        )
        .env("NO_PROXY", "*")
        .output()
        .unwrap(),
    );
    assert_eq!(proxy.requests().len(), 2);
    let bare = proxy.url.trim_start_matches("http://");
    success(
        &command(
            "http://destination.invalid/media.mp4",
            &["-j", "--proxy", bare],
            directory.path(),
        )
        .output()
        .unwrap(),
    );
    assert_eq!(proxy.requests().len(), 3);
}

#[test]
fn proxy_routes_hls_rotating_keys_initialization_and_dash_selected_fragments() {
    for (page, selector, kind) in [("hls.html", None, "hls"), ("dash.html", Some("0"), "dash")] {
        let proxy = Proxy::new();
        let directory = tempfile::tempdir().unwrap();
        let mut args = vec!["-o", "out.mp4"];
        if let Some(selector) = selector {
            args.extend(["-f", selector]);
        }
        success(&through(
            &proxy,
            &format!("http://destination.invalid/{page}"),
            &args,
            directory.path(),
        ));
        assert!(
            !std::fs::read(directory.path().join("out.mp4"))
                .unwrap()
                .is_empty()
        );
        let requests = proxy.requests();
        for request in requests.iter().skip(1) {
            assert!(request.target.starts_with("http://destination.invalid/"));
            assert_eq!(
                request.value("referer"),
                Some(format!("http://destination.invalid/{page}").as_str())
            );
            assert!(request.value("cookie").unwrap().contains("proxy=learned"));
        }
        assert!(requests.iter().any(|r| r.target.ends_with(".m4s")));
        if kind == "hls" {
            assert!(requests.iter().any(|r| r.target.ends_with("0.key")));
            assert!(requests.iter().any(|r| r.target.ends_with("1.key")));
        } else {
            assert!(requests.iter().any(|r| r.target.ends_with("manifest.mpd")));
        }
    }
}

#[test]
fn redirects_and_whole_extraction_retries_continue_using_the_explicit_proxy() {
    let proxy = Proxy::new();
    let directory = tempfile::tempdir().unwrap();
    success(&through(
        &proxy,
        "http://destination.invalid/redirect",
        &["-j"],
        directory.path(),
    ));
    success(&through(
        &proxy,
        "http://destination.invalid/retry.mp4",
        &["-j", "--extractor-retries", "1"],
        directory.path(),
    ));
    let requests = proxy.requests();
    assert_eq!(
        requests
            .iter()
            .map(|r| r.target.as_str())
            .collect::<Vec<_>>(),
        [
            "http://destination.invalid/redirect",
            "http://other.invalid/media.mp4",
            "http://destination.invalid/retry.mp4",
            "http://destination.invalid/retry.mp4"
        ]
    );
    assert_eq!(requests[1].value("host"), Some("other.invalid"));
}

#[derive(Debug)]
struct SocksRequest {
    version: u8,
    host: String,
    port: u16,
    user: Option<String>,
    password: Option<String>,
    http: Request,
}
struct Socks {
    port: u16,
    requests: Arc<Mutex<Vec<SocksRequest>>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
fn take(stream: &mut TcpStream, len: usize) -> Vec<u8> {
    let mut bytes = vec![0; len];
    stream.read_exact(&mut bytes).unwrap();
    bytes
}
fn zero_terminated(stream: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    loop {
        assert!(bytes.len() < 1024);
        let byte = take(stream, 1)[0];
        if byte == 0 {
            break;
        }
        bytes.push(byte);
    }
    String::from_utf8(bytes).unwrap()
}
impl Socks {
    fn new(auth: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
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
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let version = take(&mut stream, 1)[0];
                let (host, port, user, password) = if version == 4 {
                    let header = take(&mut stream, 7);
                    assert_eq!(header[0], 1);
                    let port = u16::from_be_bytes([header[1], header[2]]);
                    let user = zero_terminated(&mut stream);
                    let ip = &header[3..7];
                    let host = if ip[0..3] == [0, 0, 0] && ip[3] != 0 {
                        zero_terminated(&mut stream)
                    } else {
                        format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3])
                    };
                    stream.write_all(&[0, 90, 0, 0, 0, 0, 0, 0]).unwrap();
                    (host, port, (!user.is_empty()).then_some(user), None)
                } else {
                    assert_eq!(version, 5);
                    let len = take(&mut stream, 1)[0] as usize;
                    let methods = take(&mut stream, len);
                    let method = if auth { 2 } else { 0 };
                    assert!(methods.contains(&method));
                    if !auth {
                        assert_eq!(methods, [0], "incomplete credentials must use NOAUTH only");
                    }
                    stream.write_all(&[5, method]).unwrap();
                    let (user, password) = if auth {
                        assert_eq!(take(&mut stream, 1), [1]);
                        let len = take(&mut stream, 1)[0] as usize;
                        let user = String::from_utf8(take(&mut stream, len)).unwrap();
                        let len = take(&mut stream, 1)[0] as usize;
                        let password = String::from_utf8(take(&mut stream, len)).unwrap();
                        stream.write_all(&[1, 0]).unwrap();
                        (Some(user), Some(password))
                    } else {
                        (None, None)
                    };
                    let command = take(&mut stream, 4);
                    assert_eq!(&command[..3], &[5, 1, 0]);
                    let host = match command[3] {
                        3 => {
                            let len = take(&mut stream, 1)[0] as usize;
                            String::from_utf8(take(&mut stream, len)).unwrap()
                        }
                        1 => {
                            let ip = take(&mut stream, 4);
                            format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3])
                        }
                        other => panic!("unexpected SOCKS address type {other}"),
                    };
                    let port = take(&mut stream, 2);
                    stream.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0]).unwrap();
                    (host, u16::from_be_bytes([port[0], port[1]]), user, password)
                };
                let http = read_request(&mut stream);
                record.lock().unwrap().push(SocksRequest {
                    version,
                    host,
                    port,
                    user,
                    password,
                    http,
                });
                send(&mut stream, 200, "video/mp4", b"socks-media", "");
            }
        });
        Self {
            port,
            requests,
            stop,
            worker: Some(worker),
        }
    }
    fn url(&self, scheme: &str) -> String {
        format!("{scheme}://127.0.0.1:{}", self.port)
    }
}
impl Drop for Socks {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
    }
}

#[test]
fn socks4_4a_and_legacy_alias_negotiate_without_connecting_to_a_destination() {
    for (scheme, host) in [
        ("socks4", "127.0.0.2"),
        ("socks4a", "destination.invalid"),
        ("socks", "127.0.0.2"),
    ] {
        let proxy = Socks::new(false);
        let directory = tempfile::tempdir().unwrap();
        success(
            &command(
                &format!("http://{host}:5678/media.mp4"),
                &["-j", "--proxy", &proxy.url(scheme)],
                directory.path(),
            )
            .output()
            .unwrap(),
        );
        let requests = proxy.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].version, 4);
        assert_eq!(requests[0].host, host);
        assert_eq!(requests[0].port, 5678);
        assert_eq!(requests[0].http.target, "/media.mp4");
    }
}

#[test]
fn socks5_and_5h_send_destination_domain_without_local_dns_or_incomplete_authentication() {
    for (scheme, credentials) in [
        ("socks5", ""),
        ("socks5h", ""),
        ("socks5h", "user@"),
        ("socks5h", ":pass@"),
        ("socks5h", "user:@"),
    ] {
        let proxy = Socks::new(false);
        let directory = tempfile::tempdir().unwrap();
        let uri = proxy
            .url(scheme)
            .replace("://", &format!("://{credentials}"));
        success(
            &command(
                "http://destination.invalid/media.mp4",
                &["-j", "--proxy", &uri],
                directory.path(),
            )
            .env("NO_PROXY", "*")
            .output()
            .unwrap(),
        );
        let requests = proxy.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].version, 5);
        assert_eq!(requests[0].host, "destination.invalid");
        assert_eq!(requests[0].port, 80);
        assert_eq!(requests[0].http.value("host"), Some("destination.invalid"));
    }
}

#[test]
fn socks5_username_password_authentication_percent_decodes_uri_credentials() {
    let proxy = Socks::new(true);
    let directory = tempfile::tempdir().unwrap();
    let uri = proxy
        .url("socks5h")
        .replace("socks5h://", "socks5h://alice:p%40ss@");
    success(
        &command(
            "http://destination.invalid/media.mp4",
            &["-j", "--proxy", &uri],
            directory.path(),
        )
        .output()
        .unwrap(),
    );
    let requests = proxy.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].user.as_deref(), Some("alice"));
    assert_eq!(requests[0].password.as_deref(), Some("p@ss"));
    assert_eq!(requests[0].host, "destination.invalid");
}

#[test]
fn https_destinations_use_connect_and_proxy_authentication_failure_is_explicit() {
    let proxy = Proxy::new();
    let directory = tempfile::tempdir().unwrap();
    let uri = proxy
        .url
        .replace("http://", "http://connect-private-user:connect-secret@");
    let output = command(
        "https://destination.invalid/media.mp4",
        &["-j", "--proxy", &uri],
        directory.path(),
    )
    .output()
    .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("connect-secret"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("connect-private-user"));
    let requests = proxy.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "CONNECT");
    assert_eq!(requests[0].target, "destination.invalid:443");
    assert!(requests[0].value("proxy-authorization").is_some());
}

#[test]
fn invalid_proxy_urls_fail_before_network_and_do_not_expose_credentials() {
    let proxy = Proxy::new();
    let directory = tempfile::tempdir().unwrap();
    for uri in [
        "file://proxy-private-user:proxy-secret@127.0.0.1",
        "http://proxy-private-user:proxy-secret@127.0.0.1:99999",
        "http://proxy-private-user:proxy-secret@127.0.0.1/path",
        "http://proxy-private-user:proxy-secret@127.0.0.1?x=1",
        "http://proxy-private-user:proxy-secret@127.0.0.1#fragment",
        "http://proxy-private-user:proxy-secret@bad host",
        "http://proxy-private-user:proxy-secret@",
    ] {
        assert!(
            HttpOptions {
                proxy: Some(uri.into()),
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        let output = command(
            &proxy.origin("/media.mp4"),
            &["-j", "--proxy", uri],
            directory.path(),
        )
        .output()
        .unwrap();
        assert!(!output.status.success(), "invalid proxy was accepted");
        assert!(output.stdout.is_empty());
        assert!(!String::from_utf8_lossy(&output.stderr).contains("proxy-secret"));
        assert!(!String::from_utf8_lossy(&output.stderr).contains("proxy-private-user"));
    }
    for scheme in ["socks4", "socks4a", "socks"] {
        let uri = proxy.url.replace(
            "http://",
            &format!("{scheme}://proxy-private-user:proxy-secret@"),
        );
        assert!(
            HttpOptions {
                proxy: Some(uri.clone()),
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        let output = command(
            &proxy.origin("/media.mp4"),
            &["-j", "--proxy", &uri],
            directory.path(),
        )
        .output()
        .unwrap();
        assert!(!output.status.success());
        assert!(!String::from_utf8_lossy(&output.stderr).contains("proxy-secret"));
        assert!(!String::from_utf8_lossy(&output.stderr).contains("proxy-private-user"));
    }
    assert!(proxy.requests().is_empty());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn http_proxy_credentials_preserve_empty_and_user_only_fields() {
    for (userinfo, expected) in [
        ("user", "Basic dXNlcjo="),
        ("user:", "Basic dXNlcjo="),
        (":pass", "Basic OnBhc3M="),
        (":", "Basic Og=="),
        ("", "Basic Og=="),
    ] {
        let proxy = Proxy::new();
        let directory = tempfile::tempdir().unwrap();
        let url = format!(
            "http://{userinfo}@{}",
            proxy.url.strip_prefix("http://").unwrap()
        );
        let output = command(
            "http://destination.invalid/media.mp4",
            &["--proxy", &url, "-j"],
            directory.path(),
        )
        .output()
        .unwrap();
        success(&output);
        let requests = proxy.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].value("proxy-authorization"), Some(expected));
        assert!(requests[0].value("authorization").is_none());
    }
}

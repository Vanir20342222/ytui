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
use yt_dlp_rs::{DownloadOptions, Error, ExtractionOptions, YoutubeDl};

struct Response {
    status: u16,
    mime: &'static str,
    body: &'static str,
    length: usize,
    headers: &'static str,
}

impl Response {
    fn media() -> Self {
        Self {
            status: 200,
            mime: "video/mp4",
            body: "complete media",
            length: 14,
            headers: "",
        }
    }

    fn status(status: u16) -> Self {
        Self {
            status,
            ..Self::media()
        }
    }
}

// The first media GET is extraction's probe; subsequent GETs are downloads.
struct Server {
    base: String,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Server {
    fn start(respond: impl Fn(&str, usize) -> Response + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::<String>::new()));
        let recorded = requests.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let shutdown = stop.clone();
        let worker = thread::spawn(move || {
            while !shutdown.load(Ordering::Relaxed) {
                let (mut stream, _) = match listener.accept() {
                    Ok(connection) => connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(error) => panic!("fixture accept failed: {error}"),
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0; 1024];
                while !bytes.windows(4).any(|part| part == b"\r\n\r\n") {
                    match stream.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(length) => bytes.extend_from_slice(&buffer[..length]),
                    }
                }
                if !bytes.windows(4).any(|part| part == b"\r\n\r\n") {
                    continue;
                }
                let request = String::from_utf8(bytes).unwrap();
                let path = request.split_whitespace().nth(1).unwrap().to_owned();
                let count = {
                    let mut requests = recorded.lock().unwrap();
                    requests.push(request);
                    requests
                        .iter()
                        .filter(|request| request.split_whitespace().nth(1) == Some(path.as_str()))
                        .count()
                };
                let response = respond(&path, count);
                let length = if response.length == usize::MAX {
                    String::new()
                } else {
                    format!("Content-Length: {}\r\n", response.length)
                };
                let wire = format!(
                    "HTTP/1.1 {} Fixture\r\nContent-Type: {}\r\n{}{}Connection: close\r\n\r\n{}",
                    response.status, response.mime, length, response.headers, response.body
                );
                let _ = stream.write_all(wire.as_bytes());
            }
        });
        Self {
            base,
            requests,
            stop,
            worker: Some(worker),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    fn requests_for(&self, path: &str) -> Vec<String> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.split_whitespace().nth(1) == Some(path))
            .cloned()
            .collect()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
    }
}

fn directory_names(directory: &std::path::Path) -> Vec<String> {
    let mut names = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<Vec<_>>();
    names.sort();
    names
}

#[test]
fn transient_statuses_retry_and_preserve_complete_output() {
    // Upstream HttpFD retries every 5xx status.
    for status in [500, 501, 502, 503, 504, 599] {
        let server = Server::start(move |_, count| {
            if count == 2 {
                Response::status(status)
            } else {
                Response::media()
            }
        });
        let downloader = YoutubeDl::new().unwrap();
        let info = downloader.extract_info(&server.url("/clip.mp4")).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("clip.mp4");
        assert_eq!(
            downloader
                .download_with_options(
                    &info,
                    &destination,
                    DownloadOptions {
                        retries: 1,
                        ..Default::default()
                    }
                )
                .unwrap(),
            14
        );
        assert_eq!(std::fs::read(&destination).unwrap(), b"complete media");
        assert_eq!(server.requests_for("/clip.mp4").len(), 3, "status {status}");
        assert_eq!(directory_names(directory.path()), ["clip.mp4"]);
    }
}

#[test]
fn truncated_body_restarts_without_range_or_partial_bytes() {
    let server = Server::start(|_, count| {
        if count == 2 {
            Response {
                body: "partial",
                length: 100,
                ..Response::media()
            }
        } else {
            Response::media()
        }
    });
    let downloader = YoutubeDl::new().unwrap();
    let info = downloader.extract_info(&server.url("/clip.mp4")).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("clip.mp4");
    assert_eq!(
        downloader
            .download_with_options(
                &info,
                &destination,
                DownloadOptions {
                    retries: 1,
                    ..Default::default()
                }
            )
            .unwrap(),
        14
    );
    assert_eq!(std::fs::read(destination).unwrap(), b"complete media");
    let requests = server.requests_for("/clip.mp4");
    assert_eq!(requests.len(), 3);
    assert!(
        requests
            .iter()
            .all(|request| !request.to_ascii_lowercase().contains("\r\nrange:"))
    );
    assert_eq!(directory_names(directory.path()), ["clip.mp4"]);
}

#[test]
fn exhausted_retries_count_additional_attempts_and_clean_temporary_files() {
    for truncated in [false, true] {
        let server = Server::start(move |_, count| {
            if count == 1 {
                Response::media()
            } else if truncated {
                Response {
                    body: "partial",
                    length: 100,
                    ..Response::media()
                }
            } else {
                Response::status(503)
            }
        });
        let downloader = YoutubeDl::new().unwrap();
        let info = downloader.extract_info(&server.url("/clip.mp4")).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let error = downloader
            .download_with_options(
                &info,
                directory.path().join("clip.mp4"),
                DownloadOptions {
                    retries: 2,
                    ..Default::default()
                },
            )
            .unwrap_err();
        assert_eq!(
            server.requests_for("/clip.mp4").len(),
            4,
            "truncated={truncated}: {error}"
        );
        assert!(directory_names(directory.path()).is_empty());
    }
}

#[test]
fn permanent_download_failures_are_not_retried() {
    for status in [401, 404, 408, 429, 206] {
        let server = Server::start(move |_, count| {
            if count == 1 {
                Response::media()
            } else {
                Response::status(status)
            }
        });
        let downloader = YoutubeDl::new().unwrap();
        let info = downloader.extract_info(&server.url("/clip.mp4")).unwrap();
        let directory = tempfile::tempdir().unwrap();
        assert!(
            downloader
                .download_with_options(
                    &info,
                    directory.path().join("clip.mp4"),
                    DownloadOptions {
                        retries: 2,
                        ..Default::default()
                    }
                )
                .is_err()
        );
        assert_eq!(server.requests_for("/clip.mp4").len(), 2, "status {status}");
        assert!(directory_names(directory.path()).is_empty());
    }
}

#[test]
fn existing_destination_is_preserved_without_attempting_download() {
    let server = Server::start(|_, _| Response::media());
    let downloader = YoutubeDl::new().unwrap();
    let info = downloader.extract_info(&server.url("/clip.mp4")).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("clip.mp4");
    std::fs::write(&destination, "existing file").unwrap();
    assert!(
        downloader
            .download_with_options(
                &info,
                &destination,
                DownloadOptions {
                    retries: 2,
                    ..Default::default()
                }
            )
            .is_err()
    );
    assert_eq!(std::fs::read(destination).unwrap(), b"existing file");
    assert_eq!(server.requests_for("/clip.mp4").len(), 1);
    assert_eq!(directory_names(directory.path()), ["clip.mp4"]);
}

#[test]
fn html5_page_referrer_is_preserved_on_every_retry() {
    let server = Server::start(|path, count| {
        if path == "/page.html" {
            let body = "<video src='/clip.mp4'></video>";
            Response {
                mime: "text/html",
                body,
                length: body.len(),
                ..Response::media()
            }
        } else if count == 2 {
            Response::status(503)
        } else {
            Response::media()
        }
    });
    let downloader = YoutubeDl::new().unwrap();
    let page_url = server.url("/page.html");
    let info = downloader.extract_info(&page_url).unwrap();
    let directory = tempfile::tempdir().unwrap();
    downloader
        .download_with_options(
            &info,
            directory.path().join("clip.mp4"),
            DownloadOptions {
                retries: 1,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(server.requests_for("/page.html").len(), 1);
    let requests = server.requests_for("/clip.mp4");
    assert_eq!(requests.len(), 3);
    assert!(requests.iter().all(|request| {
        request
            .to_ascii_lowercase()
            .contains(&format!("\r\nreferer: {page_url}\r\n"))
    }));
}

#[test]
fn extraction_failures_are_not_retried_by_default() {
    let server = Server::start(|_, _| Response::status(503));
    assert!(
        YoutubeDl::new()
            .unwrap()
            .extract_info(&server.url("/clip.mp4"))
            .is_err()
    );
    assert_eq!(server.requests_for("/clip.mp4").len(), 1);
}

#[test]
fn default_download_retries_three_times() {
    assert_eq!(DownloadOptions::default().retries, 3);
    let server = Server::start(|_, count| {
        if (2..=4).contains(&count) {
            Response::status(503)
        } else {
            Response::media()
        }
    });
    let downloader = YoutubeDl::new().unwrap();
    let info = downloader.extract_info(&server.url("/clip.mp4")).unwrap();
    let directory = tempfile::tempdir().unwrap();
    assert_eq!(
        downloader
            .download(&info, directory.path().join("clip.mp4"))
            .unwrap(),
        14
    );
    assert_eq!(server.requests_for("/clip.mp4").len(), 5);
}

#[test]
fn cli_zero_retries_fails_once_and_continues_to_the_next_url() {
    let server = Server::start(|path, count| {
        if path == "/failed.mp4" && count > 1 {
            Response::status(503)
        } else {
            Response::media()
        }
    });
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args([
            "--retries",
            "0",
            "-o",
            "%(id)s.%(ext)s",
            &server.url("/failed.mp4"),
            &server.url("/success.mp4"),
        ])
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(server.requests_for("/failed.mp4").len(), 2);
    assert_eq!(server.requests_for("/success.mp4").len(), 2);
    assert_eq!(
        std::fs::read(directory.path().join("success.mp4")).unwrap(),
        b"complete media"
    );
    // Like upstream, a transfer that fails before receiving data leaves no part file.
    assert_eq!(directory_names(directory.path()), ["success.mp4"]);
}

#[test]
fn cli_retry_count_errors_match_upstream() {
    for (args, message) in [
        (
            &["--retries", "x"][..],
            "yt-dlp-rs: error: invalid download retry count \"x\" given",
        ),
        (
            &["--fragment-retries", "-1"][..],
            "yt-dlp-rs: error: fragment retry count \"-1\" must be positive or 0",
        ),
        (
            &["--retry-sleep", "http:bogus"][..],
            "yt-dlp-rs: error: invalid http retry sleep expression 'bogus'",
        ),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .args(args)
            .arg("http://127.0.0.1:1/unused.mp4")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.trim_end().ends_with(message), "{stderr}");
    }
}

#[test]
fn cli_rejects_retry_values_outside_bounded_integer_range() {
    for (option, values) in [
        ("--retries", &["-1", "infinity", "1.5", "x"][..]),
        ("--fragment-retries", &["-1", "infinity", "1.5"][..]),
        ("--extractor-retries", &["-1", "256", "infinity", "1.5"][..]),
    ] {
        for value in values {
            let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
                .args([
                    &format!("{option}={value}"),
                    "http://127.0.0.1:1/unused.mp4",
                ])
                .output()
                .unwrap();
            assert_eq!(
                output.status.code(),
                Some(2),
                "{option} value {value}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}

#[test]
fn html5_source_that_stops_serving_media_is_not_retried() {
    let server = Server::start(|path, count| {
        if path == "/page.html" {
            let body = "<video src='/clip.mp4'></video>";
            Response {
                mime: "text/html",
                body,
                length: body.len(),
                ..Response::media()
            }
        } else if count == 1 {
            Response::media()
        } else {
            Response {
                mime: "text/html",
                ..Response::media()
            }
        }
    });
    let downloader = YoutubeDl::new().unwrap();
    let info = downloader.extract_info(&server.url("/page.html")).unwrap();
    let directory = tempfile::tempdir().unwrap();
    assert!(
        downloader
            .download_with_options(
                &info,
                directory.path().join("clip.mp4"),
                DownloadOptions {
                    retries: 2,
                    ..Default::default()
                }
            )
            .is_err()
    );
    assert_eq!(server.requests_for("/clip.mp4").len(), 2);
    assert!(directory_names(directory.path()).is_empty());
}

#[test]
fn filesystem_failure_does_not_retry_or_make_download_requests() {
    let server = Server::start(|_, _| Response::media());
    let downloader = YoutubeDl::new().unwrap();
    let info = downloader.extract_info(&server.url("/clip.mp4")).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let parent = directory.path().join("parent");
    std::fs::write(&parent, "existing file").unwrap();
    assert!(
        downloader
            .download_with_options(
                &info,
                parent.join("clip.mp4"),
                DownloadOptions {
                    retries: 2,
                    ..Default::default()
                }
            )
            .is_err()
    );
    assert_eq!(server.requests_for("/clip.mp4").len(), 1);
    assert_eq!(std::fs::read(parent).unwrap(), b"existing file");
    assert_eq!(directory_names(directory.path()), ["parent"]);
}

#[test]
fn cli_one_retry_recovers_from_transient_download_failure() {
    let server = Server::start(|_, count| {
        if count == 2 {
            Response::status(503)
        } else {
            Response::media()
        }
    });
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--retries", "1", "-o", "clip.mp4", &server.url("/clip.mp4")])
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(server.requests_for("/clip.mp4").len(), 3);
    assert_eq!(
        std::fs::read(directory.path().join("clip.mp4")).unwrap(),
        b"complete media"
    );
    assert_eq!(directory_names(directory.path()), ["clip.mp4"]);
}

fn interrupted() -> Response {
    Response {
        body: "comp",
        headers: "ETag: \"version-one\"\r\n",
        ..Response::media()
    }
}

fn remainder() -> Response {
    Response {
        status: 206,
        body: "lete media",
        length: 10,
        headers: "ETag: \"version-one\"\r\nContent-Range: bytes 4-13/14\r\n",
        ..Response::media()
    }
}

#[test]
fn interrupted_download_resumes_with_download_validator_and_html5_referrer() {
    for html5 in [false, true] {
        let server = Server::start(|path, count| {
            if path == "/page.html" {
                let body = "<video src='/clip.mp4'></video>";
                Response {
                    mime: "text/html",
                    body,
                    length: body.len(),
                    ..Response::media()
                }
            } else {
                match count {
                    1 => Response {
                        headers: "ETag: \"probe-only\"\r\n",
                        ..Response::media()
                    },
                    2 => interrupted(),
                    _ => remainder(),
                }
            }
        });
        let downloader = YoutubeDl::new().unwrap();
        let input = server.url(if html5 { "/page.html" } else { "/clip.mp4" });
        let info = downloader.extract_info(&input).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("clip.mp4");
        assert_eq!(
            downloader
                .download_with_options(
                    &info,
                    &destination,
                    DownloadOptions {
                        retries: 1,
                        ..Default::default()
                    }
                )
                .unwrap(),
            14
        );
        assert_eq!(std::fs::read(destination).unwrap(), b"complete media");
        let requests = server.requests_for("/clip.mp4");
        assert_eq!(requests.len(), 3);
        let retry = requests[2].to_ascii_lowercase();
        assert!(retry.contains("\r\nrange: bytes=4-\r\n"));
        assert!(retry.contains("\r\nif-range: \"version-one\"\r\n"));
        assert!(retry.contains("\r\naccept-encoding: identity\r\n"));
        if html5 {
            assert!(retry.contains(&format!("\r\nreferer: {input}\r\n")));
        }
        assert_eq!(directory_names(directory.path()), ["clip.mp4"]);
    }
}

#[test]
fn interrupted_ranges_advance_offsets_and_share_retry_budget() {
    for retries in [1, 2] {
        let server = Server::start(|_, count| match count {
            1 => Response::media(),
            2 => interrupted(),
            3 => Response {
                body: "let",
                ..remainder()
            },
            _ => Response {
                body: "e media",
                length: 7,
                headers: "ETag: \"version-one\"\r\nContent-Range: bytes 7-13/14\r\n",
                ..remainder()
            },
        });
        let downloader = YoutubeDl::new().unwrap();
        let info = downloader.extract_info(&server.url("/clip.mp4")).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("clip.mp4");
        let result = downloader.download_with_options(
            &info,
            &destination,
            DownloadOptions {
                retries,
                ..Default::default()
            },
        );
        let requests = server.requests_for("/clip.mp4");
        assert_eq!(requests.len(), retries as usize + 2);
        if retries == 2 {
            assert_eq!(result.unwrap(), 14);
            assert_eq!(std::fs::read(destination).unwrap(), b"complete media");
            assert!(requests[3].contains("range: bytes=7-\r\n"));
        } else {
            assert!(result.is_err());
            assert!(directory_names(directory.path()).is_empty());
        }
    }
}

#[test]
fn full_response_to_range_replaces_partial_bytes() {
    let server = Server::start(|_, count| match count {
        1 => Response::media(),
        2 => interrupted(),
        _ => Response {
            body: "new",
            length: 3,
            headers: "ETag: \"new\"\r\n",
            ..Response::media()
        },
    });
    let downloader = YoutubeDl::new().unwrap();
    let info = downloader.extract_info(&server.url("/clip.mp4")).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("clip.mp4");
    assert_eq!(
        downloader
            .download_with_options(
                &info,
                &destination,
                DownloadOptions {
                    retries: 1,
                    ..Default::default()
                }
            )
            .unwrap(),
        3
    );
    assert_eq!(std::fs::read(destination).unwrap(), b"new");
}

#[test]
fn unsafe_partial_responses_fail_without_publishing_or_retrying() {
    for headers in [
        "ETag: \"version-one\"\r\n",
        "ETag: \"changed\"\r\nContent-Range: bytes 4-13/14\r\n",
        "Content-Range: bytes 4-13/14\r\n",
        "ETag: W/\"version-one\"\r\nContent-Range: bytes 4-13/14\r\n",
        "ETag: \"version-one\"\r\nContent-Range: bytes 3-12/14\r\n",
        "ETag: \"version-one\"\r\nContent-Range: bytes 4-12/14\r\n",
        "ETag: \"version-one\"\r\nContent-Range: bytes 4-13/15\r\n",
        "ETag: \"version-one\"\r\nContent-Range: bytes 4-13/*\r\n",
        "ETag: \"version-one\"\r\nContent-Range: bytes +4-13/14\r\n",
        "ETag: \"version-one\"\r\nContent-Range: bytes 4-18446744073709551616/14\r\n",
        "ETag: \"version-one\"\r\nContent-Range: bytes 4-13/14\r\nContent-Encoding: gzip\r\n",
        "ETag: \"version-one\"\r\nContent-Range: bytes 4-13/14\r\nContent-Range: bytes 4-13/14\r\n",
    ] {
        let server = Server::start(move |_, count| match count {
            1 => Response::media(),
            2 => interrupted(),
            _ => Response {
                headers,
                ..remainder()
            },
        });
        let downloader = YoutubeDl::new().unwrap();
        let info = downloader.extract_info(&server.url("/clip.mp4")).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let error = downloader
            .download_with_options(
                &info,
                directory.path().join("clip.mp4"),
                DownloadOptions {
                    retries: 2,
                    ..Default::default()
                },
            )
            .unwrap_err();
        assert!(
            matches!(error, yt_dlp_rs::Error::InvalidRangeResponse),
            "{headers}: {error}"
        );
        assert_eq!(server.requests_for("/clip.mp4").len(), 3);
        assert!(directory_names(directory.path()).is_empty());
    }
}

#[test]
fn weak_or_malformed_etags_and_encoded_bodies_restart_from_zero() {
    for headers in [
        "",
        "ETag: W/\"weak\"\r\n",
        "ETag: unquoted\r\n",
        "ETag: \"has space\"\r\n",
        "ETag: \"one\"\r\nETag: \"one\"\r\n",
        "ETag: \"one\"\r\nContent-Encoding: gzip\r\n",
        "Last-Modified: Wed, 21 Oct 2015 07:28:00 GMT\r\n",
    ] {
        let server = Server::start(move |_, count| {
            if count == 2 {
                Response {
                    headers,
                    ..interrupted()
                }
            } else {
                Response::media()
            }
        });
        let downloader = YoutubeDl::new().unwrap();
        let info = downloader.extract_info(&server.url("/clip.mp4")).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("clip.mp4");
        downloader
            .download_with_options(
                &info,
                &destination,
                DownloadOptions {
                    retries: 1,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(std::fs::read(destination).unwrap(), b"complete media");
        let requests = server.requests_for("/clip.mp4");
        assert_eq!(requests.len(), 3);
        assert!(!requests[2].contains("range:"));
    }
}

#[test]
fn range_length_mismatch_or_416_is_not_accepted_as_complete() {
    for status in [206, 416] {
        let server = Server::start(move |_, count| match count {
            1 => Response::media(),
            2 => interrupted(),
            _ => Response {
                status,
                length: 9,
                ..remainder()
            },
        });
        let downloader = YoutubeDl::new().unwrap();
        let info = downloader.extract_info(&server.url("/clip.mp4")).unwrap();
        let directory = tempfile::tempdir().unwrap();
        assert!(
            downloader
                .download_with_options(
                    &info,
                    directory.path().join("clip.mp4"),
                    DownloadOptions {
                        retries: 2,
                        ..Default::default()
                    }
                )
                .is_err()
        );
        assert_eq!(server.requests_for("/clip.mp4").len(), 3);
        assert!(directory_names(directory.path()).is_empty());
    }
}

#[test]
fn range_redirect_to_different_resource_is_rejected() {
    let server = Server::start(|path, count| {
        if path == "/other.mp4" {
            remainder()
        } else {
            match count {
                1 => Response::media(),
                2 => interrupted(),
                _ => Response {
                    status: 302,
                    headers: "Location: /other.mp4\r\n",
                    ..Response::media()
                },
            }
        }
    });
    let downloader = YoutubeDl::new().unwrap();
    let info = downloader.extract_info(&server.url("/clip.mp4")).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let error = downloader
        .download_with_options(
            &info,
            directory.path().join("clip.mp4"),
            DownloadOptions {
                retries: 1,
                ..Default::default()
            },
        )
        .unwrap_err();
    assert!(matches!(error, yt_dlp_rs::Error::InvalidRangeResponse));
    assert!(directory_names(directory.path()).is_empty());
}

#[test]
fn unknown_initial_length_restarts_even_with_a_strong_etag() {
    let server = Server::start(|_, count| {
        if count == 2 {
            // Incomplete chunk framing forces a network read failure, while
            // deliberately withholding a total representation length.
            Response {
                body: "4\r\ncomp\r\n",
                length: usize::MAX,
                headers: "ETag: \"version-one\"\r\nTransfer-Encoding: chunked\r\n",
                ..Response::media()
            }
        } else {
            Response::media()
        }
    });
    let downloader = YoutubeDl::new().unwrap();
    let info = downloader.extract_info(&server.url("/clip.mp4")).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("clip.mp4");
    downloader
        .download_with_options(
            &info,
            &destination,
            DownloadOptions {
                retries: 1,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(std::fs::read(destination).unwrap(), b"complete media");
    let requests = server.requests_for("/clip.mp4");
    assert_eq!(requests.len(), 3);
    assert!(!requests[2].contains("range:"));
}

#[test]
fn range_body_without_content_length_is_checked_against_content_range() {
    for body in ["lete media", "too much data"] {
        let server = Server::start(move |_, count| match count {
            1 => Response::media(),
            2 => interrupted(),
            _ => Response {
                body,
                length: usize::MAX,
                ..remainder()
            },
        });
        let downloader = YoutubeDl::new().unwrap();
        let info = downloader.extract_info(&server.url("/clip.mp4")).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("clip.mp4");
        let result = downloader.download_with_options(
            &info,
            &destination,
            DownloadOptions {
                retries: 1,
                ..Default::default()
            },
        );
        if body == "lete media" {
            assert_eq!(result.unwrap(), 14);
            assert_eq!(std::fs::read(destination).unwrap(), b"complete media");
        } else {
            assert!(matches!(
                result,
                Err(yt_dlp_rs::Error::InvalidRangeResponse)
            ));
            assert!(directory_names(directory.path()).is_empty());
        }
    }
}

#[test]
fn transient_range_status_retains_offset_and_validator() {
    let server = Server::start(|_, count| match count {
        1 => Response::media(),
        2 => interrupted(),
        3 => Response {
            headers: "Retry-After: 0\r\n",
            ..Response::status(503)
        },
        _ => remainder(),
    });
    let downloader = YoutubeDl::new().unwrap();
    let info = downloader.extract_info(&server.url("/clip.mp4")).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("clip.mp4");
    downloader
        .download_with_options(
            &info,
            &destination,
            DownloadOptions {
                retries: 2,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(std::fs::read(destination).unwrap(), b"complete media");
    let requests = server.requests_for("/clip.mp4");
    assert_eq!(requests.len(), 4);
    for request in &requests[2..] {
        assert!(request.contains("range: bytes=4-\r\n"));
        assert!(request.contains("if-range: \"version-one\"\r\n"));
    }
}

#[test]
fn interrupted_full_replacement_resumes_using_new_validator_and_length() {
    let server = Server::start(|_, count| match count {
        1 => Response::media(),
        2 => interrupted(),
        3 => Response {
            body: "new",
            length: 10,
            headers: "ETag: \"replacement\"\r\n",
            ..Response::media()
        },
        _ => Response {
            body: " media!",
            length: 7,
            headers: "ETag: \"replacement\"\r\nContent-Range: bytes 3-9/10\r\n",
            ..remainder()
        },
    });
    let downloader = YoutubeDl::new().unwrap();
    let info = downloader.extract_info(&server.url("/clip.mp4")).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("clip.mp4");
    assert_eq!(
        downloader
            .download_with_options(
                &info,
                &destination,
                DownloadOptions {
                    retries: 2,
                    ..Default::default()
                }
            )
            .unwrap(),
        10
    );
    assert_eq!(std::fs::read(destination).unwrap(), b"new media!");
    let requests = server.requests_for("/clip.mp4");
    assert_eq!(requests.len(), 4);
    assert!(requests[3].contains("range: bytes=3-\r\n"));
    assert!(requests[3].contains("if-range: \"replacement\"\r\n"));
    assert_eq!(directory_names(directory.path()), ["clip.mp4"]);
}

fn page(body: &'static str) -> Response {
    Response {
        mime: "text/html",
        body,
        length: body.len(),
        ..Response::media()
    }
}

#[test]
fn extraction_retries_selected_transient_statuses() {
    for status in [408, 429, 500, 502, 503, 504] {
        let server = Server::start(move |_, count| {
            if count == 1 {
                Response::status(status)
            } else {
                Response::media()
            }
        });
        let result = YoutubeDl::new()
            .unwrap()
            .extract_result_with_options(
                &server.url("/clip.mp4"),
                None,
                ExtractionOptions { retries: 1 },
            )
            .unwrap();
        assert_eq!(result.into_video().unwrap().ext, "mp4");
        assert_eq!(server.requests_for("/clip.mp4").len(), 2, "{status}");
    }
}

#[test]
fn extraction_budget_is_bounded_and_returns_final_error() {
    assert_eq!(ExtractionOptions::default().retries, 0);
    for retries in [0, 2] {
        let server = Server::start(|_, count| Response::status(if count == 1 { 503 } else { 429 }));
        let error = YoutubeDl::new()
            .unwrap()
            .extract_result_with_options(
                &server.url("/clip.mp4"),
                None,
                ExtractionOptions { retries },
            )
            .unwrap_err();
        // Reported as upstream's `Unable to download webpage`.
        let Error::Webpage { source, .. } = error else {
            panic!("{error}")
        };
        let Error::Http(error) = *source else {
            panic!("{source}")
        };
        assert_eq!(
            error.status().unwrap().as_u16(),
            if retries == 0 { 503 } else { 429 }
        );
        assert_eq!(
            server.requests_for("/clip.mp4").len(),
            usize::from(retries) + 1
        );
    }
}

#[test]
fn extraction_restarts_truncated_html_without_reusing_partial_bytes() {
    let server = Server::start(|path, count| {
        if path == "/page.html" {
            if count == 1 {
                Response {
                    length: 200,
                    ..page("<video src='/wrong.mp4'>")
                }
            } else {
                page("<video src='/clip.mp4'></video>")
            }
        } else {
            Response::media()
        }
    });
    let info = YoutubeDl::new()
        .unwrap()
        .extract_result_with_options(
            &server.url("/page.html"),
            None,
            ExtractionOptions { retries: 1 },
        )
        .unwrap()
        .into_video()
        .unwrap();
    assert_eq!(info.url.as_str(), server.url("/clip.mp4"));
    assert_eq!(server.requests_for("/page.html").len(), 2);
    assert!(server.requests_for("/wrong.mp4").is_empty());
    assert_eq!(server.requests_for("/clip.mp4").len(), 1);
}

#[test]
fn source_probe_failure_restarts_entire_playlist_and_preserves_referrer() {
    let server = Server::start(|path, count| match path {
        "/page.html" => page("<video src='/first.mp4'></video><audio src='/second.mp4'></audio>"),
        "/second.mp4" if count == 1 => Response::status(503),
        _ => Response::media(),
    });
    let result = YoutubeDl::new()
        .unwrap()
        .extract_result_with_options(
            &server.url("/page.html"),
            None,
            ExtractionOptions { retries: 1 },
        )
        .unwrap();
    assert_eq!(result.into_entries().len(), 2);
    for path in ["/page.html", "/first.mp4", "/second.mp4"] {
        assert_eq!(server.requests_for(path).len(), 2, "{path}");
    }
    for path in ["/first.mp4", "/second.mp4"] {
        assert!(server.requests_for(path).iter().all(|request| {
            request
                .to_ascii_lowercase()
                .contains(&format!("\r\nreferer: {}\r\n", server.url("/page.html")))
        }));
    }
}

#[test]
fn extraction_does_not_retry_permanent_statuses_or_unsupported_pages() {
    for status in [200, 206, 401, 404, 416, 501] {
        let server = Server::start(move |_, _| Response {
            status,
            ..page("<video src='/manifest.m3u8'></video>")
        });
        assert!(
            YoutubeDl::new()
                .unwrap()
                .extract_result_with_options(
                    &server.url("/page.html"),
                    None,
                    ExtractionOptions { retries: 2 },
                )
                .is_err()
        );
        assert_eq!(server.requests_for("/page.html").len(), 1, "{status}");
        assert_eq!(
            server.requests_for("/manifest.m3u8").len(),
            usize::from(status == 200)
        );
    }
}

#[test]
fn extraction_does_not_retry_format_selection_or_oversized_html() {
    let server = Server::start(|path, _| {
        if path == "/large.html" {
            Response {
                length: 3 * 1024 * 1024,
                ..page("")
            }
        } else {
            Response::media()
        }
    });
    let downloader = YoutubeDl::new().unwrap();
    assert!(matches!(
        downloader.extract_result_with_options(
            &server.url("/clip.mp4"),
            Some("unavailable"),
            ExtractionOptions { retries: 2 },
        ),
        Err(Error::FormatSelection(_))
    ));
    assert!(matches!(
        downloader.extract_result_with_options(
            &server.url("/large.html"),
            None,
            ExtractionOptions { retries: 2 },
        ),
        Err(Error::HtmlTooLarge(_))
    ));
    assert_eq!(server.requests_for("/clip.mp4").len(), 1);
    assert_eq!(server.requests_for("/large.html").len(), 1);
}

#[test]
fn cli_extraction_retries_apply_to_json_and_simulation_without_downloads() {
    for mode in ["--dump-json", "--simulate"] {
        let server = Server::start(|_, count| {
            if count == 1 {
                Response::status(503)
            } else {
                Response::media()
            }
        });
        let directory = tempfile::tempdir().unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .args([
                mode,
                "--extractor-retries",
                "1",
                "--retries",
                "0",
                &server.url("/clip.mp4"),
            ])
            .current_dir(directory.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(server.requests_for("/clip.mp4").len(), 2);
        assert!(directory_names(directory.path()).is_empty());
        if mode == "--dump-json" {
            let info: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(info["ext"], "mp4");
        } else {
            // Simulation prints no JSON: stdout holds only screen lines.
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(stdout.lines().all(|line| line.starts_with('[')), "{stdout}");
        }
    }
}

#[test]
fn cli_exhausted_extraction_emits_no_partial_playlist_and_continues() {
    let server = Server::start(|path, _| match path {
        "/page.html" => page("<video src='/first.mp4'></video><video src='/failed.mp4'></video>"),
        "/failed.mp4" => Response::status(503),
        _ => Response::media(),
    });
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args([
            "-j",
            "--extractor-retries",
            "1",
            &server.url("/page.html"),
            &server.url("/next.mp4"),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let info: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(info["id"], "next");
    assert_eq!(server.requests_for("/page.html").len(), 2);
    assert_eq!(server.requests_for("/first.mp4").len(), 2);
    assert_eq!(server.requests_for("/failed.mp4").len(), 2);
    assert_eq!(server.requests_for("/next.mp4").len(), 1);
}

#[test]
fn cli_download_retry_budget_does_not_enable_extraction_retries() {
    for options in [vec![], vec!["--retries", "2"]] {
        let server = Server::start(|_, _| Response::status(503));
        let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .args(options)
            .args(["-j", &server.url("/clip.mp4")])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert_eq!(server.requests_for("/clip.mp4").len(), 1);
    }
}

#[test]
fn custom_extractor_filesystem_errors_are_not_retried() {
    use std::sync::atomic::AtomicUsize;
    use yt_dlp_rs::{Result, VideoInfo, extractor::Extractor};

    struct FailingExtractor(Arc<AtomicUsize>);
    impl Extractor for FailingExtractor {
        fn key(&self) -> &'static str {
            "Failing"
        }
        fn extract(
            &self,
            _: &reqwest::blocking::Client,
            _: &url::Url,
        ) -> Result<Option<VideoInfo>> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Err(Error::Io(std::io::Error::other("local fixture failure")))
        }
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let mut downloader = YoutubeDl::new().unwrap();
    downloader.register_extractor(Box::new(FailingExtractor(calls.clone())));
    let error = downloader
        .extract_result_with_options(
            "http://127.0.0.1:1/unused.mp4",
            None,
            ExtractionOptions { retries: 2 },
        )
        .unwrap_err();
    assert!(matches!(error, Error::Io(_)));
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[test]
fn exhausted_html_read_retries_return_final_read_error() {
    let server = Server::start(|_, _| Response {
        length: 100,
        ..page("<video")
    });
    let error = YoutubeDl::new()
        .unwrap()
        .extract_result_with_options(
            &server.url("/page.html"),
            None,
            ExtractionOptions { retries: 2 },
        )
        .unwrap_err();
    assert!(matches!(error, Error::ExtractionRead(_)), "{error}");
    assert_eq!(server.requests_for("/page.html").len(), 3);
}

#[test]
fn extraction_retry_restarts_previously_visited_custom_extractors() {
    use yt_dlp_rs::{Result, VideoInfo, extractor::Extractor};

    struct RecordingExtractor {
        name: &'static str,
        fail: bool,
        calls: Arc<Mutex<Vec<&'static str>>>,
    }
    impl Extractor for RecordingExtractor {
        fn key(&self) -> &'static str {
            self.name
        }
        fn extract(
            &self,
            _: &reqwest::blocking::Client,
            _: &url::Url,
        ) -> Result<Option<VideoInfo>> {
            self.calls.lock().unwrap().push(self.name);
            if self.fail {
                Err(Error::ExtractionRead(std::io::Error::other(
                    "fixture network read",
                )))
            } else {
                Ok(None)
            }
        }
    }
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut downloader = YoutubeDl::new().unwrap();
    // Registration prepends, so the declining extractor runs before the failing one.
    for (name, fail) in [("failing", true), ("declining", false)] {
        downloader.register_extractor(Box::new(RecordingExtractor {
            name,
            fail,
            calls: calls.clone(),
        }));
    }
    let error = downloader
        .extract_result_with_options(
            "http://127.0.0.1:1/unused.mp4",
            None,
            ExtractionOptions { retries: 1 },
        )
        .unwrap_err();
    assert!(matches!(error, Error::ExtractionRead(_)));
    assert_eq!(
        *calls.lock().unwrap(),
        ["declining", "failing", "declining", "failing"]
    );
}

#[test]
fn retry_after_delays_extraction_and_download_recovery() {
    for extraction in [true, false] {
        let server = Server::start(move |_, count| {
            if count == if extraction { 1 } else { 2 } {
                Response {
                    headers: "Retry-After: 1\r\n",
                    // Downloads retry 5xx statuses only.
                    ..Response::status(if extraction { 429 } else { 503 })
                }
            } else {
                Response::media()
            }
        });
        let downloader = YoutubeDl::new().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let start = std::time::Instant::now();
        let info = downloader
            .extract_result_with_options(
                &server.url("/clip.mp4"),
                None,
                ExtractionOptions { retries: 1 },
            )
            .unwrap()
            .into_video()
            .unwrap();
        if !extraction {
            let destination = directory.path().join("clip.mp4");
            downloader
                .download_with_options(
                    &info,
                    &destination,
                    DownloadOptions {
                        retries: 1,
                        ..Default::default()
                    },
                )
                .unwrap();
            assert_eq!(std::fs::read(destination).unwrap(), b"complete media");
        }
        assert!(start.elapsed() >= Duration::from_secs(1));
        assert_eq!(
            server.requests_for("/clip.mp4").len(),
            if extraction { 2 } else { 3 }
        );
    }
}

#[test]
fn retry_after_respects_exhausted_budgets_and_permanent_statuses() {
    for extraction in [true, false] {
        for (status, headers) in [
            (503, "Retry-After: 999999999999999999999999\r\n"),
            (404, "Retry-After: 999999999999999999999999\r\n"),
            (503, "Retry-After: Fri, 31 Dec 9999 23:59:59 GMT\r\n"),
            (404, "Retry-After: Fri, 31 Dec 9999 23:59:59 GMT\r\n"),
        ] {
            let server = Server::start(move |_, count| {
                if !extraction && count == 1 {
                    Response::media()
                } else {
                    Response {
                        headers,
                        ..Response::status(status)
                    }
                }
            });
            let downloader = YoutubeDl::new().unwrap();
            let directory = tempfile::tempdir().unwrap();
            let retries = if status == 404 { 2 } else { 0 };
            let error = if extraction {
                downloader
                    .extract_result_with_options(
                        &server.url("/clip.mp4"),
                        None,
                        ExtractionOptions { retries },
                    )
                    .unwrap_err()
            } else {
                let info = downloader.extract_info(&server.url("/clip.mp4")).unwrap();
                downloader
                    .download_with_options(
                        &info,
                        directory.path().join("clip.mp4"),
                        DownloadOptions {
                            retries: retries.into(),
                            ..Default::default()
                        },
                    )
                    .unwrap_err()
            };
            // A retryable download failure reports through `RetryManager`,
            // a page failure as `Unable to download webpage`.
            let error = match error {
                Error::RetriesExhausted { source, .. } | Error::Webpage { source, .. } => *source,
                error => error,
            };
            if status == 503 {
                match error {
                    Error::HttpRetryAfter {
                        source,
                        retry_after,
                    } => {
                        assert_eq!(source.status().unwrap().as_u16(), 503);
                        assert_eq!(retry_after, Duration::from_secs(60));
                    }
                    error => panic!("unexpected error: {error}"),
                }
            } else {
                assert!(matches!(error, Error::Http(_)));
            }
            assert_eq!(
                server.requests_for("/clip.mp4").len(),
                if extraction { 1 } else { 2 }
            );
            assert!(directory_names(directory.path()).is_empty());
        }
    }
}

#[test]
fn unsupported_retry_after_values_keep_transient_failures_retryable() {
    for headers in [
        "Retry-After: nonsense\r\n",
        "Retry-After: Wednesday, 21-Oct-15 07:28:00 GMT\r\n",
        "Retry-After: 60\r\nRetry-After: 60\r\n",
    ] {
        let server = Server::start(move |_, count| {
            if count == 1 {
                Response {
                    headers,
                    ..Response::status(503)
                }
            } else {
                Response::media()
            }
        });
        YoutubeDl::new()
            .unwrap()
            .extract_result_with_options(
                &server.url("/clip.mp4"),
                None,
                ExtractionOptions { retries: 1 },
            )
            .unwrap();
        assert_eq!(server.requests_for("/clip.mp4").len(), 2);
    }
}

#[test]
fn expired_http_date_hints_allow_extraction_and_download_recovery() {
    for extraction in [true, false] {
        let server = Server::start(move |_, count| {
            if count == if extraction { 1 } else { 2 } {
                Response {
                    headers: "Retry-After: Wed, 21 Oct 2015 07:28:00 GMT\r\n",
                    ..Response::status(503)
                }
            } else {
                Response::media()
            }
        });
        let downloader = YoutubeDl::new().unwrap();
        let info = downloader
            .extract_result_with_options(
                &server.url("/clip.mp4"),
                None,
                ExtractionOptions { retries: 1 },
            )
            .unwrap()
            .into_video()
            .unwrap();
        if !extraction {
            let directory = tempfile::tempdir().unwrap();
            let destination = directory.path().join("clip.mp4");
            downloader
                .download_with_options(
                    &info,
                    &destination,
                    DownloadOptions {
                        retries: 1,
                        ..Default::default()
                    },
                )
                .unwrap();
            assert_eq!(std::fs::read(destination).unwrap(), b"complete media");
        }
        assert_eq!(
            server.requests_for("/clip.mp4").len(),
            if extraction { 2 } else { 3 }
        );
    }
}

#[test]
fn cli_part_files_resume_across_runs_or_restart_with_no_continue() {
    let server = Server::start(|request, _| {
        if request.to_ascii_lowercase().contains("range: bytes=9-") {
            Response {
                status: 206,
                body: "media",
                length: 5,
                headers: "Content-Range: bytes 9-13/14\r\n",
                ..Response::media()
            }
        } else {
            Response::media()
        }
    });
    for (args, resumed) in [(vec![], true), (vec!["--no-continue"], false)] {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("clip.mp4.part"), "complete ").unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .args(&args)
            .args(["-o", "clip.%(ext)s", &server.url("/clip.mp4")])
            .current_dir(directory.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_eq!(
            stdout.contains("[download] Resuming download at byte 9"),
            resumed,
            "{stdout}"
        );
        assert_eq!(
            std::fs::read(directory.path().join("clip.mp4")).unwrap(),
            b"complete media"
        );
        assert_eq!(directory_names(directory.path()), ["clip.mp4"]);
    }
    // --no-part writes the destination directly.
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--no-part", "-o", "clip.%(ext)s", &server.url("/clip.mp4")])
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(directory_names(directory.path()), ["clip.mp4"]);
}

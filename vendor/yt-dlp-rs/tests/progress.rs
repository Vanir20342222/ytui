use std::{
    io::{Read, Write},
    net::{Shutdown, TcpListener},
    ops::ControlFlow,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use yt_dlp_rs::{DownloadOptions, DownloadPhase, DownloadProgress, Error, YoutubeDl};

struct Response {
    status: u16,
    body: &'static str,
    length: Option<usize>,
    headers: &'static str,
}

impl Response {
    fn media() -> Self {
        Self {
            status: 200,
            body: "complete media",
            length: Some(14),
            headers: "",
        }
    }
}

// A stoppable listener also terminates when cancellation prevents an expected GET.
struct Server {
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Server {
    fn start(respond: impl Fn(usize) -> Response + Send + 'static) -> Self {
        Self::start_with_delay(respond, Duration::ZERO)
    }

    fn start_with_delay(
        respond: impl Fn(usize) -> Response + Send + 'static,
        body_delay: Duration,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/clip.mp4", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
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
                // Accepted sockets can inherit the listener's nonblocking mode
                // on Windows; requests and responses below require blocking I/O.
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(2)))
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
                let count = {
                    let mut requests = recorded.lock().unwrap();
                    requests.push(String::from_utf8(bytes).unwrap());
                    requests.len()
                };
                let response = respond(count);
                let length = response.length.map_or_else(String::new, |length| {
                    format!("Content-Length: {length}\r\n")
                });
                let wire = format!(
                    "HTTP/1.1 {} Fixture\r\nContent-Type: video/mp4\r\n{}{}Connection: close\r\n\r\n",
                    response.status, length, response.headers
                );
                if count > 1 && !body_delay.is_zero() {
                    let _ = stream.write_all(wire.as_bytes());
                    thread::sleep(body_delay);
                    let _ = stream.write_all(response.body.as_bytes());
                } else {
                    // Extraction probes drop the response without reading its body.
                    // Send ordinary responses together so that close cannot race a
                    // second write and turn the fixture response into a Windows reset.
                    let wire = format!("{wire}{}", response.body);
                    let _ = stream.write_all(wire.as_bytes());
                }
                // Deliver EOF before closing the socket, and let the client close
                // its side so queued response bytes are not lost to a TCP reset.
                let _ = stream.shutdown(Shutdown::Write);
                while let Ok(count) = stream.read(&mut buffer) {
                    if count == 0 {
                        break;
                    }
                }
            }
        });
        Self {
            url,
            requests,
            stop,
            worker: Some(worker),
        }
    }

    fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
    }
}

fn assert_empty(directory: &std::path::Path) {
    assert_eq!(std::fs::read_dir(directory).unwrap().count(), 0);
}

#[test]
fn success_reports_download_headers_bytes_and_publication_order() {
    let server = Server::start(|count| {
        if count == 1 {
            Response::media()
        } else {
            Response {
                body: "hello",
                length: Some(5),
                ..Response::media()
            }
        }
    });
    let downloader = YoutubeDl::new().unwrap();
    let info = downloader.extract_info(&server.url).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("clip.mp4");
    let mut events = Vec::<DownloadProgress>::new();
    let bytes = downloader
        .download_with_progress(
            &info,
            &destination,
            DownloadOptions {
                retries: 0,
                ..Default::default()
            },
            |event| {
                // Even Publishing is cancellable before the final path appears.
                assert!(!destination.exists());
                events.push(event);
                ControlFlow::Continue(())
            },
        )
        .unwrap();
    assert_eq!(bytes, 5);
    assert_eq!(std::fs::read(destination).unwrap(), b"hello");
    assert_eq!(events.first().unwrap().phase, DownloadPhase::Requesting);
    assert_eq!(events.last().unwrap().phase, DownloadPhase::Publishing);
    assert!(events.iter().all(|event| event.attempt == 1));
    let downloading = events
        .iter()
        .filter(|event| event.phase == DownloadPhase::Downloading)
        .collect::<Vec<_>>();
    assert!(downloading.len() >= 2);
    assert_eq!(downloading.first().unwrap().downloaded_bytes, 0);
    assert_eq!(downloading.last().unwrap().downloaded_bytes, 5);
    assert!(downloading.iter().all(|event| event.total_bytes == Some(5)));
    assert!(
        downloading
            .windows(2)
            .all(|pair| pair[0].downloaded_bytes <= pair[1].downloaded_bytes)
    );
    assert_eq!(events.last().unwrap().downloaded_bytes, 5);
    assert_eq!(events.last().unwrap().total_bytes, Some(5));
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn unknown_download_length_does_not_reuse_extraction_length() {
    let server = Server::start(|count| {
        if count == 1 {
            Response::media()
        } else {
            Response {
                length: None,
                ..Response::media()
            }
        }
    });
    let downloader = YoutubeDl::new().unwrap();
    let info = downloader.extract_info(&server.url).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let mut events = Vec::new();
    assert_eq!(
        downloader
            .download_with_progress(
                &info,
                directory.path().join("clip.mp4"),
                DownloadOptions {
                    retries: 0,
                    ..Default::default()
                },
                |event| {
                    events.push(event);
                    ControlFlow::Continue(())
                },
            )
            .unwrap(),
        14
    );
    assert!(events.iter().all(|event| event.total_bytes.is_none()));
    assert_eq!(events.last().unwrap().downloaded_bytes, 14);
}

#[test]
fn cancellation_before_request_after_bytes_and_before_publication_cleans_output() {
    for phase in [
        DownloadPhase::Requesting,
        DownloadPhase::Downloading,
        DownloadPhase::Publishing,
    ] {
        let server = Server::start(|_| Response::media());
        let downloader = YoutubeDl::new().unwrap();
        let info = downloader.extract_info(&server.url).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let mut cancelled = false;
        let error = downloader
            .download_with_progress(
                &info,
                directory.path().join("clip.mp4"),
                DownloadOptions {
                    retries: 2,
                    ..Default::default()
                },
                |event| {
                    assert!(!cancelled, "callback invoked after cancellation");
                    if event.phase == phase
                        && (phase != DownloadPhase::Downloading || event.downloaded_bytes > 0)
                    {
                        cancelled = true;
                        ControlFlow::Break(())
                    } else {
                        ControlFlow::Continue(())
                    }
                },
            )
            .unwrap_err();
        assert!(cancelled);
        assert!(matches!(error, Error::DownloadCancelled), "{error}");
        assert_empty(directory.path());
        assert_eq!(
            server.requests().len(),
            if phase == DownloadPhase::Requesting {
                1
            } else {
                2
            }
        );
    }
}

#[test]
fn cancellation_during_retry_after_wait_prevents_the_retry() {
    let server = Server::start(|count| {
        if count == 1 {
            Response::media()
        } else {
            Response {
                status: 503,
                headers: "Retry-After: 60\r\n",
                ..Response::media()
            }
        }
    });
    let downloader = YoutubeDl::new().unwrap();
    let info = downloader.extract_info(&server.url).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let start = Instant::now();
    let mut waits = 0;
    let error = downloader
        .download_with_progress(
            &info,
            directory.path().join("clip.mp4"),
            DownloadOptions {
                retries: 1,
                ..Default::default()
            },
            |event| {
                if event.phase == DownloadPhase::WaitingToRetry {
                    waits += 1;
                    assert_eq!(event.attempt, 1);
                    if waits == 2 {
                        return ControlFlow::Break(());
                    }
                }
                ControlFlow::Continue(())
            },
        )
        .unwrap_err();
    assert!(matches!(error, Error::DownloadCancelled));
    assert_eq!(waits, 2);
    // A generous bound tolerates scheduling load while rejecting the 60-second sleep.
    assert!(start.elapsed() < Duration::from_secs(3));
    assert_eq!(server.requests().len(), 2);
    assert_empty(directory.path());
}

#[test]
fn resumed_retry_keeps_offsets_and_original_total() {
    let server = Server::start(|count| match count {
        1 => Response::media(),
        2 => Response {
            body: "comp",
            headers: "ETag: \"version-one\"\r\n",
            ..Response::media()
        },
        _ => Response {
            status: 206,
            body: "lete media",
            length: Some(10),
            headers: "ETag: \"version-one\"\r\nContent-Range: bytes 4-13/14\r\n",
        },
    });
    let downloader = YoutubeDl::new().unwrap();
    let info = downloader.extract_info(&server.url).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("clip.mp4");
    let mut events = Vec::new();
    assert_eq!(
        downloader
            .download_with_progress(
                &info,
                &destination,
                DownloadOptions {
                    retries: 1,
                    ..Default::default()
                },
                |event| {
                    events.push(event);
                    ControlFlow::Continue(())
                }
            )
            .unwrap(),
        14
    );
    assert_eq!(std::fs::read(destination).unwrap(), b"complete media");
    let retry = events
        .iter()
        .filter(|event| event.attempt == 2 && event.phase == DownloadPhase::Downloading)
        .collect::<Vec<_>>();
    assert_eq!(retry.first().unwrap().downloaded_bytes, 4);
    assert_eq!(retry.last().unwrap().downloaded_bytes, 14);
    assert!(retry.iter().all(|event| event.total_bytes == Some(14)));
    assert!(
        events
            .windows(2)
            .all(|pair| pair[0].downloaded_bytes <= pair[1].downloaded_bytes)
    );
    assert!(
        server.requests()[2]
            .to_ascii_lowercase()
            .contains("\r\nrange: bytes=4-\r\n")
    );
}

#[test]
fn full_response_to_range_retry_resets_bytes_and_total() {
    let server = Server::start(|count| match count {
        1 => Response::media(),
        2 => Response {
            body: "comp",
            headers: "ETag: \"version-one\"\r\n",
            ..Response::media()
        },
        _ => Response {
            body: "new",
            length: Some(3),
            ..Response::media()
        },
    });
    let downloader = YoutubeDl::new().unwrap();
    let info = downloader.extract_info(&server.url).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("clip.mp4");
    let mut events = Vec::new();
    assert_eq!(
        downloader
            .download_with_progress(
                &info,
                &destination,
                DownloadOptions {
                    retries: 1,
                    ..Default::default()
                },
                |event| {
                    events.push(event);
                    ControlFlow::Continue(())
                }
            )
            .unwrap(),
        3
    );
    assert_eq!(std::fs::read(destination).unwrap(), b"new");
    assert!(
        events
            .iter()
            .any(|event| event.attempt == 1 && event.downloaded_bytes == 4)
    );
    let retry = events
        .iter()
        .filter(|event| event.attempt == 2 && event.phase == DownloadPhase::Downloading)
        .collect::<Vec<_>>();
    assert_eq!(retry.first().unwrap().downloaded_bytes, 0);
    assert_eq!(retry.last().unwrap().downloaded_bytes, 3);
    assert!(retry.iter().all(|event| event.total_bytes == Some(3)));
    assert_eq!(events.last().unwrap().phase, DownloadPhase::Publishing);
    assert_eq!(events.last().unwrap().downloaded_bytes, 3);
}

#[test]
fn cli_progress_uses_stdout_and_preserves_download_bytes() {
    let server = Server::start_with_delay(|_| Response::media(), Duration::from_millis(1100));
    let directory = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--retries", "0", "-o", "clip.mp4", &server.url])
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // Like upstream, progress goes to stdout; this direct download warns nothing.
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    // Upstream `report_progress` lines share one carriage-return status line,
    // ending with the finished line and a newline.
    assert!(
        stdout.contains("\r[download] 100.0% of     14.00B at "),
        "{stdout}"
    );
    let finished = stdout
        .split('\r')
        .find(|line| line.starts_with("[download] 100% of     14.00B in 00:00:0"))
        .unwrap_or_else(|| panic!("{stdout}"));
    assert!(finished.trim_end().ends_with("B/s"), "{finished:?}");
    assert!(stdout.ends_with('\n') && !stdout.contains("Downloaded 14 bytes"));
    let quiet = std::process::Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--no-progress", "-o", "other.mp4", &server.url])
        .current_dir(directory.path())
        .output()
        .unwrap();
    let quiet = String::from_utf8(quiet.stdout).unwrap();
    assert!(quiet.contains("[download] Download completed\n"), "{quiet}");
    assert!(!quiet.contains('\r'));
    std::fs::remove_file(directory.path().join("other.mp4")).unwrap();
    assert_eq!(
        std::fs::read(directory.path().join("clip.mp4")).unwrap(),
        b"complete media"
    );
    assert_eq!(server.requests().len(), 4);
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn destination_created_during_publication_is_preserved_and_temp_is_removed() {
    let server = Server::start(|_| Response::media());
    let downloader = YoutubeDl::new().unwrap();
    let info = downloader.extract_info(&server.url).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("clip.mp4");
    let mut publishing = false;
    let error = downloader
        .download_with_progress(
            &info,
            &destination,
            DownloadOptions {
                retries: 2,
                ..Default::default()
            },
            |event| {
                assert!(!publishing, "callback invoked after publication attempt");
                if event.phase == DownloadPhase::Publishing {
                    publishing = true;
                    std::fs::write(&destination, b"competing file").unwrap();
                }
                ControlFlow::Continue(())
            },
        )
        .unwrap_err();
    assert!(publishing);
    assert!(matches!(error, Error::OutputExists(_)), "{error}");
    assert_eq!(std::fs::read(destination).unwrap(), b"competing file");
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    assert_eq!(server.requests().len(), 2);
}

#[test]
fn cli_progress_follows_httpfd_block_sizes() {
    let server = Server::start(|_| Response::media());
    let percents = |args: &[&str]| -> Vec<String> {
        let directory = tempfile::tempdir().unwrap();
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .args(["--newline", "-o", "clip.mp4"])
            .args(args)
            .arg(&server.url)
            .current_dir(directory.path())
            .output()
            .unwrap();
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .filter_map(|line| line.strip_prefix("[download] "))
            .filter(|line| line.contains("% of ") && !line.starts_with("100% "))
            .map(|line| line.split_whitespace().next().unwrap().to_owned())
            .collect()
    };
    // `data.read(block_size)` with a fixed block of 5 bytes.
    assert_eq!(
        percents(&["--buffer-size", "5", "--no-resize-buffer"]),
        ["35.7%", "71.4%", "100.0%"]
    );
    // `best_block_size` doubles the block on a fast link: 2, 4, then 8 bytes.
    assert_eq!(
        percents(&["--buffer-size", "2"]),
        ["14.3%", "42.9%", "100.0%"]
    );
}

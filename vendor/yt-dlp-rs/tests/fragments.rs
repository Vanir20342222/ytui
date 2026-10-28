use std::{
    io::{Read, Write},
    net::TcpListener,
    ops::ControlFlow,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
use yt_dlp_rs::{DownloadOptions, DownloadPhase, Error, VideoInfo, YoutubeDl};
struct Response {
    status: u16,
    headers: String,
    body: Vec<u8>,
}
impl Response {
    fn media(body: impl Into<Vec<u8>>) -> Self {
        let body = body.into();
        Self {
            status: 200,
            headers: format!(
                "Content-Type: video/mp2t\r\nContent-Length: {}\r\n",
                body.len()
            ),
            body,
        }
    }
    fn range(body: &str, content_range: &str) -> Self {
        Self {
            status: 206,
            headers: format!(
                "Content-Type: application/octet-stream\r\nContent-Length: {}\r\nContent-Range: {content_range}\r\n",
                body.len()
            ),
            body: body.as_bytes().to_vec(),
        }
    }
}
struct Server {
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Server {
    fn new(handler: impl Fn(&str, usize) -> Response + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (record, done) = (requests.clone(), stop.clone());
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
                while !request.windows(4).any(|part| part == b"\r\n\r\n") {
                    let n = stream.read(&mut buffer).unwrap();
                    if n == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..n]);
                }
                let request = String::from_utf8(request).unwrap();
                let index = record.lock().unwrap().len();
                record.lock().unwrap().push(request.clone());
                let response = handler(&request, index);
                let header = format!(
                    "HTTP/1.1 {} fixture\r\nConnection: close\r\n{}\r\n",
                    response.status, response.headers
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(&response.body);
            }
        });
        Self {
            url,
            requests,
            stop,
            worker: Some(worker),
        }
    }
    fn info(&self, fragments: serde_json::Value) -> VideoInfo {
        serde_json::from_value(serde_json::json!({"id":"fixture","title":"fixture","url":format!("{}/playlist.m3u8",self.url),"webpage_url":format!("{}/watch",self.url),"ext":"ts","extractor":"fixture","extractor_key":"Fixture","direct":false,"fragments":fragments})).unwrap()
    }
    fn whole(&self) -> VideoInfo {
        self.info(serde_json::json!([{"url":format!("{}/first",self.url)},{"url":format!("{}/second",self.url)}]))
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
fn clean(directory: &Path, expected: &[&str]) {
    let mut names: Vec<_> = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, expected);
}
#[test]
fn ordered_assembly_referrers_identity_and_fragment_progress() {
    let server = Server::new(|request, _| {
        Response::media(if request.starts_with("GET /first ") {
            b"one".to_vec()
        } else {
            b"two".to_vec()
        })
    });
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("out.ts");
    let mut progress = Vec::new();
    let bytes = YoutubeDl::new()
        .unwrap()
        .download_with_progress(
            &server.whole(),
            &output,
            DownloadOptions::default(),
            |event| {
                progress.push(event);
                ControlFlow::Continue(())
            },
        )
        .unwrap();
    assert_eq!(bytes, 6);
    assert_eq!(std::fs::read(output).unwrap(), b"onetwo");
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].starts_with("GET /first "));
    assert!(requests[1].starts_with("GET /second "));
    for request in requests {
        let lower = request.to_ascii_lowercase();
        assert!(lower.contains("accept-encoding: identity\r\n"));
        assert!(lower.contains(&format!("referer: {}/watch\r\n", server.url)));
        assert!(!lower.contains("range:"));
    }
    for index in [1, 2] {
        assert!(
            progress
                .iter()
                .any(|event| event.fragment_index == Some(index)
                    && event.fragment_count == Some(2)
                    && event.phase == DownloadPhase::Downloading)
        );
    }
    assert!(
        progress
            .iter()
            .all(|event| event.stream_index.is_none() && event.stream_count.is_none())
    );
    assert_eq!(
        progress
            .iter()
            .filter(|event| event.phase == DownloadPhase::Publishing)
            .count(),
        1
    );
    clean(directory.path(), &["out.ts"]);
}
#[test]
fn retries_discard_partial_fragment_without_repeating_previous_fragments() {
    let server = Server::new(|_, index| match index {
        0 => Response::media(b"prefix".to_vec()),
        1 => Response {
            status: 200,
            headers: "Content-Type: video/mp2t\r\nContent-Length: 4096\r\n".into(),
            body: vec![b'x'; 1024],
        },
        _ => Response::media(b"replacement".to_vec()),
    });
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("out.ts");
    let mut progress = Vec::new();
    YoutubeDl::new()
        .unwrap()
        .download_with_progress(
            &server.whole(),
            &output,
            DownloadOptions {
                retries: 1,
                ..Default::default()
            },
            |event| {
                progress.push(event);
                ControlFlow::Continue(())
            },
        )
        .unwrap();
    assert_eq!(std::fs::read(output).unwrap(), b"prefixreplacement");
    let requests = server.requests();
    assert_eq!(requests.len(), 3);
    assert!(requests[0].starts_with("GET /first "));
    assert!(requests[1].starts_with("GET /second "));
    assert!(requests[2].starts_with("GET /second "));
    assert!(!requests[2].to_ascii_lowercase().contains("range:"));
    assert!(progress.iter().any(|event| event.fragment_index == Some(2)
        && event.attempt == 2
        && event.phase == DownloadPhase::Requesting
        && event.downloaded_bytes == 6));
    assert!(
        progress.iter().any(
            |event| event.phase == DownloadPhase::WaitingToRetry && event.downloaded_bytes == 6
        )
    );
    clean(directory.path(), &["out.ts"]);
}
#[test]
fn transient_status_retries_and_cancelled_retry_wait_cleans_output() {
    for cancel in [false, true] {
        let server = Server::new(|_, index| {
            if index == 0 {
                Response {
                    status: 503,
                    headers: "Content-Length: 0\r\nRetry-After: 0\r\n".into(),
                    body: vec![],
                }
            } else {
                Response::media(b"segment".to_vec())
            }
        });
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("out.ts");
        let result = YoutubeDl::new().unwrap().download_with_progress(
            &server.whole(),
            &output,
            DownloadOptions {
                retries: 1,
                ..Default::default()
            },
            |event| {
                if cancel && event.phase == DownloadPhase::WaitingToRetry {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            },
        );
        if cancel {
            assert!(matches!(result, Err(Error::DownloadCancelled)));
            assert_eq!(server.requests().len(), 1);
            clean(directory.path(), &[]);
        } else {
            assert_eq!(result.unwrap(), 14);
            assert_eq!(server.requests().len(), 3);
            assert_eq!(std::fs::read(output).unwrap(), b"segmentsegment");
        }
    }
}
#[test]
fn exact_byte_ranges_are_ordered_and_use_explicit_bounded_headers() {
    let server = Server::new(|_, index| {
        if index == 0 {
            Response::range("BCD", "bytes 1-3/10")
        } else {
            Response::range("GH", "bytes 6-7/10")
        }
    });
    let info=server.info(serde_json::json!([{"url":format!("{}/blob",server.url),"byte_range":{"offset":1,"length":3}},{"url":format!("{}/blob",server.url),"byte_range":{"offset":6,"length":2}}]));
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("out.ts");
    assert_eq!(
        YoutubeDl::new().unwrap().download(&info, &output).unwrap(),
        5
    );
    assert_eq!(std::fs::read(output).unwrap(), b"BCDGH");
    let requests = server.requests();
    assert!(
        requests[0]
            .to_ascii_lowercase()
            .contains("range: bytes=1-3\r\n")
    );
    assert!(
        requests[1]
            .to_ascii_lowercase()
            .contains("range: bytes=6-7\r\n")
    );
}
#[test]
fn malformed_or_ignored_ranges_and_unsolicited_partials_fail_without_publication() {
    for mode in [
        "ignored",
        "wrong-start",
        "wrong-end",
        "wrong-total",
        "wrong-length",
        "encoded",
        "unsolicited",
    ] {
        let server = Server::new(move |_, _| match mode {
            "ignored" => Response::media(b"BCD".to_vec()),
            "wrong-start" => Response::range("BCD", "bytes 0-2/10"),
            "wrong-end" => Response::range("BCD", "bytes 1-4/10"),
            "wrong-total" => Response::range("BCD", "bytes 1-3/3"),
            "wrong-length" => Response::range("BC", "bytes 1-3/10"),
            "encoded" => {
                let mut response = Response::range("BCD", "bytes 1-3/10");
                response.headers.push_str("Content-Encoding: gzip\r\n");
                response
            }
            "unsolicited" => Response::range("BCD", "bytes 1-3/10"),
            _ => unreachable!(),
        });
        let fragment = if mode == "unsolicited" {
            serde_json::json!({"url":format!("{}/blob",server.url)})
        } else {
            serde_json::json!({"url":format!("{}/blob",server.url),"byte_range":{"offset":1,"length":3}})
        };
        let directory = tempfile::tempdir().unwrap();
        let result = YoutubeDl::new().unwrap().download(
            &server.info(serde_json::json!([fragment])),
            directory.path().join("out.ts"),
        );
        assert!(result.is_err(), "{mode}");
        assert_eq!(server.requests().len(), 1, "{mode}");
        clean(directory.path(), &[]);
    }
}
#[test]
fn cancellation_at_fragment_and_publication_checkpoints_cleans_files() {
    for (phase, index) in [
        (DownloadPhase::Requesting, 1),
        (DownloadPhase::Downloading, 1),
        (DownloadPhase::Requesting, 2),
        (DownloadPhase::Downloading, 2),
        (DownloadPhase::Publishing, 2),
    ] {
        let server = Server::new(|_, _| Response::media(b"media".to_vec()));
        let directory = tempfile::tempdir().unwrap();
        let result = YoutubeDl::new().unwrap().download_with_progress(
            &server.whole(),
            directory.path().join("out.ts"),
            DownloadOptions::default(),
            |event| {
                if event.phase == phase && event.fragment_index == Some(index) {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            },
        );
        assert!(matches!(result, Err(Error::DownloadCancelled)));
        clean(directory.path(), &[]);
    }
}
#[test]
fn existing_output_and_publication_race_preserve_destination() {
    for race in [false, true] {
        let server = Server::new(|_, _| Response::media(b"media".to_vec()));
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("out.ts");
        if !race {
            std::fs::write(&output, b"existing").unwrap();
        }
        let result = YoutubeDl::new().unwrap().download_with_progress(
            &server.whole(),
            &output,
            DownloadOptions::default(),
            |event| {
                if race && event.phase == DownloadPhase::Publishing {
                    std::fs::write(&output, b"existing").unwrap();
                }
                ControlFlow::Continue(())
            },
        );
        assert!(matches!(result, Err(Error::OutputExists(_))));
        assert_eq!(std::fs::read(&output).unwrap(), b"existing");
        if !race {
            assert!(server.requests().is_empty());
        }
        clean(directory.path(), &["out.ts"]);
    }
}
#[test]
fn html_nested_manifests_empty_and_permanent_http_fail_without_skipping() {
    for mode in ["html-type", "html-body", "hls", "dash", "empty", "404"] {
        let server = Server::new(move |_, index| {
            if index == 0 {
                Response::media(b"prefix".to_vec())
            } else {
                match mode {
                    "html-type" => Response {
                        status: 200,
                        headers: "Content-Type: text/html\r\nContent-Length: 4\r\n".into(),
                        body: b"oops".to_vec(),
                    },
                    "html-body" => Response {
                        status: 200,
                        headers: "Content-Length: 19\r\n".into(),
                        body: b"<!doctype html>oops".to_vec(),
                    },
                    "hls" => Response::media(b"#EXTM3U\nsegment.ts".to_vec()),
                    "dash" => Response::media(b"<MPD>no</MPD>".to_vec()),
                    "empty" => Response::media(vec![]),
                    "404" => Response {
                        status: 404,
                        headers: "Content-Length: 0\r\n".into(),
                        body: vec![],
                    },
                    _ => unreachable!(),
                }
            }
        });
        let directory = tempfile::tempdir().unwrap();
        let result = YoutubeDl::new()
            .unwrap()
            .download(&server.whole(), directory.path().join("out.ts"));
        assert!(result.is_err(), "{mode}");
        assert_eq!(server.requests().len(), 2, "{mode}");
        clean(directory.path(), &[]);
    }
}
#[test]
fn invalid_fragment_plans_fail_before_requesting() {
    for range in [
        serde_json::json!({"offset":0,"length":0}),
        serde_json::json!({"offset":u64::MAX,"length":1}),
    ] {
        let server = Server::new(|_, _| panic!("invalid plan must not request"));
        let directory = tempfile::tempdir().unwrap();
        let info = server
            .info(serde_json::json!([{"url":format!("{}/blob",server.url),"byte_range":range}]));
        assert!(
            YoutubeDl::new()
                .unwrap()
                .download(&info, directory.path().join("out.ts"))
                .is_err()
        );
        assert!(server.requests().is_empty());
        clean(directory.path(), &[]);
    }
}

#[test]
fn missing_content_type_accepts_media_bytes_but_manifest_type_is_rejected() {
    for manifest in [false, true] {
        let server = Server::new(move |_, _| Response {
            status: 200,
            headers: format!(
                "{}Content-Length: 5\r\n",
                if manifest {
                    "Content-Type: audio/x-mpegurl\r\n"
                } else {
                    ""
                }
            ),
            body: b"media".to_vec(),
        });
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("out.ts");
        let result = YoutubeDl::new().unwrap().download(&server.whole(), &output);
        if manifest {
            assert!(matches!(result, Err(Error::UnsupportedManifest(_))));
            clean(directory.path(), &[]);
        } else {
            assert_eq!(result.unwrap(), 10);
            assert_eq!(std::fs::read(&output).unwrap(), b"mediamedia");
        }
    }
}

#[test]
fn exhausted_partial_fragment_retries_remove_the_assembled_prefix() {
    let server = Server::new(|_, index| {
        if index == 0 {
            Response::media(b"prefix".to_vec())
        } else {
            Response {
                status: 200,
                headers: "Content-Type: video/mp2t\r\nContent-Length: 4096\r\n".into(),
                body: vec![b'x'; 1024],
            }
        }
    });
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("out.ts");
    assert!(
        YoutubeDl::new()
            .unwrap()
            .download_with_options(
                &server.whole(),
                &output,
                DownloadOptions {
                    fragment_retries: 1,
                    ..Default::default()
                }
            )
            .is_err()
    );
    assert_eq!(server.requests().len(), 3);
    clean(directory.path(), &[]);
}

#[test]
fn excessive_fragment_counts_and_non_http_urls_fail_before_io() {
    let server = Server::new(|_, _| panic!("invalid plan must not request"));
    let directory = tempfile::tempdir().unwrap();
    for fragments in [
        serde_json::json!([{"url":"file:///fragment.ts"}]),
        serde_json::Value::Array(vec![
            serde_json::json!({"url":format!("{}/fragment",server.url)});
            10_001
        ]),
    ] {
        assert!(
            YoutubeDl::new()
                .unwrap()
                .download(&server.info(fragments), directory.path().join("out.ts"))
                .is_err()
        );
        assert!(server.requests().is_empty());
        clean(directory.path(), &[]);
    }
}
#[test]
fn update_time_uses_the_latest_fragment_last_modified() {
    let server = Server::new(|request, _| {
        let mut response = Response::media(b"part".to_vec());
        if request.starts_with("GET /first ") {
            response
                .headers
                .push_str("Last-Modified: Wed, 21 Oct 2015 07:28:00 GMT\r\n");
        } else {
            // The epoch is ignored; the earlier fragment's time remains.
            response
                .headers
                .push_str("Last-Modified: Thu, 01 Jan 1970 00:00:00 GMT\r\n");
        }
        response
    });
    let directory = tempfile::tempdir().unwrap();
    for (update_time, name) in [(true, "out.ts"), (false, "plain.ts")] {
        let output = directory.path().join(name);
        YoutubeDl::new()
            .unwrap()
            .download_with_progress(
                &server.whole(),
                &output,
                DownloadOptions {
                    update_time,
                    ..Default::default()
                },
                |_| ControlFlow::Continue(()),
            )
            .unwrap();
        let modified = std::fs::metadata(&output)
            .unwrap()
            .modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert_eq!(modified == 1_445_412_480, update_time, "{name}");
    }
}

#[test]
fn concurrent_fragments_assemble_in_order_with_per_fragment_retries() {
    let failed_once = Arc::new(AtomicBool::new(false));
    let failure = failed_once.clone();
    let server = Server::new(move |request, _| {
        let name = request
            .split_whitespace()
            .nth(1)
            .unwrap()
            .trim_start_matches('/')
            .to_owned();
        if name == "f3" && !failure.swap(true, Ordering::Relaxed) {
            return Response {
                status: 503,
                headers: "Content-Length: 0\r\n".into(),
                body: Vec::new(),
            };
        }
        Response::media(format!("<{name}>").into_bytes())
    });
    let fragments: Vec<_> = (0..8)
        .map(|index| serde_json::json!({"url": format!("{}/f{index}", server.url)}))
        .collect();
    let info = server.info(serde_json::Value::Array(fragments));
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("out.ts");
    let bytes = YoutubeDl::new()
        .unwrap()
        .with_concurrent_fragments(3)
        .download_with_progress(&info, &output, DownloadOptions::default(), |_| {
            ControlFlow::Continue(())
        })
        .unwrap();
    let expected: String = (0..8).map(|index| format!("<f{index}>")).collect();
    assert_eq!(std::fs::read_to_string(&output).unwrap(), expected);
    assert_eq!(bytes, expected.len() as u64);
    assert!(failed_once.load(Ordering::Relaxed));
    // Every fragment once, plus the retried one; worker files are removed.
    assert_eq!(server.requests().len(), 9);
    clean(directory.path(), &["out.ts"]);
}

#[test]
fn keep_fragments_writes_upstream_frag_files_sequentially_and_concurrently() {
    for concurrency in [1, 3] {
        let server = Server::new(|request, _| {
            let name = request
                .split_whitespace()
                .nth(1)
                .unwrap()
                .trim_start_matches('/')
                .to_owned();
            Response::media(format!("<{name}>").into_bytes())
        });
        let fragments: Vec<_> = (0..4)
            .map(|index| serde_json::json!({"url": format!("{}/f{index}", server.url)}))
            .collect();
        let info = server.info(serde_json::Value::Array(fragments));
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("o.ts");
        YoutubeDl::new()
            .unwrap()
            .with_concurrent_fragments(concurrency)
            .with_keep_fragments(true)
            .download_with_progress(
                &info,
                &output,
                DownloadOptions {
                    part_files: yt_dlp_rs::PartFiles::Part { resume: false },
                    ..Default::default()
                },
                |_| ControlFlow::Continue(()),
            )
            .unwrap();
        // Upstream names them `TMPFILENAME-FragN`.
        for index in 0..4 {
            assert_eq!(
                std::fs::read_to_string(
                    directory
                        .path()
                        .join(format!("o.ts.part-Frag{}", index + 1))
                )
                .unwrap(),
                format!("<f{index}>")
            );
        }
        clean(
            directory.path(),
            &[
                "o.ts",
                "o.ts.part-Frag1",
                "o.ts.part-Frag2",
                "o.ts.part-Frag3",
                "o.ts.part-Frag4",
            ],
        );
    }
}

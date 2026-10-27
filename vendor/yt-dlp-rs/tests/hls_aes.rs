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

const KEY: [u8; 16] = [0x42; 16];
const IV: [u8; 16] = [0x24; 16];
const PLAIN: &[u8] = b"hello world! this is my plaintext.";
fn fixed_cipher() -> Vec<u8> {
    let hex = "c7fe247ef97b21f07cbdd26cb5d346bfd27867cb00d9486723e159978fb9a5f914cfb228a710de4171e396e7b6cf859e";
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect()
}
fn encrypt(plain: &[u8], key: &[u8; 16]) -> Vec<u8> {
    use aes::cipher::{BlockModeEncrypt, KeyIvInit, block_padding::Pkcs7};
    let mut buffer = vec![0; plain.len() + 16];
    buffer[..plain.len()].copy_from_slice(plain);
    cbc::Encryptor::<aes::Aes128>::new(key.into(), (&IV).into())
        .encrypt_padded::<Pkcs7>(&mut buffer, plain.len())
        .unwrap()
        .to_vec()
}
fn encrypted_info(server: &Server, keys: &[&str]) -> VideoInfo {
    server.info(serde_json::Value::Array(keys.iter().enumerate().map(|(index,key)|serde_json::json!({"url":format!("{}/segment-{index}",server.url),"encryption":{"key_url":format!("{}/{key}",server.url),"iv":IV}})).collect()))
}
#[test]
fn fixed_vector_key_cache_rotation_metadata_and_plaintext_counts() {
    let server = Server::new(|request, _| {
        if request.starts_with("GET /key-two ") {
            Response::media(vec![0x43; 16])
        } else if request.starts_with("GET /key ") {
            Response::media(KEY.to_vec())
        } else if request.starts_with("GET /segment-2 ") {
            Response::media(encrypt(PLAIN, &[0x43; 16]))
        } else {
            Response::media(fixed_cipher())
        }
    });
    let info = encrypted_info(&server, &["key", "key", "key-two", "key"]);
    let before = serde_json::to_string(&info).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("out.ts");
    let mut progress = Vec::new();
    let count = YoutubeDl::new()
        .unwrap()
        .download_with_progress(&info, &output, DownloadOptions::default(), |event| {
            progress.push(event);
            ControlFlow::Continue(())
        })
        .unwrap();
    assert_eq!(count, 4 * PLAIN.len() as u64);
    assert_eq!(std::fs::read(&output).unwrap(), PLAIN.repeat(4));
    assert_eq!(before, serde_json::to_string(&info).unwrap());
    assert!(!before.contains("BBBBBBBBBBBBBBBB"));
    let requests = server.requests();
    assert_eq!(requests.len(), 6);
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.starts_with("GET /key "))
            .count(),
        1
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.starts_with("GET /key-two "))
            .count(),
        1
    );
    for request in &requests {
        let lower = request.to_ascii_lowercase();
        assert!(lower.contains("accept-encoding: identity\r\n"));
        assert!(lower.contains(&format!("referer: {}/watch\r\n", server.url)));
    }
    assert!(progress.iter().all(|event| event.downloaded_bytes <= count));
    assert_eq!(progress.last().unwrap().downloaded_bytes, count);
    clean(directory.path(), &["out.ts"]);
}
#[test]
fn key_status_and_truncated_key_retries_share_fragment_budget() {
    for truncated in [false, true] {
        let server = Server::new(move |request, index| {
            if index == 0 {
                if truncated {
                    Response {
                        status: 200,
                        headers: "Content-Type: application/octet-stream\r\nContent-Length: 16\r\n"
                            .into(),
                        body: vec![0x42; 8],
                    }
                } else {
                    Response {
                        status: 503,
                        headers: "Content-Length: 0\r\n".into(),
                        body: vec![],
                    }
                }
            } else if request.starts_with("GET /key ") {
                Response::media(KEY.to_vec())
            } else {
                Response::media(fixed_cipher())
            }
        });
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("out.ts");
        let mut second_attempt = false;
        YoutubeDl::new()
            .unwrap()
            .download_with_progress(
                &encrypted_info(&server, &["key"]),
                &output,
                DownloadOptions {
                    retries: 1,
                    ..Default::default()
                },
                |event| {
                    second_attempt |= event.attempt == 2;
                    ControlFlow::Continue(())
                },
            )
            .unwrap();
        assert!(second_attempt);
        assert_eq!(std::fs::read(output).unwrap(), PLAIN);
        assert_eq!(server.requests().len(), 3);
    }
}
#[test]
fn ciphertext_retry_keeps_key_and_discards_partial_plaintext() {
    let plain = vec![b'v'; 130_007];
    let encrypted = encrypt(&plain, &KEY);
    let server = Server::new(move |request, index| {
        if request.starts_with("GET /key ") {
            Response::media(KEY.to_vec())
        } else if index == 1 {
            Response {
                status: 200,
                headers: format!(
                    "Content-Type: application/octet-stream\r\nContent-Length: {}\r\n",
                    encrypted.len()
                ),
                body: encrypted[..65_536].to_vec(),
            }
        } else {
            Response::media(encrypted.clone())
        }
    });
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("out.ts");
    let mut retry_reset = false;
    let mut chunks = 0;
    let bytes = YoutubeDl::new()
        .unwrap()
        .download_with_progress(
            &encrypted_info(&server, &["key"]),
            &output,
            DownloadOptions {
                retries: 1,
                ..Default::default()
            },
            |event| {
                retry_reset |= event.attempt == 2
                    && event.phase == DownloadPhase::Requesting
                    && event.downloaded_bytes == 0;
                if event.phase == DownloadPhase::Downloading && event.downloaded_bytes > 0 {
                    chunks += 1;
                }
                ControlFlow::Continue(())
            },
        )
        .unwrap();
    assert_eq!(bytes, plain.len() as u64);
    assert_eq!(std::fs::read(&output).unwrap(), plain);
    assert!(retry_reset);
    assert!(chunks >= 4);
    let requests = server.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.starts_with("GET /key "))
            .count(),
        1
    );
    clean(directory.path(), &["out.ts"]);
}
#[test]
fn invalid_keys_padding_ciphertext_and_decrypted_html_fail_without_publication() {
    for mode in [
        "short-key",
        "long-key",
        "html-key",
        "encoded-key",
        "partial-key",
        "padding",
        "non-block",
        "decrypted-html",
    ] {
        let server = Server::new(move |request, _| {
            if request.starts_with("GET /key ") {
                match mode {
                    "short-key" => Response::media(vec![0x42; 15]),
                    "long-key" => Response::media(vec![0x42; 17]),
                    "html-key" => Response {
                        status: 200,
                        headers: "Content-Type: text/html\r\nContent-Length: 16\r\n".into(),
                        body: b"<html>secret!!!!".to_vec(),
                    },
                    "encoded-key" => {
                        let mut response = Response::media(KEY.to_vec());
                        response.headers.push_str("Content-Encoding: gzip\r\n");
                        response
                    }
                    "partial-key" => Response::range("BBBBBBBBBBBBBBBB", "bytes 0-15/32"),
                    _ => Response::media(KEY.to_vec()),
                }
            } else {
                let mut cipher = fixed_cipher();
                match mode {
                    "padding" => cipher[31] ^= 1,
                    "non-block" => {
                        cipher.pop();
                    }
                    "decrypted-html" => {
                        cipher = encrypt(b"<!doctype html><html>not media</html>", &KEY)
                    }
                    _ => {}
                }
                Response::media(cipher)
            }
        });
        let directory = tempfile::tempdir().unwrap();
        let result = YoutubeDl::new().unwrap().download(
            &encrypted_info(&server, &["key"]),
            directory.path().join("out.ts"),
        );
        let error = result.unwrap_err().to_string();
        assert!(!error.contains("BBBBBBBBBBBBBBBB"));
        clean(directory.path(), &[]);
        assert_eq!(
            server.requests().len(),
            if mode.ends_with("key") { 1 } else { 2 },
            "{mode}"
        );
    }
}
#[test]
fn cancellation_before_key_during_key_and_after_plaintext_cleans_files() {
    for stage in [0, 1, 2] {
        let server = Server::new(|request, _| {
            if request.starts_with("GET /key ") {
                Response::media(KEY.to_vec())
            } else {
                Response::media(encrypt(&vec![b'v'; 100_000], &KEY))
            }
        });
        let directory = tempfile::tempdir().unwrap();
        let mut cancelled = false;
        let result = YoutubeDl::new().unwrap().download_with_progress(
            &encrypted_info(&server, &["key"]),
            directory.path().join("out.ts"),
            DownloadOptions::default(),
            |event| {
                let stop = match stage {
                    0 => event.phase == DownloadPhase::Requesting,
                    1 => event.phase == DownloadPhase::Downloading,
                    2 => event.phase == DownloadPhase::Downloading && event.downloaded_bytes > 0,
                    _ => unreachable!(),
                };
                if stop {
                    cancelled = true;
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            },
        );
        assert!(cancelled);
        assert!(matches!(result, Err(Error::DownloadCancelled)));
        assert_eq!(server.requests().len(), stage);
        clean(directory.path(), &[]);
    }
}
#[test]
fn ciphertext_ranges_validate_encrypted_length_but_publish_plaintext_length() {
    let server = Server::new(|request, _| {
        if request.starts_with("GET /key ") {
            Response::media(KEY.to_vec())
        } else {
            let mut response = Response::media(fixed_cipher());
            response.status = 206;
            response
                .headers
                .push_str("Content-Range: bytes 16-63/100\r\n");
            response
        }
    });
    let mut info = encrypted_info(&server, &["key"]);
    info.fragments[0].byte_range = Some(yt_dlp_rs::ByteRange {
        offset: 16,
        length: 48,
    });
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("out.ts");
    assert_eq!(
        YoutubeDl::new().unwrap().download(&info, &output).unwrap(),
        PLAIN.len() as u64
    );
    assert_eq!(std::fs::read(output).unwrap(), PLAIN);
    assert!(
        server.requests()[1]
            .to_ascii_lowercase()
            .contains("range: bytes=16-63\r\n")
    );
}

#[test]
fn arbitrary_raw_key_patterns_and_text_plain_keys_are_supported() {
    let key = *b"<html>raw-key-16";
    let server = Server::new(move |request, _| {
        if request.starts_with("GET /key ") {
            Response {
                status: 200,
                headers: "Content-Type: text/plain\r\nContent-Length: 16\r\n".into(),
                body: key.to_vec(),
            }
        } else {
            Response::media(encrypt(PLAIN, &key))
        }
    });
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("out.ts");
    YoutubeDl::new()
        .unwrap()
        .download(&encrypted_info(&server, &["key"]), &output)
        .unwrap();
    assert_eq!(std::fs::read(output).unwrap(), PLAIN);
}

#[test]
fn non_http_key_urls_fail_before_fragment_or_key_requests() {
    let server = Server::new(|_, _| panic!("invalid key plan must not request"));
    let directory = tempfile::tempdir().unwrap();
    let mut info = encrypted_info(&server, &["key"]);
    info.fragments[0].encryption.as_mut().unwrap().key_url =
        url::Url::parse("file:///key").unwrap();
    let result = YoutubeDl::new()
        .unwrap()
        .download(&info, directory.path().join("out.ts"));
    assert!(matches!(result, Err(Error::UnsupportedScheme(_))));
    assert!(server.requests().is_empty());
    clean(directory.path(), &[]);
}

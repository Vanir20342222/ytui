//! Network options through the CLI: certificate validation, socket timeouts
//! and source addresses, with saved offline TLS credentials.
use rustls::{
    ServerConfig, ServerConnection, StreamOwned,
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
};
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    process::{Command, Output},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

const CERT: &[u8] = include_bytes!("fixtures/proxies/tls/server.der");
const KEY: &[u8] = include_bytes!("fixtures/proxies/tls/key.der");

fn tls_config() -> Arc<ServerConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(CERT.to_vec())],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(KEY.to_vec())),
        )
        .unwrap();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Arc::new(config)
}

/// Serves `media` over HTTPS (or stalls without responding) until dropped.
struct Server {
    port: u16,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Server {
    fn new(tls: bool, stall: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let config = tls_config();
        let worker = thread::spawn(move || {
            let mut held = Vec::new();
            while !done.load(Ordering::Relaxed) {
                let stream = match listener.accept() {
                    Ok((stream, _)) => stream,
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
                if stall {
                    // Accept the connection but never answer.
                    held.push(stream);
                    continue;
                }
                let respond = |stream: &mut dyn ReadWrite| {
                    let mut request = Vec::new();
                    let mut byte = [0];
                    while !request.ends_with(b"\r\n\r\n") {
                        match stream.read(&mut byte) {
                            Ok(1) => request.push(byte[0]),
                            _ => return,
                        }
                    }
                    let _ = stream.write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nContent-Length: 5\r\nConnection: close\r\n\r\nmedia",
                    );
                    let _ = stream.flush();
                };
                if tls {
                    let connection = ServerConnection::new(config.clone()).unwrap();
                    let mut stream = StreamOwned::new(connection, stream);
                    respond(&mut stream);
                    stream.conn.send_close_notify();
                    let _ = stream.flush();
                } else {
                    let mut stream = stream;
                    respond(&mut stream);
                }
            }
        });
        Self {
            port,
            stop,
            worker: Some(worker),
        }
    }
}

trait ReadWrite: Read + Write {}
impl<T: Read + Write> ReadWrite for T {}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
    }
}

fn cli(args: &[&str], directory: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(args)
        .current_dir(directory)
        .output()
        .unwrap()
}

#[test]
fn no_check_certificates_accepts_an_untrusted_https_origin() {
    let server = Server::new(true, false);
    let directory = tempfile::tempdir().unwrap();
    let url = format!("https://localhost:{}/clip.mp4", server.port);
    let rejected = cli(&["--proxy", "", &url], directory.path());
    assert!(!rejected.status.success());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    let accepted = cli(
        &["--proxy", "", "--no-check-certificates", &url],
        directory.path(),
    );
    assert!(
        accepted.status.success(),
        "{}",
        String::from_utf8_lossy(&accepted.stderr)
    );
    assert_eq!(
        std::fs::read(directory.path().join("clip [clip].mp4")).unwrap(),
        b"media"
    );
}

#[test]
fn socket_timeout_bounds_a_stalled_response_and_invalid_values_fail_early() {
    let server = Server::new(false, true);
    let directory = tempfile::tempdir().unwrap();
    let url = format!("http://127.0.0.1:{}/clip.mp4", server.port);
    let start = Instant::now();
    let output = cli(
        &["--proxy", "", "--socket-timeout", "0.5", "-s", &url],
        directory.path(),
    );
    assert!(!output.status.success());
    assert!(start.elapsed() < Duration::from_secs(10));
    for value in ["-1", "nan", "inf"] {
        let output = cli(&["--socket-timeout", value, &url], directory.path());
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("socket timeout"));
    }
}

#[test]
fn source_address_binds_and_force_ipv4_uses_the_last_family_option() {
    let server = Server::new(false, false);
    let directory = tempfile::tempdir().unwrap();
    let url = format!("http://127.0.0.1:{}/clip.mp4", server.port);
    for args in [
        vec!["--source-address", "127.0.0.1"],
        vec!["-6", "-4"],
        vec!["--source-address", "::1", "--force-ipv4"],
    ] {
        let mut all = vec!["--proxy", "", "-s"];
        all.extend(args.iter().copied());
        all.push(&url);
        let output = cli(&all, directory.path());
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let invalid = cli(&["--source-address", "not-an-ip", &url], directory.path());
    assert!(!invalid.status.success());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("source address"));
}

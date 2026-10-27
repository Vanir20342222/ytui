//! `--client-certificate`: TLS client authentication against a local server
//! that requires a certificate from the fixture CA.
use rustls::{
    RootCertStore, ServerConfig, ServerConnection, StreamOwned,
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
    server::WebPkiClientVerifier,
};
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::Arc,
    thread,
    time::Duration,
};

const SERVER_CERT: &[u8] = include_bytes!("fixtures/proxies/tls/server.der");
const SERVER_KEY: &[u8] = include_bytes!("fixtures/proxies/tls/key.der");
const CLIENT_CA: &[u8] = include_bytes!("fixtures/client_cert/ca.der");

fn serve() -> u16 {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut roots = RootCertStore::empty();
    roots.add(CertificateDer::from(CLIENT_CA.to_vec())).unwrap();
    let verifier = WebPkiClientVerifier::builder_with_provider(Arc::new(roots), provider.clone())
        .build()
        .unwrap();
    let mut config = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_client_cert_verifier(verifier)
        .with_single_cert(
            vec![CertificateDer::from(SERVER_CERT.to_vec())],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(SERVER_KEY.to_vec())),
        )
        .unwrap();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    let config = Arc::new(config);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let connection = ServerConnection::new(config.clone()).unwrap();
            let mut stream = StreamOwned::new(connection, stream);
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") {
                match stream.read(&mut byte) {
                    Ok(1) => request.push(byte[0]),
                    _ => break,
                }
            }
            if request.ends_with(b"\r\n\r\n") {
                let _ = stream.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nContent-Length: 5\r\nConnection: close\r\n\r\nmedia",
                );
                stream.conn.send_close_notify();
                let _ = stream.flush();
            }
        }
    });
    port
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/client_cert")
        .join(name)
}

fn run(port: u16, args: &[&str], directory: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args([
            "--ignore-config",
            "--no-check-certificates",
            "--retries",
            "0",
            "-o",
            "o.%(ext)s",
        ])
        .args(args)
        .arg(format!("https://127.0.0.1:{port}/v.mp4"))
        .current_dir(directory)
        .output()
        .unwrap()
}

#[test]
fn client_certificates_authenticate_tls_requests() {
    let port = serve();
    let directory = tempfile::tempdir().unwrap();
    // The server requires a client certificate.
    let output = run(port, &[], directory.path());
    assert!(!output.status.success());
    let combined = fixture("combined.pem");
    let output = run(
        port,
        &["--client-certificate", combined.to_str().unwrap()],
        directory.path(),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read(directory.path().join("o.mp4")).unwrap(),
        b"media"
    );
    std::fs::remove_file(directory.path().join("o.mp4")).unwrap();
    let (cert, key) = (fixture("cert.pem"), fixture("key.pem"));
    let output = run(
        port,
        &[
            "--client-certificate",
            cert.to_str().unwrap(),
            "--client-certificate-key",
            key.to_str().unwrap(),
        ],
        directory.path(),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let encrypted = fixture("encrypted-key.pem");
    let output = run(
        port,
        &[
            "--client-certificate",
            cert.to_str().unwrap(),
            "--client-certificate-key",
            encrypted.to_str().unwrap(),
            "--client-certificate-password",
            "secret",
        ],
        directory.path(),
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("encrypted client certificate keys"));
}

//! Saved-certificate TLS regression tests. No public CA, origin DNS or external IO.
use super::ProxyConfig;
use reqwest::blocking::{Client, ClientBuilder};
use rustls::{
    ServerConfig, ServerConnection, StreamOwned,
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

const CA: &[u8] = include_bytes!("../../../tests/fixtures/proxies/tls/ca.der");
const CERT: &[u8] = include_bytes!("../../../tests/fixtures/proxies/tls/server.der");
const KEY: &[u8] = include_bytes!("../../../tests/fixtures/proxies/tls/key.der");
fn assert_proxy_auth(request: &str) {
    let values: Vec<_> = request
        .lines()
        .filter_map(|line| line.split_once(':'))
        .filter(|(name, _)| name.eq_ignore_ascii_case("proxy-authorization"))
        .map(|(_, value)| value.trim())
        .collect();
    assert_eq!(values, ["Basic dXNlcjpwYXNz"]);
}

fn server_config() -> Arc<ServerConfig> {
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
fn listener() -> TcpListener {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    listener
}
fn accept(listener: TcpListener) -> TcpStream {
    let start = Instant::now();
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                return stream;
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(
                    start.elapsed() < Duration::from_secs(5),
                    "TLS proxy accept deadline"
                );
                thread::sleep(Duration::from_millis(2));
            }
            Err(error) => panic!("TLS proxy accept: {error}"),
        }
    }
}
fn headers(stream: &mut impl Read) -> std::io::Result<String> {
    let mut bytes = Vec::new();
    let mut byte = [0];
    while !bytes.ends_with(b"\r\n\r\n") {
        if bytes.len() >= 16 * 1024 {
            return Err(std::io::Error::other("test header budget"));
        }
        stream.read_exact(&mut byte)?;
        bytes.push(byte[0]);
    }
    String::from_utf8(bytes).map_err(std::io::Error::other)
}
fn media(stream: &mut impl Write) {
    stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nContent-Length: 9\r\nConnection: close\r\n\r\nTLS media").unwrap();
    stream.flush().unwrap();
}
fn trusted_builder() -> ClientBuilder {
    Client::builder()
        .tls_built_in_root_certs(false)
        .add_root_certificate(reqwest::Certificate::from_der(CA).unwrap())
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(5))
}
fn client(proxy: &str) -> Client {
    ProxyConfig::parse(Some(proxy))
        .unwrap()
        .apply(trusted_builder())
        .unwrap()
        .build()
        .unwrap()
}

#[test]
fn http_connect_keeps_proxy_credentials_outside_verified_origin_tls() {
    let listener = listener();
    let address = listener.local_addr().unwrap();
    thread::scope(|scope| {
        let server = scope.spawn(move || {
            let mut stream = accept(listener);
            let connect = headers(&mut stream).unwrap();
            stream
                .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                .unwrap();
            stream.flush().unwrap();
            let mut tls = StreamOwned::new(ServerConnection::new(server_config()).unwrap(), stream);
            let inner = headers(&mut tls).unwrap();
            media(&mut tls);
            (connect, inner)
        });
        let result = client(&format!("http://user:pass@{address}"))
            .get("https://media.example.test/media.mp4")
            .send()
            .and_then(|response| response.text());
        let (connect, inner) = server.join().unwrap();
        assert_eq!(result.unwrap(), "TLS media");
        assert!(
            connect.starts_with("CONNECT media.example.test:443 HTTP/1.1\r\n"),
            "{connect}"
        );
        assert_proxy_auth(&connect);
        assert!(inner.starts_with("GET /media.mp4 HTTP/1.1\r\n"), "{inner}");
        assert!(
            inner
                .to_ascii_lowercase()
                .contains("host: media.example.test\r\n")
        );
        assert!(!inner.to_ascii_lowercase().contains("proxy-authorization:"));
        assert!(!inner.to_ascii_lowercase().contains("authorization:"));
    });
}

#[test]
fn https_proxy_tls_is_verified_and_carries_absolute_http_request() {
    let listener = listener();
    let port = listener.local_addr().unwrap().port();
    thread::scope(|scope| {
        let server = scope.spawn(move || {
            let mut tls = StreamOwned::new(
                ServerConnection::new(server_config()).unwrap(),
                accept(listener),
            );
            let request = headers(&mut tls).unwrap();
            media(&mut tls);
            request
        });
        let result = client(&format!("https://user:pass@localhost:{port}"))
            .get("http://media.example.test/media.mp4")
            .send()
            .and_then(|response| response.text());
        let request = server.join().unwrap();
        assert_eq!(result.unwrap(), "TLS media");
        assert!(
            request.starts_with("GET http://media.example.test/media.mp4 HTTP/1.1\r\n"),
            "{request}"
        );
        assert_proxy_auth(&request);
    });
}

#[test]
fn configured_proxy_does_not_trust_the_saved_private_origin_ca() {
    let listener = listener();
    let address = listener.local_addr().unwrap();
    thread::scope(|scope| {
        let server = scope.spawn(move || {
            let mut stream = accept(listener);
            let connect = headers(&mut stream).unwrap();
            stream
                .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                .unwrap();
            stream.flush().unwrap();
            let mut tls = StreamOwned::new(ServerConnection::new(server_config()).unwrap(), stream);
            (connect, headers(&mut tls))
        });
        // The production constructor uses its normal roots and must reject this
        // test CA. Trust is overridden only in the two private test builders above.
        let options = crate::HttpOptions {
            proxy: Some(format!("http://{address}")),
            ..Default::default()
        };
        let result = crate::YoutubeDl::with_options(crate::YoutubeOptions::default(), options)
            .unwrap()
            .extract_info("https://media.example.test/media.mp4");
        let (connect, inner) = server.join().unwrap();
        assert!(connect.starts_with("CONNECT media.example.test:443 HTTP/1.1\r\n"));
        assert!(
            inner.is_err(),
            "untrusted TLS must not reach an inner HTTP request"
        );
        let error = result.unwrap_err();
        // Reported like upstream's `Unable to download webpage`.
        assert!(matches!(error, crate::Error::Webpage { .. }), "{error}");
        let mut chain = String::new();
        let mut source: Option<&dyn std::error::Error> = Some(&error);
        while let Some(error) = source {
            chain.push_str(&error.to_string().to_ascii_lowercase());
            source = error.source();
        }
        assert!(
            chain.contains("certificate") || chain.contains("unknownissuer"),
            "{chain}"
        );
    });
}

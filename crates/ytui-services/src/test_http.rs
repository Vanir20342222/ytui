//! Loopback fixtures only; no external account, provider, keyring or desktop access.
use tokio::io::{AsyncReadExt, AsyncWriteExt};
pub async fn serve(
    handler: impl Fn(&str) -> (u16, String, Vec<u8>) + Send + 'static,
) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut chunk = [0u8; 1024];
            while !request.windows(4).any(|v| v == b"\r\n\r\n") && request.len() < 8192 {
                let count = stream.read(&mut chunk).await.unwrap();
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..count]);
            }
            let request = String::from_utf8_lossy(&request);
            assert!(!request.to_ascii_lowercase().contains("\ncookie:"));
            let target = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap_or("/");
            let (status, headers, body) = handler(target);
            let header = format!(
                "HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n",
                body.len()
            );
            if stream.write_all(header.as_bytes()).await.is_ok() {
                let _ = stream.write_all(&body).await;
            }
        }
    });
    (base, task)
}

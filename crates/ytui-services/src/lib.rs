mod audio;
pub mod auth;
pub mod browse;
mod cache;
pub mod desktop;
pub mod diagnostics;
pub mod download;
pub mod failure;
pub mod formats;
mod http;
pub mod images;
pub mod library;
pub mod lyrics;
pub mod migration;
pub mod mpris;
pub mod native;
mod native_playback;
mod native_post;
pub mod native_runtime;
pub mod paths;
mod playback_cache;
mod playback_extract;
pub mod player;
pub mod process;
pub mod profiles;
pub mod queue_import;
pub mod radio;
pub mod sponsorblock;
pub mod store;
#[cfg(test)]
mod test_http;
pub mod theme;
mod throttle;
pub mod vpn;
pub mod watch;
mod worker_process;

pub fn http_client(proxy: Option<&str>) -> anyhow::Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/132.0.0.0 Safari/537.36")
        .timeout(std::time::Duration::from_secs(25))
        .connect_timeout(std::time::Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none());
    if let Some(proxy) = explicit_proxy(proxy)? {
        builder = builder.no_proxy().proxy(proxy);
    }
    Ok(builder.build()?)
}

fn explicit_proxy(proxy: Option<&str>) -> anyhow::Result<Option<reqwest::Proxy>> {
    proxy
        .filter(|p| !p.is_empty())
        .map(|proxy| {
            let mut proxy =
                url::Url::parse(proxy).map_err(|_| anyhow::anyhow!("Invalid proxy URL"))?;
            // Explicit SOCKS5 always uses proxy-side DNS, including cover downloads.
            if proxy.scheme() == "socks5" {
                let _ = proxy.set_scheme("socks5h");
            }
            reqwest::Proxy::all(proxy).map_err(Into::into)
        })
        .transpose()
}

// Only call from a blocking worker. Match the async client's routing and TLS
// policy; public covers carry no browser authorization or cookie headers.
pub(crate) fn blocking_http_client(
    proxy: Option<&str>,
) -> anyhow::Result<reqwest::blocking::Client> {
    let mut builder = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .connect_timeout(std::time::Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() > 4 {
                return attempt.error("Service redirect limit exceeded");
            }
            if ytui_core::http_url(attempt.url().as_str()).is_err()
                || (attempt.previous().iter().any(|url| url.scheme() == "https")
                    && attempt.url().scheme() != "https")
            {
                return attempt.error("Unsafe service redirect");
            }
            attempt.follow()
        }));
    if let Some(proxy) = explicit_proxy(proxy)? {
        builder = builder.no_proxy().proxy(proxy);
    }
    Ok(builder.build()?)
}

#[cfg(test)]
mod proxy_tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    #[tokio::test]
    async fn explicit_http_proxy_receives_unresolvable_destination_without_direct_dns() {
        let (proxy, server) = test_http::serve_request(|request| {
            assert!(request.starts_with("GET http://ytui-proxy-fixture.invalid:2345/check "));
            (200, String::new(), b"proxied".to_vec())
        })
        .await;
        let response = http_client(Some(&proxy))
            .unwrap()
            .get("http://ytui-proxy-fixture.invalid:2345/check")
            .send()
            .await
            .unwrap();
        assert_eq!(response.text().await.unwrap(), "proxied");
        let body = tokio::task::spawn_blocking(move || {
            blocking_http_client(Some(&proxy))
                .unwrap()
                .get("http://ytui-proxy-fixture.invalid:2345/check")
                .send()
                .unwrap()
                .text()
                .unwrap()
        })
        .await
        .unwrap();
        assert_eq!(body, "proxied");
        server.abort();
    }
    #[tokio::test]
    async fn explicit_socks5_uses_remote_dns_and_does_not_fall_back_to_direct() {
        for (scheme, blocking) in [
            ("socks5", false),
            ("socks5h", false),
            ("socks5", true),
            ("socks5h", true),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let proxy = format!("{scheme}://{}", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut hello = [0; 2];
                stream.read_exact(&mut hello).await.unwrap();
                assert_eq!(hello[0], 5);
                let mut methods = vec![0; hello[1] as usize];
                stream.read_exact(&mut methods).await.unwrap();
                stream.write_all(&[5, 0]).await.unwrap();
                let mut command = [0; 4];
                stream.read_exact(&mut command).await.unwrap();
                assert_eq!(
                    command,
                    [5, 1, 0, 3],
                    "destination must be a remote DNS name"
                );
                let length = stream.read_u8().await.unwrap();
                let mut domain = vec![0; length as usize];
                stream.read_exact(&mut domain).await.unwrap();
                assert_eq!(domain, b"ytui-proxy-fixture.invalid");
                assert_eq!(stream.read_u16().await.unwrap(), 2345);
                stream
                    .write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0])
                    .await
                    .unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    request.push(stream.read_u8().await.unwrap());
                }
                assert!(request.starts_with(b"GET /check "));
                stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\nproxied",
                    )
                    .await
                    .unwrap();
            });
            let body = if blocking {
                tokio::task::spawn_blocking(move || {
                    blocking_http_client(Some(&proxy))
                        .unwrap()
                        .get("http://ytui-proxy-fixture.invalid:2345/check")
                        .send()
                        .unwrap()
                        .text()
                        .unwrap()
                })
                .await
                .unwrap()
            } else {
                http_client(Some(&proxy))
                    .unwrap()
                    .get("http://ytui-proxy-fixture.invalid:2345/check")
                    .send()
                    .await
                    .unwrap()
                    .text()
                    .await
                    .unwrap()
            };
            assert_eq!(body, "proxied");
            server.await.unwrap();
        }
    }
}

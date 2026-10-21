//! Explicit redirects: browser credentials never cross an origin boundary.
use anyhow::{Context, Result, bail, ensure};
use reqwest::{RequestBuilder, Response};
use url::Url;
use ytui_core::http_url;

#[derive(Clone, Copy)]
pub(crate) enum Redirects {
    SameOrigin,
    Public,
}

pub(crate) fn request_error(service: &str, url: &Url, error: reqwest::Error) -> anyhow::Error {
    let reason = if error.is_timeout() {
        "request timed out".to_owned()
    } else {
        crate::process::diagnostic(&format!("{:#}", anyhow::Error::new(error.without_url())))
    };
    anyhow::anyhow!(
        "{service}: cannot reach {} ({reason}). Check DNS, TLS certificates, proxy or VPN.",
        url.host_str().unwrap_or("the service")
    )
}

fn destination(start: &Url, current: &Url, location: &str, policy: Redirects) -> Result<Url> {
    let next = current.join(location).context("Invalid service redirect")?;
    http_url(next.as_str()).context("Unsafe service redirect")?;
    ensure!(
        start.scheme() != "https" || next.scheme() == "https",
        "Service redirect would downgrade HTTPS"
    );
    if matches!(policy, Redirects::SameOrigin) && start.origin() != next.origin() {
        let host = next.host_str().unwrap_or("another service");
        if [
            "consent.youtube.com",
            "consent.google.com",
            "accounts.google.com",
        ]
        .contains(&host)
        {
            bail!(
                "YouTube requires browser consent or sign-in at {host}. Open YouTube and YouTube Music in your browser, then use :auth to import that profile."
            );
        }
        bail!(
            "YouTube redirected to another origin ({host}); credentials were not forwarded. Check service availability in your VPN region or refresh your browser session with :auth."
        );
    }
    Ok(next)
}

pub(crate) async fn get(
    start: Url,
    mut request: impl FnMut(&Url) -> Result<RequestBuilder>,
    policy: Redirects,
    service: &str,
) -> Result<Response> {
    let mut url = start.clone();
    for hop in 0..=4 {
        let response = request(&url)?
            .send()
            .await
            .map_err(|error| request_error(service, &url, error))?;
        if !response.status().is_redirection() {
            return Ok(response);
        }
        ensure!(
            hop < 4,
            "{service}: redirect limit exceeded (four redirects)"
        );
        let location = response
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .context("Service redirect has no destination")?;
        url = destination(&start, &url, location, policy)?;
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn browser_redirects_are_scoped_and_public_redirects_never_downgrade() {
        let start = Url::parse("https://music.youtube.com/").unwrap();
        assert_eq!(
            destination(&start, &start, "/?cbrd=1", Redirects::SameOrigin)
                .unwrap()
                .host_str(),
            start.host_str()
        );
        for target in [
            "https://example.test/",
            "https://www.youtube.com/",
            "http://music.youtube.com/",
            "https://music.youtube.com:444/",
            "https://user:secret@music.youtube.com/",
            "file:///secret",
        ] {
            assert!(destination(&start, &start, target, Redirects::SameOrigin).is_err());
        }
        let error = destination(
            &start,
            &start,
            "https://consent.youtube.com/m?continue=synthetic-secret",
            Redirects::SameOrigin,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains(":auth"));
        assert!(!error.contains("synthetic-secret"));
        assert!(destination(&start, &start, "http://radio.test/", Redirects::Public).is_err());
        assert!(destination(&start, &start, "https://radio.test/", Redirects::Public).is_ok());
    }

    #[tokio::test]
    async fn redirects_do_not_contact_a_foreign_origin_or_loop_forever() {
        let contacted = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let observed = contacted.clone();
        let (foreign, foreign_task) = crate::test_http::serve_request(move |_| {
            observed.store(true, std::sync::atomic::Ordering::SeqCst);
            (200, String::new(), b"unexpected".to_vec())
        })
        .await;
        let (base, task) = crate::test_http::serve_request(move |request| {
            let lower = request.to_ascii_lowercase();
            assert!(lower.contains("\ncookie: synthetic=private\r"));
            assert!(lower.contains("\nauthorization: synthetic\r"));
            let path = request
                .lines()
                .next()
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap();
            let target = if path == "/loop" {
                "/loop".to_owned()
            } else {
                foreign.clone()
            };
            (302, format!("Location: {target}\r\n"), vec![])
        })
        .await;
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        for path in ["/foreign", "/loop"] {
            assert!(
                get(
                    Url::parse(&format!("{base}{path}")).unwrap(),
                    |u| Ok(client
                        .get(u.clone())
                        .header("Authorization", "synthetic")
                        .header("Cookie", "synthetic=private")),
                    Redirects::SameOrigin,
                    "Fixture"
                )
                .await
                .is_err()
            );
        }
        assert!(!contacted.load(std::sync::atomic::Ordering::SeqCst));
        task.abort();
        foreign_task.abort();
    }
}

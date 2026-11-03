//! Check a small number of native candidates before handing their URLs to mpv.
use crate::{
    native::{Resolved, playback_source},
    process,
};
use anyhow::{Result, bail, ensure};
use yt_dlp_rs::{VideoInfo, YoutubeDl};

fn choose(
    client: &YoutubeDl,
    mut info: VideoInfo,
    selector: &str,
    audio: bool,
    explicit: bool,
    mut probe: impl FnMut(&str, &[(String, String)]) -> Result<u16>,
) -> Result<Resolved> {
    let mut rejected_status = None;
    for _ in 0..3 {
        let mut selected = info.clone();
        if let Err(error) = client.select_video(&mut selected, Some(selector)) {
            if let Some(status) = rejected_status {
                bail!(
                    "Native stream HTTP error {status}: no playable alternative matched the requested format"
                );
            }
            return Err(error.into());
        }
        let stream = playback_source(&selected, audio)?;
        let mut rejected_url = None;
        for url in std::iter::once(&stream.url).chain(stream.audio_url.iter()) {
            let status = probe(url, &stream.headers)?;
            if (200..300).contains(&status) {
                continue;
            }
            ensure!(
                status != 429,
                "Native stream HTTP error 429: Too Many Requests; wait for the service cooldown"
            );
            ensure!(
                matches!(status, 403 | 404 | 410),
                "Native stream HTTP error {status}"
            );
            rejected_url = Some(url.clone());
            rejected_status = Some(status);
            break;
        }
        let Some(rejected_url) = rejected_url else {
            return Ok(stream);
        };
        ensure!(
            !explicit,
            "Native stream HTTP error {} for the selected format; refresh it or choose another format",
            rejected_status.unwrap_or(403)
        );
        let before = info.formats.len();
        info.formats
            .retain(|format| format.url.as_str() != rejected_url);
        ensure!(
            info.formats.len() < before,
            "Native stream HTTP error {}: stream rejected",
            rejected_status.unwrap_or(403)
        );
        ensure!(
            !info.formats.is_empty(),
            "Native stream HTTP error {}: no playable formats remain",
            rejected_status.unwrap_or(403)
        );
    }
    bail!(
        "Native stream HTTP error {}: three candidate formats were rejected",
        rejected_status.unwrap_or(403)
    )
}

pub(super) fn resolve(
    client: &YoutubeDl,
    info: VideoInfo,
    selector: &str,
    audio: bool,
    explicit: bool,
    proxy: Option<&str>,
    verify: bool,
) -> Result<Resolved> {
    if !verify {
        let mut selected = info;
        client.select_video(&mut selected, Some(selector))?;
        return playback_source(&selected, audio);
    }
    let http = crate::blocking_http_client(proxy)?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
    choose(client, info, selector, audio, explicit, |url, headers| {
        let remaining = deadline
            .checked_duration_since(std::time::Instant::now())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(|| anyhow::anyhow!("Native stream probe timed out"))?;
        let mut request = http
            .get(url)
            .header(reqwest::header::RANGE, "bytes=0-0")
            .timeout(remaining.min(std::time::Duration::from_secs(5)));
        for (name, value) in headers {
            request = request.header(name, value);
        }
        let response = request.send().map_err(|error| {
            anyhow::anyhow!(
                "Native stream probe failed: {}",
                process::diagnostic(&error.without_url().to_string())
            )
        })?;
        // Dropping after headers bounds the transfer even if Range is ignored.
        if response.status().is_success()
            && response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.starts_with("text/html"))
        {
            bail!(
                "Native stream returned a browser verification page; check the current account/network"
            );
        }
        Ok(response.status().as_u16())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn fixture() -> (YoutubeDl, VideoInfo) {
        let client = YoutubeDl::with_options(Default::default(), Default::default())
            .unwrap()
            .with_check_formats(yt_dlp_rs::CheckFormats::Never);
        let info=serde_json::from_value(json!({"id":"abcdefghijk","title":"Fixture","ext":"m4a","url":"https://cdn.invalid/base","webpage_url":"https://www.youtube.com/watch?v=abcdefghijk","extractor":"youtube","extractor_key":"Youtube","formats":[
            {"format_id":"good","url":"https://cdn.invalid/good","ext":"m4a","vcodec":"none","acodec":"aac","abr":128,"stream_kind":"audio_only"},
            {"format_id":"bad","url":"https://cdn.invalid/bad","ext":"m4a","vcodec":"none","acodec":"aac","abr":192,"stream_kind":"audio_only"}
        ]})).unwrap();
        (client, info)
    }
    #[test]
    fn a_rejected_preferred_native_format_uses_a_valid_alternative_without_reextracting() {
        let (client, info) = fixture();
        let mut calls = Vec::new();
        let stream = choose(&client, info, "bestaudio/best", true, false, |url, _| {
            calls.push(url.to_owned());
            Ok(if url.ends_with("/bad") { 403 } else { 206 })
        })
        .unwrap();
        assert!(stream.url.ends_with("/good"));
        assert_eq!(calls.len(), 2);
    }
    #[test]
    fn rate_limits_stop_after_one_probe_and_an_explicit_format_is_never_replaced() {
        for (status, explicit, selector) in [(429, false, "bestaudio/best"), (403, true, "bad")] {
            let (client, info) = fixture();
            let mut calls = 0;
            let error = choose(&client, info, selector, true, explicit, |_, _| {
                calls += 1;
                Ok(status)
            })
            .err()
            .unwrap();
            assert_eq!(calls, 1);
            assert!(error.to_string().contains(&status.to_string()));
        }
    }
    #[test]
    fn candidate_budget_and_transport_errors_do_not_loop() {
        let (client, mut info) = fixture();
        for index in 0..5 {
            let mut format = info.formats[0].clone();
            format.format_id = format!("more{index}");
            format.url = url::Url::parse(&format!("https://cdn.invalid/more{index}")).unwrap();
            info.formats.push(format);
        }
        let mut calls = 0;
        assert!(
            choose(
                &client,
                info.clone(),
                "bestaudio/best",
                true,
                false,
                |_, _| {
                    calls += 1;
                    Ok(403)
                }
            )
            .is_err()
        );
        assert_eq!(calls, 3);
        calls = 0;
        assert!(
            choose(&client, info, "bestaudio/best", true, false, |_, _| {
                calls += 1;
                anyhow::bail!("connection reset")
            })
            .is_err()
        );
        assert_eq!(calls, 1);
    }
    #[tokio::test]
    async fn actual_probe_uses_a_byte_range_and_the_scoped_native_identity() {
        let (url, server) = crate::test_http::serve_request(|request| {
            let headers = request.to_ascii_lowercase();
            assert!(headers.contains("range: bytes=0-0"));
            assert!(headers.contains("user-agent: native-probe-fixture"));
            assert!(!headers.contains("authorization:"));
            assert!(!headers.contains("cookie:"));
            (206, "Content-Type: audio/mp4\r\n".into(), vec![0])
        })
        .await;
        let result = tokio::task::spawn_blocking(move || {
            let (client, mut info) = fixture();
            info.formats.truncate(1);
            info.formats[0].url = url::Url::parse(&url).unwrap();
            let mut value = serde_json::to_value(info).unwrap();
            value["http_headers"] = json!({"User-Agent":"native-probe-fixture", "Authorization":"synthetic-must-not-leak", "Cookie":"synthetic-must-not-leak"});
            // extract_result_unselected normalizes entry headers onto formats.
            value["formats"][0]["http_headers"] = value["http_headers"].clone();
            let info = serde_json::from_value(value).unwrap();
            resolve(&client, info, "bestaudio/best", true, false, None, true)
        }).await.unwrap().unwrap();
        server.abort();
        let _ = server.await;
        assert!(
            result
                .headers
                .iter()
                .any(|(name, value)| name.eq_ignore_ascii_case("user-agent")
                    && value == "native-probe-fixture")
        );
    }
}

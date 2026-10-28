//! Explicit lyric lookup: only reviewed title/artist are sent, with a fresh client.
//! No session, cookies, stream URL or automatic listening-history request.
use anyhow::{Result, ensure};
use serde::Deserialize;
use std::path::Path;
use tokio::io::AsyncReadExt;

const MAX_TEXT: usize = 512 * 1024;
const MAX_LINES: usize = 10_000;
const DAY_MS: u64 = 86_400_000;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, Deserialize)]
pub struct Line {
    pub at_ms: Option<u64>,
    pub text: String,
}
#[derive(Clone, Debug, serde::Serialize, Deserialize)]
pub struct Track {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: f64,
    pub instrumental: bool,
    pub lines: Vec<Line>,
}
impl Track {
    pub fn active_line(&self, seconds: f64) -> Option<usize> {
        if !seconds.is_finite() || seconds < 0.0 {
            return None;
        }
        let now = (seconds * 1000.0) as u64;
        self.lines
            .iter()
            .rposition(|line| line.at_ms.is_some_and(|at| at <= now))
    }
    pub fn synced(&self) -> bool {
        self.lines.iter().any(|line| line.at_ms.is_some())
    }
}

fn timestamp(text: &str) -> Option<u64> {
    let (minutes, seconds) = text.split_once(':')?;
    let minutes: u64 = minutes.parse().ok()?;
    let seconds: f64 = seconds.parse().ok()?;
    if !seconds.is_finite() || !(0.0..60.0).contains(&seconds) || minutes > 1440 {
        return None;
    }
    let at = minutes
        .checked_mul(60_000)?
        .checked_add((seconds * 1000.0).round() as u64)?;
    (at <= DAY_MS).then_some(at)
}
pub fn parse(text: &str) -> Result<Vec<Line>> {
    ensure!(text.len() <= MAX_TEXT, "Lyrics exceed the 512 KiB limit");
    let mut offset = 0i64;
    let mut synced = Vec::new();
    let mut plain = Vec::new();
    for raw in text.trim_start_matches('\u{feff}').lines() {
        let mut rest = raw.trim();
        if let Some(value) = rest
            .strip_prefix("[offset:")
            .and_then(|v| v.strip_suffix(']'))
        {
            offset = value.trim().parse()?;
            ensure!(offset.unsigned_abs() <= DAY_MS, "Invalid LRC offset");
            continue;
        }
        let mut times = Vec::new();
        while let Some((tag, tail)) = rest.strip_prefix('[').and_then(|s| s.split_once(']')) {
            let Some(at) = timestamp(tag) else { break };
            times.push(at);
            rest = tail;
        }
        if times.is_empty() {
            if !["[ar:", "[ti:", "[al:", "[by:", "[re:", "[ve:", "[length:"]
                .iter()
                .any(|prefix| rest.starts_with(prefix))
            {
                plain.push(Line {
                    at_ms: None,
                    text: raw.to_owned(),
                });
            }
        } else {
            for at in times {
                synced.push(Line {
                    at_ms: Some(at),
                    text: rest.trim().to_owned(),
                });
            }
        }
        ensure!(
            synced.len() + plain.len() <= MAX_LINES,
            "Too many lyric lines"
        );
    }
    if synced.is_empty() {
        return Ok(plain);
    }
    for line in &mut synced {
        line.at_ms = line
            .at_ms
            .map(|at| at.saturating_add_signed(offset).min(DAY_MS));
    }
    synced.sort_by_key(|line| line.at_ms);
    Ok(synced)
}
pub async fn local(path: &Path) -> Result<Track> {
    let mut options = tokio::fs::OpenOptions::new();
    options.read(true);
    // Reject pipes/devices after opening without blocking a runtime worker on
    // a named pipe that has no writer. Regular files ignore O_NONBLOCK.
    #[cfg(unix)]
    options.custom_flags(libc::O_NONBLOCK);
    let file = options.open(path).await?;
    let metadata = file.metadata().await?;
    ensure!(
        metadata.is_file() && metadata.len() <= MAX_TEXT as u64,
        "Choose a regular lyrics file up to 512 KiB"
    );
    let mut bytes = Vec::new();
    file.take(MAX_TEXT as u64 + 1)
        .read_to_end(&mut bytes)
        .await?;
    ensure!(bytes.len() <= MAX_TEXT, "Lyrics exceed the 512 KiB limit");
    let lines = parse(std::str::from_utf8(&bytes)?)?;
    ensure!(
        lines.iter().any(|line| !line.text.trim().is_empty()),
        "Lyrics file is empty"
    );
    Ok(Track {
        title: "Local lyrics".into(),
        artist: String::new(),
        album: String::new(),
        duration: 0.0,
        instrumental: false,
        lines,
    })
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Response {
    track_name: String,
    artist_name: String,
    #[serde(default)]
    album_name: String,
    #[serde(default)]
    duration: f64,
    #[serde(default)]
    instrumental: bool,
    synced_lyrics: Option<String>,
    plain_lyrics: Option<String>,
}
async fn fetch(
    client: reqwest::Client,
    endpoint: &str,
    title: &str,
    artist: &str,
) -> Result<Vec<Track>> {
    ensure!(
        !title.trim().is_empty() && !artist.trim().is_empty(),
        "Review both song title and artist before lookup"
    );
    ensure!(
        title.len() <= 512 && artist.len() <= 512,
        "Song title and artist must be at most 512 bytes"
    );
    let response = client
        .get(endpoint)
        .header(
            reqwest::header::USER_AGENT,
            concat!("ytui/", env!("CARGO_PKG_VERSION")),
        )
        .query(&[("track_name", title.trim()), ("artist_name", artist.trim())])
        .timeout(std::time::Duration::from_secs(8))
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Lyrics lookup unavailable: {}", e.without_url()))?;
    ensure!(
        response.status().is_success(),
        "LRCLIB HTTP {}",
        response.status().as_u16()
    );
    let text = crate::browse::limited_text(response, 2 * 1024 * 1024).await?;
    let rows: Vec<Response> = serde_json::from_str(&text)?;
    ensure!(rows.len() <= 100, "Too many lyric candidates");
    let mut tracks = Vec::new();
    for row in rows.into_iter().take(25) {
        let text = row
            .synced_lyrics
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .or(row.plain_lyrics.as_deref())
            .unwrap_or("");
        let lines = parse(text)?;
        if row.instrumental || lines.iter().any(|line| !line.text.trim().is_empty()) {
            tracks.push(Track {
                title: row.track_name,
                artist: row.artist_name,
                album: row.album_name,
                duration: row.duration,
                instrumental: row.instrumental,
                lines,
            });
        }
    }
    Ok(tracks)
}
pub async fn lookup(proxy: Option<&str>, title: &str, artist: &str) -> Result<Vec<Track>> {
    fetch(
        crate::http_client(proxy)?,
        "https://lrclib.net/api/search",
        title,
        artist,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lrc_unicode_repeats_global_offsets_and_bounds() {
        let lines =
            parse("[ar:Fixture]\n[00:01.20][00:03.250] Grüße 🎵\n[00:00.50]Start\n[offset:-1000]")
                .unwrap();
        assert_eq!(
            lines.iter().map(|l| l.at_ms.unwrap()).collect::<Vec<_>>(),
            [0, 200, 2250]
        );
        assert_eq!(lines[1].text, "Grüße 🎵");
        assert!(parse(&"x".repeat(MAX_TEXT + 1)).is_err());
        assert!(parse("[offset:999999999999999]").is_err());
        assert!(parse(&"[00:00.01]a\n".repeat(MAX_LINES + 1)).is_err());
        assert_eq!(parse("first\nsecond").unwrap()[0].at_ms, None);
    }
    #[tokio::test]
    async fn lookup_sends_only_reviewed_metadata_and_never_cookies() {
        let (endpoint, server) = crate::test_http::serve_request(|request| {
            let request = request.to_ascii_lowercase();
            assert!(request.starts_with("get /?track_name="));
            assert!(request.contains("artist_name="));
            assert!(!request.contains("cookie:") && !request.contains("authorization:"));
            assert!(!request.contains("youtube") && !request.contains("album_name="));
            (200, String::new(), br#"[{"trackName":"Song","artistName":"Artist","syncedLyrics":"[00:01.00]Hello","plainLyrics":null}]"#.to_vec())
        }).await;
        let tracks = fetch(
            reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
            &endpoint,
            "Song & title",
            "Artist",
        )
        .await
        .unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].active_line(0.5), None);
        assert_eq!(tracks[0].active_line(1.1), Some(0));
        assert_eq!(tracks[0].active_line(f64::NAN), None);
        server.abort();
    }
    #[tokio::test]
    async fn local_text_and_invalid_files() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("song.lrc");
        tokio::fs::write(&path, "[00:01]Hello").await.unwrap();
        assert!(local(&path).await.unwrap().synced());
        assert!(local(temp.path()).await.is_err());
        tokio::fs::write(&path, [255u8]).await.unwrap();
        assert!(local(&path).await.is_err());
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn local_pipe_is_rejected_without_waiting_for_a_writer() {
        let temp = tempfile::tempdir().unwrap();
        let pipe = temp.path().join("lyrics.pipe");
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&pipe)
                .status()
                .unwrap()
                .success()
        );
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(1), local(&pipe))
                .await
                .unwrap()
                .is_err()
        );
    }
    #[tokio::test]
    async fn lookup_rejects_redirects_http_errors_and_oversized_responses() {
        for status in [302, 503, 200] {
            let (endpoint, server) = crate::test_http::serve_request(move |_| {
                (
                    status,
                    "Location: http://lyrics-redirect-fixture.invalid/secret\r\n".into(),
                    if status == 200 {
                        vec![b'x'; 2 * 1024 * 1024 + 1]
                    } else {
                        Vec::new()
                    },
                )
            })
            .await;
            let client = reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap();
            let error = fetch(client, &endpoint, "Title", "Artist")
                .await
                .unwrap_err()
                .to_string();
            assert!(!error.contains("secret") && !error.contains("127.0.0.1"));
            if status != 200 {
                assert!(error.contains(&status.to_string()));
            }
            server.abort();
        }
    }
}

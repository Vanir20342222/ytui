//! Read-only, non-destructive playback marks. Requests contain a public video ID,
//! never browser cookies or a signed media URL.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use ytui_core::{Config, Media};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Segment {
    pub start: f64,
    pub end: f64,
    pub category: String,
}
pub fn video_id(media: &Media) -> Option<String> {
    let url = ytui_core::http_url(&media.url).ok()?;
    if !ytui_core::youtube_host(url.host_str()?) {
        return None;
    }
    let id = if url.host_str() == Some("youtu.be") {
        url.path_segments()?.next()?.to_owned()
    } else {
        url.query_pairs()
            .find(|(k, _)| k == "v")
            .map(|(_, v)| v.into_owned())
            .or_else(|| url.path().strip_prefix("/shorts/").map(str::to_owned))?
    };
    (id.len() == 11
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)))
    .then_some(id)
}
pub fn parse(data: &serde_json::Value, categories: &[String]) -> Result<Vec<Segment>> {
    let rows = data
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Invalid SponsorBlock response"))?;
    ensure!(rows.len() <= 1000, "Too many SponsorBlock segments");
    let mut segments = rows
        .iter()
        .filter_map(|row| {
            let category = row.get("category")?.as_str()?;
            if !categories.iter().any(|c| c == category)
                || row
                    .get("actionType")
                    .and_then(|v| v.as_str())
                    .is_some_and(|t| t != "skip")
            {
                return None;
            }
            let start = row.pointer("/segment/0")?.as_f64()?;
            let end = row.pointer("/segment/1")?.as_f64()?;
            (start.is_finite() && end.is_finite() && start >= 0.0 && end > start).then(|| Segment {
                start,
                end,
                category: category.into(),
            })
        })
        .collect::<Vec<_>>();
    segments.sort_by(|a, b| a.start.total_cmp(&b.start));
    Ok(segments)
}
pub async fn fetch(config: &Config, media: &Media) -> Result<Vec<Segment>> {
    let Some(id) = video_id(media) else {
        return Ok(Vec::new());
    };
    let mut url = ytui_core::http_url(&config.download.sponsorblock_api)?;
    url.set_path(&format!(
        "{}/api/skipSegments",
        url.path().trim_end_matches('/')
    ));
    url.query_pairs_mut()
        .append_pair("videoID", &id)
        .append_pair(
            "categories",
            &serde_json::to_string(&config.playback_sponsor_categories)?,
        )
        .append_pair("actionTypes", "[\"skip\"]");
    let response = crate::http_client(config.proxy.as_deref())?
        .get(url)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("SponsorBlock unavailable: {}", e.without_url()))?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(Vec::new());
    }
    ensure!(
        response.status().is_success(),
        "SponsorBlock HTTP {}",
        response.status().as_u16()
    );
    let data = serde_json::from_str(&crate::browse::limited_text(response, 256 * 1024).await?)?;
    parse(&data, &config.playback_sponsor_categories)
}
/// Merge overlapping skip ranges so short adjacent marks cannot seek backwards.
pub fn skip_target(segments: &[Segment], position: f64, ignore_until: f64) -> Option<f64> {
    if position < ignore_until {
        return None;
    }
    let mut target = segments
        .iter()
        .filter(|s| position >= s.start && position < s.end)
        .map(|s| s.end)
        .max_by(f64::total_cmp)?;
    loop {
        let next = segments
            .iter()
            .filter(|s| s.start <= target && s.end > target)
            .map(|s| s.end)
            .max_by(f64::total_cmp);
        if let Some(next) = next {
            target = next;
        } else {
            break;
        }
    }
    Some(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[tokio::test]
    async fn public_api_marks_skips_and_replay_preserve_the_video() {
        let (base, task) = crate::test_http::serve_request(|request| {
            let headers = request.split("\r\n\r\n").next().unwrap().to_lowercase();
            assert!(!headers.contains("\ncookie:"));
            assert!(!headers.contains("\nauthorization:"));
            assert!(headers.contains("videoid=baw_jenozkc"));
            assert!(headers.contains("categories=%5b%22sponsor%22%5d"));
            (
                200,
                String::new(),
                json!([
                    {"category":"sponsor","actionType":"skip","segment":[2.0,5.0]},
                    {"category":"sponsor","segment":[4.0,8.0]},
                    {"category":"intro","segment":[8.0,12.0]},
                    {"category":"sponsor","segment":[-1.0,3.0]},
                    {"category":"sponsor","actionType":"mute","segment":[10.0,15.0]}
                ])
                .to_string()
                .into_bytes(),
            )
        })
        .await;
        let config = Config {
            download: ytui_core::DownloadOptions {
                sponsorblock_api: base,
                ..Default::default()
            },
            ..Default::default()
        };
        let media = Media {
            url: "https://www.youtube.com/watch?v=BaW_jenozKc".into(),
            ..Default::default()
        };
        let segments = fetch(&config, &media).await.unwrap();
        assert_eq!(segments.len(), 2);
        assert_eq!(skip_target(&segments, 3.0, 0.0), Some(8.0));
        assert_eq!(
            skip_target(&segments, 3.0, 8.0),
            None,
            "undo must allow watching the whole overlapping range"
        );
        assert_eq!(skip_target(&segments, 8.0, 8.0), None);
        assert!(
            video_id(&Media {
                url: "https://radio.test/stream".into(),
                ..Default::default()
            })
            .is_none()
        );
        task.abort();
        let _ = task.await;
    }
}

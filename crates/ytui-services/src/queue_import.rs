//! Import URLs and download intent, never executable settings from an untrusted export.
use anyhow::{Result, ensure};
use serde_json::Value;
use std::{io::Read, path::Path};
use ytui_core::{Config, DownloadQueue, Job, Media, clean_text, normalize_media_url};
pub fn read(path: &Path, config: &Config) -> Result<Vec<Job>> {
    let mut raw = String::new();
    std::fs::File::open(path)?
        .take(5 * 1024 * 1024 + 1)
        .read_to_string(&mut raw)?;
    ensure!(raw.len() <= 5 * 1024 * 1024, "Queue import exceeds 5 MiB");
    let rows = if let Ok(data) = serde_json::from_str::<Value>(&raw) {
        data.as_array()
            .or_else(|| data.get("items").and_then(Value::as_array))
            .or_else(|| data.pointer("/downloads/jobs").and_then(Value::as_array))
            .ok_or_else(|| {
                anyhow::anyhow!("Expected a Rust snapshot, legacy items export, or URL array")
            })?
            .clone()
    } else {
        ensure!(
            !raw.trim_start().starts_with(['{', '[']),
            "Invalid JSON queue export"
        );
        raw.lines()
            .map(str::trim)
            .filter(|s| !s.is_empty() && !s.starts_with('#'))
            .map(|s| Value::String(s.into()))
            .collect()
    };
    ensure!(rows.len() <= 10000, "Import at most 10,000 entries at once");
    let mut jobs = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        let url = row
            .as_str()
            .or_else(|| row.get("url").and_then(Value::as_str))
            .or_else(|| row.pointer("/media/url").and_then(Value::as_str))
            .ok_or_else(|| anyhow::anyhow!("Import entry {} has no URL", index + 1))?;
        let url = normalize_media_url(url).map_err(|_| {
            anyhow::anyhow!("Import entry {} has an invalid HTTP(S) URL", index + 1)
        })?;
        let mut options = config.download.clone();
        if let Some(mode) = row.get("download_mode").and_then(Value::as_str) {
            options.audio = mode == "audio";
        }
        if let Some(audio) = row.pointer("/options/audio").and_then(Value::as_bool) {
            options.audio = audio;
        }
        if let Some(quality) = row
            .get("quality_override")
            .or_else(|| row.pointer("/options/quality"))
            .and_then(Value::as_str)
            .filter(|q| !q.is_empty())
        {
            options.quality = quality.into();
        }
        options.playlist = row
            .pointer("/options/playlist")
            .and_then(Value::as_bool)
            .unwrap_or(options.playlist)
            || ytui_core::is_playlist_url(&url);
        options.playlist_items = row
            .pointer("/options/playlist_items")
            .and_then(Value::as_str)
            .map(str::to_owned);
        options.playlist |= options.playlist_items.is_some();
        options.validate()?;
        let title = row
            .pointer("/media/title")
            .or_else(|| row.pointer("/info/title"))
            .and_then(Value::as_str)
            .unwrap_or("Imported media");
        jobs.push(Job::new(
            Media {
                url,
                title: clean_text(title),
                ..Default::default()
            },
            options,
        ));
    }
    Ok(jobs)
}
pub fn append(queue: &mut DownloadQueue, jobs: Vec<Job>) -> usize {
    let before = queue.jobs.len();
    for job in jobs {
        if !queue.jobs.iter().any(|j| {
            j.media.url == job.media.url
                && j.options.audio == job.options.audio
                && j.options.playlist == job.options.playlist
                && j.options.playlist_items == job.options.playlist_items
                && j.options.quality == job.options.quality
        }) {
            queue.jobs.push(job);
        }
    }
    queue.jobs.len() - before
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn import_is_atomic_repeat_safe_and_does_not_load_exported_hooks() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("queue.json");
        std::fs::write(&path, r#"{"downloads":{"jobs":[{"media":{"url":"https://youtu.be/abcdefghijk","title":"Fixture"},"options":{"audio":true,"post_download_hook":"/untrusted"}}]}}"#).unwrap();
        let mut queue = DownloadQueue::default();
        let jobs = read(&path, &Config::default()).unwrap();
        assert!(jobs[0].options.post_download_hook.is_none());
        assert!(jobs[0].options.audio);
        assert_eq!(append(&mut queue, jobs), 1);
        assert_eq!(
            append(&mut queue, read(&path, &Config::default()).unwrap()),
            0
        );
        std::fs::write(&path, "https://example.test/video\nfile:///secret\n").unwrap();
        assert!(read(&path, &Config::default()).is_err());
        assert_eq!(queue.jobs.len(), 1);
    }
    #[test]
    fn playlist_intent_and_indices_survive_import_and_differ_from_a_single_video() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("playlist.json");
        std::fs::write(&path,r#"[{"media":{"url":"https://www.youtube.com/watch?v=synthetic00&list=PLsynthetic"},"options":{"audio":true,"playlist":true,"playlist_items":"2,3","post_download_hook":"/untrusted"}}]"#).unwrap();
        let jobs = read(&path, &Config::default()).unwrap();
        assert!(jobs[0].options.playlist);
        assert_eq!(jobs[0].options.playlist_items.as_deref(), Some("2,3"));
        assert!(jobs[0].options.post_download_hook.is_none());
        let mut queue = DownloadQueue::default();
        let mut single = jobs[0].clone();
        single.options.playlist = false;
        single.options.playlist_items = None;
        assert_eq!(append(&mut queue, vec![single]), 1);
        assert_eq!(append(&mut queue, jobs), 1);
        assert_eq!(
            append(&mut queue, read(&path, &Config::default()).unwrap()),
            0
        );
    }
}

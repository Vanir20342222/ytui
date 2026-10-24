//! Import URLs and download intent, never executable settings from an untrusted export.
use anyhow::{Result, ensure};
use serde_json::Value;
use std::{io::Read, path::Path};
use ytui_core::{Config, DownloadQueue, Job, Media, clean_text, normalize_media_url};
pub fn read(path: &Path, config: &Config) -> Result<Vec<Job>> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    // A reviewed path can still name a FIFO or device. Opening nonblocking
    // allows us to reject it without stranding an import worker indefinitely.
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    ensure!(metadata.is_file(), "Choose a regular queue import file");
    ensure!(
        metadata.len() <= 5 * 1024 * 1024,
        "Queue import exceeds 5 MiB"
    );
    let mut raw = String::new();
    file.take(5 * 1024 * 1024 + 1).read_to_string(&mut raw)?;
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
                && j.options.same_output(&job.options)
                && !matches!(
                    j.state,
                    ytui_core::JobState::Error | ytui_core::JobState::Cancelled
                )
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

#[cfg(test)]
mod integration_review_tests {
    use super::*;
    use ytui_core::{DownloadOptions, JobState};
    fn job(options: DownloadOptions) -> Job {
        Job::new(
            Media {
                url: "https://www.youtube.com/watch?v=abcdefghijk".into(),
                ..Default::default()
            },
            options,
        )
    }
    #[test]
    fn imported_requests_keep_distinct_formats_bitrates_and_destinations() {
        let base = DownloadOptions {
            audio: true,
            ..Default::default()
        };
        let mut queue = DownloadQueue {
            jobs: vec![job(base.clone())],
        };
        let mut flac = base.clone();
        flac.audio_container = "flac".into();
        let mut bitrate = base.clone();
        bitrate.audio_quality = "320K".into();
        let mut folder = base.clone();
        folder.audio_directory = "other-output".into();
        for options in [flac, bitrate, folder] {
            assert_eq!(append(&mut queue, vec![job(options.clone())]), 1);
            assert_eq!(append(&mut queue, vec![job(options)]), 0);
        }
    }
    #[test]
    fn imported_video_containers_and_postprocessing_are_distinct() {
        let base = DownloadOptions::default();
        let mut queue = DownloadQueue {
            jobs: vec![job(base.clone())],
        };
        let mut mkv = base.clone();
        mkv.container = "mkv".into();
        let mut captions = base.clone();
        captions.subtitles = vec!["en".into()];
        let mut artwork = base;
        artwork.thumbnail = true;
        assert_eq!(
            append(&mut queue, vec![job(mkv), job(captions), job(artwork)]),
            3
        );
    }
    #[test]
    fn import_retries_failed_or_cancelled_requests() {
        for state in [JobState::Error, JobState::Cancelled] {
            let mut existing = job(DownloadOptions::default());
            existing.state = state;
            let mut queue = DownloadQueue {
                jobs: vec![existing],
            };
            assert_eq!(append(&mut queue, vec![job(DownloadOptions::default())]), 1);
            assert_eq!(append(&mut queue, vec![job(DownloadOptions::default())]), 0);
        }
    }
    #[test]
    fn transport_only_changes_do_not_duplicate_imported_outputs() {
        let base = DownloadOptions::default();
        let mut changed = base.clone();
        changed.backend = Some("native".into());
        changed.bandwidth_kib = 32;
        let mut queue = DownloadQueue {
            jobs: vec![job(base)],
        };
        assert_eq!(append(&mut queue, vec![job(changed)]), 0);
    }
    #[test]
    fn imports_use_reviewed_local_destinations_and_hooks() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("export.json");
        std::fs::write(&path, r#"[{"url":"https://youtu.be/abcdefghijk","options":{"directory":"/unreviewed","post_download_hook":"/untrusted","backend":"arbitrary-tool","sponsorblock_api":"http://untrusted.test"}}]"#).unwrap();
        let mut config = Config::default();
        config.download.directory = temp.path().join("reviewed");
        let jobs = read(&path, &config).unwrap();
        assert_eq!(jobs[0].options.directory, config.download.directory);
        assert_eq!(jobs[0].options.backend, config.download.backend);
        assert_eq!(
            jobs[0].options.sponsorblock_api,
            config.download.sponsorblock_api
        );
        assert!(jobs[0].options.post_download_hook.is_none());
    }
    #[test]
    fn oversized_or_non_utf8_imports_fail_without_partial_jobs() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("export");
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(5 * 1024 * 1024 + 1).unwrap();
        assert!(read(&path, &Config::default()).is_err());
        std::fs::write(&path, [0xff, 0xfe]).unwrap();
        assert!(read(&path, &Config::default()).is_err());
    }
    #[test]
    fn malformed_json_and_late_invalid_entries_do_not_import_a_prefix() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("export.json");
        for raw in [
            r#"["https://youtu.be/abcdefghijk","file:///invalid"]"#,
            r#"{"items":["https://youtu.be/abcdefghijk"]"#,
        ] {
            std::fs::write(&path, raw).unwrap();
            assert!(read(&path, &Config::default()).is_err());
        }
    }
    #[test]
    #[cfg(unix)]
    fn named_pipes_are_rejected_without_blocking_the_import_worker() {
        use std::os::unix::ffi::OsStrExt;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("pipe");
        let raw = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(raw.as_ptr(), 0o600) }, 0);
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(read(&path, &Config::default()));
        });
        let result = rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("Queue import blocked on a named pipe");
        assert!(result.unwrap_err().to_string().contains("regular"));
    }
}

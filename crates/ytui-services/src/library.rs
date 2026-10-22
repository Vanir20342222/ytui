//! A local catalog of recorded completed outputs. No network and no recursive scan.
use ytui_core::{DownloadQueue, JobState, MediaKind, Page};
pub fn catalog(queue: &DownloadQueue, query: &str, audio: Option<bool>) -> Page {
    let query = query.to_lowercase();
    let mut seen = std::collections::HashSet::new();
    let mut items = Vec::new();
    for job in queue
        .jobs
        .iter()
        .rev()
        .filter(|j| j.state == JobState::Done && audio.is_none_or(|a| a == j.options.audio))
    {
        for output in &job.outputs {
            let extension = output
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_lowercase();
            if ![
                "mp4", "mkv", "webm", "mp3", "m4a", "flac", "ogg", "opus", "wav", "aac", "mov",
                "m4v", "avi", "flv", "ts",
            ]
            .contains(&extension.as_str())
            {
                continue;
            }
            let Ok(path) = std::fs::canonicalize(output) else {
                continue;
            };
            if !path.is_file() || !seen.insert(path.clone()) {
                continue;
            }
            let mut media = job.media.clone();
            if job.outputs.len() > 1 {
                media.title = ytui_core::clean_text(
                    output
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or(&media.title),
                );
            }
            if !query.is_empty()
                && !media.title.to_lowercase().contains(&query)
                && !media.creator.to_lowercase().contains(&query)
            {
                continue;
            }
            media.id = format!("local:{}:{}", job.id, items.len());
            media.url = path.to_string_lossy().into();
            media.thumbnail.clear();
            media.browse_id = None;
            media.tracks_url = None;
            media.live = false;
            media.kind = if job.options.audio {
                MediaKind::Song
            } else {
                MediaKind::Video
            };
            items.push(media);
        }
    }
    Page { title: "Offline downloads".into(), items, notice: Some("Recorded completed media · missing files and subtitle/image sidecars are hidden · no network scan".into()), ..Default::default() }
}
#[cfg(test)]
mod tests {
    use super::*;
    use ytui_core::{DownloadOptions, Job, Media};
    #[test]
    fn only_existing_completed_media_are_listed_without_remote_thumbnails() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("音楽.mp3");
        std::fs::write(&file, b"fixture").unwrap();
        let mut job = Job::new(
            Media {
                title: "音楽".into(),
                thumbnail: "https://example.test/cover.jpg".into(),
                ..Default::default()
            },
            DownloadOptions {
                audio: true,
                ..Default::default()
            },
        );
        job.state = JobState::Done;
        job.outputs = vec![
            file,
            temp.path().join("missing.mp3"),
            temp.path().join("captions.vtt"),
        ];
        let queue = DownloadQueue { jobs: vec![job] };
        let page = catalog(&queue, "音楽", Some(true));
        assert_eq!(page.items.len(), 1);
        assert!(page.items[0].thumbnail.is_empty());
        assert_eq!(page.items[0].kind, MediaKind::Song);
        assert!(catalog(&queue, "", Some(false)).items.is_empty());
    }
}

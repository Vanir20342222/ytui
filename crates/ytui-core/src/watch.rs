use crate::{BrowseRequest, DownloadOptions, Media};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CollectionWatch {
    pub id: uuid::Uuid,
    pub title: String,
    pub request: BrowseRequest,
    pub music: bool,
    pub options: DownloadOptions,
    pub interval_seconds: u64,
    pub next_check: u64,
    pub enabled: bool,
    pub generation: u64,
    pub owner: String,
    pub seen: Option<Vec<String>>,
    pub failures: u32,
    pub error: Option<String>,
}
impl CollectionWatch {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            matches!(&self.request, BrowseRequest::Collection(id) if !id.is_empty()),
            "Watches require a collection/channel/playlist"
        );
        ensure!(
            (60..=30 * 86400).contains(&self.interval_seconds),
            "Watch interval must be between 1 minute and 30 days"
        );
        self.options.validate()
    }
    pub fn observe(&mut self, items: Vec<Media>, now: u64) -> Vec<Media> {
        let baseline = self.seen.is_none();
        let mut seen = self.seen.take().unwrap_or_default();
        let mut added = Vec::new();
        for media in items {
            if media.collection() || media.live || crate::http_url(&media.url).is_err() {
                continue;
            }
            let key = if media.id.is_empty() {
                media.url.clone()
            } else {
                media.id.clone()
            };
            if !seen.contains(&key) {
                seen.push(key);
                if !baseline {
                    added.push(media);
                }
            }
        }
        if seen.len() > 5000 {
            seen.drain(..seen.len() - 5000);
        }
        self.seen = Some(seen);
        self.failures = 0;
        self.error = None;
        self.next_check = now.saturating_add(self.interval_seconds);
        added
    }
    pub fn failed(&mut self, message: String, now: u64) {
        self.failures = self.failures.saturating_add(1);
        self.error = Some(message);
        self.next_check = now.saturating_add(
            self.interval_seconds
                .saturating_mul(1u64 << self.failures.min(6))
                .min(86400.max(self.interval_seconds)),
        );
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn initial_baseline_never_downloads_existing_items_and_failed_polls_back_off() {
        let mut watch = CollectionWatch {
            id: uuid::Uuid::new_v4(),
            title: "Fixture".into(),
            request: BrowseRequest::Collection("VLfixture".into()),
            music: false,
            options: DownloadOptions::default(),
            interval_seconds: 60,
            next_check: 0,
            enabled: true,
            generation: 0,
            owner: "anonymous".into(),
            seen: None,
            failures: 0,
            error: None,
        };
        let media = |id: &str| Media {
            id: id.into(),
            url: format!("https://www.youtube.com/watch?v={id}"),
            ..Default::default()
        };
        watch.validate().unwrap();
        assert!(watch.observe(vec![media("old")], 100).is_empty());
        let new = watch.observe(vec![media("new"), media("old"), media("new")], 200);
        assert_eq!(new.len(), 1);
        assert_eq!(new[0].id, "new");
        watch.failed("offline".into(), 300);
        assert_eq!(watch.next_check, 420);
        assert_eq!(watch.seen.as_ref().unwrap().len(), 2);
        watch.interval_seconds = 2 * 86400;
        watch.failed("still offline".into(), 500);
        assert_eq!(watch.next_check, 500 + 2 * 86400);
    }
}

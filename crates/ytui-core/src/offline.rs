//! Portable offline playlist metadata. Device URI grants and media bytes remain platform-owned.
use crate::{Media, http_url};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Default, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct OfflinePlaylist {
    pub id: String,
    pub title: String,
    pub items: Vec<Media>,
}
impl OfflinePlaylist {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            uuid::Uuid::parse_str(&self.id).is_ok(),
            "Invalid offline playlist identity"
        );
        ensure!(
            !self.title.trim().is_empty()
                && self.title.chars().count() <= 120
                && !self.title.chars().any(char::is_control),
            "Use a playlist name from 1 to 120 characters"
        );
        ensure!(
            self.items.len() <= 1000,
            "Offline playlist limit is 1000 items"
        );
        let mut seen = HashSet::new();
        for media in &self.items {
            http_url(&media.url)?;
            ensure!(
                !media.live && !media.collection(),
                "Only saved songs/videos belong in an offline playlist"
            );
            let id = media.offline_id.as_deref().unwrap_or_default();
            ensure!(
                id.len() == 64
                    && id
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                "Invalid saved-media identity"
            );
            ensure!(
                seen.insert(media.url.as_str()),
                "This media is already in the playlist"
            );
            ensure!(
                media.retained_bytes() <= 64 * 1024,
                "Offline item metadata is too large"
            );
        }
        Ok(())
    }
}
pub fn validate_library(playlists: &[OfflinePlaylist]) -> Result<()> {
    ensure!(
        playlists.len() <= 100,
        "Offline playlist limit is 100 playlists"
    );
    ensure!(
        playlists.iter().map(|p| p.items.len()).sum::<usize>() <= 10_000,
        "Offline library item limit reached"
    );
    ensure!(
        playlists
            .iter()
            .flat_map(|p| &p.items)
            .map(Media::retained_bytes)
            .sum::<usize>()
            <= 8 * 1024 * 1024,
        "Offline playlist metadata exceeds memory budget"
    );
    let mut seen = HashSet::new();
    for playlist in playlists {
        playlist.validate()?;
        ensure!(
            seen.insert(&playlist.id),
            "Duplicate offline playlist identity"
        );
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn offline_edits_reject_duplicates_live_streams_uris_and_unbounded_metadata() {
        let media = Media {
            url: "https://media.test/song".into(),
            offline_id: Some("a".repeat(64)),
            ..Default::default()
        };
        let mut playlist = OfflinePlaylist {
            id: uuid::Uuid::new_v4().to_string(),
            title: "Offline music".into(),
            items: vec![media.clone()],
        };
        playlist.validate().unwrap();
        playlist.items.push(media);
        assert!(playlist.validate().is_err());
        playlist.items.pop();
        playlist.items[0].live = true;
        assert!(playlist.validate().is_err());
        playlist.items[0].live = false;
        playlist.items[0].url = "content://private/file".into();
        assert!(playlist.validate().is_err());
        playlist.items[0].url = "https://media.test/song".into();
        playlist.items[0].offline_id = Some("../../file".into());
        assert!(playlist.validate().is_err());
        playlist.items[0].offline_id = Some("b".repeat(64));
        playlist.items[0].description = "x".repeat(65536);
        assert!(playlist.validate().is_err());
        playlist.items.clear();
        playlist.title = "Bad\nname".into();
        assert!(playlist.validate().is_err());
    }
}

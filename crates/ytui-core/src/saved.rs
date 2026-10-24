//! A bounded local shelf of stable YouTube links, independent of account state.
use crate::{Media, MediaKind, Page, clean_text, http_url, youtube_host};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedMedia {
    pub entries: Vec<Media>,
}
fn id(media: &Media) -> Option<String> {
    let url = http_url(&media.url).ok()?;
    if !youtube_host(url.host_str()?) {
        return None;
    }
    let id = if url.host_str() == Some("youtu.be") {
        url.path().trim_matches('/').to_owned()
    } else if url.path() == "/watch" {
        url.query_pairs()
            .find(|(key, _)| key == "v")?
            .1
            .into_owned()
    } else {
        let segments = url.path().trim_matches('/').split('/').collect::<Vec<_>>();
        if segments.len() != 2 || !["shorts", "live", "embed"].contains(&segments[0]) {
            return None;
        }
        segments[1].to_owned()
    };
    (id.len() == 11
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)))
    .then_some(id)
}
impl SavedMedia {
    pub fn contains(&self, media: &Media) -> bool {
        id(media).is_some_and(|key| {
            self.entries
                .iter()
                .any(|item| id(item).as_ref() == Some(&key))
        })
    }
    /// Returns true when saved, false when removed. Capacity failures are atomic.
    pub fn toggle(&mut self, mut media: Media) -> Result<bool> {
        ensure!(
            matches!(media.kind, MediaKind::Video | MediaKind::Song),
            "Save an individual video or song; radio stations have Favorites"
        );
        let key = id(&media).ok_or_else(|| {
            anyhow::anyhow!(
                "Choose a YouTube video or song; downloaded files are in Offline library"
            )
        })?;
        if let Some(index) = self
            .entries
            .iter()
            .position(|item| id(item).as_ref() == Some(&key))
        {
            self.entries.remove(index);
            return Ok(false);
        }
        ensure!(
            self.entries.len() < 500,
            "Saved shelf is full (500 items); remove an item first"
        );
        media.id = key.clone();
        media.url = format!(
            "https://{}/watch?v={key}",
            if media.kind == MediaKind::Song {
                "music.youtube.com"
            } else {
                "www.youtube.com"
            }
        );
        media.title = clean_text(&media.title).chars().take(512).collect();
        media.creator = clean_text(&media.creator).chars().take(256).collect();
        media.description = String::new();
        media.chapters = Vec::new();
        media.browse_id = None;
        media.tracks_url = None;
        media.section = String::new();
        if media.thumbnail.len() > 4096 || http_url(&media.thumbnail).is_err() {
            media.thumbnail = String::new();
        }
        media
            .thumbnails
            .retain(|t| t.url.len() <= 4096 && http_url(&t.url).is_ok());
        media.thumbnails.truncate(8);
        ensure!(
            media.retained_bytes().saturating_add(
                self.entries
                    .iter()
                    .map(Media::retained_bytes)
                    .sum::<usize>()
            ) <= 4 * 1024 * 1024,
            "Saved shelf exceeds 4 MiB; remove an item first"
        );
        self.entries.insert(0, media);
        Ok(true)
    }
    pub fn catalog(&self, query: &str) -> Page {
        let query = query.trim().to_lowercase();
        Page {
            title: "Saved locally".into(),
            items: self
                .entries
                .iter()
                .filter(|m| {
                    query.is_empty()
                        || m.title.to_lowercase().contains(&query)
                        || m.creator.to_lowercase().contains(&query)
                        || m.url.to_lowercase().contains(&query)
                })
                .cloned()
                .collect(),
            notice: Some(
                "Local bookmarks · playback needs a connection · search filters this shelf".into(),
            ),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn video(n: usize) -> Media {
        Media {
            url: format!("https://www.youtube.com/watch?v={n:011}"),
            title: format!("Video {n}"),
            ..Default::default()
        }
    }
    #[test]
    fn stable_links_remove_tracking_and_aliases_toggle_the_same_item() {
        let mut shelf = SavedMedia::default();
        let mut media = video(1);
        media.url.push_str("&list=PLfixture&t=50&si=tracking");
        assert!(shelf.toggle(media).unwrap());
        assert_eq!(
            shelf.entries[0].url,
            "https://www.youtube.com/watch?v=00000000001"
        );
        let alias = Media {
            url: "https://youtu.be/00000000001?si=other".into(),
            ..video(1)
        };
        assert!(shelf.contains(&alias));
        assert!(!shelf.toggle(alias).unwrap());
        assert!(shelf.entries.is_empty());
    }
    #[test]
    fn song_intent_survives_and_large_remote_metadata_is_trimmed() {
        let mut shelf = SavedMedia::default();
        let mut media = video(2);
        media.kind = MediaKind::Song;
        media.title = "x".repeat(2000);
        media.creator = "artist".repeat(1000);
        media.description = "secret/unneeded".repeat(10000);
        media.thumbnail = "file:///unsafe".into();
        media.browse_id = Some("unused".into());
        shelf.toggle(media).unwrap();
        let saved = &shelf.entries[0];
        assert!(saved.url.starts_with("https://music.youtube.com/"));
        assert_eq!(saved.kind, MediaKind::Song);
        assert_eq!(saved.title.len(), 512);
        assert_eq!(saved.creator.len(), 256);
        assert!(
            saved.description.is_empty() && saved.thumbnail.is_empty() && saved.browse_id.is_none()
        );
    }
    #[test]
    fn catalog_is_newest_first_and_searches_unicode_titles_creators_and_ids() {
        let mut shelf = SavedMedia::default();
        let mut media = video(1);
        media.title = "Grüße".into();
        media.creator = "ARTIST".into();
        shelf.toggle(media).unwrap();
        shelf.toggle(video(2)).unwrap();
        assert_eq!(shelf.catalog("").items[0].id, "00000000002");
        for query in ["GRÜßE", "artist", "00000000001"] {
            assert_eq!(shelf.catalog(query).items.len(), 1);
        }
        assert!(shelf.catalog("absent").items.is_empty());
        assert!(shelf.catalog("").continuation.is_none());
    }
    #[test]
    fn invalid_sources_and_collections_do_not_change_the_shelf() {
        let mut shelf = SavedMedia::default();
        shelf.toggle(video(1)).unwrap();
        for media in [
            Media {
                url: "file:///tmp/movie".into(),
                ..video(2)
            },
            Media {
                kind: MediaKind::Playlist,
                ..video(2)
            },
            Media {
                url: "https://user:password@www.youtube.com/watch?v=00000000002".into(),
                ..video(2)
            },
            Media {
                url: "https://youtube.com.evil.test/watch?v=00000000002".into(),
                ..video(2)
            },
            Media {
                url: "https://www.youtube.com/watch?v=bad".into(),
                ..video(2)
            },
        ] {
            assert!(shelf.toggle(media).is_err());
            assert_eq!(shelf.entries.len(), 1);
        }
    }
    #[test]
    fn full_shelf_never_evicts_user_items_and_removal_still_works() {
        let mut shelf = SavedMedia::default();
        for n in 0..500 {
            shelf.toggle(video(n)).unwrap();
        }
        assert!(shelf.toggle(video(500)).is_err());
        assert_eq!(shelf.entries.len(), 500);
        assert!(!shelf.toggle(video(1)).unwrap());
        assert!(shelf.toggle(video(500)).unwrap());
        assert_eq!(shelf.entries.len(), 500);
    }
    #[test]
    fn thumbnail_memory_budget_is_atomic() {
        let mut shelf = SavedMedia::default();
        for n in 0..500 {
            let mut media = video(n);
            media.thumbnails = (0..8)
                .map(|i| crate::Thumbnail {
                    url: format!("https://images.test/{i}/{}", "x".repeat(4000)),
                    width: 320,
                    height: 180,
                })
                .collect();
            let before = shelf.entries.len();
            if shelf.toggle(media).is_err() {
                assert_eq!(shelf.entries.len(), before);
                assert!(before < 500);
                return;
            }
        }
        panic!("Saved shelf exceeded its byte budget");
    }
}

#[cfg(test)]
mod alias_tests {
    use super::*;
    #[test]
    fn shorts_live_and_embed_links_share_a_stable_bookmark() {
        let mut shelf = SavedMedia::default();
        for path in ["shorts", "live", "embed"] {
            let media = Media {
                url: format!("https://www.youtube.com/{path}/abcdefghijk?si=tracking"),
                ..Default::default()
            };
            assert!(shelf.toggle(media.clone()).unwrap());
            assert_eq!(
                shelf.entries[0].url,
                "https://www.youtube.com/watch?v=abcdefghijk"
            );
            assert!(
                !shelf
                    .toggle(Media {
                        url: "https://youtu.be/abcdefghijk".into(),
                        ..media
                    })
                    .unwrap()
            );
        }
    }
}

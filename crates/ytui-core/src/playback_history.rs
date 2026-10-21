use crate::Media;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Position {
    pub media: Media,
    pub seconds: f64,
    pub duration: Option<f64>,
    pub updated_at: u64,
    pub finished: bool,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PlaybackHistory {
    pub entries: Vec<Position>,
}
impl Position {
    pub fn resumable(&self) -> bool {
        !self.finished
            && self.seconds.is_finite()
            && self.seconds >= 5.0
            && self
                .duration
                .is_none_or(|d| d.is_finite() && d > self.seconds + 5.0)
    }
}
impl PlaybackHistory {
    pub fn key(media: &Media) -> String {
        if let Ok(url) = crate::http_url(&media.url)
            && crate::youtube_host(url.host_str().unwrap_or_default())
        {
            let id = if url.host_str() == Some("youtu.be") {
                url.path().trim_start_matches('/').to_owned()
            } else {
                url.query_pairs()
                    .find(|(k, _)| k == "v")
                    .map(|(_, v)| v.into_owned())
                    .unwrap_or_else(|| media.id.clone())
            };
            if !id.is_empty() {
                return format!("youtube:{id}");
            }
        }
        media.url.clone()
    }
    pub fn record(
        &mut self,
        media: &Media,
        seconds: f64,
        duration: Option<f64>,
        finished: bool,
    ) -> bool {
        if media.live
            || media.url.is_empty()
            || !seconds.is_finite()
            || seconds < 0.0
            || (!finished && seconds < 5.0)
        {
            return false;
        }
        let duration = duration.filter(|d| d.is_finite() && *d > 0.0);
        let finished = finished || duration.is_some_and(|d| seconds >= (d - 5.0).max(d * 0.98));
        let old = self
            .entries
            .iter()
            .position(|p| Self::key(&p.media) == Self::key(media));
        if let Some(index) = old {
            let entry = &self.entries[index];
            if entry.finished == finished && (entry.seconds - seconds).abs() < 2.0 {
                return false;
            }
            self.entries.remove(index);
        }
        let mut media = media.clone();
        media.description.clear();
        media.chapters.clear();
        media.title = media.title.chars().take(512).collect();
        media.creator = media.creator.chars().take(256).collect();
        self.entries.insert(
            0,
            Position {
                media,
                seconds,
                duration,
                finished,
                updated_at: crate::schedule::unix_now(),
            },
        );
        self.entries.truncate(500);
        true
    }
    pub fn resumable(&self) -> Vec<&Position> {
        self.entries.iter().filter(|p| p.resumable()).collect()
    }
    pub fn for_media(&self, media: &Media) -> Option<&Position> {
        self.entries
            .iter()
            .find(|p| Self::key(&p.media) == Self::key(media) && p.resumable())
    }
    pub fn clear_remote(&mut self) {
        self.entries
            .retain(|p| crate::http_url(&p.media.url).is_err());
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn positions_are_bounded_finite_and_private_to_the_active_account() {
        let mut history = PlaybackHistory::default();
        let media = Media {
            url: "https://www.youtube.com/watch?v=abcdefghijk".into(),
            ..Default::default()
        };
        assert!(!history.record(&media, f64::NAN, Some(100.0), false));
        assert!(history.record(&media, 20.0, Some(100.0), false));
        assert!(history.for_media(&media).is_some());
        assert!(!history.record(&media, 20.5, Some(100.0), false));
        history.record(&media, 100.0, Some(100.0), true);
        assert!(history.resumable().is_empty());
        let local = Media {
            url: "/tmp/own.mp4".into(),
            ..Default::default()
        };
        history.record(&local, 10.0, Some(200.0), false);
        history.clear_remote();
        assert_eq!(history.entries.len(), 1);
        for i in 0..510 {
            history.record(
                &Media {
                    url: format!("/tmp/{i}.mp4"),
                    ..Default::default()
                },
                10.0,
                None,
                false,
            );
        }
        assert_eq!(history.entries.len(), 500);
    }
}

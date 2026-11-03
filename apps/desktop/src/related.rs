//! Related-video continuation for the playback queue (M5/M9 parity with Android's
//! "Related media autoplay" and its watch-page Up next list).
//!
//! When the item that ends the queue is a YouTube video or song, ytui fetches its
//! related list. Automatic advance (and Next) continue with the first entry once the
//! queue is exhausted; the Queue popup lists the entries as "Recommended". Nothing
//! here is persisted: the queue only grows by the entries that actually play.
use ytui_core::{Media, MediaKind, PlaybackQueue, Repeat};

/// At most this many related entries are kept for one source video.
pub const LIMIT: usize = 25;

/// Related videos for one source item. `source` is the item's URL.
#[derive(Clone, Debug, Default)]
pub struct RelatedNext {
    pub source: String,
    pub items: Vec<Media>,
    pub loading: bool,
    pub error: Option<String>,
    /// The queue ended while the list was still loading: play its first entry on arrival.
    pub play_when_ready: bool,
}

fn valid_id(id: &str) -> bool {
    id.len() == 11
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The YouTube video ID of a playable video or song, if it has one.
pub fn video_id(media: &Media) -> Option<String> {
    if !matches!(media.kind, MediaKind::Video | MediaKind::Song)
        || media.live
        || media.collection()
        || media.browse_id.is_some()
    {
        return None;
    }
    if valid_id(&media.id) {
        return Some(media.id.clone());
    }
    let url = url::Url::parse(&media.url).ok()?;
    let host = url.host_str()?.trim_start_matches("www.");
    let id = match host {
        "youtu.be" => url.path_segments()?.next()?.to_string(),
        "youtube.com" | "m.youtube.com" | "music.youtube.com" => url
            .query_pairs()
            .find(|(key, _)| key == "v")
            .map(|(_, value)| value.into_owned())?,
        _ => return None,
    };
    valid_id(&id).then_some(id)
}

/// True when automatic advance from the current item would stop: no later queue
/// entry, no repeat and no shuffle choice among other entries.
pub fn queue_ends_after_current(queue: &PlaybackQueue) -> bool {
    let Some(current) = queue.current else {
        return false;
    };
    current < queue.items.len()
        && current + 1 == queue.items.len()
        && queue.repeat == Repeat::Off
        && !(queue.shuffle && queue.items.len() > 1)
}

/// Playable related entries: no collections, live streams, the source itself or
/// anything already in the queue (recently played), bounded to [`LIMIT`].
pub fn candidates(items: Vec<Media>, source: &str, queue: &[Media]) -> Vec<Media> {
    let mut seen = std::collections::HashSet::new();
    items
        .into_iter()
        .filter(|m| {
            matches!(m.kind, MediaKind::Video | MediaKind::Song)
                && !m.collection()
                && m.browse_id.is_none()
                && !m.live
                && m.url != source
                && !queue.iter().any(|q| q.url == m.url)
                && seen.insert(m.url.clone())
        })
        .take(LIMIT)
        .collect()
}

/// Index of the entry that automatic advance plays next in the combined Up next
/// list (queue entries first, then `related` recommendations). `None` when shuffle
/// picks at random, repeat-one replays the current item, or nothing follows.
pub fn plays_next(queue: &PlaybackQueue, related: usize, autoplay: bool) -> Option<usize> {
    let current = queue.current?;
    if queue.repeat == Repeat::One || (queue.shuffle && queue.items.len() > 1) {
        return None;
    }
    if current + 1 < queue.items.len() {
        Some(current + 1)
    } else if queue.repeat == Repeat::All {
        Some(0)
    } else if autoplay && related > 0 {
        Some(queue.items.len())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn video(id: &str) -> Media {
        Media {
            id: id.into(),
            url: format!("https://www.youtube.com/watch?v={id}"),
            kind: MediaKind::Video,
            ..Default::default()
        }
    }
    #[test]
    fn video_ids_come_from_the_id_or_a_youtube_url_only() {
        assert_eq!(
            video_id(&video("abcdefghijk")).as_deref(),
            Some("abcdefghijk")
        );
        let short = Media {
            url: "https://youtu.be/ABCDEFGHI-_".into(),
            kind: MediaKind::Song,
            ..Default::default()
        };
        assert_eq!(video_id(&short).as_deref(), Some("ABCDEFGHI-_"));
        let music = Media {
            url: "https://music.youtube.com/watch?v=abcdefghijk&list=RDsynthetic".into(),
            kind: MediaKind::Song,
            ..Default::default()
        };
        assert_eq!(video_id(&music).as_deref(), Some("abcdefghijk"));
        let local = Media {
            url: "/home/user/Videos/clip.mp4".into(),
            kind: MediaKind::Video,
            ..Default::default()
        };
        assert_eq!(video_id(&local), None);
        let mut live = video("abcdefghijk");
        live.live = true;
        assert_eq!(video_id(&live), None);
        let mut station = video("abcdefghijk");
        station.kind = MediaKind::Station;
        assert_eq!(video_id(&station), None);
        let mut playlist = video("abcdefghijk");
        playlist.browse_id = Some("VLsynthetic".into());
        assert_eq!(video_id(&playlist), None);
        let other = Media {
            url: "https://media.test/watch?v=abcdefghijk".into(),
            kind: MediaKind::Video,
            ..Default::default()
        };
        assert_eq!(video_id(&other), None);
    }
    #[test]
    fn queue_end_respects_repeat_and_shuffle() {
        let mut queue = PlaybackQueue::default();
        assert!(!queue_ends_after_current(&queue));
        queue.play(video("aaaaaaaaaaa"));
        assert!(queue_ends_after_current(&queue));
        queue.shuffle = true;
        assert!(queue_ends_after_current(&queue), "shuffle of one item ends");
        queue.push(video("bbbbbbbbbbb"));
        assert!(!queue_ends_after_current(&queue));
        queue.shuffle = false;
        assert!(!queue_ends_after_current(&queue), "a later entry follows");
        queue.current = Some(1);
        assert!(queue_ends_after_current(&queue));
        queue.repeat = Repeat::All;
        assert!(!queue_ends_after_current(&queue));
        queue.repeat = Repeat::One;
        assert!(!queue_ends_after_current(&queue));
    }
    #[test]
    fn candidates_drop_collections_live_duplicates_and_queued_items() {
        let source = video("sssssssssss");
        let queued = video("qqqqqqqqqqq");
        let mut live = video("lllllllllll");
        live.live = true;
        let mut playlist = video("ppppppppppp");
        playlist.browse_id = Some("VLsynthetic".into());
        let keep = video("kkkkkkkkkkk");
        let items = vec![
            source.clone(),
            queued.clone(),
            live,
            playlist,
            keep.clone(),
            keep.clone(),
        ];
        let result = candidates(items, &source.url, &[queued]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].url, keep.url);
        let many = (0..40).map(|i| video(&format!("{i:0>11}"))).collect();
        assert_eq!(candidates(many, &source.url, &[]).len(), LIMIT);
    }
    #[test]
    fn plays_next_prefers_the_queue_then_recommendations() {
        let mut queue = PlaybackQueue::default();
        assert_eq!(plays_next(&queue, 3, true), None);
        queue.play(video("aaaaaaaaaaa"));
        assert_eq!(plays_next(&queue, 3, true), Some(1));
        assert_eq!(plays_next(&queue, 3, false), None);
        assert_eq!(plays_next(&queue, 0, true), None);
        queue.push(video("bbbbbbbbbbb"));
        assert_eq!(plays_next(&queue, 3, true), Some(1));
        queue.current = Some(1);
        assert_eq!(plays_next(&queue, 3, true), Some(2));
        queue.repeat = Repeat::All;
        assert_eq!(plays_next(&queue, 3, true), Some(0));
        queue.repeat = Repeat::One;
        assert_eq!(plays_next(&queue, 3, true), None);
        queue.repeat = Repeat::Off;
        queue.shuffle = true;
        assert_eq!(plays_next(&queue, 3, true), None);
    }
}

use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    #[default]
    Video,
    Song,
    Playlist,
    Album,
    Artist,
    Channel,
    Station,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Media {
    pub id: String,
    pub title: String,
    pub creator: String,
    pub url: String,
    pub thumbnail: String,
    pub duration: Option<u64>,
    pub description: String,
    pub kind: MediaKind,
    pub browse_id: Option<String>,
    pub tracks_url: Option<String>,
    pub live: bool,
    pub chapters: Vec<crate::Chapter>,
}
impl Media {
    pub fn collection(&self) -> bool {
        matches!(
            self.kind,
            MediaKind::Playlist | MediaKind::Album | MediaKind::Artist | MediaKind::Channel
        )
    }
    pub fn label(&self) -> String {
        if self.live {
            "LIVE".into()
        } else if let Some(d) = self.duration {
            if d >= 3600 {
                format!("{}:{:02}:{:02}", d / 3600, d / 60 % 60, d % 60)
            } else {
                format!("{}:{:02}", d / 60, d % 60)
            }
        } else {
            format!("{:?}", self.kind)
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Page {
    pub title: String,
    pub items: Vec<Media>,
    pub continuation: Option<String>,
    pub notice: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum BrowseRequest {
    MusicSearch { query: String, category: String },
    Details(String),
    Home,
    Search(String),
    Subscriptions,
    WatchLater,
    Liked,
    Library,
    History,
    Collection(String),
    Related(String),
    Continue(String),
    Cliamp,
    CliampTracks(String),
    RadioSearch(String),
    RadioCountry(String),
    RadioTag(String),
    Favorites,
    OfflineLibrary { query: String, audio: Option<bool> },
}

pub fn http_url(raw: &str) -> anyhow::Result<Url> {
    let url = Url::parse(raw)?;
    anyhow::ensure!(
        matches!(url.scheme(), "http" | "https") && url.host_str().is_some(),
        "Only HTTP(S) URLs are supported"
    );
    anyhow::ensure!(
        url.username().is_empty() && url.password().is_none(),
        "Credentials in media URLs are unsupported"
    );
    Ok(url)
}
pub fn youtube_host(host: &str) -> bool {
    host == "youtube.com" || host.ends_with(".youtube.com") || host == "youtu.be"
}
/// Preserve real playlist context, but never expand an auto-generated mix implicitly.
pub fn normalize_media_url(raw: &str) -> anyhow::Result<String> {
    let raw = raw.trim();
    let raw = if raw.starts_with("youtu.be/")
        || raw.starts_with("youtube.com/")
        || raw.starts_with("www.youtube.com/")
    {
        format!("https://{raw}")
    } else {
        raw.to_owned()
    };
    let mut url = http_url(&raw)?;
    if youtube_host(url.host_str().unwrap_or_default()) {
        let pairs: Vec<_> = url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        let mix = pairs
            .iter()
            .any(|(k, v)| k == "list" && v.starts_with("RD"));
        if mix {
            url.query_pairs_mut().clear().extend_pairs(
                pairs
                    .iter()
                    .filter(|(k, _)| k != "list" && k != "index" && k != "start_radio"),
            );
        }
    }
    Ok(url.to_string())
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PlaybackQueue {
    pub items: Vec<Media>,
    pub current: Option<usize>,
    pub repeat: Repeat,
    pub shuffle: bool,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum Repeat {
    #[default]
    Off,
    All,
    One,
}
impl PlaybackQueue {
    pub fn push(&mut self, media: Media) {
        self.items.push(media);
    }
    pub fn play(&mut self, media: Media) {
        self.items.push(media);
        self.current = Some(self.items.len() - 1);
    }
    pub fn advance(&mut self, automatic: bool) -> Option<Media> {
        let i = self.current?;
        if self.items.is_empty() {
            self.current = None;
            return None;
        }
        let next = if automatic && self.repeat == Repeat::One {
            i
        } else if self.shuffle && self.items.len() > 1 {
            // UUID randomness avoids a global RNG in the reducer and never repeats current.
            (i + 1 + (uuid::Uuid::new_v4().as_u128() as usize % (self.items.len() - 1)))
                % self.items.len()
        } else if i + 1 < self.items.len() {
            i + 1
        } else if self.repeat == Repeat::All {
            0
        } else {
            self.current = None;
            return None;
        };
        self.current = Some(next);
        self.items.get(next).cloned()
    }
    pub fn previous(&mut self) -> Option<Media> {
        let i = self.current?.saturating_sub(1);
        self.current = Some(i);
        self.items.get(i).cloned()
    }
    pub fn remove(&mut self, index: usize) {
        if index >= self.items.len() {
            return;
        }
        self.items.remove(index);
        self.current = match self.current {
            Some(i) if i == index => None,
            Some(i) if i > index => Some(i - 1),
            other => other,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn url_queries_survive_mix_removal() {
        let u =
            normalize_media_url("https://music.youtube.com/watch?list=RD123&v=abcdefghijk&t=21")
                .unwrap();
        let u = Url::parse(&u).unwrap();
        assert_eq!(
            u.query_pairs().collect::<Vec<_>>(),
            vec![
                ("v".into(), "abcdefghijk".into()),
                ("t".into(), "21".into())
            ]
        );
        assert!(http_url("file:///etc/passwd").is_err());
        assert!(!youtube_host("youtube.com.evil.test"));
    }
    #[test]
    fn repeat_and_removal_preserve_queue_position() {
        let mut q = PlaybackQueue::default();
        q.play(Media {
            id: "one".into(),
            ..Default::default()
        });
        q.push(Media {
            id: "two".into(),
            ..Default::default()
        });
        q.repeat = Repeat::One;
        assert_eq!(q.advance(true).unwrap().id, "one");
        assert_eq!(q.advance(false).unwrap().id, "two");
        q.remove(0);
        assert_eq!(q.current, Some(0));
        q.repeat = Repeat::Off;
        assert!(q.advance(true).is_none());
    }
}

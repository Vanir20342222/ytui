//! Explicit, bounded local lyric edits. No cookies or remote lookup on load/save.
use super::*;

#[derive(Clone, Serialize, Deserialize)]
pub struct Document {
    pub title: String,
    pub artist: String,
    pub text: String,
    pub offset_ms: i64,
}
impl Document {
    pub fn track(&self) -> Result<ytui_providers::lyrics::Track> {
        ensure!(
            self.title.len() <= 512 && self.artist.len() <= 512,
            "Lyric title/artist exceed 512 bytes"
        );
        ensure!(
            self.offset_ms.unsigned_abs() <= 120_000,
            "Lyric adjustment must be within two minutes"
        );
        let lines = ytui_providers::lyrics::parse(&self.text)?;
        ensure!(
            lines.iter().any(|l| !l.text.trim().is_empty()),
            "Lyrics are empty"
        );
        Ok(ytui_providers::lyrics::Track {
            title: self.title.clone(),
            artist: self.artist.clone(),
            album: String::new(),
            duration: 0.0,
            instrumental: false,
            lines,
        })
    }
}
impl Engine {
    fn edit_lyrics(
        &self,
        update: impl FnOnce(&mut HashMap<String, Document>) -> Result<Value>,
    ) -> Result<Value> {
        // Lyrics have their own file so periodic resume/queue writes never clone
        // or re-serialize megabytes of text in the playback path.
        let mut stored = self.lyrics.lock().expect("lyrics");
        let mut draft = stored.clone();
        let result = update(&mut draft)?;
        let bytes = serde_json::to_vec(&draft)?;
        ensure!(
            bytes.len() <= 16 * 1024 * 1024,
            "Encoded lyric library exceeds limit"
        );
        atomic_write(&self.root.join("lyrics.json"), &bytes)?;
        *stored = draft;
        Ok(result)
    }
    pub(crate) fn local_lyrics(&self, r: &Value) -> Result<Value> {
        let url = r["url"].as_str().context("Missing lyric media URL")?;
        ensure!(url.len() <= 8192, "Media URL exceeds limit");
        ytui_core::http_url(url)?;
        match r["op"].as_str().unwrap_or_default() {
            "lyrics_get" => {
                let stored = self.lyrics.lock().expect("lyrics");
                if let Some(document) = stored.get(url) {
                    Ok(json!({"document":document,"track":document.track()?}))
                } else {
                    Ok(Value::Null)
                }
            }
            "lyrics_remove" => self.edit_lyrics(|s| {
                s.remove(url);
                Ok(json!({"removed":true}))
            }),
            "lyrics_save" => {
                let document: Document = serde_json::from_value(r["document"].clone())?;
                let track = document.track()?;
                self.edit_lyrics(|s| {
                    ensure!(
                        s.contains_key(url) || s.len() < 100,
                        "Local lyric library is full (100 songs); remove an entry first"
                    );
                    let bytes = s
                        .iter()
                        .filter(|(key, _)| *key != url)
                        .map(|(key, doc)| {
                            key.len() + doc.text.len() + doc.title.len() + doc.artist.len()
                        })
                        .sum::<usize>()
                        + url.len()
                        + document.text.len()
                        + document.title.len()
                        + document.artist.len();
                    ensure!(
                        bytes <= 8 * 1024 * 1024,
                        "Local lyrics exceed the 8 MiB library limit"
                    );
                    s.insert(url.into(), document.clone());
                    Ok(json!({"document":document,"track":track}))
                })
            }
            _ => anyhow::bail!("Unknown local lyric operation"),
        }
    }
}

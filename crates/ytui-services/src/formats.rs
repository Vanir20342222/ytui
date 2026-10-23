//! Available extractor formats and captions, rather than invented quality rows.
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Catalog {
    pub formats: Vec<Format>,
    pub captions: Vec<Caption>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Format {
    pub id: String,
    pub label: String,
    pub video: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Caption {
    pub label: String,
    pub url: String,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub id: i64,
    pub kind: String,
    pub label: String,
    pub selected: bool,
}
fn text(value: &Value, key: &str) -> String {
    ytui_core::clean_text(value[key].as_str().unwrap_or(""))
        .chars()
        .take(100)
        .collect()
}
impl Catalog {
    pub fn parse(value: &Value) -> Self {
        let mut formats = Vec::new();
        if let Some(rows) = value["formats"].as_array() {
            for row in rows.iter().take(1000) {
                if row["has_drm"].as_bool() == Some(true) {
                    continue;
                }
                let id = text(row, "format_id");
                if id.is_empty()
                    || !id
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
                {
                    continue;
                }
                let video = row["vcodec"].as_str().is_some_and(|value| value != "none");
                let audio = row["acodec"].as_str().is_some_and(|value| value != "none");
                if !video && !audio {
                    continue;
                }
                let height = row["height"]
                    .as_u64()
                    .map_or_else(String::new, |height| format!("{height}p"));
                let label = format!(
                    "{id} · {} {height} · {} {} · {} {}",
                    if video { "Video" } else { "Audio" },
                    text(row, "ext"),
                    text(row, if video { "vcodec" } else { "acodec" }),
                    text(row, "language"),
                    text(row, "format_note")
                );
                formats.push(Format { id, label, video });
            }
        }
        let mut captions = Vec::new();
        for (key, automatic) in [("subtitles", false), ("automatic_captions", true)] {
            if let Some(languages) = value[key].as_object() {
                for (language, rows) in languages.iter().take(200) {
                    let Some(rows) = rows.as_array() else {
                        continue;
                    };
                    let Some(row) = rows
                        .iter()
                        .find(|row| row["ext"] == "vtt")
                        .or_else(|| rows.iter().find(|row| row["ext"] == "srt"))
                    else {
                        continue;
                    };
                    let Some(url) = row["url"]
                        .as_str()
                        .filter(|url| ytui_core::http_url(url).is_ok())
                    else {
                        continue;
                    };
                    captions.push(Caption {
                        label: format!(
                            "{} · {} {}",
                            ytui_core::clean_text(language),
                            if automatic { "automatic" } else { "captions" },
                            text(row, "name")
                        ),
                        url: url.into(),
                    });
                }
            }
        }
        Self { formats, captions }
    }
}
pub fn tracks(value: &Value) -> Vec<Track> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .take(128)
        .filter_map(|row| {
            let kind = row["type"].as_str()?;
            if !["audio", "sub"].contains(&kind) {
                return None;
            }
            Some(Track {
                id: row["id"].as_i64()?,
                kind: kind.into(),
                selected: row["selected"].as_bool().unwrap_or(false),
                label: format!(
                    "{} · {} · {} {}",
                    kind,
                    text(row, "lang"),
                    text(row, "title"),
                    text(row, "codec")
                ),
            })
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_format_ids_languages_and_caption_classes_are_preserved() {
        let catalog = Catalog::parse(
            &serde_json::json!({"formats":[{"format_id":"137","height":1080,"vcodec":"avc1","acodec":"none","ext":"mp4"},{"format_id":"140-de","vcodec":"none","acodec":"aac","language":"de"},{"format_id":"bad+selector","vcodec":"vp9"},{"format_id":"999","vcodec":"av1","has_drm":true}], "subtitles":{"de":[{"ext":"vtt","url":"https://example.test/captions"}]}, "automatic_captions":{"en":[{"ext":"vtt","url":"https://example.test/auto"}]}}),
        );
        assert_eq!(catalog.formats.len(), 2);
        assert!(catalog.formats[0].label.contains("1080p"));
        assert!(catalog.formats[1].label.contains("de"));
        assert!(catalog.captions[1].label.contains("automatic"));
    }
}

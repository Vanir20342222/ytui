use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Chapter {
    pub title: String,
    pub start: f64,
    #[serde(default)]
    pub end: Option<f64>,
}
fn timestamp(value: &str) -> Option<f64> {
    let values = value
        .trim_matches(['[', ']', '(', ')'])
        .split(':')
        .map(str::parse::<u32>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    if !(2..=3).contains(&values.len()) || values.iter().skip(1).any(|v| *v >= 60) {
        return None;
    }
    Some(
        values
            .into_iter()
            .fold(0u64, |total, v| total.saturating_mul(60) + v as u64) as f64,
    )
}
pub fn from_description(value: &str) -> Vec<Chapter> {
    let mut chapters = Vec::new();
    for line in value.lines().take(10000) {
        let line = line.trim();
        let Some((time, title)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let Some(start) = timestamp(time) else {
            continue;
        };
        let title = crate::clean_text(title.trim_start_matches([' ', '-', '–', '—', ':']));
        if title.is_empty() {
            continue;
        }
        chapters.push(Chapter {
            start,
            end: None,
            title,
        });
        if chapters.len() == 1000 {
            break;
        }
    }
    chapters.sort_by(|a, b| a.start.total_cmp(&b.start));
    chapters.dedup_by(|a, b| a.start == b.start);
    // A lone timestamp is usually a link/reference rather than a chapter list.
    if chapters.len() < 2 {
        chapters.clear();
    }
    chapters
}
pub fn normalize(chapters: &[Chapter], duration: f64) -> Vec<Chapter> {
    if !duration.is_finite() || duration <= 0.0 {
        return Vec::new();
    }
    let mut chapters = chapters
        .iter()
        .filter(|c| c.start.is_finite() && c.start >= 0.0 && c.start < duration)
        .take(1000)
        .cloned()
        .collect::<Vec<_>>();
    chapters.sort_by(|a, b| a.start.total_cmp(&b.start));
    chapters.dedup_by(|a, b| a.start == b.start);
    let mut normalized = Vec::new();
    for (i, chapter) in chapters.iter().enumerate() {
        let boundary = chapters.get(i + 1).map(|c| c.start).unwrap_or(duration);
        let end = chapter
            .end
            .filter(|v| v.is_finite())
            .unwrap_or(boundary)
            .min(boundary)
            .min(duration);
        if (end * 1000.0).round() > (chapter.start * 1000.0).round() {
            normalized.push(Chapter {
                start: chapter.start,
                end: Some(end),
                title: crate::clean_text(&chapter.title),
            });
        }
    }
    normalized
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_description_and_validates_timestamps() {
        let chapters = from_description(
            "Notes\n0:00 Intro\n01:20 - Details 日本語\n1:02:03 Outro\n4:99 Invalid\n",
        );
        assert_eq!(chapters.len(), 3);
        assert_eq!(chapters[2].start, 3723.0);
        let chapters = normalize(&chapters, 3800.0);
        assert_eq!(chapters[0].end, Some(80.0));
        assert_eq!(chapters[2].end, Some(3800.0));
        assert!(from_description("5:20 A reference").is_empty());
        assert!(normalize(&chapters, f64::NAN).is_empty());
    }
}

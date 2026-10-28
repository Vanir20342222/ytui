//! Upstream `render_subtitles_table` / `list_subtitles` output; never selects
//! or inspects resources.
use super::formats::{escape, render_table};
use crate::{Subtitles, VideoInfo};

/// `[info] Available subtitles for ID:` and the table, or `ID has no subtitles`.
pub fn render_subtitles(info: &VideoInfo) -> String {
    listing(info, &info.subtitles, "subtitles")
}

/// The same listing for automatic captions.
pub fn render_automatic_captions(info: &VideoInfo) -> String {
    listing(info, &info.automatic_captions, "automatic captions")
}

fn listing(info: &VideoInfo, subtitles: &Subtitles, name: &str) -> String {
    match render_subtitles_table(subtitles) {
        Some(table) => format!(
            "[info] Available {name} for {}:\n{table}\n",
            escape(&info.id)
        ),
        None => format!("{} has no {name}\n", escape(&info.id)),
    }
}

/// Upstream `render_subtitles_table`: formats in reverse order, with one name
/// when all agree (none when all are unknown); `None` without subtitles.
pub fn render_subtitles_table(subtitles: &Subtitles) -> Option<String> {
    if subtitles.entries.is_empty() {
        return None;
    }
    let rows = subtitles
        .entries
        .iter()
        .map(|(language, formats)| {
            let (exts, mut names): (Vec<String>, Vec<String>) = formats
                .iter()
                .rev()
                .map(|format| {
                    (
                        escape(&format.ext),
                        format
                            .name
                            .as_deref()
                            .filter(|name| !name.is_empty())
                            .map_or_else(|| "unknown".to_owned(), escape),
                    )
                })
                .unzip();
            if names.iter().all(|name| Some(name) == names.first()) {
                names.truncate(usize::from(
                    names.first().is_some_and(|name| name != "unknown"),
                ));
            }
            vec![
                escape(language.as_deref().unwrap_or("None")),
                names.join(", "),
                exts.join(", "),
            ]
        })
        .collect();
    let header = ["Language", "Name", "Formats"].map(String::from);
    Some(render_table(header.to_vec(), rows, None, 0, true))
}

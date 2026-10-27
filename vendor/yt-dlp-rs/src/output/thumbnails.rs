//! Upstream `render_thumbnails_table` / `list_thumbnails` output.
use crate::VideoInfo;

/// Upstream `render_table` without styles: each cell padded to the column width
/// plus one space, rows right-trimmed. Widths count characters.
fn render_table(header: &[&str], rows: &[Vec<String>]) -> String {
    let mut widths: Vec<usize> = header.iter().map(|cell| cell.chars().count()).collect();
    for row in rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    let line = |cells: &mut dyn Iterator<Item = &str>| {
        let mut line = String::new();
        for (cell, width) in cells.zip(&widths) {
            line.push_str(cell);
            line.push_str(&" ".repeat(width - cell.chars().count() + 1));
        }
        line.trim_end().to_owned()
    };
    let mut lines = vec![line(&mut header.iter().copied())];
    lines.extend(
        rows.iter()
            .map(|row| line(&mut row.iter().map(String::as_str))),
    );
    lines.join("\n")
}

/// `[info] Available thumbnails for ID:` and the table, or `ID has no thumbnails`.
pub fn render_thumbnails(info: &VideoInfo) -> String {
    match render_thumbnails_table(info) {
        Some(table) => format!("[info] Available thumbnails for {}:\n{table}\n", info.id),
        None => format!("{} has no thumbnails\n", info.id),
    }
}

/// Upstream `render_thumbnails_table`: `None` without thumbnails.
pub fn render_thumbnails_table(info: &VideoInfo) -> Option<String> {
    let thumbnails = &info.metadata.thumbnails;
    if thumbnails.is_empty() {
        return None;
    }
    let rows: Vec<Vec<String>> = thumbnails
        .iter()
        .map(|thumbnail| {
            let dimension = |value: Option<u32>| {
                value
                    .filter(|&v| v != 0)
                    .map_or_else(|| "unknown".to_owned(), |v| v.to_string())
            };
            vec![
                thumbnail.id.clone().unwrap_or_else(|| "None".into()),
                dimension(thumbnail.width),
                dimension(thumbnail.height),
                thumbnail.url.clone(),
            ]
        })
        .collect();
    Some(render_table(&["ID", "Width", "Height", "URL"], &rows))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_matches_upstream_render_table_layout() {
        let mut info: VideoInfo = serde_json::from_value(serde_json::json!({
            "id": "v", "title": "t", "ext": "mp4", "url": "https://e.invalid/v.mp4",
            "webpage_url": "https://e.invalid/", "extractor": "x", "extractor_key": "X",
            "direct": true,
        }))
        .unwrap();
        assert_eq!(render_thumbnails(&info), "v has no thumbnails\n");
        info.metadata.thumbnails = vec![
            crate::Thumbnail {
                url: "https://e.invalid/a.jpg".into(),
                id: Some("0".into()),
                ..Default::default()
            },
            crate::Thumbnail {
                url: "https://e.invalid/b.jpg".into(),
                id: Some("10".into()),
                width: Some(1280),
                height: Some(720),
                ..Default::default()
            },
        ];
        assert_eq!(
            render_thumbnails(&info),
            "[info] Available thumbnails for v:\nID Width   Height  URL\n0  unknown unknown https://e.invalid/a.jpg\n10 1280    720     https://e.invalid/b.jpg\n"
        );
    }
}

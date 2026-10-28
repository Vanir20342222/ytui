pub fn diagnostic(raw: &str) -> String {
    let raw = raw.trim().strip_prefix("ytui:").unwrap_or(raw);
    raw.split_whitespace()
        .map(|w| {
            if w.contains("://") {
                "[URL]".to_owned()
            } else if w.len() > 100 {
                "[redacted]".to_owned()
            } else {
                ytui_core::clean_text(w)
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(500)
        .collect()
}

pub mod chapters;
pub mod config;
pub mod saved;
pub use chapters::Chapter;
pub mod history;
pub mod media;
pub mod offline;
pub mod playback_history;
pub mod queue;
pub mod schedule;
pub mod shortcuts;
pub mod watch;
pub use config::*;
pub use media::*;
pub use queue::*;

/// Remote titles must never inject terminal control sequences.
pub fn clean_text(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .collect()
}

pub mod theme;

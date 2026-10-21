pub mod config;
pub mod history;
pub mod media;
pub mod queue;
pub mod schedule;
pub mod shortcuts;
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

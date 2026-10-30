//! Generate a player contract from synthetic FFmpeg single-file DASH resources.
#[path = "../src/indexed_playback.rs"]
mod indexed_playback;
use anyhow::{Result, ensure};
use serde_json::json;
use std::path::PathBuf;

fn main() -> Result<()> {
    let root = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or_else(|| anyhow::anyhow!("Use the synthetic DASH asset directory"))?,
    );
    let mut tracks = Vec::new();
    for (index, video) in [(0, true), (1, false)] {
        let bytes = std::fs::read(root.join(format!("index-stream{index}.mp4")))?;
        let mut pos = 0;
        let mut ranges = None;
        while pos + 8 <= bytes.len() {
            let size = u32::from_be_bytes(bytes[pos..pos + 4].try_into()?) as usize;
            ensure!(
                size >= 8 && pos + size <= bytes.len(),
                "Invalid synthetic MP4 box"
            );
            if &bytes[pos + 4..pos + 8] == b"sidx" {
                ensure!(pos > 0, "Synthetic index must follow initialization");
                ranges = Some((pos, pos + size - 1));
                break;
            }
            pos += size;
        }
        let (start, end) =
            ranges.ok_or_else(|| anyhow::anyhow!("Missing global synthetic sidx"))?;
        tracks.push(json!({"url":format!("http://127.0.0.1:1/videoplayback?track={index}&clen={}",bytes.len()),"protocol":"http","ext":"mp4","filesize":bytes.len(),"vcodec":if video {"avc1.42c00c"} else {"none"},"acodec":if video {"none"} else {"mp4a.40.2"},"width":if video {320} else {0},"height":if video {180} else {0},"tbr":if video {500} else {128},"ytui_init_range":{"start":0,"end":start-1},"ytui_index_range":{"start":start,"end":end},"ytui_duration_ms":8000}));
    }
    let manifest = indexed_playback::manifest(&[&tracks[0], &tracks[1]], Some(8.0))
        .ok_or_else(|| anyhow::anyhow!("Synthetic contract rejected"))?;
    std::fs::write(
        root.join("contract.json"),
        serde_json::to_vec_pretty(&json!({"tracks":tracks,"dash_manifest":manifest}))?,
    )?;
    Ok(())
}

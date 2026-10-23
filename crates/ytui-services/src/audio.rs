//! Consistent FFmpeg audio encoding for local conversion and native editing.
use anyhow::{Result, ensure};
pub(crate) fn encoding(format: &str, quality: &str) -> Result<Vec<String>> {
    let codec = match format {
        "mp3" => "libmp3lame",
        "flac" => "flac",
        "wav" => "pcm_s16le",
        "opus" => "libopus",
        "vorbis" => "libvorbis",
        "m4a" | "aac" => "aac",
        _ => anyhow::bail!("Unsupported audio format"),
    };
    let bitrate = quality.ends_with(['k', 'K']);
    let value = quality.trim_end_matches(['k', 'K']).parse::<f64>()?;
    ensure!(
        value.is_finite() && (0.0..=1_000_000.0).contains(&value),
        "Invalid audio quality"
    );
    let mut args = vec!["-c:a".into(), codec.into()];
    if !["flac", "wav"].contains(&format) {
        if bitrate || value > 10.0 {
            ensure!(value > 0.0, "Audio bitrate must be positive");
            args.extend(["-b:a".into(), format!("{value}k")]);
        } else if format == "opus" {
            args.extend(["-b:a".into(), format!("{}k", 256.0 - value * 16.0)]);
        } else {
            let value = match format {
                "mp3" => value,
                "vorbis" => 10.0 - value,
                _ => 4.0 - 3.9 * value / 10.0,
            };
            args.extend(["-q:a".into(), value.to_string()]);
        }
    }
    Ok(args)
}

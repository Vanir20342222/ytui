//! Local subtitle conversion, with native TTML preprocessing and atomic outputs.
pub(crate) mod ffmpeg;
pub(crate) mod ttml;

use crate::{DownloadPhase, DownloadProgress, Error, Result, YoutubeDl};
use std::{
    fs,
    io::Read,
    ops::ControlFlow,
    path::{Path, PathBuf},
};
const MAX_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct SubtitleConversion {
    target: Option<&'static str>,
    pub keep_original: bool,
}
impl SubtitleConversion {
    pub fn is_enabled(&self) -> bool {
        self.target.is_some()
    }
    /// The requested extension, if conversion is enabled.
    pub fn target(&self) -> Option<&'static str> {
        self.target
    }
    pub fn new(format: &str) -> Result<Self> {
        let target = match format {
            "srt" => Some("srt"),
            "vtt" => Some("vtt"),
            "ass" => Some("ass"),
            "lrc" => Some("lrc"),
            "none" => None,
            _ => {
                return Err(Error::Subtitle(
                    "unsupported subtitle conversion target".into(),
                ));
            }
        };
        Ok(Self {
            target,
            keep_original: false,
        })
    }
    /// Predict the converted sidecar name without I/O. Source JSON and matching
    /// extensions are skips; active conversions replace the final suffix.
    pub fn output_path(&self, source: &Path, source_ext: &str) -> Result<PathBuf> {
        if source.file_name().is_none() {
            return Err(Error::Subtitle("subtitle source has no filename".into()));
        }
        Ok(
            match self
                .target
                .filter(|target| *target != source_ext && source_ext != "json")
            {
                Some(target) => source.with_extension(target),
                None => source.into(),
            },
        )
    }
    /// TTML first becomes SRT when another output format is requested.
    pub fn intermediate_path(&self, source: &Path, source_ext: &str) -> Option<PathBuf> {
        (matches!(source_ext, "dfxp" | "ttml" | "tt")
            && self.target.is_some_and(|target| target != "srt"))
        .then(|| source.with_extension("srt"))
    }
}

#[derive(Debug, Clone)]
pub struct ConvertedSubtitle {
    pub path: PathBuf,
    pub ext: String,
    pub bytes: u64,
    pub original: Option<PathBuf>,
    pub intermediates: Vec<PathBuf>,
    pub warnings: Vec<String>,
    pub converted: bool,
}
fn regular(path: &Path) -> Result<fs::Metadata> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(Error::Subtitle(
            "subtitle conversion requires a regular source file".into(),
        ));
    }
    Ok(metadata)
}
fn unchanged(before: &fs::Metadata, after: &fs::Metadata) -> bool {
    let same = before.len() == after.len() && before.modified().ok() == after.modified().ok();
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        same && before.dev() == after.dev() && before.ino() == after.ino()
    }
    #[cfg(not(unix))]
    {
        same
    }
}
fn publish(source: &Path, destination: &Path) -> Result<()> {
    fs::OpenOptions::new()
        .write(true)
        .open(source)?
        .sync_all()?;
    // Upstream's FFmpeg `-y` replaces an existing output.
    tempfile::TempPath::try_from_path(source)?
        .persist(destination)
        .map_err(|error| Error::Io(error.error))?;
    Ok(())
}
fn progress(
    callback: &mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
    bytes: u64,
    phase: DownloadPhase,
) -> ControlFlow<()> {
    callback(DownloadProgress {
        attempt: 1,
        downloaded_bytes: bytes,
        total_bytes: Some(bytes),
        phase,
        stream_index: None,
        stream_count: None,
        fragment_index: None,
        fragment_count: None,
        fragment: None,
    })
}

pub(crate) fn convert(
    downloader: &YoutubeDl,
    source: &Path,
    source_ext: &str,
    options: &SubtitleConversion,
    callback: &mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
) -> Result<ConvertedSubtitle> {
    let path = options.output_path(source, source_ext)?;
    let Some(target) = options
        .target
        .filter(|target| *target != source_ext && source_ext != "json")
    else {
        let metadata = fs::metadata(source)?;
        if !metadata.is_file() {
            return Err(Error::Subtitle(
                "subtitle source is not a regular file".into(),
            ));
        }
        return Ok(ConvertedSubtitle {
            path: source.into(),
            ext: source_ext.into(),
            bytes: metadata.len(),
            original: None,
            intermediates: Vec::new(),
            warnings: if source_ext == "json"
                && options.target.is_some_and(|target| target != source_ext)
            {
                vec!["JSON subtitles cannot be converted".into()]
            } else {
                Vec::new()
            },
            converted: false,
        });
    };
    let metadata = regular(source)?;
    // FFmpeg refuses to write over its input.
    if path == source {
        return Err(Error::Subtitle(
            "subtitle conversion output would replace its input".into(),
        ));
    }
    if metadata.len() > MAX_BYTES {
        return Err(Error::Subtitle(
            "subtitle conversion input exceeds 16 MiB".into(),
        ));
    }
    if progress(callback, metadata.len(), DownloadPhase::Postprocessing).is_break() {
        return Err(Error::DownloadCancelled);
    }
    let mut input = Vec::new();
    fs::File::open(source)?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut input)?;
    if input.len() as u64 > MAX_BYTES {
        return Err(Error::Subtitle(
            "subtitle conversion input exceeds 16 MiB".into(),
        ));
    }
    if !unchanged(&metadata, &regular(source)?) {
        return Err(Error::Subtitle(
            "subtitle source changed during conversion".into(),
        ));
    }
    let parent = source
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let workspace = tempfile::Builder::new()
        .prefix(".yt-dlp-rs-subs-")
        .tempdir_in(parent)?;
    let is_ttml = matches!(source_ext, "dfxp" | "ttml" | "tt");
    let mut warnings = Vec::new();
    let mut intermediate = None;
    let (private_input, input_ext) = if is_ttml {
        warnings.push("TTML conversion can lose style information".into());
        let srt = ttml::to_srt(&input)?;
        let private = workspace.path().join("input.srt");
        fs::write(&private, srt.as_bytes())?;
        if target != "srt" {
            let srt_path = options.intermediate_path(source, source_ext).unwrap();
            intermediate = Some(srt_path);
        }
        (private, "srt")
    } else {
        let private = workspace.path().join("input.sub");
        fs::write(&private, input)?;
        (private, source_ext)
    };
    let output = if is_ttml && target == "srt" {
        private_input.clone()
    } else {
        workspace.path().join(format!("converted.{target}"))
    };
    let process = (|| -> Result<ConvertedSubtitle> {
        if !(is_ttml && target == "srt") {
            let executable = downloader.ffmpeg.as_ref().ok_or_else(|| {
                Error::Postprocess(
                    "subtitle conversion requires an explicit --ffmpeg-location PATH".into(),
                )
            })?;
            let mut observe = || progress(callback, metadata.len(), DownloadPhase::Postprocessing);
            ffmpeg::convert(
                executable,
                &private_input,
                input_ext,
                &output,
                target,
                &mut observe,
            )?;
        }
        if !unchanged(&metadata, &regular(source)?) {
            return Err(Error::Subtitle(
                "subtitle source changed during conversion".into(),
            ));
        }
        let bytes = fs::metadata(&output)?.len();
        if bytes > MAX_BYTES {
            return Err(Error::Subtitle("converted subtitles exceed 16 MiB".into()));
        }
        if progress(callback, bytes, DownloadPhase::Publishing).is_break() {
            return Err(Error::DownloadCancelled);
        }
        if !unchanged(&metadata, &regular(source)?) {
            return Err(Error::Subtitle(
                "subtitle source changed during conversion".into(),
            ));
        }
        let mut intermediates = Vec::new();
        if options.keep_original
            && let Some(path) = &intermediate
        {
            publish(&private_input, path)?;
            intermediates.push(path.clone());
        }
        publish(&output, &path)?;
        if !options.keep_original {
            if !unchanged(&metadata, &regular(source)?) {
                return Err(Error::Subtitle(
                    "converted subtitle published, but source changed; source retained".into(),
                ));
            }
            fs::remove_file(source)?;
        }
        Ok(ConvertedSubtitle {
            path: path.clone(),
            ext: target.into(),
            bytes,
            original: options.keep_original.then(|| source.into()),
            intermediates,
            warnings,
            converted: true,
        })
    })();
    match process {
        Err(Error::DownloadCancelled) => Err(Error::DownloadCancelled),
        Err(error) => {
            if let Some(path) = intermediate
                && private_input.exists()
                && let Err(preserve) = publish(&private_input, &path)
            {
                return Err(Error::Subtitle(format!(
                    "{error}; cannot preserve intermediate subtitle: {preserve}"
                )));
            }
            Err(error)
        }
        result => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saved_source_plans_match_pure_output_names_and_all_target_validation() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../tests/fixtures/subtitle_conversion/cases.json"
        ))
        .unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let conversion =
                SubtitleConversion::new(case["expected"]["target"].as_str().unwrap_or("none"))
                    .unwrap();
            let (Some(sources), Some(outputs)) = (
                case["subtitles"].as_array(),
                case["expected"]["subtitles"].as_array(),
            ) else {
                continue;
            };
            for (source, output) in sources.iter().zip(outputs) {
                // Missing-file skips are filesystem policy, outside this pure
                // filename projection. The CLI acquires sidecars before conversion.
                if source["missing"].as_bool() == Some(true) {
                    continue;
                }
                let path = conversion
                    .output_path(
                        Path::new(source["filepath"].as_str().unwrap()),
                        source["ext"].as_str().unwrap(),
                    )
                    .unwrap();
                assert_eq!(
                    path.to_str().unwrap(),
                    output["filepath"].as_str().unwrap(),
                    "{}",
                    case["name"]
                );
            }
        }
        for invalid in ["", "SRT", "json", "srt/vtt", "vtt>srt", "ass "] {
            assert!(SubtitleConversion::new(invalid).is_err(), "{invalid}");
        }
    }
}

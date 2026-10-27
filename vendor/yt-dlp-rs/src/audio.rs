//! Pure extract-audio mapping, codec/quality plans and local output names.
pub(crate) mod ffmpeg;

use crate::{Error, Result};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

const TARGETS: [&str; 9] = [
    "best", "mp3", "aac", "m4a", "opus", "vorbis", "flac", "alac", "wav",
];
const COMMON: [&str; 10] = [
    "aiff", "alac", "flac", "m4a", "mka", "mp3", "ogg", "opus", "wav", "wma",
];

#[derive(Debug, Clone)]
struct Mapping {
    source: Option<String>,
    target: String,
}

/// Audio conversion preferences. Mapping and quality are validated before any
/// extraction, download or subprocess is started.
#[derive(Debug, Clone)]
pub struct AudioOptions {
    mappings: Vec<Mapping>,
    quality: f64,
    pub keep_video: bool,
}
impl Default for AudioOptions {
    fn default() -> Self {
        Self {
            mappings: vec![Mapping {
                source: None,
                target: "best".into(),
            }],
            quality: 5.0,
            keep_video: false,
        }
    }
}
#[derive(Debug, Clone)]
pub(crate) struct AudioPlan {
    pub extension: String,
    pub encoder: Option<String>,
    pub args: Vec<String>,
    pub muxer: String,
    pub skip: bool,
}
#[derive(Debug, Clone)]
pub struct AudioDownload {
    pub path: PathBuf,
    pub bytes: u64,
    pub kept_video: Option<PathBuf>,
}
fn error(message: impl Into<String>) -> Error {
    Error::Postprocess(message.into())
}

impl AudioOptions {
    /// Mappings are ordered source>target or unconditional target branches.
    /// Quality accepts finite 0..10 VBR values or >10 kbit/s, capped at 1000000.
    pub fn new(format: &str, quality: &str) -> Result<Self> {
        if format.len() > 16 * 1024 || format.split('/').count() > 1024 {
            return Err(error(
                "audio format mapping exceeds 16 KiB or 1024 branches",
            ));
        }
        let mut mappings = Vec::new();
        for branch in format.split('/') {
            let branch = branch.to_lowercase();
            let (source, target) = match branch.split_once('>') {
                Some((source, target)) => (Some(source.trim()), target.trim()),
                None => (None, branch.trim()),
            };
            if target.is_empty() || !TARGETS.contains(&target) {
                return Err(error("unsupported audio target format"));
            }
            if source.is_some_and(|source| {
                source.is_empty()
                    || !source
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            }) {
                return Err(error("audio mapping source must be an ASCII word"));
            }
            mappings.push(Mapping {
                source: source.map(str::to_owned),
                target: target.into(),
            });
        }
        if quality.len() > 128 {
            return Err(error("audio quality exceeds 128 bytes"));
        }
        // Match the pinned CLI's two successive strips, including leading units.
        let quality = quality.trim_matches('k').trim_matches('K').trim();
        let quality = quality
            .parse::<f64>()
            .map_err(|_| error("audio quality must be numeric"))?;
        if !quality.is_finite() || !(0.0..=1_000_000.0).contains(&quality) {
            return Err(error(
                "audio quality must be finite and between 0 and 1000000",
            ));
        }
        Ok(Self {
            mappings,
            quality,
            keep_video: false,
        })
    }
    /// Apply the pinned CLI's lowercase target validation before resolving the
    /// mapping. The library constructor also accepts case-insensitive targets.
    pub fn from_cli(format: &str, quality: &str) -> Result<Self> {
        if format.split('/').any(|branch| {
            branch
                .rsplit_once('>')
                .map_or(branch, |(_, target)| target)
                .trim()
                .bytes()
                .any(|byte| byte.is_ascii_uppercase())
        }) {
            return Err(error("CLI audio target formats must be lowercase"));
        }
        Self::new(format, quality)
    }
    pub(crate) fn target<'a>(&'a self, source_ext: &str) -> Option<&'a str> {
        self.mappings
            .iter()
            .find(|mapping| {
                mapping
                    .source
                    .as_deref()
                    .is_none_or(|source| source == source_ext)
            })
            .map(|mapping| mapping.target.as_str())
    }
    pub(crate) fn needs_probe(&self, source_ext: &str) -> bool {
        self.target(source_ext)
            .is_some_and(|target| target != "best" || !COMMON.contains(&source_ext))
    }
    pub(crate) fn plan(&self, source_ext: &str, codec: Option<&str>) -> Result<AudioPlan> {
        if !self.needs_probe(source_ext) {
            return Ok(AudioPlan {
                extension: source_ext.into(),
                encoder: None,
                args: Vec::new(),
                muxer: String::new(),
                skip: true,
            });
        }
        let target = self.target(source_ext).unwrap();
        let codec = codec
            .filter(|codec| !codec.is_empty())
            .ok_or_else(|| error("unable to obtain file audio codec with ffprobe"))?;
        let (format, copy) = if codec == "aac" && matches!(target, "m4a" | "best") {
            ("m4a", true)
        } else if target == "best" || target == codec {
            if TARGETS.contains(&codec) && codec != "best" {
                (codec, true)
            } else {
                ("mp3", false)
            }
        } else {
            (target, false)
        };
        let (extension, encoder, muxer) = match format {
            "mp3" => ("mp3", Some("libmp3lame"), "mp3"),
            "aac" => ("m4a", Some("aac"), if copy { "adts" } else { "ipod" }),
            "m4a" => ("m4a", Some("aac"), "ipod"),
            "opus" => ("opus", Some("libopus"), "opus"),
            "vorbis" => ("ogg", Some("libvorbis"), "ogg"),
            "flac" => ("flac", Some("flac"), "flac"),
            // Native correction: upstream's overwritten opts can accidentally
            // encode AAC instead of ALAC, or override an intended copy.
            "alac" => ("m4a", Some("alac"), "ipod"),
            "wav" => ("wav", None, "wav"),
            _ => return Err(error("unsupported audio codec plan")),
        };
        let args = if copy {
            if format == "m4a" {
                vec!["-bsf:a".into(), "aac_adtstoasc".into()]
            } else {
                Vec::new()
            }
        } else {
            self.quality_args(encoder)
        };
        Ok(AudioPlan {
            extension: extension.into(),
            encoder: if copy {
                Some("copy".into())
            } else {
                encoder.map(str::to_owned)
            },
            args,
            muxer: muxer.into(),
            skip: false,
        })
    }
    fn quality_args(&self, encoder: Option<&str>) -> Vec<String> {
        if self.quality > 10.0 {
            return vec!["-b:a".into(), format!("{}k", python_float(self.quality))];
        }
        let (low, high) = match encoder {
            Some("libmp3lame") => (10.0, 0.0),
            Some("libvorbis") => (0.0, 10.0),
            Some("aac") => (0.1, 4.0),
            _ => return Vec::new(),
        };
        let quality = high + (low - high) * (self.quality / 10.0);
        vec!["-q:a".into(), python_float(quality)]
    }
}
fn python_float(number: f64) -> String {
    let value = format!("{number:?}");
    match value.split_once('e') {
        Some((mantissa, exponent)) => {
            let exponent = exponent.parse::<i32>().unwrap();
            format!(
                "{mantissa}e{}{:02}",
                if exponent < 0 { "-" } else { "+" },
                exponent.abs()
            )
        }
        None => value,
    }
}
fn valid_extension(extension: &str) -> bool {
    !extension.is_empty()
        && extension.len() <= 32
        && extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
}
fn valid_media(media: &Path) -> Result<()> {
    if media.file_name().is_none() {
        return Err(error("audio media path must name a file"));
    }
    Ok(())
}
/// Replace only a matching source suffix; literal nonmatching suffixes are kept
/// and the audio extension is appended. No metadata can add path components.
pub fn audio_output_path(media: &Path, source_ext: &str, output_ext: &str) -> Result<PathBuf> {
    valid_media(media)?;
    if !valid_extension(source_ext) || !valid_extension(output_ext) {
        return Err(error(
            "audio extensions must be 1..32 ASCII letters or digits",
        ));
    }
    let mut path = media.to_path_buf();
    if path
        .extension()
        .is_some_and(|extension| extension == source_ext)
    {
        path.set_extension(output_ext);
        return Ok(path);
    }
    let mut path = path.into_os_string();
    path.push(".");
    path.push(output_ext);
    Ok(path.into())
}
/// Insert .orig before the last suffix, preserving literal non-UTF-8 path bytes.
pub fn audio_original_path(media: &Path) -> Result<PathBuf> {
    valid_media(media)?;
    let stem = media
        .file_stem()
        .ok_or_else(|| error("audio media path must have a file stem"))?;
    let mut name = OsString::from(stem);
    name.push(".orig");
    if let Some(extension) = media.extension() {
        name.push(".");
        name.push(extension);
    }
    Ok(media.with_file_name(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn mapping_order_case_trim_and_all_branches_validate_before_selection() {
        let options = AudioOptions::new(" WEBM > OPUS / MP4 > M4A / mp3 ", "5").unwrap();
        assert_eq!(options.target("webm"), Some("opus"));
        assert_eq!(options.target("mp4"), Some("m4a"));
        assert_eq!(options.target("bin"), Some("mp3"));
        assert_eq!(
            AudioOptions::new("webm>opus", "5").unwrap().target("mp4"),
            None
        );
        for mapping in [
            "",
            "mp3/",
            "/mp3",
            "mp4>>mp3",
            ">mp3",
            "mp3/unsupported",
            "mp3/☃>opus",
        ] {
            assert!(AudioOptions::new(mapping, "5").is_err(), "{mapping}");
        }
        assert!(AudioOptions::new(&vec!["mp3"; 1024].join("/"), "5").is_ok());
        assert!(AudioOptions::new(&vec!["mp3"; 1025].join("/"), "5").is_err());
        assert!(AudioOptions::new(&"x".repeat(16 * 1024 + 1), "5").is_err());
        assert!(AudioOptions::new("MP3", "5").is_ok());
        assert!(AudioOptions::from_cli("MP3", "5").is_err());
        assert!(AudioOptions::from_cli("WebM > mp3", "5").is_ok());
    }
    #[test]
    fn common_audio_and_unmapped_rules_skip_before_probe_but_aac_requires_probe() {
        let options = AudioOptions::default();
        for extension in COMMON {
            assert!(!options.needs_probe(extension));
            assert!(options.plan(extension, None).unwrap().skip);
        }
        assert!(options.needs_probe("aac"));
        assert!(options.plan("aac", None).is_err());
        let unmapped = AudioOptions::new("webm>opus", "5").unwrap();
        assert!(!unmapped.needs_probe("mp4"));
        assert!(unmapped.plan("mp4", None).unwrap().skip);
        let explicit = AudioOptions::new("mp3", "5").unwrap();
        assert!(explicit.needs_probe("mp3"));
        let plan = explicit.plan("mp3", Some("mp3")).unwrap();
        assert!(!plan.skip);
        assert_eq!(plan.encoder.as_deref(), Some("copy"));
    }
    #[test]
    fn aac_container_options_and_native_alac_fix_are_explicit() {
        let copy = AudioOptions::new("aac", "5")
            .unwrap()
            .plan("mp4", Some("aac"))
            .unwrap();
        assert_eq!(
            (&*copy.extension, &*copy.muxer, copy.encoder.as_deref()),
            ("m4a", "adts", Some("copy"))
        );
        assert!(copy.args.is_empty());
        let transcode = AudioOptions::new("aac", "5")
            .unwrap()
            .plan("mp4", Some("opus"))
            .unwrap();
        assert_eq!(&*transcode.muxer, "ipod");
        assert_eq!(transcode.args, vec!["-q:a", "2.05"]);
        let copy = AudioOptions::default().plan("aac", Some("aac")).unwrap();
        assert_eq!(copy.args, vec!["-bsf:a", "aac_adtstoasc"]);
        assert_eq!(&*copy.muxer, "ipod");
        let alac = AudioOptions::new("alac", "5")
            .unwrap()
            .plan("mp4", Some("aac"))
            .unwrap();
        assert_eq!(alac.encoder.as_deref(), Some("alac"));
        assert!(alac.args.is_empty());
        let alac = AudioOptions::new("alac", "5")
            .unwrap()
            .plan("mp4", Some("alac"))
            .unwrap();
        assert_eq!(alac.encoder.as_deref(), Some("copy"));
        assert!(alac.args.is_empty());
    }
    #[test]
    fn quality_curves_bitrate_threshold_strip_semantics_and_invalid_values() {
        for (format, quality, expected) in [
            ("mp3", "0", "0.0"),
            ("mp3", "10", "10.0"),
            ("vorbis", "0", "10.0"),
            ("vorbis", "10", "0.0"),
            ("m4a", "0", "4.0"),
            ("m4a", "10", "0.10000000000000009"),
        ] {
            assert_eq!(
                AudioOptions::new(format, quality)
                    .unwrap()
                    .plan("mp4", Some("unknown"))
                    .unwrap()
                    .args,
                vec!["-q:a", expected]
            );
        }
        for quality in ["128k", "128K", "k128K", "kkKK128KKkk"] {
            assert_eq!(
                AudioOptions::new("opus", quality)
                    .unwrap()
                    .plan("mp4", Some("aac"))
                    .unwrap()
                    .args,
                vec!["-b:a", "128.0k"]
            );
        }
        assert_eq!(
            AudioOptions::new("opus", "10.5")
                .unwrap()
                .plan("mp4", Some("aac"))
                .unwrap()
                .args,
            vec!["-b:a", "10.5k"]
        );
        for quality in [
            "",
            "bad",
            "inf",
            "NaN",
            "-1",
            "-0.1",
            "1000000.1",
            "1e100",
            "Kk128kK",
        ] {
            assert!(AudioOptions::new("mp3", quality).is_err(), "{quality}");
        }
        assert!(AudioOptions::new("mp3", &"0".repeat(129)).is_err());
        assert_eq!(python_float(1e-7), "1e-07");
        assert_eq!(python_float(1e16), "1e+16");
    }
    #[test]
    fn paths_replace_only_matching_suffix_and_keep_original_name_before_suffix() {
        for (media, source, target, expected) in [
            ("directory/movie.mp4", "mp4", "mp3", "directory/movie.mp3"),
            ("movie.final", "mp4", "mp3", "movie.final.mp3"),
            ("movie", "mp4", "mp3", "movie.mp3"),
            ("movie.MP4", "mp4", "mp3", "movie.MP4.mp3"),
        ] {
            assert_eq!(
                audio_output_path(Path::new(media), source, target).unwrap(),
                PathBuf::from(expected)
            );
        }
        for (media, expected) in [
            ("directory/movie.mp4", "directory/movie.orig.mp4"),
            ("movie", "movie.orig"),
            ("movie.", "movie.orig."),
            (".hidden", ".hidden.orig"),
            (".hidden.mp4", ".hidden.orig.mp4"),
        ] {
            assert_eq!(
                audio_original_path(Path::new(media)).unwrap(),
                PathBuf::from(expected)
            );
        }
        for extension in ["", "../mp3", "a/b", "a\\b", "mp3\n", ".mp3"] {
            assert!(audio_output_path(Path::new("movie.mp4"), "mp4", extension).is_err());
        }
        assert!(audio_output_path(Path::new(""), "mp4", "mp3").is_err());
        assert!(audio_original_path(Path::new("/")).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn non_utf8_literal_paths_preserve_every_byte() {
        use std::os::unix::ffi::OsStringExt;
        let path = PathBuf::from(OsString::from_vec(b"literal/\xff.mp4".to_vec()));
        assert_eq!(
            audio_output_path(&path, "mp4", "mp3").unwrap(),
            PathBuf::from(OsString::from_vec(b"literal/\xff.mp3".to_vec()))
        );
        assert_eq!(
            audio_original_path(&path).unwrap(),
            PathBuf::from(OsString::from_vec(b"literal/\xff.orig.mp4".to_vec()))
        );
    }
    #[test]
    fn saved_pinned_audio_plans_and_quality_values_match() {
        let fixture: Value =
            serde_json::from_str(include_str!("../tests/fixtures/audio/cases.json")).unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let arguments = case["args"].as_array().unwrap();
            let option = |key: &str, default: &'static str| {
                arguments
                    .windows(2)
                    .find(|pair| pair[0].as_str() == Some(key))
                    .and_then(|pair| pair[1].as_str())
                    .unwrap_or(default)
            };
            let options = AudioOptions::new(
                option("--audio-format", "best"),
                option("--audio-quality", "5"),
            )
            .unwrap();
            let source = case["source"]["ext"].as_str().unwrap();
            let source_path = Path::new(case["source"]["filepath"].as_str().unwrap());
            let plan = options
                .plan(source, case["source"]["codec"].as_str())
                .unwrap();
            assert_eq!(
                options.needs_probe(source),
                !case["expected"]["probes"].as_array().unwrap().is_empty(),
                "{}",
                case["name"]
            );
            assert_eq!(
                plan.extension,
                case["expected"]["output_ext"].as_str().unwrap(),
                "{}",
                case["name"]
            );
            let output = if plan.skip {
                source_path.to_owned()
            } else {
                audio_output_path(source_path, source, &plan.extension).unwrap()
            };
            assert_eq!(
                output,
                PathBuf::from(case["expected"]["output_path"].as_str().unwrap()),
                "{}",
                case["name"]
            );
            let calls = case["expected"]["calls"].as_array().unwrap();
            let Some(call) = calls.first() else {
                assert!(
                    plan.skip || (plan.encoder.as_deref() == Some("copy") && output == source_path),
                    "{}",
                    case["name"]
                );
                continue;
            };
            assert_eq!(
                plan.encoder.as_deref(),
                call["codec"].as_str(),
                "{}",
                case["name"]
            );
            let args: Vec<String> = serde_json::from_value(call["more_opts"].clone()).unwrap();
            let args: Vec<_> = args
                .chunks(2)
                .filter(|pair| pair[0] != "-f")
                .flatten()
                .cloned()
                .collect();
            if case["expected"]["quality_args"].is_object() {
                let quality = &case["expected"]["quality_args"];
                assert_eq!(
                    plan.args[0],
                    quality["flag"].as_str().unwrap(),
                    "{}",
                    case["name"]
                );
                let number = plan.args[1]
                    .strip_suffix('k')
                    .unwrap_or(&plan.args[1])
                    .parse::<f64>()
                    .unwrap();
                // Parse the exact source argv string: serde_json's default
                // numeric parser may round this boundary decimal differently.
                let expected_number = args[1]
                    .strip_suffix('k')
                    .unwrap_or(&args[1])
                    .parse::<f64>()
                    .unwrap();
                assert_eq!(number, expected_number, "{}", case["name"]);
                assert_eq!(plan.args, args, "{}", case["name"]);
                assert_eq!(
                    plan.args[1].ends_with('k'),
                    quality["unit"].as_str() == Some("k"),
                    "{}",
                    case["name"]
                );
            } else {
                assert_eq!(plan.args, args, "{}", case["name"]);
            }
        }
    }
}

//! Pure pinned video remux/recode mapping and stream options.
pub(crate) mod ffmpeg;

use crate::{Error, Result};
const TARGETS: [&str; 18] = [
    "avi", "flv", "gif", "mkv", "mov", "mp4", "webm", "aac", "aiff", "alac", "flac", "m4a", "mka",
    "mp3", "ogg", "opus", "vorbis", "wav",
];
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoMode {
    Remux,
    Recode,
}
#[derive(Debug, Clone)]
struct Mapping {
    source: Option<String>,
    target: String,
}
#[derive(Debug, Clone)]
pub struct VideoOptions {
    mappings: Vec<Mapping>,
    mode: VideoMode,
}
#[derive(Debug, Clone)]
pub(crate) struct VideoPlan {
    pub extension: String,
    pub args: Vec<String>,
    pub muxer: String,
    pub skip: bool,
    /// A second, FFMETADATA input (upstream's chapter metadata file).
    pub metadata_input: Option<std::path::PathBuf>,
    /// The postprocessor key for `--postprocessor-args`.
    pub pp: &'static str,
}
fn error(message: &str) -> Error {
    Error::Postprocess(message.into())
}
impl VideoOptions {
    /// Ordered source>target mappings, or an unconditional target. Library
    /// mapping names are case insensitive; source conditions are ASCII words.
    pub fn new(format: &str, mode: VideoMode) -> Result<Self> {
        if format.len() > 16 * 1024 || format.split('/').count() > 1024 {
            return Err(error("video mapping exceeds 16 KiB or 1024 branches"));
        }
        let mut mappings = Vec::new();
        for branch in format.split('/') {
            let branch = branch.to_lowercase();
            let (source, target) = branch
                .split_once('>')
                .map_or((None, branch.trim()), |(source, target)| {
                    (Some(source.trim()), target.trim())
                });
            if !TARGETS.contains(&target) {
                return Err(error("unsupported video target format"));
            }
            if source.is_some_and(|source| {
                source.is_empty()
                    || !source
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            }) {
                return Err(error("video mapping source must be an ASCII word"));
            }
            mappings.push(Mapping {
                source: source.map(str::to_owned),
                target: target.into(),
            });
        }
        Ok(Self { mappings, mode })
    }
    /// Match the pinned CLI's lowercase target validation before applying the
    /// library resolver. Uppercase conditional source words remain accepted.
    pub fn from_cli(format: &str, mode: VideoMode) -> Result<Self> {
        if format.split('/').any(|branch| {
            branch
                .rsplit_once('>')
                .map_or(branch, |(_, target)| target)
                .trim()
                .bytes()
                .any(|byte| byte.is_ascii_uppercase())
        }) {
            return Err(error("CLI video target formats must be lowercase"));
        }
        Self::new(format, mode)
    }
    /// Upstream `_ACTION` (`remuxing`/`converting`) and its PP name.
    pub(crate) fn action(&self) -> (&'static str, &'static str) {
        match self.mode {
            VideoMode::Remux => ("remuxing", "VideoRemuxer"),
            VideoMode::Recode => ("converting", "VideoConvertor"),
        }
    }

    /// Upstream `resolve_mapping` skip reason for `source_ext`, if any.
    pub(crate) fn skip_reason(&self, source_ext: &str) -> Option<String> {
        let source = source_ext.to_lowercase();
        match self
            .mappings
            .iter()
            .find(|m| m.source.as_deref().is_none_or(|c| c == source))
        {
            None => Some(format!("could not find a mapping for {source}")),
            Some(mapping) if mapping.target == source => {
                Some(format!("already is in target format {source}"))
            }
            Some(_) => None,
        }
    }

    pub(crate) fn plan(&self, source_ext: &str) -> Result<VideoPlan> {
        let source = source_ext.to_lowercase();
        let target = self
            .mappings
            .iter()
            .find(|mapping| {
                mapping
                    .source
                    .as_deref()
                    .is_none_or(|condition| condition == source)
            })
            .map(|mapping| mapping.target.as_str());
        let pp = match self.mode {
            VideoMode::Remux => "VideoRemuxer",
            VideoMode::Recode => "VideoConvertor",
        };
        let Some(target) = target.filter(|target| *target != source) else {
            return Ok(VideoPlan {
                extension: source_ext.into(),
                args: Vec::new(),
                muxer: String::new(),
                skip: true,
                metadata_input: None,
                pp,
            });
        };
        let mut args = vec![
            "-map".into(),
            "0".into(),
            "-dn".into(),
            "-ignore_unknown".into(),
        ];
        match self.mode {
            VideoMode::Remux => args.extend(["-c".into(), "copy".into()]),
            VideoMode::Recode if target == "avi" => args.extend([
                "-c:v".into(),
                "libxvid".into(),
                "-vtag".into(),
                "XVID".into(),
            ]),
            VideoMode::Recode => {}
        }
        let muxer = match target {
            "mkv" | "mka" => "matroska",
            "m4a" => "ipod",
            "aac" => "adts",
            "alac" | "vorbis" => "",
            target => target,
        };
        Ok(VideoPlan {
            extension: target.into(),
            args,
            muxer: muxer.into(),
            skip: false,
            metadata_input: None,
            pp,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    #[test]
    fn mappings_normalize_source_case_preserve_skip_metadata_and_validate_every_branch() {
        let options = VideoOptions::new(" WEBM > MP4 / mkv / mp3 ", VideoMode::Remux).unwrap();
        assert_eq!(options.plan("WEBM").unwrap().extension, "mp4");
        assert_eq!(options.plan("flv").unwrap().extension, "mkv");
        let same = VideoOptions::new("WEBM", VideoMode::Recode)
            .unwrap()
            .plan("WEBM")
            .unwrap();
        assert!(same.skip);
        assert_eq!(same.extension, "WEBM");
        let unmapped = VideoOptions::new("mkv>mp4", VideoMode::Remux)
            .unwrap()
            .plan("WEBM")
            .unwrap();
        assert!(unmapped.skip);
        assert_eq!(unmapped.extension, "WEBM");
        for mapping in [
            "",
            "mp4/",
            "webm>>mp4",
            ">mp4",
            "mp4/unknown",
            "mp4/a-b>mkv",
            "☃>mp4",
        ] {
            assert!(
                VideoOptions::new(mapping, VideoMode::Remux).is_err(),
                "{mapping}"
            );
        }
        assert!(VideoOptions::from_cli("MP4", VideoMode::Remux).is_err());
        assert!(VideoOptions::from_cli("WEBM > mp4", VideoMode::Remux).is_ok());
        assert!(VideoOptions::new(&vec!["mp4"; 1024].join("/"), VideoMode::Remux).is_ok());
        assert!(VideoOptions::new(&vec!["mp4"; 1025].join("/"), VideoMode::Remux).is_err());
        assert!(VideoOptions::new(&"x".repeat(16 * 1024 + 1), VideoMode::Remux).is_err());
    }
    #[test]
    fn plans_preserve_source_copy_defaults_avi_override_and_unsupported_container_targets() {
        for target in TARGETS {
            let remux = VideoOptions::new(target, VideoMode::Remux)
                .unwrap()
                .plan("bin")
                .unwrap();
            assert_eq!(
                remux.args,
                vec!["-map", "0", "-dn", "-ignore_unknown", "-c", "copy"]
            );
            let recode = VideoOptions::new(target, VideoMode::Recode)
                .unwrap()
                .plan("bin")
                .unwrap();
            assert!(!recode.args.iter().any(|arg| arg == "-c:s"));
            if target == "avi" {
                assert_eq!(
                    recode.args,
                    vec![
                        "-map",
                        "0",
                        "-dn",
                        "-ignore_unknown",
                        "-c:v",
                        "libxvid",
                        "-vtag",
                        "XVID"
                    ]
                );
            } else {
                assert_eq!(recode.args, vec!["-map", "0", "-dn", "-ignore_unknown"]);
            }
            assert_eq!(recode.muxer.is_empty(), matches!(target, "alac" | "vorbis"));
        }
    }
    #[test]
    fn saved_pinned_video_mapping_stream_options_and_output_paths_match() {
        let fixture: Value =
            serde_json::from_str(include_str!("../tests/fixtures/video/cases.json")).unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let expected = &case["expected"];
            let mode = match expected["mode"].as_str().unwrap() {
                "remux" => VideoMode::Remux,
                "recode" => VideoMode::Recode,
                _ => panic!(),
            };
            let options =
                VideoOptions::from_cli(expected["mapping"].as_str().unwrap(), mode).unwrap();
            let source = case["source"]["ext"].as_str().unwrap();
            let path = std::path::Path::new(case["source"]["filepath"].as_str().unwrap());
            let plan = options.plan(source).unwrap();
            let calls = expected["calls"].as_array().unwrap();
            assert_eq!(plan.skip, calls.is_empty(), "{}", case["name"]);
            assert_eq!(
                plan.extension,
                expected["output_ext"].as_str().unwrap(),
                "{}",
                case["name"]
            );
            let output = if plan.skip {
                path.to_owned()
            } else {
                crate::audio_output_path(path, &source.to_lowercase(), &plan.extension).unwrap()
            };
            assert_eq!(
                output,
                std::path::PathBuf::from(expected["output_path"].as_str().unwrap()),
                "{}",
                case["name"]
            );
            if let Some(call) = calls.first() {
                let args: Vec<String> = serde_json::from_value(call["args"].clone()).unwrap();
                assert_eq!(plan.args, args, "{}", case["name"]);
            }
        }
        for case in fixture["invalid_cases"].as_array().unwrap() {
            let args = case["args"].as_array().unwrap();
            for pair in args.windows(2) {
                if matches!(pair[0].as_str(), Some("--remux-video" | "--recode-video")) {
                    assert!(
                        VideoOptions::from_cli(pair[1].as_str().unwrap(), VideoMode::Remux)
                            .is_err(),
                        "{}",
                        case["name"]
                    );
                }
            }
        }
    }
}

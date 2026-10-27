//! Subtitle-embedding metadata plans. Publication and cleanup belong to orchestration.
pub(crate) mod ffmpeg;
mod languages;
use crate::{Error, Result};
pub(crate) use languages::short_to_long;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct SubtitleFile {
    pub path: PathBuf,
    pub ext: String,
    pub language: String,
    pub name: Option<String>,
}
#[derive(Debug, Clone, Default)]
pub struct SubtitleEmbedding {
    pub subtitles: Vec<SubtitleFile>,
    pub keep_subtitles: bool,
}
#[derive(Debug, Clone)]
pub(crate) struct EmbedPlan {
    pub files: Vec<SubtitleFile>,
    pub args: Vec<String>,
    pub muxer: String,
    pub warnings: Vec<String>,
    pub messages: Vec<String>,
    pub skip: bool,
}
fn error(message: &str) -> Error {
    Error::Postprocess(message.into())
}

pub(crate) fn plan(container: &str, files: &[SubtitleFile]) -> Result<EmbedPlan> {
    let muxer = match container {
        "mp4" => "mp4",
        "mov" => "mov",
        "m4a" => "ipod",
        "webm" => "webm",
        "mkv" | "mka" => "matroska",
        _ => "",
    };
    let mut result = EmbedPlan {
        files: Vec::new(),
        args: Vec::new(),
        muxer: muxer.into(),
        warnings: Vec::new(),
        messages: Vec::new(),
        skip: true,
    };
    if muxer.is_empty() {
        result
            .messages
            .push("Subtitles can only be embedded in mp4, mov, m4a, webm, mkv, mka files".into());
        return Ok(result);
    }
    if files.is_empty() {
        result
            .messages
            .push("There aren't any subtitles to embed".into());
        return Ok(result);
    }
    if files.len() > 1000 {
        return Err(error("subtitle embedding exceeds 1000 tracks"));
    }
    let mut webm_warn = false;
    let mut ass_warn = false;
    for file in files {
        if file.language.len() > 128
            || file.language.chars().any(char::is_control)
            || file
                .name
                .as_ref()
                .is_some_and(|name| name.len() > 4096 || name.chars().any(char::is_control))
        {
            return Err(error(
                "subtitle embedding metadata exceeds limits or contains controls",
            ));
        }
        if file.ext.is_empty()
            || file.ext.len() > 32
            || !file.ext.bytes().all(|byte| byte.is_ascii_alphanumeric())
        {
            return Err(error(
                "subtitle embedding extension must be 1..32 ASCII letters or digits",
            ));
        }
        match std::fs::metadata(&file.path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                result.warnings.push(format!(
                    "Skipping embedding {} subtitle because the file is missing",
                    file.language
                ));
                continue;
            }
            Err(_) => return Err(error("cannot inspect subtitle embedding input")),
            Ok(metadata) if !metadata.is_file() => {
                return Err(error("subtitle embedding input must be a regular file"));
            }
            Ok(_) => {}
        }
        if file.ext == "json" {
            result
                .warnings
                .push("JSON subtitles cannot be embedded".into());
        } else if container != "webm" || file.ext == "vtt" {
            result.files.push(file.clone());
        } else if !webm_warn {
            webm_warn = true;
            result
                .warnings
                .push("Only WebVTT subtitles can be embedded in webm files".into());
        }
        if !ass_warn && container == "mp4" && file.ext == "ass" {
            ass_warn = true;
            result.warnings.push(
                "ASS subtitles cannot be properly embedded in mp4 files; expect issues".into(),
            );
        }
    }
    if result.files.is_empty() {
        return Ok(result);
    }
    result.skip = false;
    result.args = ["-map", "0", "-dn", "-ignore_unknown", "-c", "copy"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    if matches!(container, "mp4" | "mov" | "m4a") {
        result.args.extend(["-c:s".into(), "mov_text".into()]);
    }
    result.args.extend(["-map".into(), "-0:s".into()]);
    for (index, file) in result.files.iter().enumerate() {
        result.args.extend([
            "-map".into(),
            format!("{}:0", index + 1),
            format!("-metadata:s:s:{index}"),
            format!(
                "language={}",
                languages::short_to_long(&file.language).unwrap_or(&file.language)
            ),
        ]);
        if let Some(name) = file.name.as_deref().filter(|name| !name.is_empty()) {
            result.args.extend([
                format!("-metadata:s:s:{index}"),
                format!("handler_name={name}"),
                format!("-metadata:s:s:{index}"),
                format!("title={name}"),
            ]);
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    fn subtitle(
        directory: &std::path::Path,
        ext: &str,
        language: &str,
        name: Option<&str>,
    ) -> SubtitleFile {
        let path = directory.join(format!("track.{language}.{ext}"));
        std::fs::write(&path, b"subtitle content").unwrap();
        SubtitleFile {
            path,
            ext: ext.into(),
            language: language.into(),
            name: name.map(str::to_owned),
        }
    }
    #[test]
    fn plans_preserve_warnings_filter_order_and_source_metadata() {
        let directory = tempfile::tempdir().unwrap();
        let vtt = subtitle(directory.path(), "vtt", "en-US", Some("English"));
        let ass = subtitle(directory.path(), "ass", "fr", Some("French"));
        let json = subtitle(directory.path(), "json", "de", None);
        let files = vec![vtt.clone(), ass.clone(), ass.clone(), json.clone()];
        let mp4 = plan("mp4", &files).unwrap();
        assert_eq!(mp4.files.len(), 3);
        assert_eq!(mp4.warnings.len(), 2);
        assert_eq!(
            mp4.args[0..10],
            [
                "-map",
                "0",
                "-dn",
                "-ignore_unknown",
                "-c",
                "copy",
                "-c:s",
                "mov_text",
                "-map",
                "-0:s"
            ]
        );
        assert!(mp4.args.contains(&"language=eng".into()));
        assert!(mp4.args.contains(&"handler_name=English".into()));
        assert!(mp4.args.contains(&"title=English".into()));
        let webm = plan("webm", &files).unwrap();
        assert_eq!(webm.files.len(), 1);
        assert_eq!(webm.warnings.len(), 2);
        assert!(!webm.args.contains(&"mov_text".into()));
        assert!(plan("MP4", &files).unwrap().skip);
        assert!(!plan("mkv", &files).unwrap().skip);
        assert!(plan("mp4", &[]).unwrap().messages[0].contains("aren't any"));
    }
    #[test]
    fn unavailable_inputs_unsafe_metadata_and_native_bounds_are_explicit() {
        let directory = tempfile::tempdir().unwrap();
        let mut file = subtitle(directory.path(), "vtt", "en", None);
        std::fs::remove_file(&file.path).unwrap();
        let missing = plan("mp4", &[file.clone()]).unwrap();
        assert!(missing.skip);
        assert_eq!(missing.warnings.len(), 1);
        file.path = directory.path().into();
        assert!(plan("mp4", &[file.clone()]).is_err());
        file = subtitle(directory.path(), "vtt", "en", None);
        file.name = Some("x\n".into());
        assert!(plan("mp4", &[file.clone()]).is_err());
        file.name = Some("x".repeat(4097));
        assert!(plan("mp4", &[file.clone()]).is_err());
        file.name = None;
        file.language = "x".repeat(129);
        assert!(plan("mp4", &[file.clone()]).is_err());
        file.language = "en".into();
        file.ext = "../vtt".into();
        assert!(plan("mp4", &[file.clone()]).is_err());
        file.ext = "vtt".into();
        assert!(plan("mp4", &vec![file; 1001]).is_err());
        assert!(plan("avi", &[]).unwrap().skip);
    }
    #[test]
    fn saved_pinned_plans_and_all_iso639_language_projections_match() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/subtitle_embedding/cases.json"
        ))
        .unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            if !case["expected"]["enabled"].as_bool().unwrap() {
                continue;
            }
            let directory = tempfile::tempdir().unwrap();
            let mut files = Vec::new();
            for file in case["subtitles"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or(&[])
            {
                let path = directory.path().join(file["filepath"].as_str().unwrap());
                if !file["missing"].as_bool().unwrap_or(false) {
                    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                    std::fs::write(&path, b"subtitle").unwrap();
                }
                files.push(SubtitleFile {
                    path,
                    ext: file["ext"].as_str().unwrap().into(),
                    language: file["language"].as_str().unwrap().into(),
                    name: file["name"].as_str().map(str::to_owned),
                });
            }
            let plan = plan(case["source"]["ext"].as_str().unwrap(), &files).unwrap();
            let expected = &case["expected"];
            let calls = expected["calls"].as_array().unwrap();
            assert_eq!(plan.skip, calls.is_empty(), "{}", case["name"]);
            assert_eq!(
                plan.warnings,
                serde_json::from_value::<Vec<String>>(expected["warnings"].clone()).unwrap(),
                "{}",
                case["name"]
            );
            if let Some(call) = calls.first() {
                assert_eq!(
                    plan.args,
                    serde_json::from_value::<Vec<String>>(call["args"].clone()).unwrap(),
                    "{}",
                    case["name"]
                );
                let expected_inputs = call["inputs"].as_array().unwrap();
                assert_eq!(plan.files.len(), expected_inputs.len() - 1);
                for (file, expected) in plan.files.iter().zip(expected_inputs.iter().skip(1)) {
                    assert_eq!(
                        file.path.strip_prefix(directory.path()).unwrap(),
                        std::path::Path::new(expected.as_str().unwrap())
                    );
                }
            }
        }
        for case in fixture["language_cases"].as_array().unwrap() {
            let input = case["input"].as_str().unwrap();
            assert_eq!(
                languages::short_to_long(input).unwrap_or(input),
                case["expected"].as_str().unwrap()
            );
        }
    }
}

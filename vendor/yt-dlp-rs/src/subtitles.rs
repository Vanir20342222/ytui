//! Ordered subtitle metadata, pure language/format selection and sidecar paths.
use crate::{Error, Result, VideoInfo};
use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{MapAccess, Visitor},
    ser::SerializeMap,
};
use std::{
    collections::{HashMap, HashSet},
    fmt,
    marker::PhantomData,
    path::{Path, PathBuf},
};
use url::Url;

/// An insertion-ordered subtitle object. JSON represents an unknown language as
/// the key "null"; deserializing JSON cannot distinguish it from that literal tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubtitleMap<T> {
    pub entries: Vec<(Option<String>, T)>,
}
impl<T> Default for SubtitleMap<T> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
}
impl<T> SubtitleMap<T> {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    /// Replace an existing value without changing the original declaration order.
    pub fn insert(&mut self, language: Option<String>, value: T) -> Option<T> {
        if let Some((_, current)) = self.entries.iter_mut().find(|(key, _)| key == &language) {
            return Some(std::mem::replace(current, value));
        }
        self.entries.push((language, value));
        None
    }
}
impl<T: Serialize> Serialize for SubtitleMap<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.entries.len()))?;
        for (key, value) in &self.entries {
            map.serialize_entry(key.as_deref().unwrap_or("null"), value)?;
        }
        map.end()
    }
}
impl<'de, T: Deserialize<'de>> Deserialize<'de> for SubtitleMap<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct Ordered<T>(PhantomData<T>);
        impl<'de, T: Deserialize<'de>> Visitor<'de> for Ordered<T> {
            type Value = SubtitleMap<T>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an ordered subtitle object")
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                mut map: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut result = SubtitleMap::default();
                while let Some((key, value)) = map.next_entry::<String, T>()? {
                    result.insert(Some(key), value);
                }
                Ok(result)
            }
        }
        deserializer.deserialize_map(Ordered(PhantomData))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubtitleFormat {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub url: Url,
    pub ext: String,
    /// `m3u8_native` for an HLS subtitle rendition (downloaded by HlsFD) or
    /// `http_dash_segments` for a segmented DASH text track.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    /// A segmented DASH text track's fragments.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fragments: Vec<crate::Fragment>,
    /// Keys this port does not model (a DASH track's `manifest_url`).
    #[serde(flatten, default)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}
pub type Subtitles = SubtitleMap<Vec<SubtitleFormat>>;
pub type RequestedSubtitles = SubtitleMap<SubtitleFormat>;

#[derive(Debug, Clone)]
pub struct SubtitleOptions {
    pub languages: Option<String>,
    pub format: String,
}
impl Default for SubtitleOptions {
    fn default() -> Self {
        Self {
            languages: None,
            format: "best".into(),
        }
    }
}
#[derive(Debug, Clone)]
pub struct SubtitleSelection {
    pub requested: RequestedSubtitles,
    pub warnings: Vec<String>,
}
fn error(message: impl Into<String>) -> Error {
    Error::Subtitle(message.into())
}

impl SubtitleOptions {
    /// Language expressions support comma-separated literal tags, all and exclusions.
    /// Regular expressions are deliberately unsupported. Formats are ordered slash
    /// preferences; an empty query remains a valid request for the fallback format.
    pub fn new(languages: Option<&str>, format: &str) -> Result<Self> {
        validate_options(languages, format)?;
        Ok(Self {
            languages: languages.map(str::to_owned),
            format: format.into(),
        })
    }
    pub fn select(&self, subtitles: &Subtitles) -> Result<SubtitleSelection> {
        self.select_sources(subtitles, &Subtitles::default(), true, false)
    }

    /// Select enabled normal/automatic sources, preferring normal tracks for a
    /// shared language and normal English variants before automatic English.
    pub fn select_sources(
        &self,
        normal: &Subtitles,
        automatic: &Subtitles,
        write_normal: bool,
        write_automatic: bool,
    ) -> Result<SubtitleSelection> {
        validate_options(self.languages.as_deref(), &self.format)?;
        let mut available = SubtitleMap::<&[SubtitleFormat]>::default();
        let mut positions = HashMap::<Option<String>, usize>::new();
        if write_normal {
            for (language, formats) in &normal.entries {
                if let Some(&index) = positions.get(language) {
                    available.entries[index].1 = formats;
                } else {
                    positions.insert(language.clone(), available.entries.len());
                    available.entries.push((language.clone(), formats));
                }
            }
        }
        let normal_count = available.entries.len();
        if write_automatic {
            for (language, formats) in &automatic.entries {
                if !positions.contains_key(language) {
                    positions.insert(language.clone(), available.entries.len());
                    available.entries.push((language.clone(), formats));
                }
            }
        }
        self.select_available(&available, normal_count)
    }

    fn select_available(
        &self,
        subtitles: &SubtitleMap<&[SubtitleFormat]>,
        normal_count: usize,
    ) -> Result<SubtitleSelection> {
        let mut requested = Vec::new();
        if let Some(languages) = &self.languages {
            for token in languages.split(',') {
                let token = token.trim();
                let (exclude, tag) = token
                    .strip_prefix('-')
                    .map_or((false, token), |tag| (true, tag));
                for (index, (language, _)) in subtitles.entries.iter().enumerate() {
                    if tag != "all"
                        && !language
                            .as_deref()
                            .is_some_and(|language| same_tag(language, tag))
                    {
                        continue;
                    }
                    if exclude {
                        requested.retain(|current| *current != index);
                    } else if !requested.contains(&index) {
                        requested.push(index);
                    }
                }
            }
        } else if !subtitles.is_empty() {
            let english =
                |entries: &[(Option<String>, &[SubtitleFormat])]| -> Result<Option<usize>> {
                    if let Some(index) = entries
                        .iter()
                        .position(|(tag, _)| tag.as_deref() == Some("en"))
                    {
                        return Ok(Some(index));
                    }
                    for (index, (tag, _)) in entries.iter().enumerate() {
                        let tag = tag.as_deref().ok_or_else(|| error("cannot choose a default subtitle language from an unknown language"))?;
                        if tag.starts_with("en") {
                            return Ok(Some(index));
                        }
                    }
                    Ok(None)
                };
            let index = match english(&subtitles.entries[..normal_count])? {
                Some(index) => index,
                None => english(&subtitles.entries)?.unwrap_or(0),
            };
            requested.push(index);
        }
        let mut result = SubtitleSelection {
            requested: RequestedSubtitles::default(),
            warnings: Vec::new(),
        };
        for index in requested {
            let (language, formats) = &subtitles.entries[index];
            let fallback = formats
                .last()
                .ok_or_else(|| error(format!("subtitle language {language:?} has no formats")))?;
            let chosen = self.format.split('/').find_map(|preference| {
                if preference == "best" {
                    Some(fallback)
                } else {
                    formats.iter().rev().find(|format| format.ext == preference)
                }
            });
            let chosen = match chosen {
                Some(format) => format,
                None => {
                    result.warnings.push(format!("No subtitle format found matching {:?} for language {:?}, using {:?}. Use --list-subs for a list of available subtitles", self.format, language.as_deref().unwrap_or("null"), fallback.ext));
                    fallback
                }
            };
            if chosen.ext.is_empty() {
                return Err(error("subtitle format has an empty extension"));
            }
            result.requested.insert(language.clone(), chosen.clone());
        }
        Ok(result)
    }
}

fn validate_options(languages: Option<&str>, format: &str) -> Result<()> {
    for (value, separator) in [(languages, ','), (Some(format), '/')] {
        if let Some(value) = value {
            if value.len() > 16 * 1024 || value.split(separator).count() > 1024 {
                return Err(error("subtitle option exceeds 16 KiB or 1024 entries"));
            }
            if value.chars().any(char::is_control) {
                return Err(error("subtitle options must not contain controls"));
            }
        }
    }
    if let Some(languages) = languages {
        for token in languages.split(',') {
            let token = token.trim();
            let tag = token.strip_prefix('-').unwrap_or(token);
            if tag.is_empty() || tag.chars().any(|c| ".[]{}()*+?|^$\\".contains(c)) {
                return Err(error(format!(
                    "unsupported subtitle language expression {token:?}; use literal tags or all"
                )));
            }
        }
    }
    Ok(())
}
fn same_tag(left: &str, right: &str) -> bool {
    left.to_lowercase() == right.to_lowercase()
}

/// Plan every sidecar name before writing. Components use the native filename
/// sanitizer; collisions are compared with ASCII case folding on the full path.
pub fn subtitle_paths(
    info: &VideoInfo,
    media_path: &Path,
) -> Result<Vec<(String, SubtitleFormat, PathBuf)>> {
    let mut paths = Vec::new();
    let mut seen = HashSet::new();
    let Some(subtitles) = &info.requested_subtitles else {
        return Ok(paths);
    };
    for (language, format) in &subtitles.entries {
        let language = language
            .as_deref()
            .ok_or_else(|| error("cannot write subtitles with an unknown language"))?;
        if format.ext.is_empty() {
            return Err(error("subtitle format has an empty extension"));
        }
        let mut path = media_path.to_path_buf();
        if path
            .extension()
            .is_some_and(|extension| extension == info.ext.as_str())
        {
            path.set_extension("");
        }
        let mut name = path.into_os_string();
        name.push(".");
        name.push(crate::output::safe_component(language));
        name.push(".");
        name.push(crate::output::safe_component(&format.ext));
        let path = PathBuf::from(name);
        if !seen.insert(path.as_os_str().to_ascii_lowercase()) {
            return Err(error(format!("subtitle filenames collide at {path:?}")));
        }
        paths.push((language.into(), format.clone(), path));
    }
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render_subtitles;
    use serde_json::{Value, json};

    fn format(ext: &str, id: &str) -> SubtitleFormat {
        SubtitleFormat {
            extra: Default::default(),
            fragments: Vec::new(),
            protocol: None,
            name: None,
            url: format!("https://example.test/{id}.{ext}").parse().unwrap(),
            ext: ext.into(),
        }
    }
    fn subtitles(tags: &[Option<&str>]) -> Subtitles {
        let mut subs = Subtitles::default();
        for (index, tag) in tags.iter().enumerate() {
            subs.insert(
                tag.map(str::to_owned),
                vec![
                    format("vtt", &index.to_string()),
                    format("srt", &index.to_string()),
                ],
            );
        }
        subs
    }
    fn info() -> VideoInfo {
        serde_json::from_value(json!({"id":"test","title":"Subtitles","url":"https://example.test/video.mp4","webpage_url":"https://example.test/page","ext":"mp4","extractor":"test","extractor_key":"Test","direct":false})).unwrap()
    }
    fn requested_tags(selection: &SubtitleSelection) -> Vec<Option<&str>> {
        selection
            .requested
            .entries
            .iter()
            .map(|(tag, _)| tag.as_deref())
            .collect()
    }

    #[test]
    fn pinned_normal_and_automatic_selection() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/youtube/captions/cases.json"
        ))
        .unwrap();
        for case in fixture["selection_cases"].as_array().unwrap() {
            let map = |field: &str| -> Subtitles {
                Subtitles {
                    entries: case[field]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|group| {
                            (
                                group["language"].as_str().map(str::to_owned),
                                serde_json::from_value(group["formats"].clone()).unwrap(),
                            )
                        })
                        .collect(),
                }
            };
            let selection =
                SubtitleOptions::new(case["languages"].as_str(), case["format"].as_str().unwrap())
                    .unwrap()
                    .select_sources(
                        &map("subtitles"),
                        &map("automatic_captions"),
                        case["write_manual"].as_bool().unwrap(),
                        case["write_auto"].as_bool().unwrap(),
                    )
                    .unwrap();
            let selected: Vec<Value> = selection.requested.entries.iter().map(|(language, format)| json!({
                "language": language, "url": format.url, "ext": format.ext, "name": format.name,
            })).collect();
            assert_eq!(
                json!(selected),
                case["expected"]["selected"],
                "{}",
                case["name"]
            );
            assert_eq!(
                !selection.warnings.is_empty(),
                case["expected"]["warning"].as_bool().unwrap()
            );
        }
    }

    #[test]
    fn inventory_matches_upstream_table() {
        let mut info = info();
        let named = |ext: &str, name: Option<&str>| SubtitleFormat {
            extra: Default::default(),
            fragments: Vec::new(),
            protocol: None,
            name: name.map(str::to_owned),
            ..format(ext, "secret-url")
        };
        info.subtitles.insert(
            Some("en".into()),
            vec![
                named("vtt", Some("English")),
                named("srv1", Some("English")),
            ],
        );
        info.subtitles.insert(
            Some("nb".into()),
            vec![named("vtt", None), named("ttml", Some("Norsk"))],
        );
        info.subtitles
            .insert(Some("de".into()), vec![named("vtt", None)]);
        // Rendered by pinned upstream `render_subtitles_table`.
        assert_eq!(
            render_subtitles(&info),
            "[info] Available subtitles for test:\n\
             Language Name           Formats\n\
             en       English        srv1, vtt\n\
             nb       Norsk, unknown ttml, vtt\n\
             de                      vtt\n"
        );
        info.automatic_captions.insert(
            Some("de".into()),
            vec![named("vtt", None), named("srt", None)],
        );
        assert_eq!(
            crate::render_automatic_captions(&info),
            "[info] Available automatic captions for test:\nLanguage Formats\nde       srt, vtt\n"
        );
        info.subtitles = Subtitles::default();
        assert_eq!(render_subtitles(&info), "test has no subtitles\n");
    }

    #[test]
    fn ordered_object_serialization_duplicate_replacement_and_unknown_roundtrip() {
        struct NoDefault;
        assert!(SubtitleMap::<NoDefault>::default().is_empty());
        let mut map = SubtitleMap::default();
        map.insert(Some("fr".into()), 1);
        map.insert(None, 2);
        map.insert(Some("en".into()), 3);
        assert_eq!(map.insert(Some("fr".into()), 4), Some(1));
        let json = serde_json::to_string(&map).unwrap();
        assert_eq!(json, "{\"fr\":4,\"null\":2,\"en\":3}");
        let roundtrip: SubtitleMap<u32> = serde_json::from_str(&json).unwrap();
        assert_eq!(roundtrip.entries[1].0.as_deref(), Some("null"));
        let duplicate: SubtitleMap<u32> =
            serde_json::from_str("{\"en\":1,\"fr\":2,\"en\":3}").unwrap();
        assert_eq!(
            duplicate.entries,
            vec![(Some("en".into()), 3), (Some("fr".into()), 2)]
        );
    }

    #[test]
    fn default_language_and_literal_selection_exclusion_and_readd_order() {
        let subs = subtitles(&[Some("fr"), Some("en-US"), Some("en"), Some("de")]);
        assert_eq!(
            requested_tags(&SubtitleOptions::default().select(&subs).unwrap()),
            vec![Some("en")]
        );
        let selection = SubtitleOptions::new(Some("all,-en,en,-en-US,EN-us"), "vtt/srt")
            .unwrap()
            .select(&subs)
            .unwrap();
        assert_eq!(
            requested_tags(&selection),
            vec![Some("fr"), Some("de"), Some("en"), Some("en-US")]
        );
        assert!(
            selection
                .requested
                .entries
                .iter()
                .all(|(_, format)| format.ext == "vtt")
        );
        let missing = SubtitleOptions::new(Some("xx"), "best")
            .unwrap()
            .select(&subs)
            .unwrap();
        assert!(missing.requested.is_empty());
        assert!(missing.warnings.is_empty());
        let unicode = subtitles(&[Some("ÉN")]);
        assert_eq!(
            requested_tags(
                &SubtitleOptions::new(Some("én"), "best")
                    .unwrap()
                    .select(&unicode)
                    .unwrap()
            ),
            vec![Some("ÉN")]
        );
    }

    #[test]
    fn comma_language_tokens_trim_like_cli_before_alias_and_exclusion_processing() {
        let subs = subtitles(&[Some("fr"), Some("en"), Some("de"), Some(" en ")]);
        let expected = vec![Some("en"), Some("de"), Some(" en ")];
        let parsed = SubtitleOptions::new(Some(" en , all , -fr "), "best").unwrap();
        assert_eq!(requested_tags(&parsed.select(&subs).unwrap()), expected);
        let manual = SubtitleOptions {
            languages: Some(" en , all , -fr ".into()),
            format: "best".into(),
        };
        assert_eq!(requested_tags(&manual.select(&subs).unwrap()), expected);
        let literal = SubtitleOptions::new(Some(" en "), "best")
            .unwrap()
            .select(&subs)
            .unwrap();
        assert_eq!(requested_tags(&literal), vec![Some("en")]);
        assert!(SubtitleOptions::new(Some("en,   ,fr"), "best").is_err());
        assert!(SubtitleOptions::new(Some("en,\tfr"), "best").is_err());
        let controlled = subtitles(&[Some("en\u{1b}[31m")]);
        let warning = SubtitleOptions::new(Some("all"), "missing")
            .unwrap()
            .select(&controlled)
            .unwrap();
        assert!(!warning.warnings[0].contains('\u{1b}'));
        assert!(warning.warnings[0].contains("\\u{1b}"));
    }

    #[test]
    fn unknown_language_default_errors_but_exact_english_shortcuts_and_all_preserves_metadata() {
        let unknown = subtitles(&[None, Some("en-US")]);
        assert!(SubtitleOptions::default().select(&unknown).is_err());
        let known = subtitles(&[None, Some("en")]);
        assert_eq!(
            requested_tags(&SubtitleOptions::default().select(&known).unwrap()),
            vec![Some("en")]
        );
        let all = SubtitleOptions::new(Some("all"), "best")
            .unwrap()
            .select(&unknown)
            .unwrap();
        assert_eq!(requested_tags(&all), vec![None, Some("en-US")]);
        let mut info = info();
        info.requested_subtitles = Some(all.requested);
        assert!(subtitle_paths(&info, Path::new("video.mp4")).is_err());
        let early_english = subtitles(&[Some("en-US"), None]);
        assert_eq!(
            requested_tags(&SubtitleOptions::default().select(&early_english).unwrap()),
            vec![Some("en-US")]
        );
    }

    #[test]
    fn format_preference_uses_last_matching_and_warns_only_for_fallback() {
        let mut subs = Subtitles::default();
        subs.insert(
            Some("en".into()),
            vec![
                format("srt", "first"),
                format("vtt", "middle"),
                format("srt", "last"),
            ],
        );
        let selection = SubtitleOptions::new(Some("en"), "ass/srt/vtt")
            .unwrap()
            .select(&subs)
            .unwrap();
        assert!(selection.requested.entries[0].1.url.path().contains("last"));
        assert!(selection.warnings.is_empty());
        let fallback = SubtitleOptions::new(Some("en"), "")
            .unwrap()
            .select(&subs)
            .unwrap();
        assert_eq!(
            fallback.requested.entries[0].1,
            selection.requested.entries[0].1
        );
        assert_eq!(fallback.warnings.len(), 1);
        subs.entries[0].1.clear();
        assert!(SubtitleOptions::default().select(&subs).is_err());
    }

    #[test]
    fn unsupported_languages_controls_and_option_limits_are_preflight_errors() {
        for language in [
            "", "-", "en,", ",en", "en.*", "en|fr", "en[US]", "en\\fr", "en\n",
        ] {
            assert!(
                SubtitleOptions::new(Some(language), "best").is_err(),
                "{language}"
            );
        }
        assert!(SubtitleOptions::new(None, "vtt\n").is_err());
        assert!(SubtitleOptions::new(Some(&"x".repeat(16 * 1024 + 1)), "best").is_err());
        assert!(SubtitleOptions::new(Some(&vec!["en"; 1025].join(",")), "best").is_err());
        assert!(SubtitleOptions::new(None, &vec!["vtt"; 1025].join("/")).is_err());
        assert!(SubtitleOptions::new(Some(&vec!["en"; 1024].join(",")), "best").is_ok());
    }

    #[test]
    fn paths_preserve_literal_directories_and_sanitize_colliding_components_before_io() {
        let mut info = info();
        info.requested_subtitles = Some(SubtitleMap {
            entries: vec![(Some("../../EN".into()), format("vtt", "en"))],
        });
        let paths = subtitle_paths(&info, Path::new("literal/parent/movie.mp4")).unwrap();
        assert_eq!(paths[0].2, PathBuf::from("literal/parent/movie._.._EN.vtt"));
        info.requested_subtitles
            .as_mut()
            .unwrap()
            .entries
            .push((Some("../../en".into()), format("vtt", "en2")));
        assert!(subtitle_paths(&info, Path::new("literal/parent/movie.mp4")).is_err());
        info.requested_subtitles = Some(SubtitleMap {
            entries: vec![
                (Some("en".into()), format("vtt", "en")),
                (Some("EN".into()), format("VTT", "en2")),
            ],
        });
        assert!(subtitle_paths(&info, Path::new("movie.mp4")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn sidecar_paths_preserve_non_utf8_literal_path_bytes() {
        use std::os::unix::ffi::OsStringExt;
        let mut info = info();
        info.requested_subtitles = Some(SubtitleMap {
            entries: vec![(Some("en".into()), format("vtt", "en"))],
        });
        let media = PathBuf::from(std::ffi::OsString::from_vec(b"directory/\xff.mp4".to_vec()));
        let expected = PathBuf::from(std::ffi::OsString::from_vec(
            b"directory/\xff.en.vtt".to_vec(),
        ));
        assert_eq!(subtitle_paths(&info, &media).unwrap()[0].2, expected);
    }

    #[test]
    fn rendered_inventory_escapes_metadata_without_resource_urls() {
        let mut info = info();
        info.id = "x\u{1b}[31m\u{202e}\u{2028}".into();
        info.subtitles
            .insert(None, vec![format("vtt", "private-resource")]);
        info.subtitles
            .insert(Some("en\n\u{1b}[31m".into()), vec![format("x", "secret")]);
        let rendered = render_subtitles(&info);
        assert!(!rendered.contains('\u{1b}'));
        assert!(!rendered.contains('\u{202e}'));
        assert!(!rendered.contains('\u{2028}'));
        assert!(!rendered.contains("https://"));
        assert!(!rendered.contains("private-resource"));
        assert!(rendered.contains("\nNone           vtt\n"), "{rendered}");
    }

    fn fixture_subtitles(value: &Value) -> Subtitles {
        let mut subtitles = Subtitles::default();
        for language in value.as_array().unwrap() {
            subtitles.insert(
                language["language"].as_str().map(str::to_owned),
                serde_json::from_value(language["formats"].clone()).unwrap(),
            );
        }
        subtitles
    }
    #[test]
    fn saved_actual_upstream_selection_and_filename_cases_match() {
        let fixture: Value =
            serde_json::from_str(include_str!("../tests/fixtures/subtitles/cases.json")).unwrap();
        for case in fixture["selection_cases"].as_array().unwrap() {
            // Write-disabled processing is an orchestration policy; selection is
            // intentionally invoked only when subtitle writing is requested.
            if case["write"].as_bool() == Some(false) {
                continue;
            }
            let subs = fixture_subtitles(&case["subtitles"]);
            let selection = SubtitleOptions::new(
                case["languages"].as_str(),
                case["formats"].as_str().unwrap(),
            )
            .unwrap()
            .select(&subs)
            .unwrap();
            let expected = case["expected"]["selected"].as_array().unwrap();
            assert_eq!(
                selection.requested.entries.len(),
                expected.len(),
                "{}",
                case["name"]
            );
            for ((language, format), expected) in selection.requested.entries.iter().zip(expected) {
                assert_eq!(
                    language.as_deref(),
                    expected["language"].as_str(),
                    "{}",
                    case["name"]
                );
                assert_eq!(
                    format.url.as_str(),
                    expected["url"].as_str().unwrap(),
                    "{}",
                    case["name"]
                );
                assert_eq!(
                    format.ext,
                    expected["ext"].as_str().unwrap(),
                    "{}",
                    case["name"]
                );
            }
            assert_eq!(
                !selection.warnings.is_empty(),
                case["expected"]["warning"].as_bool().unwrap(),
                "{}",
                case["name"]
            );
        }
        for case in fixture["filename_cases"].as_array().unwrap() {
            let mut info = info();
            info.ext = case["media_ext"].as_str().unwrap().into();
            info.requested_subtitles = Some(SubtitleMap {
                entries: vec![(
                    case["language"].as_str().map(str::to_owned),
                    SubtitleFormat {
                        extra: Default::default(),
                        fragments: Vec::new(),
                        protocol: None,
                        name: None,
                        url: "https://example.test/sub".parse().unwrap(),
                        ext: case["ext"].as_str().unwrap().into(),
                    },
                )],
            });
            assert_eq!(
                subtitle_paths(&info, Path::new(case["filename"].as_str().unwrap())).unwrap()[0].2,
                PathBuf::from(case["expected"]["filename"].as_str().unwrap()),
                "{}",
                case["name"]
            );
        }
    }
}

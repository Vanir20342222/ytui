//! Typed output templates (`-o TYPE:TEMPLATE`) and output paths (`-P TYPE:PATH`).

use crate::{PlaylistCounts, PlaylistInfo, Result, VideoInfo};
use std::{collections::BTreeMap, path::PathBuf};

/// Upstream `OUTTMPL_TYPES` with their forced extensions.
pub const OUTPUT_TYPES: [(&str, Option<&str>); 11] = [
    ("chapter", None),
    ("subtitle", None),
    ("thumbnail", None),
    ("description", Some("description")),
    ("annotation", Some("annotations.xml")),
    ("infojson", Some("info.json")),
    ("link", None),
    ("pl_video", None),
    ("pl_thumbnail", None),
    ("pl_description", Some("description")),
    ("pl_infojson", Some("info.json")),
];

pub(super) fn forced_extension(kind: &str) -> Option<&'static str> {
    OUTPUT_TYPES
        .iter()
        .find(|(name, _)| *name == kind)
        .and_then(|(_, ext)| *ext)
}

fn is_output_type(key: &str) -> bool {
    OUTPUT_TYPES.iter().any(|(name, _)| *name == key)
}

/// `_dict_from_options_callback`: `KEY[,KEY...]:VALUE` with case-insensitive
/// allowed keys, otherwise the whole value for `default_key`.
fn parse_typed<'a>(
    value: &'a str,
    allowed: impl Fn(&str) -> bool,
    default_key: &str,
) -> (Vec<String>, &'a str) {
    if let Some((keys, rest)) = value.split_once(':') {
        let keys: Vec<String> = keys.split(',').map(str::to_lowercase).collect();
        if keys.iter().all(|key| allowed(key)) {
            return (keys, rest);
        }
    }
    (vec![default_key.to_owned()], value)
}

/// Output templates by type; `default` names media files.
#[derive(Debug, Clone)]
pub struct OutputTemplates {
    templates: BTreeMap<String, String>,
}

impl OutputTemplates {
    /// Parse repeated `-o [TYPES:]TEMPLATE` values; later values win per type.
    /// A missing or empty default template selects the upstream default.
    pub fn parse(values: &[String], restrict: bool) -> Self {
        let mut templates = BTreeMap::new();
        for value in values {
            let (keys, template) = parse_typed(value, is_output_type, "default");
            for key in keys {
                templates.insert(key, template.to_owned());
            }
        }
        if templates.get("default").is_none_or(String::is_empty) {
            templates.insert("default".into(), super::default_template(restrict));
        }
        // Upstream `DEFAULT_OUTTMPL['chapter']`.
        templates.entry("chapter".into()).or_insert_with(|| {
            "%(title)s - %(section_number)03d %(section_title)s [%(id)s].%(ext)s".into()
        });
        Self { templates }
    }

    /// Whether `values` set the default (untyped or `default:`) template.
    pub fn sets_default(values: &[String]) -> bool {
        values.iter().any(|value| {
            parse_typed(value, is_output_type, "default")
                .0
                .iter()
                .any(|key| key == "default")
        })
    }

    /// The template for `kind`, falling back to the default template.
    pub fn get(&self, kind: Option<&str>) -> &str {
        kind.and_then(|kind| self.templates.get(kind))
            .unwrap_or(&self.templates["default"])
    }

    /// Every configured template, for startup validation.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.templates
            .iter()
            .map(|(kind, template)| (kind.as_str(), template.as_str()))
    }

    /// Render a typed filename (`None` for media), or `None` when the type's
    /// template is empty (upstream then skips writing that file).
    pub fn prepare(
        &self,
        info: &VideoInfo,
        kind: Option<&str>,
        options: &super::TemplateOptions,
    ) -> Result<Option<PathBuf>> {
        let template = self.get(kind);
        if template.is_empty() {
            return Ok(None);
        }
        super::template::render_kind(
            template,
            super::template::Source::Video(info),
            options,
            kind,
        )
        .map(Some)
    }

    /// Render a playlist-level typed filename such as `pl_infojson`.
    pub fn prepare_playlist(
        &self,
        info: &PlaylistInfo,
        counts: PlaylistCounts,
        kind: &str,
        options: &super::TemplateOptions,
    ) -> Result<Option<PathBuf>> {
        let template = self.get(Some(kind));
        if template.is_empty() {
            return Ok(None);
        }
        let source = super::template::Source::Playlist {
            info,
            playlist_count: counts.playlist_count,
            n_entries: counts.n_entries,
            last_playlist_index: counts.last_playlist_index,
        };
        super::template::render_kind(template, source, options, Some(kind)).map(Some)
    }
}

/// Parse repeated `-P [TYPES:]PATH` values (`home` by default).
pub fn parse_paths(values: &[String]) -> Result<BTreeMap<String, String>> {
    let mut paths = BTreeMap::new();
    for value in values {
        let allowed = |key: &str| key == "home" || key == "temp" || is_output_type(key);
        let (keys, path) = parse_typed(value, allowed, "home");
        for key in keys {
            paths.insert(key, path.to_owned());
        }
    }
    Ok(paths)
}

/// Python `posixpath.join`.
fn join(parts: &[&str]) -> String {
    let mut path = String::new();
    for part in parts {
        if part.starts_with('/') {
            path = (*part).to_owned();
        } else if path.is_empty() || path.ends_with('/') {
            path.push_str(part);
        } else {
            path.push('/');
            path.push_str(part);
        }
    }
    path
}

/// Upstream `get_output_path(kind, filename)` before Windows sanitization.
pub(super) fn output_path(
    options: &super::TemplateOptions,
    kind: Option<&str>,
    filename: &str,
) -> String {
    let path = |key: &str| {
        let value = options.paths.get(key).map_or("", String::as_str);
        crate::batch::expand_path_text(crate::batch::py_strip(value))
    };
    let home = path("home");
    let typed = kind.map(path).unwrap_or_default();
    join(&[&home, &typed, filename])
}

/// Python `replace_extension(filename, ext, expected_real_ext)` on path text.
pub fn replace_extension(filename: &str, ext: &str, expected: Option<&str>) -> String {
    let start = filename.rfind('/').map_or(0, |index| index + 1);
    let name = &filename[start..];
    // `splitext`: the last dot, if a non-dot character precedes it.
    let dot = name
        .rfind('.')
        .filter(|&dot| name[..dot].bytes().any(|byte| byte != b'.'));
    match dot {
        Some(dot) if expected.is_none_or(|expected| &name[dot + 1..] == expected) => {
            format!("{}.{ext}", &filename[..start + dot])
        }
        _ => format!("{filename}.{ext}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_upstream_typed_templates_and_paths() {
        use crate::output::TemplateOptions;
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/paths/cases.json")).unwrap();
        let info: VideoInfo = serde_json::from_value(fixture["info"].clone()).unwrap();
        let playlist: PlaylistInfo = serde_json::from_value(serde_json::json!({
            "id": "pl", "title": "List", "extractor": "generic", "extractor_key": "Generic",
            "webpage_url": "https://example.test/list", "entries": [],
        }))
        .unwrap();
        let counts = PlaylistCounts {
            playlist_count: 3,
            n_entries: Some(3),
            last_playlist_index: 3,
        };
        for case in fixture["cases"].as_array().unwrap() {
            // Environment expansion is covered by a subprocess CLI test.
            if case["name"] == "home-expansion" {
                continue;
            }
            let args: Vec<String> = serde_json::from_value(case["args"].clone()).unwrap();
            let (mut outputs, mut paths, mut restrict, mut windows, mut trim) =
                (Vec::new(), Vec::new(), false, None, 0);
            let mut iter = args.iter();
            while let Some(arg) = iter.next() {
                match arg.as_str() {
                    "-o" => outputs.push(iter.next().unwrap().clone()),
                    "-P" => paths.push(iter.next().unwrap().clone()),
                    "--restrict-filenames" => restrict = true,
                    "--windows-filenames" => windows = Some(true),
                    "--trim-filenames" => trim = iter.next().unwrap().parse().unwrap(),
                    other => panic!("unexpected fixture argument {other}"),
                }
            }
            let templates = OutputTemplates::parse(&outputs, restrict);
            let options = TemplateOptions {
                filename_mode: TemplateOptions::filename_mode(restrict, windows),
                windows_filenames: windows == Some(true),
                trim_file_name: trim,
                paths: parse_paths(&paths).unwrap(),
                ..Default::default()
            };
            let text = |path: Option<PathBuf>| {
                path.map(|path| path.to_string_lossy().into_owned())
                    .unwrap_or_default()
            };
            let expected = &case["expected"];
            for (key, kind) in [
                ("media", None),
                ("subtitle", Some("subtitle")),
                ("infojson", Some("infojson")),
            ] {
                let actual = text(templates.prepare(&info, kind, &options).unwrap());
                assert_eq!(
                    actual,
                    expected[key].as_str().unwrap(),
                    "{} {key}",
                    case["name"]
                );
            }
            let actual = text(
                templates
                    .prepare_playlist(&playlist, counts, "pl_infojson", &options)
                    .unwrap(),
            );
            assert_eq!(
                actual,
                expected["pl_infojson"].as_str().unwrap(),
                "{} playlist",
                case["name"]
            );
        }
    }

    #[test]
    fn typed_options_follow_the_upstream_dict_callback() {
        let templates = OutputTemplates::parse(
            &[
                "x.%(ext)s".into(),
                "INFOJSON,subtitle:meta/%(id)s".into(),
                "c:\\dir\\%(id)s".into(),
                "pl_infojson:".into(),
            ],
            false,
        );
        assert_eq!(templates.get(None), "c:\\dir\\%(id)s");
        assert_eq!(templates.get(Some("infojson")), "meta/%(id)s");
        assert_eq!(templates.get(Some("subtitle")), "meta/%(id)s");
        assert_eq!(templates.get(Some("thumbnail")), "c:\\dir\\%(id)s");
        assert_eq!(templates.get(Some("pl_infojson")), "");
        assert_eq!(
            OutputTemplates::parse(&["".into()], true).get(None),
            "%(title)s-[%(id)s].%(ext)s"
        );
        let paths =
            parse_paths(&["a".into(), "Home:b".into(), "subtitle,infojson:s".into()]).unwrap();
        assert_eq!(paths["home"], "b");
        assert_eq!(paths["subtitle"], "s");
        assert_eq!(paths["infojson"], "s");
        assert_eq!(parse_paths(&["temp:t".into()]).unwrap()["temp"], "t");
        assert_eq!(parse_paths(&["bogus:x".into()]).unwrap()["home"], "bogus:x");
    }

    #[test]
    fn join_and_replace_extension_match_python() {
        assert_eq!(join(&["", "", "f"]), "f");
        assert_eq!(join(&["a", "", "f"]), "a/f");
        assert_eq!(join(&["a/", "b", "/abs/f"]), "/abs/f");
        assert_eq!(
            replace_extension("d.x/a.mp4", "info.json", Some("mp4")),
            "d.x/a.info.json"
        );
        assert_eq!(
            replace_extension("a.webm", "info.json", Some("mp4")),
            "a.webm.info.json"
        );
        assert_eq!(
            replace_extension("d/.hidden", "info.json", None),
            "d/.hidden.info.json"
        );
        assert_eq!(replace_extension("a.b", "info.json", None), "a.info.json");
    }
}

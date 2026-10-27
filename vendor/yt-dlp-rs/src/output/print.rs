//! `--print` templates: upstream `[WHEN:]` prefixes, field-name shorthand and
//! unsanitized rendering of processed video metadata.
use super::{TemplateOptions, render_template_with_options, template};
use crate::{Error, Result, VideoInfo};
use std::path::PathBuf;

use super::Stage;

/// Which `--get-*` fields to print.
#[derive(Debug, Clone, Copy, Default)]
pub struct ForcedFields {
    pub title: bool,
    pub id: bool,
    pub url: bool,
    pub thumbnail: bool,
    pub description: bool,
    pub filename: bool,
    pub duration: bool,
    pub format: bool,
}

/// A validated `--print` template and the stage that prints it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrintTemplate {
    template: String,
    stage: Stage,
    /// A `--get-*` field printed only when present (upstream `optional`).
    optional: Option<&'static str>,
}

/// The stage's file paths: `filepath` (set from `post_process` on) and the
/// prepared `_filename`.
#[derive(Debug, Clone, Copy, Default)]
pub struct StagePaths<'a> {
    pub filepath: Option<&'a str>,
    pub prepared: Option<&'a str>,
}

impl PrintTemplate {
    /// Parse `[WHEN:]TEMPLATE`. WHEN is case-insensitive and defaults to
    /// `video`; `pre_process` and `after_filter` are rejected
    /// rather than printed at the wrong time.
    /// Bare field lists become one `%(field)s` line per field, and a trailing
    /// `=` prints `field = %(field)#j`, as upstream's `_forceprint` does.
    pub fn parse(spec: &str) -> Result<Self> {
        let (when, template) = Stage::split(spec, "video");
        let stage = Stage::parse(&when)
            .map_err(|_| Error::InvalidTemplate(format!("print stage {when:?} is unsupported")))?;
        let template = Self {
            template: expand_shorthand(template)?,
            stage,
            optional: None,
        };
        // Validate syntax and field names before any extraction, with
        // placeholder metadata standing in for the processed video.
        template.render(
            &placeholder_info(),
            Ok("filename"),
            &TemplateOptions::default(),
        )?;
        Ok(template)
    }

    /// Upstream `__forced_printings` for `--get-title`, `--get-id`,
    /// `--get-url`, `--get-thumbnail`, `--get-description`, `--get-filename`,
    /// `--get-duration` (`formatSeconds`) and `--get-format`, in that order;
    /// thumbnail, description and duration print only when present.
    pub fn forced_fields(fields: ForcedFields) -> Vec<Self> {
        [
            (fields.title, "title", "title", false),
            (fields.id, "id", "id", false),
            (fields.url, "urls", "urls", false),
            (fields.thumbnail, "thumbnail", "thumbnail", true),
            (fields.description, "description", "description", true),
            (fields.filename, "filename", "filename", false),
            (fields.duration, "duration_string", "duration", true),
            (fields.format, "format", "format", false),
        ]
        .into_iter()
        .filter(|(enabled, ..)| *enabled)
        .map(|(_, field, source, optional)| Self {
            template: format!("%({field})s"),
            stage: Stage::Video,
            optional: optional.then_some(source),
        })
        .collect()
    }

    /// Whether this template prints for `info` (optional forced fields need
    /// a value).
    pub fn applies(&self, info: &VideoInfo) -> bool {
        self.optional
            .is_none_or(|field| template::has_value(info, field))
    }

    /// Render one line (without its terminating newline). `filename` is the
    /// prepared output filename, or the error from preparing it; the error
    /// is reported only when the template refers to `filename`.
    pub fn render(
        &self,
        info: &VideoInfo,
        filename: std::result::Result<&str, &Error>,
        options: &TemplateOptions,
    ) -> Result<String> {
        let paths = StagePaths {
            filepath: None,
            prepared: filename.ok(),
        };
        self.render_at(info, filename, paths, options)
    }

    /// Render at a stage with its `filepath`/`_filename` values.
    pub fn render_at(
        &self,
        info: &VideoInfo,
        filename: std::result::Result<&str, &Error>,
        paths: StagePaths<'_>,
        options: &TemplateOptions,
    ) -> Result<String> {
        let urls = print_urls(info);
        template::expand(
            &self.template,
            template::Source::Print {
                info,
                filename,
                urls: &urls,
                filepath: paths.filepath,
                prepared: paths.prepared,
            },
            options,
        )
    }

    /// Render over a playlist's info dict (the `playlist` stage).
    pub fn render_dict(
        &self,
        dict: &serde_json::Value,
        options: &TemplateOptions,
    ) -> Result<String> {
        template::expand(&self.template, template::Source::Json(dict), options)
    }

    /// The stage that prints this template.
    pub fn stage(&self) -> Stage {
        self.stage
    }
}

/// A validated `--print-to-file [WHEN:]TEMPLATE FILE` pair. Like upstream,
/// FILE is an output (filename) template rendered per video, and each
/// rendered line is appended to it with a trailing newline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrintToFile {
    template: PrintTemplate,
    file: String,
}

impl PrintToFile {
    /// Validate both templates before extraction, as upstream does.
    pub fn parse(spec: &str, file: &str) -> Result<Self> {
        let template = PrintTemplate::parse(spec)?;
        render_template_with_options(file, &placeholder_info(), &TemplateOptions::default())?;
        Ok(Self {
            template,
            file: file.into(),
        })
    }

    /// The expanded line template, for diagnostics.
    pub fn template(&self) -> &str {
        &self.template.template
    }

    /// Render the line and its sanitized destination for one video.
    pub fn render(
        &self,
        info: &VideoInfo,
        filename: std::result::Result<&str, &Error>,
        options: &TemplateOptions,
    ) -> Result<(PathBuf, String)> {
        let paths = StagePaths {
            filepath: None,
            prepared: filename.ok(),
        };
        self.render_at(info, filename, paths, options)
    }

    /// Render at a stage with its `filepath`/`_filename` values.
    pub fn render_at(
        &self,
        info: &VideoInfo,
        filename: std::result::Result<&str, &Error>,
        paths: StagePaths<'_>,
        options: &TemplateOptions,
    ) -> Result<(PathBuf, String)> {
        let path = render_template_with_options(&self.file, info, options)?;
        Ok((
            path,
            self.template.render_at(info, filename, paths, options)?,
        ))
    }

    /// Render the line and destination over a playlist's info dict.
    pub fn render_dict(
        &self,
        dict: &serde_json::Value,
        options: &TemplateOptions,
    ) -> Result<(PathBuf, String)> {
        let path =
            template::render_kind(&self.file, template::Source::JsonFile(dict), options, None)?;
        Ok((path, self.template.render_dict(dict, options)?))
    }

    /// The stage that prints this pair.
    pub fn stage(&self) -> Stage {
        self.template.stage
    }
}

pub(super) fn placeholder_info() -> VideoInfo {
    serde_json::from_value(serde_json::json!({
        "id": "id", "title": "title", "ext": "mp4", "extractor": "generic",
        "extractor_key": "Generic", "url": "https://example.invalid/media",
        "webpage_url": "https://example.invalid/", "direct": true,
    }))
    .expect("static placeholder metadata is valid")
}

/// Selected stream URLs, one per line for multi-stream selections.
fn print_urls(info: &VideoInfo) -> String {
    if info.requested_formats.is_empty() {
        info.url.to_string()
    } else {
        info.requested_formats
            .iter()
            .map(|format| format.url.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Length of one `[\w.:,]|-\d` shorthand unit at the start of `input`.
fn unit(input: &str) -> Option<usize> {
    let mut chars = input.chars();
    match chars.next()? {
        c if word(c) || matches!(c, '.' | ':' | ',') => Some(c.len_utf8()),
        '-' if chars.next().is_some_and(|c| c.is_ascii_digit()) => Some(2),
        _ => None,
    }
}

/// Upstream `([\w.:,]|-\d|(?P<dict>{([\w.:,]|-\d)+}))+=?` full match:
/// `Some(has_dict)` for shorthand, `None` for a literal output template.
fn shorthand_kind(body: &str) -> Option<bool> {
    let (mut rest, mut dict) = (body, false);
    if rest.is_empty() {
        return None;
    }
    while !rest.is_empty() {
        if let Some(inner) = rest.strip_prefix('{') {
            let mut count = 0;
            let mut inner = inner;
            while let Some(length) = unit(inner) {
                inner = &inner[length..];
                count += 1;
            }
            rest = inner.strip_prefix('}').filter(|_| count > 0)?;
            dict = true;
        } else {
            rest = &rest[unit(rest)?..];
        }
    }
    Some(dict)
}

fn expand_shorthand(template: &str) -> Result<String> {
    // Upstream `format_tmpl`.
    let Some(dict) = shorthand_kind(template.strip_suffix('=').unwrap_or(template)) else {
        return Ok(template.into());
    };
    let mut body = template.to_owned();
    let mut json = false;
    if body.starts_with('{') {
        body.insert(0, '.');
        json = true;
    }
    let debug = body.ends_with('=');
    if debug {
        body.pop();
    }
    let format = |field: &str| {
        if debug {
            format!("{field} = %({field})#j")
        } else if json {
            format!("%({field})j")
        } else {
            format!("%({field})s")
        }
    };
    Ok(if dict {
        format(&body)
    } else {
        body.split(',').map(format).collect::<Vec<_>>().join("\n")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn info() -> VideoInfo {
        serde_json::from_value(json!({"id":"a/b","title":"Épisode: \"1\"\n","ext":"mp4","extractor":"generic","extractor_key":"Generic","url":"https://example.com/m.mp4","webpage_url":"https://example.com/watch","direct":false,"format_id":"","width":1280,"playlist_index":3,"__last_playlist_index":120})).unwrap()
    }

    fn print(spec: &str) -> String {
        PrintTemplate::parse(spec)
            .unwrap()
            .render(&info(), Ok("out/a.mp4"), &TemplateOptions::default())
            .unwrap()
    }

    #[test]
    fn when_prefixes_and_shorthand_follow_upstream() {
        assert_eq!(print("id"), "a/b");
        assert_eq!(print("VIDEO:id,width"), "a/b\n1280");
        assert_eq!(print("video:%(id)s:%(width)05d"), "a/b:01280");
        assert_eq!(print("id,ext="), "id = \"a/b\"\next = \"mp4\"");
        assert_eq!(print("width="), "width = 1280");
        assert_eq!(print("title)"), "title)");
        assert_eq!(print("pre-x"), "pre-x");
        assert_eq!(print("{x"), "{x");
        assert_eq!(print("unknown: id"), "unknown: id");
        assert_eq!(
            print("filename,urls"),
            "out/a.mp4\nhttps://example.com/m.mp4"
        );
        assert!(PrintTemplate::parse("pre_process:%(id)s").is_err());
        assert_eq!(print("unknown:id"), "NA");
        // Upstream's key grammar: unmatched keys and missing traversals are NA.
        assert_eq!(print("-1"), "NA");
        assert_eq!(print("pre-1"), "NA");
        assert_eq!(print("%(id)q"), "a/b");
        assert!(PrintTemplate::parse("%(id)j").is_ok());
        let staged = PrintTemplate::parse("Before_DL:%(id)s %(_filename)s %(filepath)s").unwrap();
        assert_eq!(staged.stage(), Stage::BeforeDownload);
        let paths = StagePaths {
            filepath: Some("/abs/a.mp4"),
            prepared: Some("a.mp4"),
        };
        assert_eq!(
            staged
                .render_at(&info(), Ok("a.mp4"), paths, &TemplateOptions::default())
                .unwrap(),
            "a/b a.mp4 /abs/a.mp4"
        );
        // Filenames sanitize the JSON text like upstream.
        assert_eq!(
            super::super::render_template("%(id)j", &info()).unwrap(),
            std::path::PathBuf::from("＂a⧸b＂")
        );
    }

    #[test]
    fn printing_is_unsanitized_and_keeps_empty_values() {
        assert_eq!(print("%(title)s"), "Épisode: \"1\"\n");
        assert_eq!(print("%(format_id,id)s|"), "|");
        assert_eq!(print("%(height|-)s %(height)s"), "- NA");
        assert_eq!(print("%(playlist_index)s"), "003");
        assert_eq!(print("%(direct)s %(direct)j"), "False false");
        assert_eq!(print("%(title)j"), r#""\u00c9pisode: \"1\"\n""#);
        assert_eq!(print("%(title)+j"), r#""Épisode: \"1\"\n""#);
        assert_eq!(print("%(id)-7j|%(id)7.3j"), "\"a/b\"  |    \"a/");
        assert_eq!(print(""), "");
        assert_eq!(print("-"), "-");
        let error = Error::InvalidTemplate("bad".into());
        let template = PrintTemplate::parse("%(id)s").unwrap();
        assert_eq!(
            template
                .render(&info(), Err(&error), &TemplateOptions::default())
                .unwrap(),
            "a/b"
        );
        // Alternates after an available value are not evaluated.
        let template = PrintTemplate::parse("%(id,filename)s").unwrap();
        assert_eq!(
            template
                .render(&info(), Err(&error), &TemplateOptions::default())
                .unwrap(),
            "a/b"
        );
        let template = PrintTemplate::parse("%(height,filename)s").unwrap();
        assert!(
            template
                .render(&info(), Err(&error), &TemplateOptions::default())
                .is_err()
        );
    }

    #[test]
    fn json_strings_match_python_escaping() {
        assert_eq!(
            template_json("\u{1f600}\u{7f}\u{1}\u{8}\u{c}\t\\", true),
            r#""\ud83d\ude00\u007f\u0001\b\f\t\\""#
        );
        assert_eq!(
            template_json("\u{1f600}\u{7f}\u{1}", false),
            "\"\u{1f600}\u{7f}\\u0001\""
        );
    }

    fn template_json(value: &str, ensure_ascii: bool) -> String {
        let mut info = info();
        info.title = value.into();
        let spec = if ensure_ascii {
            "%(title)j"
        } else {
            "%(title)+j"
        };
        PrintTemplate::parse(spec)
            .unwrap()
            .render(&info, Ok(""), &TemplateOptions::default())
            .unwrap()
    }

    /// Parse a saved case's `-O`/`--print` and `--get-*` arguments.
    fn templates(args: &[serde_json::Value]) -> Result<Vec<PrintTemplate>> {
        let (mut templates, mut get) = (Vec::new(), [false; 4]);
        let mut args = args.iter().map(|arg| arg.as_str().unwrap());
        while let Some(arg) = args.next() {
            match arg {
                "-O" | "--print" => templates.push(PrintTemplate::parse(args.next().unwrap())?),
                "-e" => get[0] = true,
                "--get-id" => get[1] = true,
                "-g" => get[2] = true,
                "--get-filename" => get[3] = true,
                other => panic!("unexpected fixture argument {other}"),
            }
        }
        templates.extend(PrintTemplate::forced_fields(ForcedFields {
            title: get[0],
            id: get[1],
            url: get[2],
            filename: get[3],
            ..Default::default()
        }));
        Ok(templates)
    }

    #[test]
    fn saved_pinned_print_cases_match_actual_upstream() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/print/cases.json")).unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let info: VideoInfo = serde_json::from_value(case["info"].clone()).unwrap();
            let options = TemplateOptions {
                na_placeholder: case["na_placeholder"].as_str().unwrap().into(),
                ..Default::default()
            };
            let lines: Vec<_> = templates(case["args"].as_array().unwrap())
                .unwrap()
                .iter()
                .map(|template| {
                    template
                        .render(&info, Ok(case["filename"].as_str().unwrap()), &options)
                        .unwrap()
                })
                .collect();
            assert_eq!(
                serde_json::json!(lines),
                case["expected"],
                "{}",
                case["name"]
            );
        }
        // Source-only cases are rejected when parsed or rendered.
        for case in fixture["source_only_cases"].as_array().unwrap() {
            let info: VideoInfo = serde_json::from_value(case["info"].clone()).unwrap();
            let rejected = templates(case["args"].as_array().unwrap()).is_err()
                || templates(case["args"].as_array().unwrap())
                    .unwrap()
                    .iter()
                    .any(|template| {
                        template
                            .render(
                                &info,
                                Ok(case["filename"].as_str().unwrap()),
                                &TemplateOptions::default(),
                            )
                            .is_err()
                    });
            assert!(rejected, "{}", case["name"]);
        }
    }

    #[test]
    fn print_to_file_renders_sanitized_destinations() {
        let pair = PrintToFile::parse("video:id,ext", "lists/%(title)s.txt").unwrap();
        assert_eq!(pair.template(), "%(id)s\n%(ext)s");
        let (path, line) = pair
            .render(&info(), Ok("f.mp4"), &TemplateOptions::default())
            .unwrap();
        assert_eq!(path, PathBuf::from("lists/Épisode： ＂1＂.txt"));
        assert_eq!(line, "a/b\nmp4");
        assert!(PrintToFile::parse("playlist:id", "x.txt").is_ok());
        assert!(PrintToFile::parse("id", "%(id.a-b)s.txt").is_ok());
        assert!(PrintToFile::parse("id", "").is_err());
    }

    #[test]
    fn forced_fields_use_upstream_order() {
        let fields = PrintTemplate::forced_fields(ForcedFields {
            title: true,
            id: true,
            url: true,
            filename: true,
            ..Default::default()
        });
        let lines: Vec<_> = fields
            .iter()
            .map(|t| {
                t.render(&info(), Ok("f.mp4"), &TemplateOptions::default())
                    .unwrap()
            })
            .collect();
        assert_eq!(
            lines,
            [
                "Épisode: \"1\"\n",
                "a/b",
                "https://example.com/m.mp4",
                "f.mp4"
            ]
        );
    }
}

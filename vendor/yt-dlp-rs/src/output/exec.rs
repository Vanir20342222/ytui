//! `--exec [WHEN:]CMD`: upstream `ExecPP` command templates. Fields use the
//! output-template syntax restricted to the `d`/`i`/`f`/`q` conversions, and
//! commands without fields get the shell-quoted file path for `{}` (appended
//! when absent). Commands run through `/bin/sh -c`; upstream's Windows `cmd`
//! quoting is not ported, so `--exec` is rejected there.
use super::{TemplateOptions, template};
use crate::{Error, Result, VideoInfo};

/// Upstream `POSTPROCESS_WHEN`.
const WHEN: &[&str] = &[
    "pre_process",
    "after_filter",
    "video",
    "before_dl",
    "post_process",
    "after_move",
    "after_video",
    "playlist",
];

const ADVISORY: &str =
    "See  https://github.com/yt-dlp/yt-dlp/security/advisories/GHSA-69qj-pvh9-c5wg  for details";
const UNSAFE_DEFAULT_CHARS: &str = "\"' \n\t;&|^$%*<>{}()[]`#\\";

/// The implemented `POSTPROCESS_WHEN` stages for `--exec` and `--print`,
/// in processing order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Stage {
    /// After format selection, before printing (also when simulating).
    Video,
    /// After sidecars and info JSON, before the media download.
    BeforeDownload,
    /// After postprocessing, before files are moved into place.
    PostProcess,
    /// After the final file is in place (the default).
    AfterMove,
    /// After a video and its archive record are complete.
    AfterVideo,
    /// After every playlist entry, with the playlist's info dict.
    Playlist,
}

impl Stage {
    /// Split a case-insensitive `WHEN:` prefix from `value`, defaulting to
    /// `default` when the prefix is absent or not a `POSTPROCESS_WHEN` name.
    pub(super) fn split<'a>(value: &'a str, default: &str) -> (String, &'a str) {
        match value.split_once(':') {
            Some((when, rest)) if WHEN.iter().any(|w| w.eq_ignore_ascii_case(when)) => {
                (when.to_ascii_lowercase(), rest)
            }
            _ => (default.into(), value),
        }
    }

    pub(super) fn parse(when: &str) -> Result<Self> {
        Ok(match when {
            "video" => Self::Video,
            "before_dl" => Self::BeforeDownload,
            "post_process" => Self::PostProcess,
            "after_move" => Self::AfterMove,
            "after_video" => Self::AfterVideo,
            "playlist" => Self::Playlist,
            other => {
                return Err(Error::Exec(format!("stage {other:?} is unsupported")));
            }
        })
    }
}

/// One validated command template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecCommand {
    template: String,
}

/// Upstream POSIX shell quoting (`shlex.quote`).
pub fn shell_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".into();
    }
    if value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c))
    {
        return value.into();
    }
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

impl ExecCommand {
    /// Upstream `ExecPP.set_downloader` validation (`prepare_outtmpl(cmd,
    /// {}, _exec=True)`) plus this port's template field checks.
    fn validate(template: &str, options: &TemplateOptions) -> Result<Self> {
        let repr = crate::batch::py_repr(template);
        let checked = if options.allow_unsafe_exec {
            Vec::new()
        } else {
            template::exec_fields(template)
        };
        for (conversion, default) in checked {
            if !conversion.is_some_and(|c| "difq".contains(c)) {
                return Err(Error::Exec(format!(
                    "Unsafe conversion(s) in exec command: {repr}\nConversions such as %()s are too dangerous to be used in --exec command templates; use %()q instead. {ADVISORY}"
                )));
            }
            let unsafe_text = |text: &str| text.chars().any(|c| UNSAFE_DEFAULT_CHARS.contains(c));
            match default {
                Some(default) if unsafe_text(default) => {
                    return Err(Error::Exec(format!(
                        "Unsafe default(s) in exec command: {repr}\nConversions are not applied to --exec command template defaults, e.g. %(...|DEFAULT;)q. {ADVISORY}"
                    )));
                }
                None if unsafe_text(&options.na_placeholder) => {
                    return Err(Error::Exec(format!(
                        "Unsafe placeholder for exec command: {}\nThe --output-na-placeholder argument also applies to --exec command templates. {ADVISORY}",
                        crate::batch::py_repr(&options.na_placeholder)
                    )));
                }
                _ => {}
            }
        }
        let command = Self {
            template: template.into(),
        };
        // Field names and syntax must also be representable here.
        command.render(
            &super::print::placeholder_info(),
            Some("file"),
            Some("file"),
            options,
        )?;
        Ok(command)
    }

    /// `ExecPP.parse_cmd`: render fields, or substitute the quoted file path
    /// (`filepath`, else `_filename`) for `{}` when there are none.
    pub fn render(
        &self,
        info: &VideoInfo,
        filepath: Option<&str>,
        filename: Option<&str>,
        options: &TemplateOptions,
    ) -> Result<String> {
        if !template::exec_fields(&self.template).is_empty() {
            return template::expand(
                &self.template,
                template::Source::Exec {
                    info,
                    filepath,
                    filename,
                },
                options,
            );
        }
        let Some(path) = filepath.or(filename) else {
            return Ok(self.template.clone());
        };
        let mut command = self.template.clone();
        if !command.contains("{}") {
            command.push_str(" {}");
        }
        Ok(command.replace("{}", &shell_quote(path)))
    }

    /// `parse_cmd` over a playlist's info dict, which has no file path.
    pub fn render_dict(
        &self,
        dict: &serde_json::Value,
        options: &TemplateOptions,
    ) -> Result<String> {
        if template::exec_fields(&self.template).is_empty() {
            return Ok(self.template.clone());
        }
        template::expand(&self.template, template::Source::JsonExec(dict), options)
    }

    /// Run through `/bin/sh -c` with inherited standard streams, returning
    /// upstream's failure message.
    pub fn run(command: &str) -> std::result::Result<(), String> {
        let status = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(command)
            .status()
            .map_err(|error| format!("Command could not be started: {error}"))?;
        match status.code() {
            Some(0) => Ok(()),
            Some(code) => Err(format!("Command returned error code {code}")),
            // Python reports a signal as a negative return code.
            None => Err(format!(
                "Command returned error code {}",
                signal(&status).map_or_else(|| "None".to_owned(), |s| (-s).to_string())
            )),
        }
    }
}

#[cfg(unix)]
fn signal(status: &std::process::ExitStatus) -> Option<i32> {
    std::os::unix::process::ExitStatusExt::signal(status)
}

#[cfg(not(unix))]
fn signal(_: &std::process::ExitStatus) -> Option<i32> {
    None
}

/// Commands by stage, in the order given.
#[derive(Debug, Clone, Default)]
pub struct ExecCommands {
    commands: Vec<(Stage, ExecCommand)>,
}

impl ExecCommands {
    /// Split upstream's case-insensitive `WHEN:` prefix (default
    /// `after_move`) from each value, in option order.
    pub fn parse(values: &[(Option<&str>, &str)], options: &TemplateOptions) -> Result<Self> {
        if cfg!(windows) && !values.is_empty() {
            return Err(Error::Exec(
                "--exec is unsupported on Windows (cmd quoting is not implemented)".into(),
            ));
        }
        let mut commands = Vec::new();
        for (forced, value) in values {
            let (when, template) = match forced {
                Some(when) => (when.to_string(), *value),
                None => Stage::split(value, "after_move"),
            };
            commands.push((
                Stage::parse(&when)
                    .map_err(|_| Error::Exec(format!("--exec stage {when:?} is unsupported")))?,
                ExecCommand::validate(template, options)?,
            ));
        }
        Ok(Self { commands })
    }

    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    pub fn has(&self, stage: Stage) -> bool {
        self.commands.iter().any(|(s, _)| *s == stage)
    }

    pub fn stage(&self, stage: Stage) -> impl Iterator<Item = &ExecCommand> {
        self.commands
            .iter()
            .filter(move |(s, _)| *s == stage)
            .map(|(_, command)| command)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posix_quoting_and_validation_follow_upstream() {
        assert_eq!(shell_quote(""), "''");
        assert_eq!(shell_quote("a/b-c.mp4"), "a/b-c.mp4");
        assert_eq!(shell_quote("it's here"), "'it'\"'\"'s here'");
        if cfg!(windows) {
            return;
        }
        let options = TemplateOptions::default();
        let error = ExecCommands::parse(&[(None, "echo %(title)s")], &options).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Unsafe conversion(s) in exec command: 'echo %(title)s'")
        );
        let error = ExecCommands::parse(&[(None, "echo %(title|a b)q")], &options).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Unsafe default(s) in exec command")
        );
        let options = TemplateOptions {
            na_placeholder: "N A".into(),
            ..Default::default()
        };
        let error = ExecCommands::parse(&[(None, "echo %(title)q")], &options).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Unsafe placeholder for exec command: 'N A'")
        );
        assert!(ExecCommands::parse(&[(None, "echo 100% {}")], &options).is_ok());
    }
}

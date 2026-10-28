//! Upstream `ExternalFD` subclasses for whole-file HTTP/FTP downloads
//! (`--downloader curl|wget|aria2c|axel|httpie`): the command lines of
//! `CurlFD`, `WgetFD`, `Aria2cFD`, `AxelFD` and `HttpieFD`, run into the
//! temporary name and published like `ExternalFD.real_download`.
use crate::{
    CookieJar, DownloadOptions, DownloadPhase, DownloadProgress, Error, PartFiles, Result,
    VideoInfo,
};
use serde_json::Value;
use std::{
    collections::HashMap,
    ffi::OsString,
    io::{Read, Write},
    ops::ControlFlow,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{LazyLock, Mutex},
    time::Duration,
};

/// An external downloader class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Kind {
    Curl,
    Axel,
    Wget,
    Aria2c,
    Httpie,
}

impl Kind {
    /// `_BY_NAME` order (class definition order).
    const ALL: [Kind; 5] = [
        Kind::Curl,
        Kind::Axel,
        Kind::Wget,
        Kind::Aria2c,
        Kind::Httpie,
    ];

    /// `get_basename()`: the class name without `FD`, lowercased.
    pub(crate) fn basename(self) -> &'static str {
        match self {
            Kind::Curl => "curl",
            Kind::Axel => "axel",
            Kind::Wget => "wget",
            Kind::Aria2c => "aria2c",
            Kind::Httpie => "httpie",
        }
    }

    /// `EXE_NAME`.
    fn exe_name(self) -> &'static str {
        match self {
            Kind::Httpie => "http",
            other => other.basename(),
        }
    }

    fn available_opt(self) -> &'static str {
        match self {
            Kind::Curl | Kind::Axel => "-V",
            Kind::Wget | Kind::Httpie => "--version",
            Kind::Aria2c => "-v",
        }
    }

    /// Upstream `get_external_downloader` for the non-FFmpeg classes: the
    /// basename without its extension, else the first class whose
    /// `EXE_NAME` it contains.
    pub(crate) fn from_name(name: &str) -> Option<Kind> {
        let base = Path::new(name)
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        Kind::ALL
            .into_iter()
            .find(|kind| kind.basename() == base)
            .or_else(|| {
                Kind::ALL
                    .into_iter()
                    .find(|kind| base.contains(kind.exe_name()))
            })
    }
}

/// The downloader parameters the command lines read.
#[derive(Debug, Clone, Default)]
pub struct ExternalSettings {
    /// `--downloader-args` by key.
    pub args: HashMap<String, Vec<String>>,
    /// `noprogress` (`--no-progress`, or `--quiet` without `--progress`).
    pub no_progress: bool,
    /// `continuedl` (`--no-continue` clears it).
    pub continue_download: bool,
    /// `updatetime` (`--mtime`/`--no-mtime`; unset by default).
    pub update_time: Option<bool>,
    /// `source_address` (`--source-address`, `-4`, `-6`).
    pub source_address: Option<String>,
    /// `nocheckcertificate`.
    pub no_check_certificate: bool,
    /// The `--cookies` file (`cookiejar.filename`).
    pub cookie_file: Option<PathBuf>,
}

/// An available downloader: its class, the executable to run and (curl)
/// the version tuple.
#[derive(Debug, Clone)]
pub(crate) struct Available {
    kind: Kind,
    exe: String,
    curl_version: Vec<u64>,
}

impl Available {
    /// The downloader's `get_basename()`.
    pub(crate) fn basename(&self) -> &'static str {
        self.kind.basename()
    }
}

/// `available(path)` results, as upstream caches `cls.exe`.
static AVAILABLE: LazyLock<Mutex<HashMap<String, Option<Available>>>> =
    LazyLock::new(Default::default);

/// Upstream `ExternalFD.available(path)` (`CurlFD.available` parses
/// `curl -V`).
pub(crate) fn available(name: &str) -> Option<Available> {
    let kind = Kind::from_name(name)?;
    AVAILABLE
        .lock()
        .unwrap()
        .entry(name.to_owned())
        .or_insert_with(|| {
            let exe = if kind == Kind::Curl || name != kind.basename() {
                name
            } else {
                kind.exe_name()
            };
            let output = Command::new(exe)
                .arg(kind.available_opt())
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .ok()?;
            let mut curl_version = Vec::new();
            if kind == Kind::Curl {
                // `_get_exe_version_output`: a failing `curl -V` is unavailable.
                if !output.status.success() {
                    return None;
                }
                let text = String::from_utf8_lossy(&output.stdout).into_owned();
                let parts: Vec<&str> = text.splitn(3, ' ').collect();
                if parts.len() < 3 {
                    return None;
                }
                curl_version = version_tuple(parts[1]);
            }
            Some(Available {
                kind,
                exe: exe.to_owned(),
                curl_version,
            })
        })
        .clone()
}

/// `version_tuple(v, lenient=True)`: leading integers of the dotted parts.
fn version_tuple(version: &str) -> Vec<u64> {
    version
        .split(['.', '-'])
        .map_while(|part| {
            let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
            digits.parse().ok()
        })
        .collect()
}

/// `ExternalFD.supports`: one whole-file HTTP/FTP format without HLS
/// key or query arguments, not to stdout.
pub(crate) fn supports(info: &VideoInfo, protocol: &str, to_stdout: bool) -> bool {
    let extra = |key: &str| {
        info.extra
            .get(key)
            .is_some_and(|value| !value.is_null() && value != "")
    };
    !to_stdout
        && !protocol.contains('+')
        && !(extra("hls_aes")
            || extra("extra_param_to_segment_url")
            || extra("extra_param_to_key_url"))
        && matches!(protocol, "http" | "https" | "ftp" | "ftps")
}

/// Python `str()` of an option value.
fn retries_text(retries: u32) -> String {
    if retries == u32::MAX {
        "inf".into()
    } else {
        retries.to_string()
    }
}

/// `_configuration_args()`: `ROOT` (the basename, or `BASENAME+EXE`), then
/// `(BASENAME, EXE)` together, then `default`.
fn configuration_args(settings: &ExternalSettings, kind: Kind) -> Vec<String> {
    let (main, exe) = (kind.basename(), kind.exe_name());
    let root = if main == exe {
        main.to_owned()
    } else {
        format!("{main}+{exe}")
    };
    let mut keys: Vec<Vec<&str>> = vec![vec![&root]];
    if main != exe {
        keys.push(vec![main, exe]);
    }
    keys.push(vec!["default"]);
    keys.iter()
        .map(|group| {
            group
                .iter()
                .filter_map(|key| settings.args.get(*key))
                .flatten()
                .cloned()
                .collect::<Vec<_>>()
        })
        .find(|args| !args.is_empty())
        .unwrap_or_default()
}

/// The request headers for the format (`info_dict['http_headers']`).
fn http_headers(info: &VideoInfo) -> Vec<(String, String)> {
    info.extra
        .get("http_headers")
        .and_then(Value::as_object)
        .map(|headers| {
            headers
                .iter()
                .map(|(key, value)| {
                    let value = match value {
                        Value::String(text) => text.clone(),
                        other => other.to_string(),
                    };
                    (key.clone(), value)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The parameters of one download.
pub(crate) struct Params<'a> {
    pub settings: &'a ExternalSettings,
    pub cookies: &'a CookieJar,
    pub proxy: Option<&'a str>,
    pub options: &'a DownloadOptions,
}

/// How the command gets its cookies.
enum CookieInput {
    None,
    /// curl's `--cookie -`.
    Stdin(String),
}

/// Upstream `_write_cookies`: the `--cookies` file, or a temporary one
/// (announced) that is removed afterwards.
fn write_cookies(
    params: &Params<'_>,
    temporary_cookies: &mut Option<tempfile::TempPath>,
) -> Result<String> {
    if let Some(path) = &params.settings.cookie_file {
        params.cookies.save(path)?;
        return Ok(path.display().to_string());
    }
    let file = tempfile::Builder::new()
        .prefix("tmp")
        .suffix(".cookies")
        .tempfile()?;
    let path = file.into_temp_path();
    crate::screen::screen(&format!(
        "[download] Writing temporary cookies file to \"{}\"",
        path.display()
    ));
    params.cookies.save(&path)?;
    let text = path.display().to_string();
    *temporary_cookies = Some(path);
    Ok(text)
}

/// `_make_cmd(tmpfilename, info_dict)`.
fn make_cmd(
    available: &Available,
    params: &Params<'_>,
    info: &VideoInfo,
    temporary: &Path,
    temporary_cookies: &mut Option<tempfile::TempPath>,
) -> Result<(Vec<String>, CookieInput)> {
    let settings = params.settings;
    let options = params.options;
    let temporary_text = temporary.display().to_string();
    let url = info.url.to_string();
    let headers = http_headers(info);
    let option = |name: &str, value: Option<String>| -> Vec<String> {
        value.map_or_else(Vec::new, |value| vec![name.to_owned(), value])
    };
    let joined = |name: &str, value: bool, yes: &str, no: &str| {
        format!("{name}={}", if value { yes } else { no })
    };
    let mut input = CookieInput::None;
    let mut cmd = vec![available.exe.clone()];
    match available.kind {
        Kind::Curl => {
            cmd.extend(["--location", "-o"].map(String::from));
            cmd.push(temporary_text);
            cmd.push("--compressed".into());
            // `_MIN_VERSION_FOR_STDIN_COOKIES`, else `/dev/fd/0`, else a file.
            let version = available.curl_version.as_slice();
            let device = std::fs::symlink_metadata("/dev/fd/0")
                .is_ok_and(|meta| meta.file_type().is_symlink());
            cmd.push("--cookie".into());
            if version >= [7, 59].as_slice() {
                cmd.push("-".into());
            } else if device {
                cmd.push("/dev/fd/0".into());
            } else {
                let file = write_cookies(params, temporary_cookies)?;
                if file.contains('=') {
                    return Err(Error::Reported(
                        "curl version too old or temp directory contains `=`; please use another downloader or update curl".into(),
                    ));
                }
                cmd.push(file);
            }
            if version > [7, 59].as_slice() || device {
                input = CookieInput::Stdin(params.cookies.netscape_lines()?);
            }
            for (key, value) in &headers {
                cmd.push("--header".into());
                cmd.push(format!("{key}: {value}"));
            }
            cmd.push("--continue-at".into());
            cmd.push(if settings.continue_download { "-" } else { "0" }.into());
            if settings.no_progress {
                cmd.push("--silent".into());
            }
            if crate::screen::verbose() {
                cmd.push("--verbose".into());
            }
            cmd.extend(option(
                "--limit-rate",
                options.rate_limit.map(|rate| rate.to_string()),
            ));
            let retries = retries_text(options.retries);
            cmd.push("--retry".into());
            cmd.push(if retries == "inf" {
                "2147483647".into()
            } else {
                retries
            });
            cmd.extend(option(
                "--max-filesize",
                options.max_filesize.map(|size| size.to_string()),
            ));
            cmd.extend(option("--interface", settings.source_address.clone()));
            cmd.extend(option("--proxy", params.proxy.map(str::to_owned)));
            if settings.no_check_certificate {
                cmd.push("--insecure".into());
            }
            cmd.extend(configuration_args(settings, available.kind));
            cmd.push("--".into());
            cmd.push(url);
        }
        Kind::Axel => {
            cmd.push("-o".into());
            cmd.push(temporary_text);
            for (key, value) in &headers {
                cmd.push("-H".into());
                cmd.push(format!("{key}: {value}"));
            }
            if let Some(cookie) = params.cookies.cookie_header(&info.url) {
                cmd.push("-H".into());
                cmd.push(format!("Cookie: {cookie}"));
                cmd.push("--max-redirect=0".into());
            }
            cmd.extend(configuration_args(settings, available.kind));
            cmd.push("--".into());
            cmd.push(url);
        }
        Kind::Wget => {
            cmd.push("-O".into());
            cmd.push(temporary_text);
            cmd.extend(["-nv", "--compression=auto", "--load-cookies"].map(String::from));
            cmd.push(write_cookies(params, temporary_cookies)?);
            for (key, value) in &headers {
                cmd.push("--header".into());
                cmd.push(format!("{key}: {value}"));
            }
            cmd.extend(option(
                "--limit-rate",
                options.rate_limit.map(|rate| rate.to_string()),
            ));
            let retries = retries_text(options.retries);
            cmd.push("--tries".into());
            cmd.push(if retries == "inf" {
                "0".into()
            } else {
                retries
            });
            cmd.extend(option("--bind-address", settings.source_address.clone()));
            if let Some(proxy) = params.proxy.filter(|proxy| !proxy.is_empty()) {
                for var in ["http_proxy", "https_proxy"] {
                    cmd.push("--execute".into());
                    cmd.push(format!("{var}={proxy}"));
                }
            }
            if settings.no_check_certificate {
                cmd.push("--no-check-certificate".into());
            }
            cmd.extend(configuration_args(settings, available.kind));
            cmd.push("--".into());
            cmd.push(url);
        }
        Kind::Aria2c => {
            cmd.extend(
                [
                    "--no-conf",
                    "--auto-save-interval=10",
                    "--console-log-level=warn",
                    "--summary-interval=0",
                    "--download-result=hide",
                    "--http-accept-gzip=true",
                    "--file-allocation=none",
                    "-x16",
                    "-j16",
                    "-s16",
                    "--min-split-size",
                    "1M",
                ]
                .map(String::from),
            );
            cmd.push(format!(
                "--load-cookies={}",
                write_cookies(params, temporary_cookies)?
            ));
            for (key, value) in &headers {
                cmd.push("--header".into());
                cmd.push(format!("{key}: {value}"));
            }
            cmd.extend(option(
                "--max-overall-download-limit",
                options.rate_limit.map(|rate| rate.to_string()),
            ));
            cmd.extend(option("--interface", settings.source_address.clone()));
            cmd.extend(option("--all-proxy", params.proxy.map(str::to_owned)));
            cmd.push(joined(
                "--check-certificate",
                settings.no_check_certificate,
                "false",
                "true",
            ));
            if let Some(update) = settings.update_time {
                cmd.push(joined("--remote-time", update, "true", "false"));
            }
            cmd.push(joined(
                "--show-console-readout",
                settings.no_progress,
                "false",
                "true",
            ));
            cmd.push(joined(
                "--remove-control-file",
                settings.continue_download,
                "false",
                "true",
            ));
            cmd.extend(configuration_args(settings, available.kind));
            cmd.extend(
                [
                    "--allow-overwrite=true",
                    "--always-resume=false",
                    "--auto-file-renaming=false",
                    "--force-save=false",
                ]
                .map(String::from),
            );
            // `./` and a trailing separator keep aria2c from stripping spaces.
            let relative = |name: String| {
                if Path::new(&name).is_absolute() {
                    name
                } else {
                    format!(".{}{name}", std::path::MAIN_SEPARATOR)
                }
            };
            if let Some(parent) = temporary
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                cmd.push("--dir".into());
                cmd.push(format!(
                    "{}{}",
                    relative(parent.display().to_string()),
                    std::path::MAIN_SEPARATOR
                ));
            }
            let name = temporary
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            cmd.push("--out".into());
            cmd.push(relative(name));
            cmd.push("--".into());
            cmd.push(url);
        }
        Kind::Httpie => {
            // Upstream runs `http` whatever path was given.
            cmd = vec![
                "http".into(),
                "--download".into(),
                "--output".into(),
                temporary_text,
                url,
            ];
            for (key, value) in &headers {
                cmd.push(format!("{key}:{value}"));
            }
            if let Some(cookie) = params.cookies.cookie_header(&info.url) {
                cmd.push(format!("Cookie:{cookie}"));
            }
        }
    }
    Ok((cmd, input))
}

/// `ExternalFD.real_download` for a whole-file format.
pub(crate) fn download(
    available: &Available,
    params: &Params<'_>,
    info: &VideoInfo,
    destination: &Path,
    callback: &mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
) -> Result<u64> {
    let options = params.options;
    crate::download::check_destination(destination, options)?;
    let parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut private = None;
    let temporary: PathBuf = match options.part_files {
        PartFiles::NoPart { .. } => destination.to_owned(),
        PartFiles::Part { .. } => {
            let mut name = destination.as_os_str().to_owned();
            name.push(".part");
            name.into()
        }
        PartFiles::Private => {
            let file = tempfile::Builder::new()
                .prefix(".yt-dlp-rs-")
                .suffix(".part")
                .tempfile_in(parent)?;
            let path = file.path().to_owned();
            private = Some(file);
            path
        }
    };
    let progress = |phase, downloaded_bytes, total_bytes| DownloadProgress {
        attempt: 1,
        phase,
        downloaded_bytes,
        total_bytes,
        stream_index: None,
        stream_count: None,
        fragment_index: None,
        fragment_count: None,
        fragment: None,
    };
    // `report_destination`.
    if callback(progress(DownloadPhase::Downloading, 0, None)).is_break() {
        return Err(Error::DownloadCancelled);
    }
    let mut temporary_cookies = None;
    let (cmd, input) = make_cmd(available, params, info, &temporary, &mut temporary_cookies)?;
    let args: Vec<OsString> = cmd[1..].iter().map(OsString::from).collect();
    crate::screen::debug_named_command(&cmd[0], &args);
    let mut command = Command::new(&cmd[0]);
    command.args(&args);
    // Every class but curl captures stderr (curl writes its progress there).
    let capture = available.kind != Kind::Curl;
    if capture {
        command.stderr(Stdio::piped());
    }
    if matches!(input, CookieInput::Stdin(_)) {
        command.stdin(Stdio::piped());
    }
    let mut child = command.spawn().map_err(|error| {
        Error::Reported(format!(
            "{} could not be run: {error}",
            available.kind.basename()
        ))
    })?;
    if let CookieInput::Stdin(text) = input
        && let Some(mut stdin) = child.stdin.take()
    {
        let _ = stdin.write_all(text.as_bytes());
    }
    let stderr_reader = child.stderr.take().map(|mut stderr| {
        std::thread::spawn(move || {
            let mut text = Vec::new();
            let _ = stderr.read_to_end(&mut text);
            text
        })
    });
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if callback(progress(DownloadPhase::Downloading, 0, None)).is_break() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::DownloadCancelled);
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let stderr = stderr_reader
        .and_then(|reader| reader.join().ok())
        .unwrap_or_default();
    drop(temporary_cookies);
    let code = status.code().unwrap_or(-1);
    if code != 0 {
        if !stderr.is_empty() {
            crate::output::encoding::write_message(
                true,
                &format!("{}\n", String::from_utf8_lossy(&stderr)),
            );
        }
        if private.is_some() {
            let _ = std::fs::remove_file(&temporary);
        }
        crate::output::encoding::write_message(true, "\n\n");
        let _ = callback(progress(DownloadPhase::Failed, 0, None));
        return Err(Error::Reported(format!(
            "{} exited with code {code}",
            available.kind.basename()
        )));
    }
    let size = std::fs::metadata(&temporary)?.len();
    match private {
        Some(file) => crate::download::persist(file, destination, options.overwrite)?,
        None if temporary != destination => std::fs::rename(&temporary, destination)?,
        None => {}
    }
    if callback(progress(DownloadPhase::Completed, size, Some(size))).is_break() {
        return Err(Error::DownloadCancelled);
    }
    Ok(size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_configuration_keys_follow_upstream() {
        assert_eq!(Kind::from_name("/usr/bin/curl"), Some(Kind::Curl));
        assert_eq!(Kind::from_name("aria2c.exe"), Some(Kind::Aria2c));
        assert_eq!(Kind::from_name("httpie"), Some(Kind::Httpie));
        // `EXE_NAME in bn`: `http` is httpie's executable.
        assert_eq!(Kind::from_name("myhttp"), Some(Kind::Httpie));
        assert_eq!(Kind::from_name("x"), None);
        assert_eq!(version_tuple("8.7.1"), [8, 7, 1]);
        let mut settings = ExternalSettings::default();
        settings.args.insert("default".into(), vec!["-d".into()]);
        settings.args.insert("http".into(), vec!["-h".into()]);
        assert_eq!(configuration_args(&settings, Kind::Curl), ["-d"]);
        assert_eq!(configuration_args(&settings, Kind::Httpie), ["-h"]);
        settings
            .args
            .insert("httpie+http".into(), vec!["-r".into()]);
        assert_eq!(configuration_args(&settings, Kind::Httpie), ["-r"]);
    }
}

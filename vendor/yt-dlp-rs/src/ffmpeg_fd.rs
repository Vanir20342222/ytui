//! Upstream `FFmpegFD` for partial (`--download-sections`) downloads: FFmpeg
//! reads the selected format URLs itself (`-ss`/`-t` input options, stream
//! copy) and writes the destination's temporary file. FFmpeg's own output is
//! inherited like upstream's `Popen`.
use crate::{
    CookieJar, DownloadOptions, DownloadPhase, DownloadProgress, Error, PartFiles, Result,
    VideoInfo, info::PyNumber,
};
use reqwest::{
    cookie::CookieStore,
    header::{COOKIE, HeaderMap, HeaderValue, REFERER, USER_AGENT},
};
use std::{
    ffi::OsString,
    ops::ControlFlow,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

/// Upstream `EXT_TO_OUT_FORMATS`.
fn out_format(ext: &str) -> &str {
    match ext {
        "aac" => "adts",
        "flac" => "flac",
        "m4a" => "ipod",
        "mka" | "mkv" => "matroska",
        "mpg" => "mpeg",
        "ogv" => "ogg",
        "ts" => "mpegts",
        "wma" | "wmv" => "asf",
        "weba" => "webm",
        "vtt" => "webvtt",
        other => other,
    }
}

/// Python `str()` of a number.
fn py_str(value: PyNumber) -> String {
    match value {
        PyNumber::Int(value) => value.to_string(),
        PyNumber::Float(value) => crate::screen::py_float(value),
    }
}

fn difference(end: PyNumber, start: PyNumber) -> PyNumber {
    match (end, start) {
        (PyNumber::Int(end), PyNumber::Int(start)) => PyNumber::Int(end - start),
        (end, start) => PyNumber::Float(end.as_f64() - start.as_f64()),
    }
}

/// Request settings FFmpeg needs for network inputs.
pub(crate) struct Network<'a> {
    pub headers: &'a HeaderMap,
    pub cookies: &'a CookieJar,
    pub proxy: Option<&'a str>,
}

/// FFmpeg settings from the downloader parameters.
#[derive(Debug, Clone, Default)]
pub(crate) struct Settings {
    /// `--downloader-args` by key (`ffmpeg`, `ffmpeg_i1`, `ffmpeg_o`, ...).
    pub args: std::collections::HashMap<String, Vec<String>>,
    /// `-loglevel quiet` (upstream `quiet`).
    pub quiet: bool,
    /// `--hls-use-mpegts`: MPEG-TS instead of MP4 for HLS inputs.
    pub hls_use_mpegts: Option<bool>,
}

/// The `-headers` value for one input (title-cased names): the defaults, a User-Agent,
/// the page Referer of non-direct media and the jar's cookies.
fn header_lines(network: &Network<'_>, info: &VideoInfo, url: &url::Url) -> String {
    let mut headers = network.headers.clone();
    if !headers.contains_key(USER_AGENT) {
        headers.insert(
            USER_AGENT,
            HeaderValue::from_static(concat!("yt-dlp-rs/", env!("CARGO_PKG_VERSION"))),
        );
    }
    if !info.is_direct()
        && !headers.contains_key(REFERER)
        && let Ok(value) = HeaderValue::from_str(info.webpage_url.as_str())
    {
        headers.insert(REFERER, value);
    }
    if let Some(cookie) = network.cookies.cookies(url) {
        headers.insert(COOKIE, cookie);
    }
    headers
        .iter()
        .map(|(name, value)| {
            format!(
                "{}: {}\r\n",
                py_title(name.as_str()),
                String::from_utf8_lossy(value.as_bytes())
            )
        })
        .collect()
}

/// Python `str.title()` for header names, as `HTTPHeaderDict` stores them.
pub(crate) fn py_title(name: &str) -> String {
    let mut previous_letter = false;
    name.chars()
        .map(|c| {
            let mapped = if previous_letter {
                c.to_ascii_lowercase()
            } else {
                c.to_ascii_uppercase()
            };
            previous_letter = c.is_ascii_alphabetic();
            mapped
        })
        .collect()
}

/// `FFmpegFD._call_downloader` arguments, without the executable.
/// Upstream `cli_configuration_args`: the first key with arguments.
fn configured<'a>(settings: &'a Settings, keys: &[String]) -> &'a [String] {
    keys.iter()
        .find_map(|key| settings.args.get(key))
        .map_or(&[], Vec::as_slice)
}

fn arguments(
    network: &Network<'_>,
    settings: &Settings,
    info: &VideoInfo,
    temporary: &Path,
) -> Result<Vec<OsString>> {
    let mut args: Vec<OsString> = vec!["-y".into()];
    let verbose = crate::screen::verbose();
    if settings.quiet {
        args.extend(["-loglevel".into(), "quiet".into()]);
    } else if verbose {
        args.extend(["-loglevel".into(), "verbose".into()]);
    }
    if !verbose {
        args.push("-hide_banner".into());
    }
    let start = info
        .section_start
        .filter(|start| start.as_f64() != 0.0)
        .unwrap_or(PyNumber::Int(0));
    let end = info.section_end.filter(|end| end.as_f64() != 0.0);
    // Each input with its `manifest_stream_number`.
    let formats: Vec<(VideoInfo, usize)> = if info.requested_formats.is_empty() {
        let number = info
            .formats
            .iter()
            .find(|format| info.format_id.as_deref() == Some(format.format_id.as_str()))
            .and_then(|format| format.manifest_stream_number);
        vec![(info.clone(), number.unwrap_or(0))]
    } else {
        info.requested_formats
            .iter()
            .map(|format| {
                (
                    info.with_selected_format(format),
                    format.manifest_stream_number.unwrap_or(0),
                )
            })
            .collect()
    };
    let mut protocol = String::new();
    for (input, (format, _)) in formats.iter().enumerate() {
        protocol = format.protocol.clone().unwrap_or_default();
        if matches!(format.url.scheme(), "http" | "https") {
            let headers = header_lines(network, format, &format.url);
            if !headers.is_empty() {
                args.extend(["-headers".into(), headers.into()]);
            }
        }
        if start.as_f64() != 0.0 {
            args.extend(["-ss".into(), py_str(start).into()]);
        }
        if let Some(end) = end {
            args.extend(["-t".into(), py_str(difference(end, start)).into()]);
        }
        // `_configuration_args(('_i1', '_i'))` before each input.
        let keys = [format!("ffmpeg_i{}", input + 1), "ffmpeg_i".into()];
        args.extend(configured(settings, &keys).iter().map(OsString::from));
        args.extend(["-i".into(), format.url.as_str().into()]);
    }
    args.extend(["-c".into(), "copy".into()]);
    if !info.requested_formats.is_empty() || protocol == "http_dash_segments" {
        for (index, (_, number)) in formats.iter().enumerate() {
            args.extend(["-map".into(), format!("{index}:{number}").into()]);
        }
    }
    let format = if protocol.starts_with("m3u8") {
        // `use_mpegts`: `--hls-use-mpegts`, else whether the stream is live.
        if settings
            .hls_use_mpegts
            .unwrap_or_else(|| crate::is_live(info))
        {
            "mpegts"
        } else {
            "mp4"
        }
    } else {
        out_format(&info.ext)
    };
    args.extend(["-f".into(), format.into()]);
    // `_configuration_args(('_o1', '_o', ''))` (with `default`).
    let keys = ["ffmpeg_o1", "ffmpeg_o", "ffmpeg", "default"].map(String::from);
    args.extend(configured(settings, &keys).iter().map(OsString::from));
    // `_ffmpeg_filename_argument`.
    let mut output = OsString::from("file:");
    output.push(temporary);
    args.push(output);
    Ok(args)
}

/// `ExternalFD.real_download` with `FFmpegFD`: run FFmpeg into the
/// temporary name, then publish it and report one finished status.
pub(crate) fn download(
    ffmpeg: &Path,
    network: &Network<'_>,
    settings: &Settings,
    info: &VideoInfo,
    destination: &Path,
    options: &DownloadOptions,
    callback: &mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
) -> Result<u64> {
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
    let args = arguments(network, settings, info, &temporary)?;
    let progress = |phase, downloaded_bytes| DownloadProgress {
        attempt: 1,
        phase,
        downloaded_bytes,
        total_bytes: None,
        stream_index: None,
        stream_count: None,
        fragment_index: None,
        fragment_count: None,
        fragment: None,
    };
    if callback(progress(DownloadPhase::Downloading, 0)).is_break() {
        return Err(Error::DownloadCancelled);
    }
    crate::screen::debug_command(ffmpeg, &args);
    let mut command = Command::new(ffmpeg);
    command.args(&args).stdin(Stdio::piped());
    if let Some(proxy) = network.proxy.filter(|proxy| !proxy.is_empty()) {
        let proxy = if proxy.contains("://") {
            proxy.to_owned()
        } else {
            format!("http://{proxy}")
        };
        command.env("HTTP_PROXY", &proxy).env("http_proxy", &proxy);
    }
    let mut child = command.spawn()?;
    // Upstream keeps FFmpeg's stdin open (a pipe) so it never reads the terminal.
    let _stdin = child.stdin.take();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if callback(progress(DownloadPhase::Downloading, 0)).is_break() {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_file(&temporary);
            return Err(Error::DownloadCancelled);
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    if !status.success() {
        if private.is_some() {
            let _ = std::fs::remove_file(&temporary);
        }
        crate::output::encoding::write_message(true, "\n\n");
        let _ = callback(progress(DownloadPhase::Failed, 0));
        return Err(Error::Reported(format!(
            "ffmpeg exited with code {}",
            status.code().unwrap_or(-1)
        )));
    }
    let size = std::fs::metadata(&temporary)?.len();
    match private {
        Some(file) => {
            crate::download::persist(file, destination, options.overwrite)?;
        }
        None if temporary != destination => std::fs::rename(&temporary, destination)?,
        None => {}
    }
    if callback(progress(DownloadPhase::Completed, size)).is_break() {
        return Err(Error::DownloadCancelled);
    }
    Ok(size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn arguments_follow_ffmpeg_fd() {
        let mut info: VideoInfo = serde_json::from_value(json!({"id":"v","title":"V",
            "url":"https://example.test/v.mp4","webpage_url":"https://example.test/page",
            "ext":"mp4","extractor":"html5","extractor_key":"HTML5MediaEmbed","protocol":"https"}))
        .unwrap();
        info.section_start = Some(PyNumber::Int(2));
        info.section_end = Some(PyNumber::Float(4.5));
        let headers = HeaderMap::new();
        let cookies = CookieJar::default();
        let network = Network {
            headers: &headers,
            cookies: &cookies,
            proxy: None,
        };
        let args = arguments(
            &network,
            &Settings::default(),
            &info,
            Path::new("o.mp4.part"),
        )
        .unwrap();
        let args: Vec<_> = args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            [
                "-y",
                "-hide_banner",
                "-headers",
                concat!(
                    "User-Agent: yt-dlp-rs/",
                    env!("CARGO_PKG_VERSION"),
                    "\r\nReferer: https://example.test/page\r\n"
                ),
                "-ss",
                "2",
                "-t",
                "2.5",
                "-i",
                "https://example.test/v.mp4",
                "-c",
                "copy",
                "-f",
                "mp4",
                "file:o.mp4.part"
            ]
        );
        info.ext = "m4a".into();
        info.section_start = None;
        let args = arguments(
            &network,
            &Settings {
                quiet: true,
                ..Default::default()
            },
            &info,
            Path::new("o"),
        )
        .unwrap();
        let args: Vec<_> = args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(&args[..4], ["-y", "-loglevel", "quiet", "-hide_banner"]);
        assert!(args.windows(2).any(|pair| pair == ["-t", "4.5"]));
        assert!(!args.contains(&"-ss".to_owned()));
        assert!(args.windows(2).any(|pair| pair == ["-f", "ipod"]));
    }

    #[test]
    fn downloader_args_follow_configuration_args_keys() {
        let info: VideoInfo = serde_json::from_value(json!({"id":"v","title":"V",
            "url":"https://example.test/v.mp4","webpage_url":"https://example.test/v.mp4",
            "ext":"mp4","extractor":"generic","extractor_key":"Generic","direct":true,"protocol":"https"}))
        .unwrap();
        let headers = HeaderMap::new();
        let cookies = CookieJar::default();
        let network = Network {
            headers: &headers,
            cookies: &cookies,
            proxy: None,
        };
        let mut settings = Settings::default();
        settings.args.insert("ffmpeg_i".into(), vec!["-re".into()]);
        settings
            .args
            .insert("ffmpeg_i1".into(), vec!["-ss".into(), "1".into()]);
        settings.args.insert("default".into(), vec!["-x".into()]);
        settings.args.insert("ffmpeg".into(), vec!["-y2".into()]);
        let args: Vec<_> = arguments(&network, &settings, &info, Path::new("o"))
            .unwrap()
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        // `_i1` wins over `_i` before the input; `ffmpeg` over `default` before the output.
        let input = args.iter().position(|arg| arg == "-i").unwrap();
        assert_eq!(&args[input - 2..input], ["-ss", "1"]);
        assert!(!args.contains(&"-re".to_owned()) && !args.contains(&"-x".to_owned()));
        assert_eq!(&args[args.len() - 2..], ["-y2", "file:o"]);
    }

    #[test]
    fn dash_inputs_map_their_manifest_stream_numbers() {
        let format = |id: &str, number: usize, kind: &str| {
            json!({"format_id": id, "url": "https://example.test/manifest.mpd", "ext": "mp4",
                "protocol": "http_dash_segments", "stream_kind": kind, "manifest_stream_number": number})
        };
        let mut info: VideoInfo = serde_json::from_value(json!({"id":"v","title":"V",
            "url":"https://example.test/manifest.mpd","webpage_url":"https://example.test/manifest.mpd",
            "ext":"mp4","extractor":"generic","extractor_key":"Generic","direct":true,
            "protocol":"http_dash_segments","format_id":"a",
            "formats":[format("v", 0, "video_only"), format("a", 1, "audio_only")]}))
        .unwrap();
        info.section_end = Some(PyNumber::Int(4));
        let headers = HeaderMap::new();
        let cookies = CookieJar::default();
        let network = Network {
            headers: &headers,
            cookies: &cookies,
            proxy: None,
        };
        let render = |info: &VideoInfo| -> Vec<String> {
            arguments(&network, &Settings::default(), info, Path::new("o"))
                .unwrap()
                .iter()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect()
        };
        // Upstream maps a single DASH format's stream too.
        let single = render(&info);
        assert!(
            single.windows(2).any(|pair| pair == ["-map", "0:1"]),
            "{single:?}"
        );
        info.requested_formats = info.formats.clone();
        let pair = render(&info);
        let maps: Vec<_> = pair
            .windows(2)
            .filter(|pair| pair[0] == "-map")
            .map(|pair| pair[1].clone())
            .collect();
        assert_eq!(maps, ["0:0", "1:1"]);
    }
}

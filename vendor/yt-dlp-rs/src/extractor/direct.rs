use super::Extractor;
use crate::{Error, Result, VideoInfo};
use percent_encoding::percent_decode_str;
use reqwest::{
    blocking::{Client, Response},
    header::{ACCEPT_ENCODING, CONTENT_TYPE, REFERER},
};
use url::Url;

pub struct DirectHttp;

/// An extraction request upstream also makes (`_request_webpage`), subject
/// to `--sleep-requests`.
pub(crate) fn request(client: &Client, url: &Url, referer: Option<&Url>) -> Result<Response> {
    request_with_headers(client, url, referer, &[])
}

/// [`request`] with an extractor's `headers=`.
pub(crate) fn request_with_headers(
    client: &Client,
    url: &Url,
    referer: Option<&Url>,
    headers: &[(&str, &str)],
) -> Result<Response> {
    crate::screen::before_extraction_request();
    probe_with_headers(client, url, referer, headers)
}

/// [`request`] that also accepts the `expected` error statuses
/// (`expected_status=`).
pub(crate) fn request_expecting(
    client: &Client,
    url: &Url,
    headers: &[(&str, &str)],
    expected: &[u16],
) -> Result<Response> {
    crate::screen::before_extraction_request();
    let mut request = client.get(url.clone()).header(ACCEPT_ENCODING, "identity");
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    crate::screen::load_cookies()?;
    let response = request.send();
    crate::screen::flush_traffic();
    let response = response?;
    if expected.contains(&response.status().as_u16()) {
        return Ok(response);
    }
    let response = crate::retry::check_response(response)?;
    if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
        return Err(Error::PartialResponse);
    }
    Ok(response)
}

/// An extractor's `HEADRequest(url)` with headers (redirects followed).
pub(crate) fn head_with_headers(
    client: &Client,
    url: &Url,
    headers: &[(&str, &str)],
) -> Result<Response> {
    crate::screen::before_extraction_request();
    let mut request = client.head(url.clone());
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    crate::screen::load_cookies()?;
    let response = request.send();
    crate::screen::flush_traffic();
    crate::retry::check_response(response?)
}

/// An extractor's POST request with a body (`data=`) and headers.
pub(crate) fn post(
    client: &Client,
    url: &Url,
    headers: &[(&str, &str)],
    body: String,
) -> Result<Response> {
    post_expecting(client, url, headers, body, &[])
}

/// [`post`] answering the `expected` statuses instead of erroring.
pub(crate) fn post_expecting(
    client: &Client,
    url: &Url,
    headers: &[(&str, &str)],
    body: String,
    expected: &[u16],
) -> Result<Response> {
    crate::screen::before_extraction_request();
    let mut request = client
        .post(url.clone())
        .header(ACCEPT_ENCODING, "identity")
        .body(body);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    crate::screen::load_cookies()?;
    let response = request.send();
    crate::screen::flush_traffic();
    let response = response?;
    if expected.contains(&response.status().as_u16()) {
        return Ok(response);
    }
    crate::retry::check_response(response)
}

/// A request this port adds to inspect media or child playlists that
/// upstream does not fetch during extraction; never delayed.
pub(crate) fn probe(client: &Client, url: &Url, referer: Option<&Url>) -> Result<Response> {
    probe_with_headers(client, url, referer, &[])
}

fn probe_with_headers(
    client: &Client,
    url: &Url,
    referer: Option<&Url>,
    headers: &[(&str, &str)],
) -> Result<Response> {
    let mut request = client.get(url.clone()).header(ACCEPT_ENCODING, "identity");
    if let Some(referer) = referer {
        request = request.header(REFERER, referer.as_str());
    }
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    crate::screen::load_cookies()?;
    let response = request.send();
    crate::screen::flush_traffic();
    let response = crate::retry::check_response(response?)?;
    if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
        return Err(Error::PartialResponse);
    }
    Ok(response)
}

pub(crate) fn response_mime(response: &Response) -> String {
    response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}

// Manifests need their own downloader; never save an HLS/DASH playlist as a video.
pub(crate) fn media_extension(value: &str) -> Option<&'static str> {
    match value.to_ascii_lowercase().as_str() {
        "mp4" | "m4v" => Some("mp4"),
        "webm" => Some("webm"),
        "mkv" => Some("mkv"),
        "mov" => Some("mov"),
        "avi" => Some("avi"),
        "flv" => Some("flv"),
        "mp3" => Some("mp3"),
        "m4a" => Some("m4a"),
        "aac" => Some("aac"),
        "ogg" | "oga" => Some("ogg"),
        "opus" => Some("opus"),
        "wav" => Some("wav"),
        "flac" => Some("flac"),
        _ => None,
    }
}

fn mime_extension(mime: &str) -> Option<&'static str> {
    match mime {
        "video/mp4" => Some("mp4"),
        "video/webm" | "audio/webm" => Some("webm"),
        "video/x-matroska" => Some("mkv"),
        "video/quicktime" => Some("mov"),
        "video/x-msvideo" => Some("avi"),
        "video/x-flv" => Some("flv"),
        "audio/mpeg" => Some("mp3"),
        "audio/mp4" | "audio/x-m4a" => Some("m4a"),
        "audio/aac" => Some("aac"),
        "audio/ogg" | "video/ogg" | "application/ogg" => Some("ogg"),
        "audio/opus" => Some("opus"),
        "audio/wav" | "audio/x-wav" => Some("wav"),
        "audio/flac" | "audio/x-flac" => Some("flac"),
        _ => None,
    }
}

impl Extractor for DirectHttp {
    fn key(&self) -> &'static str {
        "Generic"
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        // GET rather than HEAD: some media servers reject HEAD or return different headers.
        // The body remains streamed and is dropped after inspecting the headers.
        let response = request(client, url, None)?;
        let timestamp = last_modified(&response);
        let mime = response_mime(&response);
        Ok(from_response(&response, url).map(|mut info| {
            info.metadata.timestamp = timestamp.map(crate::info::PyNumber::Int);
            with_format(info, &mime)
        }))
    }
}

/// Upstream generic direct results carry one format: a matching media
/// Content-Type names it by its subtype (with `vcodec: none` for audio);
/// otherwise `process_video_result` numbers the bare result `0`.
pub(crate) fn with_format(mut info: VideoInfo, mime: &str) -> VideoInfo {
    let (kind, subtype) = mime.split_once('/').unwrap_or((mime, ""));
    let media = matches!(kind, "audio" | "video") || mime == "application/ogg";
    let format_id = if media && !subtype.is_empty() {
        subtype.to_owned()
    } else {
        "0".to_owned()
    };
    if kind == "audio" {
        info.stream_kind = crate::StreamKind::AudioOnly;
    }
    info.format_id = Some(format_id.clone());
    info.formats = vec![crate::VideoFormat {
        http_headers: None,
        extra: Default::default(),
        protocol: None,
        vcodec: (kind == "audio").then(|| "none".to_owned()),
        acodec: None,
        dynamic_range: None,
        video_ext: None,
        audio_ext: None,
        vbr: None,
        source_preference: None,
        quality: None,
        fps: None,
        asr: None,
        audio_channels: None,
        resolution: None,
        format: None,
        aspect_ratio: None,
        format_id,
        stream_kind: info.stream_kind,
        audio_group: None,
        language: None,
        format_note: None,
        audio_default: false,
        audio_autoselect: false,
        downloader_options: None,
        manifest_stream_number: None,
        format_index: None,
        container: None,
        abr: None,
        tbr: None,
        url: info.url.clone(),
        ext: info.ext.clone(),
        filesize: info.filesize,
        fragments: Vec::new(),
        width: None,
        height: None,
    }];
    if media {
        crate::info_order::mark_format(&mut info.formats[0], "direct");
    }
    info
}

/// Generic `unified_timestamp(response.headers.get('Last-Modified'))`.
pub(crate) fn last_modified(response: &Response) -> Option<i64> {
    let value = response.headers().get(reqwest::header::LAST_MODIFIED)?;
    // Python decodes header bytes as Latin-1.
    let text: String = value.as_bytes().iter().map(|&b| char::from(b)).collect();
    crate::dates::unified_timestamp(&text, true)
}

pub(crate) fn from_response(response: &Response, url: &Url) -> Option<VideoInfo> {
    let final_url = response.url().clone();
    let mime = response_mime(response);
    let filename = final_url
        .path_segments()
        .and_then(|mut p| p.next_back())
        .unwrap_or("");
    let filename = percent_decode_str(filename).decode_utf8_lossy();
    let suffix = filename
        .rsplit_once('.')
        .and_then(|(_, ext)| media_extension(ext));
    // A misleading .mp4 URL serving HTML is not a direct media result.
    let ext = mime_extension(&mime).or_else(|| {
        if mime.is_empty() || mime == "application/octet-stream" {
            suffix
        } else {
            None
        }
    });
    Some(build(response, url, ext?.to_owned()))
}

/// Upstream `KNOWN_EXTENSIONS`.
pub(crate) fn known_extension(ext: &str) -> bool {
    KNOWN.contains(&ext)
}

const KNOWN: &[&str] = &[
    "avi", "flv", "mkv", "mov", "mp4", "webm", "3g2", "3gp", "f4v", "mk3d", "divx", "mpg", "ogv",
    "m4v", "wmv", "aiff", "alac", "flac", "m4a", "mka", "mp3", "ogg", "opus", "wav", "aac", "ape",
    "asf", "f4a", "f4b", "m4b", "m4r", "oga", "ogx", "spx", "vorbis", "wma", "weba", "f4f", "f4m",
    "m3u8", "smil", "mpd",
];

/// Upstream `determine_ext(url, default_ext=None)`.
pub(crate) fn determine_ext(url: &str) -> Option<String> {
    if !url.contains('.') {
        return None;
    }
    let path = url.split('?').next().unwrap_or("");
    let guess = path.rsplit_once('.').map_or(path, |(_, guess)| guess);
    if !guess.is_empty() && guess.bytes().all(|b| b.is_ascii_alphanumeric()) {
        Some(guess.to_owned())
    } else if KNOWN.contains(&guess.trim_end_matches('/')) {
        Some(guess.trim_end_matches('/').to_owned())
    } else {
        None
    }
}

/// Upstream `mimetype2ext(mime, default)`.
pub(crate) fn mimetype2ext_or(mime: &str, default: Option<&str>) -> String {
    const MAP: &[(&str, &str)] = &[
        ("3gpp", "3gp"),
        ("mp2t", "ts"),
        ("mp4", "mp4"),
        ("mpeg", "mpeg"),
        ("mpegurl", "m3u8"),
        ("quicktime", "mov"),
        ("webm", "webm"),
        ("vp9", "vp9"),
        ("video/ogg", "ogv"),
        ("x-flv", "flv"),
        ("x-m4v", "m4v"),
        ("x-matroska", "mkv"),
        ("x-mng", "mng"),
        ("x-mp4-fragmented", "mp4"),
        ("x-ms-asf", "asf"),
        ("x-ms-wmv", "wmv"),
        ("x-msvideo", "avi"),
        ("vnd.dlna.mpeg-tts", "mpeg"),
        ("dash+xml", "mpd"),
        ("f4m+xml", "f4m"),
        ("hds+xml", "f4m"),
        ("vnd.apple.mpegurl", "m3u8"),
        ("vnd.ms-sstr+xml", "ism"),
        ("x-mpegurl", "m3u8"),
        ("audio/mp4", "m4a"),
        ("audio/mpeg", "mp3"),
        ("audio/webm", "webm"),
        ("audio/x-matroska", "mka"),
        ("audio/x-mpegurl", "m3u"),
        ("aacp", "aac"),
        ("flac", "flac"),
        ("midi", "mid"),
        ("ogg", "ogg"),
        ("wav", "wav"),
        ("wave", "wav"),
        ("x-aac", "aac"),
        ("x-flac", "flac"),
        ("x-m4a", "m4a"),
        ("x-realaudio", "ra"),
        ("x-wav", "wav"),
        ("avif", "avif"),
        ("bmp", "bmp"),
        ("gif", "gif"),
        ("jpeg", "jpg"),
        ("png", "png"),
        ("svg+xml", "svg"),
        ("tiff", "tif"),
        ("vnd.wap.wbmp", "wbmp"),
        ("webp", "webp"),
        ("x-icon", "ico"),
        ("x-jng", "jng"),
        ("x-ms-bmp", "bmp"),
        ("filmstrip+json", "fs"),
        ("smptett+xml", "tt"),
        ("ttaf+xml", "dfxp"),
        ("ttml+xml", "ttml"),
        ("x-ms-sami", "sami"),
        ("x-subrip", "srt"),
        ("x-srt", "srt"),
        ("gzip", "gz"),
        ("json", "json"),
        ("xml", "xml"),
        ("zip", "zip"),
    ];
    let mimetype = mime.split(';').next().unwrap_or("").trim().to_lowercase();
    let subtype = mimetype
        .rsplit_once('/')
        .map_or(mimetype.as_str(), |(_, s)| s);
    let lookup = |key: &str| MAP.iter().find(|(k, _)| *k == key).map(|(_, v)| *v);
    let last = subtype.rsplit('+').next().unwrap_or(subtype);
    lookup(&mimetype)
        .or_else(|| lookup(subtype))
        .or_else(|| lookup(last))
        .map(str::to_owned)
        .or_else(|| default.map(str::to_owned))
        .unwrap_or_else(|| subtype.replace('+', "."))
}

/// Upstream `disposition_filename`: the Content-Disposition `filename*`
/// (RFC 2231/5987) or `filename` parameter.
fn disposition_filename(response: &Response) -> Option<String> {
    let header = response
        .headers()
        .get(reqwest::header::CONTENT_DISPOSITION)?;
    let text: String = header.as_bytes().iter().map(|&b| char::from(b)).collect();
    let mut plain = None;
    for parameter in text.split(';').skip(1) {
        let Some((name, value)) = parameter.split_once('=') else {
            continue;
        };
        let name = name.trim().to_lowercase();
        let value = value.trim();
        if name == "filename*" {
            // charset'language'percent-encoded
            let encoded = value.splitn(3, '\'').nth(2).unwrap_or(value);
            return Some(
                percent_decode_str(encoded.trim_matches('"'))
                    .decode_utf8_lossy()
                    .trim()
                    .to_owned(),
            );
        }
        if name == "filename" && plain.is_none() {
            plain = Some(value.trim_matches('"').trim().to_owned());
        }
    }
    plain
}

/// Upstream `urlhandle_detect_ext`.
pub(crate) fn urlhandle_detect_ext(response: &Response, default: Option<&str>) -> String {
    let header = |name: &str| {
        response
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    };
    disposition_filename(response)
        .and_then(|name| determine_ext(&name))
        .or_else(|| header("x-amz-meta-name").and_then(|name| determine_ext(&name)))
        .or_else(|| header("x-amz-meta-file-type"))
        .unwrap_or_else(|| match header("content-type") {
            Some(mime) => mimetype2ext_or(&mime, default),
            None => default.unwrap_or_default().to_owned(),
        })
}

/// Generic's direct-link result: `ext` is `determine_ext(url)` of the
/// requested URL, else `urlhandle_detect_ext`.
pub(crate) fn generic_direct(response: &Response, url: &Url, default: Option<&str>) -> VideoInfo {
    let ext =
        determine_ext(url.as_str()).unwrap_or_else(|| urlhandle_detect_ext(response, default));
    build(response, url, ext)
}

/// Generic's untyped non-HTML response ("URL could be a direct video
/// link"): `determine_ext(url)` if allowed, else `unknown_video`.
pub(crate) fn generic_untyped(response: &Response, url: &Url) -> VideoInfo {
    let ext = determine_ext(url.as_str())
        .filter(|ext| crate::output::allowed_extension(ext))
        .unwrap_or_else(|| "unknown_video".into());
    build(response, url, ext)
}

fn build(response: &Response, url: &Url, ext: String) -> VideoInfo {
    build_at(response.url().clone(), url, ext)
}

/// Python `mimetypes.guess_type` for the media extensions a `file:` URL
/// can name.
fn guess_file_type(path: &str) -> Option<&'static str> {
    let ext = path.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match ext.as_str() {
        "mp4" => "video/mp4",
        "m4v" => "video/mp4",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "mov" => "video/quicktime",
        "avi" => "video/x-msvideo",
        "flv" => "video/x-flv",
        "ts" => "video/mp2t",
        "mpg" | "mpeg" => "video/mpeg",
        "3gp" => "video/3gpp",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        "opus" => "audio/opus",
        "ogg" | "oga" => "audio/ogg",
        "wav" => "audio/x-wav",
        "flac" => "audio/flac",
        "weba" => "audio/webm",
        _ => return None,
    })
}

/// Upstream generic extraction of a `file:` URL (`--enable-file-urls`):
/// urllib's file handler supplies the guessed Content-Type and the file's
/// mtime as Last-Modified, so media files are direct links.
pub(crate) fn from_file(url: &Url) -> crate::Result<VideoInfo> {
    let path = url
        .to_file_path()
        .map_err(|_| crate::Error::Reported(format!("[generic] Unable to open {url}")))?;
    let metadata = std::fs::metadata(&path).map_err(|error| {
        crate::Error::Reported(format!(
            "[generic] {}: Unable to download webpage: {error}",
            crate::screen::generic_id(url.as_str())
        ))
    })?;
    let mime = guess_file_type(url.path())
        .filter(|_| metadata.is_file())
        .ok_or_else(|| {
            crate::Error::UnsupportedHtml5(
                "file: URLs are only supported for direct media files natively".into(),
            )
        })?;
    let ext = determine_ext(url.as_str()).unwrap_or_else(|| mimetype2ext_or(mime, None));
    let mut info = crate::info_order::mark_info(
        with_format(build_at(url.clone(), url, ext), mime),
        "generic-direct",
    );
    info.protocol = Some("file".into());
    for format in &mut info.formats {
        format.protocol = Some("file".into());
    }
    info.metadata.timestamp = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|age| age.as_secs() as i64)
        .map(crate::info::PyNumber::Int);
    Ok(info)
}

pub(crate) fn build_at(final_url: Url, url: &Url, ext: String) -> VideoInfo {
    let filename = final_url
        .path_segments()
        .and_then(|mut p| p.next_back())
        .unwrap_or("");
    let filename = percent_decode_str(filename).decode_utf8_lossy();
    let stem = filename
        .rsplit_once('.')
        .map_or(filename.as_ref(), |(stem, _)| stem);
    let title = if stem.is_empty() { "video" } else { stem };
    VideoInfo {
        protocol: None,
        format_sort_fields: Vec::new(),
        resolution: None,
        format: None,
        aspect_ratio: None,
        fps: None,
        tbr: None,
        vbr: None,
        abr: None,
        asr: None,
        audio_channels: None,
        vcodec: None,
        acodec: None,
        dynamic_range: None,
        container: None,
        section_start: None,
        section_end: None,
        section_title: None,
        section_number: None,
        extra: Default::default(),
        downloader_options: None,
        video_ext: None,
        audio_ext: None,
        metadata: Default::default(),
        automatic_captions: Default::default(),
        subtitles: Default::default(),
        requested_subtitles: None,
        last_playlist_index: None,
        playlist_index: None,
        playlist_autonumber: None,
        playlist_count: None,
        n_entries: None,
        fragments: Vec::new(),
        id: title.to_owned(),
        title: title.to_owned(),
        fulltitle: None,
        ext: ext.to_owned(),
        url: final_url,
        webpage_url: url.clone(),
        extractor: "generic".into(),
        extractor_key: "Generic".into(),
        direct: Some(true),
        formats: Vec::new(),
        stream_kind: crate::StreamKind::Standalone,
        requested_formats: Vec::new(),
        format_id: None,
        // Upstream's generic direct format has no filesize; the transfer's
        // Content-Length is only known to the downloader.
        filesize: None,
        width: None,
        height: None,
    }
}

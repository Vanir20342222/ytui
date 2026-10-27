//! Bounded static DASH acquisition; parsing and fragment transport are separate.
use super::direct;
use crate::{Error, Result, VideoFormat, VideoInfo};
use percent_encoding::percent_decode_str;
use reqwest::{
    blocking::Response,
    header::{CONTENT_ENCODING, CONTENT_TYPE},
};
use std::io::Read;
use url::Url;

const MAX_MANIFEST_BYTES: u64 = 2 * 1024 * 1024;

fn unsupported(message: &str) -> Error {
    Error::UnsupportedManifest(message.into())
}

pub(crate) fn recognized(response: &Response) -> bool {
    direct::response_mime(response) == "application/dash+xml"
        || response.url().path().to_ascii_lowercase().ends_with(".mpd")
}

fn read_manifest(
    response: Response,
) -> Result<(Url, Vec<VideoFormat>, crate::dash::ManifestSubtitles)> {
    if response
        .content_length()
        .is_some_and(|size| size > MAX_MANIFEST_BYTES)
    {
        return Err(unsupported(
            "DASH playlist exceeds the 2097152-byte extraction limit",
        ));
    }
    let encodings = response.headers().get_all(CONTENT_ENCODING);
    if encodings.iter().count() > 1 {
        return Err(unsupported(
            "duplicate DASH Content-Encoding headers are not supported",
        ));
    }
    if encodings
        .iter()
        .any(|value| !value.as_bytes().eq_ignore_ascii_case(b"identity"))
    {
        return Err(unsupported("compressed DASH playlists are not supported"));
    }
    let content_types = response.headers().get_all(CONTENT_TYPE);
    if content_types.iter().count() > 1 {
        return Err(unsupported(
            "duplicate DASH Content-Type headers are not supported",
        ));
    }
    if let Some(value) = content_types.iter().next() {
        let content_type = value
            .to_str()
            .map_err(|_| unsupported("invalid DASH Content-Type header"))?;
        for parameter in content_type.split(';').skip(1) {
            if let Some((name, value)) = parameter.trim().split_once('=')
                && name.trim().eq_ignore_ascii_case("charset")
                && !matches!(
                    value
                        .trim()
                        .trim_matches(['\'', '"'])
                        .to_ascii_lowercase()
                        .as_str(),
                    "utf-8" | "utf8" | "us-ascii"
                )
            {
                return Err(unsupported("only UTF-8/ASCII DASH playlists are supported"));
            }
        }
    }
    let base = response.url().clone();
    let mut bytes = Vec::new();
    response
        .take(MAX_MANIFEST_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(Error::ExtractionRead)?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(unsupported(
            "DASH playlist exceeds the 2097152-byte extraction limit",
        ));
    }
    crate::screen::page_read(&base, &bytes)?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| unsupported("only UTF-8/ASCII DASH playlists are supported"))?;
    let (formats, subtitles) = crate::dash::parse_with_subtitles(text, &base)?;
    Ok((base.clone(), formats, subtitles))
}

pub(crate) fn from_response(
    response: Response,
    original: &Url,
    force_dash: bool,
) -> Result<Option<VideoInfo>> {
    if !force_dash && !recognized(&response) {
        return Ok(None);
    }
    // Upstream's generic and HTML5 DASH extraction pass no `mpd_id`, so
    // representation IDs are used as they are.
    let (manifest_url, formats, subtitle_list) = read_manifest(response)?;
    // `period_entry['subtitles'][lang or 'und'].append(f)`.
    let mut subtitles = crate::Subtitles::default();
    for (language, format) in subtitle_list {
        let language = Some(language);
        match subtitles
            .entries
            .iter_mut()
            .find(|(key, _)| *key == language)
        {
            Some((_, formats)) => formats.push(format),
            None => subtitles.entries.push((language, vec![format])),
        }
    }
    let first = formats
        .first()
        .ok_or_else(|| unsupported("DASH manifest has no supported representations"))?;
    let filename = manifest_url
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .unwrap_or("");
    let filename = percent_decode_str(filename).decode_utf8_lossy();
    let stem = filename
        .rsplit_once('.')
        .map_or(filename.as_ref(), |(stem, _)| stem);
    let title = if stem.is_empty() { "video" } else { stem }.to_owned();
    Ok(Some(VideoInfo {
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
        subtitles,
        requested_subtitles: None,
        last_playlist_index: None,
        playlist_index: None,
        playlist_autonumber: None,
        playlist_count: None,
        n_entries: None,
        id: title.clone(),
        title,
        fulltitle: None,
        ext: first.ext.clone(),
        url: first.url.clone(),
        webpage_url: original.clone(),
        extractor: "generic".into(),
        extractor_key: "Generic".into(),
        direct: None,
        stream_kind: first.stream_kind,
        fragments: first.fragments.clone(),
        requested_formats: Vec::new(),
        format_id: None,
        filesize: first.filesize,
        width: first.width,
        height: first.height,
        formats,
    }))
}

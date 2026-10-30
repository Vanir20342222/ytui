//! Bounded VOD HLS extraction. Segment bodies are fetched only by the downloader.
use super::direct;
use crate::{Error, Result, StreamKind, VideoFormat, VideoInfo, hls::Playlist};
use percent_encoding::percent_decode_str;
use reqwest::{
    blocking::{Client, Response},
    header::{CONTENT_ENCODING, CONTENT_TYPE},
};
use std::io::Read;
use url::Url;

const MAX_MANIFEST_BYTES: u64 = 2 * 1024 * 1024;
const MAX_VARIANTS: usize = 256;
const MAX_TOTAL_FRAGMENTS: usize = 10_000;

fn unsupported(message: &str) -> Error {
    Error::UnsupportedManifest(message.into())
}

fn recognized(response: &Response) -> bool {
    let mime = direct::response_mime(response);
    matches!(
        mime.as_str(),
        "application/vnd.apple.mpegurl"
            | "application/x-mpegurl"
            | "audio/mpegurl"
            | "audio/x-mpegurl"
    ) || response
        .url()
        .path()
        .to_ascii_lowercase()
        .ends_with(".m3u8")
}

/// Upstream `_extract_m3u8_formats` keys this port does not model as fields:
/// `has_drm` (always false: DRM playlists are rejected) and, for formats from
/// a master playlist, its `manifest_url`.
fn hls_extra(master: Option<&Url>, shape: &str) -> serde_json::Map<String, serde_json::Value> {
    let mut extra = serde_json::Map::new();
    extra.insert(crate::info_order::FORMAT_SHAPE.into(), shape.into());
    if let Some(master) = master {
        extra.insert("manifest_url".into(), master.as_str().into());
    }
    extra.insert("has_drm".into(), false.into());
    extra
}

pub(crate) fn read_playlist(response: Response) -> Result<(Url, Playlist)> {
    if response
        .content_length()
        .is_some_and(|size| size > MAX_MANIFEST_BYTES)
    {
        return Err(unsupported(
            "HLS playlist exceeds the 2097152-byte extraction limit",
        ));
    }
    let encodings = response.headers().get_all(CONTENT_ENCODING);
    if encodings.iter().count() > 1 {
        return Err(unsupported(
            "duplicate HLS Content-Encoding headers are not supported",
        ));
    }
    if encodings
        .iter()
        .any(|value| !value.as_bytes().eq_ignore_ascii_case(b"identity"))
    {
        return Err(unsupported("compressed HLS playlists are not supported"));
    }
    let content_types = response.headers().get_all(CONTENT_TYPE);
    if content_types.iter().count() > 1 {
        return Err(unsupported(
            "duplicate HLS Content-Type headers are not supported",
        ));
    }
    if let Some(value) = content_types.iter().next() {
        let content_type = value
            .to_str()
            .map_err(|_| unsupported("invalid HLS Content-Type header"))?;
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
                return Err(unsupported("only UTF-8/ASCII HLS playlists are supported"));
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
            "HLS playlist exceeds the 2097152-byte extraction limit",
        ));
    }
    crate::screen::page_read(&base, &bytes)?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| unsupported("only UTF-8/ASCII HLS playlists are supported"))?;
    Ok((base.clone(), crate::hls::parse(text, &base)?))
}

/// Upstream generic `_extra_manifest_info` ("Checking m3u8 live status"):
/// the first `m3u8_native` format's playlist, refetched without failing
/// extraction, supplies a missing VOD duration.
pub(crate) fn check_live_status(client: &Client, info: &mut VideoInfo) {
    let Some(format) = info
        .formats
        .iter()
        .find(|format| format.determine_protocol() == "m3u8_native")
    else {
        return;
    };
    let Ok(response) = direct::probe(client, &format.url, None) else {
        return;
    };
    let url = response.url().clone();
    let mut bytes = Vec::new();
    if response
        .take(MAX_MANIFEST_BYTES)
        .read_to_end(&mut bytes)
        .is_err()
        || crate::screen::page_read(&url, &bytes).is_err()
        || !bytes.starts_with(b"#EXTM3U")
    {
        return;
    }
    let duration = vod_duration(&String::from_utf8_lossy(&bytes));
    info.metadata.duration = info
        .metadata
        .duration
        .or(duration.map(|duration| crate::info::PyNumber::Int(duration as i64)));
}

/// Upstream `_parse_m3u8_vod_duration`: `int(sum(EXTINF))`, `None` for zero
/// or without `#EXT-X-ENDLIST`.
fn vod_duration(playlist: &str) -> Option<u64> {
    if !playlist.contains("#EXT-X-ENDLIST") {
        return None;
    }
    let mut total = 0.0;
    for line in playlist.lines() {
        if let Some(value) = line.strip_prefix("#EXTINF:") {
            total += value.split(',').next()?.trim().parse::<f64>().ok()?;
        }
    }
    Some(total.trunc() as u64).filter(|duration| *duration > 0)
}

static SPLIT_DISCONTINUITY: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// `--hls-split-discontinuity`: one format per discontinuity section.
pub fn set_split_discontinuity(split: bool) {
    SPLIT_DISCONTINUITY.store(split, std::sync::atomic::Ordering::Relaxed);
}

/// Upstream `_extract_m3u8_playlist_indices`: with
/// `--hls-split-discontinuity`, `format` becomes one format per section
/// (`idx` joins the format ID unless it is 0); otherwise it is unchanged.
fn split_sections(format: VideoFormat, discontinuity: &[u32], count: u32) -> Vec<VideoFormat> {
    if !SPLIT_DISCONTINUITY.load(std::sync::atomic::Ordering::Relaxed) {
        return vec![format];
    }
    (0..=count)
        .map(|index| {
            let mut section = format.clone();
            if index > 0 {
                section.format_id = if format.format_id.is_empty() {
                    index.to_string()
                } else {
                    format!("{}-{index}", format.format_id)
                };
            }
            section.format_index = Some(index);
            section.fragments =
                crate::hls::section(format.fragments.clone(), discontinuity, Some(index));
            section
        })
        .collect()
}

/// Upstream `_extract_m3u8_formats(..., ext=None)`: a variant is `m4a`
/// when audio-only, else `mp4`; a media playlist or rendition keeps no
/// `ext`, so processing takes its URL's (`determine_ext`).
pub(crate) fn without_ext(info: &mut VideoInfo) {
    for format in &mut info.formats {
        let shape = format
            .extra
            .get(crate::info_order::FORMAT_SHAPE)
            .and_then(serde_json::Value::as_str);
        format.ext = match shape {
            Some("hls-variant") if format.vcodec.as_deref() == Some("none") => "m4a".into(),
            Some("hls-variant") => "mp4".into(),
            _ => direct::determine_ext(format.url.as_str())
                .unwrap_or_else(|| "unknown_video".into())
                .to_lowercase(),
        };
    }
    if let Some(first) = info.formats.first() {
        info.ext = first.ext.clone();
    }
}

/// `_parse_m3u8_formats(..., live=True)`: a live variant keeps only its
/// `m3u8_id` (its bandwidth changes over time), so unnamed IDs become
/// indexes when processed.
pub(crate) fn live_format_ids(formats: &mut [VideoFormat], m3u8_id: Option<&str>) {
    for format in formats {
        if format
            .extra
            .get(crate::info_order::FORMAT_SHAPE)
            .and_then(serde_json::Value::as_str)
            .is_some_and(|shape| shape.starts_with("hls-variant"))
        {
            format.format_id = m3u8_id.unwrap_or_default().to_owned();
        }
    }
}

pub(crate) fn from_response(
    client: &Client,
    response: Response,
    original: &Url,
    referer: Option<&Url>,
    force_hls: bool,
    m3u8_id: Option<&str>,
) -> Result<Option<VideoInfo>> {
    from_response_inner(
        client, response, original, referer, force_hls, m3u8_id, false,
    )
}

/// Playback delegates media-playlist/segment loading to the player. A master
/// already supplies the quality/codec/audio-group catalog; traversing every
/// variant here adds serial requests and then the player fetches them again.
pub(crate) fn from_response_for_playback(
    client: &Client,
    response: Response,
    original: &Url,
) -> Result<Option<VideoInfo>> {
    from_response_inner(client, response, original, None, true, None, true)
}

#[allow(clippy::too_many_arguments)]
fn from_response_inner(
    client: &Client,
    response: Response,
    original: &Url,
    referer: Option<&Url>,
    force_hls: bool,
    m3u8_id: Option<&str>,
    playback: bool,
) -> Result<Option<VideoInfo>> {
    if !force_hls && !recognized(&response) {
        return Ok(None);
    }
    let (manifest_url, playlist) = read_playlist(response)?;
    let filename = manifest_url
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .unwrap_or("");
    let filename = percent_decode_str(filename).decode_utf8_lossy();
    let stem = filename
        .rsplit_once('.')
        .map_or(filename.as_ref(), |(stem, _)| stem);
    let title = if stem.is_empty() { "video" } else { stem }.to_owned();
    let mut info = VideoInfo {
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
        id: title.clone(),
        title,
        fulltitle: None,
        ext: String::new(),
        url: manifest_url.clone(),
        webpage_url: original.clone(),
        extractor: "generic".into(),
        extractor_key: "Generic".into(),
        direct: None,
        formats: Vec::new(),
        fragments: Vec::new(),
        requested_formats: Vec::new(),
        stream_kind: StreamKind::Standalone,
        format_id: None,
        filesize: None,
        width: None,
        height: None,
    };
    // Upstream `join_nonempty(...)` format IDs; empty IDs are numbered later.
    let join = |parts: &[Option<&str>]| -> String {
        parts
            .iter()
            .flatten()
            .filter(|part| !part.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join("-")
    };
    match playlist {
        Playlist::Media {
            fragments,
            discontinuity,
            discontinuities,
            ..
        } => {
            // A media playlist is returned as one format (per section).
            let format = VideoFormat {
                http_headers: None,
                extra: hls_extra(None, "hls-media"),
                protocol: Some("m3u8_native".into()),
                vcodec: None,
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
                format_id: join(&[m3u8_id]),
                stream_kind: StreamKind::Standalone,
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
                // Upstream passes `ext='mp4'` for every HLS format.
                ext: "mp4".into(),
                filesize: None,
                width: None,
                height: None,
                fragments,
            };
            info.formats = split_sections(format, &discontinuity, discontinuities);
            info.ext = "mp4".into();
            info.fragments = info.formats[0].fragments.clone();
        }
        Playlist::Master {
            variants,
            audio,
            subtitles,
        } => {
            // Upstream `extract_media`: an m3u8 subtitle rendition is WebVTT
            // fetched through HlsFD; other URIs keep their extension.
            for subtitle in subtitles {
                let ext = direct::determine_ext(subtitle.url.as_str())
                    .unwrap_or_else(|| "unknown_video".into());
                let (ext, protocol) = if ext == "m3u8" {
                    ("vtt".to_owned(), Some("m3u8_native".to_owned()))
                } else {
                    (ext, None)
                };
                let language = Some(subtitle.language.unwrap_or_else(|| "und".into()));
                let format = crate::SubtitleFormat {
                    extra: Default::default(),
                    fragments: Vec::new(),
                    name: None,
                    url: subtitle.url,
                    ext,
                    protocol,
                };
                // `subtitles.setdefault(lang, []).append(sub_info)`.
                match info
                    .subtitles
                    .entries
                    .iter_mut()
                    .find(|(key, _)| *key == language)
                {
                    Some((_, formats)) => formats.push(format),
                    None => info.subtitles.entries.push((language, vec![format])),
                }
            }
            if variants.len() + audio.len() > MAX_VARIANTS {
                return Err(unsupported(
                    "HLS master exceeds the 256-variant extraction limit",
                ));
            }
            let mut total_fragments = 0usize;
            // Upstream `_audio_group_id` of video variants with CODECS.
            let mut grouped_videos: Vec<(String, f64, u32)> = Vec::new();
            // Upstream lists renditions before variants (and counts them for
            // unnamed variants); playlists are still fetched variants first.
            let renditions = audio.len();
            for variant in variants {
                let (url, fragments, discontinuity, discontinuities) =
                    child_playlist(client, variant.url, referer, playback)?;
                total_fragments = total_fragments
                    .checked_add(fragments.len())
                    .ok_or_else(|| unsupported("HLS master fragment count overflow"))?;
                if total_fragments > MAX_TOTAL_FRAGMENTS {
                    return Err(unsupported(
                        "HLS master exceeds the 10000-fragment extraction limit",
                    ));
                }
                // Upstream `'%d' % (tbr or len(formats))` without a stream name.
                let fallback = match variant.bandwidth.filter(|tbr| *tbr != 0.0) {
                    Some(tbr) => format!("{}", tbr.trunc() as i64),
                    None => (renditions + info.formats.len()).to_string(),
                };
                let name = variant.name.clone().unwrap_or(fallback);
                if let (Some(codecs), Some(group)) = (&variant.codecs, &variant.audio_group)
                    && codecs.vcodec != "none"
                {
                    grouped_videos.push((
                        group.clone(),
                        variant.bandwidth.unwrap_or(0.0),
                        variant.width.unwrap_or(0),
                    ));
                }
                let codecs = variant.codecs.unwrap_or_default();
                let known = |codec: String| (!codec.is_empty()).then_some(codec);
                let format = VideoFormat {
                    http_headers: None,
                    extra: hls_extra(Some(&manifest_url), "hls-variant"),
                    protocol: Some("m3u8_native".into()),
                    vcodec: known(codecs.vcodec),
                    acodec: known(codecs.acodec),
                    dynamic_range: codecs.dynamic_range.map(str::to_owned),
                    video_ext: None,
                    audio_ext: None,
                    vbr: None,
                    source_preference: None,
                    quality: None,
                    fps: variant.fps.map(crate::info::PyNumber::Float),
                    asr: None,
                    audio_channels: None,
                    resolution: None,
                    format: None,
                    aspect_ratio: None,
                    format_id: join(&[m3u8_id, Some(&name)]),
                    stream_kind: variant.stream_kind,
                    audio_group: variant.audio_group,
                    language: None,
                    format_note: None,
                    audio_default: false,
                    audio_autoselect: false,
                    downloader_options: None,
                    manifest_stream_number: None,
                    format_index: None,
                    container: None,
                    abr: (variant.stream_kind == StreamKind::AudioOnly)
                        .then_some(variant.bandwidth)
                        .flatten(),
                    tbr: variant.bandwidth,
                    url,
                    ext: "mp4".into(),
                    filesize: None,
                    width: variant.width,
                    height: variant.height,
                    fragments,
                };
                // `PROGRESSIVE-URI`: an `http-` copy without the manifest.
                let progressive = variant.progressive.map(|progressive| {
                    let mut http = format.clone();
                    http.extra.shift_remove("manifest_url");
                    http.format_id = http.format_id.replacen("hls-", "http-", 1);
                    http.protocol = Some("http".into());
                    http.url = progressive;
                    http.fragments = Vec::new();
                    http
                });
                info.formats
                    .extend(split_sections(format, &discontinuity, discontinuities));
                info.formats.extend(progressive);
            }
            let mut rendition_formats = Vec::new();
            let mut alternates: Vec<bool> = Vec::new();
            for rendition in audio {
                let alternate = rendition.alternate;
                let (url, fragments, discontinuity, discontinuities) =
                    child_playlist(client, rendition.url, referer, playback)?;
                total_fragments = total_fragments
                    .checked_add(fragments.len())
                    .ok_or_else(|| unsupported("HLS master fragment count overflow"))?;
                if total_fragments > MAX_TOTAL_FRAGMENTS {
                    return Err(unsupported(
                        "HLS master exceeds the 10000-fragment extraction limit",
                    ));
                }
                let format = VideoFormat {
                    http_headers: None,
                    extra: hls_extra(Some(&manifest_url), "hls-rendition"),
                    protocol: Some("m3u8_native".into()),
                    vcodec: Some("none".into()),
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
                    format_id: join(&[m3u8_id, Some(&rendition.group_id), Some(&rendition.name)]),
                    stream_kind: StreamKind::AudioOnly,
                    audio_group: Some(rendition.group_id),
                    language: rendition.language,
                    format_note: Some(rendition.name),
                    audio_default: rendition.default,
                    audio_autoselect: rendition.autoselect,
                    downloader_options: None,
                    manifest_stream_number: None,
                    format_index: None,
                    container: None,
                    abr: None,
                    tbr: None,
                    url,
                    ext: "mp4".into(),
                    filesize: None,
                    width: None,
                    height: None,
                    fragments,
                };
                for section in split_sections(format, &discontinuity, discontinuities) {
                    alternates.push(alternate);
                    rendition_formats.push(section);
                }
            }
            // Upstream: alternate renditions get source_preference -2; with
            // several audio groups, the others are ranked by the quality of
            // their videos and noted `low`/`high`.
            grouped_videos.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.2.cmp(&b.2)));
            let mut groups: Vec<String> = Vec::new();
            for (group, ..) in grouped_videos {
                if !groups.contains(&group) {
                    groups.push(group);
                }
            }
            for (format, alternate) in rendition_formats.iter_mut().zip(&alternates) {
                if *alternate {
                    format.source_preference = Some(-2);
                    continue;
                }
                let Some(group) = format.audio_group.as_deref().filter(|_| groups.len() > 1) else {
                    continue;
                };
                format.source_preference = Some(
                    groups
                        .iter()
                        .position(|candidate| candidate == group)
                        .map_or(-1, |index| index as i64),
                );
                let note = if Some(group) == groups.first().map(String::as_str) {
                    Some("low")
                } else if Some(group) == groups.last().map(String::as_str) {
                    Some("high")
                } else {
                    None
                };
                if let Some(note) = note {
                    format.format_note = Some(match format.format_note.take() {
                        Some(name) if !name.is_empty() => format!("{name}, {note}"),
                        _ => note.to_owned(),
                    });
                }
            }
            info.formats.splice(0..0, rendition_formats);
            // Renditions precede variants; the top level mirrors a variant.
            let first = info
                .formats
                .get(renditions)
                .or(info.formats.first())
                .ok_or_else(|| unsupported("HLS master has no variants"))?;
            info.ext = first.ext.clone();
            info.url = first.url.clone();
            info.stream_kind = first.stream_kind;
            info.fragments = first.fragments.clone();
        }
    }
    Ok(Some(info))
}

fn child_playlist(
    client: &Client,
    url: Url,
    referer: Option<&Url>,
    playback: bool,
) -> Result<(Url, Vec<crate::Fragment>, Vec<u32>, u32)> {
    if playback {
        return Ok((url, Vec::new(), Vec::new(), 0));
    }
    let response = direct::probe(client, &url, referer)?;
    let (url, playlist) = read_playlist(response)?;
    let Playlist::Media {
        fragments,
        discontinuity,
        discontinuities,
        ..
    } = playlist
    else {
        return Err(unsupported("nested HLS master playlists are not supported"));
    };
    Ok((url, fragments, discontinuity, discontinuities))
}

#[cfg(test)]
mod tests {
    use super::vod_duration;

    #[test]
    fn vod_duration_sums_extinf_like_upstream() {
        let vod = "#EXTM3U\n#EXTINF:2.5,\na.ts\n#EXTINF:1.75,x\nb.ts\n#EXT-X-ENDLIST\n";
        assert_eq!(vod_duration(vod), Some(4));
        assert_eq!(vod_duration("#EXTM3U\n#EXTINF:2.5,\na.ts\n"), None);
        assert_eq!(
            vod_duration("#EXTM3U\n#EXTINF:0.5,\na.ts\n#EXT-X-ENDLIST"),
            None
        );
    }
}

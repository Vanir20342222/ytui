//! Upstream `InfoExtractor._parse_ism_formats_and_subtitles` for a Smooth
//! Streaming manifest: `protocol: ism` formats and `ismt` subtitles with
//! their fragments and IsmFD `_download_params`.
use crate::{Fragment, Result, SubtitleFormat, Subtitles, VideoFormat};
use serde_json::{Value, json};
use url::Url;

pub(crate) struct Parsed {
    pub formats: Vec<VideoFormat>,
    pub subtitles: Subtitles,
}

fn unsupported(message: &str) -> crate::Error {
    crate::Error::UnsupportedHtml5(message.into())
}

/// Python's `str` of the fragment time: an `int` until a computed
/// duration makes it a `float`.
fn time_text(time: f64, float: bool) -> String {
    if float {
        format!("{time:?}")
    } else {
        format!("{}", time as i64)
    }
}

/// The document of a GenericIE `SmoothStreamingMedia` root, or `None`.
pub(crate) fn parse(body: &str, ism_url: &Url, ism_id: Option<&str>) -> Option<Result<Parsed>> {
    let document = roxmltree::Document::parse_with_options(
        body.trim_start_matches('\u{feff}'),
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    )
    .ok()?;
    let root = document.root_element();
    if root.tag_name().name() != "SmoothStreamingMedia" || root.tag_name().namespace().is_some() {
        return None;
    }
    Some(parse_document(root, ism_url, ism_id))
}

/// `node.findall(name)`.
fn elements<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    name: &str,
) -> Vec<roxmltree::Node<'a, 'input>> {
    node.children()
        .filter(|child| child.is_element() && child.tag_name().name() == name)
        .collect()
}

fn parse_document(
    root: roxmltree::Node<'_, '_>,
    ism_url: &Url,
    ism_id: Option<&str>,
) -> Result<Parsed> {
    let mut parsed = Parsed {
        formats: Vec::new(),
        subtitles: Subtitles::default(),
    };
    if root.attribute("IsLive") == Some("TRUE") {
        return Ok(parsed);
    }
    let int = |value: Option<&str>| value.and_then(|value| value.trim().parse::<i64>().ok());
    let duration = int(root.attribute("Duration")).ok_or_else(|| {
        unsupported("ISM manifests without an integer Duration are not supported natively")
    })?;
    let timescale = int(root.attribute("TimeScale"))
        .filter(|scale| *scale != 0)
        .unwrap_or(10_000_000);
    let has_drm = root
        .children()
        .any(|node| node.is_element() && node.tag_name().name() == "Protection");
    static BITRATE: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"\{[Bb]itrate\}").expect("static pattern"));
    static START: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"\{start[ _]time\}").expect("static pattern")
    });
    for stream in elements(root, "StreamIndex") {
        let stream_type = stream.attribute("Type").unwrap_or_default();
        if !matches!(stream_type, "video" | "audio" | "text") {
            continue;
        }
        let url_pattern = stream
            .attribute("Url")
            .ok_or_else(|| unsupported("ISM streams without a Url are not supported natively"))?;
        let stream_timescale = int(stream.attribute("TimeScale"))
            .filter(|scale| *scale != 0)
            .unwrap_or(timescale);
        let name = stream.attribute("Name");
        // IsmFD expects ISO 639-2 codes.
        let mut language = stream
            .attribute("Language")
            .filter(|language| !language.is_empty())
            .unwrap_or("und")
            .to_owned();
        if language.chars().count() != 3 {
            language = crate::embed_subtitles::short_to_long(&language)
                .unwrap_or("und")
                .to_owned();
        }
        for track in elements(stream, "QualityLevel") {
            let fourcc = track
                .attribute("FourCC")
                .filter(|fourcc| !fourcc.is_empty())
                .or(match track.attribute("AudioTag") {
                    Some("255") => Some("AACL"),
                    Some("65534") => Some("EC-3"),
                    _ => None,
                });
            let Some(fourcc) = fourcc
                .filter(|fourcc| matches!(*fourcc, "H264" | "AVC1" | "AACL" | "TTML" | "EC-3"))
            else {
                crate::screen::warning(&format!(
                    "[generic] {} is not a supported codec",
                    fourcc.unwrap_or("None")
                ));
                continue;
            };
            let bitrate = track.attribute("Bitrate").ok_or_else(|| {
                unsupported("ISM tracks without a Bitrate are not supported natively")
            })?;
            let tbr = int(Some(bitrate))
                .ok_or_else(|| unsupported("non-integer ISM bitrates are not supported natively"))?
                .div_euclid(1000);
            let dimension = |max: &str, plain: &str| {
                int(track
                    .attribute(max)
                    .filter(|value| !value.is_empty())
                    .or_else(|| track.attribute(plain)))
            };
            let width = dimension("MaxWidth", "Width");
            let height = dimension("MaxHeight", "Height");
            let sampling_rate = int(track.attribute("SamplingRate"));
            // `urljoin(ism_url, pattern)` keeps the placeholder; the joined
            // URL is built once it is substituted.
            let pattern = BITRATE.replace_all(url_pattern, regex::NoExpand(bitrate));
            let mut fragments = Vec::new();
            let mut time = 0f64;
            let mut float = false;
            for chunk in elements(stream, "c") {
                if let Some(start) = int(chunk.attribute("t")).filter(|start| *start != 0) {
                    time = start as f64;
                    float = false;
                }
                let repeat = int(chunk.attribute("r"))
                    .filter(|repeat| *repeat != 0)
                    .unwrap_or(1);
                // Upstream indexes the `c` element's own (absent) children,
                // so a missing duration always uses the stream's end.
                let (length, computed) =
                    match int(chunk.attribute("d")).filter(|length| *length != 0) {
                        Some(length) => (length as f64, false),
                        None => ((duration as f64 - time) / repeat as f64, true),
                    };
                if repeat > 100_000 || fragments.len() > 100_000 {
                    return Err(crate::Error::UnsupportedManifest(
                        "ISM fragment count must be at most 100000".into(),
                    ));
                }
                for _ in 0..repeat {
                    let url = START.replace_all(&pattern, regex::NoExpand(&time_text(time, float)));
                    fragments.push(Fragment {
                        encryption: None,
                        url: ism_url.join(&url)?,
                        byte_range: None,
                        path: None,
                        duration: Some(length / stream_timescale as f64),
                    });
                    time += length;
                    float |= computed;
                }
            }
            let private = track.attribute("CodecPrivateData");
            if stream_type == "text" {
                let mut extra = serde_json::Map::new();
                extra.insert("manifest_url".into(), ism_url.as_str().into());
                extra.insert(
                    "_download_params".into(),
                    json!({
                        "stream_type": stream_type,
                        "duration": duration,
                        "timescale": stream_timescale,
                        "fourcc": fourcc,
                        "language": language,
                        "codec_private_data": private,
                    }),
                );
                extra.insert(crate::info_order::FORMAT_SHAPE.into(), "ism-text".into());
                let track = SubtitleFormat {
                    name: None,
                    url: ism_url.clone(),
                    ext: "ismt".into(),
                    protocol: Some("ism".into()),
                    fragments,
                    extra,
                };
                match parsed
                    .subtitles
                    .entries
                    .iter_mut()
                    .find(|(key, _)| key.as_deref() == Some(language.as_str()))
                {
                    Some((_, tracks)) => tracks.push(track),
                    None => parsed
                        .subtitles
                        .entries
                        .push((Some(language.clone()), vec![track])),
                }
                continue;
            }
            let video = stream_type == "video";
            let format_id = [ism_id, name, Some(tbr.to_string().as_str())]
                .into_iter()
                .flatten()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join("-");
            let mut format: VideoFormat = serde_json::from_value(json!({
                "format_id": format_id,
                "url": ism_url.as_str(),
                "ext": if video { "ismv" } else { "isma" },
                "width": width,
                "height": height,
                "tbr": tbr,
                "asr": sampling_rate,
                "vcodec": if video { fourcc } else { "none" },
                "acodec": if video { "none" } else { fourcc },
                "protocol": "ism",
                "language": language,
                "audio_channels": int(track.attribute("Channels")),
            }))
            .map_err(|error| unsupported(&format!("ISM track: {error}")))?;
            format.fragments = fragments;
            for (key, value) in [
                ("manifest_url", Value::from(ism_url.as_str())),
                ("has_drm", Value::from(has_drm)),
                (
                    "_download_params",
                    json!({
                        "stream_type": stream_type,
                        "duration": duration,
                        "timescale": stream_timescale,
                        "width": width.unwrap_or(0),
                        "height": height.unwrap_or(0),
                        "fourcc": fourcc,
                        "language": language,
                        "codec_private_data": private,
                        "sampling_rate": sampling_rate,
                        "channels": int(Some(track.attribute("Channels").unwrap_or("2"))),
                        "bits_per_sample": int(Some(track.attribute("BitsPerSample").unwrap_or("16"))),
                        "nal_unit_length_field": int(Some(track.attribute("NALUnitLengthField").unwrap_or("4"))),
                    }),
                ),
            ] {
                format.extra.insert(key.into(), value);
            }
            crate::info_order::mark_format(&mut format, "ism");
            parsed.formats.push(format);
        }
    }
    Ok(parsed)
}

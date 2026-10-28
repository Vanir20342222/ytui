//! Bounded, pure HLS VOD parsing. Transport and playlist traversal live elsewhere.
use crate::{ByteRange, Error, Fragment, HlsAes128, Result, StreamKind};
use std::collections::{BTreeMap, BTreeSet};
use url::Url;

const MAX_FRAGMENTS: usize = 10_000;
const MAX_VARIANTS: usize = 256;

#[derive(Debug)]
pub(crate) enum Playlist {
    Master {
        variants: Vec<Variant>,
        audio: Vec<AudioRendition>,
        /// `SUBTITLES` renditions with a URI, in document order.
        subtitles: Vec<SubtitleRendition>,
    },
    Media {
        fragments: Vec<Fragment>,
        /// Each fragment's count of preceding `#EXT-X-DISCONTINUITY*` lines
        /// (upstream HlsFD's `discontinuity_count`).
        discontinuity: Vec<u32>,
        /// All `#EXT-X-DISCONTINUITY*` lines (upstream counts the whole
        /// document for `--hls-split-discontinuity`).
        discontinuities: u32,
        /// Segment container from the URLs; formats are labeled `mp4` like
        /// upstream and FFmpeg inputs sniff their contents instead.
        #[cfg_attr(not(test), allow(dead_code))]
        ext: String,
    },
}

#[derive(Debug)]
pub(crate) struct Variant {
    pub url: Url,
    /// Kilobits per second, with valid average bandwidth preferred.
    pub bandwidth: Option<f64>,
    /// `float_or_none(FRAME-RATE)`.
    pub fps: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub stream_kind: StreamKind,
    /// Upstream `parse_codecs(CODECS)`, with `acodec` `none` for a variant
    /// whose audio comes from an external rendition group.
    pub codecs: Option<crate::codecs::Codecs>,
    pub audio_group: Option<String>,
    /// Upstream `build_stream_name`: the variant's NAME, else its VIDEO
    /// group's first rendition NAME or the group ID.
    pub name: Option<String>,
    /// `PROGRESSIVE-URI`: the same stream as a progressive HTTP format.
    pub progressive: Option<Url>,
}

/// An `EXT-X-MEDIA:TYPE=SUBTITLES` rendition (upstream `extract_media`).
#[derive(Debug)]
pub(crate) struct SubtitleRendition {
    pub url: Url,
    pub language: Option<String>,
}

#[derive(Debug)]
pub(crate) struct AudioRendition {
    pub url: Url,
    pub group_id: String,
    pub name: String,
    pub language: Option<String>,
    pub default: bool,
    pub autoselect: bool,
    /// Upstream `is_alternate`: `DEFAULT=NO` or `AUTOSELECT=NO`.
    pub alternate: bool,
}

fn invalid(message: impl Into<String>) -> Error {
    Error::UnsupportedManifest(message.into())
}

/// Parse an attribute list without splitting commas inside quoted values.
fn attributes(input: &str) -> Result<BTreeMap<&str, &str>> {
    let mut result = BTreeMap::new();
    let mut rest = input;
    while !rest.is_empty() {
        let (name, value) = rest
            .split_once('=')
            .ok_or_else(|| invalid("malformed HLS attribute"))?;
        if name.is_empty()
            || !name
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err(invalid("invalid HLS attribute name"));
        }
        let (value, tail) = if let Some(quoted) = value.strip_prefix('"') {
            let end = quoted
                .find('"')
                .ok_or_else(|| invalid("unterminated HLS attribute quote"))?;
            (&quoted[..end], &quoted[end + 1..])
        } else {
            let end = value.find(',').unwrap_or(value.len());
            let plain = &value[..end];
            if plain.is_empty() || plain.contains(['"', ' ', '\t']) {
                return Err(invalid("invalid unquoted HLS attribute"));
            }
            (plain, &value[end..])
        };
        if result.insert(name, value).is_some() {
            return Err(invalid("duplicate HLS attribute"));
        }
        if tail.is_empty() {
            break;
        }
        rest = tail
            .strip_prefix(',')
            .filter(|v| !v.is_empty())
            .ok_or_else(|| invalid("malformed HLS attribute separator"))?;
    }
    Ok(result)
}

fn absolute(base: &Url, value: &str) -> Result<Url> {
    if value.is_empty() || value.contains(['\r', '\n']) || value.contains("{$") {
        return Err(invalid("empty or variable HLS URI"));
    }
    let url = base.join(value)?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || url.fragment().is_some()
    {
        return Err(invalid("HLS URI must be an HTTP(S) URL without a fragment"));
    }
    Ok(url)
}

/// A variant (or `PROGRESSIVE-URI`) URL: upstream keeps a fragment, which
/// is never sent.
fn absolute_variant(base: &Url, value: &str) -> Result<Url> {
    let (without, _) = value.split_once('#').unwrap_or((value, ""));
    let mut url = absolute(base, without)?;
    if let Some((_, fragment)) = value.split_once('#') {
        url.set_fragment(Some(fragment));
    }
    Ok(url)
}

fn positive_integer(value: &str) -> Result<u64> {
    let number = unsigned(value)?;
    if number == 0 {
        return Err(invalid("HLS number must be positive"));
    }
    Ok(number)
}

fn unsigned(value: &str) -> Result<u64> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid("invalid HLS integer"));
    }
    value.parse().map_err(|_| invalid("HLS integer overflow"))
}

fn byte_range(
    value: &str,
    previous: Option<(&Url, &ByteRange)>,
    url: &Url,
    explicit: bool,
) -> Result<ByteRange> {
    let (length, offset) = match value.split_once('@') {
        Some((length, offset)) => (positive_integer(length)?, unsigned(offset)?),
        None if !explicit => {
            let (_, previous) = previous
                .filter(|(previous_url, _)| *previous_url == url)
                .ok_or_else(|| {
                    invalid("implicit HLS range requires preceding range on the same URI")
                })?;
            (
                positive_integer(value)?,
                previous
                    .offset
                    .checked_add(previous.length)
                    .ok_or_else(|| invalid("HLS range overflow"))?,
            )
        }
        None => {
            return Err(invalid(
                "HLS initialization range requires an explicit offset",
            ));
        }
    };
    offset
        .checked_add(length)
        .ok_or_else(|| invalid("HLS range overflow"))?;
    Ok(ByteRange { offset, length })
}

struct AesKey {
    url: Url,
    iv: Option<[u8; 16]>,
}

impl AesKey {
    fn snapshot(&self, sequence: u64, initialization: bool) -> Result<HlsAes128> {
        let iv = match self.iv {
            Some(iv) => iv,
            None if initialization => {
                return Err(invalid("encrypted HLS MAP requires an explicit IV"));
            }
            None => u128::from(sequence).to_be_bytes(),
        };
        Ok(HlsAes128 {
            key_url: self.url.clone(),
            iv,
        })
    }
}

fn aes_key(value: &str, base: &Url) -> Result<Option<AesKey>> {
    let attrs = attributes(value)?;
    if attrs.get("METHOD") == Some(&"NONE") {
        return if attrs.len() == 1 {
            Ok(None)
        } else {
            Err(invalid("HLS METHOD=NONE cannot have other attributes"))
        };
    }
    if attrs.get("METHOD") != Some(&"AES-128") {
        return Err(invalid("only HLS identity AES-128 encryption is supported"));
    }
    if attrs.keys().any(|key| {
        !matches!(
            *key,
            "METHOD" | "URI" | "IV" | "KEYFORMAT" | "KEYFORMATVERSIONS"
        )
    }) {
        return Err(invalid("unsupported HLS key attribute"));
    }
    if attrs
        .get("KEYFORMAT")
        .is_some_and(|format| *format != "identity")
        || attrs
            .get("KEYFORMATVERSIONS")
            .is_some_and(|versions| *versions != "1")
    {
        return Err(invalid("unsupported HLS key format or version"));
    }
    let uri = attrs
        .get("URI")
        .ok_or_else(|| invalid("HLS AES-128 key has no URI"))?;
    if uri.bytes().any(|byte| byte <= b' ' || byte == 0x7f) {
        return Err(invalid("invalid HLS key URI"));
    }
    let url = absolute(base, uri)?;
    let iv = attrs
        .get("IV")
        .map(|value| {
            let digits = value
                .strip_prefix("0x")
                .or_else(|| value.strip_prefix("0X"))
                .filter(|digits| {
                    !digits.is_empty()
                        && digits.len() <= 32
                        && digits.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
                .ok_or_else(|| {
                    invalid("HLS IV must be a hexadecimal integer of at most 128 bits")
                })?;
            u128::from_str_radix(digits, 16)
                .map(u128::to_be_bytes)
                .map_err(|_| invalid("invalid HLS IV"))
        })
        .transpose()?;
    Ok(Some(AesKey { url, iv }))
}

fn variant(
    values: &BTreeMap<&str, &str>,
    url: Url,
    groups: &BTreeMap<(&str, &str), bool>,
    group_names: &BTreeMap<&str, &str>,
) -> Result<Variant> {
    // SUBTITLES groups become the result's subtitles.
    for group_type in ["VIDEO", "CLOSED-CAPTIONS"] {
        if let Some(group) = values.get(group_type).filter(|group| **group != "NONE")
            && groups.get(&(group_type, *group)) != Some(&false)
        {
            return Err(invalid(
                "external or unresolved HLS rendition group is unsupported",
            ));
        }
    }
    let audio_group = values.get("AUDIO").filter(|group| **group != "NONE");
    // Upstream ignores an AUDIO reference to an unknown group.
    let external_audio = audio_group
        .and_then(|group| groups.get(&("AUDIO", *group)).copied())
        .unwrap_or(false);
    let bandwidth = |name| {
        values
            .get(name)
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|value| value.is_finite() && *value > 0.0)
            .map(|value| value / 1000.0)
    };
    let (width, height) = if let Some(resolution) = values.get("RESOLUTION") {
        let (width, height) = resolution
            .split_once('x')
            .ok_or_else(|| invalid("invalid HLS resolution"))?;
        let dimension = |value| {
            u32::try_from(positive_integer(value)?).map_err(|_| invalid("HLS dimension overflow"))
        };
        (Some(dimension(width)?), Some(dimension(height)?))
    } else {
        (None, None)
    };
    // Upstream `parse_codecs(CODECS)` (unknown codecs only warn). A variant
    // referencing an audio group with URIs loses its audio when it declares
    // video codecs; otherwise upstream treats it as a complete format.
    let mut codecs = crate::codecs::parse(values.get("CODECS").copied());
    if external_audio
        && let Some(codecs) = &mut codecs
        && codecs.vcodec != "none"
    {
        codecs.acodec = "none".into();
    }
    let stream_kind = match &codecs {
        Some(codecs) if codecs.vcodec == "none" => StreamKind::AudioOnly,
        Some(codecs) if codecs.acodec == "none" => StreamKind::VideoOnly,
        _ => StreamKind::Standalone,
    };
    Ok(Variant {
        codecs,
        fps: values
            .get("FRAME-RATE")
            .and_then(|value| value.trim().parse::<f64>().ok()),
        url,
        bandwidth: bandwidth("AVERAGE-BANDWIDTH").or_else(|| bandwidth("BANDWIDTH")),
        width,
        height,
        stream_kind,
        audio_group: audio_group.map(|group| (*group).to_owned()),
        name: values
            .get("NAME")
            .filter(|name| !name.is_empty())
            .or_else(|| {
                values
                    .get("VIDEO")
                    .filter(|group| !group.is_empty())
                    .map(|group| group_names.get(group).unwrap_or(group))
            })
            .map(|name| (*name).to_owned()),
        progressive: None,
    })
}

pub(crate) fn parse(text: &str, base: &Url) -> Result<Playlist> {
    let lines: Vec<_> = text
        .trim_start_matches('\u{feff}')
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    if lines.first() != Some(&"#EXTM3U") {
        return Err(invalid("missing HLS EXTM3U header"));
    }
    let mut groups = BTreeMap::new();
    // Upstream `groups`: renditions with TYPE, GROUP-ID and NAME, by GROUP-ID.
    let mut group_names: BTreeMap<&str, &str> = BTreeMap::new();
    let mut audio = Vec::new();
    let mut subtitles = Vec::new();
    let mut audio_names = BTreeSet::new();
    let mut default_groups = BTreeSet::new();
    let mut rendition_count = 0usize;
    for line in &lines {
        if let Some(value) = line.strip_prefix("#EXT-X-MEDIA:") {
            let attrs = attributes(value)?;
            let kind = attrs
                .get("TYPE")
                .ok_or_else(|| invalid("HLS rendition has no TYPE"))?;
            let group = attrs
                .get("GROUP-ID")
                .filter(|v| !v.is_empty())
                .ok_or_else(|| invalid("HLS rendition has no GROUP-ID"))?;
            if !matches!(*kind, "AUDIO" | "VIDEO" | "SUBTITLES" | "CLOSED-CAPTIONS") {
                return Err(invalid("unsupported HLS rendition type"));
            }
            if let Some(name) = attrs.get("NAME").filter(|name| !name.is_empty()) {
                group_names.entry(*group).or_insert(*name);
            }
            let external = attrs.contains_key("URI");
            // TYPE, GROUP-ID and NAME are required; a URI-less one is skipped.
            if *kind == "SUBTITLES"
                && attrs.get("NAME").is_some_and(|name| !name.is_empty())
                && let Some(uri) = attrs.get("URI")
            {
                subtitles.push(SubtitleRendition {
                    url: absolute(base, uri)?,
                    language: attrs.get("LANGUAGE").map(|language| (*language).to_owned()),
                });
            }
            rendition_count += 1;
            if rendition_count > MAX_VARIANTS {
                return Err(invalid("HLS variant/rendition limit exceeded"));
            }
            if *kind == "AUDIO" {
                if groups
                    .get(&(*kind, *group))
                    .is_some_and(|previous| *previous != external)
                {
                    return Err(invalid(
                        "mixed external and inband HLS audio group is unsupported",
                    ));
                }
                let name = attrs
                    .get("NAME")
                    .filter(|name| !name.is_empty())
                    .ok_or_else(|| invalid("HLS audio rendition has no NAME"))?;
                if !audio_names.insert((*group, *name)) {
                    return Err(invalid("duplicate HLS audio rendition group/name"));
                }
                let flag = |name| match attrs.get(name) {
                    None | Some(&"NO") => Ok(false),
                    Some(&"YES") => Ok(true),
                    _ => Err(invalid("HLS audio rendition flag must be YES or NO")),
                };
                let default = flag("DEFAULT")?;
                let autoselect = flag("AUTOSELECT")?;
                if default && !default_groups.insert(*group) {
                    return Err(invalid(
                        "multiple default HLS audio renditions in one group",
                    ));
                }
                if default && attrs.get("AUTOSELECT") == Some(&"NO") {
                    return Err(invalid(
                        "default HLS audio rendition cannot disable AUTOSELECT",
                    ));
                }
                let language = attrs.get("LANGUAGE").map(|language| (*language).to_owned());
                if let Some(uri) = attrs.get("URI") {
                    audio.push(AudioRendition {
                        url: absolute(base, uri)?,
                        group_id: (*group).to_owned(),
                        name: (*name).to_owned(),
                        language,
                        default,
                        autoselect,
                        alternate: attrs.get("DEFAULT") == Some(&"NO")
                            || attrs.get("AUTOSELECT") == Some(&"NO"),
                    });
                }
            }
            groups
                .entry((*kind, *group))
                .and_modify(|value| *value |= external)
                .or_insert(external);
        }
    }
    let mut variants = Vec::new();
    let mut fragments: Vec<Fragment> = Vec::new();
    let mut pending_variant = None;
    let mut pending_duration = false;
    let mut pending_range = None;
    let mut endlist = false;
    let mut media_seen = false;
    let mut has_map = false;
    let mut ext: Option<&str> = None;
    let mut current_key: Option<AesKey> = None;
    let mut sequence = Some(0u64);
    let mut sequence_seen = false;
    let mut discontinuity = Vec::new();
    let mut discontinuities = 0u32;
    for line in lines.into_iter().skip(1) {
        if endlist && (line.starts_with("#EXT") || !line.starts_with('#')) {
            return Err(invalid("HLS ENDLIST must terminate the media playlist"));
        }
        if line == "#EXT-X-ENDLIST" {
            if endlist || pending_duration || pending_range.is_some() || pending_variant.is_some() {
                return Err(invalid("misplaced HLS ENDLIST"));
            }
            endlist = true;
            media_seen = true;
        } else if let Some(value) = line.strip_prefix("#EXT-X-STREAM-INF:") {
            if media_seen || pending_variant.is_some() {
                return Err(invalid("mixed or incomplete HLS playlist"));
            }
            pending_variant = Some(attributes(value)?);
        } else if let Some(value) = line.strip_prefix("#EXTINF:") {
            if endlist || !variants.is_empty() || pending_variant.is_some() || pending_duration {
                return Err(invalid("misplaced HLS segment duration"));
            }
            let duration = value
                .split(',')
                .next()
                .unwrap_or("")
                .parse::<f64>()
                .map_err(|_| invalid("invalid HLS duration"))?;
            if !duration.is_finite() || duration <= 0.0 {
                return Err(invalid("invalid HLS duration"));
            }
            pending_duration = true;
            media_seen = true;
        } else if let Some(value) = line.strip_prefix("#EXT-X-BYTERANGE:") {
            if endlist
                || pending_range.is_some()
                || !variants.is_empty()
                || pending_variant.is_some()
            {
                return Err(invalid("misplaced HLS byte range"));
            }
            pending_range = Some(value);
            media_seen = true;
        } else if let Some(value) = line.strip_prefix("#EXT-X-MAP:") {
            if endlist
                || has_map
                || !fragments.is_empty()
                || pending_duration
                || pending_range.is_some()
                || pending_variant.is_some()
                || !variants.is_empty()
            {
                return Err(invalid("only one initial HLS MAP is supported"));
            }
            let attrs = attributes(value)?;
            let url = absolute(
                base,
                attrs
                    .get("URI")
                    .ok_or_else(|| invalid("HLS MAP has no URI"))?,
            )?;
            let range = attrs
                .get("BYTERANGE")
                .map(|value| byte_range(value, None, &url, true))
                .transpose()?;
            let encryption = current_key
                .as_ref()
                .map(|key| key.snapshot(0, true))
                .transpose()?;
            fragments.push(Fragment {
                url,
                byte_range: range,
                encryption,
                path: None,
                duration: None,
            });
            discontinuity.push(discontinuities);
            has_map = true;
            media_seen = true;
            ext = Some("mp4");
        } else if let Some(value) = line.strip_prefix("#EXT-X-KEY:") {
            if !variants.is_empty() || pending_variant.is_some() {
                return Err(invalid("HLS media key appears in a master playlist"));
            }
            media_seen = true;
            current_key = aes_key(value, base)?;
        } else if let Some(value) = line.strip_prefix("#EXT-X-VERSION:") {
            positive_integer(value)?;
        } else if let Some(value) = line.strip_prefix("#EXT-X-TARGETDURATION:") {
            positive_integer(value)?;
            media_seen = true;
        } else if let Some(value) = line.strip_prefix("#EXT-X-MEDIA-SEQUENCE:") {
            if sequence_seen
                || fragments.len() > usize::from(has_map)
                || pending_duration
                || pending_range.is_some()
            {
                return Err(invalid(
                    "HLS media sequence must occur once before media segments",
                ));
            }
            sequence = Some(unsigned(value)?);
            sequence_seen = true;
            media_seen = true;
        } else if let Some(value) = line.strip_prefix("#EXT-X-PLAYLIST-TYPE:") {
            if !matches!(value, "VOD" | "EVENT") {
                return Err(invalid("invalid HLS playlist type"));
            }
            media_seen = true;
        } else if let Some(value) = line.strip_prefix("#EXT-X-ALLOW-CACHE:") {
            if !matches!(value, "YES" | "NO") {
                return Err(invalid("invalid HLS cache policy"));
            }
        } else if line.starts_with("#EXT-X-DISCONTINUITY") {
            // Upstream counts `#EXT-X-DISCONTINUITY-SEQUENCE` lines too.
            if !variants.is_empty() || pending_variant.is_some() {
                return Err(invalid("HLS discontinuity appears in a master playlist"));
            }
            if line != "#EXT-X-DISCONTINUITY"
                && line
                    .strip_prefix("#EXT-X-DISCONTINUITY-SEQUENCE:")
                    .is_none_or(|value| unsigned(value).is_err())
            {
                return Err(invalid("invalid HLS discontinuity tag"));
            }
            discontinuities = discontinuities
                .checked_add(1)
                .ok_or_else(|| invalid("HLS discontinuity count overflow"))?;
            media_seen = true;
        } else if line == "#EXT-X-INDEPENDENT-SEGMENTS"
            || line.starts_with("#EXT-X-PROGRAM-DATE-TIME:")
            || line.starts_with("#EXT-X-MEDIA:")
            // Twitch's informational vendor tags (`#EXT-X-TWITCH-INFO`,
            // `-ELAPSED-SECS`, `-TOTAL-SECS`), which upstream ignores.
            || line.starts_with("#EXT-X-TWITCH-")
        {
            // Informational metadata; media groups are validated against references above.
        } else if line.starts_with("#EXT-X-") || line.starts_with("#EXT") {
            return Err(invalid(format!(
                "unsupported HLS tag: {}",
                line.split(':').next().unwrap_or(line)
            )));
        } else if line.starts_with('#') {
            continue;
        } else {
            let url = if pending_variant.is_some() {
                absolute_variant(base, line)?
            } else {
                absolute(base, line)?
            };
            if let Some(attrs) = pending_variant.take() {
                if variants.len() + rendition_count == MAX_VARIANTS {
                    return Err(invalid("HLS variant limit exceeded"));
                }
                let mut found = variant(&attrs, url, &groups, &group_names)?;
                found.progressive = attrs
                    .get("PROGRESSIVE-URI")
                    .filter(|uri| !uri.is_empty())
                    .map(|uri| absolute_variant(base, uri))
                    .transpose()?;
                variants.push(found);
                continue;
            }
            if endlist || !pending_duration || !variants.is_empty() {
                return Err(invalid("HLS media URI has no preceding EXTINF"));
            }
            if fragments.len() == MAX_FRAGMENTS {
                return Err(invalid("HLS fragment limit exceeded"));
            }
            if !has_map {
                let suffix = url
                    .path()
                    .rsplit('/')
                    .next()
                    .unwrap_or("")
                    .rsplit('.')
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase();
                let segment_ext = match suffix.as_str() {
                    "aac" => "aac",
                    "m3u8" | "mpd" | "html" | "htm" | "mp4" | "m4s" => {
                        return Err(invalid(
                            "unsupported HLS segment without initialization MAP",
                        ));
                    }
                    _ => "ts",
                };
                if ext.is_some_and(|ext| ext != segment_ext) {
                    return Err(invalid("mixed HLS segment containers"));
                }
                ext = Some(segment_ext);
            }
            let previous = fragments
                .last()
                .filter(|_| fragments.len() > usize::from(has_map))
                .and_then(|fragment| {
                    fragment
                        .byte_range
                        .as_ref()
                        .map(|range| (&fragment.url, range))
                });
            let range = pending_range
                .take()
                .map(|value| byte_range(value, previous, &url, false))
                .transpose()?;
            let number = sequence.ok_or_else(|| invalid("HLS media sequence overflow"))?;
            let encryption = current_key
                .as_ref()
                .map(|key| key.snapshot(number, false))
                .transpose()?;
            fragments.push(Fragment {
                url,
                byte_range: range,
                encryption,
                path: None,
                duration: None,
            });
            discontinuity.push(discontinuities);
            sequence = number.checked_add(1);
            pending_duration = false;
        }
    }
    if pending_variant.is_some() || pending_duration || pending_range.is_some() {
        return Err(invalid("incomplete HLS playlist"));
    }
    if !variants.is_empty() || !audio.is_empty() {
        if media_seen {
            return Err(invalid("mixed HLS master/media playlist"));
        }
        return Ok(Playlist::Master {
            variants,
            audio,
            subtitles,
        });
    }
    if !endlist {
        return Err(invalid("live HLS without ENDLIST is unsupported"));
    }
    if fragments.len() <= usize::from(has_map) {
        return Err(invalid("HLS playlist has no media segments"));
    }
    if !groups.is_empty() {
        return Err(invalid("HLS media playlist contains rendition groups"));
    }
    Ok(Playlist::Media {
        fragments,
        discontinuity,
        discontinuities,
        ext: ext.unwrap_or("ts").into(),
    })
}

/// The fragments of discontinuity section `index` (upstream HlsFD with a
/// `format_index`), or all of them.
pub(crate) fn section(
    fragments: Vec<Fragment>,
    discontinuity: &[u32],
    index: Option<u32>,
) -> Vec<Fragment> {
    match index {
        None => fragments,
        Some(index) => fragments
            .into_iter()
            .zip(discontinuity)
            .filter(|(_, count)| **count == index)
            .map(|(fragment, _)| fragment)
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn base() -> Url {
        Url::parse("https://example.com/path/list.m3u8?token=one").unwrap()
    }
    fn media(text: &str) -> (Vec<Fragment>, String) {
        match parse(text, &base()).unwrap() {
            Playlist::Media { fragments, ext, .. } => (fragments, ext),
            _ => panic!("expected media playlist"),
        }
    }
    #[test]
    fn vod_segments_preserve_order_and_resolve_relative_urls() {
        let (fragments, ext) = media(
            "\u{feff}#EXTM3U\r\n#EXT-X-VERSION:3\r\n#EXT-X-TARGETDURATION:6\r\n#EXT-X-MEDIA-SEQUENCE:12\r\n#EXTINF:5.25,title\r\na.ts?sig=x\r\n# comment\r\n#EXTINF:4,\r\n../b.ts\r\n#EXT-X-ENDLIST\r\n",
        );
        assert_eq!(ext, "ts");
        assert_eq!(fragments.len(), 2);
        assert_eq!(
            fragments[0].url.as_str(),
            "https://example.com/path/a.ts?sig=x"
        );
        assert_eq!(fragments[1].url.as_str(), "https://example.com/b.ts");
        assert!(fragments.iter().all(|f| f.byte_range.is_none()));
        assert_eq!(
            media("#EXTM3U\n#EXTINF:1,\nchunk?id=1\n#EXT-X-ENDLIST").1,
            "ts"
        );
        assert_eq!(
            media("#EXTM3U\n#EXT-X-KEY:METHOD=NONE\n#EXTINF:1,\na.aac\n#EXT-X-ENDLIST").1,
            "aac"
        );
    }
    #[test]
    fn variant_frame_rate_is_upstream_float_or_none() {
        let Playlist::Master { variants, .. } = parse("#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1,FRAME-RATE=29.970\na.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=1,FRAME-RATE=fast\nb.m3u8", &base()).unwrap() else { panic!() };
        assert_eq!(variants[0].fps, Some(29.97));
        assert_eq!(variants[1].fps, None);
    }

    #[test]
    fn master_quoted_codecs_dimensions_bandwidth_and_audio_kind() {
        let Playlist::Master { variants, .. } = parse("#EXTM3U\n#EXT-X-INDEPENDENT-SEGMENTS\n#EXT-X-STREAM-INF:BANDWIDTH=900000,AVERAGE-BANDWIDTH=800000,RESOLUTION=1280x720,CODECS=\"avc1.64001F,mp4a.40.2\"\nvideo/playlist.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=128000,CODECS=\"mp4a.40.2\"\naudio.m3u8", &base()).unwrap() else { panic!() };
        assert_eq!(variants.len(), 2);
        assert_eq!(
            variants[0].url.as_str(),
            "https://example.com/path/video/playlist.m3u8"
        );
        assert_eq!(variants[0].bandwidth, Some(800.0));
        assert_eq!(
            (variants[0].width, variants[0].height),
            (Some(1280), Some(720))
        );
        assert_eq!(variants[0].stream_kind, StreamKind::Standalone);
        assert_eq!(variants[1].stream_kind, StreamKind::AudioOnly);
        assert_eq!(variants[1].bandwidth, Some(128.0));
    }
    #[test]
    fn master_inband_groups_are_resolved_even_when_declared_later() {
        let Playlist::Master { variants, .. } = parse("#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1000,AUDIO=\"a\",CLOSED-CAPTIONS=NONE,CODECS=\"avc1.4,mp4a.40.2\"\nv.m3u8\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"Audio, original\"", &base()).unwrap() else { panic!() };
        assert_eq!(variants[0].stream_kind, StreamKind::Standalone);
    }
    #[test]
    fn external_audio_metadata_classifies_referencing_video_and_allows_audio_only_master() {
        let text = "#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"English, main\",LANGUAGE=\"en\",DEFAULT=YES,AUTOSELECT=YES,URI=\"en.m3u8\"\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"Norsk\",LANGUAGE=\"nb\",DEFAULT=NO,AUTOSELECT=YES,URI=\"nb.m3u8\"\n#EXT-X-STREAM-INF:BANDWIDTH=900000,CODECS=\"avc1.4,mp4a.40.2\",AUDIO=\"a\"\nv.m3u8";
        let Playlist::Master {
            variants, audio, ..
        } = parse(text, &base()).unwrap()
        else {
            panic!()
        };
        assert_eq!(variants[0].stream_kind, StreamKind::VideoOnly);
        assert_eq!(variants[0].audio_group.as_deref(), Some("a"));
        assert_eq!(audio.len(), 2);
        assert_eq!(audio[0].name, "English, main");
        assert_eq!(audio[0].language.as_deref(), Some("en"));
        assert_eq!(audio[0].url.as_str(), "https://example.com/path/en.m3u8");
        assert!(audio[0].default && audio[0].autoselect);
        assert!(!audio[1].default);
        let only_audio = text.split("#EXT-X-STREAM-INF").next().unwrap();
        assert!(
            matches!(parse(only_audio, &base()).unwrap(), Playlist::Master { variants, audio, .. } if variants.is_empty() && audio.len() == 2)
        );
    }

    #[test]
    fn audio_groups_reject_ambiguous_metadata_and_keep_unpaired_variants() {
        let rendition = "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"Main\",URI=\"a.m3u8\"";
        for extra in [
            ",DEFAULT=maybe",
            ",AUTOSELECT=yes",
            ",DEFAULT=YES,AUTOSELECT=NO",
        ] {
            assert!(
                parse(&format!("#EXTM3U\n{rendition}{extra}"), &base()).is_err(),
                "{extra}"
            );
        }
        for second in [
            rendition.to_owned(),
            "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"Other\"".into(),
            format!("{rendition},DEFAULT=YES"),
        ] {
            let first = format!("{rendition},DEFAULT=YES");
            assert!(parse(&format!("#EXTM3U\n{first}\n{second}"), &base()).is_err());
        }
        // Upstream treats a variant without video CODECS as complete (or
        // audio-only) instead of pairing it with the external group.
        for (codec, kind) in [
            ("", StreamKind::Standalone),
            (",CODECS=\"mp4a.40.2\"", StreamKind::AudioOnly),
        ] {
            let text = format!(
                "#EXTM3U\n{rendition}\n#EXT-X-STREAM-INF:BANDWIDTH=1000,AUDIO=\"a\"{codec}\nv.m3u8"
            );
            let Playlist::Master { variants, .. } = parse(&text, &base()).unwrap() else {
                panic!()
            };
            assert_eq!(variants[0].stream_kind, kind);
        }
        for uri in ["", "file:///audio.m3u8", "audio.m3u8#fragment"] {
            assert!(parse(&format!("#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"Main\",URI=\"{uri}\""), &base()).is_err());
        }
        let declarations = (0..MAX_VARIANTS)
            .map(|index| {
                format!("#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"{index}\",URI=\"a.m3u8\"\n")
            })
            .collect::<String>();
        assert!(parse(&format!("#EXTM3U\n{declarations}"), &base()).is_ok());
        assert!(
            parse(
                &format!(
                    "#EXTM3U\n{declarations}#EXT-X-STREAM-INF:CODECS=\"avc1.4\",AUDIO=\"a\"\nv.m3u8"
                ),
                &base()
            )
            .is_err()
        );
    }

    #[test]
    fn fmp4_map_precedes_explicit_and_implicit_ranges() {
        let (fragments, ext) = media(
            "#EXTM3U\n#EXT-X-MAP:URI=\"all.mp4\",BYTERANGE=\"20@0\"\n#EXTINF:1,\n#EXT-X-BYTERANGE:10@20\nall.mp4\n#EXTINF:1,\n#EXT-X-BYTERANGE:15\nall.mp4\n#EXT-X-ENDLIST",
        );
        assert_eq!(ext, "mp4");
        assert_eq!(fragments.len(), 3);
        let ranges: Vec<_> = fragments
            .iter()
            .map(|f| {
                let r = f.byte_range.as_ref().unwrap();
                (r.offset, r.length)
            })
            .collect();
        assert_eq!(ranges, [(0, 20), (20, 10), (30, 15)]);
        assert_eq!(
            media("#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:1,\na.m4s\n#EXT-X-ENDLIST").1,
            "mp4"
        );
    }
    #[test]
    fn rejects_unsafe_or_unsupported_state_and_incomplete_playlists() {
        let valid = "#EXTM3U\n#EXTINF:1,\na.ts\n#EXT-X-ENDLIST";
        for tag in [
            "#EXT-X-KEY:METHOD=SAMPLE-AES",
            "#EXT-X-KEY:METHOD=NONE,URI=\"key\"",
            "#EXT-X-DISCONTINUITY-SEQUENCE:x",
            "#EXT-X-GAP",
            "#EXT-X-PART:DURATION=1,URI=\"a.ts\"",
            "#EXT-X-PRELOAD-HINT:TYPE=PART,URI=\"a\"",
            "#EXT-X-SKIP:SKIPPED-SEGMENTS=1",
            "#EXT-X-UNKNOWN:1",
            "#EXT-X-START:TIME-OFFSET=2",
        ] {
            let text = valid.replace("#EXTINF", &format!("{tag}\n#EXTINF"));
            assert!(parse(&text, &base()).is_err(), "{tag}");
        }
        for text in [
            "not a manifest",
            "#EXTM3U",
            "#EXTM3U\n#EXTINF:1,\na.ts",
            "#EXTM3U\n#EXT-X-ENDLIST",
            "#EXTM3U\n#EXTINF:1,\n#EXT-X-ENDLIST",
            "#EXTM3U\na.ts\n#EXT-X-ENDLIST",
            "#EXTM3U\n#EXTINF:NaN,\na.ts\n#EXT-X-ENDLIST",
            "#EXTM3U\n#EXTINF:0,\na.ts\n#EXT-X-ENDLIST",
            "#EXTM3U\n#EXTINF:1,\nfile:///a.ts\n#EXT-X-ENDLIST",
            "#EXTM3U\n#EXTINF:1,\na.m3u8\n#EXT-X-ENDLIST",
            "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1000",
            "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1000\na.m3u8\n#EXTINF:1,\na.ts\n#EXT-X-ENDLIST",
        ] {
            assert!(parse(text, &base()).is_err(), "{text}");
        }
    }
    #[test]
    fn discontinuities_are_counted_per_fragment_and_select_sections() {
        let text = "#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXTINF:1,\na.ts\n#EXT-X-DISCONTINUITY\n#EXTINF:1,\nb.ts\n#EXTINF:1,\nc.ts\n#EXT-X-DISCONTINUITY-SEQUENCE:4\n#EXTINF:1,\nd.ts\n#EXT-X-ENDLIST";
        let Playlist::Media {
            fragments,
            discontinuity,
            discontinuities,
            ..
        } = parse(text, &base()).unwrap()
        else {
            panic!("media playlist");
        };
        // Like upstream, DISCONTINUITY-SEQUENCE lines count too.
        assert_eq!(discontinuity, [0, 1, 1, 2]);
        assert_eq!(discontinuities, 2);
        let names = |index| {
            section(fragments.clone(), &discontinuity, index)
                .iter()
                .map(|fragment| fragment.url.path().to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(names(Some(1)), ["/path/b.ts", "/path/c.ts"]);
        assert_eq!(names(Some(3)), Vec::<String>::new());
        assert_eq!(names(None).len(), 4);
    }
    #[test]
    fn endlist_is_terminal_and_media_keys_cannot_appear_in_master() {
        let text = "#EXTM3U\n#EXTINF:1,\na.ts\n#EXT-X-ENDLIST";
        assert!(parse(&format!("{text}\n# trailing comment"), &base()).is_ok());
        for trailing in [
            "#EXT-X-KEY:METHOD=NONE",
            "#EXT-X-VERSION:3",
            "#EXT-X-ENDLIST",
            "b.ts",
        ] {
            assert!(parse(&format!("{text}\n{trailing}"), &base()).is_err());
        }
        assert!(
            parse(
                "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1000\na.m3u8\n#EXT-X-KEY:METHOD=NONE",
                &base()
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_external_non_audio_rendition_groups() {
        for group in ["VIDEO", "CLOSED-CAPTIONS"] {
            let text = format!(
                "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1000,{group}=\"a\"\na.m3u8\n#EXT-X-MEDIA:TYPE={group},GROUP-ID=\"a\",NAME=\"A\",URI=\"extra.m3u8\""
            );
            assert!(parse(&text, &base()).is_err());
            let text =
                format!("#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1000,{group}=\"missing\"\na.m3u8");
            assert!(parse(&text, &base()).is_err());
        }
        // Like upstream, an unknown AUDIO group is ignored.
        let text = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1000,AUDIO=\"missing\"\na.m3u8";
        assert!(parse(text, &base()).is_ok());
    }
    #[test]
    fn rejects_invalid_ranges_maps_and_container_changes() {
        for body in [
            "#EXT-X-BYTERANGE:10\n#EXTINF:1,\na.ts",
            "#EXT-X-BYTERANGE:0@0\n#EXTINF:1,\na.ts",
            "#EXT-X-BYTERANGE:2@18446744073709551615\n#EXTINF:1,\na.ts",
            "#EXT-X-BYTERANGE:1@0@1\n#EXTINF:1,\na.ts",
            "#EXTINF:1,\n#EXT-X-BYTERANGE:10@0\na.ts\n#EXTINF:1,\n#EXT-X-BYTERANGE:10\nb.ts",
            "#EXT-X-MAP:URI=\"a.mp4\",BYTERANGE=\"10\"\n#EXTINF:1,\na.m4s",
            "#EXT-X-MAP:URI=\"a.mp4\",BYTERANGE=\"10@0\"\n#EXTINF:1,\n#EXT-X-BYTERANGE:10\na.mp4",
            "#EXTINF:1,\na.ts\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:1,\nb.m4s",
            "#EXT-X-MAP:URI=\"init.mp4\"\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:1,\na.m4s",
            "#EXTINF:1,\na.aac\n#EXTINF:1,\nb.ts",
        ] {
            let text = format!("#EXTM3U\n{body}\n#EXT-X-ENDLIST");
            assert!(parse(&text, &base()).is_err(), "{body}");
        }
    }
    #[test]
    fn malformed_attributes_fail_closed() {
        for attributes in [
            "CODECS=\"avc1,mp4a",
            "BANDWIDTH=1000,BANDWIDTH=2000",
            "BANDWIDTH=1000,",
            "RESOLUTION=0x720",
            "RESOLUTION=1280",
            "BANDWIDTH=1000 junk",
            "CODECS=\"avc1\"junk",
            "BANDWIDTH=1,,CODECS=\"avc1\"",
        ] {
            assert!(
                parse(
                    &format!("#EXTM3U\n#EXT-X-STREAM-INF:{attributes}\na.m3u8"),
                    &base()
                )
                .is_err(),
                "{attributes}"
            );
        }
    }
    #[test]
    fn aes_implicit_iv_uses_media_sequence_and_rotation_snapshots() {
        let (fragments, _) = media(
            "#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:255\n#EXT-X-KEY:METHOD=AES-128,URI=\"../key-a\"\n#EXTINF:1,\na.ts\n#EXT-X-KEY:METHOD=AES-128,URI=\"key-b\",IV=0xabc,KEYFORMAT=\"identity\",KEYFORMATVERSIONS=\"1\"\n#EXTINF:1,\nb.ts\n#EXT-X-KEY:METHOD=NONE\n#EXTINF:1,\nc.ts\n#EXT-X-KEY:METHOD=AES-128,URI=\"key-a\"\n#EXTINF:1,\nd.ts\n#EXT-X-ENDLIST",
        );
        let first = fragments[0].encryption.as_ref().unwrap();
        assert_eq!(first.key_url.as_str(), "https://example.com/key-a");
        assert_eq!(first.iv, 255u128.to_be_bytes());
        let second = fragments[1].encryption.as_ref().unwrap();
        assert_eq!(second.key_url.as_str(), "https://example.com/path/key-b");
        assert_eq!(second.iv, 0xabcu128.to_be_bytes());
        assert!(fragments[2].encryption.is_none());
        assert_eq!(
            fragments[3].encryption.as_ref().unwrap().iv,
            258u128.to_be_bytes()
        );
        let (fragments, _) = media(
            "#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"key\"\n#EXTINF:1,\na.ts\n#EXTINF:1,\nb.ts\n#EXT-X-ENDLIST",
        );
        assert_eq!(fragments[0].encryption.as_ref().unwrap().iv, [0; 16]);
        assert_eq!(
            fragments[1].encryption.as_ref().unwrap().iv,
            1u128.to_be_bytes()
        );
    }

    #[test]
    fn encrypted_map_requires_explicit_iv_and_does_not_increment_media_sequence() {
        let text = "#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:7\n#EXT-X-KEY:METHOD=AES-128,URI=\"key\",IV=0Xffffffffffffffffffffffffffffffff\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXT-X-KEY:METHOD=AES-128,URI=\"key\"\n#EXTINF:1,\na.m4s\n#EXT-X-ENDLIST";
        let (fragments, ext) = media(text);
        assert_eq!(ext, "mp4");
        assert_eq!(fragments[0].encryption.as_ref().unwrap().iv, [255; 16]);
        assert_eq!(
            fragments[1].encryption.as_ref().unwrap().iv,
            7u128.to_be_bytes()
        );
        assert!(
            parse(
                &text.replace(",IV=0Xffffffffffffffffffffffffffffffff", ""),
                &base()
            )
            .is_err()
        );
        assert!(
            parse(
                &text.replace("#EXT-X-MEDIA-SEQUENCE:7\n", "").replace(
                    "#EXT-X-MAP:URI=\"init.mp4\"\n",
                    "#EXT-X-MAP:URI=\"init.mp4\"\n#EXT-X-MEDIA-SEQUENCE:7\n"
                ),
                &base()
            )
            .is_ok()
        );
    }

    #[test]
    fn encrypted_byte_ranges_retain_their_own_iv_and_do_not_alter_offsets() {
        let (fragments, _) = media(
            "#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"key\"\n#EXTINF:1,\n#EXT-X-BYTERANGE:16@32\na.ts\n#EXTINF:1,\n#EXT-X-BYTERANGE:32\na.ts\n#EXT-X-ENDLIST",
        );
        assert_eq!(fragments[0].byte_range.as_ref().unwrap().offset, 32);
        assert_eq!(fragments[1].byte_range.as_ref().unwrap().offset, 48);
        assert_eq!(fragments[1].byte_range.as_ref().unwrap().length, 32);
        assert_eq!(
            fragments[0].encryption.as_ref().unwrap().iv,
            0u128.to_be_bytes()
        );
        assert_eq!(
            fragments[1].encryption.as_ref().unwrap().iv,
            1u128.to_be_bytes()
        );
    }

    #[test]
    fn malformed_encryption_keys_and_iv_fail_closed() {
        for attrs in [
            "METHOD=AES-128",
            "METHOD=AES-128,URI=\"\"",
            "METHOD=AES-128,URI=\"file:///key\"",
            "METHOD=AES-128,URI=\"data:key\"",
            "METHOD=AES-128,URI=\"key#fragment\"",
            "METHOD=AES-128,URI=\"key space\"",
            "METHOD=AES-128,URI=\"key\",KEYFORMAT=\"com.apple.streamingkeydelivery\"",
            "METHOD=AES-128,URI=\"key\",KEYFORMATVERSIONS=\"1/2\"",
            "METHOD=AES-128,URI=\"key\",UNKNOWN=1",
            "METHOD=SAMPLE-AES,URI=\"key\"",
            "METHOD=NONE,IV=0x1",
            "METHOD=AES-128,URI=\"key\",IV=0x",
            "METHOD=AES-128,URI=\"key\",IV=abc",
            "METHOD=AES-128,URI=\"key\",IV=0xgg",
            "METHOD=AES-128,URI=\"key\",IV=0x100000000000000000000000000000000",
        ] {
            let text = format!("#EXTM3U\n#EXT-X-KEY:{attrs}\n#EXTINF:1,\na.ts\n#EXT-X-ENDLIST");
            assert!(parse(&text, &base()).is_err(), "{attrs}");
        }
    }

    #[test]
    fn media_sequence_rejects_late_duplicate_and_overflow_without_wrapping() {
        for body in [
            "#EXT-X-MEDIA-SEQUENCE:1\n#EXT-X-MEDIA-SEQUENCE:1\n#EXTINF:1,\na.ts",
            "#EXTINF:1,\na.ts\n#EXT-X-MEDIA-SEQUENCE:1",
            "#EXTINF:1,\n#EXT-X-MEDIA-SEQUENCE:1\na.ts",
            "#EXT-X-MEDIA-SEQUENCE:18446744073709551616\n#EXTINF:1,\na.ts",
            "#EXT-X-MEDIA-SEQUENCE:18446744073709551615\n#EXTINF:1,\na.ts\n#EXTINF:1,\nb.ts",
        ] {
            assert!(
                parse(&format!("#EXTM3U\n{body}\n#EXT-X-ENDLIST"), &base()).is_err(),
                "{body}"
            );
        }
        let (fragments, _) = media(
            "#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:18446744073709551615\n#EXT-X-KEY:METHOD=AES-128,URI=\"key\"\n#EXTINF:1,\na.ts\n#EXT-X-ENDLIST",
        );
        assert_eq!(
            fragments[0].encryption.as_ref().unwrap().iv,
            u128::from(u64::MAX).to_be_bytes()
        );
    }

    #[test]
    fn enforces_fragment_and_variant_resource_limits() {
        let media = format!(
            "#EXTM3U\n{}#EXT-X-ENDLIST",
            "#EXTINF:1,\na.ts\n".repeat(MAX_FRAGMENTS)
        );
        assert!(parse(&media, &base()).is_ok());
        let media = media.replace("#EXT-X-ENDLIST", "#EXTINF:1,\nb.ts\n#EXT-X-ENDLIST");
        assert!(parse(&media, &base()).is_err());
        let master = format!(
            "#EXTM3U\n{}",
            "#EXT-X-STREAM-INF:BANDWIDTH=1000\na.m3u8\n".repeat(MAX_VARIANTS)
        );
        assert!(parse(&master, &base()).is_ok());
        assert!(
            parse(
                &format!("{master}#EXT-X-STREAM-INF:BANDWIDTH=1000\nb.m3u8"),
                &base()
            )
            .is_err()
        );
    }
}

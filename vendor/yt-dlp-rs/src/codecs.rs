//! Upstream `parse_codecs` (RFC 6381 codec lists) and `get_compatible_ext`.

/// The codec fields `parse_codecs` returns for a non-empty result.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Codecs {
    pub vcodec: String,
    pub acodec: String,
    /// `None` both for SDR/unknown and for the two-unknown-codecs fallback,
    /// which upstream returns without a `dynamic_range` key.
    pub dynamic_range: Option<&'static str>,
    pub scodec: Option<String>,
}

const VIDEO: &[&str] = &[
    "avc1", "avc2", "avc3", "avc4", "vp9", "vp8", "hev1", "hev2", "h263", "h264", "mp4v", "hvc1",
    "av1", "theora", "dvh1", "dvhe",
];
const AUDIO: &[&str] = &[
    "flac", "mp4a", "opus", "vorbis", "mp3", "aac", "ac-4", "ac-3", "ec-3", "eac3", "dtsc", "dtse",
    "dtsh", "dtsl",
];

/// Python `str.strip()` whitespace.
fn py_strip(text: &str) -> &str {
    text.trim_matches(|c: char| c.is_whitespace())
}

/// `re.sub(r'0+(?=\d)', '', codec)`: drop runs of zeros followed by a digit.
fn strip_leading_zeros(codec: &str) -> String {
    let bytes = codec.as_bytes();
    let mut out = String::with_capacity(codec.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'0' {
            let mut end = index;
            while end < bytes.len() && bytes[end] == b'0' {
                end += 1;
            }
            // The regex is greedy but backtracks to leave the final digit.
            if end < bytes.len() && bytes[end].is_ascii_digit() {
                index = end;
            } else if end - index > 1 {
                // `0+(?=\d)` matches all but the last zero of the run.
                index = end - 1;
            } else {
                out.push('0');
                index += 1;
            }
            continue;
        }
        let ch = codec[index..].chars().next().unwrap();
        out.push(ch);
        index += ch.len_utf8();
    }
    out
}

/// Upstream `parse_codecs`. Unknown codecs are reported through `warn`
/// (upstream writes `WARNING: Unknown codec X` to stderr directly).
pub fn parse_codecs(text: Option<&str>, warn: &mut dyn FnMut(&str)) -> Option<Codecs> {
    let text = text.filter(|text| !text.is_empty())?;
    let split: Vec<&str> = py_strip(text)
        .trim_matches(',')
        .split(',')
        .map(py_strip)
        .filter(|codec| !codec.is_empty())
        .collect();
    let (mut vcodec, mut acodec, mut scodec, mut hdr) = (None, None, None, None);
    for codec in &split {
        // `re.sub(r'^([^.]+)', lower)`: lowercase the part before the first dot.
        let full = match codec.find('.') {
            Some(0) => (*codec).to_owned(),
            Some(dot) => format!("{}{}", codec[..dot].to_lowercase(), &codec[dot..]),
            None => codec.to_lowercase(),
        };
        let stripped = strip_leading_zeros(&full);
        let parts: Vec<&str> = stripped.split('.').collect();
        let family = parts[0];
        if VIDEO.contains(&family) {
            if vcodec.is_some() {
                continue;
            }
            vcodec = Some(full);
            if matches!(family, "dvh1" | "dvhe") {
                hdr = Some("DV");
            } else if (family == "av1" && parts.get(3) == Some(&"10"))
                || (family == "vp9" && parts.get(1) == Some(&"2"))
            {
                hdr = Some("HDR10");
            }
        } else if AUDIO.contains(&family) {
            acodec = acodec.or(Some(full));
        } else if matches!(family, "stpp" | "wvtt") {
            scodec = scodec.or(Some(full));
        } else {
            warn(&format!("WARNING: Unknown codec {full}"));
        }
    }
    if vcodec.is_some() || acodec.is_some() || scodec.is_some() {
        return Some(Codecs {
            vcodec: vcodec.unwrap_or_else(|| "none".into()),
            acodec: acodec.unwrap_or_else(|| "none".into()),
            dynamic_range: hdr,
            scodec,
        });
    }
    if let [video, audio] = split[..] {
        return Some(Codecs {
            vcodec: video.into(),
            acodec: audio.into(),
            dynamic_range: None,
            scodec: None,
        });
    }
    None
}

/// `parse_codecs` with upstream's direct stderr warnings.
pub(crate) fn parse(text: Option<&str>) -> Option<Codecs> {
    parse_codecs(text, &mut |warning| crate::screen::stderr(warning))
}

/// Upstream `get_compatible_ext` for a merge.
pub fn compatible_ext(
    vcodecs: &[Option<&str>],
    acodecs: &[Option<&str>],
    vexts: &[&str],
    aexts: &[&str],
    preferences: Option<&[&str]>,
) -> String {
    let preferences = preferences.filter(|preferences| !preferences.is_empty());
    let allow_mkv = preferences.is_none_or(|preferences| preferences.contains(&"mkv"));
    if allow_mkv && acodecs.len().max(vcodecs.len()) > 1 {
        return "mkv".into();
    }
    const MP4: &[&str] = &[
        "av1", "hevc", "avc1", "mp4a", "ac-4", "h264", "aacl", "ec-3",
    ];
    const WEBM: &[&str] = &["av1", "vp9", "vp8", "opus", "vrbs", "vp9x", "vp8x"];
    // `x[0].split('.')[0].replace('0', '').lower()`, `None` when missing.
    let sanitize = |codecs: &[Option<&str>]| {
        codecs.first().copied().flatten().map(|codec| {
            codec
                .split('.')
                .next()
                .unwrap_or("")
                .replace('0', "")
                .to_lowercase()
        })
    };
    let (vcodec, acodec) = (sanitize(vcodecs), sanitize(acodecs));
    let defaults = ["mp4", "webm"];
    let order: Vec<&str> = preferences.map_or(defaults.to_vec(), <[&str]>::to_vec);
    for ext in &order {
        let set: &[&str] = match *ext {
            "mp4" => MP4,
            "webm" => WEBM,
            _ => &[],
        };
        let contains = |codec: &Option<String>| codec.as_deref().is_some_and(|c| set.contains(&c));
        if *ext == "mkv" || (contains(&vcodec) && contains(&acodec)) {
            return (*ext).into();
        }
    }
    const COMPATIBLE: [&[&str]; 2] = [
        &[
            "mp3", "mp4", "m4a", "m4p", "m4b", "m4r", "m4v", "ismv", "isma", "mov",
        ],
        &["webm", "weba"],
    ];
    let candidates: Vec<&str> = preferences.map_or(vexts.to_vec(), <[&str]>::to_vec);
    for ext in candidates {
        let mut current: Vec<&str> = vec![ext];
        current.extend(vexts.iter().chain(aexts));
        let single = current.iter().all(|candidate| *candidate == ext);
        if ext == "mkv"
            || single
            || COMPATIBLE
                .iter()
                .any(|set| current.iter().all(|candidate| set.contains(candidate)))
        {
            return ext.into();
        }
    }
    match preferences {
        Some(preferences) if !allow_mkv => (*preferences.last().unwrap()).into(),
        _ => "mkv".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn borrowed(values: &[Option<String>]) -> Vec<Option<&str>> {
        values.iter().map(Option::as_deref).collect()
    }

    fn as_strs(values: &[String]) -> Vec<&str> {
        values.iter().map(String::as_str).collect()
    }

    #[test]
    fn saved_upstream_parse_codecs_and_compatible_ext_cases() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/codecs/cases.json")).unwrap();
        for case in fixture["parse_codecs"].as_array().unwrap() {
            let input = case["input"].as_str().unwrap();
            let mut warnings = Vec::new();
            let parsed = parse_codecs(Some(input), &mut |warning| {
                warnings.push(warning.to_owned())
            });
            let expected = &case["result"];
            match parsed {
                None => assert_eq!(expected, &serde_json::json!({}), "{input:?}"),
                Some(codecs) => {
                    assert_eq!(codecs.vcodec, expected["vcodec"], "{input:?}");
                    assert_eq!(codecs.acodec, expected["acodec"], "{input:?}");
                    assert_eq!(
                        codecs.dynamic_range,
                        expected["dynamic_range"].as_str(),
                        "{input:?}"
                    );
                    assert_eq!(codecs.scodec.as_deref(), expected["scodec"].as_str());
                }
            }
            assert_eq!(
                warnings,
                case["warnings"].as_array().unwrap().to_owned(),
                "{input:?}"
            );
        }
        for case in fixture["compatible_ext"].as_array().unwrap() {
            let strings = |key: &str| -> Vec<Option<String>> {
                case[key]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|value| value.as_str().map(str::to_owned))
                    .collect()
            };
            let (vcodecs, acodecs) = (strings("vcodecs"), strings("acodecs"));

            let exts = |key: &str| -> Vec<String> {
                case[key]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|value| value.as_str().unwrap().to_owned())
                    .collect()
            };
            let (vexts, aexts) = (exts("vexts"), exts("aexts"));
            let preferences: Option<Vec<String>> = case["preferences"].as_array().map(|values| {
                values
                    .iter()
                    .map(|value| value.as_str().unwrap().to_owned())
                    .collect()
            });

            let preferences = preferences.as_deref().map(as_strs);
            assert_eq!(
                compatible_ext(
                    &borrowed(&vcodecs),
                    &borrowed(&acodecs),
                    &as_strs(&vexts),
                    &as_strs(&aexts),
                    preferences.as_deref(),
                ),
                case["ext"].as_str().unwrap(),
                "{case}"
            );
        }
    }
}

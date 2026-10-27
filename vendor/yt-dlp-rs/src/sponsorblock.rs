//! Upstream `SponsorBlockPP`: segments from the SponsorBlock API for YouTube
//! videos, turned into `sponsorblock_chapters` for `ModifyChaptersPP`.
use crate::{Error, Result, VideoInfo, info::PyNumber};

/// `SponsorBlockPP.CATEGORIES`, in upstream order.
pub const CATEGORIES: [(&str, &str); 11] = [
    ("sponsor", "Sponsor"),
    ("intro", "Intermission/Intro Animation"),
    ("outro", "Endcards/Credits"),
    ("selfpromo", "Unpaid/Self Promotion"),
    ("preview", "Preview/Recap"),
    ("filler", "Filler Tangent"),
    ("interaction", "Interaction Reminder"),
    ("music_offtopic", "Non-Music Section"),
    ("hook", "Hook/Greetings"),
    ("poi_highlight", "Highlight"),
    ("chapter", "Chapter"),
];

/// `NON_SKIPPABLE_CATEGORIES` (the POI and chapter categories).
pub const NON_SKIPPABLE: [&str; 2] = ["poi_highlight", "chapter"];

/// Upstream `_set_from_options_callback` for `--sponsorblock-mark`
/// (`remove == false`) and `--sponsorblock-remove`: comma-separated
/// categories, `all`, `default` and `-` exclusions, added to `current`.
pub fn parse_categories(
    current: &mut Vec<String>,
    value: &str,
    option: &str,
    remove: bool,
) -> std::result::Result<(), String> {
    let allowed: Vec<&str> = CATEGORIES
        .iter()
        .map(|(name, _)| *name)
        .filter(|name| !remove || !NON_SKIPPABLE.contains(name))
        .collect();
    let default: &[&str] = if remove {
        &["all", "-filler"]
    } else {
        &["all"]
    };
    // `orderedSet_from_options(values, ChainMap(aliases, {'all': allowed}))`.
    fn apply(
        current: &mut Vec<String>,
        values: &[String],
        allowed: &[&str],
        default: &[&str],
        recursing: bool,
    ) -> std::result::Result<(), String> {
        for value in values {
            let (discard, name) = match value.strip_prefix('-') {
                Some(name) => (true, name),
                None => (false, value.as_str()),
            };
            let alias: Option<Vec<String>> = match name {
                "all" => Some(allowed.iter().map(|name| (*name).to_owned()).collect()),
                "default" if !recursing => {
                    Some(default.iter().map(|name| (*name).to_owned()).collect())
                }
                _ => None,
            };
            if let Some(alias) = alias {
                let alias: Vec<String> = if discard {
                    alias
                        .into_iter()
                        .map(|name| match name.strip_prefix('-') {
                            Some(name) => name.to_owned(),
                            None => format!("-{name}"),
                        })
                        .collect()
                } else {
                    alias
                };
                apply(current, &alias, allowed, default, true)?;
                continue;
            }
            if !allowed.contains(&name) {
                return Err(name.to_owned());
            }
            if discard {
                current.retain(|item| item != name);
            } else if !current.iter().any(|item| item == name) {
                current.push(name.to_owned());
            }
        }
        Ok(())
    }
    let values: Vec<String> = value
        .split(',')
        .map(|value| value.trim().to_lowercase())
        .collect();
    apply(current, &values, &allowed, default, false)
        .map_err(|value| format!("wrong CATS for {option}: {value}"))
}

/// One fetched segment (upstream's `to_chapter` dict).
#[derive(Debug, Clone, PartialEq)]
pub struct SponsorChapter {
    pub start: PyNumber,
    pub end: PyNumber,
    pub category: String,
    pub title: String,
    pub action: String,
}

impl SponsorChapter {
    /// The info dict's `sponsorblock_chapters` entry.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "start_time": self.start,
            "end_time": self.end,
            "category": self.category,
            "title": self.title,
            "type": self.action,
            "_categories": [[self.category, self.start, self.end, self.title]],
        })
    }
}

/// SHA-256 (FIPS 180-4).
fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut state: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut message = data.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend(((data.len() as u64) * 8).to_be_bytes());
    for block in message.chunks(64) {
        let mut w = [0u32; 64];
        for (index, word) in block.chunks(4).enumerate() {
            w[index] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choice = (e & f) ^ (!e & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(choice)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(majority);
            (h, g, f, e, d, c, b, a) = (g, f, e, d.wrapping_add(t1), c, b, a, t1.wrapping_add(t2));
        }
        for (value, add) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *value = value.wrapping_add(add);
        }
    }
    let mut digest = [0u8; 32];
    for (chunk, value) in digest.chunks_mut(4).zip(state) {
        chunk.copy_from_slice(&value.to_be_bytes());
    }
    digest
}

/// Python `urllib.parse.quote_plus`.
fn quote_plus(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-' | b'~' => {
                out.push(byte as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// `_get_sponsor_segments`' query URL: `json.dumps` lists, `urlencode`.
pub fn query_url(api: &str, video_id: &str, categories: &[String]) -> String {
    let api = if api.starts_with("http://") || api.starts_with("https://") {
        api.to_owned()
    } else {
        format!("https://{api}")
    };
    let hash: String = sha256(video_id.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let dumps = |items: &[&str]| {
        let quoted: Vec<String> = items.iter().map(|item| format!("\"{item}\"")).collect();
        format!("[{}]", quoted.join(", "))
    };
    let categories: Vec<&str> = categories.iter().map(String::as_str).collect();
    format!(
        "{api}/api/skipSegments/{}?service=YouTube&categories={}&actionTypes={}",
        &hash[..4],
        quote_plus(&dumps(&categories)),
        quote_plus(&dumps(&["skip", "poi", "chapter"]))
    )
}

fn number(value: &serde_json::Value) -> Option<PyNumber> {
    serde_json::from_value(value.clone()).ok()
}

fn float(value: PyNumber) -> f64 {
    value.as_f64()
}

/// `_get_sponsor_chapters` over the API's segments for this video: the
/// chapters and whether some segments came from a different duration.
pub(crate) fn chapters(
    segments: &[serde_json::Value],
    duration: Option<PyNumber>,
) -> Result<(Vec<SponsorChapter>, bool)> {
    let invalid = || Error::Postprocess("invalid SponsorBlock segment".into());
    let mut chapters = Vec::new();
    for segment in segments {
        let bounds = segment["segment"].as_array().ok_or_else(invalid)?;
        let (Some(mut start), Some(mut end)) = (
            bounds.first().and_then(number),
            bounds.get(1).and_then(number),
        ) else {
            return Err(invalid());
        };
        let category = segment["category"].as_str().ok_or_else(invalid)?;
        // Upstream's `start_end == (0, 0)` compares a list with a tuple and
        // never matches, so whole-video segments are kept.
        if float(start) <= 1.0 {
            start = PyNumber::Int(0);
        }
        if category == "poi_highlight" {
            end = match end {
                PyNumber::Int(end) => PyNumber::Int(end + 1),
                PyNumber::Float(end) => PyNumber::Float(end + 1.0),
            };
        }
        if let Some(duration) = duration.filter(|duration| duration.as_f64() != 0.0)
            && duration.as_f64() - float(end) <= 1.0
        {
            end = duration;
        }
        let video_duration = segment["videoDuration"].as_f64().unwrap_or(0.0);
        let diff = if video_duration != 0.0 {
            (duration.map_or(0.0, PyNumber::as_f64) - video_duration).abs()
        } else {
            0.0
        };
        let matches = diff < 1.0 || (diff < 5.0 && diff / (float(end) - float(start)) < 0.05);
        if !matches {
            continue;
        }
        let title = if category == "chapter" {
            segment["description"].as_str().unwrap_or("").to_owned()
        } else {
            CATEGORIES
                .iter()
                .find(|(name, _)| *name == category)
                .map(|(_, title)| (*title).to_owned())
                .ok_or_else(invalid)?
        };
        chapters.push(SponsorChapter {
            start,
            end,
            category: category.to_owned(),
            title,
            action: segment["actionType"].as_str().unwrap_or("").to_owned(),
        });
    }
    let mismatched = chapters.len() != segments.len();
    Ok((chapters, mismatched))
}

/// Whether upstream's `SponsorBlockPP.EXTRACTORS` supports `info`.
pub(crate) fn supported(info: &VideoInfo) -> bool {
    info.extractor_key == "Youtube"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_and_query_urls_follow_python() {
        let hex = |bytes: [u8; 32]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
        assert_eq!(
            hex(sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hex(sha256(&[b'a'; 200])),
            "c2a908d98f5df987ade41b5fce213067efbcc21ef2240212a41e54b5e7c28ae5"
        );
        assert_eq!(
            query_url(
                "sponsor.example",
                "dQw4w9WgXcQ",
                &["sponsor".into(), "intro".into()]
            ),
            "https://sponsor.example/api/skipSegments/5f6b?service=YouTube&categories=%5B%22sponsor%22%2C+%22intro%22%5D&actionTypes=%5B%22skip%22%2C+%22poi%22%2C+%22chapter%22%5D"
        );
    }

    #[test]
    fn categories_follow_set_from_options_callback() {
        let parse = |values: &[&str], remove: bool| {
            let mut current = Vec::new();
            for value in values {
                parse_categories(&mut current, value, "--sponsorblock-mark", remove)?;
            }
            Ok::<_, String>(current)
        };
        assert_eq!(
            parse(&["sponsor,Intro"], false).unwrap(),
            ["sponsor", "intro"]
        );
        assert_eq!(parse(&["all,-preview"], false).unwrap().len(), 10);
        assert_eq!(
            parse(&["default"], true).unwrap(),
            [
                "sponsor",
                "intro",
                "outro",
                "selfpromo",
                "preview",
                "interaction",
                "music_offtopic",
                "hook"
            ]
        );
        assert!(parse(&["sponsor", "-all"], false).unwrap().is_empty());
        assert_eq!(
            parse(&["chapter"], true).unwrap_err(),
            "wrong CATS for --sponsorblock-mark: chapter"
        );
        assert_eq!(
            parse(&["sponsor,,intro"], false).unwrap_err(),
            "wrong CATS for --sponsorblock-mark: "
        );
    }
}

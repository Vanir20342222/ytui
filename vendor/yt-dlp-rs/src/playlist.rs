//! Bounded eager playlist item selection, following the pinned PlaylistEntries grammar.
use std::collections::HashSet;

use crate::{ExtractResult, Result};

/// Upstream playlist-file counts after item selection, before archive filtering:
/// the full count, selected count and largest selected index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaylistCounts {
    pub playlist_count: usize,
    /// The selected count; `None` with `--lazy-playlist` (upstream `N/A`).
    pub n_entries: Option<usize>,
    pub last_playlist_index: usize,
}

#[derive(Debug, Clone)]
pub struct PlaylistSelection {
    segments: Vec<Segment>,
    reverse: bool,
    random: bool,
    lazy: bool,
    selection_index: bool,
}

#[derive(Debug, Clone)]
struct Segment {
    start: Option<i64>,
    end: Endpoint,
    step: i64,
}

#[derive(Debug, Clone, Copy)]
enum Endpoint {
    Open,
    Infinite,
    Index(i64),
}

fn error(message: impl Into<String>) -> crate::Error {
    crate::Error::PlaylistSelection(message.into())
}

impl PlaylistSelection {
    /// Parse ASCII item specifications. Bounds are validated even when items override them.
    pub fn new(items: Option<&str>, start: i64, end: Option<i64>, reverse: bool) -> Result<Self> {
        if start <= 0 {
            return Err(error(format!(
                "playlist start \"{start}\" must be positive"
            )));
        }
        if let Some(end) = end
            && end != -1
            && end < start
        {
            return Err(error(format!(
                "playlist end \"{end}\" must be must be greater than or equal to playlist start \"{start}\""
            )));
        }
        let segments = if let Some(items) = items {
            if items.len() > 16 * 1024 {
                return Err(error("playlist item specification exceeds 16 KiB"));
            }
            let mut segments = Vec::new();
            for part in items.split(',') {
                if segments.len() == 1024 {
                    return Err(error("playlist item specification exceeds 1024 segments"));
                }
                segments.push(parse_segment(part)?);
            }
            segments
        } else {
            vec![Segment {
                start: Some(start),
                end: match end {
                    None | Some(-1) => Endpoint::Open,
                    Some(end) => Endpoint::Index(end),
                },
                step: 1,
            }]
        };
        Ok(Self {
            segments,
            reverse,
            random: false,
            lazy: false,
            selection_index: false,
        })
    }

    /// `--compat-options playlist-index`: an entry's `playlist_index` is the
    /// index selected at its position before reversing or shuffling.
    pub fn selection_index(mut self, enabled: bool) -> Self {
        self.selection_index = enabled;
        self
    }

    /// Upstream `--playlist-random`: shuffle the selected entries.
    pub fn random(mut self, random: bool) -> Self {
        self.random = random;
        self
    }

    /// Upstream `--lazy-playlist`: entries keep their order and the selected
    /// count is unknown (`N/A`).
    pub fn lazy(mut self, lazy: bool) -> Self {
        self.lazy = lazy;
        self
    }

    /// Return zero-based positions, removing duplicates before optional reversal.
    pub fn indices(&self, len: usize) -> Vec<usize> {
        let mut indices = Vec::new();
        if len == 0 {
            return indices;
        }
        let mut seen = HashSet::new();
        let length = len as i128;
        let translate = |index: i64| {
            if index >= 0 {
                i128::from(index) - 1
            } else {
                length + i128::from(index)
            }
        };
        for segment in &self.segments {
            let step = i128::from(segment.step);
            let positive = step > 0;
            let mut position = segment
                .start
                .map_or(if positive { 0 } else { length - 1 }, translate);
            let end = match segment.end {
                Endpoint::Index(index) => translate(index) + if positive { 1 } else { -1 },
                Endpoint::Open => {
                    if positive {
                        length
                    } else {
                        -1
                    }
                }
                Endpoint::Infinite if positive => length,
                Endpoint::Infinite => continue,
            };
            // Skip unavailable prefixes in constant time, retaining the requested phase.
            if positive && position < 0 {
                position += (-position + step - 1) / step * step;
            } else if !positive && position >= length {
                let stride = -step;
                position -= (position - (length - 1) + stride - 1) / stride * stride;
            }
            let end = if positive {
                end.min(length)
            } else {
                end.max(-1)
            };
            while position >= 0
                && position < length
                && if positive {
                    position < end
                } else {
                    position > end
                }
            {
                let index = position as usize;
                if seen.insert(index) {
                    indices.push(index);
                }
                position += step;
            }
            if seen.len() == len {
                break;
            }
        }
        if self.lazy {
            // Upstream ignores reverse/random for lazy playlists.
        } else if self.reverse {
            indices.reverse();
        } else if self.random {
            shuffle(&mut indices);
        }
        indices
    }

    /// Reorder playlist entries and assign positions before any archive filtering.
    /// Returns the playlist-level counts used for playlist file names.
    pub fn apply(&self, result: &mut ExtractResult) -> Option<PlaylistCounts> {
        let ExtractResult::Playlist(playlist) = result else {
            return None;
        };
        let count = playlist.entries.len();
        // Upstream's `_playlist_infodict` extras each entry gets by default.
        let title = Some(playlist.title.clone()).filter(|title| !title.is_empty());
        let field = |key: &str| {
            playlist
                .extra
                .get(key)
                .cloned()
                .unwrap_or(serde_json::Value::Null)
        };
        let defaults: [(&str, serde_json::Value); 10] = [
            (
                "playlist",
                title.clone().unwrap_or_else(|| playlist.id.clone()).into(),
            ),
            ("playlist_id", playlist.id.clone().into()),
            ("playlist_title", title.into()),
            ("playlist_uploader", field("uploader")),
            ("playlist_uploader_id", field("uploader_id")),
            ("playlist_channel", field("channel")),
            ("playlist_channel_id", field("channel_id")),
            (
                "playlist_webpage_url",
                playlist.webpage_url.to_string().into(),
            ),
            (
                "webpage_url_basename",
                crate::info::url_basename(&playlist.webpage_url).into(),
            ),
            (
                "webpage_url_domain",
                crate::info::get_domain(&playlist.webpage_url).into(),
            ),
        ];
        let indices = self.indices(count);
        let requested = if self.selection_index && !self.lazy {
            Self {
                reverse: false,
                random: false,
                ..self.clone()
            }
            .indices(count)
        } else {
            indices.clone()
        };
        let selected_count = indices.len();
        let last_index = indices.iter().max().map(|index| index + 1).unwrap_or(0);
        let mut entries: Vec<_> = std::mem::take(&mut playlist.entries)
            .into_iter()
            .map(Some)
            .collect();
        playlist.entries = indices
            .into_iter()
            .enumerate()
            .map(|(order, index)| {
                let mut entry = entries[index]
                    .take()
                    .expect("selection positions are unique");
                for (key, value) in defaults.clone() {
                    crate::info::set_default(&mut entry.extra, key, value);
                }
                entry.last_playlist_index = Some(last_index);
                entry.playlist_index = Some(requested[order] + 1);
                entry.playlist_autonumber = Some(order + 1);
                entry.playlist_count = Some(count);
                entry.n_entries = (!self.lazy).then_some(selected_count);
                entry
            })
            .collect();
        Some(PlaylistCounts {
            playlist_count: count,
            n_entries: (!self.lazy).then_some(selected_count),
            last_playlist_index: last_index,
        })
    }
}

/// `random.shuffle`: a Fisher-Yates shuffle seeded from the clock.
fn shuffle(items: &mut [usize]) {
    let mut state = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0x9e37_79b9_7f4a_7c15, |time| time.as_nanos() as u64)
        ^ u64::from(std::process::id()).rotate_left(32)
        | 1;
    for index in (1..items.len()).rev() {
        // xorshift64*
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        let value = state.wrapping_mul(0x2545_f491_4f6c_dd1d);
        items.swap(index, (value % (index as u64 + 1)) as usize);
    }
}

fn integer_prefix(input: &str) -> Result<(Option<i64>, &str)> {
    let bytes = input.as_bytes();
    let offset = usize::from(matches!(bytes.first(), Some(b'+' | b'-')));
    let digits = bytes
        .iter()
        .skip(offset)
        .take_while(|c| c.is_ascii_digit())
        .count();
    if digits == 0 {
        return Ok((None, input));
    }
    let end = offset + digits;
    let number = input[..end]
        .parse()
        .map_err(|_| error("playlist integer exceeds signed 64-bit range"))?;
    Ok((Some(number), &input[end..]))
}

/// Upstream `parse_playlist_items` message for a malformed segment.
fn invalid_segment(input: &str) -> crate::Error {
    error(format!(
        "{} is not a valid specification",
        crate::batch::py_repr(input)
    ))
}

fn parse_segment(input: &str) -> Result<Segment> {
    if input.is_empty() {
        return Err(error("There is two or more consecutive commas"));
    }
    let (start, remaining) = integer_prefix(input)?;
    if remaining.is_empty() {
        return start
            .map(|index| Segment {
                start: Some(index),
                end: Endpoint::Index(index),
                step: 1,
            })
            .ok_or_else(|| invalid_segment(input));
    }
    let Some(remaining) = remaining.strip_prefix([':', '-']) else {
        return Err(invalid_segment(input));
    };
    let (end, remaining) = if let Some(rest) = remaining.strip_prefix("infinite") {
        // Pinned float_or_none('infinite') is None, unlike float_or_none('inf').
        (Endpoint::Open, rest)
    } else if let Some(rest) = remaining.strip_prefix("inf") {
        (Endpoint::Infinite, rest)
    } else {
        let (end, rest) = integer_prefix(remaining)?;
        (end.map_or(Endpoint::Open, Endpoint::Index), rest)
    };
    let step = if remaining.is_empty() {
        1
    } else {
        let Some(rest) = remaining.strip_prefix(':') else {
            return Err(invalid_segment(input));
        };
        let (step, rest) = integer_prefix(rest)?;
        if !rest.is_empty() || step.is_none() {
            return Err(invalid_segment(input));
        }
        step.unwrap()
    };
    if step == 0 {
        return Err(error(format!(
            "Step in {} cannot be zero",
            crate::batch::py_repr(input)
        )));
    }
    Ok(Segment { start, end, step })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PlaylistInfo, VideoInfo};
    use serde_json::json;

    fn select(items: &str, len: usize) -> Vec<usize> {
        PlaylistSelection::new(Some(items), 1, None, false)
            .unwrap()
            .indices(len)
    }

    #[test]
    fn pinned_signed_inclusive_and_legacy_examples() {
        for (items, expected) in [
            ("2-4,3-4,3", vec![1, 2, 3]),
            ("4,2,4", vec![3, 1]),
            ("0", vec![]),
            ("::-1", (0..10).rev().collect()),
            (":-6", (0..5).collect()),
            ("-1:6:-2", vec![9, 7, 5]),
            ("9:-6:-2", vec![8, 6, 4]),
            ("0-2:2", vec![1]),
            ("0--2:2", vec![1, 3, 5, 7]),
            ("-15::2", vec![1, 3, 5, 7, 9]),
            ("-15::15", vec![]),
            (":inf:-1", vec![]),
            (":infinite:-1", (0..10).rev().collect()),
            ("-", (0..10).collect()),
            ("+2-+4", vec![1, 2, 3]),
        ] {
            assert_eq!(select(items, 10), expected, "{items}");
        }
        assert!(select("::-1", 0).is_empty());
    }

    #[test]
    fn reverse_follows_deduplication_and_legacy_end_minus_one_is_open() {
        let selection = PlaylistSelection::new(Some("3,1,3,2"), 1, None, true).unwrap();
        assert_eq!(selection.indices(5), vec![1, 0, 2]);
        assert_eq!(
            PlaylistSelection::new(None, 3, Some(-1), false)
                .unwrap()
                .indices(5),
            vec![2, 3, 4]
        );
        assert_eq!(select("3:-1", 5), vec![2, 3, 4]);
        assert_eq!(
            PlaylistSelection::new(None, 2, Some(3), false)
                .unwrap()
                .indices(5),
            vec![1, 2]
        );
    }

    #[test]
    fn invalid_syntax_and_bounds_fail_even_when_items_override() {
        for items in [
            "",
            "1,",
            ",1",
            "1,,2",
            " ",
            "1:2:0",
            "::",
            "1:2:",
            "1:2:3:4",
            "1.5",
            "١",
            "1:Infinity",
            "1:+inf",
            "9223372036854775808",
            "1:2:-9223372036854775809",
        ] {
            assert!(
                PlaylistSelection::new(Some(items), 1, None, false).is_err(),
                "{items:?}"
            );
        }
        for (start, end) in [
            (0, None),
            (-1, None),
            (3, Some(2)),
            (1, Some(0)),
            (1, Some(-2)),
        ] {
            assert!(PlaylistSelection::new(Some("1"), start, end, false).is_err());
        }
        assert!(PlaylistSelection::new(Some(&"1".repeat(16 * 1024 + 1)), 1, None, false).is_err());
        let max_segments = vec!["1"; 1024].join(",");
        assert_eq!(select(&max_segments, 1), vec![0]);
        assert!(PlaylistSelection::new(Some(&(max_segments + ",1")), 1, None, false).is_err());
    }

    #[test]
    fn huge_out_of_range_prefixes_keep_phase_without_iteration() {
        assert_eq!(
            select("-9223372036854775808::9223372036854775807", 10),
            vec![9]
        );
        assert_eq!(
            select("9223372036854775807::-9223372036854775808", 10),
            Vec::<usize>::new()
        );
        assert_eq!(
            select("9223372036854775807::-9223372036854775806", 10),
            vec![0]
        );
        assert_eq!(
            select("-9223372036854775808:9223372036854775807:2", 10),
            vec![0, 2, 4, 6, 8]
        );
    }

    #[test]
    fn clipped_iteration_matches_naive_finite_ranges() {
        for len in 0..8 {
            for start in -12..=12 {
                for end in -12..=12 {
                    for step in [-5, -2, -1, 1, 2, 5] {
                        let translate = |n: i64| if n >= 0 { n - 1 } else { len as i64 + n };
                        let mut current = translate(start);
                        let last = translate(end);
                        let mut expected = Vec::new();
                        while if step > 0 {
                            current <= last
                        } else {
                            current >= last
                        } {
                            if current >= 0 && current < len as i64 {
                                expected.push(current as usize);
                            }
                            current += step;
                        }
                        let expression = format!("{start}:{end}:{step}");
                        assert_eq!(select(&expression, len), expected, "{expression} len={len}");
                    }
                }
            }
        }
    }

    #[test]
    fn saved_upstream_playlist_selection_cases() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/playlist/cases.json")).unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let selection = PlaylistSelection::new(
                case["items"].as_str(),
                case["start"].as_i64().unwrap(),
                case["end"].as_i64(),
                case["reverse"].as_bool().unwrap(),
            )
            .unwrap_or_else(|error| panic!("{}: {error}", case["name"]));
            let expected: Vec<usize> =
                serde_json::from_value(case["expected_indices"].clone()).unwrap();
            assert_eq!(
                selection.indices(case["length"].as_u64().unwrap() as usize),
                expected,
                "{}",
                case["name"]
            );
        }
        for case in fixture["invalid_cases"].as_array().unwrap() {
            assert!(
                PlaylistSelection::new(
                    case["items"].as_str(),
                    case["start"].as_i64().unwrap(),
                    case["end"].as_i64(),
                    case["reverse"].as_bool().unwrap(),
                )
                .is_err(),
                "{}",
                case["name"]
            );
        }
    }

    fn video(id: &str) -> VideoInfo {
        serde_json::from_value(json!({"id":id,"title":id,"ext":"mp4","url":"https://example.com/media.mp4","webpage_url":"https://example.com/watch","extractor":"test","extractor_key":"Test","direct":true})).unwrap()
    }

    #[test]
    fn apply_records_original_and_selected_positions_and_leaves_video_unchanged() {
        let mut result = ExtractResult::Playlist(PlaylistInfo {
            id: "p".into(),
            title: "playlist".into(),
            webpage_url: "https://example.com/".parse().unwrap(),
            extractor: "test".into(),
            extractor_key: "Test".into(),
            entries: (1..=4).map(|i| video(&i.to_string())).collect(),
            extra: Default::default(),
        });
        PlaylistSelection::new(Some("4,2,4"), 1, None, true)
            .unwrap()
            .apply(&mut result);
        let ExtractResult::Playlist(playlist) = result else {
            panic!()
        };
        assert_eq!(
            playlist
                .entries
                .iter()
                .map(|v| v.id.as_str())
                .collect::<Vec<_>>(),
            vec!["2", "4"]
        );
        for (i, entry) in playlist.entries.iter().enumerate() {
            assert_eq!(entry.last_playlist_index, Some(4));
            assert_eq!(entry.playlist_index, Some([2, 4][i]));
            assert!(
                serde_json::to_value(entry)
                    .unwrap()
                    .get("__last_playlist_index")
                    .is_none()
            );
            assert_eq!(entry.playlist_autonumber, Some(i + 1));
            assert_eq!(entry.playlist_count, Some(4));
            assert_eq!(entry.n_entries, Some(2));
        }
        let mut single = ExtractResult::Video(video("single"));
        let before = serde_json::to_value(&single).unwrap();
        PlaylistSelection::new(Some("0"), 1, None, false)
            .unwrap()
            .apply(&mut single);
        assert_eq!(serde_json::to_value(single).unwrap(), before);
    }
}

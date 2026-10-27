//! Upstream `yt_dlp/webvtt.py` and HlsFD's WebVTT fragment packing: enough
//! of WebVTT to join segmented subtitles into one file, moving cues by each
//! segment's `X-TIMESTAMP-MAP` and merging cues repeated across segments.
use crate::{Error, Result};

/// `ParseError`: `Parse error at position N (near '…')`.
fn parse_error(data: &str, position: usize) -> Error {
    let near: String = data[position..].chars().take(100).collect();
    Error::Subtitle(format!(
        "Parse error at position {position} (near {})",
        crate::batch::py_repr(&near)
    ))
}

/// A newline (`\r\n`, `\r` or `\n`) at `position`: its length.
fn newline(data: &str, position: usize) -> Option<usize> {
    let rest = &data.as_bytes()[position..];
    match rest {
        [b'\r', b'\n', ..] => Some(2),
        [b'\r' | b'\n', ..] => Some(1),
        _ => None,
    }
}

/// `_REGEX_NL`: a newline, or the end (`$` also before a final newline).
fn line_end(data: &str, position: usize) -> Option<usize> {
    newline(data, position).or_else(|| {
        let rest = &data[position..];
        (rest.is_empty()).then_some(0)
    })
}

/// The rest of the line at `position` (without its newline).
fn line_at(data: &str, position: usize) -> &str {
    let rest = &data[position..];
    &rest[..rest.find(['\r', '\n']).unwrap_or(rest.len())]
}

/// `((?:(?!-->)[^\r\n])+(?:\r\n|[\r\n]))*`: non-empty lines without `-->`,
/// each with a newline. Returns the length consumed.
fn plain_lines(data: &str, mut position: usize) -> usize {
    let start = position;
    loop {
        let line = line_at(data, position);
        if line.is_empty() || line.contains("-->") {
            break;
        }
        let Some(length) = newline(data, position + line.len()) else {
            break;
        };
        position += line.len() + length;
    }
    position - start
}

/// `_REGEX_TS` at `position`: (length, 90 kHz ticks).
fn timestamp(data: &str, position: usize) -> Option<(usize, i64)> {
    static TS: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"^(?:([0-9]+):)?([0-9]{2}):([0-9]{2})\.([0-9]{3})?").unwrap()
    });
    let captures = TS.captures(&data[position..])?;
    let part = |index: usize| {
        captures.get(index).map_or(0, |part| {
            part.as_str().parse::<i64>().unwrap_or(i64::MAX / 4)
        })
    };
    let milliseconds = part(1)
        .saturating_mul(3_600_000)
        .saturating_add(part(2) * 60_000)
        .saturating_add(part(3) * 1000)
        .saturating_add(part(4));
    Some((captures[0].len(), milliseconds.saturating_mul(90)))
}

/// `_format_ts`: 90 kHz ticks as `HH:MM:SS.mmm` (rounded to milliseconds).
fn format_ts(ticks: i64) -> String {
    let milliseconds = (ticks + 45).div_euclid(90);
    let (hours, rest) = (milliseconds / 3_600_000, milliseconds % 3_600_000);
    format!(
        "{hours:02}:{:02}:{:02}.{:03}",
        rest / 60_000,
        rest % 60_000 / 1000,
        rest % 1000
    )
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Cue {
    id: Option<String>,
    start: i64,
    end: i64,
    settings: Option<String>,
    text: String,
}

impl Cue {
    fn write_into(&self, output: &mut String) {
        if let Some(id) = &self.id {
            output.push_str(id);
            output.push('\n');
        }
        output.push_str(&format_ts(self.start));
        output.push_str(" --> ");
        output.push_str(&format_ts(self.end));
        if let Some(settings) = &self.settings {
            output.push(' ');
            output.push_str(settings);
        }
        output.push('\n');
        output.push_str(&self.text);
        output.push('\n');
    }

    /// `CueBlock.hinges`: `other` continues this cue.
    fn hinges(&self, other: &Self) -> bool {
        self.text == other.text
            && self.settings == other.settings
            && self.start <= self.end
            && self.end == other.start
            && other.start <= other.end
    }
}

#[derive(Debug, Clone)]
enum Block {
    Magic {
        extra: Option<String>,
        mpegts: Option<i64>,
        local: Option<i64>,
        meta: String,
    },
    /// A STYLE or REGION block (upstream `HeaderBlock` subclasses).
    Header {
        kind: &'static str,
        raw: String,
    },
    Comment(String),
    Cue(Cue),
}

/// `Magic.parse`.
fn magic(data: &str, position: &mut usize) -> Result<Block> {
    let mut at = *position;
    if data[at..].starts_with('\u{feff}') {
        at += '\u{feff}'.len_utf8();
    }
    if !data[at..].starts_with("WEBVTT") {
        return Err(parse_error(data, *position));
    }
    at += "WEBVTT".len();
    let rest = line_at(data, at);
    let extra = if rest.starts_with([' ', '\t']) {
        at += rest.len();
        Some(rest.to_owned())
    } else if rest.is_empty() {
        None
    } else {
        return Err(parse_error(data, *position));
    };
    at += newline(data, at).ok_or_else(|| parse_error(data, *position))?;
    let (mut mpegts, mut local, mut meta) = (None, None, String::new());
    loop {
        if let Some(length) = line_end(data, at) {
            at += length;
            break;
        }
        if let Some(map) = data[at..].strip_prefix("X-TIMESTAMP-MAP=") {
            let _ = map;
            at += "X-TIMESTAMP-MAP=".len();
            loop {
                if data[at..].starts_with("LOCAL:") {
                    at += "LOCAL:".len();
                    let (length, ticks) =
                        timestamp(data, at).ok_or_else(|| parse_error(data, at))?;
                    at += length;
                    local = Some(ticks);
                } else if let Some(digits) = data[at..]
                    .strip_prefix("MPEGTS:")
                    .map(|rest| rest.bytes().take_while(u8::is_ascii_digit).count())
                {
                    if digits == 0 {
                        return Err(parse_error(data, at));
                    }
                    let value = &data[at + "MPEGTS:".len()..at + "MPEGTS:".len() + digits];
                    mpegts = Some(value.parse().map_err(|_| parse_error(data, at))?);
                    at += "MPEGTS:".len() + digits;
                } else {
                    return Err(parse_error(data, at));
                }
                // `[ \t]*,[ \t]*`
                let separator = data[at..].trim_start_matches([' ', '\t']);
                if let Some(after) = separator.strip_prefix(',') {
                    at = data.len() - after.trim_start_matches([' ', '\t']).len();
                    continue;
                }
                if let Some(length) = line_end(data, at) {
                    at += length;
                    break;
                }
                return Err(parse_error(data, at));
            }
            continue;
        }
        // `_REGEX_META`: `NAME:VALUE` without `-->`.
        let line = line_at(data, at);
        let colon = line[1.min(line.len())..].find(':').map(|index| index + 1);
        if !line.contains("-->")
            && colon.is_some_and(|colon| colon + 1 < line.len())
            && let Some(length) = newline(data, at + line.len())
        {
            meta.push_str(&data[at..at + line.len() + length]);
            at += line.len() + length;
            continue;
        }
        return Err(parse_error(data, at));
    }
    *position = at;
    Ok(Block::Magic {
        extra,
        mpegts,
        local,
        meta,
    })
}

/// `StyleBlock`/`RegionBlock`/`CommentBlock` regexes.
fn raw_block(data: &str, position: usize, keyword: &str) -> Option<usize> {
    let mut at = position;
    if !data[at..].starts_with(keyword) {
        return None;
    }
    at += keyword.len();
    match keyword {
        "NOTE" => {
            // `(?:\r\n|[\ \t\r\n])`
            at += match newline(data, at) {
                Some(length) => length,
                None if data[at..].starts_with([' ', '\t']) => 1,
                None => return None,
            };
        }
        "STYLE" => {
            at += data[at..].len() - data[at..].trim_start_matches([' ', '\t']).len();
            at += newline(data, at)?;
        }
        _ => {
            at += data[at..].len() - data[at..].trim_start_matches([' ', '\t']).len();
        }
    }
    at += plain_lines(data, at);
    at += newline(data, at)?;
    Some(at - position)
}

/// `CueBlock.parse`.
fn cue(data: &str, position: usize) -> Option<(usize, Cue)> {
    let mut at = position;
    let mut id = None;
    let line = line_at(data, at);
    if !line.is_empty()
        && !line.contains("-->")
        && let Some(length) = newline(data, at + line.len())
    {
        id = Some(line.to_owned());
        at += line.len() + length;
    }
    let (length, start) = timestamp(data, at)?;
    at += length;
    // `[ \t]+-->[ \t]+`
    let spaces = |at: usize| data[at..].len() - data[at..].trim_start_matches([' ', '\t']).len();
    let before = spaces(at);
    if before == 0 || !data[at + before..].starts_with("-->") {
        return None;
    }
    at += before + 3;
    let after = spaces(at);
    if after == 0 {
        return None;
    }
    at += after;
    let (length, end) = timestamp(data, at)?;
    at += length;
    // `[ \t]+((?:(?!-->)[^\r\n])+)`: the rest of the line up to any `-->`.
    let mut settings = None;
    let lead = spaces(at);
    if lead > 0 {
        let rest = line_at(data, at + lead);
        let value = &rest[..rest.find("-->").unwrap_or(rest.len())];
        if !value.is_empty() {
            settings = Some(value.to_owned());
            at += lead + value.len();
        }
    }
    at += spaces(at);
    at += line_end(data, at)?;
    let mut text = String::new();
    // `[^\r\n]+(?:\r\n|[\r\n])?`
    loop {
        let line = line_at(data, at);
        if line.is_empty() {
            break;
        }
        let length = line.len() + newline(data, at + line.len()).unwrap_or(0);
        text.push_str(&data[at..at + length]);
        at += length;
    }
    Some((
        at - position,
        Cue {
            id,
            start,
            end,
            settings,
            text,
        },
    ))
}

/// `(?:\r\n|[\r\n])+`.
fn blanks(data: &str, mut at: usize) -> usize {
    let start = at;
    while let Some(length) = newline(data, at) {
        at += length;
    }
    at - start
}

/// `parse_fragment`.
fn parse_fragment(data: &str) -> Result<Vec<Block>> {
    let mut position = 0;
    let mut blocks = vec![magic(data, &mut position)?];
    while position < data.len() {
        let skipped = blanks(data, position);
        if skipped > 0 {
            position += skipped;
            continue;
        }
        if let Some(length) = raw_block(data, position, "REGION") {
            blocks.push(Block::Header {
                kind: "RegionBlock",
                raw: data[position..position + length].to_owned(),
            });
            position += length;
            continue;
        }
        if let Some(length) = raw_block(data, position, "STYLE") {
            blocks.push(Block::Header {
                kind: "StyleBlock",
                raw: data[position..position + length].to_owned(),
            });
            position += length;
            continue;
        }
        if let Some(length) = raw_block(data, position, "NOTE") {
            blocks.push(Block::Comment(data[position..position + length].to_owned()));
            position += length;
            continue;
        }
        break;
    }
    while position < data.len() {
        let skipped = blanks(data, position);
        if skipped > 0 {
            position += skipped;
            continue;
        }
        if let Some(length) = raw_block(data, position, "NOTE") {
            blocks.push(Block::Comment(data[position..position + length].to_owned()));
            position += length;
            continue;
        }
        if let Some((length, cue)) = cue(data, position) {
            blocks.push(Block::Cue(cue));
            position += length;
            continue;
        }
        return Err(parse_error(data, position));
    }
    Ok(blocks)
}

/// HlsFD's `pack_fragment`/`fin_fragments` state (`extra_state`).
#[derive(Debug, Default)]
pub(crate) struct Packer {
    mpegts: i64,
    local: i64,
    mpegts_last: Option<i64>,
    mpegts_adjust: i64,
    dedup_window: Vec<Cue>,
}

impl Packer {
    /// One fragment (1-based `index`) as its share of the joined file;
    /// `warn` reports header blocks discarded mid-stream.
    pub(crate) fn pack(
        &mut self,
        content: &[u8],
        index: usize,
        warn: &mut dyn FnMut(&str),
    ) -> Result<Vec<u8>> {
        let data = String::from_utf8(content.to_vec())
            .map_err(|_| Error::Subtitle("WebVTT fragment is not UTF-8".into()))?;
        let mut output = String::new();
        let (mut adjust, mut overflow, mut mpegts_last) = (0, false, None);
        for block in parse_fragment(&data)? {
            match block {
                Block::Cue(mut cue) => {
                    self.mpegts_last = mpegts_last;
                    if overflow {
                        self.mpegts_adjust += 1;
                        overflow = false;
                    }
                    cue.start += adjust;
                    cue.end += adjust;
                    let mut ready = Vec::new();
                    let mut is_new = true;
                    let mut i = 0;
                    while i < self.dedup_window.len() {
                        let window = self.dedup_window[i].clone();
                        i += 1;
                        if window.hinges(&cue) {
                            self.dedup_window[i - 1].end = cue.end;
                            is_new = false;
                            continue;
                        }
                        if window == cue {
                            is_new = false;
                            continue;
                        }
                        if window.end > cue.start {
                            continue;
                        }
                        ready.push(window);
                        i -= 1;
                        self.dedup_window.remove(i);
                    }
                    if is_new {
                        self.dedup_window.push(cue);
                    }
                    for cue in ready {
                        cue.write_into(&mut output);
                    }
                }
                Block::Magic {
                    extra,
                    mpegts,
                    local,
                    meta,
                } => {
                    // MPEG PES timestamps wrap at 2^33.
                    let mut mpegts = mpegts.unwrap_or(0) + (self.mpegts_adjust << 33);
                    if mpegts < self.mpegts_last.unwrap_or(0) {
                        overflow = true;
                        mpegts += 1 << 33;
                    }
                    mpegts_last = Some(mpegts);
                    if index == 1 {
                        self.mpegts = mpegts;
                        self.local = local.unwrap_or(0);
                        output.push_str("WEBVTT");
                        if let Some(extra) = &extra {
                            output.push_str(extra);
                        }
                        output.push('\n');
                        if local.is_some_and(|local| local != 0) || mpegts != 0 {
                            output.push_str("X-TIMESTAMP-MAP=LOCAL:");
                            output.push_str(&format_ts(local.unwrap_or(0)));
                            output.push_str(",MPEGTS:");
                            output.push_str(&mpegts.to_string());
                            output.push('\n');
                        }
                        output.push_str(&meta);
                        output.push('\n');
                    } else if let Some(local) = local {
                        adjust = (mpegts - self.mpegts) - (local - self.local);
                    }
                }
                Block::Header { kind, raw } => {
                    if index != 1 {
                        // Upstream appends its `bug_reports_message` (yt-dlp's
                        // tracker and `yt-dlp -U`), which does not apply here.
                        warn(&format!(
                            "Discarding a {kind} block found in the middle of the stream; if the subtitles display incorrectly, please report this issue"
                        ));
                        continue;
                    }
                    output.push_str(&raw);
                }
                Block::Comment(raw) => output.push_str(&raw),
            }
        }
        Ok(output.into_bytes())
    }

    /// `fin_fragments`: the cues still in the duplicate window.
    pub(crate) fn finish(&mut self) -> Vec<u8> {
        let mut output = String::new();
        for cue in self.dedup_window.drain(..) {
            cue.write_into(&mut output);
        }
        output.into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_join_like_hlsfd() {
        let first = b"WEBVTT\nX-TIMESTAMP-MAP=MPEGTS:900000,LOCAL:00:00:00.000\n\n1\n00:00:00.500 --> 00:00:01.500\nHello\n\n00:00:01.800 --> 00:00:02.000\nSpans\n";
        let second = b"WEBVTT\nX-TIMESTAMP-MAP=MPEGTS:1080000,LOCAL:00:00:00.000\n\n00:00:00.000 --> 00:00:00.400\nSpans\n\nNOTE comment\n\n00:00:00.500 --> 00:00:01.000 align:start\nWorld\n";
        let mut packer = Packer::default();
        let mut warn = |_: &str| panic!("no warnings");
        let mut output = packer.pack(first, 1, &mut warn).unwrap();
        output.extend(packer.pack(second, 2, &mut warn).unwrap());
        output.extend(packer.finish());
        // Captured from the pinned upstream checkout over the same segments.
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "WEBVTT\nX-TIMESTAMP-MAP=LOCAL:00:00:00.000,MPEGTS:900000\n\n1\n00:00:00.500 --> 00:00:01.500\nHello\n\n\
             NOTE comment\n\n00:00:01.800 --> 00:00:02.400\nSpans\n\n00:00:02.500 --> 00:00:03.000 align:start\nWorld\n\n"
        );
    }

    #[test]
    fn parse_errors_and_timestamps_follow_upstream() {
        assert_eq!(format_ts(90 * 3_723_456), "01:02:03.456");
        assert_eq!(timestamp("01:02.5", 0), Some((6, 90 * 62_000)));
        let error = parse_fragment("WEBVTT\n\nnot a cue\n")
            .unwrap_err()
            .to_string();
        assert!(
            error.ends_with("Parse error at position 8 (near 'not a cue\\n')"),
            "{error}"
        );
        assert!(parse_fragment("SRT").is_err());
    }
}

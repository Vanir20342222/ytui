//! Media container detection from file contents. HLS formats are labeled
//! `mp4` like upstream even when their assembled segments are MPEG-TS or
//! raw AAC, so FFmpeg inputs choose demuxers from the bytes.
use std::{io::Read, path::Path};

/// The container of `bytes` as one of this port's extension names.
pub fn sniff_bytes(bytes: &[u8]) -> Option<&'static str> {
    if bytes.first() == Some(&0x47) && bytes.get(188).is_none_or(|byte| *byte == 0x47) {
        return Some("ts");
    }
    if bytes.len() >= 8
        && matches!(
            &bytes[4..8],
            b"ftyp" | b"moov" | b"moof" | b"styp" | b"sidx" | b"free" | b"mdat"
        )
    {
        return Some("mp4");
    }
    if bytes.starts_with(b"\x1a\x45\xdf\xa3") {
        return Some("webm");
    }
    // Packed audio: bare ADTS or MPEG audio frames, or an ID3 tag before
    // them (HLS packed audio is AAC unless the frames say otherwise).
    if bytes.starts_with(b"ID3") {
        let header = id3_end(bytes).and_then(|frame| bytes.get(frame..frame + 2));
        return Some(header.and_then(audio_frame).unwrap_or("aac"));
    }
    audio_frame(bytes)
}

/// The offset after a leading ID3v2 tag (and its footer).
fn id3_end(bytes: &[u8]) -> Option<usize> {
    if !bytes.starts_with(b"ID3") || bytes.len() < 10 {
        return None;
    }
    let size = bytes[6..10]
        .iter()
        .fold(0usize, |size, byte| (size << 7) | usize::from(byte & 0x7f));
    let footer = if bytes[5] & 0x10 != 0 { 10 } else { 0 };
    Some(10 + size + footer)
}

/// ADTS (`aac`) or MPEG audio layer III (`mp3`) frame sync.
fn audio_frame(header: &[u8]) -> Option<&'static str> {
    match header {
        [0xff, second, ..] if second & 0xf6 == 0xf0 => Some("aac"),
        [0xff, second, ..] if second & 0xe0 == 0xe0 && (second >> 1) & 3 == 1 => Some("mp3"),
        _ => None,
    }
}

/// The container of the file at `path`, read from its first bytes.
pub fn sniff(path: &Path) -> Option<&'static str> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut head = Vec::with_capacity(256);
    (&mut file).take(256).read_to_end(&mut head).ok()?;
    // A large ID3 tag: read the frame header after it.
    if let Some(frame) = id3_end(&head).filter(|frame| frame + 2 > head.len()) {
        use std::io::{Seek, SeekFrom};
        let mut header = [0; 2];
        let found = file.seek(SeekFrom::Start(frame as u64)).is_ok()
            && file.read_exact(&mut header).is_ok();
        return Some(
            found
                .then(|| audio_frame(&header))
                .flatten()
                .unwrap_or("aac"),
        );
    }
    sniff_bytes(&head)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_containers_are_recognized() {
        let mut ts = vec![0x47; 1];
        ts.resize(188, 0);
        ts.push(0x47);
        assert_eq!(sniff_bytes(&ts), Some("ts"));
        assert_eq!(sniff_bytes(b"\0\0\0\x18ftypisom"), Some("mp4"));
        assert_eq!(sniff_bytes(b"\0\0\0\x18moof...."), Some("mp4"));
        assert_eq!(sniff_bytes(b"\x1a\x45\xdf\xa3...."), Some("webm"));
        assert_eq!(sniff_bytes(b"ID3\x04..."), Some("aac"));
        assert_eq!(sniff_bytes(&[0xff, 0xf1, 0x50]), Some("aac"));
        // ID3 (here 2 tag bytes) before MP3 or ADTS frames.
        assert_eq!(
            sniff_bytes(b"ID3\x04\0\0\0\0\0\x02ab\xff\xfb\x90"),
            Some("mp3")
        );
        assert_eq!(
            sniff_bytes(b"ID3\x04\0\0\0\0\0\x02ab\xff\xf1\x50"),
            Some("aac")
        );
        assert_eq!(sniff_bytes(&[0xff, 0xfb, 0x90]), Some("mp3"));
        assert_eq!(sniff_bytes(b"media bytes"), None);
    }
}

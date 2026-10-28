//! Upstream `IsmFD` (`yt_dlp/downloader/ism.py`): Smooth Streaming
//! fragments appended raw after a PIFF `ftyp`/`moov` header built from the
//! format's `_download_params` and the first fragment's `tfhd` track ID.
use crate::{Error, Result};
use serde_json::Value;

fn reported(message: impl Into<String>) -> Error {
    Error::Reported(message.into())
}

fn put_box(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut out = ((8 + payload.len()) as u32).to_be_bytes().to_vec();
    out.extend_from_slice(kind);
    out.extend_from_slice(payload);
    out
}

fn full_box(kind: &[u8; 4], version: u8, flags: u32, payload: &[u8]) -> Vec<u8> {
    let mut body = vec![version];
    body.extend_from_slice(&flags.to_be_bytes()[1..]);
    body.extend_from_slice(payload);
    put_box(kind, &body)
}

fn unity_matrix() -> Vec<u8> {
    let mut out = Vec::new();
    for _ in 0..2 {
        out.extend_from_slice(&0x10000i32.to_be_bytes());
        out.extend_from_slice(&[0; 12]);
    }
    out.extend_from_slice(&0x4000_0000i32.to_be_bytes());
    out
}

/// `extract_box_data(data, box_sequence)`.
pub(crate) fn extract_box_data<'a>(data: &'a [u8], sequence: &[&[u8; 4]]) -> Result<&'a [u8]> {
    let truncated = || reported("unpack requires a buffer of 4 bytes");
    let mut position = 0usize;
    loop {
        let size = data.get(position..position + 4).ok_or_else(truncated)?;
        let size = u32::from_be_bytes(size.try_into().expect("4 bytes")) as usize;
        let kind = data.get(position + 4..position + 8).unwrap_or_default();
        let start = position + 8;
        let end = (start + size.saturating_sub(8)).min(data.len());
        if kind == sequence[0] {
            let content = &data[start.min(data.len())..end];
            if sequence.len() == 1 {
                return Ok(content);
            }
            return extract_box_data(content, &sequence[1..]);
        }
        position = start + size.saturating_sub(8);
        if size < 8 && position <= start {
            return Err(truncated());
        }
    }
}

/// The first fragment's `moof/traf/tfhd` track ID.
pub(crate) fn track_id(fragment: &[u8]) -> Result<u32> {
    let tfhd = extract_box_data(fragment, &[b"moof", b"traf", b"tfhd"])?;
    let bytes = tfhd
        .get(4..8)
        .ok_or_else(|| reported("unpack requires a buffer of 4 bytes"))?;
    Ok(u32::from_be_bytes(bytes.try_into().expect("4 bytes")))
}

/// `write_piff_header(stream, params)` at `now` (`int(time.time())`).
pub(crate) fn piff_header(params: &Value, track_id: u32, now: u64) -> Result<Vec<u8>> {
    let missing = |key: &str| reported(format!("KeyError: '{key}'"));
    let int = |key: &str, default: i64| -> i64 {
        params.get(key).and_then(Value::as_i64).unwrap_or(default)
    };
    let text = |key: &str| params.get(key).and_then(Value::as_str);
    let fourcc = text("fourcc").ok_or_else(|| missing("fourcc"))?;
    let duration = params
        .get("duration")
        .and_then(Value::as_u64)
        .ok_or_else(|| missing("duration"))?;
    let timescale = int("timescale", 10_000_000) as u32;
    let language: Vec<u8> = text("language").unwrap_or("und").bytes().collect();
    let width = int("width", 0) as u16;
    let height = int("height", 0) as u16;
    let stream_type = text("stream_type").ok_or_else(|| missing("stream_type"))?;
    let assertion = || reported("AssertionError");
    let mut out = put_box(b"ftyp", b"isml\x00\x00\x00\x01piffiso2");

    let mut mvhd = Vec::new();
    mvhd.extend_from_slice(&now.to_be_bytes());
    mvhd.extend_from_slice(&now.to_be_bytes());
    mvhd.extend_from_slice(&timescale.to_be_bytes());
    mvhd.extend_from_slice(&duration.to_be_bytes());
    mvhd.extend_from_slice(&[0, 1, 0, 0]);
    mvhd.extend_from_slice(&[1, 0]);
    mvhd.extend_from_slice(&[0; 2 + 8]);
    mvhd.extend(unity_matrix());
    mvhd.extend_from_slice(&[0; 24]);
    mvhd.extend_from_slice(&0xffff_ffffu32.to_be_bytes());
    let mut moov = full_box(b"mvhd", 1, 0, &mvhd);

    let mut tkhd = Vec::new();
    tkhd.extend_from_slice(&now.to_be_bytes());
    tkhd.extend_from_slice(&now.to_be_bytes());
    tkhd.extend_from_slice(&track_id.to_be_bytes());
    tkhd.extend_from_slice(&[0; 4]);
    tkhd.extend_from_slice(&duration.to_be_bytes());
    tkhd.extend_from_slice(&[0; 8 + 2 + 2]);
    tkhd.extend_from_slice(&[u8::from(stream_type == "audio"), 0]);
    tkhd.extend_from_slice(&[0; 2]);
    tkhd.extend(unity_matrix());
    tkhd.extend_from_slice(&width.to_be_bytes());
    tkhd.extend_from_slice(&[0; 2]);
    tkhd.extend_from_slice(&height.to_be_bytes());
    tkhd.extend_from_slice(&[0; 2]);
    let mut trak = full_box(b"tkhd", 1, 0x7, &tkhd);

    let mut mdhd = Vec::new();
    mdhd.extend_from_slice(&now.to_be_bytes());
    mdhd.extend_from_slice(&now.to_be_bytes());
    mdhd.extend_from_slice(&timescale.to_be_bytes());
    mdhd.extend_from_slice(&duration.to_be_bytes());
    if language.len() < 3 {
        return Err(reported("IndexError: string index out of range"));
    }
    let code = |byte: u8| u16::from(byte).wrapping_sub(0x60);
    let packed = (code(language[0]) << 10) | (code(language[1]) << 5) | code(language[2]);
    mdhd.extend_from_slice(&packed.to_be_bytes());
    mdhd.extend_from_slice(&[0; 2]);
    let mut mdia = full_box(b"mdhd", 1, 0, &mdhd);

    let (handler, name, media_header): (&[u8; 4], &[u8], Vec<u8>) = match stream_type {
        "audio" => (b"soun", b"SoundHandler\0", full_box(b"smhd", 0, 0, &[0; 4])),
        "video" => (b"vide", b"VideoHandler\0", full_box(b"vmhd", 0, 1, &[0; 8])),
        "text" => (b"subt", b"SubtitleHandler\0", full_box(b"sthd", 0, 0, b"")),
        _ => return Err(assertion()),
    };
    let mut hdlr = vec![0; 4];
    hdlr.extend_from_slice(handler);
    hdlr.extend_from_slice(&[0; 12]);
    hdlr.extend_from_slice(name);
    mdia.extend(full_box(b"hdlr", 0, 0, &hdlr));

    let mut minf = media_header;
    let mut dref = 1u32.to_be_bytes().to_vec();
    dref.extend(full_box(b"url ", 0, 1, b""));
    minf.extend(put_box(b"dinf", &full_box(b"dref", 0, 0, &dref)));

    let mut entry = vec![0; 6];
    entry.extend_from_slice(&1u16.to_be_bytes());
    let sample_entry = match stream_type {
        "audio" => {
            entry.extend_from_slice(&[0; 8]);
            entry.extend_from_slice(&(int("channels", 2) as u16).to_be_bytes());
            entry.extend_from_slice(&(int("bits_per_sample", 16) as u16).to_be_bytes());
            entry.extend_from_slice(&[0; 4]);
            let rate = params
                .get("sampling_rate")
                .and_then(Value::as_i64)
                .ok_or_else(|| reported("required argument is not an integer"))?;
            entry.extend_from_slice(&(rate as u16).to_be_bytes());
            entry.extend_from_slice(&[0; 2]);
            match fourcc {
                "AACL" => put_box(b"mp4a", &entry),
                "EC-3" => put_box(b"ec-3", &entry),
                _ => return Err(assertion()),
            }
        }
        "video" => {
            entry.extend_from_slice(&[0; 4 + 12]);
            entry.extend_from_slice(&width.to_be_bytes());
            entry.extend_from_slice(&height.to_be_bytes());
            entry.extend_from_slice(&[0, 0x48, 0, 0, 0, 0x48, 0, 0]);
            entry.extend_from_slice(&[0; 4]);
            entry.extend_from_slice(&1u16.to_be_bytes());
            entry.extend_from_slice(&[0; 32]);
            entry.extend_from_slice(&0x18u16.to_be_bytes());
            entry.extend_from_slice(&(-1i16).to_be_bytes());
            let private = text("codec_private_data").ok_or_else(|| {
                reported("argument should be bytes, buffer or ASCII string, not 'NoneType'")
            })?;
            let private = hex(private)?;
            if !matches!(fourcc, "H264" | "AVC1") {
                return Err(assertion());
            }
            let parts = split(&private, &[0, 0, 0, 1]);
            let [_, sps, pps] = parts.as_slice() else {
                return Err(reported("not enough values to unpack"));
            };
            if sps.len() < 4 {
                return Err(reported("H.264 SPS too short"));
            }
            let mut avcc = vec![1];
            avcc.extend_from_slice(&sps[1..4]);
            avcc.push(0xfc | ((int("nal_unit_length_field", 4) - 1) as u8));
            avcc.push(1);
            avcc.extend_from_slice(&(sps.len() as u16).to_be_bytes());
            avcc.extend_from_slice(sps);
            avcc.push(1);
            avcc.extend_from_slice(&(pps.len() as u16).to_be_bytes());
            avcc.extend_from_slice(pps);
            entry.extend(put_box(b"avcC", &avcc));
            put_box(b"avc1", &entry)
        }
        _ => {
            if fourcc != "TTML" {
                return Err(assertion());
            }
            entry.extend_from_slice(b"http://www.w3.org/ns/ttml\0\0\0");
            put_box(b"stpp", &entry)
        }
    };
    let mut stsd = 1u32.to_be_bytes().to_vec();
    stsd.extend(sample_entry);
    let mut stbl = full_box(b"stsd", 0, 0, &stsd);
    for kind in [b"stts", b"stsc", b"stco"] {
        stbl.extend(full_box(kind, 0, 0, &[0; 4]));
    }
    minf.extend(put_box(b"stbl", &stbl));
    mdia.extend(put_box(b"minf", &minf));
    trak.extend(put_box(b"mdia", &mdia));
    moov.extend(put_box(b"trak", &trak));

    let mut mvex = full_box(b"mehd", 1, 0, &duration.to_be_bytes());
    let mut trex = track_id.to_be_bytes().to_vec();
    trex.extend_from_slice(&1u32.to_be_bytes());
    trex.extend_from_slice(&[0; 12]);
    mvex.extend(full_box(b"trex", 0, 0, &trex));
    moov.extend(put_box(b"mvex", &mvex));
    out.extend(put_box(b"moov", &moov));
    Ok(out)
}

/// `binascii.unhexlify`.
fn hex(text: &str) -> Result<Vec<u8>> {
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(2) {
        return Err(reported("Odd-length string"));
    }
    bytes
        .chunks(2)
        .map(|pair| {
            std::str::from_utf8(pair)
                .ok()
                .and_then(|pair| u8::from_str_radix(pair, 16).ok())
                .ok_or_else(|| reported("Non-hexadecimal digit found"))
        })
        .collect()
}

/// `bytes.split(separator)`.
fn split<'a>(data: &'a [u8], separator: &[u8]) -> Vec<&'a [u8]> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut index = 0;
    while index + separator.len() <= data.len() {
        if &data[index..index + separator.len()] == separator {
            parts.push(&data[start..index]);
            index += separator.len();
            start = index;
        } else {
            index += 1;
        }
    }
    parts.push(&data[start..]);
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn piff_headers_match_pinned_upstream() {
        // `write_piff_header` of pinned upstream with `time.time()` fixed.
        let fixture: Value =
            serde_json::from_str(include_str!("../tests/fixtures/ism_piff.json")).unwrap();
        for kind in ["video", "audio", "text"] {
            let params = &fixture["params"][kind];
            let track = params["track_id"].as_u64().unwrap() as u32;
            let header = piff_header(params, track, 1_700_000_000).unwrap();
            let hex: String = header.iter().map(|byte| format!("{byte:02x}")).collect();
            assert_eq!(hex, fixture["headers"][kind].as_str().unwrap(), "{kind}");
        }
    }

    #[test]
    fn track_ids_come_from_tfhd() {
        let tfhd = full_box(b"tfhd", 0, 0, &7u32.to_be_bytes());
        let fragment = put_box(
            b"moof",
            &[put_box(b"mfhd", &[0; 8]), put_box(b"traf", &tfhd)].concat(),
        );
        assert_eq!(track_id(&fragment).unwrap(), 7);
    }
}

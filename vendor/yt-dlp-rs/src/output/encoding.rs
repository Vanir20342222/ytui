//! `--encoding`: Python's `str.encode(ENCODING, 'ignore')` for the codecs
//! this port implements (characters a codec cannot represent are dropped).
//! `utf-16`/`utf-32` start every encoded call with a little-endian BOM, as
//! CPython does on little-endian hosts.

/// A supported Python codec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Ascii,
    Latin1,
    Cp1252,
    Utf16Le,
    Utf16Be,
    Utf32Le,
    Utf32Be,
    /// `utf-16`: a BOM, then little-endian units.
    Utf16,
    /// `utf-32`: a BOM, then little-endian code points.
    Utf32,
}

/// cp1252's 0x80-0x9F (`None`: undefined bytes Python cannot encode).
const CP1252_HIGH: [Option<char>; 32] = [
    Some('\u{20AC}'),
    None,
    Some('\u{201A}'),
    Some('\u{0192}'),
    Some('\u{201E}'),
    Some('\u{2026}'),
    Some('\u{2020}'),
    Some('\u{2021}'),
    Some('\u{02C6}'),
    Some('\u{2030}'),
    Some('\u{0160}'),
    Some('\u{2039}'),
    Some('\u{0152}'),
    None,
    Some('\u{017D}'),
    None,
    None,
    Some('\u{2018}'),
    Some('\u{2019}'),
    Some('\u{201C}'),
    Some('\u{201D}'),
    Some('\u{2022}'),
    Some('\u{2013}'),
    Some('\u{2014}'),
    Some('\u{02DC}'),
    Some('\u{2122}'),
    Some('\u{0161}'),
    Some('\u{203A}'),
    Some('\u{0153}'),
    None,
    Some('\u{017E}'),
    Some('\u{0178}'),
];

impl Encoding {
    /// Python `codecs.lookup` for the implemented codecs and their aliases
    /// (case-insensitive, `-` and spaces as `_`).
    pub fn lookup(name: &str) -> Option<Self> {
        let name = name.trim().to_lowercase().replace(['-', ' '], "_");
        Some(match name.as_str() {
            "utf_8" | "utf8" | "u8" | "utf" | "utf8_ucs2" | "utf8_ucs4" | "cp65001" => Self::Utf8,
            "ascii" | "us_ascii" | "646" | "ansi_x3.4_1968" | "ansi_x3_4_1968"
            | "ansi_x3.4_1986" | "cp367" | "csascii" | "ibm367" | "iso646_us"
            | "iso_646.irv_1991" | "iso_ir_6" | "us" => Self::Ascii,
            "latin_1" | "latin1" | "latin" | "l1" | "iso8859_1" | "iso_8859_1" | "8859"
            | "cp819" | "iso_ir_100" | "csisolatin1" | "ibm819" | "iso_8859_1_1987" => Self::Latin1,
            "cp1252" | "windows_1252" => Self::Cp1252,
            "utf_16_le" | "utf_16le" | "unicodelittleunmarked" => Self::Utf16Le,
            "utf_16_be" | "utf_16be" | "unicodebigunmarked" => Self::Utf16Be,
            "utf_32_le" | "utf_32le" => Self::Utf32Le,
            "utf_32_be" | "utf_32be" => Self::Utf32Be,
            "utf_16" | "utf16" | "u16" => Self::Utf16,
            "utf_32" | "utf32" | "u32" => Self::Utf32,
            _ => return None,
        })
    }

    /// Whether every `encode` call starts with a BOM.
    pub fn has_bom(self) -> bool {
        matches!(self, Self::Utf16 | Self::Utf32)
    }

    /// `text.encode(self, 'ignore')`.
    pub fn encode(self, text: &str) -> Vec<u8> {
        match self {
            Self::Utf16 => {
                let mut bytes = vec![0xFF, 0xFE];
                bytes.extend(Self::Utf16Le.encode(text));
                return bytes;
            }
            Self::Utf32 => {
                let mut bytes = vec![0xFF, 0xFE, 0, 0];
                bytes.extend(Self::Utf32Le.encode(text));
                return bytes;
            }
            _ => {}
        }
        let mut bytes = Vec::with_capacity(text.len());
        for c in text.chars() {
            let code = u32::from(c);
            match self {
                Self::Utf8 => {
                    let mut buffer = [0; 4];
                    bytes.extend_from_slice(c.encode_utf8(&mut buffer).as_bytes());
                }
                Self::Ascii if code < 0x80 => bytes.push(code as u8),
                Self::Latin1 if code < 0x100 => bytes.push(code as u8),
                Self::Cp1252 => {
                    if code < 0x80 || (0xA0..0x100).contains(&code) {
                        bytes.push(code as u8);
                    } else if let Some(index) = CP1252_HIGH.iter().position(|high| *high == Some(c))
                    {
                        bytes.push(0x80 + index as u8);
                    }
                }
                Self::Utf16Le | Self::Utf16Be => {
                    let mut buffer = [0; 2];
                    for unit in c.encode_utf16(&mut buffer) {
                        bytes.extend_from_slice(&if self == Self::Utf16Le {
                            unit.to_le_bytes()
                        } else {
                            unit.to_be_bytes()
                        });
                    }
                }
                Self::Utf32Le => bytes.extend_from_slice(&code.to_le_bytes()),
                Self::Utf32Be => bytes.extend_from_slice(&code.to_be_bytes()),
                Self::Ascii | Self::Latin1 | Self::Utf16 | Self::Utf32 => {}
            }
        }
        bytes
    }
}

static OUTPUT: std::sync::OnceLock<Encoding> = std::sync::OnceLock::new();

/// Set the `--encoding` for messages (once).
pub fn set_output(encoding: Encoding) {
    let _ = OUTPUT.set(encoding);
}

/// The `--encoding` for messages (UTF-8 by default).
pub fn output() -> Encoding {
    OUTPUT.get().copied().unwrap_or(Encoding::Utf8)
}

/// Upstream `write_string(text, out, encoding)`: one encoded write,
/// flushed (stderr when `stderr`).
pub fn write_message(stderr: bool, text: &str) {
    use std::io::Write;
    let bytes = output().encode(text);
    if stderr {
        let mut stream = std::io::stderr().lock();
        let _ = stream.write_all(&bytes);
        let _ = stream.flush();
    } else {
        let mut stream = std::io::stdout().lock();
        let _ = stream.write_all(&bytes);
        let _ = stream.flush();
    }
}

/// A writer encoding the UTF-8 text written to it (incomplete sequences
/// wait for the next write; invalid bytes pass through unchanged). A BOM
/// codec encodes what was written since the last `flush` as one call, so
/// callers flush after each upstream `write_string` message.
pub struct EncodingWriter<W: std::io::Write> {
    inner: W,
    encoding: Encoding,
    pending: Vec<u8>,
}

impl<W: std::io::Write> EncodingWriter<W> {
    pub fn new(inner: W, encoding: Encoding) -> Self {
        Self {
            inner,
            encoding,
            pending: Vec::new(),
        }
    }
}

impl<W: std::io::Write> std::io::Write for EncodingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.encoding == Encoding::Utf8 {
            return self.inner.write(buf);
        }
        self.pending.extend_from_slice(buf);
        if self.encoding.has_bom() {
            return Ok(buf.len());
        }
        let mut out = Vec::new();
        let mut rest: &[u8] = &self.pending;
        loop {
            match std::str::from_utf8(rest) {
                Ok(text) => {
                    out.extend(self.encoding.encode(text));
                    rest = &[];
                    break;
                }
                Err(error) => {
                    let (valid, after) = rest.split_at(error.valid_up_to());
                    out.extend(
                        self.encoding
                            .encode(std::str::from_utf8(valid).expect("valid prefix")),
                    );
                    match error.error_len() {
                        Some(length) => {
                            out.extend_from_slice(&after[..length]);
                            rest = &after[length..];
                        }
                        // An incomplete sequence at the end.
                        None => {
                            rest = after;
                            break;
                        }
                    }
                }
            }
        }
        self.pending = rest.to_vec();
        self.inner.write_all(&out)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if self.encoding.has_bom() && !self.pending.is_empty() {
            let text = String::from_utf8_lossy(&self.pending).into_owned();
            self.pending.clear();
            self.inner.write_all(&self.encoding.encode(&text))?;
        }
        self.inner.flush()
    }
}

impl<W: std::io::Write> Drop for EncodingWriter<W> {
    fn drop(&mut self) {
        if self.encoding.has_bom() {
            let _ = std::io::Write::flush(self);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn bom_codecs_mark_every_call() {
        assert_eq!(Encoding::lookup("UTF-16"), Some(Encoding::Utf16));
        assert_eq!(Encoding::Utf16.encode("a"), b"\xff\xfea\x00");
        assert_eq!(
            Encoding::Utf32.encode("a"),
            b"\xff\xfe\x00\x00a\x00\x00\x00"
        );
        let mut writer = EncodingWriter::new(Vec::new(), Encoding::Utf16);
        write!(writer, "a").unwrap();
        write!(writer, "b").unwrap();
        writer.flush().unwrap();
        writeln!(writer, "c").unwrap();
        writer.flush().unwrap();
        assert_eq!(
            writer.inner,
            b"\xff\xfea\x00b\x00\xff\xfec\x00\n\x00".to_vec()
        );
    }

    #[test]
    fn encodings_drop_unrepresentable_characters_like_python_ignore() {
        let text = "Ünï ☃ 日本 café €";
        assert_eq!(Encoding::lookup("Latin-1"), Some(Encoding::Latin1));
        assert_eq!(Encoding::lookup("bogus"), None);
        assert_eq!(Encoding::lookup("utf-16"), Some(Encoding::Utf16));
        assert_eq!(Encoding::Ascii.encode(text), b"n   caf ");
        assert_eq!(Encoding::Latin1.encode(text), b"\xdcn\xef   caf\xe9 ");
        assert_eq!(Encoding::Cp1252.encode(text), b"\xdcn\xef   caf\xe9 \x80");
        assert_eq!(Encoding::Utf16Le.encode("a☃"), b"a\x00\x03\x26");
        assert_eq!(Encoding::Utf32Be.encode("a"), b"\x00\x00\x00a");
        let mut writer = EncodingWriter::new(Vec::new(), Encoding::Latin1);
        let bytes = "é☃".as_bytes();
        writer.write_all(&bytes[..1]).unwrap();
        writer.write_all(&bytes[1..]).unwrap();
        assert_eq!(writer.inner, b"\xe9");
    }
}

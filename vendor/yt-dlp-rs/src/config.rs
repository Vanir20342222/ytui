//! Upstream configuration files: `Config.read_file` (`shlex.split` with
//! comments), `determine_file_encoding` and the user/system search order.
use std::path::{Path, PathBuf};

/// Configuration package name. Upstream reads `yt-dlp` files; this port
/// reads its own `yt-dlp-rs` files so an upstream configuration with
/// unsupported options cannot break it.
pub const PACKAGE_NAME: &str = "yt-dlp-rs";

/// Python `shlex.split(text, comments=True)` (POSIX mode).
pub fn shlex_split(text: &str) -> Result<Vec<String>, String> {
    #[derive(Clone, Copy, PartialEq)]
    enum State {
        Whitespace,
        Word,
        Quote(char),
        Escape { after: Option<char> },
        Comment,
    }
    let is_space = |c: char| matches!(c, ' ' | '\t' | '\r' | '\n');
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut quoted = false;
    let mut state = State::Whitespace;
    // `escapedstate` is the quote (or word) the escape returns to.
    for c in text.chars() {
        match state {
            State::Comment => {
                if c == '\n' {
                    // After a comment, POSIX mode emits any pending token.
                    if !token.is_empty() || quoted {
                        tokens.push(std::mem::take(&mut token));
                        quoted = false;
                    }
                    state = State::Whitespace;
                }
            }
            State::Whitespace => {
                if is_space(c) {
                    continue;
                } else if c == '#' {
                    state = State::Comment;
                } else if c == '\\' {
                    state = State::Escape { after: None };
                } else if c == '\'' || c == '"' {
                    state = State::Quote(c);
                } else {
                    token.push(c);
                    state = State::Word;
                }
            }
            State::Word => {
                if is_space(c) {
                    tokens.push(std::mem::take(&mut token));
                    quoted = false;
                    state = State::Whitespace;
                } else if c == '#' {
                    state = State::Comment;
                } else if c == '\'' || c == '"' {
                    state = State::Quote(c);
                } else if c == '\\' {
                    state = State::Escape { after: None };
                } else {
                    token.push(c);
                }
            }
            State::Quote(quote) => {
                quoted = true;
                if c == quote {
                    state = State::Word;
                } else if c == '\\' && quote == '"' {
                    state = State::Escape { after: Some(quote) };
                } else {
                    token.push(c);
                }
            }
            State::Escape { after } => {
                // Inside double quotes only the quote or the escape itself
                // may be escaped; otherwise the backslash is kept.
                if let Some(quote) = after
                    && c != '\\'
                    && c != quote
                {
                    token.push('\\');
                }
                token.push(c);
                state = match after {
                    Some(quote) => State::Quote(quote),
                    None => State::Word,
                };
            }
        }
    }
    match state {
        State::Quote(_) => return Err("No closing quotation".into()),
        State::Escape { .. } => return Err("No escaped character".into()),
        _ => {}
    }
    if !token.is_empty() || quoted {
        tokens.push(token);
    }
    Ok(tokens)
}

/// Upstream `determine_file_encoding` plus decoding: BOMs first, then a
/// first-line `# coding: NAME` declaration; UTF-8 otherwise.
fn decode(data: &[u8]) -> Result<String, String> {
    const BOMS: [(&[u8], &str); 5] = [
        (b"\xef\xbb\xbf", "utf-8"),
        (b"\x00\x00\xfe\xff", "utf-32-be"),
        (b"\xff\xfe\x00\x00", "utf-32-le"),
        (b"\xff\xfe", "utf-16-le"),
        (b"\xfe\xff", "utf-16-be"),
    ];
    let (encoding, body) = BOMS
        .iter()
        .find(|(bom, _)| data.starts_with(bom))
        .map(|(bom, encoding)| (Some((*encoding).to_owned()), &data[bom.len()..]))
        .unwrap_or_else(|| {
            let head: Vec<u8> = data.iter().take(512).copied().filter(|b| *b != 0).collect();
            static CODING: std::sync::LazyLock<regex::bytes::Regex> =
                std::sync::LazyLock::new(|| {
                    regex::bytes::Regex::new(
                        r"^#[ \t\r\f\v]*coding[ \t\r\f\v]*:[ \t\r\f\v]*(\S+)[ \t\r\f\v]*(?:\n|$)",
                    )
                    .unwrap()
                });
            let encoding = CODING
                .captures(&head)
                .map(|found| String::from_utf8_lossy(&found[1]).to_lowercase());
            (encoding, data)
        });
    let units = |size: usize, big: bool| -> Vec<u32> {
        body.chunks(size)
            .map(|chunk| {
                let mut bytes = chunk.to_vec();
                bytes.resize(size, 0);
                if !big {
                    bytes.reverse();
                }
                bytes
                    .iter()
                    .fold(0u32, |value, byte| value << 8 | u32::from(*byte))
            })
            .collect()
    };
    let invalid = |name: &str| format!("'{name}' codec can't decode the file");
    match encoding.as_deref().map(|e| e.replace('_', "-")).as_deref() {
        None | Some("utf-8" | "utf8" | "utf-8-sig") => {
            String::from_utf8(body.to_vec()).map_err(|_| invalid("utf-8"))
        }
        Some("ascii" | "us-ascii") if body.is_ascii() => Ok(String::from_utf8_lossy(body).into()),
        Some("latin-1" | "latin1" | "iso-8859-1" | "iso8859-1") => {
            Ok(body.iter().map(|byte| char::from(*byte)).collect())
        }
        Some(name @ ("utf-16-le" | "utf-16-be")) => {
            let units: Vec<u16> = units(2, name.ends_with("be"))
                .into_iter()
                .map(|unit| unit as u16)
                .collect();
            String::from_utf16(&units).map_err(|_| invalid(name))
        }
        Some(name @ ("utf-32-le" | "utf-32-be")) => units(4, name.ends_with("be"))
            .into_iter()
            .map(|unit| char::from_u32(unit).ok_or_else(|| invalid(name)))
            .collect(),
        Some(name) => Err(format!("unknown encoding: {name}")),
    }
}

/// Upstream `Config.read_file`: `Ok(None)` when the file cannot be opened;
/// parse failures are `Unable to parse "FILE": REASON`.
pub fn read_file(path: &Path) -> Result<Option<Vec<String>>, String> {
    let Ok(data) = std::fs::read(path) else {
        return Ok(None);
    };
    decode(&data)
        .and_then(|text| shlex_split(&text))
        .map(Some)
        .map_err(|error| format!("Unable to parse \"{}\": {error}", path.display()))
}

/// Upstream `get_user_config_dirs`: `$XDG_CONFIG_HOME` (or `~/.config`),
/// `%APPDATA%`, then `~/.NAME`.
pub fn user_config_dirs(home: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let xdg = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.map(|home| home.join(".config")));
    if let Some(xdg) = xdg {
        dirs.push(xdg.join(PACKAGE_NAME));
    }
    if let Some(appdata) = std::env::var_os("appdata")
        .or_else(|| std::env::var_os("APPDATA"))
        .filter(|value| !value.is_empty())
    {
        dirs.push(PathBuf::from(appdata).join(PACKAGE_NAME));
    }
    if let Some(home) = home {
        dirs.push(home.join(format!(".{PACKAGE_NAME}")));
    }
    dirs
}

/// Upstream `get_system_config_dirs`.
pub fn system_config_dirs() -> Vec<PathBuf> {
    vec![Path::new("/etc").join(PACKAGE_NAME)]
}

/// Upstream `_load_from_config_dirs` candidates for one directory, in order.
pub fn dir_candidates(dir: &Path) -> Vec<PathBuf> {
    let head = dir.parent().unwrap_or(Path::new(""));
    let tail = dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut candidates = vec![head.join(format!("{PACKAGE_NAME}.conf"))];
    if tail.starts_with('.') {
        candidates.push(head.join(format!("{PACKAGE_NAME}.conf.txt")));
    }
    candidates.push(dir.join("config"));
    candidates.push(dir.join("config.txt"));
    candidates
}

/// The first readable configuration among the directories' candidates.
pub fn first_config(dirs: &[PathBuf]) -> Result<Option<(Vec<String>, PathBuf)>, String> {
    for dir in dirs {
        for candidate in dir_candidates(dir) {
            if let Some(args) = read_file(&candidate)? {
                return Ok(Some((args, candidate)));
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_python_shlex_cases() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/config/shlex.json")).unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let input = case["input"].as_str().unwrap();
            let result = shlex_split(input);
            match case.get("error").and_then(|e| e.as_str()) {
                Some(error) => assert_eq!(result, Err(error.to_owned()), "{input:?}"),
                None => assert_eq!(
                    result.unwrap(),
                    case["tokens"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|t| t.as_str().unwrap().to_owned())
                        .collect::<Vec<_>>(),
                    "{input:?}"
                ),
            }
        }
    }

    #[test]
    fn encodings_follow_boms_and_coding_declarations() {
        assert_eq!(decode(b"\xef\xbb\xbf-f b").unwrap(), "-f b");
        assert_eq!(decode(b"\xff\xfe-\x00f\x00").unwrap(), "-f");
        assert_eq!(
            decode(b"# coding: latin-1\n-o \xe9").unwrap(),
            "# coding: latin-1\n-o \u{e9}"
        );
        assert!(decode(b"-o \xe9").is_err());
    }

    #[test]
    fn config_dir_candidates_follow_upstream_order() {
        assert_eq!(
            dir_candidates(Path::new("/home/u/.config/yt-dlp-rs")),
            [
                PathBuf::from("/home/u/.config/yt-dlp-rs.conf"),
                PathBuf::from("/home/u/.config/yt-dlp-rs/config"),
                PathBuf::from("/home/u/.config/yt-dlp-rs/config.txt"),
            ]
        );
        assert_eq!(
            dir_candidates(Path::new("/home/u/.yt-dlp-rs")),
            [
                PathBuf::from("/home/u/yt-dlp-rs.conf"),
                PathBuf::from("/home/u/yt-dlp-rs.conf.txt"),
                PathBuf::from("/home/u/.yt-dlp-rs/config"),
                PathBuf::from("/home/u/.yt-dlp-rs/config.txt"),
            ]
        );
    }
}

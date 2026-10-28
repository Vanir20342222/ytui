//! Cookie material is scoped, never Debug-printed, and only exported to private files.
use crate::paths::private_dir;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Serialize, Deserialize)]
pub struct Cookie {
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub expires: Option<u64>,
    pub name: String,
    pub value: String,
    pub http_only: bool,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Session {
    pub cookies: Vec<Cookie>,
    pub account: String,
    pub imported_at: u64,
}
impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Session([redacted])")
    }
}
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn scoped(domain: &str) -> bool {
    let d = domain.trim_start_matches('.');
    d == "youtube.com"
        || d.ends_with(".youtube.com")
        || d == "google.com"
        || d == "accounts.google.com"
}
impl Session {
    pub fn new(cookies: Vec<Cookie>) -> Result<Self> {
        let cookies = cookies
            .into_iter()
            .filter(|c| scoped(&c.domain) && c.expires.is_none_or(|e| e == 0 || e > now()))
            .filter(|c| {
                ![&c.name, &c.value, &c.domain, &c.path]
                    .iter()
                    .any(|s| s.chars().any(char::is_control))
            })
            .collect::<Vec<_>>();
        let result = Self {
            cookies,
            account: String::new(),
            imported_at: now(),
        };
        ensure!(
            result.sapisid("www.youtube.com").is_some(),
            "No active YouTube SAPISID cookie found; sign in to YouTube in this profile first"
        );
        Ok(result)
    }
    fn domain_matches(cookie: &Cookie, host: &str) -> bool {
        let d = cookie.domain.trim_start_matches('.');
        host == d || (cookie.domain.starts_with('.') && host.ends_with(&format!(".{d}")))
    }
    fn sapisid(&self, host: &str) -> Option<&str> {
        ["SAPISID", "__Secure-3PAPISID", "__Secure-1PAPISID"]
            .iter()
            .find_map(|name| {
                self.cookies
                    .iter()
                    .find(|c| {
                        c.name == *name
                            && Self::domain_matches(c, host)
                            && c.expires.is_none_or(|e| e == 0 || e > now())
                    })
                    .map(|c| c.value.as_str())
            })
    }
    pub fn headers(&self, origin: &str) -> Result<(String, String)> {
        ensure!(
            ["https://www.youtube.com", "https://music.youtube.com"].contains(&origin),
            "Authentication origin is not allowed"
        );
        let host = origin
            .strip_prefix("https://")
            .context("Invalid authentication origin")?;
        let sapisid = self
            .sapisid(host)
            .context("YouTube session expired; import your browser session again")?;
        let cookie = self
            .cookies
            .iter()
            .filter(|c| {
                Self::domain_matches(c, host)
                    && c.path == "/"
                    && c.expires.is_none_or(|e| e == 0 || e > now())
            })
            .map(|c| format!("{}={}", c.name, c.value))
            .collect::<Vec<_>>()
            .join("; ");
        let timestamp = now();
        let hash = hex::encode(Sha1::digest(format!("{timestamp} {sapisid} {origin}")));
        Ok((cookie, format!("SAPISIDHASH {timestamp}_{hash}")))
    }
    pub fn export(&self, dir: &Path) -> Result<tempfile::NamedTempFile> {
        use std::io::Write;
        private_dir(dir)?;
        let mut file = tempfile::Builder::new()
            .prefix("session-")
            .suffix(".cookies.txt")
            .tempfile_in(dir)?;
        writeln!(file, "# Netscape HTTP Cookie File")?;
        for c in &self.cookies {
            let domain = if c.http_only {
                format!("#HttpOnly_{}", c.domain)
            } else {
                c.domain.clone()
            };
            writeln!(
                file,
                "{}\t{}\t{}\t{}\t{}\t{}\t{}",
                domain,
                if c.domain.starts_with('.') {
                    "TRUE"
                } else {
                    "FALSE"
                },
                c.path,
                if c.secure { "TRUE" } else { "FALSE" },
                c.expires.unwrap_or(0),
                c.name,
                c.value
            )?;
        }
        file.flush()?;
        Ok(file)
    }
}

pub fn import_netscape(path: &Path) -> Result<Session> {
    use std::io::Read;
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options
        .open(path)
        .context("Cannot read Netscape cookie file")?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file() && metadata.len() <= 2 * 1024 * 1024,
        "Choose a regular cookie file up to 2 MiB"
    );
    let mut data = String::new();
    file.take(2 * 1024 * 1024 + 1)
        .read_to_string(&mut data)
        .context("Cannot read UTF-8 cookie document")?;
    parse_netscape(&data)
}

/// Parse a user-selected document without writing a plaintext intermediate file.
pub fn parse_netscape(data: &str) -> Result<Session> {
    ensure!(data.len() <= 2 * 1024 * 1024, "Cookie file exceeds 2 MiB");
    let mut cookies = Vec::new();
    for line in data.lines() {
        let (line, http_only) = if let Some(l) = line.strip_prefix("#HttpOnly_") {
            (l, true)
        } else {
            (line, false)
        };
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let fields = line.splitn(7, '\t').collect::<Vec<_>>();
        ensure!(fields.len() == 7, "Invalid Netscape cookie line");
        if !scoped(fields[0]) {
            continue;
        }
        cookies.push(Cookie {
            domain: fields[0].into(),
            path: fields[2].into(),
            secure: fields[3] == "TRUE",
            expires: fields[4].parse().ok(),
            name: fields[5].into(),
            value: fields[6].into(),
            http_only,
        });
    }
    Session::new(cookies)
}

#[cfg(test)]
mod document_tests {
    use super::*;
    #[test]
    fn in_memory_cookie_import_and_file_import_agree_without_unbounded_reads() {
        let text = "# Netscape HTTP Cookie File\n.youtube.com\tTRUE\t/\tTRUE\t0\tSAPISID\tsynthetic-only\n";
        let parsed = parse_netscape(text).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.txt");
        std::fs::write(&path, text).unwrap();
        assert_eq!(
            import_netscape(&path)
                .unwrap()
                .headers("https://www.youtube.com")
                .unwrap()
                .0,
            parsed.headers("https://www.youtube.com").unwrap().0
        );
        assert!(parse_netscape(&"x".repeat(2 * 1024 * 1024 + 1)).is_err());
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(2 * 1024 * 1024 + 1)
            .unwrap();
        assert!(import_netscape(&path).is_err());
        assert!(import_netscape(dir.path()).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn cookie_pipe_is_rejected_without_waiting_for_a_writer() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.pipe");
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&path)
                .status()
                .unwrap()
                .success()
        );
        assert!(import_netscape(&path).is_err());
    }
}

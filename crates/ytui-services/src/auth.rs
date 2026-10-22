//! Cookie material is scoped, never Debug-printed, and only exported to private files.
use crate::paths::{Paths, atomic_write, private_dir};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::{
    path::{Path, PathBuf},
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
    let data = std::fs::read_to_string(path).context("Cannot read Netscape cookie file")?;
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

pub fn import_browser(browser: &str, profile: Option<&Path>) -> Result<Session> {
    if browser == "brave-origin" && profile.is_none() {
        let paths = Paths::discover()?;
        let candidates = profiles(&paths)
            .into_iter()
            .filter(|p| p.browser == "brave-origin")
            .collect::<Vec<_>>();
        ensure!(
            candidates.len() == 1,
            "Brave Origin has zero or multiple discoverable profiles; use `ytui-rs auth profiles` and select --profile PATH"
        );
        return import_browser("brave", Some(&candidates[0].path));
    }
    let domains = Some(vec!["youtube.com".into(), "google.com".into()]);
    // No automatic all-browser scan: opening a browser store is an explicit user action.
    let result = if let Some(profile) = profile {
        let file = if profile.is_file() {
            profile.into()
        } else if profile.join("Network/Cookies").exists() {
            profile.join("Network/Cookies")
        } else if profile.join("Cookies").exists() {
            profile.join("Cookies")
        } else {
            profile.join("cookies.sqlite")
        };
        if matches!(browser, "firefox" | "zen" | "librewolf") {
            rookie::firefox_based(file, domains)
        } else {
            #[cfg(unix)]
            {
                let name = if browser == "brave-origin" {
                    "brave"
                } else {
                    browser
                };
                ensure!(
                    ["brave", "chrome", "chromium", "edge", "vivaldi"].contains(&name),
                    "Unsupported browser"
                );
                rookie::chromium_based(rookie::config::get_browser_config(name), file, domains)
            }
            #[cfg(not(unix))]
            {
                bail!(
                    "Custom Chromium profile import currently requires Linux or macOS; use a Netscape cookie file"
                );
            }
        }
    } else {
        match browser {
            "brave" => rookie::brave(domains),
            "chrome" => rookie::chrome(domains),
            "chromium" => rookie::chromium(domains),
            "firefox" => rookie::firefox(domains),
            "librewolf" => rookie::librewolf(domains),
            "edge" => rookie::edge(domains),
            "vivaldi" => rookie::vivaldi(domains),
            "brave-origin" | "zen" => bail!(
                "Select a profile path for {browser}; use `ytui-rs auth profiles` to discover local stores"
            ),
            _ => bail!(
                "Unsupported browser; select a supported browser/profile or import a Netscape cookie file"
            ),
        }
    };
    let cookies=result.map_err(|_|anyhow::anyhow!("Cannot read/decrypt the selected browser profile. Unlock the desktop keyring or select another profile / Netscape cookie file. Root is not required."))?;
    Session::new(
        cookies
            .into_iter()
            .map(|c| Cookie {
                domain: c.domain,
                path: c.path,
                secure: c.secure,
                expires: c.expires,
                name: c.name,
                value: c.value,
                http_only: c.http_only,
            })
            .collect(),
    )
}
#[derive(Clone, Debug, Serialize)]
pub struct BrowserProfile {
    pub browser: String,
    pub path: PathBuf,
}
/// Discover directory names only; never open a cookie database during discovery.
pub fn profiles(paths: &Paths) -> Vec<BrowserProfile> {
    let roots = [
        (
            "brave",
            paths
                .config
                .parent()
                .unwrap_or(&paths.home)
                .join("BraveSoftware"),
        ),
        (
            "chromium",
            paths
                .config
                .parent()
                .unwrap_or(&paths.home)
                .join("chromium"),
        ),
        (
            "chrome",
            paths
                .config
                .parent()
                .unwrap_or(&paths.home)
                .join("google-chrome"),
        ),
        ("firefox", paths.home.join(".mozilla/firefox")),
        (
            "zen",
            paths.config.parent().unwrap_or(&paths.home).join("zen"),
        ),
        ("librewolf", paths.home.join(".librewolf")),
        (
            "brave",
            paths
                .home
                .join(".var/app/com.brave.Browser/config/BraveSoftware"),
        ),
        (
            "firefox",
            paths
                .home
                .join(".var/app/org.mozilla.firefox/.mozilla/firefox"),
        ),
    ];
    fn walk(browser: &str, path: &Path, depth: usize, out: &mut Vec<BrowserProfile>) {
        if depth == 0 {
            return;
        }
        if ["Cookies", "Network/Cookies", "cookies.sqlite"]
            .iter()
            .any(|f| path.join(f).is_file())
        {
            out.push(BrowserProfile {
                browser: if path
                    .components()
                    .any(|c| c.as_os_str().to_string_lossy().starts_with("Brave-Origin"))
                {
                    "brave-origin".into()
                } else {
                    browser.into()
                },
                path: path.into(),
            });
            return;
        }
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten().take(100) {
                if entry.file_type().is_ok_and(|t| t.is_dir()) {
                    walk(browser, &entry.path(), depth - 1, out);
                }
            }
        }
    }
    let mut out = Vec::new();
    for (b, p) in roots {
        walk(b, &p, 4, &mut out);
    }
    out
}

#[derive(Clone)]
pub struct SessionStore {
    mode: String,
    dir: PathBuf,
}
impl SessionStore {
    pub fn new(paths: &Paths, mode: &str) -> Self {
        Self {
            mode: mode.into(),
            dir: paths.state.join("auth"),
        }
    }
    fn entry(&self) -> Result<keyring::Entry> {
        Ok(keyring::Entry::new("ytui-rust", "youtube-session")?)
    }
    pub fn load(&self) -> Result<Option<Session>> {
        let data = if self.mode == "file" {
            match std::fs::read_to_string(self.dir.join("session.json")) {
                Ok(v) => Some(v),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => return Err(e.into()),
            }
        } else {
            match self.entry()?.get_password() {
                Ok(v) => Some(v),
                Err(keyring::Error::NoEntry) => None,
                Err(_) => bail!(
                    "Desktop keyring unavailable. Unlock it, or explicitly choose auth_store = 'file' in rust.toml."
                ),
            }
        };
        data.map(|v| serde_json::from_str(&v).context("Stored session is invalid; import it again"))
            .transpose()
    }
    pub fn save(&self, session: &Session) -> Result<()> {
        let data = serde_json::to_string(session)?;
        if self.mode == "file" {
            private_dir(&self.dir)?;
            atomic_write(&self.dir.join("session.json"), data.as_bytes())
        } else {
            self.entry()?.set_password(&data).map_err(|_|anyhow::anyhow!("Could not store the session in the desktop keyring; existing credentials were left intact"))
        }
    }
    pub fn disconnect(&self) -> Result<()> {
        if self.dir.join("session.json").exists() {
            std::fs::remove_file(self.dir.join("session.json"))?;
        }
        match self
            .entry()
            .and_then(|e| e.delete_credential().map_err(Into::into))
        {
            Ok(()) => {}
            Err(_) if self.mode == "file" => {}
            Err(e)
                if matches!(
                    e.downcast_ref::<keyring::Error>(),
                    Some(keyring::Error::NoEntry)
                ) => {}
            Err(e) => return Err(e),
        };
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Session {
        Session::new(vec![
            Cookie {
                domain: ".youtube.com".into(),
                path: "/".into(),
                secure: true,
                expires: None,
                name: "SAPISID".into(),
                value: "synthetic-secret".into(),
                http_only: true,
            },
            Cookie {
                domain: "evilgoogle.com".into(),
                path: "/".into(),
                secure: true,
                expires: None,
                name: "bad".into(),
                value: "secret".into(),
                http_only: false,
            },
        ])
        .unwrap()
    }
    #[test]
    fn scopes_authentication_and_redacts_debug() {
        let s = fixture();
        assert_eq!(s.cookies.len(), 1);
        assert!(s.headers("https://radio.cliamp.stream").is_err());
        assert!(
            s.headers("https://music.youtube.com")
                .unwrap()
                .1
                .starts_with("SAPISIDHASH ")
        );
        assert!(!format!("{s:?}").contains("synthetic"));
    }
    #[test]
    fn netscape_roundtrip_is_private() {
        let d = tempfile::tempdir().unwrap();
        let file = fixture().export(d.path()).unwrap();
        assert_eq!(import_netscape(file.path()).unwrap().cookies.len(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(file.path()).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}

#[cfg(test)]
mod discovery_tests {
    use super::*;
    #[test]
    fn origin_profile_discovery_only_lists_paths() {
        let t = tempfile::tempdir().unwrap();
        let p = Paths {
            config_override: None,
            xdg_state: t.path().join("state"),
            config: t.path().join("config/ytui"),
            state: t.path().join("state"),
            cache: t.path().join("cache"),
            legacy_data: t.path().join("data"),
            home: t.path().into(),
        };
        let profile = t
            .path()
            .join("config/BraveSoftware/Brave-Origin/Default/Network");
        std::fs::create_dir_all(&profile).unwrap();
        std::fs::write(
            profile.join("Cookies"),
            b"not a database; discovery must not read it",
        )
        .unwrap();
        let discovered = profiles(&p);
        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].browser, "brave-origin");
    }
}

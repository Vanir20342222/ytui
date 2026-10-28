//! Upstream `--cookies-from-browser` (`yt_dlp/cookies.py`): cookies read
//! from a browser profile's database, with upstream's messages.
use crate::{Error, Result, sqlite};
use std::path::{Path, PathBuf};

pub const SUPPORTED_BROWSERS: [&str; 9] = [
    "brave", "chrome", "chromium", "edge", "firefox", "opera", "safari", "vivaldi", "whale",
];
pub const SUPPORTED_KEYRINGS: [&str; 5] = [
    "BASICTEXT",
    "GNOMEKEYRING",
    "KWALLET",
    "KWALLET5",
    "KWALLET6",
];

/// `BROWSER[+KEYRING][:PROFILE][::CONTAINER]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserSpec {
    pub browser: String,
    pub profile: Option<String>,
    pub keyring: Option<String>,
    pub container: Option<String>,
}

/// `\s*::\s*(?P<container>.+)` matching all of `text`.
fn container_part(text: &str) -> Option<&str> {
    let rest = text.trim_start().strip_prefix("::")?;
    let trimmed = rest.trim_start();
    if !trimmed.is_empty() {
        Some(trimmed)
    } else if !rest.is_empty() {
        // `\s*` gives a whitespace character back to `.+`.
        Some(&rest[rest.len() - rest.chars().last()?.len_utf8()..])
    } else {
        None
    }
}

impl BrowserSpec {
    /// Upstream `__init__` validation; errors are usage-error texts.
    pub fn parse(text: &str) -> std::result::Result<Self, String> {
        let invalid = || format!("invalid cookies from browser arguments: {text}");
        let name_end = text.find(['+', ':']).unwrap_or(text.len());
        if name_end == 0 {
            return Err(invalid());
        }
        let mut rest = &text[name_end..];
        let mut keyring = None;
        if let Some(after) = rest.strip_prefix('+') {
            let after = after.trim_start();
            let end = after.find(':').unwrap_or(after.len());
            if end == 0 {
                return Err(invalid());
            }
            keyring = Some(&after[..end]);
            rest = &after[end..];
        }
        let (mut profile, mut container) = (None, None);
        if !rest.is_empty() {
            if let Some(found) = container_part(rest) {
                container = Some(found);
            } else {
                let after = rest
                    .trim_start()
                    .strip_prefix(':')
                    .ok_or_else(invalid)?
                    .trim_start();
                if after.is_empty() || after.starts_with(':') {
                    return Err(invalid());
                }
                // `(?P<profile>.+?)` is as short as the optional container allows.
                let mut split = None;
                for (index, _) in after.char_indices().skip(1) {
                    if let Some(found) = container_part(&after[index..]) {
                        split = Some((index, found));
                        break;
                    }
                }
                match split {
                    Some((index, found)) => {
                        profile = Some(&after[..index]);
                        container = Some(found);
                    }
                    None => profile = Some(after),
                }
            }
        }
        let browser = text[..name_end].to_lowercase();
        if !SUPPORTED_BROWSERS.contains(&browser.as_str()) {
            return Err(format!(
                "unsupported browser specified for cookies: \"{browser}\". Supported browsers are: {}",
                SUPPORTED_BROWSERS.join(", ")
            ));
        }
        let keyring = keyring.map(str::to_uppercase);
        if let Some(keyring) = &keyring
            && !SUPPORTED_KEYRINGS.contains(&keyring.as_str())
        {
            return Err(format!(
                "unsupported keyring specified for cookies: \"{keyring}\". Supported keyrings are: {}",
                SUPPORTED_KEYRINGS.join(", ")
            ));
        }
        // `_parse_browser_specification`: a profile path is expanded.
        let profile = profile.map(|profile| {
            let expanded = crate::batch::expand_path_text(profile);
            if is_path(&expanded) {
                expanded
            } else {
                profile.to_owned()
            }
        });
        Ok(Self {
            browser,
            profile,
            keyring,
            container: container.map(str::to_owned),
        })
    }
}

fn is_path(value: &str) -> bool {
    value.contains(std::path::MAIN_SEPARATOR) || (cfg!(windows) && value.contains('/'))
}

/// One extracted cookie, as `http.cookiejar.Cookie` fields.
#[derive(Debug, Clone, PartialEq)]
pub struct BrowserCookie {
    pub domain: String,
    pub name: String,
    pub value: String,
    pub path: String,
    pub expires: Option<f64>,
    pub secure: bool,
}

/// A message upstream's `YDLLogger` emits while extracting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    Info(String),
    Debug(String),
    Warning(String),
    /// `logger.error`: an `ERROR:` line that fails the run without ending it.
    Error(String),
}

/// Extracted cookies (deduplicated like a `CookieJar` by domain, path and
/// name) and the messages to report when upstream would load the jar.
#[derive(Debug, Default)]
pub struct Extracted {
    pub cookies: Vec<BrowserCookie>,
    pub messages: Vec<Message>,
    /// Upstream's `CookieLoadError` cause, raised where it loads the jar.
    pub error: Option<String>,
}

impl Extracted {
    fn info(&mut self, text: impl Into<String>) {
        self.messages.push(Message::Info(text.into()));
    }
    fn debug(&mut self, text: impl Into<String>) {
        self.messages.push(Message::Debug(text.into()));
    }
    fn warning(&mut self, text: impl Into<String>) {
        self.messages.push(Message::Warning(text.into()));
    }
    /// `report_warning(..., only_once=True)`.
    fn warning_once(&mut self, text: impl Into<String>) {
        let message = Message::Warning(text.into());
        if !self.messages.contains(&message) {
            self.messages.push(message);
        }
    }
    fn error(&mut self, text: impl Into<String>) {
        self.messages.push(Message::Error(text.into()));
    }
    /// `CookieJar.set_cookie`: a later cookie replaces one with the same key.
    fn set_cookie(&mut self, cookie: BrowserCookie) {
        match self.cookies.iter_mut().find(|existing| {
            existing.domain == cookie.domain
                && existing.path == cookie.path
                && existing.name == cookie.name
        }) {
            Some(existing) => *existing = cookie,
            None => self.cookies.push(cookie),
        }
    }
}

/// Upstream `extract_cookies_from_browser`; a failure keeps the messages
/// so far and upstream's error text (without cookies).
pub fn extract(spec: &BrowserSpec) -> Extracted {
    let mut extracted = Extracted::default();
    let result = match spec.browser.as_str() {
        "firefox" => firefox(spec, &mut extracted),
        "safari" => safari(spec, &mut extracted),
        browser => chromium(browser, spec, &mut extracted),
    };
    if let Err(error) = result {
        extracted.cookies.clear();
        extracted.error = Some(match error {
            Error::Reported(message) => message,
            other => other.to_string(),
        });
    }
    extracted
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
}

/// `_config_home`.
fn config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config"))
}

/// `_firefox_browser_dirs`.
fn firefox_browser_dirs() -> Vec<PathBuf> {
    let home = home();
    if cfg!(target_os = "macos") {
        vec![home.join("Library/Application Support/Firefox/Profiles")]
    } else if cfg!(windows) {
        let var = |name: &str| {
            std::env::var_os(name)
                .map(PathBuf::from)
                .unwrap_or_default()
        };
        vec![
            var("APPDATA").join(r"Mozilla\Firefox\Profiles"),
            var("LOCALAPPDATA").join(
                r"Packages\Mozilla.Firefox_n80bbvh6b1yt2\LocalCache\Roaming\Mozilla\Firefox\Profiles",
            ),
        ]
    } else {
        vec![
            config_home().join("mozilla/firefox"),
            home.join(".mozilla/firefox"),
            home.join(".var/app/org.mozilla.firefox/config/mozilla/firefox"),
            home.join(".var/app/org.mozilla.firefox/.mozilla/firefox"),
            home.join("snap/firefox/common/.mozilla/firefox"),
        ]
    }
}

/// Python `os.path.abspath` of a search root.
fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_owned())
}

/// `glob(os.path.join(root, pattern, 'cookies.sqlite'))` for the patterns
/// `''`, `'*/'` and `'Profiles/*/'`.
fn firefox_cookie_dbs(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let children = |directory: &Path| -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(directory) else {
            return Vec::new();
        };
        let mut entries: Vec<PathBuf> = entries
            .flatten()
            .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect();
        entries.sort();
        entries
    };
    for root in roots.iter().map(|root| absolute(root)) {
        let mut candidates = vec![root.join("cookies.sqlite")];
        candidates.extend(
            children(&root)
                .into_iter()
                .map(|child| child.join("cookies.sqlite")),
        );
        candidates.extend(
            children(&root.join("Profiles"))
                .into_iter()
                .map(|child| child.join("cookies.sqlite")),
        );
        found.extend(
            candidates
                .into_iter()
                .filter(|path| std::fs::symlink_metadata(path).is_ok()),
        );
    }
    found
}

/// `_newest`: the most recently modified (`lstat`) path; the first on ties.
fn newest(paths: Vec<PathBuf>) -> Option<PathBuf> {
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for path in paths {
        let Ok(modified) = std::fs::symlink_metadata(&path).and_then(|meta| meta.modified()) else {
            continue;
        };
        if best.as_ref().is_none_or(|(time, _)| modified > *time) {
            best = Some((modified, path));
        }
    }
    best.map(|(_, path)| path)
}

fn not_found(message: String) -> Error {
    Error::Reported(message)
}

/// Upstream `_extract_firefox_cookies`.
fn firefox(spec: &BrowserSpec, out: &mut Extracted) -> Result<()> {
    const MAX_SUPPORTED_DB_SCHEMA_VERSION: i64 = 17;
    out.info("Extracting cookies from firefox");
    let roots: Vec<PathBuf> = match &spec.profile {
        None => firefox_browser_dirs(),
        Some(profile) if is_path(profile) => vec![PathBuf::from(profile)],
        Some(profile) => firefox_browser_dirs()
            .into_iter()
            .map(|directory| directory.join(profile))
            .collect(),
    };
    let search_root = roots
        .iter()
        .map(|root| crate::batch::py_repr(&root.to_string_lossy()))
        .collect::<Vec<_>>()
        .join(", ");
    let database = newest(firefox_cookie_dbs(&roots)).ok_or_else(|| {
        not_found(format!(
            "could not find firefox cookies database in {search_root}"
        ))
    })?;
    out.debug(format!(
        "Extracting cookies from: \"{}\"",
        database.display()
    ));
    let container = spec.container.as_deref();
    let mut container_id = None;
    if let Some(container) = container.filter(|container| *container != "none") {
        let path = database.with_file_name("containers.json");
        let text = std::fs::read_to_string(&path)
            .map_err(|_| not_found(format!("could not read containers.json in {search_root}")))?;
        let json: serde_json::Value =
            serde_json::from_str(&text).map_err(|error| Error::Cookies(error.to_string()))?;
        static L10N: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
            regex::Regex::new(r"^userContext([^\.]+)\.label$").unwrap()
        });
        let identities = json
            .get("identities")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        // Upstream compares the name or the whole matched `l10nID`.
        let found = identities.iter().find(|context| {
            context.get("name").and_then(serde_json::Value::as_str) == Some(container)
                || context
                    .get("l10nID")
                    .and_then(serde_json::Value::as_str)
                    .filter(|id| L10N.is_match(id))
                    == Some(container)
        });
        container_id = found
            .and_then(|context| context.get("userContextId"))
            .and_then(serde_json::Value::as_i64);
        if container_id.is_none() {
            return Err(Error::Reported(format!(
                "could not find firefox container \"{container}\" in containers.json"
            )));
        }
    }
    let db = sqlite::Database::open(&database)?;
    let version = db.user_version();
    if version > MAX_SUPPORTED_DB_SCHEMA_VERSION {
        out.warning(format!(
            "Possibly unsupported firefox cookies database version: {version}"
        ));
    } else {
        out.debug(format!("Firefox cookies database version: {version}"));
    }
    if let Some(id) = container_id {
        out.debug(format!(
            "Only loading cookies from firefox container \"{}\", ID {id}",
            container.unwrap_or_default()
        ));
    } else if container == Some("none") {
        out.debug("Only loading cookies not belonging to any container");
    }
    let table = db
        .table("moz_cookies")?
        .ok_or_else(|| Error::Cookies("no such table: moz_cookies".into()))?;
    let column = |name: &str| {
        table
            .column(name)
            .ok_or_else(|| Error::Cookies(format!("no such column: {name}")))
    };
    let [host, name, value, path, expiry, secure] =
        ["host", "name", "value", "path", "expiry", "isSecure"].map(column);
    let (host, name, value, path, expiry, secure) = (host?, name?, value?, path?, expiry?, secure?);
    let origin = table.column("originAttributes");
    for row in &table.rows {
        let attributes = origin
            .and_then(|index| row[index].as_text())
            .unwrap_or_default()
            .to_lowercase();
        let keep = match container_id {
            // `originAttributes LIKE '%userContextId=ID' OR LIKE '%userContextId=ID&%'`.
            Some(id) => {
                let needle = format!("usercontextid={id}");
                attributes.ends_with(&needle) || attributes.contains(&format!("{needle}&"))
            }
            None if container == Some("none") => !attributes.contains("usercontextid="),
            None => true,
        };
        if !keep {
            continue;
        }
        let text = |index: usize| row[index].as_text().unwrap_or_default().to_owned();
        let mut expires = row[expiry].as_f64();
        // Schema 16 (Firefox 142) stores milliseconds.
        if version >= 16 {
            expires = expires.map(|expires| expires / 1000.0);
        }
        out.set_cookie(BrowserCookie {
            domain: text(host),
            name: text(name),
            value: text(value),
            path: text(path),
            expires,
            secure: row[secure].truthy(),
        });
    }
    let count = out.cookies.len();
    out.info(format!("Extracted {count} cookies from firefox"));
    Ok(())
}

/// Python `str(exception)` for a failed `Popen` of `program`.
fn os_error_text(error: &std::io::Error, program: &str) -> String {
    match error.raw_os_error() {
        Some(code) => {
            let text = error.to_string();
            let text = text
                .strip_suffix(&format!(" (os error {code})"))
                .unwrap_or(&text)
                .to_owned();
            format!("[Errno {code}] {text}: {}", crate::batch::py_repr(program))
        }
        None => error.to_string(),
    }
}

/// `Popen.run(...)` with stdout captured: (stdout, return code).
fn run_command(program: &str, args: &[&str]) -> std::io::Result<(Vec<u8>, i32)> {
    let output = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()?;
    Ok((output.stdout, output.status.code().unwrap_or(-1)))
}

/// `_find_files`: every `filename` below `root` (`os.walk`).
fn find_files(root: &Path, filename: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(std::fs::DirEntry::file_name);
        let mut directories = Vec::new();
        for entry in entries {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                directories.push(entry.path());
            } else if entry.file_name() == filename {
                found.push(entry.path());
            }
        }
        stack.extend(directories.into_iter().rev());
    }
    found
}

/// `_get_chromium_based_browser_settings`: (browser_dir, keyring_name,
/// supports_profiles).
fn chromium_settings(browser: &str) -> (PathBuf, &'static str, bool) {
    let home = home();
    let directory = if cfg!(windows) {
        let var = |name: &str| {
            std::env::var_os(name)
                .map(PathBuf::from)
                .unwrap_or_default()
        };
        let (base, path) = match browser {
            "brave" => ("LOCALAPPDATA", r"BraveSoftware\Brave-Browser\User Data"),
            "chrome" => ("LOCALAPPDATA", r"Google\Chrome\User Data"),
            "chromium" => ("LOCALAPPDATA", r"Chromium\User Data"),
            "edge" => ("LOCALAPPDATA", r"Microsoft\Edge\User Data"),
            "opera" => ("APPDATA", r"Opera Software\Opera Stable"),
            "vivaldi" => ("LOCALAPPDATA", r"Vivaldi\User Data"),
            _ => ("LOCALAPPDATA", r"Naver\Naver Whale\User Data"),
        };
        var(base).join(path)
    } else if cfg!(target_os = "macos") {
        home.join("Library/Application Support")
            .join(match browser {
                "brave" => "BraveSoftware/Brave-Browser",
                "chrome" => "Google/Chrome",
                "chromium" => "Chromium",
                "edge" => "Microsoft Edge",
                "opera" => "com.operasoftware.Opera",
                "vivaldi" => "Vivaldi",
                _ => "Naver/Whale",
            })
    } else {
        config_home().join(match browser {
            "brave" => "BraveSoftware/Brave-Browser",
            "chrome" => "google-chrome",
            "chromium" => "chromium",
            "edge" => "microsoft-edge",
            "opera" => "opera",
            "vivaldi" => "vivaldi",
            _ => "naver-whale",
        })
    };
    let mac = cfg!(target_os = "macos");
    let keyring = match browser {
        "brave" => "Brave",
        "chrome" => "Chrome",
        "chromium" => "Chromium",
        "edge" if mac => "Microsoft Edge",
        "opera" if mac => "Opera",
        "vivaldi" if mac => "Vivaldi",
        "edge" | "opera" => "Chromium",
        "vivaldi" => "Chrome",
        _ => "Whale",
    };
    (directory, keyring, browser != "opera")
}

/// Upstream's `ChromeCookieDecryptor` subclasses (Linux and macOS).
struct ChromeDecryptor {
    /// `MacChromeCookieDecryptor` rather than the Linux one.
    mac: bool,
    keyring_name: &'static str,
    keyring: Option<String>,
    hash_prefix: bool,
    /// macOS keychain key, or Linux `v10` (`peanuts`) key.
    v10_key: Option<Vec<u8>>,
    empty_key: Vec<u8>,
    /// Linux `v11` keyring key, derived on first use.
    v11_key: Option<Option<Vec<u8>>>,
    counts: Vec<(&'static str, usize)>,
}

fn derive_key(password: &[u8], iterations: u32) -> Vec<u8> {
    crate::sha1::pbkdf2(password, b"saltysalt", iterations, 16)
}

impl ChromeDecryptor {
    fn new(
        keyring_name: &'static str,
        keyring: Option<String>,
        meta_version: i64,
        out: &mut Extracted,
    ) -> Result<Self> {
        if cfg!(windows) {
            return Err(Error::Reported(
                "decrypting Chromium cookies on Windows (DPAPI) is not supported natively".into(),
            ));
        }
        let mut decryptor = Self::linux(keyring_name, keyring, meta_version);
        if cfg!(target_os = "macos") {
            out.debug("using find-generic-password to obtain password from OSX keychain");
            let service = format!("{keyring_name} Safe Storage");
            let password = match run_command(
                "security",
                &[
                    "find-generic-password",
                    "-w",
                    "-a",
                    keyring_name,
                    "-s",
                    &service,
                ],
            ) {
                Ok((_, code)) if code != 0 => {
                    out.warning("find-generic-password failed");
                    None
                }
                Ok((mut stdout, _)) => {
                    while stdout.last() == Some(&b'\n') {
                        stdout.pop();
                    }
                    Some(stdout)
                }
                Err(error) => {
                    out.warning(format!(
                        "exception running find-generic-password: {}",
                        os_error_text(&error, "security")
                    ));
                    None
                }
            };
            decryptor.make_mac(password.as_deref());
        }
        Ok(decryptor)
    }

    /// `LinuxChromeCookieDecryptor`.
    fn linux(keyring_name: &'static str, keyring: Option<String>, meta_version: i64) -> Self {
        Self {
            mac: false,
            keyring_name,
            keyring,
            hash_prefix: meta_version >= 24,
            v10_key: Some(derive_key(b"peanuts", 1)),
            empty_key: derive_key(b"", 1),
            v11_key: None,
            counts: vec![("v10", 0), ("v11", 0), ("other", 0)],
        }
    }

    /// `MacChromeCookieDecryptor` with the keychain password.
    fn make_mac(&mut self, password: Option<&[u8]>) {
        self.mac = true;
        self.v10_key = password.map(|password| derive_key(password, 1003));
        self.counts = vec![("v10", 0), ("other", 0)];
    }

    fn count(&mut self, version: &str) {
        if let Some(entry) = self.counts.iter_mut().find(|(name, _)| *name == version) {
            entry.1 += 1;
        }
    }

    /// `_decrypt_aes_cbc_multi`.
    fn decrypt_cbc(
        &self,
        ciphertext: &[u8],
        keys: &[&[u8]],
        out: &mut Extracted,
    ) -> Option<String> {
        use aes::cipher::{BlockModeDecrypt, KeyIvInit};
        if ciphertext.is_empty() || !ciphertext.len().is_multiple_of(16) {
            out.warning_once(
                "failed to decrypt cookie (AES-CBC) because UTF-8 decoding failed. Possibly the key is wrong?",
            );
            return None;
        }
        for key in keys {
            let Ok(key) = <&[u8; 16]>::try_from(*key) else {
                continue;
            };
            let mut cipher = cbc::Decryptor::<aes::Aes128>::new(key.into(), &[b' '; 16].into());
            let mut plaintext = ciphertext.to_vec();
            for block in plaintext.as_chunks_mut::<16>().0 {
                let mut array = (*block).into();
                cipher.decrypt_block(&mut array);
                *block = array.into();
            }
            // `unpad_pkcs7`: drop as many bytes as the last one says.
            let pad = usize::from(*plaintext.last().unwrap());
            plaintext.truncate(plaintext.len().saturating_sub(pad));
            let text = if self.hash_prefix {
                plaintext.get(32..).unwrap_or_default()
            } else {
                &plaintext[..]
            };
            if let Ok(text) = std::str::from_utf8(text) {
                return Some(text.to_owned());
            }
        }
        out.warning_once(
            "failed to decrypt cookie (AES-CBC) because UTF-8 decoding failed. Possibly the key is wrong?",
        );
        None
    }

    fn decrypt(&mut self, encrypted: &[u8], out: &mut Extracted) -> Option<String> {
        let (version, ciphertext) = encrypted.split_at(encrypted.len().min(3));
        if self.mac {
            if version == b"v10" {
                self.count("v10");
                let Some(key) = self.v10_key.clone() else {
                    out.warning_once("cannot decrypt v10 cookies: no key found");
                    return None;
                };
                return self.decrypt_cbc(ciphertext, &[&key], out);
            }
            self.count("other");
            // Other prefixes are "old data" stored as plaintext; upstream
            // keeps the bytes, so the cookie's value is their `str()`.
            return Some(crate::batch::py_bytes_repr(encrypted));
        }
        match version {
            b"v10" => {
                self.count("v10");
                let key = self.v10_key.clone().unwrap_or_default();
                let empty = self.empty_key.clone();
                self.decrypt_cbc(ciphertext, &[&key, &empty], out)
            }
            b"v11" => {
                self.count("v11");
                if self.v11_key.is_none() {
                    let password =
                        linux_keyring_password(self.keyring_name, self.keyring.as_deref(), out);
                    self.v11_key = Some(password.map(|password| derive_key(&password, 1)));
                }
                let Some(key) = self.v11_key.clone().flatten() else {
                    out.warning_once("cannot decrypt v11 cookies: no key found");
                    return None;
                };
                let empty = self.empty_key.clone();
                self.decrypt_cbc(ciphertext, &[&key, &empty], out)
            }
            version => {
                out.warning_once(format!(
                    "unknown cookie version: \"{}\"",
                    crate::batch::py_bytes_repr(version)
                ));
                self.count("other");
                None
            }
        }
    }
}

/// `_get_linux_desktop_environment` (as its enum name) over `var`.
fn linux_desktop_environment(
    var: &dyn Fn(&str) -> Option<String>,
    out: &mut Extracted,
) -> &'static str {
    let session = var("DESKTOP_SESSION").unwrap_or_default();
    if let Some(current) = var("XDG_CURRENT_DESKTOP") {
        for part in current.split(':').map(str::trim) {
            match part {
                "Unity" if session.contains("gnome-fallback") => return "GNOME",
                "Unity" => return "UNITY",
                "Deepin" => return "DEEPIN",
                "GNOME" => return "GNOME",
                "X-Cinnamon" => return "CINNAMON",
                "KDE" => {
                    return match var("KDE_SESSION_VERSION").as_deref() {
                        Some("5") => "KDE5",
                        Some("6") => "KDE6",
                        Some("4") => "KDE4",
                        version => {
                            let version = version.map_or("None".into(), str::to_owned);
                            out.info(format!("unknown KDE version: \"{version}\". Assuming KDE4"));
                            "KDE4"
                        }
                    };
                }
                "Pantheon" => return "PANTHEON",
                "XFCE" => return "XFCE",
                "UKUI" => return "UKUI",
                "LXQt" => return "LXQT",
                _ => {}
            }
        }
        out.debug(format!(
            "XDG_CURRENT_DESKTOP is set to an unknown value: \"{current}\""
        ));
    }
    match session.as_str() {
        "deepin" => return "DEEPIN",
        "mate" | "gnome" => return "GNOME",
        "kde4" | "kde-plasma" => return "KDE4",
        "kde" if var("KDE_SESSION_VERSION").is_some() => return "KDE4",
        "kde" => return "KDE3",
        session if session.contains("xfce") || session == "xubuntu" => return "XFCE",
        "ukui" => return "UKUI",
        _ => out.debug(format!(
            "DESKTOP_SESSION is set to an unknown value: \"{session}\""
        )),
    }
    if var("GNOME_DESKTOP_SESSION_ID").is_some() {
        "GNOME"
    } else if var("KDE_FULL_SESSION").is_some() {
        if var("KDE_SESSION_VERSION").is_some() {
            "KDE4"
        } else {
            "KDE3"
        }
    } else {
        "OTHER"
    }
}

/// `_get_linux_keyring_password`: `None` for BASICTEXT.
fn linux_keyring_password(
    keyring_name: &str,
    keyring: Option<&str>,
    out: &mut Extracted,
) -> Option<Vec<u8>> {
    let keyring = match keyring {
        Some(keyring) => keyring.to_owned(),
        None => {
            let environment = linux_desktop_environment(&|name| std::env::var(name).ok(), out);
            out.debug(format!("detected desktop environment: {environment}"));
            match environment {
                "KDE4" => "KWALLET",
                "KDE5" => "KWALLET5",
                "KDE6" => "KWALLET6",
                "KDE3" | "LXQT" | "OTHER" => "BASICTEXT",
                _ => "GNOMEKEYRING",
            }
            .to_owned()
        }
    };
    out.debug(format!("Chosen keyring: {keyring}"));
    match keyring.as_str() {
        "BASICTEXT" => None,
        "GNOMEKEYRING" => {
            out.error(
                "secretstorage not available (GNOME keyring access is not implemented natively)",
            );
            Some(Vec::new())
        }
        _ => Some(kwallet_password(keyring_name, &keyring, out)),
    }
}

/// `_get_kwallet_password` with `_get_kwallet_network_wallet`.
fn kwallet_password(keyring_name: &str, keyring: &str, out: &mut Extracted) -> Vec<u8> {
    out.debug(format!(
        "using kwallet-query to obtain password from {keyring}"
    ));
    let on_path = std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|directory| directory.join("kwallet-query").is_file())
    });
    if !on_path {
        out.error(
            "kwallet-query command not found. KWallet and kwallet-query must be installed to read from KWallet. kwallet-query should beincluded in the kwallet package for your distribution",
        );
        return Vec::new();
    }
    let (service, path) = match keyring {
        "KWALLET" => ("org.kde.kwalletd", "/modules/kwalletd"),
        "KWALLET5" => ("org.kde.kwalletd5", "/modules/kwalletd5"),
        _ => ("org.kde.kwalletd6", "/modules/kwalletd6"),
    };
    let destination = format!("--dest={service}");
    let wallet = match run_command(
        "dbus-send",
        &[
            "--session",
            "--print-reply=literal",
            &destination,
            path,
            "org.kde.KWallet.networkWallet",
        ],
    ) {
        Ok((stdout, 0)) => {
            let wallet = String::from_utf8_lossy(&stdout).trim().to_owned();
            out.debug(format!("NetworkWallet = \"{wallet}\""));
            wallet
        }
        Ok(_) => {
            out.warning("failed to read NetworkWallet");
            "kdewallet".into()
        }
        Err(error) => {
            out.warning(format!(
                "exception while obtaining NetworkWallet: {}",
                os_error_text(&error, "dbus-send")
            ));
            "kdewallet".into()
        }
    };
    let password_name = format!("{keyring_name} Safe Storage");
    let folder = format!("{keyring_name} Keys");
    match run_command(
        "kwallet-query",
        &[
            "--read-password",
            &password_name,
            "--folder",
            &folder,
            &wallet,
        ],
    ) {
        Ok((_, code)) if code != 0 => {
            out.error(format!(
                "kwallet-query failed with return code {code}. Please consult the kwallet-query man page for details"
            ));
            Vec::new()
        }
        Ok((stdout, _)) if stdout.to_ascii_lowercase().starts_with(b"failed to read") => {
            out.debug("failed to read password from kwallet. Using empty string instead");
            Vec::new()
        }
        Ok((mut stdout, _)) => {
            out.debug("password found");
            while stdout.last() == Some(&b'\n') {
                stdout.pop();
            }
            stdout
        }
        Err(error) => {
            out.warning(format!(
                "exception running kwallet-query: {}",
                os_error_text(&error, "kwallet-query")
            ));
            Vec::new()
        }
    }
}

/// Upstream `_extract_chrome_cookies`.
fn chromium(browser: &str, spec: &BrowserSpec, out: &mut Extracted) -> Result<()> {
    chromium_with(browser, spec, out, ChromeDecryptor::new)
}

type MakeDecryptor =
    fn(&'static str, Option<String>, i64, &mut Extracted) -> Result<ChromeDecryptor>;

fn chromium_with(
    browser: &str,
    spec: &BrowserSpec,
    out: &mut Extracted,
    make_decryptor: MakeDecryptor,
) -> Result<()> {
    out.info(format!("Extracting cookies from {browser}"));
    let (mut browser_dir, keyring_name, supports_profiles) = chromium_settings(browser);
    let search_root = match &spec.profile {
        None => browser_dir.clone(),
        Some(profile) if is_path(profile) => {
            let profile = PathBuf::from(profile);
            browser_dir = if supports_profiles {
                profile.parent().map(Path::to_path_buf).unwrap_or_default()
            } else {
                profile.clone()
            };
            profile
        }
        Some(profile) if supports_profiles => browser_dir.join(profile),
        Some(_) => {
            out.error(format!("{browser} does not support profiles"));
            browser_dir.clone()
        }
    };
    let _ = browser_dir;
    let database = newest(find_files(&search_root, "Cookies")).ok_or_else(|| {
        Error::Reported(format!(
            "could not find {browser} cookies database in \"{}\"",
            search_root.display()
        ))
    })?;
    out.debug(format!(
        "Extracting cookies from: \"{}\"",
        database.display()
    ));
    let db = sqlite::Database::open(&database)?;
    let meta = db
        .table("meta")?
        .ok_or_else(|| Error::Cookies("no such table: meta".into()))?;
    let (key, value) = (meta.column("key"), meta.column("value"));
    let meta_version = meta
        .rows
        .iter()
        .find(|row| key.and_then(|key| row[key].as_text()) == Some("version"))
        .and_then(|row| value.map(|value| &row[value]))
        .and_then(|value| match value {
            sqlite::Value::Text(text) => text.trim().parse::<i64>().ok(),
            value => value.as_i64(),
        })
        .ok_or_else(|| Error::Cookies("invalid literal for int() in the meta version".into()))?;
    let mut decryptor = make_decryptor(keyring_name, spec.keyring.clone(), meta_version, out)?;
    let table = db
        .table("cookies")?
        .ok_or_else(|| Error::Cookies("no such table: cookies".into()))?;
    let secure_name = if table.column("is_secure").is_some() {
        "is_secure"
    } else {
        "secure"
    };
    let column = |name: &str| {
        table
            .column(name)
            .ok_or_else(|| Error::Cookies(format!("no such column: {name}")))
    };
    let [host, name, value, encrypted, path, expires, secure] = [
        "host_key",
        "name",
        "value",
        "encrypted_value",
        "path",
        "expires_utc",
        secure_name,
    ]
    .map(column);
    let (host, name, value, encrypted, path, expires, secure) =
        (host?, name?, value?, encrypted?, path?, expires?, secure?);
    let (mut failed, mut unencrypted) = (0, 0);
    for row in &table.rows {
        let text = |index: usize| match &row[index] {
            sqlite::Value::Text(text) => text.clone(),
            sqlite::Value::Blob(bytes) => String::from_utf8_lossy(bytes).into_owned(),
            _ => String::new(),
        };
        let encrypted_value = match &row[encrypted] {
            sqlite::Value::Blob(bytes) => bytes.clone(),
            sqlite::Value::Text(text) => text.clone().into_bytes(),
            _ => Vec::new(),
        };
        let mut cookie_value = text(value);
        let is_encrypted = cookie_value.is_empty() && !encrypted_value.is_empty();
        if is_encrypted {
            match decryptor.decrypt(&encrypted_value, out) {
                Some(decrypted) => cookie_value = decrypted,
                None => {
                    failed += 1;
                    continue;
                }
            }
        } else {
            unencrypted += 1;
        }
        // Session cookies store 0; others keep Chromium's raw timestamp.
        let expires_utc = row[expires].as_f64().filter(|expires| *expires != 0.0);
        out.set_cookie(BrowserCookie {
            domain: text(host),
            name: text(name),
            value: cookie_value,
            path: text(path),
            expires: expires_utc,
            secure: row[secure].truthy(),
        });
    }
    let failed_message = if failed > 0 {
        format!(" ({failed} could not be decrypted)")
    } else {
        String::new()
    };
    out.info(format!(
        "Extracted {} cookies from {browser}{failed_message}",
        out.cookies.len()
    ));
    let mut counts: Vec<String> = decryptor
        .counts
        .iter()
        .map(|(name, count)| format!("'{name}': {count}"))
        .collect();
    counts.push(format!("'unencrypted': {unencrypted}"));
    out.debug(format!(
        "cookie version breakdown: {{{}}}",
        counts.join(", ")
    ));
    Ok(())
}

/// A Safari `binarycookies` parse error (upstream `ParserError`).
fn parser_error(message: impl Into<String>) -> Error {
    Error::Reported(message.into())
}

/// Upstream `DataParser` over a byte slice.
struct DataParser<'a> {
    data: &'a [u8],
    cursor: usize,
}

impl<'a> DataParser<'a> {
    fn read_bytes(&mut self, count: usize) -> Result<&'a [u8]> {
        let end = self.cursor + count;
        let bytes = self
            .data
            .get(self.cursor..end)
            .ok_or_else(|| parser_error("reached end of input"))?;
        self.cursor = end;
        Ok(bytes)
    }
    fn expect_bytes(&mut self, expected: &[u8], message: &str) -> Result<()> {
        let value = self.read_bytes(expected.len())?;
        if value != expected {
            return Err(parser_error(format!(
                "unexpected value: {} != {} ({message})",
                crate::batch::py_bytes_repr(value),
                crate::batch::py_bytes_repr(expected)
            )));
        }
        Ok(())
    }
    fn read_uint(&mut self, big_endian: bool) -> Result<u32> {
        let bytes: [u8; 4] = self.read_bytes(4)?.try_into().unwrap();
        Ok(if big_endian {
            u32::from_be_bytes(bytes)
        } else {
            u32::from_le_bytes(bytes)
        })
    }
    fn read_double(&mut self) -> Result<f64> {
        Ok(f64::from_le_bytes(self.read_bytes(8)?.try_into().unwrap()))
    }
    fn read_cstring(&mut self) -> Result<std::result::Result<String, ()>> {
        let start = self.cursor;
        loop {
            if self.read_bytes(1)? == [0] {
                let bytes = &self.data[start..self.cursor - 1];
                return Ok(String::from_utf8(bytes.to_vec()).map_err(|_| ()));
            }
        }
    }
    fn skip(&mut self, count: isize, description: &str, out: &mut Extracted) -> Result<()> {
        if count > 0 {
            let bytes = self.read_bytes(count as usize)?;
            out.debug(format!(
                "skipping {count} bytes ({description}): {}",
                crate::batch::py_bytes_repr(bytes)
            ));
        } else if count < 0 {
            return Err(parser_error(format!("invalid skip of {count} bytes")));
        }
        Ok(())
    }
    fn skip_to(&mut self, offset: usize, description: &str, out: &mut Extracted) -> Result<()> {
        self.skip(offset as isize - self.cursor as isize, description, out)
    }
    fn skip_to_end(&mut self, description: &str, out: &mut Extracted) -> Result<()> {
        self.skip_to(self.data.len(), description, out)
    }
}

/// `_mac_absolute_time_to_posix`.
fn mac_absolute_time(timestamp: f64) -> f64 {
    (978_307_200.0 + timestamp).trunc()
}

/// `parse_safari_cookies`.
fn parse_safari_cookies(data: &[u8], out: &mut Extracted) -> Result<()> {
    let mut header = DataParser { data, cursor: 0 };
    header.expect_bytes(b"cook", "database signature")?;
    let pages = header.read_uint(true)?;
    let mut sizes = Vec::new();
    for _ in 0..pages {
        sizes.push(header.read_uint(true)? as usize);
    }
    let mut body = DataParser {
        data: &data[header.cursor..],
        cursor: 0,
    };
    for size in sizes {
        let page = body.read_bytes(size)?;
        safari_page(page, out)?;
    }
    body.skip_to_end("footer", out)
}

fn safari_page(data: &[u8], out: &mut Extracted) -> Result<()> {
    let mut page = DataParser { data, cursor: 0 };
    page.expect_bytes(&[0, 0, 1, 0], "page signature")?;
    let count = page.read_uint(false)?;
    let mut offsets = Vec::new();
    for _ in 0..count {
        offsets.push(page.read_uint(false)? as usize);
    }
    if count == 0 {
        out.debug(format!(
            "a cookies page of size {} has no cookies",
            data.len()
        ));
        return Ok(());
    }
    page.skip_to(offsets[0], "unknown page header field", out)?;
    for offset in offsets {
        page.skip_to(offset, "space between records", out)?;
        let length = safari_record(data.get(offset..).unwrap_or_default(), out)?;
        page.read_bytes(length)?;
    }
    page.skip_to_end("space in between pages", out)
}

fn safari_record(data: &[u8], out: &mut Extracted) -> Result<usize> {
    let mut record = DataParser { data, cursor: 0 };
    let size = record.read_uint(false)? as usize;
    record.skip(4, "unknown record field 1", out)?;
    let flags = record.read_uint(false)?;
    record.skip(4, "unknown record field 2", out)?;
    let offsets = [
        record.read_uint(false)? as usize,
        record.read_uint(false)? as usize,
        record.read_uint(false)? as usize,
        record.read_uint(false)? as usize,
    ];
    record.skip(8, "unknown record field 3", out)?;
    let expiration = mac_absolute_time(record.read_double()?);
    let _creation = mac_absolute_time(record.read_double()?);
    let mut fields = Vec::new();
    for offset in offsets {
        record.skip_to(offset, "unknown", out)?;
        match record.read_cstring()? {
            Ok(text) => fields.push(text),
            Err(()) => {
                out.warning_once("failed to parse Safari cookie because UTF-8 decoding failed");
                return Ok(size);
            }
        }
    }
    record.skip_to(size, "space at the end of the record", out)?;
    let [domain, name, path, value]: [String; 4] = fields.try_into().unwrap();
    out.set_cookie(BrowserCookie {
        domain,
        name,
        value,
        path,
        expires: Some(expiration),
        secure: flags & 1 != 0,
    });
    Ok(size)
}

/// Upstream `_extract_safari_cookies`.
fn safari(spec: &BrowserSpec, out: &mut Extracted) -> Result<()> {
    if !cfg!(target_os = "macos") {
        let platform = if cfg!(windows) {
            "win32"
        } else {
            std::env::consts::OS
        };
        return Err(Error::Reported(format!("unsupported platform: {platform}")));
    }
    let path = match &spec.profile {
        Some(profile) => {
            let path = PathBuf::from(crate::batch::expand_path_text(profile));
            if !path.is_file() {
                return Err(Error::Reported(
                    "custom safari cookies database not found".into(),
                ));
            }
            path
        }
        None => {
            let path = home().join("Library/Cookies/Cookies.binarycookies");
            if path.is_file() {
                path
            } else {
                out.debug("Trying secondary cookie location");
                let path = home().join(
                    "Library/Containers/com.apple.Safari/Data/Library/Cookies/Cookies.binarycookies",
                );
                if !path.is_file() {
                    return Err(Error::Reported(
                        "could not find safari cookies database".into(),
                    ));
                }
                path
            }
        }
    };
    parse_safari_cookies(&std::fs::read(path)?, out)?;
    let count = out.cookies.len();
    out.info(format!("Extracted {count} cookies from safari"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Upstream's own `test_cookies.py` vectors.
    #[test]
    fn chrome_decryptors_follow_upstreams_vectors() {
        assert_eq!(
            derive_key(b"abc", 1),
            b"7\xa1\xec\xd4m\xfcA\xc7\xb19Z\xd0\x19\xdcM\x17"
        );
        assert_eq!(
            derive_key(b"abc", 1003),
            b"Y\xe2\xc0\xd0P\xf6\xf4\xe1l\xc1\x8cQ\xcb|\xcdY"
        );
        let mut out = Extracted::default();
        let mut linux = ChromeDecryptor::linux("Chrome", None, 0);
        // `_get_linux_keyring_password` patched to an empty password.
        linux.v11_key = Some(Some(derive_key(b"", 1)));
        assert_eq!(
            linux
                .decrypt(
                    b"v10\xccW%\xcd\xe6\xe6\x9fM\" \xa7\xb0\xca\xe4\x07\xd6",
                    &mut out
                )
                .as_deref(),
            Some("USD")
        );
        assert_eq!(
            linux
                .decrypt(
                    b"v11#\x81\x10>`w\x8f)\xc0\xb2\xc1\r\xf4\x1al\xdd\x93\xfd\xf8\xf8N\xf2\xa9\x83\xf1\xe9o\x0elVQd",
                    &mut out
                )
                .as_deref(),
            Some("tz=Europe.London")
        );
        let mut meta24 = ChromeDecryptor::linux("Chrome", None, 24);
        assert_eq!(
            meta24
                .decrypt(
                    b"v10\x1f\xe4\x0e[\x83\x0c\xcc*kPi \xce\x8d\x1d\xbb\x80\r\x11\t\xbb\x9e^Hy\x94\xf4\x963\x9f\x82\xba\xfe\xa1\xed\xb9\xf1)\x00710\x92\xc8/<\x96B",
                    &mut out
                )
                .as_deref(),
            Some("DE")
        );
        let mut mac = ChromeDecryptor::linux("", None, 0);
        mac.make_mac(Some(b"6eIDUdtKAacvlHwBVwvg/Q=="));
        assert_eq!(
            mac.decrypt(
                b"v10\xb3\xbe\xad\xa1[\x9fC\xa1\x98\xe0\x9a\x01\xd9\xcf\xbfc",
                &mut out
            )
            .as_deref(),
            Some("2021-06-01-22")
        );
        assert!(out.messages.is_empty(), "{:?}", out.messages);
        // Unknown versions warn once.
        assert_eq!(linux.decrypt(b"v12abc", &mut out), None);
        assert_eq!(linux.decrypt(b"v12abc", &mut out), None);
        assert_eq!(
            out.messages,
            [Message::Warning(
                "unknown cookie version: \"b'v12'\"".into()
            )]
        );
    }

    #[test]
    fn linux_desktop_environments_follow_upstreams_table() {
        let cases: &[(&[(&str, &str)], &str)] = &[
            (&[], "OTHER"),
            (&[("DESKTOP_SESSION", "gnome")], "GNOME"),
            (&[("GNOME_DESKTOP_SESSION_ID", "1")], "GNOME"),
            (
                &[("DESKTOP_SESSION", "kde"), ("KDE_SESSION_VERSION", "5")],
                "KDE4",
            ),
            (&[("DESKTOP_SESSION", "kde")], "KDE3"),
            (
                &[("XDG_CURRENT_DESKTOP", "KDE"), ("KDE_SESSION_VERSION", "5")],
                "KDE5",
            ),
            (
                &[("XDG_CURRENT_DESKTOP", "KDE"), ("KDE_SESSION_VERSION", "6")],
                "KDE6",
            ),
            (
                &[
                    ("XDG_CURRENT_DESKTOP", "GNOME:Unity"),
                    ("DESKTOP_SESSION", "gnome-fallback"),
                ],
                "GNOME",
            ),
            (&[("XDG_CURRENT_DESKTOP", "Unity")], "UNITY"),
            (&[("XDG_CURRENT_DESKTOP", "X-Cinnamon")], "CINNAMON"),
            (&[("XDG_CURRENT_DESKTOP", "LXQt")], "LXQT"),
            (&[("DESKTOP_SESSION", "xubuntu")], "XFCE"),
        ];
        for (env, expected) in cases {
            let var = |name: &str| {
                env.iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| (*value).to_owned())
            };
            let mut out = Extracted::default();
            assert_eq!(
                linux_desktop_environment(&var, &mut out),
                *expected,
                "{env:?}"
            );
        }
    }

    /// Upstream's `test_safari_cookie_parsing` sample.
    #[test]
    fn safari_binarycookies_parse_like_upstream() {
        let data = b"cook\x00\x00\x00\x01\x00\x00\x00i\x00\x00\x01\x00\x01\x00\x00\x00\x10\x00\x00\x00\x00\x00\x00\x00Y\
            \x00\x00\x00\x00\x00\x00\x00 \x00\x00\x00\x00\x00\x00\x008\x00\x00\x00B\x00\x00\x00F\x00\x00\x00H\
            \x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x80\x03\xa5>\xc3A\x00\x00\x80\xc3\x07:\xc3A\
            localhost\x00foo\x00/\x00test%20%3Bcookie\x00\x00\x00\x054\x07\x17 \x05\x00\x00\x00Kbplist00\xd1\x01\
            \x02_\x10\x18NSHTTPCookieAcceptPolicy\x10\x02\x08\x0b&\x00\x00\x00\x00\x00\x00\x01\x01\x00\x00\x00\
            \x00\x00\x00\x00\x03\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00(";
        let mut out = Extracted::default();
        parse_safari_cookies(data, &mut out).unwrap();
        assert_eq!(
            out.cookies,
            [BrowserCookie {
                domain: "localhost".into(),
                name: "foo".into(),
                value: "test%20%3Bcookie".into(),
                path: "/".into(),
                // 2021-06-18 21:39:19 UTC.
                expires: Some(1_624_052_359.0),
                secure: false,
            }]
        );
        let expected: Vec<String> = serde_json::from_str(
            r#"["skipping 4 bytes (unknown page header field): b'\\x00\\x00\\x00\\x00'", "skipping 4 bytes (unknown record field 1): b'\\x00\\x00\\x00\\x00'", "skipping 4 bytes (unknown record field 2): b'\\x00\\x00\\x00\\x00'", "skipping 8 bytes (unknown record field 3): b'\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00'", "skipping 87 bytes (footer): b'\\x00\\x00\\x054\\x07\\x17 \\x05\\x00\\x00\\x00Kbplist00\\xd1\\x01\\x02_\\x10\\x18NSHTTPCookieAcceptPolicy\\x10\\x02\\x08\\x0b&\\x00\\x00\\x00\\x00\\x00\\x00\\x01\\x01\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x03\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00('"]"#,
        )
        .unwrap();
        // Upstream's debug lines, captured from the pinned checkout.
        assert_eq!(
            out.messages,
            expected.into_iter().map(Message::Debug).collect::<Vec<_>>()
        );
    }

    /// The saved databases read like upstream's sqlite3 queries.
    #[test]
    fn saved_firefox_profiles_match_upstream() {
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/browser_cookies");
        let expected: serde_json::Value = serde_json::from_str(include_str!(
            "../tests/fixtures/browser_cookies/expected.json"
        ))
        .unwrap();
        let root = std::path::absolute(&fixtures).unwrap();
        for case in expected.as_array().unwrap() {
            let browser = case["browser"].as_str().unwrap_or("firefox");
            let profile = root.join(browser).join(case["profile"].as_str().unwrap());
            let spec = BrowserSpec {
                browser: browser.into(),
                profile: Some(profile.to_string_lossy().into_owned()),
                keyring: None,
                container: case["container"].as_str().map(str::to_owned),
            };
            let extracted = if browser == "chrome" {
                // The keychain password the fixtures were encrypted with.
                fn keychain(
                    name: &'static str,
                    keyring: Option<String>,
                    version: i64,
                    out: &mut Extracted,
                ) -> Result<ChromeDecryptor> {
                    out.debug("using find-generic-password to obtain password from OSX keychain");
                    let mut decryptor = ChromeDecryptor::linux(name, keyring, version);
                    decryptor.make_mac(Some(
                        &std::fs::read(
                            Path::new(env!("CARGO_MANIFEST_DIR"))
                                .join("tests/fixtures/browser_cookies/chrome/password.txt"),
                        )
                        .unwrap(),
                    ));
                    Ok(decryptor)
                }
                let mut extracted = Extracted::default();
                chromium_with("chrome", &spec, &mut extracted, keychain).unwrap();
                extracted
            } else {
                extract(&spec)
            };
            assert_eq!(extracted.error, None, "{case}");
            let mut cookies: Vec<serde_json::Value> = extracted
                .cookies
                .iter()
                .map(|cookie| {
                    serde_json::json!([
                        cookie.domain,
                        cookie.path,
                        cookie.name,
                        cookie.value,
                        cookie.expires.map(|expires| expires as i64),
                        cookie.secure
                    ])
                })
                .collect();
            cookies.sort_by_key(|cookie| cookie.to_string());
            let mut wanted = case["cookies"].as_array().unwrap().clone();
            wanted.sort_by_key(|cookie| cookie.to_string());
            assert_eq!(cookies, wanted, "{}", case["container"]);
            let messages: Vec<serde_json::Value> = extracted
                .messages
                .iter()
                .map(|message| {
                    let (kind, text) = match message {
                        Message::Info(text) => ("info", text),
                        Message::Debug(text) => ("debug", text),
                        Message::Warning(text) => ("warning", text),
                        Message::Error(text) => ("error", text),
                    };
                    // The saved paths use `/`; Windows joins with `\\`.
                    let text = text.replace(&*root.to_string_lossy(), "FIXTURES");
                    serde_json::json!([kind, normalize_separators(&text)])
                })
                .collect();
            assert_eq!(messages, *case["messages"].as_array().unwrap());
        }
    }

    /// `FIXTURES\\a\\b` (a Windows path inside quotes) as `FIXTURES/a/b`.
    fn normalize_separators(text: &str) -> String {
        match text.find("FIXTURES") {
            Some(start) => {
                let end = text[start..]
                    .find('"')
                    .map_or(text.len(), |offset| start + offset);
                format!(
                    "{}{}{}",
                    &text[..start],
                    text[start..end].replace('\\', "/"),
                    &text[end..]
                )
            }
            None => text.to_owned(),
        }
    }

    fn spec(text: &str) -> (String, Option<String>, Option<String>, Option<String>) {
        let spec = BrowserSpec::parse(text).unwrap();
        (spec.browser, spec.keyring, spec.profile, spec.container)
    }

    #[test]
    fn specifications_follow_upstreams_regex() {
        let none = None::<String>;
        let some = |text: &str| Some(text.to_owned());
        assert_eq!(
            spec("Firefox"),
            ("firefox".into(), none.clone(), none.clone(), none.clone())
        );
        assert_eq!(
            spec("chrome+kwallet:Profile 1"),
            (
                "chrome".into(),
                some("KWALLET"),
                some("Profile 1"),
                none.clone()
            )
        );
        assert_eq!(
            spec("firefox::Work"),
            ("firefox".into(), none.clone(), none.clone(), some("Work"))
        );
        assert_eq!(
            spec("firefox: default ::  Work X"),
            (
                "firefox".into(),
                none.clone(),
                some("default"),
                some("Work X")
            )
        );
        assert_eq!(
            BrowserSpec::parse("opera2").unwrap_err(),
            "unsupported browser specified for cookies: \"opera2\". Supported browsers are: brave, chrome, chromium, edge, firefox, opera, safari, vivaldi, whale"
        );
        assert_eq!(
            BrowserSpec::parse("chrome+bogus").unwrap_err(),
            "unsupported keyring specified for cookies: \"BOGUS\". Supported keyrings are: BASICTEXT, GNOMEKEYRING, KWALLET, KWALLET5, KWALLET6"
        );
        assert_eq!(
            BrowserSpec::parse(":x").unwrap_err(),
            "invalid cookies from browser arguments: :x"
        );
    }
}

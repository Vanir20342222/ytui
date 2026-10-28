//! Android seals sessions before atomic persistence. Host tests retain file storage.
use super::*;

pub(crate) struct Stored {
    pub session: Option<Session>,
    pub notice: Option<String>,
}
pub(crate) fn native_jar(session: &Session) -> Result<Arc<yt_dlp_rs::CookieJar>> {
    let extracted = yt_dlp_rs::browser_cookies::Extracted {
        cookies: session
            .cookies
            .iter()
            .map(|c| yt_dlp_rs::browser_cookies::BrowserCookie {
                domain: c.domain.clone(),
                name: c.name.clone(),
                value: c.value.clone(),
                path: c.path.clone(),
                expires: c.expires.map(|n| n as f64),
                secure: c.secure,
            })
            .collect(),
        ..Default::default()
    };
    Ok(Arc::new(yt_dlp_rs::CookieJar::from_browser(
        extracted, None,
    )?))
}
pub(crate) fn clear_transient_auth(cache: &Path) -> Result<()> {
    let dir = cache.join("transient-auth");
    let metadata = match std::fs::symlink_metadata(&dir) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "Unexpected session staging directory"
    );
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("session-")
            && name.ends_with(".cookies.txt")
            && entry.file_type()?.is_file()
        {
            std::fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

#[cfg(target_os = "android")]
fn cleanup_legacy(root: &Path) -> Option<String> {
    match std::fs::remove_file(root.join("account.json")){
        Ok(())=>None,
        Err(e) if e.kind()==std::io::ErrorKind::NotFound=>None,
        Err(_)=>Some("The encrypted account is ready, but its old private plaintext copy could not be removed; disconnect and reconnect it".into()),
    }
}
pub(crate) fn load(root: &Path) -> Result<Stored> {
    #[cfg(not(target_os = "android"))]
    {
        Ok(Stored {
            session: read_json(&root.join("account.json"))?,
            notice: None,
        })
    }
    #[cfg(target_os = "android")]
    {
        use std::io::Read;
        match std::fs::File::open(root.join("account.enc")) {
            Ok(file) => {
                let mut envelope = String::new();
                file.take(3 * 1024 * 1024 + 1)
                    .read_to_string(&mut envelope)?;
                ensure!(
                    envelope.len() <= 3 * 1024 * 1024,
                    "Stored account exceeds limit"
                );
                let text = crate::platform::account_cipher("openAccount", &envelope)?;
                let session = serde_json::from_str(&text)
                    .map_err(|_| anyhow::anyhow!("Stored account is invalid; reconnect it"))?;
                Ok(Stored {
                    session: Some(session),
                    notice: cleanup_legacy(root),
                })
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let session: Option<Session> = read_json(&root.join("account.json"))?;
                let notice = if let Some(session) = &session {
                    save(root, session)?
                } else {
                    None
                };
                Ok(Stored { session, notice })
            }
            Err(e) => Err(e.into()),
        }
    }
}
pub(crate) fn save(root: &Path, session: &Session) -> Result<Option<String>> {
    let text = serde_json::to_string(session)?;
    #[cfg(not(target_os = "android"))]
    {
        atomic_write(&root.join("account.json"), text.as_bytes())?;
        Ok(None)
    }
    #[cfg(target_os = "android")]
    {
        let sealed = crate::platform::account_cipher("sealAccount", &text)?;
        atomic_write(&root.join("account.enc"), sealed.as_bytes())?;
        Ok(cleanup_legacy(root))
    }
}
pub(crate) fn remove(root: &Path) -> Result<()> {
    for name in ["account.enc", "account.json"] {
        match std::fs::remove_file(root.join(name)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    std::fs::File::open(root)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::cookie::CookieStore;
    #[test]
    fn native_auth_stays_in_memory_and_keeps_https_domain_and_path_scoping() {
        use ytui_providers::auth::Cookie;
        let cookie = |domain: &str, path: &str, name: &str| Cookie {
            domain: domain.into(),
            path: path.into(),
            name: name.into(),
            value: "synthetic".into(),
            expires: None,
            secure: true,
            http_only: true,
        };
        let session = Session::new(vec![
            cookie(".youtube.com", "/", "SAPISID"),
            cookie("www.youtube.com", "/feed", "fixture"),
        ])
        .unwrap();
        let jar = native_jar(&session).unwrap();
        for raw in [
            "https://media.test/",
            "https://youtube.com.evil.test/",
            "http://www.youtube.com/",
        ] {
            assert!(jar.cookies(&url::Url::parse(raw).unwrap()).is_none());
        }
        let header = |raw: &str| {
            jar.cookies(&url::Url::parse(raw).unwrap())
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned()
        };
        assert!(header("https://music.youtube.com/").contains("SAPISID=synthetic"));
        assert!(!header("https://music.youtube.com/feed").contains("fixture="));
        assert!(header("https://www.youtube.com/feed/library").contains("fixture=synthetic"));
    }
    #[test]
    fn migration_cleanup_removes_only_owned_regular_staging_files() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("transient-auth");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("session-fixture.cookies.txt"), "synthetic").unwrap();
        std::fs::write(dir.join("keep.txt"), "keep").unwrap();
        std::fs::create_dir(dir.join("session-directory.cookies.txt")).unwrap();
        clear_transient_auth(root.path()).unwrap();
        assert!(!dir.join("session-fixture.cookies.txt").exists());
        assert!(dir.join("keep.txt").is_file());
        assert!(dir.join("session-directory.cookies.txt").is_dir());
        #[cfg(unix)]
        {
            let linked = root.path().join("linked");
            std::fs::create_dir(&linked).unwrap();
            std::os::unix::fs::symlink(&dir, linked.join("transient-auth")).unwrap();
            assert!(clear_transient_auth(&linked).is_err());
        }
    }
}

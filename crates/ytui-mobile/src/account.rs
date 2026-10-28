//! Android seals sessions before atomic persistence. Host tests retain file storage.
use super::*;

pub(crate) struct Stored {
    pub session: Option<Session>,
    pub notice: Option<String>,
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

//! Optional desktop integration. No shell and no clipboard reads unless requested.
use crate::{
    paths::executable,
    process::{self, ProcessSpec},
};
use anyhow::{Context, Result, bail, ensure};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, time::Duration};
use tokio_util::sync::CancellationToken;
use ytui_core::{clean_text, http_url, normalize_media_url};

pub fn local_minute() -> u16 {
    #[cfg(unix)]
    {
        let mut local = std::mem::MaybeUninit::<libc::tm>::uninit();
        let now = unsafe { libc::time(std::ptr::null_mut()) };
        if !unsafe { libc::localtime_r(&now, local.as_mut_ptr()) }.is_null() {
            let local = unsafe { local.assume_init() };
            return (local.tm_hour * 60 + local.tm_min) as u16;
        }
    }
    0
}
/// Explicit user action only. The desktop owns the launched application; it is
/// not a child process that ytui's process-group cleanup could terminate.
pub async fn open_file(path: &std::path::Path) -> Result<()> {
    let file = std::fs::File::open(path)?;
    ensure!(
        file.metadata()?.is_file(),
        "Select a completed regular file"
    );
    tokio::time::timeout(Duration::from_secs(3), async {
        let connection = zbus::Connection::session().await?;
        let portal = zbus::Proxy::new(
            &connection,
            "org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.OpenURI",
        )
        .await?;
        let options: HashMap<&str, zbus::zvariant::Value<'_>> =
            HashMap::from([("ask", zbus::zvariant::Value::Bool(true))]);
        let _: zbus::zvariant::OwnedObjectPath = portal
            .call("OpenFile", &("", zbus::zvariant::Fd::from(&file), options))
            .await?;
        Ok::<_, anyhow::Error>(())
    })
    .await
    .context("Desktop open request timed out")??;
    Ok(())
}

pub fn clipboard_reader() -> Result<ProcessSpec> {
    let (program, args) =
        if std::env::var_os("WAYLAND_DISPLAY").is_some() && executable("wl-paste").is_some() {
            ("wl-paste", vec!["--no-newline", "--type", "text"])
        } else if executable("xclip").is_some() {
            ("xclip", vec!["-selection", "clipboard", "-o"])
        } else if executable("xsel").is_some() {
            ("xsel", vec!["--clipboard", "--output"])
        } else {
            bail!("Clipboard watcher needs wl-clipboard (Wayland) or xclip/xsel (X11)");
        };
    let mut spec = ProcessSpec::new(program, args.into_iter().map(str::to_owned).collect(), 2);
    spec.max_output_bytes = 8192;
    Ok(spec)
}
pub async fn read_clipboard(cancel: CancellationToken) -> Result<String> {
    let result = process::run(clipboard_reader()?, cancel, |_| {}).await?;
    ensure!(
        result.success,
        "Clipboard is unavailable or does not contain text"
    );
    Ok(result.lines.join("\n"))
}
#[derive(Default)]
pub struct ClipboardMonitor {
    previous: Option<[u8; 32]>,
}
impl ClipboardMonitor {
    /// First observation establishes a baseline. Store a hash, never arbitrary clipboard text.
    pub fn observe(&mut self, text: &str) -> Option<String> {
        let digest: [u8; 32] = Sha256::digest(text.as_bytes()).into();
        let old = self.previous.replace(digest)?;
        if old == digest {
            return None;
        }
        let url = normalize_media_url(text.trim()).ok()?;
        let parsed = http_url(&url).ok()?;
        let host = parsed.host_str()?;
        if ![
            "youtube.com",
            "www.youtube.com",
            "m.youtube.com",
            "music.youtube.com",
            "youtu.be",
        ]
        .contains(&host)
        {
            return None;
        }
        Some(url)
    }
}
fn body(text: &str) -> String {
    clean_text(text)
        .chars()
        .take(200)
        .collect::<String>()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
pub async fn notify(title: &str, text: &str, failed: bool, sound: bool) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(3), async {
        let connection = zbus::Connection::session()
            .await
            .context("No desktop session bus")?;
        let proxy = zbus::Proxy::new(
            &connection,
            "org.freedesktop.Notifications",
            "/org/freedesktop/Notifications",
            "org.freedesktop.Notifications",
        )
        .await?;
        let mut hints: HashMap<&str, zbus::zvariant::Value<'_>> = HashMap::new();
        hints.insert(
            "urgency",
            zbus::zvariant::Value::U8(if failed { 1 } else { 0 }),
        );
        hints.insert("suppress-sound", zbus::zvariant::Value::Bool(!sound));
        if sound {
            hints.insert(
                "sound-name",
                zbus::zvariant::Value::from(if failed { "dialog-warning" } else { "complete" }),
            );
        }
        let _: u32 = proxy
            .call(
                "Notify",
                &(
                    "ytui",
                    0u32,
                    "video-x-generic",
                    clean_text(title),
                    body(text),
                    Vec::<&str>::new(),
                    hints,
                    6000i32,
                ),
            )
            .await?;
        Ok(())
    })
    .await
    .context("Desktop notification timed out")?
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clipboard_requires_a_change_and_an_exact_youtube_host() {
        let mut monitor = ClipboardMonitor::default();
        assert!(monitor.observe("private initial text").is_none());
        assert!(
            monitor
                .observe("https://youtube.com.evil.test/watch?v=abcdefghijk")
                .is_none()
        );
        assert!(monitor.observe("https://youtu.be/abcdefghijk").is_some());
        assert!(monitor.observe("https://youtu.be/abcdefghijk").is_none());
        assert!(monitor.observe("arbitrary secret").is_none());
        assert!(monitor.observe("https://youtu.be/abcdefghijk").is_some());
        assert_eq!(
            body("<b>hello & bye</b>\u{1b}"),
            "&lt;b&gt;hello &amp; bye&lt;/b&gt;"
        );
    }
}

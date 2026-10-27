use crate::paths::Paths;
use anyhow::{Context, Result, ensure};
use futures_util::StreamExt;
use std::{path::Path, sync::Arc, time::Duration};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use ytui_core::Config;

pub use ytui_core::theme::{Palette, Rgb, THEMES, builtin, color};
pub use ytui_core::theme::{caelestia, omarchy};
async fn from_file(path: &Path, provider: &str) -> Result<Palette> {
    let data = tokio::fs::read_to_string(path).await?;
    if provider == "caelestia" {
        caelestia(&data)
    } else {
        omarchy(
            &data,
            path.parent().is_some_and(|p| p.join("light.mode").exists()),
        )
    }
}
pub async fn resolve(config: &Config, paths: &Paths) -> Result<Palette> {
    if config.theme != "system" {
        if Path::new(&config.theme).is_file() {
            let mut p = from_file(Path::new(&config.theme), "custom").await?;
            p.source = "Custom palette".into();
            return Ok(p);
        }
        return builtin(&config.theme);
    }
    let state_root = &paths.xdg_state;
    let config_root = paths
        .config
        .parent()
        .context("Config directory has no XDG root")?;
    let ca = state_root.join("caelestia/scheme.json");
    let om = state_root.join("omarchy/current/theme/colors.toml");
    let legacy = config_root.join("omarchy/current/theme/colors.toml");
    let desktop = std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .to_lowercase();
    let provider = config.theme_provider.as_str();
    if provider == "caelestia" {
        return from_file(&ca, "caelestia").await;
    }
    if provider == "omarchy" {
        return from_file(if om.exists() { &om } else { &legacy }, "omarchy").await;
    }
    if provider == "portal" {
        return portal().await;
    }
    if provider == "gnome" {
        return match portal().await {
            Ok(p) => Ok(p),
            Err(_) => gnome_fallback().await,
        };
    }
    ensure!(provider == "auto", "Unknown system theme provider");
    if desktop.contains("gnome") || desktop.contains("kde") {
        if let Ok(p) = portal().await {
            return Ok(p);
        }
        if desktop.contains("gnome")
            && let Ok(p) = gnome_fallback().await
        {
            return Ok(p);
        }
    } else {
        if ca.exists() {
            return from_file(&ca, "caelestia").await;
        }
        if om.exists() || legacy.exists() {
            return from_file(if om.exists() { &om } else { &legacy }, "omarchy").await;
        }
        if let Ok(p) = portal().await {
            return Ok(p);
        }
    }
    let mut p = builtin("ytui-dark")?;
    p.source = "Fallback · no desktop palette available".into();
    Ok(p)
}
/// Poll file contents to cover atomic replacements; wake immediately on portal signals.
pub fn watch(
    config: Arc<Config>,
    paths: Paths,
    cancel: CancellationToken,
) -> watch::Receiver<Palette> {
    let (tx, rx) = watch::channel(builtin("ytui-dark").expect("Valid built-in palette"));
    tokio::spawn(async move {
        let changed = Arc::new(tokio::sync::Notify::new());
        let notify = changed.clone();
        let signal_cancel = cancel.clone();
        let signals = tokio::spawn(async move {
            let connection = zbus::Connection::session().await?;
            let proxy = zbus::Proxy::new(
                &connection,
                "org.freedesktop.portal.Desktop",
                "/org/freedesktop/portal/desktop",
                "org.freedesktop.portal.Settings",
            )
            .await?;
            let mut stream = proxy.receive_signal("SettingChanged").await?;
            loop {
                tokio::select! {_=signal_cancel.cancelled()=>break,message=stream.next()=>{if message.is_none(){break;}notify.notify_one();}}
            }
            Ok::<_, anyhow::Error>(())
        });
        let mut tick = tokio::time::interval(Duration::from_secs(2));
        loop {
            tokio::select! {_=cancel.cancelled()=>break,_=tick.tick()=>{},_=changed.notified()=>{}}
            if let Ok(Ok(p)) =
                tokio::time::timeout(Duration::from_secs(3), resolve(&config, &paths)).await
            {
                let different = *tx.borrow() != p;
                if different {
                    let _ = tx.send(p);
                }
            }
        }
        signals.abort();
        let _ = signals.await;
    });
    rx
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bundled_palettes_keep_text_selection_and_status_roles_readable() {
        fn luminance((r, g, b): Rgb) -> f64 {
            let channel = |v: u8| {
                let v = v as f64 / 255.0;
                if v <= 0.04045 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            channel(r) * 0.2126 + channel(g) * 0.7152 + channel(b) * 0.0722
        }
        let ratio = |a, b| {
            let a = luminance(a);
            let b = luminance(b);
            (a.max(b) + 0.05) / (a.min(b) + 0.05)
        };
        for name in THEMES.iter().filter(|name| **name != "terminal") {
            let p = builtin(name).unwrap();
            for (role, foreground, background, minimum) in [
                ("text", p.text, p.background, 4.5),
                ("muted", p.muted, p.background, 4.5),
                ("selection", p.text, p.selection, 4.5),
                ("accent", p.accent, p.background, 3.0),
                ("error", p.error, p.background, 3.0),
                ("warning", p.warning, p.background, 3.0),
                ("success", p.success, p.background, 3.0),
            ] {
                assert!(
                    ratio(foreground, background) >= minimum,
                    "{name}/{role} contrast {} is below {minimum}",
                    ratio(foreground, background)
                );
            }
        }
    }
    #[test]
    fn parses_caelestia_and_legacy_omarchy_and_rejects_partial_writes() {
        let raw = r##"{"mode":"dark","colours":{"surface":"101010","surfaceContainer":"202020","onSurface":"eeeeee","onSurfaceVariant":"bbbbbb","primary":"88aaff","primaryContainer":"334455","error":"ff6688"}}"##;
        assert_eq!(caelestia(raw).unwrap().accent, (136, 170, 255));
        assert!(caelestia("{}").is_err());
        let old = "background = '#282828'\nforeground = '#eeeeee'\ncolor4 = '#88aaff'\n";
        assert_eq!(omarchy(old, false).unwrap().accent, (136, 170, 255));
        assert!(omarchy("background =", false).is_err());
        for name in THEMES {
            assert!(builtin(name).is_ok());
        }
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;
    #[tokio::test]
    async fn survives_atomic_replacement_and_keeps_last_valid_palette() {
        let t = tempfile::tempdir().unwrap();
        let paths = Paths {
            config_override: None,
            xdg_state: t.path().join("state"),
            config: t.path().join("config/ytui"),
            state: t.path().join("isolated-app-state"),
            cache: t.path().join("cache"),
            legacy_data: t.path().join("data"),
            home: t.path().into(),
        };
        paths.create().unwrap();
        let file = t.path().join("state/caelestia/scheme.json");
        let initial = r#"{"colours":{"surface":"101010","surfaceContainer":"202020","onSurface":"eeeeee","onSurfaceVariant":"bbbbbb","primary":"88aaff","primaryContainer":"334455","error":"ff6688"}}"#;
        crate::paths::atomic_write(&file, initial.as_bytes()).unwrap();
        let config = Config {
            theme_provider: "caelestia".into(),
            ..Default::default()
        };
        let cancel = CancellationToken::new();
        let mut rx = watch(Arc::new(config), paths, cancel.clone());
        tokio::time::timeout(Duration::from_secs(3), rx.changed())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(rx.borrow().accent, (136, 170, 255));
        crate::paths::atomic_write(&file, b"{").unwrap();
        tokio::time::sleep(Duration::from_millis(2100)).await;
        assert_eq!(rx.borrow().accent, (136, 170, 255));
        crate::paths::atomic_write(&file, initial.replace("88aaff", "ffaa88").as_bytes()).unwrap();
        tokio::time::timeout(Duration::from_secs(3), rx.changed())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(rx.borrow().accent, (255, 170, 136));
        cancel.cancel();
    }
}

async fn portal() -> Result<Palette> {
    let connection = zbus::Connection::session().await?;
    let proxy = zbus::Proxy::new(
        &connection,
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.Settings",
    )
    .await?;
    let version: u32 = proxy.get_property("version").await.unwrap_or(1);
    let method = if version >= 2 { "ReadOne" } else { "Read" };
    let value: zbus::zvariant::OwnedValue = proxy
        .call(method, &("org.freedesktop.appearance", "color-scheme"))
        .await?;
    let scheme = u32::try_from(value).context("Portal returned an invalid color-scheme")?;
    let mut p = builtin(if scheme == 2 {
        "ytui-light"
    } else {
        "ytui-dark"
    })?;
    p.source = "Desktop portal".into();
    if let Ok(value) = proxy
        .call::<_, _, zbus::zvariant::OwnedValue>(
            method,
            &("org.freedesktop.appearance", "accent-color"),
        )
        .await
        && let Ok((r, g, b)) = <(f64, f64, f64)>::try_from(value)
    {
        p.accent = (
            (r.clamp(0.0, 1.0) * 255.0) as u8,
            (g.clamp(0.0, 1.0) * 255.0) as u8,
            (b.clamp(0.0, 1.0) * 255.0) as u8,
        );
    }
    Ok(p)
}
async fn gnome_fallback() -> Result<Palette> {
    let result = crate::process::run(
        crate::process::ProcessSpec::new(
            "gsettings",
            vec![
                "get".into(),
                "org.gnome.desktop.interface".into(),
                "color-scheme".into(),
            ],
            2,
        ),
        CancellationToken::new(),
        |_| {},
    )
    .await?;
    ensure!(result.success, "GNOME GSettings unavailable");
    let light = !result.lines.join("").contains("prefer-dark");
    let mut p = builtin(if light { "ytui-light" } else { "ytui-dark" })?;
    p.source = "GNOME GSettings".into();
    Ok(p)
}

use crate::paths::{Paths, atomic_write};
use anyhow::{Result, ensure};
use std::path::PathBuf;
use ytui_core::Config;

fn file(paths: &Paths, name: &str) -> Result<PathBuf> {
    ensure!(
        !name.is_empty()
            && name.chars().count() <= 80
            && !name.contains("..")
            && !name.chars().any(|c| c.is_control() || "/\\".contains(c)),
        "Use a profile name without slashes or '..'"
    );
    Ok(paths
        .config
        .join("rust-profiles")
        .join(format!("{name}.toml")))
}
pub fn list(paths: &Paths) -> Result<Vec<String>> {
    let dir = paths.config.join("rust-profiles");
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut names = std::fs::read_dir(dir)?
        .filter_map(|e| {
            let entry = e.ok()?;
            let path = entry.path();
            (path.extension()? == "toml")
                .then(|| path.file_stem()?.to_str().map(str::to_owned))
                .flatten()
        })
        .collect::<Vec<_>>();
    names.sort();
    Ok(names)
}
pub fn save(paths: &Paths, name: &str, config: &Config) -> Result<()> {
    config.validate()?;
    atomic_write(
        &file(paths, name)?,
        toml::to_string_pretty(config)?.as_bytes(),
    )
}
pub fn load(paths: &Paths, name: &str) -> Result<Config> {
    let mut config: Config = toml::from_str(&std::fs::read_to_string(file(paths, name)?)?)?;
    paths.fill_directories(&mut config);
    config.validate()?;
    Ok(config)
}
pub fn delete(paths: &Paths, name: &str) -> Result<()> {
    Ok(std::fs::remove_file(file(paths, name)?)?)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profiles_roundtrip_keymaps_and_unmapped_legacy_settings() {
        let temp = tempfile::tempdir().unwrap();
        let paths = Paths {
            config_override: None,
            xdg_state: temp.path().join("state"),
            config: temp.path().join("config"),
            state: temp.path().join("state"),
            cache: temp.path().join("cache"),
            legacy_data: temp.path().join("legacy"),
            home: temp.path().into(),
        };
        let mut config = Config {
            legacy_settings: Some("{\"future_setting\":42}".into()),
            ..Default::default()
        };
        config.keybindings.insert("f8".into(), "preview".into());
        save(&paths, "Music 日本語", &config).unwrap();
        let restored = load(&paths, "Music 日本語").unwrap();
        assert_eq!(restored.keybindings, config.keybindings);
        assert_eq!(restored.legacy_settings, config.legacy_settings);
        assert_eq!(list(&paths).unwrap(), vec!["Music 日本語"]);
        assert!(save(&paths, "../escape", &config).is_err());
        delete(&paths, "Music 日本語").unwrap();
        assert!(list(&paths).unwrap().is_empty());
    }
}

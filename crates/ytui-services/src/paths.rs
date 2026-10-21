use anyhow::{Context, Result};
use std::{
    fs,
    path::{Path, PathBuf},
};
use ytui_core::Config;

#[derive(Clone, Debug)]
pub struct Paths {
    pub config_override: Option<PathBuf>,
    pub xdg_state: PathBuf,
    pub config: PathBuf,
    pub state: PathBuf,
    pub cache: PathBuf,
    pub legacy_data: PathBuf,
    pub home: PathBuf,
}
impl Paths {
    pub fn discover() -> Result<Self> {
        let dirs = directories::BaseDirs::new().context("Cannot find home directory")?;
        Ok(Self {
            config_override: None,
            xdg_state: dirs.state_dir().unwrap_or(dirs.data_local_dir()).into(),
            config: dirs.config_dir().join("ytui"),
            state: dirs
                .state_dir()
                .unwrap_or(dirs.data_local_dir())
                .join("ytui/rust"),
            cache: dirs.cache_dir().join("ytui/rust"),
            legacy_data: dirs.data_local_dir().join("ytui"),
            home: dirs.home_dir().into(),
        })
    }
    pub fn create(&self) -> Result<()> {
        for dir in [&self.config, &self.state, &self.cache] {
            fs::create_dir_all(dir)?;
        }
        Ok(())
    }
    pub fn config_file(&self) -> PathBuf {
        self.config_override
            .clone()
            .unwrap_or_else(|| self.config.join("rust.toml"))
    }
    pub fn load_config(&self) -> Result<Config> {
        let mut c: Config = if self.config_file().exists() {
            toml::from_str(&fs::read_to_string(self.config_file())?)
                .context("Invalid rust.toml; fix it or use --config with a different file")?
        } else {
            Config::default()
        };
        self.fill_directories(&mut c);
        c.validate()?;
        Ok(c)
    }
    pub fn fill_directories(&self, c: &mut Config) {
        if c.download.directory.as_os_str().is_empty() {
            c.download.directory = self.home.join("Videos/ytui");
        }
        if c.download.audio_directory.as_os_str().is_empty() {
            c.download.audio_directory = self.home.join("Music/ytui");
        }
    }
    pub fn save_config(&self, c: &Config) -> Result<()> {
        c.validate()?;
        atomic_write(&self.config_file(), toml::to_string_pretty(c)?.as_bytes())
    }
}
/// Same-directory rename, private temporary file from creation, and durable flush.
pub fn atomic_write(path: &Path, data: &[u8]) -> Result<()> {
    use std::io::Write;
    let parent = path.parent().context("Output has no parent directory")?;
    fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(data)?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|e| e.error)?;
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}
pub fn private_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Resolve tools for upstream libraries that require an explicit executable path.
pub fn executable(program: &str) -> Option<PathBuf> {
    let path = PathBuf::from(program);
    if path.components().count() > 1 {
        return path.is_file().then_some(path);
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(program))
            .find(|p| p.is_file())
    })
}

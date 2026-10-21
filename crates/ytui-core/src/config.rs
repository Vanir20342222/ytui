use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub version: u32,
    #[serde(skip)]
    pub offline: bool,
    pub theme: String,
    pub theme_provider: String,
    pub image_protocol: String,
    pub thumbnails: bool,
    pub reduced_motion: bool,
    pub playback_history: bool,
    pub download: DownloadOptions,
    pub concurrency: usize,
    pub max_retries: u32,
    pub retry_delay_seconds: u64,
    pub proxy: Option<String>,
    pub download_backend: String,
    pub native_js_runtime: Option<PathBuf>,
    pub yt_dlp: String,
    pub ffmpeg: String,
    pub ffprobe: String,
    pub mpv: String,
    pub cliamp: String,
    pub nmcli: String,
    pub radio_pinned_countries: Vec<String>,
    pub auth_store: String,
    pub account_index: u32,
    pub keybindings: BTreeMap<String, String>,
    pub radio_servers: Vec<String>,
    pub custom_stations: Vec<crate::Media>,
    pub clipboard_watcher: bool,
    pub notifications: bool,
    pub notification_sound: bool,
    pub schedule: crate::schedule::Schedule,
    pub legacy_settings: Option<String>,
    pub legacy_keybindings: BTreeMap<String, String>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            offline: false,
            theme: "system".into(),
            theme_provider: "auto".into(),
            image_protocol: "auto".into(),
            thumbnails: true,
            reduced_motion: false,
            playback_history: true,
            download: DownloadOptions::default(),
            concurrency: 3,
            max_retries: 3,
            retry_delay_seconds: 5,
            proxy: None,
            download_backend: "native".into(),
            native_js_runtime: None,
            yt_dlp: "yt-dlp".into(),
            ffmpeg: "ffmpeg".into(),
            ffprobe: "ffprobe".into(),
            mpv: "mpv".into(),
            cliamp: "cliamp".into(),
            nmcli: "nmcli".into(),
            radio_pinned_countries: Vec::new(),
            auth_store: "keyring".into(),
            account_index: 0,
            keybindings: BTreeMap::new(),
            radio_servers: Vec::new(),
            custom_stations: Vec::new(),
            clipboard_watcher: false,
            notifications: true,
            notification_sound: false,
            schedule: Default::default(),
            legacy_settings: None,
            legacy_keybindings: BTreeMap::new(),
        }
    }
}
impl Config {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1,
            "Unsupported configuration version {}",
            self.version
        );
        ensure!(
            (1..=16).contains(&self.concurrency),
            "concurrency must be between 1 and 16"
        );
        ensure!(self.max_retries <= 10, "max_retries must be at most 10");
        ensure!(
            self.retry_delay_seconds <= 300,
            "retry_delay_seconds must be at most 300"
        );
        ensure!(
            ["keyring", "file"].contains(&self.auth_store.as_str()),
            "auth_store must be keyring or file"
        );
        for (key, action) in &self.keybindings {
            ensure!(
                key != "ctrl+c" || action == "quit",
                "Ctrl+C is reserved for emergency quit"
            );
            ensure!(
                crate::shortcuts::valid_key(key)
                    && crate::shortcuts::Action::parse(action).is_some(),
                "Invalid keybinding {key}: {action}"
            );
        }
        ensure!(
            ["native", "yt-dlp"].contains(&self.download_backend.as_str()),
            "download_backend must be native or yt-dlp"
        );
        ensure!(
            ["auto", "caelestia", "omarchy", "portal", "gnome"]
                .contains(&self.theme_provider.as_str()),
            "Unknown theme provider"
        );
        ensure!(
            ["auto", "kitty", "sixel", "iterm2", "halfblocks", "none"]
                .contains(&self.image_protocol.as_str()),
            "Unknown image protocol"
        );
        ensure!(
            self.radio_pinned_countries
                .iter()
                .all(|c| c.len() == 2 && c.bytes().all(|b| b.is_ascii_alphabetic())),
            "Pinned radio countries must use two-letter codes"
        );
        self.schedule.validate()?;
        self.download.validate()
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct DownloadOptions {
    pub backend: Option<String>,
    pub audio: bool,
    pub quality: String,
    pub container: String,
    pub audio_container: String,
    pub audio_quality: String,
    pub directory: PathBuf,
    pub audio_directory: PathBuf,
    pub filename_template: String,
    pub subtitles: Vec<String>,
    pub auto_subtitles: bool,
    pub embed_subtitles: bool,
    pub metadata: bool,
    pub thumbnail: bool,
    pub playlist: bool,
    pub playlist_items: Option<String>,
    pub bandwidth_kib: u64,
    pub global_bandwidth_kib: u64,
    pub sponsorblock: Vec<String>,
    pub sponsorblock_api: String,
    pub sponsorblock_remove: bool,
    pub split_chapters: bool,
    pub skip_duplicates: bool,
    pub post_download_hook: Option<PathBuf>,
}
impl Default for DownloadOptions {
    fn default() -> Self {
        Self {
            backend: None,
            audio: false,
            quality: "1080".into(),
            container: "mp4".into(),
            audio_container: "mp3".into(),
            audio_quality: "0".into(),
            directory: PathBuf::new(),
            audio_directory: PathBuf::new(),
            filename_template: "%(title).180s [%(id)s].%(ext)s".into(),
            subtitles: Vec::new(),
            auto_subtitles: false,
            embed_subtitles: true,
            metadata: true,
            thumbnail: false,
            playlist: false,
            playlist_items: None,
            bandwidth_kib: 0,
            global_bandwidth_kib: 0,
            sponsorblock: Vec::new(),
            sponsorblock_api: "https://sponsor.ajay.app".into(),
            sponsorblock_remove: false,
            split_chapters: false,
            skip_duplicates: true,
            post_download_hook: None,
        }
    }
}
impl DownloadOptions {
    pub fn output_directory(&self) -> &std::path::Path {
        if self.audio {
            &self.audio_directory
        } else {
            &self.directory
        }
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.backend
                .as_deref()
                .is_none_or(|b| b == "native" || b == "yt-dlp"),
            "Unknown download backend"
        );
        ensure!(
            self.quality == "best"
                || self
                    .quality
                    .parse::<u32>()
                    .is_ok_and(|v| (144..=8640).contains(&v)),
            "quality must be best or a resolution height"
        );
        ensure!(
            ["mp4", "mkv", "webm"].contains(&self.container.as_str()),
            "Unsupported video container"
        );
        ensure!(
            ["mp3", "m4a", "flac", "opus", "vorbis", "wav", "aac"]
                .contains(&self.audio_container.as_str()),
            "Unsupported audio container"
        );
        ensure!(
            !self.filename_template.contains('\0')
                && !self.filename_template.starts_with('/')
                && !self.filename_template.split('/').any(|p| p == ".."),
            "filename_template must stay within the destination"
        );
        ensure!(
            !self.filename_template.is_empty(),
            "filename_template cannot be empty"
        );
        let quality = self
            .audio_quality
            .trim_end_matches(['k', 'K'])
            .parse::<f64>()
            .map_err(|_| {
                anyhow::anyhow!("Audio quality must be 0..10 or a bitrate such as 320K")
            })?;
        ensure!(
            quality.is_finite() && (0.0..=1_000_000.0).contains(&quality),
            "Audio quality must be finite and between 0 and 1000000"
        );
        crate::http_url(&self.sponsorblock_api)?;
        ensure!(
            self.sponsorblock.iter().all(|s| [
                "sponsor",
                "intro",
                "outro",
                "selfpromo",
                "interaction",
                "preview",
                "music_offtopic",
                "filler"
            ]
            .contains(&s.as_str())),
            "Unsupported SponsorBlock category"
        );
        if let Some(items) = &self.playlist_items {
            ensure!(
                !items.is_empty()
                    && items
                        .chars()
                        .all(|c| c.is_ascii_digit() || ",:-".contains(c)),
                "Playlist selection must use indices/ranges, e.g. 1,3:7"
            );
        }
        Ok(())
    }
}

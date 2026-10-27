use crate::editor::Editor;
use anyhow::{Result, ensure};
use ytui_core::{Config, DownloadOptions, Media};
#[derive(Clone)]
pub struct Field {
    pub label: &'static str,
    pub value: Editor,
    pub toggle: bool,
    pub choices: &'static [&'static str],
}
impl Field {
    fn text(label: &'static str, value: impl ToString) -> Self {
        Self {
            label,
            value: Editor::new(value.to_string()),
            toggle: false,
            choices: &[],
        }
    }
    fn flag(label: &'static str, value: bool) -> Self {
        Self {
            label,
            value: Editor::new(if value { "yes" } else { "no" }),
            toggle: true,
            choices: &[],
        }
    }
    fn choice(label: &'static str, value: impl ToString, choices: &'static [&'static str]) -> Self {
        Self {
            choices,
            ..Self::text(label, value)
        }
    }
}
pub const SETTINGS_SECTIONS: &[(&str, &[usize])] = &[
    ("General", &[54, 15, 16, 17, 34, 38, 50]),
    ("Appearance", &[0, 1, 2, 3, 55]),
    (
        "Downloads",
        &[39, 40, 41, 42, 37, 43, 44, 4, 5, 6, 45, 21, 32, 33, 30],
    ),
    ("Subtitles & media", &[22, 23, 24, 25, 26, 27, 28, 29, 36]),
    ("Network & account", &[12, 10, 11]),
    ("Schedule", &[18, 19, 20]),
    ("Tools", &[13, 14, 7, 8, 9, 35, 46, 47, 31]),
    ("Radio", &[48, 49]),
    ("Playback", &[53, 51, 52]),
];
pub enum FormKind {
    Download(Media, DownloadOptions),
    Settings(Config),
    Auth,
    Action {
        title: &'static str,
        command: &'static str,
        media_url: Option<String>,
    },
    Keys(Config),
}
pub struct Form {
    pub kind: FormKind,
    pub fields: Vec<Field>,
    pub selected: usize,
    pub error: Option<String>,
    pub section: usize,
    pub sections_focused: bool,
}
pub const DOWNLOAD_SECTIONS: &[(&str, &[usize])] = &[
    ("Quality & format", &[0, 1, 2, 3, 13, 12, 14]),
    ("Files & playlist", &[4, 5, 6, 11]),
    ("Captions & metadata", &[7, 8, 9, 10]),
];
impl Form {
    pub fn explanation(&self) -> Option<&'static str> {
        Some(match self.fields.get(self.selected)?.label {
            "Duration (e.g. 30m)" => {
                "Use 30m, 1h or 90s, up to 24 hours. Stops playback and queue advance without quitting ytui. Pausing or changing tracks does not reset the timer; restarting ytui clears it."
            }
            "Language" => {
                "en = English; de = Deutsch; system follows LC_ALL, LC_MESSAGES or LANG. Saved changes apply immediately. Tool/provider errors keep their original text."
            }
            "Video quality" | "Default video quality" => {
                "Maximum height in pixels, not a guarantee the source offers it. Actual formats are in Actions → Available formats."
            }
            "Audio quality / bitrate" => {
                "Use 128K/320K for a target bitrate, or 0–10 VBR quality (0 is best). Lossless formats ignore bitrate. Applies to this/new jobs."
            }
            "Destination" | "Video directory" | "Audio directory" => {
                "Directory for this/new jobs. Existing files and queued job destinations are never moved by changing a default."
            }
            "Playlist indices" => {
                "One-based available entries, e.g. 1,3:7. Use whole-collection individual jobs for persistent entry status/retry."
            }
            "Subtitle languages" => {
                "Comma-separated language codes (de,en). Available captions depend on the extractor; automatic captions require the separate toggle."
            }
            "Backend (blank=default)" | "Download backend" => {
                "Native stays in Rust; Auto can invoke installed external yt-dlp for supported limitations. Playback has its own backend setting."
            }
            "Playback backend" => {
                "Native never starts Python yt-dlp. Auto permits fallback. Changes apply to the next playback; tools are never installed automatically."
            }
            "Image protocol" => {
                "Auto uses detected terminal capability. Saved changes rebuild visible cards immediately; Ghostty supports Kitty graphics."
            }
            "Auth storage" => {
                "Keyring needs an unlocked desktop keyring. File is explicit private local storage. Browser cookies are imported only after confirmation."
            }
            "Account index" => {
                "0 selects the primary YouTube account; other indices select available brand channels. Changing a connected account requires validation."
            }
            "Proxy" | "Proxy (optional)" => {
                "HTTP(S) or SOCKS5 URL. Browsing/downloads support remote SOCKS DNS; mpv requires HTTP(S) or OS VPN routing. Credentials stay out of support exports."
            }
            "Limit KiB/s (0=off)" => {
                "Per-job transfer cap in KiB/s; 0 disables this cap. Global/schedule caps may also apply."
            }
            _ => return None,
        })
    }
    pub fn is_grouped(&self) -> bool {
        matches!(self.kind, FormKind::Settings(_) | FormKind::Download(..))
    }
    pub fn sections(&self) -> &'static [(&'static str, &'static [usize])] {
        if matches!(self.kind, FormKind::Download(..)) {
            DOWNLOAD_SECTIONS
        } else {
            SETTINGS_SECTIONS
        }
    }

    pub fn action(entry: &crate::actions::Entry) -> Self {
        Self {
            kind: FormKind::Action {
                title: entry.title,
                command: entry.command,
                media_url: None,
            },
            fields: entry
                .fields
                .iter()
                .map(|(name, default)| Field::text(name, default))
                .collect(),
            selected: 0,
            error: None,
            section: 0,
            sections_focused: false,
        }
    }

    pub fn download(media: Media, mut options: DownloadOptions) -> Self {
        options.playlist |=
            ytui_core::is_playlist_url(&media.url) || options.playlist_items.is_some();
        let o = &options;
        let fields = vec![
            Field::flag("Audio only", o.audio),
            Field::choice(
                "Video quality",
                &o.quality,
                &[
                    "144", "240", "360", "480", "720", "1080", "1440", "2160", "best",
                ],
            ),
            Field::choice("Video container", &o.container, &["mp4", "mkv", "webm"]),
            Field::choice(
                "Audio format",
                &o.audio_container,
                &["mp3", "m4a", "flac", "opus", "wav", "aac", "vorbis"],
            ),
            Field::text("Destination", o.output_directory().display()),
            Field::flag("Whole playlist", o.playlist),
            Field::text(
                "Playlist indices",
                o.playlist_items.clone().unwrap_or_default(),
            ),
            Field::text("Subtitle languages", o.subtitles.join(",")),
            Field::flag("Automatic subtitles", o.auto_subtitles),
            Field::flag("Embed metadata", o.metadata),
            Field::flag("Embed thumbnail", o.thumbnail),
            Field::text("Limit KiB/s (0=off)", o.bandwidth_kib),
            Field::choice(
                "Backend (blank=default)",
                o.backend.clone().unwrap_or_default(),
                &["", "auto", "native", "yt-dlp"],
            ),
            Field::text("Audio quality / bitrate", &o.audio_quality),
            Field::text(
                "Exact format selector (optional)",
                o.format_selector.as_deref().unwrap_or(""),
            ),
        ];
        Self {
            kind: FormKind::Download(media, options),
            fields,
            selected: 0,
            error: None,
            section: 0,
            sections_focused: false,
        }
    }
    pub fn settings(config: Config) -> Self {
        let c = &config;
        let fields = vec![
            Field::choice("Theme", &c.theme, ytui_services::theme::THEMES),
            Field::choice(
                "Theme provider",
                &c.theme_provider,
                &["auto", "caelestia", "omarchy", "portal", "gnome"],
            ),
            Field::choice(
                "Image protocol",
                &c.image_protocol,
                &["auto", "kitty", "sixel", "iterm2", "halfblocks", "none"],
            ),
            Field::flag("Thumbnails", c.thumbnails),
            Field::text("Concurrent downloads", c.concurrency),
            Field::text("Video directory", c.download.directory.display()),
            Field::text("Audio directory", c.download.audio_directory.display()),
            Field::text("yt-dlp executable", &c.yt_dlp),
            Field::text("mpv executable", &c.mpv),
            Field::text("FFmpeg executable", &c.ffmpeg),
            Field::choice("Auth storage", &c.auth_store, &["keyring", "file"]),
            Field::text("Account index", c.account_index),
            Field::text("Proxy (optional)", c.proxy.clone().unwrap_or_default()),
            Field::choice(
                "Download backend",
                &c.download_backend,
                &["auto", "native", "yt-dlp"],
            ),
            Field::text(
                "Native JS runtime (blank = auto Node/Deno)",
                c.native_js_runtime
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
            ),
            Field::flag("Clipboard watcher (opt-in)", c.clipboard_watcher),
            Field::flag("Desktop notifications", c.notifications),
            Field::flag("Notification sounds", c.notification_sound),
            Field::text(
                "Download window",
                c.schedule.download_window.clone().unwrap_or_default(),
            ),
            Field::text(
                "Bandwidth window",
                c.schedule.bandwidth_window.clone().unwrap_or_default(),
            ),
            Field::text("Window total KiB/s", c.schedule.window_bandwidth_kib),
            Field::text("Normal total KiB/s", c.download.global_bandwidth_kib),
            Field::text("Subtitle languages", c.download.subtitles.join(",")),
            Field::flag("Automatic subtitles", c.download.auto_subtitles),
            Field::flag("Embed subtitles", c.download.embed_subtitles),
            Field::flag("Embed metadata", c.download.metadata),
            Field::flag("Embed thumbnail", c.download.thumbnail),
            Field::text("SponsorBlock categories", c.download.sponsorblock.join(",")),
            Field::flag("SponsorBlock remove", c.download.sponsorblock_remove),
            Field::flag("Split chapters", c.download.split_chapters),
            Field::text("Filename template", &c.download.filename_template),
            Field::text(
                "Hook executable (opt-in)",
                c.download
                    .post_download_hook
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
            ),
            Field::text("Retry count", c.max_retries),
            Field::text("Retry delay seconds", c.retry_delay_seconds),
            Field::flag("Save local playback positions", c.playback_history),
            Field::text("FFprobe executable", &c.ffprobe),
            Field::text("SponsorBlock API", &c.download.sponsorblock_api),
            Field::text("Audio quality / bitrate", &c.download.audio_quality),
            Field::flag("Reduced motion", c.reduced_motion),
            Field::flag("Audio only by default", c.download.audio),
            Field::text("Default video quality", &c.download.quality),
            Field::choice(
                "Default video container",
                &c.download.container,
                &["mp4", "mkv", "webm"],
            ),
            Field::choice(
                "Default audio format",
                &c.download.audio_container,
                &["mp3", "m4a", "flac", "opus", "vorbis", "wav", "aac"],
            ),
            Field::flag("Whole playlists by default", c.download.playlist),
            Field::flag("Skip duplicate downloads", c.download.skip_duplicates),
            Field::text("Per-download KiB/s", c.download.bandwidth_kib),
            Field::text("cliamp executable", &c.cliamp),
            Field::text("NetworkManager executable", &c.nmcli),
            Field::text("Pinned countries", c.radio_pinned_countries.join(",")),
            Field::text("Directory servers", c.radio_servers.join(",")),
            Field::flag("Show shortcut hints (F1)", c.show_hints),
            Field::choice(
                "SponsorBlock playback",
                &c.playback_sponsorblock,
                &["skip", "mark", "off"],
            ),
            Field::text(
                "Playback skip categories",
                c.playback_sponsor_categories.join(","),
            ),
            Field::choice(
                "Playback backend",
                &c.playback_backend,
                &["native", "auto", "yt-dlp"],
            ),
            Field::choice("Language", &c.language, &["en", "de", "system"]),
            Field::choice(
                "Music player style",
                &c.music_player_style,
                ytui_core::MUSIC_PLAYER_STYLES,
            ),
        ];
        Self {
            kind: FormKind::Settings(config),
            fields,
            selected: SETTINGS_SECTIONS[0].1[0],
            error: None,
            section: 0,
            sections_focused: false,
        }
    }
    pub fn keys(config: Config) -> Self {
        let fields = ytui_core::shortcuts::REGISTRY
            .iter()
            .map(|(default, _, action, description)| {
                let mapped = config
                    .keybindings
                    .iter()
                    .filter(|(_, value)| value.as_str() == *action)
                    .map(|(key, _)| key.as_str())
                    .collect::<Vec<_>>();
                Field::text(
                    description,
                    if mapped.is_empty() {
                        if config.keybindings.contains_key(*default) {
                            format!("alt+{default}")
                        } else {
                            default.to_string()
                        }
                    } else {
                        mapped.join(" ")
                    },
                )
            })
            .collect();
        Self {
            kind: FormKind::Keys(config),
            fields,
            selected: 0,
            error: None,
            section: 0,
            sections_focused: false,
        }
    }
    pub fn key_values(&self) -> Result<Config> {
        let FormKind::Keys(base) = &self.kind else {
            anyhow::bail!("Not a keybinding form")
        };
        let mut config = base.clone();
        config.keybindings.clear();
        for (index, (_, _, action, _)) in ytui_core::shortcuts::REGISTRY.iter().enumerate() {
            ensure!(
                !self.value(index).is_empty(),
                "Each action needs at least one key"
            );
            for key in self.value(index).split_whitespace() {
                ensure!(ytui_core::shortcuts::valid_key(key), "Unknown key {key}");
                ensure!(
                    !config.keybindings.contains_key(key),
                    "Key {key} is assigned more than once"
                );
                config.keybindings.insert(key.into(), (*action).into());
            }
        }
        config.validate()?;
        Ok(config)
    }
    pub fn auth() -> Self {
        Self {
            kind: FormKind::Auth,
            fields: vec![
                Field::text("Browser", "brave-origin"),
                Field::text("Profile path", ""),
                Field::text("Netscape file (alternative)", ""),
            ],
            selected: 0,
            error: None,
            section: 0,
            sections_focused: false,
        }
    }
    pub fn title(&self) -> &str {
        match &self.kind {
            FormKind::Download(_, _) => "Download",
            FormKind::Settings(_) => "General settings",
            FormKind::Action { title, .. } => title,
            FormKind::Auth => "Connect your YouTube account",
            FormKind::Keys(_) => "Keyboard shortcuts",
        }
    }
    pub fn value(&self, index: usize) -> &str {
        self.fields[index].value.text.trim()
    }
    pub fn select_section(&mut self, section: usize) {
        self.section = section.min(self.sections().len() - 1);
        self.selected = self.sections()[self.section].1[0];
    }
    pub fn move_field(&mut self, delta: isize) {
        if self.is_grouped() {
            let indices = self.sections()[self.section].1;
            let current = indices
                .iter()
                .position(|i| *i == self.selected)
                .unwrap_or(0);
            self.selected = indices[current.saturating_add_signed(delta).min(indices.len() - 1)];
        } else {
            self.selected = self
                .selected
                .saturating_add_signed(delta)
                .min(self.fields.len() - 1);
        }
    }
    pub fn cycle_choice(&mut self, delta: isize) {
        let field = &mut self.fields[self.selected];
        if field.toggle {
            self.toggle();
            return;
        }
        if !field.choices.is_empty() {
            let current = field
                .choices
                .iter()
                .position(|v| *v == field.value.text)
                .unwrap_or(0);
            let index =
                (current as isize + delta).rem_euclid(field.choices.len() as isize) as usize;
            field.value = Editor::new(field.choices[index]);
        }
    }
    pub fn toggle(&mut self) {
        if self.selected == 0
            && let FormKind::Download(_, base) = &self.kind
        {
            let current = self.value(0) == "yes";
            let old = if current {
                &base.audio_directory
            } else {
                &base.directory
            };
            let new = if current {
                &base.directory
            } else {
                &base.audio_directory
            };
            if self.value(4) == old.to_string_lossy() {
                self.fields[4].value = Editor::new(new.to_string_lossy());
            }
        }
        if self.fields[self.selected].toggle {
            let v = self.value(self.selected) != "yes";
            self.fields[self.selected].value = Editor::new(if v { "yes" } else { "no" });
        }
    }
    pub fn download_values(&self) -> Result<(Media, DownloadOptions)> {
        let FormKind::Download(media, base) = &self.kind else {
            anyhow::bail!("Not a download form")
        };
        let mut o = base.clone();
        o.audio = self.value(0) == "yes";
        o.quality = self.value(1).into();
        o.container = self.value(2).into();
        o.audio_container = self.value(3).into();
        ensure!(!self.value(4).is_empty(), "Choose a destination directory");
        if o.audio {
            o.audio_directory = self.value(4).into();
        } else {
            o.directory = self.value(4).into();
        }
        o.playlist = self.value(5) == "yes";
        o.playlist_items = if self.value(6).is_empty() {
            None
        } else {
            Some(self.value(6).into())
        };
        ensure!(
            !ytui_core::is_playlist_url(&media.url) || o.playlist,
            "This is a playlist link. Enable Whole playlist or use a single-song URL"
        );
        if o.playlist_items.is_some() {
            o.playlist = true;
        }
        o.subtitles = self
            .value(7)
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();
        o.auto_subtitles = self.value(8) == "yes";
        o.metadata = self.value(9) == "yes";
        o.thumbnail = self.value(10) == "yes";
        o.bandwidth_kib = self.value(11).parse()?;
        o.backend = if self.value(12).is_empty() {
            None
        } else {
            Some(self.value(12).into())
        };
        o.audio_quality = self.value(13).into();
        o.format_selector = (!self.value(14).is_empty()).then(|| self.value(14).to_owned());
        o.validate()?;
        Ok((media.clone(), o))
    }
    pub fn config_values(&self) -> Result<Config> {
        let FormKind::Settings(base) = &self.kind else {
            anyhow::bail!("Not a settings form")
        };
        let mut c = base.clone();
        c.theme = self.value(0).into();
        c.theme_provider = self.value(1).into();
        c.image_protocol = self.value(2).into();
        c.thumbnails = self.value(3) == "yes";
        c.concurrency = self.value(4).parse()?;
        c.download.directory = self.value(5).into();
        c.download.audio_directory = self.value(6).into();
        c.yt_dlp = self.value(7).into();
        c.mpv = self.value(8).into();
        c.ffmpeg = self.value(9).into();
        c.auth_store = self.value(10).into();
        c.account_index = self.value(11).parse()?;
        c.proxy = if self.value(12).is_empty() {
            None
        } else {
            Some(self.value(12).into())
        };
        c.download_backend = self.value(13).into();
        c.playback_backend = self.value(53).into();
        c.language = self.value(54).into();
        c.native_js_runtime = if self.value(14).is_empty() {
            None
        } else {
            Some(self.value(14).into())
        };
        c.clipboard_watcher = self.value(15) == "yes";
        c.notifications = self.value(16) == "yes";
        c.notification_sound = self.value(17) == "yes";
        c.schedule.download_window =
            (!self.value(18).is_empty()).then(|| self.value(18).to_owned());
        c.schedule.bandwidth_window =
            (!self.value(19).is_empty()).then(|| self.value(19).to_owned());
        c.schedule.window_bandwidth_kib = self.value(20).parse()?;
        c.download.global_bandwidth_kib = self.value(21).parse()?;
        c.download.subtitles = self
            .value(22)
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();
        c.download.auto_subtitles = self.value(23) == "yes";
        c.download.embed_subtitles = self.value(24) == "yes";
        c.download.metadata = self.value(25) == "yes";
        c.download.thumbnail = self.value(26) == "yes";
        c.download.sponsorblock = self
            .value(27)
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();
        c.download.sponsorblock_remove = self.value(28) == "yes";
        c.download.split_chapters = self.value(29) == "yes";
        c.download.filename_template = self.value(30).into();
        c.download.post_download_hook = (!self.value(31).is_empty()).then(|| self.value(31).into());
        c.max_retries = self.value(32).parse()?;
        c.retry_delay_seconds = self.value(33).parse()?;
        c.playback_history = self.value(34) == "yes";
        c.ffprobe = self.value(35).into();
        c.download.sponsorblock_api = self.value(36).into();
        c.download.audio_quality = self.value(37).into();
        c.reduced_motion = self.value(38) == "yes";
        c.download.audio = self.value(39) == "yes";
        c.download.quality = self.value(40).into();
        c.download.container = self.value(41).into();
        c.download.audio_container = self.value(42).into();
        c.download.playlist = self.value(43) == "yes";
        c.download.skip_duplicates = self.value(44) == "yes";
        c.download.bandwidth_kib = self.value(45).parse()?;
        c.cliamp = self.value(46).into();
        c.nmcli = self.value(47).into();
        c.radio_pinned_countries = self
            .value(48)
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_uppercase)
            .collect();
        c.radio_servers = self
            .value(49)
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();
        c.show_hints = self.value(50) == "yes";
        c.playback_sponsorblock = self.value(51).into();
        c.playback_sponsor_categories = self
            .value(52)
            .split(',')
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
            .collect();
        c.music_player_style = self.value(55).into();
        c.validate()?;
        Ok(c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn key_editor_roundtrips_multiple_bindings_and_rejects_collisions() {
        let mut config = Config::default();
        config.keybindings.insert("f8".into(), "preview".into());
        config.keybindings.insert("V".into(), "preview".into());
        let mut form = Form::keys(config);
        let saved = form.key_values().unwrap();
        assert_eq!(saved.keybindings.get("f8").unwrap(), "preview");
        assert_eq!(saved.keybindings.get("V").unwrap(), "preview");
        assert!(!saved.keybindings.contains_key("v"));
        form.fields[0].value = Editor::new("q");
        assert!(
            form.key_values()
                .unwrap_err()
                .to_string()
                .contains("more than once")
        );
    }
    #[test]
    fn schedule_settings_roundtrip_and_reject_invalid_windows() {
        let mut config = Config::default();
        config.schedule.download_window = Some("22:30-06:00".into());
        config.schedule.bandwidth_window = Some("09:00-18:00".into());
        config.schedule.window_bandwidth_kib = 512;
        config.clipboard_watcher = true;
        let mut form = Form::settings(config);
        let saved = form.config_values().unwrap();
        assert_eq!(saved.schedule.window_bandwidth_kib, 512);
        assert!(saved.clipboard_watcher);
        form.fields[18].value = Editor::new("25:00-08:00");
        assert!(form.config_values().is_err());
    }
    #[test]
    fn switching_audio_mode_uses_its_default_destination_but_preserves_custom_edits() {
        let options = DownloadOptions {
            directory: "/video".into(),
            audio_directory: "/music".into(),
            ..Default::default()
        };
        let mut form = Form::download(Media::default(), options);
        form.toggle();
        assert_eq!(
            form.download_values().unwrap().1.audio_directory.to_str(),
            Some("/music")
        );
        form.fields[4].value = Editor::new("/custom");
        form.toggle();
        assert_eq!(
            form.download_values().unwrap().1.directory.to_str(),
            Some("/custom")
        );
    }
    #[test]
    fn playlist_links_enable_collection_download_but_watch_context_does_not() {
        let mut form = Form::download(
            Media {
                url: "https://music.youtube.com/playlist?list=PLsynthetic".into(),
                ..Default::default()
            },
            DownloadOptions {
                directory: "/synthetic/downloads".into(),
                ..Default::default()
            },
        );
        assert_eq!(form.value(5), "yes");
        assert!(form.download_values().unwrap().1.playlist);
        form.fields[5].value = Editor::new("no");
        assert!(form.download_values().is_err());
        let form = Form::download(
            Media {
                url: "https://www.youtube.com/watch?v=synthetic00&list=PLsynthetic".into(),
                ..Default::default()
            },
            DownloadOptions {
                directory: "/synthetic/downloads".into(),
                ..Default::default()
            },
        );
        assert_eq!(form.value(5), "no");
    }
}

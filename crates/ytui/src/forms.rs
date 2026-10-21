use crate::editor::Editor;
use anyhow::{Result, ensure};
use ytui_core::{Config, DownloadOptions, Media};
#[derive(Clone)]
pub struct Field {
    pub label: &'static str,
    pub value: Editor,
    pub toggle: bool,
}
impl Field {
    fn text(label: &'static str, value: impl ToString) -> Self {
        Self {
            label,
            value: Editor::new(value.to_string()),
            toggle: false,
        }
    }
    fn flag(label: &'static str, value: bool) -> Self {
        Self {
            label,
            value: Editor::new(if value { "yes" } else { "no" }),
            toggle: true,
        }
    }
}
pub enum FormKind {
    Download(Media, DownloadOptions),
    Settings(Config),
    Auth,
    Keys(Config),
}
pub struct Form {
    pub kind: FormKind,
    pub fields: Vec<Field>,
    pub selected: usize,
    pub error: Option<String>,
}
impl Form {
    pub fn download(media: Media, options: DownloadOptions) -> Self {
        let o = &options;
        let fields = vec![
            Field::flag("Audio only", o.audio),
            Field::text("Video quality", &o.quality),
            Field::text("Video container", &o.container),
            Field::text("Audio format", &o.audio_container),
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
            Field::text(
                "Backend (blank=default)",
                o.backend.clone().unwrap_or_default(),
            ),
        ];
        Self {
            kind: FormKind::Download(media, options),
            fields,
            selected: 0,
            error: None,
        }
    }
    pub fn settings(config: Config) -> Self {
        let c = &config;
        let fields = vec![
            Field::text("Theme", &c.theme),
            Field::text("Theme provider", &c.theme_provider),
            Field::text("Image protocol", &c.image_protocol),
            Field::flag("Thumbnails", c.thumbnails),
            Field::text("Concurrent downloads", c.concurrency),
            Field::text("Video directory", c.download.directory.display()),
            Field::text("Audio directory", c.download.audio_directory.display()),
            Field::text("yt-dlp executable", &c.yt_dlp),
            Field::text("mpv executable", &c.mpv),
            Field::text("FFmpeg executable", &c.ffmpeg),
            Field::text("Auth storage", &c.auth_store),
            Field::text("Account index", c.account_index),
            Field::text("Proxy (optional)", c.proxy.clone().unwrap_or_default()),
            Field::text("Download / playback backend", &c.download_backend),
            Field::text(
                "Deno runtime (optional)",
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
        ];
        Self {
            kind: FormKind::Settings(config),
            fields,
            selected: 0,
            error: None,
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
                        default.to_string()
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
        }
    }
    pub fn title(&self) -> &str {
        match &self.kind {
            FormKind::Download(_, _) => "Download",
            FormKind::Settings(_) => "Settings",
            FormKind::Auth => "Connect your YouTube account",
            FormKind::Keys(_) => "Keyboard shortcuts",
        }
    }
    pub fn value(&self, index: usize) -> &str {
        self.fields[index].value.text.trim()
    }
    pub fn toggle(&mut self) {
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
}

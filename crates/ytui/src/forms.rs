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
        ];
        Self {
            kind: FormKind::Settings(config),
            fields,
            selected: 0,
            error: None,
        }
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
        c.validate()?;
        Ok(c)
    }
}

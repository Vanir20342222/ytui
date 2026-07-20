"""Settings panel for application configuration."""

from __future__ import annotations

import logging

from textual.app import ComposeResult
from textual.containers import Horizontal, VerticalScroll
from textual.widgets import Input, Label, Select, Switch, TabbedContent, TabPane

from ytui.config.settings import Settings
from ytui.screens.panels import BasePanel

logger = logging.getLogger(__name__)


class SettingsPanel(BasePanel):
    """Panel for configuring application settings."""

    DEFAULT_CSS = """
    SettingsPanel #panel-dialog {
        width: 90%;
        max-width: 105;
        min-width: 65;
    }
    SettingsPanel TabbedContent {
        height: 1fr;
    }
    SettingsPanel TabPane {
        padding: 1 2;
    }
    SettingsPanel .setting-row {
        layout: horizontal;
        height: auto;
        margin-bottom: 1;
        align: left middle;
    }
    SettingsPanel .setting-label {
        width: 36;
        min-width: 24;
        content-align-vertical: middle;
        color: $text;
        text-wrap: nowrap;
        text-overflow: ellipsis;
        overflow: hidden;
    }
    SettingsPanel .setting-input {
        width: 1fr;
    }
    SettingsPanel .setting-row Switch {
        /* Align the switch with the inputs visually */
        margin-left: 1;
    }
    """

    def __init__(self, initial_tab: str | None = None, **kwargs) -> None:
        super().__init__(title="Application Settings", subtitle="Configure ytui options", **kwargs)
        self.settings = Settings.load()
        self.initial_tab = initial_tab
        # Collect (select_id, value) pairs to apply after the selects are mounted,
        # because setting value pre-mount silently fails to update the button label.
        self._pending_select_values: dict[str, str] = {}

    def on_mount(self) -> None:
        """Apply pending select values and active initial tab now that the widgets are mounted."""
        if self.initial_tab:
            try:
                self.query_one(TabbedContent).active = self.initial_tab
            except Exception:
                pass

        for sel_id, value in self._pending_select_values.items():
            try:
                sel = self.query_one(f"#{sel_id}", Select)
                if value in {v for _, v in sel._options}:
                    sel.value = value
            except Exception:
                pass
        self._pending_select_values.clear()

    def compose_content(self) -> ComposeResult:
        with TabbedContent():
            with TabPane("Quality", id="tab-quality"):
                with VerticalScroll():
                    yield self._make_select(
                        "Video Quality",
                        "quality.video_quality",
                        self.settings.quality.video_quality,
                        [
                            ("Best", "best"), ("4K (2160p)", "2160"), ("1440p", "1440"),
                            ("1080p", "1080"), ("720p", "720"), ("480p", "480"),
                        ]
                    )
                    yield self._make_select(
                        "Audio Quality",
                        "quality.audio_quality",
                        self.settings.quality.audio_quality,
                        [
                            ("Best", "best"), ("Lossless", "lossless"),
                            ("320k", "320"), ("256k", "256"), ("192k", "192"), ("128k", "128"),
                        ]
                    )
                    yield self._make_select(
                        "Video Container",
                        "quality.video_container",
                        self.settings.quality.video_container,
                        [("MP4", "mp4"), ("MKV", "mkv"), ("WebM", "webm")]
                    )
                    yield self._make_select(
                        "Audio Container",
                        "quality.audio_container",
                        self.settings.quality.audio_container,
                        [("MP3", "mp3"), ("M4A", "m4a"), ("FLAC", "flac"), ("WAV", "wav")]
                    )
                    yield self._make_switch(
                        "Prefer High Frame Rate",
                        "quality.prefer_hfr",
                        self.settings.quality.prefer_hfr,
                    )
                    yield self._make_switch(
                        "Playlist Mode (whole playlists)",
                        "quality.playlist_mode",
                        self.settings.quality.playlist_mode,
                    )
                    yield self._make_switch(
                        "Auto-convert video to container",
                        "quality.auto_convert_video",
                        self.settings.quality.auto_convert_video,
                    )

            with TabPane("Directories", id="tab-directories"):
                with VerticalScroll():
                    yield self._make_input(
                        "Video Directory",
                        "directories.video_dir",
                        self.settings.directories.video_dir,
                    )
                    yield self._make_input(
                        "Audio Directory",
                        "directories.audio_dir",
                        self.settings.directories.audio_dir,
                    )
                    yield self._make_switch(
                        "Open after download",
                        "directories.open_after_download",
                        self.settings.directories.open_after_download,
                    )

            with TabPane("Network", id="tab-network"):
                with VerticalScroll():
                    yield self._make_input(
                        "Max Concurrent",
                        "network.max_concurrent_downloads",
                        str(self.settings.network.max_concurrent_downloads),
                    )
                    yield self._make_input(
                        "Global BW Limit (KB/s)",
                        "network.global_bandwidth_limit",
                        str(self.settings.network.global_bandwidth_limit),
                    )
                    yield self._make_input(
                        "Per-DL BW Limit (KB/s)",
                        "network.per_download_bandwidth_limit",
                        str(self.settings.network.per_download_bandwidth_limit),
                    )
                    yield self._make_input(
                        "Proxy URL", "network.proxy_url", self.settings.network.proxy_url
                    )
                    yield self._make_select(
                        "Browser Cookies",
                        "network.cookies_from_browser",
                        self.settings.network.cookies_from_browser or self.settings.network.browser_cookies,
                        [
                            ("(none)", ""), ("Chrome", "chrome"),
                            ("Firefox", "firefox"), ("Edge", "edge"), ("Safari", "safari"),
                            ("Brave", "brave"), ("Vivaldi", "vivaldi"), ("Opera", "opera"),
                        ],
                    )
                    yield self._make_input(
                        "Cookie File Path",
                        "network.cookies_file",
                        self.settings.network.cookies_file or self.settings.network.cookie_file,
                    )

            with TabPane("Subtitles", id="tab-subtitles"):
                with VerticalScroll():
                    yield self._make_switch(
                        "Download Subtitles", "subtitles.enabled", self.settings.subtitles.enabled
                    )
                    yield self._make_input(
                        "Languages (comma-sep)",
                        "subtitles.languages",
                        ",".join(self.settings.subtitles.languages),
                    )
                    yield self._make_switch(
                        "Embed Subtitles", "subtitles.embed", self.settings.subtitles.embed
                    )
                    yield self._make_select(
                        "Subtitle Format",
                        "subtitles.format",
                        self.settings.subtitles.format,
                        [("SRT", "srt"), ("VTT", "vtt"), ("ASS", "ass")],
                    )
                    yield self._make_switch(
                        "Auto-translate",
                        "subtitles.auto_translate",
                        self.settings.subtitles.auto_translate,
                    )

            with TabPane("Metadata", id="tab-metadata"):
                with VerticalScroll():
                    yield self._make_switch(
                        "Embed Thumbnail",
                        "metadata.embed_thumbnail",
                        self.settings.metadata.embed_thumbnail,
                    )
                    yield self._make_switch(
                        "Write Metadata",
                        "metadata.write_metadata",
                        self.settings.metadata.write_metadata,
                    )
                    yield self._make_input(
                        "Filename Template",
                        "metadata.filename_template",
                        self.settings.metadata.filename_template,
                    )

            with TabPane("Appearance", id="tab-appearance"):
                with VerticalScroll():
                    yield self._make_select(
                        "Theme",
                        "appearance.theme",
                        self.settings.appearance.theme,
                        [
                            ("Default (Dark)", "default"),
                            ("Midnight", "midnight"),
                            ("Light", "light"),
                            ("Nord", "nord"),
                            ("Solarized Dark", "solarized-dark"),
                            ("Dracula", "dracula"),
                            ("Gruvbox", "gruvbox"),
                        ],
                    )
                    yield self._make_switch(
                        "Show Thumbnails",
                        "appearance.show_thumbnails",
                        self.settings.appearance.show_thumbnails,
                    )
                    yield self._make_switch(
                        "Enable Animations",
                        "appearance.animations_enabled",
                        self.settings.appearance.animations_enabled,
                    )

            with TabPane("Advanced", id="tab-advanced"):
                with VerticalScroll():
                    yield self._make_switch(
                        "SponsorBlock",
                        "advanced.sponsor_block",
                        self.settings.advanced.sponsor_block,
                    )
                    yield self._make_select(
                        "SponsorBlock Action",
                        "advanced.sponsor_block_action",
                        self.settings.advanced.sponsor_block_action,
                        [("Mark as chapters", "mark"), ("Remove / Skip segments", "skip")],
                    )
                    yield self._make_input(
                        "SponsorBlock Categories",
                        "advanced.sponsor_block_categories",
                        ",".join(self.settings.advanced.sponsor_block_categories)
                        if isinstance(self.settings.advanced.sponsor_block_categories, list)
                        else str(self.settings.advanced.sponsor_block_categories),
                    )
                    yield self._make_input(
                        "SponsorBlock API URL",
                        "advanced.sponsor_block_api",
                        self.settings.advanced.sponsor_block_api,
                    )
                    yield self._make_switch(
                        "Enable Post-DL Script",
                        "advanced.enable_custom_script",
                        self.settings.advanced.enable_custom_script,
                    )
                    yield self._make_input(
                        "Post-DL Script Path",
                        "advanced.custom_script",
                        self.settings.advanced.custom_script,
                    )
                    yield self._make_switch(
                        "Split Chapters",
                        "advanced.chapter_split",
                        self.settings.advanced.chapter_split,
                    )
                    yield self._make_switch(
                        "Desktop Notifications",
                        "advanced.desktop_notifications",
                        self.settings.advanced.desktop_notifications,
                    )

    def _make_select(
        self,
        label: str,
        id_path: str,
        value: str,
        options: list[tuple[str, str]],
    ) -> Horizontal:
        # Construct with no value; we set it in on_mount (post-mount), because
        # setting value pre-mount silently fails to update the button label due
        # to a Textual reactivity/mount-ordering quirk.
        sel = Select(options, value=Select.NULL, classes="setting-input")
        sel.id = f"setting_select_{id_path.replace('.', '__')}"
        self._pending_select_values[sel.id] = value
        return Horizontal(Label(label, classes="setting-label"), sel, classes="setting-row")

    def _make_input(self, label: str, id_path: str, value: str) -> Horizontal:
        inp = Input(value, classes="setting-input")
        inp.id = f"setting_input_{id_path.replace('.', '__')}"
        return Horizontal(Label(label, classes="setting-label"), inp, classes="setting-row")

    def _make_switch(self, label: str, id_path: str, value: bool) -> Horizontal:
        sw = Switch(value=value)
        sw.id = f"setting_switch_{id_path.replace('.', '__')}"
        return Horizontal(Label(label, classes="setting-label"), sw, classes="setting-row")

    def on_button_pressed(self, event) -> None:
        if event.button.id == "close-btn":
            self.save_settings()
            self.dismiss()
            # Stop the MRO from re-dispatching to BasePanel (double-dismiss).
            event.prevent_default()

    def action_dismiss_modal(self) -> None:
        self.save_settings()
        self.dismiss()

    def save_settings(self) -> None:
        """Collect all values and save settings."""
        # Quality
        if (sel := self.query(Select)):
            for widget in sel:
                if not widget.id or not widget.id.startswith("setting_select_"):
                    continue
                path = widget.id.replace("setting_select_", "").split("__")
                if hasattr(self.settings, path[0]):
                    val = widget.value
                    if val is Select.BLANK or val is Select.NULL:
                        val = ""
                    setattr(getattr(self.settings, path[0]), path[1], val)

        # Integer-coercion map for numeric input fields.
        _int_fields = {
            ("network", "max_concurrent_downloads"),
            ("network", "global_bandwidth_limit"),
            ("network", "per_download_bandwidth_limit"),
        }
        if (inp := self.query(Input)):
            for widget in inp:
                if not widget.id or not widget.id.startswith("setting_input_"):
                    continue
                path = widget.id.replace("setting_input_", "").split("__")
                if not hasattr(self.settings, path[0]):
                    logger.warning(f"Unknown setting path: {path}")
                    continue
                value = widget.value
                # List settings stored as comma-separated input values
                if tuple(path) in (("subtitles", "languages"), ("advanced", "sponsor_block_categories")):
                    value = [item.strip() for item in value.split(",") if item.strip()]
                elif tuple(path) in _int_fields:
                    try:
                        value = int(value) if value.strip() else 0
                    except ValueError:
                        value = 0
                setattr(getattr(self.settings, path[0]), path[1], value)

        if (sw := self.query(Switch)):
            for widget in sw:
                if not widget.id or not widget.id.startswith("setting_switch_"):
                    continue
                path = widget.id.replace("setting_switch_", "").split("__")
                if hasattr(self.settings, path[0]):
                    setattr(getattr(self.settings, path[0]), path[1], widget.value)

        self.settings.save()
        # Propagate the live settings to the running app + queue manager.
        try:
            app = self.app
            if getattr(app, "settings", None) is not None:
                app.settings = self.settings
            if getattr(app, "queue_manager", None) is not None:
                app.queue_manager.settings = self.settings
                # Re-size the concurrency semaphore if max_concurrent changed.
                new_max = max(self.settings.network.max_concurrent_downloads, 1)
                if hasattr(app.queue_manager._semaphore, "resize"):
                    app.queue_manager._semaphore.resize(new_max)
                else:
                    app.queue_manager._semaphore = __import__("asyncio").Semaphore(new_max)
            # Re-apply theme in case it changed.
            if hasattr(app, "reload_theme"):
                app.reload_theme()
        except Exception:
            pass
        self.app.notify("Settings saved")

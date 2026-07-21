"""Main application screen combining all main widgets."""

from __future__ import annotations

import asyncio
from pathlib import Path

from textual.app import ComposeResult
from textual.screen import Screen

from ytui.widgets.header_bar import HeaderBar
from ytui.widgets.queue_panel import QueuePanel
from ytui.widgets.input_bar import InputBar
from ytui.widgets.boot_screen import BootScreen
from ytui.screens.panels import (
    GenericPanel,
    VpnPanel,
    QualityPanel,
    HistoryPanel,
    StatsPanel,
    BandwidthPanel,
    ProxyPanel,
    ThemePanel,
    ProfilePanel,
    ClipboardPanel,
    LogPanel,
    QueuePanelModal,
)
from ytui.widgets.ascii_banner import AsciiBanner
from ytui.widgets.command_suggestions import CommandSuggestions
from ytui.widgets.queue_item import QueueItemWidget
from ytui.constants import COMMANDS


class MainScreen(Screen):
    """The central view."""

    def compose(self) -> ComposeResult:
        yield HeaderBar(id="header-bar")
        yield AsciiBanner(id="ascii-banner")
        yield QueuePanel(id="queue-panel")
        yield CommandSuggestions(id="cmd-suggestions")
        yield InputBar(id="input-container")

    def on_mount(self) -> None:
        """Initialize UI data."""
        self.app.push_screen(BootScreen())

    def on_input_bar_url_submitted(self, message: InputBar.UrlSubmitted) -> None:
        """Handle URL submission."""
        self.app.notify(f"Adding to queue: {message.url}")
        self.app.queue_manager_add(message.url)

    def on_input_bar_command_submitted(self, message: InputBar.CommandSubmitted) -> None:
        """Handle / command submission."""
        cmd = message.command.split()[0]
        desc = COMMANDS.get(cmd, "Unknown command")

        # Core app commands.
        if cmd in ("/quit", "/q"):
            self.app.exit()
            return
        if cmd == "/settings":
            from ytui.screens.settings_panel import SettingsPanel
            self.app.push_screen(SettingsPanel())
            return
        if cmd in ("/help", "/?"):
            self._show_help()
            return
        if cmd == "/clear":
            self._clear_queue()
            return

        # Download-mode commands.
        if cmd == "/audio":
            self._set_download_mode("audio", playlist=None)
            return
        if cmd == "/video":
            self._set_download_mode("video", playlist=None)
            return
        if cmd == "/audio-playlist":
            self._set_download_mode("audio", playlist=True)
            return
        if cmd == "/video-playlist":
            self._set_download_mode("video", playlist=True)
            return

        # Queue control.
        if cmd == "/pauseall":
            if self.app.queue_manager:
                self.app.queue_manager.pause_all()
                self.app.notify("Paused all downloads")
            return
        if cmd == "/resumeall":
            if self.app.queue_manager:
                self.app.queue_manager.resume_all()
                self.app.notify("Resumed all downloads")
            return
        if cmd == "/stopall":
            if self.app.queue_manager:
                n = self.app.queue_manager.cancel_all()
                self.app.notify(f"Stopped {n} download(s)")
            else:
                self.app.notify("Queue not ready", severity="warning")
            return

        # Maintenance.
        if cmd == "/update":
            asyncio.create_task(self._do_update())
            return

        # Panels with real implementations.
        panel_map = {
            "/vpn": VpnPanel,
            "/quality": QualityPanel,
            "/queue": QueuePanelModal,
            "/history": HistoryPanel,
            "/stats": StatsPanel,
            "/bandwidth": BandwidthPanel,
            "/limit": BandwidthPanel,
            "/proxy": ProxyPanel,
            "/theme": ThemePanel,
            "/profile": ProfilePanel,
            "/clipboard": ClipboardPanel,
            "/log": LogPanel,
        }
        if cmd in panel_map:
            self.app.push_screen(panel_map[cmd]())
            return

        # Specific Settings tabs mapping for direct navigation
        tab_map = {
            "/subs": "tab-subtitles",
            "/subtitles": "tab-subtitles",
            "/metadata": "tab-metadata",
            "/dir": "tab-directories",
            "/schedule": "tab-advanced",
            "/hooks": "tab-advanced",
            "/keys": "tab-advanced",
        }
        if cmd in tab_map:
            from ytui.screens.settings_panel import SettingsPanel
            self.app.notify(f"Opening {cmd} settings", severity="information")
            self.app.push_screen(SettingsPanel(initial_tab=tab_map[cmd]))
            return

        if cmd == "/convert":
            parts = message.command.split(maxsplit=1)
            filepath = parts[1].strip() if len(parts) > 1 else ""
            if filepath:
                self.app.notify(f"Queuing conversion for: {filepath}")
                self.app.queue_manager_add(filepath)
            else:
                from ytui.screens.settings_panel import SettingsPanel
                self.app.push_screen(SettingsPanel(initial_tab="tab-quality"))
            return

        # Search command
        if cmd == "/search":
            parts = message.command.split(maxsplit=1)
            query = parts[1].strip() if len(parts) > 1 else ""
            if query:
                from ytui.screens.panels import SearchPanel
                self.app.push_screen(SearchPanel(query))
            else:
                self.app.notify("Usage: /search <query>", severity="warning")
            return

        # Import / Export commands
        if cmd == "/import":
            parts = message.command.split(maxsplit=1)
            filepath = parts[1].strip() if len(parts) > 1 else ""
            if not filepath:
                self.app.notify("Usage: /import <path>", severity="warning")
            else:
                asyncio.create_task(self._do_import(filepath))
            return

        if cmd == "/export":
            parts = message.command.split(maxsplit=1)
            filepath = parts[1].strip() if len(parts) > 1 else ""
            self._do_export(filepath)
            return

        # Unknown command.
        self.app.push_screen(GenericPanel(cmd, desc))

    def on_input_bar_search_submitted(self, message: InputBar.SearchSubmitted) -> None:
        """Handle YouTube search by popping up SearchPanel interactive results."""
        query = message.query.strip()
        if not query:
            return
        from ytui.screens.panels import SearchPanel
        self.app.push_screen(SearchPanel(query))

    def on_queue_item_widget_retry_clicked(self, message: QueueItemWidget.RetryClicked) -> None:
        """Handle retry button click."""
        if self.app.queue_manager:
            self.app.queue_manager.retry_item(message.item_id)
            self.app.notify("Retrying download...")

    def on_queue_item_widget_open_clicked(self, message: QueueItemWidget.OpenClicked) -> None:
        """Handle opening/launching media for a completed queue item."""
        if not self.app.queue_manager:
            return
        item = self.app.queue_manager._find_item(message.item_id)
        if item:
            self._launch_item_media(item)

    # ------------------------------------------------------------------ helpers

    def _set_download_mode(self, mode: str, playlist: bool | None) -> None:
        """Switch download mode (and optionally playlist mode) on the live app."""
        if not self.app.settings:
            return
        self.app.settings.quality.download_mode = mode
        if playlist is not None:
            self.app.settings.quality.playlist_mode = playlist
        if self.app.queue_manager:
            self.app.queue_manager.settings = self.app.settings
        self.app.settings.save()
        label = "audio-only" if mode == "audio" else "video"
        if playlist:
            label += " + playlist"
        elif playlist is False:
            label += " (single)"
        self.app.notify(f"Switched to {label} mode")

    async def _do_update(self) -> None:
        """Check for and apply a yt-dlp update."""
        from ytui.engine.updater import check_for_update, update_ytdlp
        self.app.notify("Checking for yt-dlp updates...")
        current, latest, available = await check_for_update()
        if not available:
            self.app.notify(f"yt-dlp is up to date ({current})")
            return
        self.app.notify(f"Updating yt-dlp {current} -> {latest}...")
        success, msg = await asyncio.to_thread(update_ytdlp)
        self.app.notify(msg, severity="information" if success else "error")

    def _show_help(self) -> None:
        """Show a help panel listing available commands."""
        lines = ["Available commands:"]
        for c, description in COMMANDS.items():
            lines.append(f"  {c:<16} {description}")
        lines.append("")
        lines.append("Tip: type / then use Tab to autocomplete, arrow keys to navigate.")
        self.app.push_screen(GenericPanel("/help", "\n".join(lines)))

    def _clear_queue(self) -> None:
        """Remove all completed/failed items from the queue view."""
        if not self.app.queue_manager:
            return
        removed = 0
        for item in list(self.app.queue_manager.items):
            if item.state.value in ("done", "error", "cancelled"):
                self.app.queue_manager.remove_item(item.id)
                removed += 1
        self.app.notify(f"Cleared {removed} finished item(s)")

    async def _do_import(self, path_str: str) -> None:
        if not self.app.queue_manager:
            self.app.notify("Queue manager not ready", severity="warning")
            return
        try:
            count = await self.app.queue_manager.import_queue(path_str)
            self.app.notify(f"Imported {count} item(s) to queue", severity="information")
        except Exception as e:
            self.app.notify(f"Import failed: {e}", severity="error")

    def _do_export(self, path_str: str) -> None:
        if not self.app.queue_manager:
            self.app.notify("Queue manager not ready", severity="warning")
            return
        try:
            out_path = self.app.queue_manager.export_queue(path_str if path_str else None)
            self.app.notify(f"Queue exported to {out_path}", severity="information")
        except Exception as e:
            self.app.notify(f"Export failed: {e}", severity="error")

    def _launch_item_media(self, item) -> None:
        from ytui.utils.filesystem import launch_media_player
        from ytui.queue.models import ItemState

        if item.state != ItemState.DONE and not (item.output_path and Path(item.output_path).exists()):
            self.app.notify(f"Item is not completed ({item.state.value})", severity="warning")
            return

        if not item.output_path:
            self.app.notify("Output path not recorded for item", severity="error")
            return

        success, player_or_err = launch_media_player(item.output_path)
        if success:
            self.app.notify(f"Launching {player_or_err} for: {item.display_title}")
        else:
            self.app.notify(player_or_err, severity="error")
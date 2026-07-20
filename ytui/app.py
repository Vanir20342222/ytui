"""Textual Application entry point."""

from __future__ import annotations

import asyncio
import logging
from pathlib import Path

from textual.app import App

from ytui.config.settings import Settings
from ytui.constants import APP_TITLE
from ytui.queue.manager import QueueManager
from ytui.screens.main_screen import MainScreen

logger = logging.getLogger(__name__)


class YtuiApp(App):
    """Terminal YouTube & Audio Downloader."""

    TITLE = APP_TITLE
    CSS_PATH = ["themes/default.tcss"]

    BINDINGS = [
        ("ctrl+comma", "open_settings", "Settings"),
        ("f1", "show_help", "Help"),
        ("ctrl+p", "pause_all", "Pause all"),
        ("ctrl+r", "resume_all", "Resume all"),
    ]

    def __init__(self, **kwargs) -> None:
        super().__init__(**kwargs)
        self.settings: Settings | None = None
        self.queue_manager: QueueManager | None = None
        self._theme_loaded: bool = False
        self._clipboard_watcher = None

    def action_open_settings(self) -> None:
        """Open the settings panel (ctrl+comma)."""
        # Don't open a second modal if one is already up.
        if type(self.screen).__name__ != "MainScreen":
            return
        from ytui.screens.settings_panel import SettingsPanel
        self.push_screen(SettingsPanel())

    def action_show_help(self) -> None:
        """Show help (f1)."""
        if type(self.screen).__name__ != "MainScreen":
            return
        from ytui.screens.panels import GenericPanel
        from ytui.constants import COMMANDS
        lines = ["Available commands:"]
        for cmd, description in COMMANDS.items():
            lines.append(f"  {cmd:<14} {description}")
        self.push_screen(GenericPanel("/help", "\n".join(lines)))

    def action_pause_all(self) -> None:
        """Pause all active downloads (ctrl+p)."""
        if self.queue_manager:
            self.queue_manager.pause_all()
            self.notify("Paused all downloads")

    def action_resume_all(self) -> None:
        """Resume all paused downloads (ctrl+r)."""
        if self.queue_manager:
            self.queue_manager.resume_all()
            self.notify("Resumed all downloads")

    async def on_mount(self) -> None:
        """Mount the main screen and start the queue manager."""
        # Load settings off the main thread (disk I/O); construct queue manager
        # on the main thread so its SQLite connection is main-thread-owned.
        self.settings = await asyncio.to_thread(Settings.load)
        self._apply_theme()

        self.queue_manager = QueueManager(self.settings)

        # Install MainScreen by name so callbacks can resolve it via get_screen().
        main_screen = MainScreen()
        self.install_screen(main_screen, "MainScreen")
        self.push_screen("MainScreen")

        # Wire UI callbacks before starting the manager so startup items dispatch.
        self.queue_manager.on_item_added = self.on_item_added
        self.queue_manager.on_item_updated = self.on_item_updated
        self.queue_manager.on_item_completed = self.on_item_completed
        self.queue_manager.on_item_error = self.on_item_error
        self.queue_manager.on_item_removed = self.on_item_removed

        await self.queue_manager.start()

        # Start the clipboard watcher if it was enabled in settings.
        if self.settings and self.settings.advanced.clipboard_watcher:
            self.start_clipboard_watcher()

    def start_clipboard_watcher(self) -> None:
        """Start watching the clipboard for YouTube URLs (if not already)."""
        if self._clipboard_watcher is not None:
            return
        from ytui.utils.clipboard import ClipboardWatcher
        self._clipboard_watcher = ClipboardWatcher(callback=self.queue_manager_add)
        self._clipboard_watcher.start()
        self.notify("Clipboard watcher started")

    def stop_clipboard_watcher(self) -> None:
        """Stop the clipboard watcher."""
        if self._clipboard_watcher is not None:
            self._clipboard_watcher.stop()
            self._clipboard_watcher = None
            self.notify("Clipboard watcher stopped")

    def _apply_theme(self) -> None:
        """Apply the configured theme by injecting its variables into the base stylesheet.

        TCSS $variables are resolved inline at parse time. Simply layering
        a theme file on top of default.tcss does NOT retroactively change
        the variable values already baked into default.tcss's rules.

        Instead, we:
          1. Extract variable definitions ($var: value;) from the theme file.
          2. Re-read default.tcss, strip its own variable defs, and prepend
             the theme's variables so every rule resolves to the new colors.
          3. Load the full theme TCSS on top for its selector overrides.
          4. Clear caches, reparse, and force a full repaint.
        """
        if self._theme_loaded or not self.settings:
            return
        import re
        theme = self.settings.appearance.theme or "default"
        self.dark = theme != "light"

        themes_dir = Path(__file__).parent / "themes"
        default_path = themes_dir / "default.tcss"
        default_abs = str(default_path.resolve())

        # ── Read base default.tcss ──
        try:
            default_css = default_path.read_text(encoding="utf-8")
        except Exception as e:
            logger.error(f"Failed to read default.tcss: {e}")
            self._theme_loaded = True
            return

        # ── Remove ALL theme-directory sources (we'll re-add what we need) ──
        themes_dir_str = str(themes_dir.resolve())
        keys_to_remove = [
            key for key in list(self.stylesheet.source.keys())
            if isinstance(key, tuple)
            and isinstance(key[0], str)
            and key[0].startswith(themes_dir_str)
        ]
        for key in keys_to_remove:
            del self.stylesheet.source[key]

        # ── Extract variable lines from the theme file ──
        theme_var_block = ""
        theme_css_full = ""
        if theme != "default":
            theme_path = themes_dir / f"{theme}.tcss"
            if theme_path.exists():
                try:
                    theme_css_full = theme_path.read_text(encoding="utf-8")
                    var_lines = [
                        line for line in theme_css_full.split("\n")
                        if re.match(r"^\s*\$[\w-]+\s*:", line)
                    ]
                    theme_var_block = "\n".join(var_lines) + "\n"
                except Exception as e:
                    logger.error(f"Failed to read theme '{theme}': {e}")

        # ── Build merged default.tcss: strip its vars, prepend theme vars ──
        default_lines = default_css.split("\n")
        filtered = [
            line for line in default_lines
            if not re.match(r"^\s*\$[\w-]+\s*:", line)
        ]
        default_body = "\n".join(filtered)

        if theme_var_block:
            merged_default = theme_var_block + default_body
        else:
            merged_default = default_css  # use original with its own vars

        # ── Re-add merged default.tcss source ──
        from textual.css.stylesheet import CssSource
        self.stylesheet.source[(default_abs, "")] = CssSource(merged_default, False, 0)
        self.stylesheet._require_parse = True

        # ── Add full theme TCSS on top (includes vars for its own selectors) ──
        if theme_css_full.strip():
            theme_abs = str((themes_dir / f"{theme}.tcss").resolve())
            self.stylesheet.source[(theme_abs, "")] = CssSource(theme_css_full, False, 0)
            self.stylesheet._require_parse = True

        # ── Clear parse cache and reparse everything ──
        self.stylesheet._parse_cache.clear()
        try:
            self.stylesheet.reparse()
        except Exception as e:
            logger.error(f"Failed to reparse stylesheets: {e}")

        # ── Force full repaint of all widgets ──
        self._invalidate_css()
        self.call_next(self.refresh_css, animate=False)
        self._theme_loaded = True

    def reload_theme(self) -> None:
        """Re-apply the current theme (called after settings change)."""
        self._theme_loaded = False
        self._apply_theme()

    async def on_unmount(self) -> None:
        """Stop queue manager and clipboard watcher gracefully."""
        self.stop_clipboard_watcher()
        if self.queue_manager:
            await self.queue_manager.stop()

    def queue_manager_add(self, url: str) -> None:
        """Add URL to queue via QueueManager."""
        if not self.queue_manager:
            self.notify("ytui is still starting up, try again in a moment", severity="warning")
            return
        asyncio.create_task(self.queue_manager.add_url(url))

    def _safe_ui_call(self, func, *args) -> None:
        """Schedule a UI call on the main loop.

        If we're already on the app's main thread, run directly (safe). If we're
        on a worker thread, marshal via call_from_thread. If the loop is closed,
        drop the update rather than touch the UI from a non-main thread.
        """
        import threading
        if self._thread_id == threading.get_ident():
            # On the main thread — safe to run the UI call directly.
            func(*args)
            return
        try:
            self.call_from_thread(func, *args)
        except RuntimeError:
            # Loop closed/closing — drop the update.
            return

    def on_item_added(self, item) -> None:
        self._safe_ui_call(self._add_item_ui, item)

    def _add_item_ui(self, item) -> None:
        try:
            main_screen = self.get_screen("MainScreen")
            queue_panel = main_screen.query_one("QueuePanel")
            queue_panel.add_item(item)
            self._update_aggregate()
        except Exception:
            pass

    def on_item_updated(self, item) -> None:
        self._safe_ui_call(self._update_item_ui, item)

    def _update_item_ui(self, item) -> None:
        try:
            main_screen = self.get_screen("MainScreen")
            queue_panel = main_screen.query_one("QueuePanel")
            queue_panel.update_item(item)
            self._update_aggregate()
        except Exception:
            pass

    def on_item_completed(self, item) -> None:
        self._safe_ui_call(self._complete_item_ui, item)

    def _complete_item_ui(self, item) -> None:
        self.notify(f"Download complete: {item.display_title}")
        if self.settings and self.settings.advanced.desktop_notifications:
            from ytui.utils.notifications import send_notification
            send_notification("ytui", f"Download complete: {item.display_title}")
        self._update_item_ui(item)

    def on_item_error(self, item) -> None:
        self._safe_ui_call(self._error_item_ui, item)

    def _error_item_ui(self, item) -> None:
        self.notify(f"Error: {item.display_title}", severity="error")
        if self.settings and self.settings.advanced.desktop_notifications:
            from ytui.utils.notifications import send_notification
            send_notification("ytui", f"Download failed: {item.display_title}")
        self._update_item_ui(item)

    def on_item_removed(self, item) -> None:
        self._safe_ui_call(self._remove_item_ui, item)

    def _remove_item_ui(self, item) -> None:
        try:
            main_screen = self.get_screen("MainScreen")
            queue_panel = main_screen.query_one("QueuePanel")
            queue_panel.remove_item(item.id)
            self._update_aggregate()
        except Exception:
            pass

    def _update_aggregate(self) -> None:
        """Fetch latest aggregate and update UI."""
        if not self.queue_manager:
            return
        try:
            agg = self.queue_manager.get_aggregate_progress()
            main_screen = self.get_screen("MainScreen")
            header = main_screen.query_one("HeaderBar")
            header.active_count = agg.active_count
            header.paused_count = agg.paused_count
            header.completed_count = agg.completed_count
            header.total_count = agg.total_count

            # Bandwidth cap indicator
            if self.settings:
                net = self.settings.network
                try:
                    per_limit = int(net.per_download_bandwidth_limit or 0)
                except (ValueError, TypeError):
                    per_limit = 0
                try:
                    global_limit = int(net.global_bandwidth_limit or 0)
                except (ValueError, TypeError):
                    global_limit = 0

                if per_limit > 0:
                    header.bandwidth_cap = f"{per_limit} KB/s"
                elif global_limit > 0:
                    header.bandwidth_cap = f"{global_limit} KB/s"
                else:
                    header.bandwidth_cap = ""

            from ytui.utils.formatting import format_speed
            header.total_speed = format_speed(agg.total_speed) if agg.total_speed > 0 else ""

            queue_panel = main_screen.query_one("QueuePanel")
            queue_panel.update_aggregate(
                active=agg.active_count,
                total_speed=agg.total_speed,
                total_downloaded=agg.total_downloaded,
                total_size=agg.total_size,
                combined_eta=agg.combined_eta,
            )
        except Exception:
            pass


def main() -> None:
    app = YtuiApp()
    app.run()


if __name__ == "__main__":
    main()

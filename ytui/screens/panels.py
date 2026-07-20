"""Modal panels for / commands."""

from __future__ import annotations

import asyncio

from textual.app import ComposeResult
from textual.containers import Vertical, Horizontal
from textual.screen import ModalScreen
from textual.widgets import Static, Button

class BasePanel(ModalScreen):
    """Base modal screen for panels."""
    DEFAULT_CSS = """
    BasePanel {
        align: center middle;
        background: $background 85%;
    }
    BasePanel #panel-dialog {
        width: 80;
        max-width: 95%;
        height: auto;
        max-height: 85%;
        background: $surface;
        border: round $border;
        padding: 1 2;
    }
    BasePanel:focus-within #panel-dialog {
        border: round $accent;
    }
    BasePanel .panel-title {
        text-style: bold;
        color: $text;
        text-align: center;
        width: 100%;
        margin-bottom: 1;
    }
    BasePanel .panel-subtitle {
        color: $text-muted;
        text-align: center;
        width: 100%;
        margin-bottom: 1;
    }
    BasePanel .panel-actions {
        layout: horizontal;
        align: center middle;
        margin-top: 1;
        height: auto;
    }
    """
    
    BINDINGS = [
        ("escape", "dismiss_modal", "Close"),
        ("q", "dismiss_modal", "Close")
    ]
    
    def __init__(self, title: str, subtitle: str = "", **kwargs) -> None:
        super().__init__(**kwargs)
        self.panel_title = title
        self.panel_subtitle = subtitle

    def compose(self) -> ComposeResult:
        with Vertical(id="panel-dialog"):
            yield Static(self.panel_title, classes="panel-title")
            if self.panel_subtitle:
                yield Static(self.panel_subtitle, classes="panel-subtitle")
            
            yield from self.compose_content()
            
            with Horizontal(classes="panel-actions"):
                yield Button("Close", id="close-btn", variant="primary")

    def compose_content(self) -> ComposeResult:
        """Override to add specific panel content."""
        yield Static("Under construction...", classes="panel-label")

    def on_button_pressed(self, event: Button.Pressed) -> None:
        if event.button.id == "close-btn":
            self.dismiss()
            # Stop the MRO walk from re-dispatching via BasePanel, which would
            # call dismiss() a second time and raise ScreenStackError.
            event.prevent_default()

    def action_dismiss_modal(self) -> None:
        self.dismiss()

class GenericPanel(BasePanel):
    """Fallback panel for commands without a dedicated handler."""
    def __init__(self, command: str, description: str, **kwargs) -> None:
        super().__init__(title=f"Command: {command}", subtitle=description, **kwargs)

    def compose_content(self) -> ComposeResult:
        yield Static(
            "This command is not yet implemented.\n\n"
            "Type /help for the full list of available commands.",
            classes="panel-label",
        )


def _settings():
    from ytui.config.settings import Settings
    return Settings.load()


def _save_and_propagate(settings) -> None:
    """Persist settings and push them to the running app + queue manager."""
    settings.save()


class QualityPanel(BasePanel):
    """Quick video/audio quality and mode switching."""
    def __init__(self, **kwargs) -> None:
        super().__init__(title="Quality & Mode", subtitle="Quick presets", **kwargs)

    def compose_content(self) -> ComposeResult:
        yield Static("Download mode, quality ceilings, and containers.", classes="panel-label")
        yield Button("Audio mode", id="qpm-audio")
        yield Button("Video mode", id="qpm-video")
        yield Button("Open full Settings", id="qpm-settings")

    def on_button_pressed(self, event: Button.Pressed) -> None:
        s = _settings()
        if event.button.id == "qpm-audio":
            s.quality.download_mode = "audio"
            s.save()
            self._propagate(s)
            self.app.notify("Switched to audio-only mode")
            event.prevent_default()
        elif event.button.id == "qpm-video":
            s.quality.download_mode = "video"
            s.save()
            self._propagate(s)
            self.app.notify("Switched to video mode")
            event.prevent_default()
        elif event.button.id == "qpm-settings":
            self.dismiss()
            from ytui.screens.settings_panel import SettingsPanel
            self.app.push_screen(SettingsPanel())
            event.prevent_default()

    def _propagate(self, s) -> None:
        try:
            app = self.app
            app.settings = s
            if app.queue_manager:
                app.queue_manager.settings = s
        except Exception:
            pass


class HistoryPanel(BasePanel):
    """Show recent download history."""
    def __init__(self, **kwargs) -> None:
        super().__init__(title="Download History", subtitle="Recent completed downloads", **kwargs)

    def compose_content(self) -> ComposeResult:
        try:
            from ytui.queue.persistence import QueueDatabase
            db = QueueDatabase()
            rows = db.get_history(limit=20)
            db.close()
        except Exception:
            rows = []
        if not rows:
            yield Static("No downloads yet.", classes="panel-label")
            return
        lines = []
        from ytui.utils.formatting import format_size
        import datetime as _dt
        for r in rows:
            ts = _dt.datetime.fromtimestamp(r.get("completed_at", 0)).strftime("%Y-%m-%d %H:%M")
            size = format_size(r.get("file_size", 0))
            lines.append(f"{ts}  {r.get('title','')[:40]:40} {size:>10}  {r.get('download_mode','')}")
        yield Static("\n".join(lines), classes="panel-label")


class StatsPanel(BasePanel):
    """Aggregate download statistics."""
    def __init__(self, **kwargs) -> None:
        super().__init__(title="Statistics", subtitle="All-time download stats", **kwargs)

    def compose_content(self) -> ComposeResult:
        try:
            from ytui.queue.persistence import QueueDatabase
            db = QueueDatabase()
            stats = db.get_stats()
            db.close()
        except Exception:
            stats = {}
        from ytui.utils.formatting import format_size
        import datetime as _dt
        total = stats.get("total_downloads", 0)
        bytes_ = stats.get("total_bytes", 0)
        duration = stats.get("total_duration", 0)
        time_spent = stats.get("total_time_spent", 0)
        yield Static(
            f"Total downloads:   {total}\n"
            f"Total size:         {format_size(bytes_)}\n"
            f"Total duration:     {duration//3600}h {(duration%3600)//60}m\n"
            f"Total time spent:   {time_spent//3600}h {(time_spent%3600)//60}m",
            classes="panel-label",
        )


class BandwidthPanel(BasePanel):
    """Bandwidth limit configuration."""
    def __init__(self, **kwargs) -> None:
        super().__init__(title="Bandwidth Limiting", subtitle="Throttle downloads", **kwargs)

    def compose_content(self) -> ComposeResult:
        s = _settings()
        from textual.widgets import Input as TInput, Label
        n = s.network
        yield Label("Global limit (KB/s, 0 = unlimited):")
        inp = TInput(str(n.global_bandwidth_limit), id="bw-global")
        yield inp
        yield Label("Per-download limit (KB/s, 0 = unlimited):")
        inp2 = TInput(str(n.per_download_bandwidth_limit), id="bw-per")
        yield inp2
        yield Button("Save", id="bw-save")

    def on_button_pressed(self, event: Button.Pressed) -> None:
        if event.button.id == "bw-save":
            s = _settings()
            try:
                s.network.global_bandwidth_limit = int(self.query_one("#bw-global").value or 0)
            except ValueError:
                pass
            try:
                s.network.per_download_bandwidth_limit = int(self.query_one("#bw-per").value or 0)
            except ValueError:
                pass
            s.save()
            try:
                app = self.app
                app.settings = s
                if app.queue_manager:
                    app.queue_manager.settings = s
            except Exception:
                pass
            self.app.notify("Bandwidth settings saved")
            event.prevent_default()


class ProxyPanel(BasePanel):
    """Proxy configuration."""
    def __init__(self, **kwargs) -> None:
        super().__init__(title="Proxy Configuration", subtitle="Route downloads through a proxy", **kwargs)

    def compose_content(self) -> ComposeResult:
        s = _settings()
        from textual.widgets import Input as TInput, Label, Select
        yield Label("Proxy URL (http://host:port or socks5://host:port):")
        yield TInput(s.network.proxy_url, id="px-url")
        yield Label("Proxy type:")
        yield Select(
            [("HTTP", "http"), ("SOCKS5", "socks5"), ("(none)", "")],
            value=s.network.proxy_type or "",
            id="px-type",
        )
        yield Button("Save", id="px-save")

    def on_button_pressed(self, event: Button.Pressed) -> None:
        if event.button.id == "px-save":
            s = _settings()
            s.network.proxy_url = self.query_one("#px-url").value
            from textual.widgets import Select as TSelect
            s.network.proxy_type = self.query_one("#px-type", TSelect).value
            s.save()
            try:
                app = self.app
                app.settings = s
                if app.queue_manager:
                    app.queue_manager.settings = s
            except Exception:
                pass
            self.app.notify("Proxy settings saved")
            event.prevent_default()


class ThemePanel(BasePanel):
    """Quick theme switcher."""
    def __init__(self, **kwargs) -> None:
        super().__init__(title="Theme", subtitle="Color scheme", **kwargs)

    def compose_content(self) -> ComposeResult:
        s = _settings()
        from textual.widgets import Label
        themes = ["default", "midnight", "light", "nord", "solarized-dark", "dracula", "gruvbox"]
        for t in themes:
            label = t.replace("-", " ").title()
            yield Button(f"{label}{'  (active)' if s.appearance.theme == t else ''}", id=f"th-{t}")

    def on_button_pressed(self, event: Button.Pressed) -> None:
        if event.button.id and event.button.id.startswith("th-"):
            theme = event.button.id[3:]
            s = _settings()
            s.appearance.theme = theme
            s.save()
            try:
                app = self.app
                app.settings = s
                if hasattr(app, "reload_theme"):
                    app.reload_theme()
            except Exception:
                pass
            self.app.notify(f"Theme: {theme}")
            self.dismiss()
            event.prevent_default()


class ProfilePanel(BasePanel):
    """Save/load/switch named settings profiles."""
    def __init__(self, **kwargs) -> None:
        super().__init__(title="Profiles", subtitle="Save/switch bundles of settings", **kwargs)

    def compose_content(self) -> ComposeResult:
        from ytui.config.profiles import list_profiles, BUILTIN_PROFILES
        from textual.widgets import Label
        # Sanitize preset names into valid widget IDs (no spaces/em-dashes).
        self._builtin_ids = {self._slug(n): n for n in BUILTIN_PROFILES}
        yield Label("Built-in presets:")
        for slug, name in self._builtin_ids.items():
            yield Button(f"Load: {name}", id=f"pf-builtin-{slug}")
        saved = list_profiles()
        self._saved_ids = {self._slug(n): n for n in saved}
        if saved:
            yield Label("Saved profiles:")
            for slug, name in self._saved_ids.items():
                yield Button(f"Load: {name}", id=f"pf-saved-{slug}")
        yield Label("")
        yield Button("Save current as 'default'", id="pf-save")

    @staticmethod
    def _slug(name: str) -> str:
        import re
        return re.sub(r"[^A-Za-z0-9_-]", "-", name).strip("-") or "preset"

    def on_button_pressed(self, event: Button.Pressed) -> None:
        bid = event.button.id or ""
        from ytui.config.profiles import BUILTIN_PROFILES, save_profile, load_profile
        if bid.startswith("pf-builtin-"):
            slug = bid[len("pf-builtin-"):]
            name = self._builtin_ids.get(slug)
            data = BUILTIN_PROFILES.get(name)
            if data:
                from ytui.config.settings import Settings
                s = Settings._from_dict(data)
                s.save()
                try:
                    app = self.app
                    app.settings = s
                    if app.queue_manager:
                        app.queue_manager.settings = s
                    if hasattr(app, "reload_theme"):
                        app.reload_theme()
                except Exception:
                    pass
                self.app.notify(f"Loaded preset: {name}")
                self.dismiss()
            event.prevent_default()
        elif bid.startswith("pf-saved-"):
            slug = bid[len("pf-saved-"):]
            name = self._saved_ids.get(slug)
            s = load_profile(name)
            if s:
                s.save()
                try:
                    app = self.app
                    app.settings = s
                    if app.queue_manager:
                        app.queue_manager.settings = s
                    if hasattr(app, "reload_theme"):
                        app.reload_theme()
                except Exception:
                    pass
                self.app.notify(f"Loaded profile: {name}")
                self.dismiss()
            event.prevent_default()
        elif bid == "pf-save":
            try:
                save_profile("default", self.app.settings)
                self.app.notify("Saved current settings as 'default'")
            except Exception:
                self.app.notify("Failed to save profile", severity="error")
            event.prevent_default()


class ClipboardPanel(BasePanel):
    """Toggle clipboard watcher."""
    def __init__(self, **kwargs) -> None:
        super().__init__(title="Clipboard Watcher", subtitle="Auto-add copied YouTube URLs", **kwargs)

    def compose_content(self) -> ComposeResult:
        s = _settings()
        from textual.widgets import Label
        state = "ON" if s.advanced.clipboard_watcher else "OFF"
        yield Label(f"Clipboard watcher is currently: {state}")
        yield Button("Toggle watcher", id="cb-toggle")

    def on_button_pressed(self, event: Button.Pressed) -> None:
        if event.button.id == "cb-toggle":
            s = _settings()
            s.advanced.clipboard_watcher = not s.advanced.clipboard_watcher
            s.save()
            try:
                app = self.app
                app.settings = s
                if app.queue_manager:
                    app.queue_manager.settings = s
                if s.advanced.clipboard_watcher and hasattr(app, "start_clipboard_watcher"):
                    app.start_clipboard_watcher()
                elif not s.advanced.clipboard_watcher and hasattr(app, "stop_clipboard_watcher"):
                    app.stop_clipboard_watcher()
            except Exception:
                pass
            self.app.notify(f"Clipboard watcher {'enabled' if s.advanced.clipboard_watcher else 'disabled'}")
            self.dismiss()
            event.prevent_default()


class LogPanel(BasePanel):
    """Diagnostics log."""
    def __init__(self, **kwargs) -> None:
        super().__init__(title="Diagnostics & Logs", subtitle="Recent log output", **kwargs)

    def compose_content(self) -> ComposeResult:
        from textual.widgets import RichLog
        log = RichLog(id="diag-log", wrap=True, markup=True)
        log.write("ytui diagnostics — recent log lines appear here.")
        try:
            import logging
            handler = getattr(self.app, "_log_handler", None)
            if handler and hasattr(handler, "records"):
                for rec in handler.records[-50:]:
                    log.write(f"[{rec.levelname}] {rec.getMessage()}")
        except Exception:
            pass
        yield log


class QueuePanelModal(BasePanel):
    """Queue management — cancel/pause/resume/clear."""
    def __init__(self, **kwargs) -> None:
        super().__init__(title="Queue Management", subtitle="Pause/resume/cancel downloads", **kwargs)

    def compose_content(self) -> ComposeResult:
        yield Button("Pause all", id="qm-pause")
        yield Button("Resume all", id="qm-resume")
        yield Button("Cancel all (stop)", id="qm-cancel")
        yield Button("Clear finished", id="qm-clear")

    def on_button_pressed(self, event: Button.Pressed) -> None:
        qm = getattr(self.app, "queue_manager", None)
        bid = event.button.id or ""
        if not qm:
            return
        if bid == "qm-pause":
            qm.pause_all()
            self.app.notify("Paused all downloads")
        elif bid == "qm-resume":
            qm.resume_all()
            self.app.notify("Resumed all downloads")
        elif bid == "qm-cancel":
            n = qm.cancel_all()
            self.app.notify(f"Cancelled {n} download(s)")
        elif bid == "qm-clear":
            removed = 0
            for item in list(qm.items):
                if item.state.value in ("done", "error", "cancelled"):
                    qm.on_item_removed(item)
                    qm.items.remove(item)
                    qm.db.remove_queue_item(item.id)
                    removed += 1
            self.app.notify(f"Cleared {removed} finished item(s)")
        if bid.startswith("qm-"):
            self.dismiss()
            event.prevent_default()

from ytui.utils.vpn import ProtonVpnManager, ProtonServer, VpnStatus


class VpnPanel(BasePanel):
    """Dedicated Proton VPN Manager panel with Login/Logged-In status button."""

    def __init__(self, **kwargs) -> None:
        super().__init__(title="Proton VPN Manager", subtitle="Official Proton VPN Linux CLI", **kwargs)
        self._status = ProtonVpnManager.get_status() if not asyncio.get_event_loop().is_running() else VpnStatus(connected=False, details="Checking...")
        self._logged_in = False
        self._account = "Checking..."

    async def on_mount(self) -> None:
        self._status = await ProtonVpnManager.async_get_status()
        self._logged_in, self._account = await ProtonVpnManager.async_check_account()
        try:
            status_txt = self.query_one("#vpn-status-text", Static)
            if self._status.connected:
                status_txt.update(f"Status: CONNECTED ({self._status.server or 'Active'})")
            else:
                status_txt.update("Status: DISCONNECTED")
        except Exception:
            pass

        try:
            login_btn = self.query_one("#vpn-login-btn", Button)
            if self._logged_in:
                login_btn.label = f"Logged In ({self._account})"
                login_btn.variant = "success"
            else:
                login_btn.label = "Sign In to Proton"
                login_btn.variant = "default"
        except Exception:
            pass

        try:
            btn = self.query_one("#toggle-vpn", Button)
            btn.label = "Disconnect" if self._status.connected else "Connect"
            btn.variant = "default" if self._status.connected else "primary"
        except Exception:
            pass

    def compose_content(self) -> ComposeResult:
        from textual.widgets import Select
        if self._status.connected:
            status_str = f"Status: CONNECTED ({self._status.server or 'Active'})"
        else:
            status_str = "Status: DISCONNECTED"
        yield Static(status_str, id="vpn-status-text")

        if ProtonVpnManager.is_installed():
            options = [(srv.label, srv.code) for srv in ProtonVpnManager.SERVERS]
            yield Select(options, value="fastest", id="vpn-profile-select")
        else:
            yield Static(
                "[red]Proton VPN CLI not found.[/red]\n[dim]Install via 'pip install protonvpn-cli' or official package.[/dim]",
                id="vpn-server-text",
            )

        with Horizontal(classes="vpn-btn-row"):
            if self._logged_in:
                yield Button(f"Logged In ({self._account})", id="vpn-login-btn", variant="success")
            else:
                yield Button("Sign In to Proton", id="vpn-login-btn")

            yield Button("Disconnect" if self._status.connected else "Connect", id="toggle-vpn", variant="primary" if not self._status.connected else "default")

    async def on_button_pressed(self, event: Button.Pressed) -> None:
        if event.button.id == "vpn-login-btn":
            event.prevent_default()
            login_btn = self.query_one("#vpn-login-btn", Button)
            self._logged_in, self._account = await ProtonVpnManager.async_check_account()
            if self._logged_in:
                login_btn.label = f"Logged In ({self._account})"
                login_btn.variant = "success"
                self.app.notify(f"Proton VPN: Logged in as {self._account}", severity="information")
            else:
                self.app.notify("Run 'protonvpn signin' in terminal to log in to your Proton account.", severity="warning")

        elif event.button.id == "toggle-vpn":
            event.prevent_default()
            from textual.widgets import Select

            btn = self.query_one("#toggle-vpn", Button)
            status_txt = self.query_one("#vpn-status-text", Static)

            if not ProtonVpnManager.is_installed():
                self.app.notify("Proton VPN CLI is not installed on your system", severity="error")
                return

            self._status = await ProtonVpnManager.async_get_status()

            if self._status.connected:
                btn.label = "Disconnecting..."
                ok, msg = await ProtonVpnManager.async_disconnect()
                self._status = await ProtonVpnManager.async_get_status()
                status_txt.update(f"Status: {'CONNECTED (' + self._status.server + ')' if self._status.connected else 'DISCONNECTED'}")
                btn.label = "Connect" if not self._status.connected else "Disconnect"
                self.app.notify(msg, severity="information" if ok else "error")
            else:
                try:
                    select_widget = self.query_one("#vpn-profile-select", Select)
                    selected_code = str(select_widget.value)
                except Exception:
                    selected_code = "fastest"

                server = next((s for s in ProtonVpnManager.SERVERS if s.code == selected_code), ProtonVpnManager.SERVERS[0])
                btn.label = f"Connecting to {server.label}..."
                ok, msg = await ProtonVpnManager.async_connect(server)
                self._status = await ProtonVpnManager.async_get_status()
                status_txt.update(f"Status: {'CONNECTED (' + (self._status.server or server.label) + ')' if self._status.connected else 'DISCONNECTED'}")
                btn.label = "Disconnect" if self._status.connected else "Connect"
                self.app.notify(msg, severity="information" if ok else "error")

            # Update HeaderBar status badge across app
            try:
                main_screen = self.app.get_screen("MainScreen")
                header = main_screen.query_one("HeaderBar")
                header.vpn_connected = self._status.connected
                header.vpn_label = self._status.server if self._status.connected else ""
            except Exception:
                pass


class SearchPanel(BasePanel):
    """Interactive YouTube search results panel allowing users to choose videos to queue."""

    DEFAULT_CSS = """
    SearchPanel #panel-dialog {
        width: 95;
        max-width: 95%;
        height: 85%;
    }
    SearchPanel #search-status {
        text-align: center;
        padding: 0 0 1 0;
        color: $text-muted;
    }
    SearchPanel SelectionList {
        height: 1fr;
        border: round $border;
        margin-bottom: 1;
    }
    SearchPanel .search-btn-row {
        height: auto;
        align: center middle;
    }
    SearchPanel .search-btn-row Button {
        margin: 0 1;
    }
    """

    def __init__(self, query: str, **kwargs) -> None:
        super().__init__(title="YouTube Search Results", subtitle=f"Query: '{query}'", **kwargs)
        self.query_text = query

    def compose_content(self) -> ComposeResult:
        from textual.widgets import SelectionList
        yield Static(f"Searching YouTube for '{self.query_text}'...", id="search-status")
        yield SelectionList(id="search-selection-list")

        with Horizontal(classes="search-btn-row"):
            yield Button("Add Selected", id="add-selected-btn", variant="primary")
            yield Button("Add All", id="add-all-btn", variant="default")

    def on_mount(self) -> None:
        import asyncio
        asyncio.create_task(self._fetch_results())

    async def _fetch_results(self) -> None:
        import asyncio
        status = self.query_one("#search-status", Static)
        slist = self.query_one("#search-selection-list")

        engine = getattr(self.app, "engine", None)
        if not engine:
            from ytui.engine.downloader import DownloadEngine
            from ytui.config.settings import Settings
            engine = DownloadEngine(Settings.load())

        try:
            info = await asyncio.get_event_loop().run_in_executor(
                None, lambda: engine.extract_info(f"ytsearch10:{self.query_text}", flat=True)
            )
            raw_entries = info.get("entries", []) if info else []
            entries = [e for e in raw_entries if e]

            if not entries:
                status.update(f"[red]No YouTube results found for '{self.query_text}'[/red]")
                return

            status.update(f"[green]Found {len(entries)} result(s).[/green] Use Space/Click to select, then click 'Add Selected':")
            from textual.widgets._selection_list import Selection
            from ytui.utils.formatting import format_duration

            for entry in entries:
                v_title = entry.get("title", "Untitled Video")
                v_uploader = entry.get("uploader") or entry.get("channel") or "YouTube"
                dur_sec = entry.get("duration") or 0
                dur_str = format_duration(dur_sec) if dur_sec else ""
                url = entry.get("url") or entry.get("webpage_url") or ""
                if not url and entry.get("id"):
                    url = f"https://www.youtube.com/watch?v={entry['id']}"

                label = f"{v_title} • {v_uploader}"
                if dur_str:
                    label += f" [{dur_str}]"

                if url:
                    slist.add_option(Selection(label, url, False))
        except Exception as e:
            status.update(f"[red]Search failed: {e}[/red]")

    async def on_button_pressed(self, event: Button.Pressed) -> None:
        if event.button.id == "add-selected-btn":
            event.prevent_default()
            from textual.widgets import SelectionList
            slist = self.query_one("#search-selection-list", SelectionList)
            selected = slist.selected
            if not selected:
                self.app.notify("No videos selected. Check items or click 'Add All'.", severity="warning")
                return

            for url in selected:
                self.app.queue_manager_add(url)
            self.app.notify(f"Added {len(selected)} video(s) to download queue", severity="information")
            self.dismiss()

        elif event.button.id == "add-all-btn":
            event.prevent_default()
            from textual.widgets import SelectionList
            slist = self.query_one("#search-selection-list", SelectionList)
            added = 0
            for opt in slist._options:
                url = opt.value
                if url:
                    self.app.queue_manager_add(url)
                    added += 1
            if added:
                self.app.notify(f"Added all {added} video(s) to download queue", severity="information")
            self.dismiss()
        # For close-btn (and any other button): let the MRO reach BasePanel.

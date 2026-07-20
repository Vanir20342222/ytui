"""Header status bar — always visible at the top.

Shows: active/paused download counts, VPN status, bandwidth cap, profile name.
"""

from __future__ import annotations

from textual.app import ComposeResult
from textual.reactive import reactive
from textual.widget import Widget
from textual.widgets import Static

from ytui.constants import APP_VERSION, ICON_SETS


class HeaderBar(Widget):
    """Slim status header bar."""

    DEFAULT_CSS = """
    HeaderBar {
        dock: top;
        height: 1;
        background: $surface;
        color: $text-muted;
        layout: horizontal;
        padding: 0 1;
    }
    HeaderBar #hdr-left {
        width: 1fr;
    }
    HeaderBar #hdr-right {
        width: auto;
        text-align: right;
    }
    """

    active_count = reactive(0)
    paused_count = reactive(0)
    completed_count = reactive(0)
    total_count = reactive(0)
    vpn_connected = reactive(False)
    vpn_label = reactive("")
    bandwidth_cap = reactive("")
    profile_name = reactive("")
    total_speed = reactive("")

    def compose(self) -> ComposeResult:
        yield Static(id="hdr-left")
        yield Static(id="hdr-right")

    def _build_left(self) -> str:
        icon_style = "badges"
        if hasattr(self.app, "settings") and self.app.settings and hasattr(self.app.settings, "appearance"):
            icon_style = getattr(self.app.settings.appearance, "icon_style", "badges")
        icon_map = ICON_SETS.get(icon_style, ICON_SETS["badges"])

        parts = [f" ytui v{APP_VERSION}"]
        if self.total_count > 0:
            parts.append(f"  │  QUEUE: {self.total_count}")
        if self.active_count > 0:
            parts.append(f"  {icon_map.get('DOWNLOADING', 'v')} {self.active_count} active")
        if self.paused_count > 0:
            parts.append(f"  {icon_map.get('PAUSED', '||')} {self.paused_count} paused")
        if self.completed_count > 0:
            parts.append(f"  {icon_map.get('DONE', '+')} {self.completed_count} done")
        if self.total_speed:
            parts.append(f"  {icon_map.get('DOWNLOADING', 'v')} {self.total_speed}")
        return "".join(parts)

    def _build_right(self) -> str:
        parts = []
        if self.bandwidth_cap:
            parts.append(f"BW: {self.bandwidth_cap}")
        if self.vpn_label:
            dot = "*" if self.vpn_connected else "o"
            parts.append(f"{dot} VPN: {self.vpn_label}")
        elif self.vpn_connected:
            parts.append("* VPN")
        if self.profile_name:
            parts.append(f"[{self.profile_name}]")
        return "  |  ".join(parts) + " " if parts else ""

    def watch_active_count(self) -> None:
        self._refresh_display()

    def watch_paused_count(self) -> None:
        self._refresh_display()

    def watch_completed_count(self) -> None:
        self._refresh_display()

    def watch_total_count(self) -> None:
        self._refresh_display()

    def watch_vpn_connected(self) -> None:
        self._refresh_display()

    def watch_vpn_label(self) -> None:
        self._refresh_display()

    def watch_bandwidth_cap(self) -> None:
        self._refresh_display()

    def watch_profile_name(self) -> None:
        self._refresh_display()

    def watch_total_speed(self) -> None:
        self._refresh_display()

    def _refresh_display(self) -> None:
        try:
            self.query_one("#hdr-left", Static).update(self._build_left())
            self.query_one("#hdr-right", Static).update(self._build_right())
        except Exception:
            pass

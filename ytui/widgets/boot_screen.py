"""Boot screen with animated wordmark and dependency checks."""

from __future__ import annotations

import asyncio
import shutil
import sys

from textual.app import ComposeResult
from textual.containers import Center, Vertical
from textual.screen import Screen
from textual.widgets import Static

from ytui.constants import APP_VERSION


LOGO = r"""
         _         _
  _   _ | |_ _   _(_)
 | | | || __| | | | |
 | |_| || |_| |_| | |
  \__, | \__|\__,_|_|
  |___/
"""


class BootScreen(Screen):
    """Animated boot screen with dependency checks."""

    DEFAULT_CSS = """
    BootScreen {
        align: center middle;
        background: $background;
    }
    BootScreen #boot-container {
        width: auto;
        max-width: 100%;
        height: auto;
        padding: 1 2;
    }
    BootScreen #boot-logo {
        text-align: center;
        color: $accent;
        text-style: bold;
    }
    BootScreen #boot-version {
        text-align: center;
        color: $text-muted;
        margin: 0 0 1 0;
    }
    BootScreen .check-line {
        height: 1;
        padding: 0 2;
    }
    BootScreen .check--ok {
        color: $success;
    }
    BootScreen .check--fail {
        color: $error;
    }
    BootScreen .check--pending {
        color: $text-muted;
    }
    """

    def compose(self) -> ComposeResult:
        with Center():
            with Vertical(id="boot-container"):
                yield Static(LOGO, id="boot-logo")
                yield Static(
                    f"Terminal YouTube & Audio Downloader  v{APP_VERSION}",
                    id="boot-version",
                )
                yield Static("  . Python...", classes="check-line check--pending", id="chk-python")
                yield Static("  . yt-dlp...", classes="check-line check--pending", id="chk-ytdlp")
                yield Static("  . ffmpeg...", classes="check-line check--pending", id="chk-ffmpeg")

    async def on_mount(self) -> None:
        """Run dependency checks with fast animated reveals."""
        loop = asyncio.get_running_loop()

        # Python check
        py_version = f"{sys.version_info.major}.{sys.version_info.minor}.{sys.version_info.micro}"
        self._set_check("chk-python", True, f"Python {py_version}")
        await asyncio.sleep(0.05)

        # yt-dlp check
        try:
            import yt_dlp
            version = getattr(yt_dlp.version, "__version__", "installed")
            self._set_check("chk-ytdlp", True, f"yt-dlp {version}")
        except ImportError:
            self._set_check("chk-ytdlp", False, "yt-dlp not found")
        await asyncio.sleep(0.05)

        # ffmpeg check (non-blocking executor)
        ffmpeg_path = shutil.which("ffmpeg")
        if ffmpeg_path:
            from ytui.engine.ffmpeg import get_ffmpeg_version
            ff_ver = await loop.run_in_executor(None, get_ffmpeg_version) or "found"
            self._set_check("chk-ffmpeg", True, f"ffmpeg {ff_ver}")
        else:
            self._set_check("chk-ffmpeg", False, "ffmpeg not found (optional)")

        await asyncio.sleep(0.1)

        # Transition to main screen
        self.dismiss()

    def _set_check(self, widget_id: str, ok: bool, label: str) -> None:
        """Update a check line with result."""
        try:
            widget = self.query_one(f"#{widget_id}", Static)
            icon = "+" if ok else "x"
            widget.update(f"  {icon} {label}")
            widget.remove_class("check--pending")
            widget.add_class("check--ok" if ok else "check--fail")
        except Exception:
            pass

"""ASCII Art logo banner widget for MainScreen."""

from __future__ import annotations

from textual.app import ComposeResult
from textual.widget import Widget
from textual.widgets import Static

ASCII_LOGO = r"""  ██╗   ██╗████████╗██╗   ██╗██╗
  ╚██╗ ██╔╝╚══██╔══╝██║   ██║██║
   ╚████╔╝    ██║   ██║   ██║██║
    ╚██╔╝     ██║   ██║   ██║██║
     ██║      ██║   ╚██████╔╝██║
     ╚═╝      ╚═╝    ╚═════╝ ╚═╝"""

SUBTITLE = "[dim]─────────────── Terminal YouTube Downloader ───────────────[/dim]"


class AsciiBanner(Widget):
    """Upper third ASCII logo banner."""

    DEFAULT_CSS = """
    AsciiBanner {
        height: 1fr;
        min-height: 5;
        max-height: 9;
        align: center middle;
        content-align: center middle;
        background: $background;
        padding: 0;
    }
    AsciiBanner #banner-logo {
        text-align: center;
        color: $accent;
        text-style: bold;
        width: 100%;
    }
    AsciiBanner #banner-subtitle {
        text-align: center;
        color: $text-muted;
        width: 100%;
    }
    """

    def compose(self) -> ComposeResult:
        yield Static(ASCII_LOGO, id="banner-logo")
        yield Static(SUBTITLE, id="banner-subtitle")

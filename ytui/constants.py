"""Application constants and defaults."""

from __future__ import annotations

APP_NAME = "ytui"
APP_VERSION = "0.1.0"
APP_TITLE = f"ytui v{APP_VERSION}"

# Quality presets
VIDEO_QUALITY_OPTIONS = [
    ("best", "Best Available"),
    ("2160", "4K (2160p)"),
    ("1440", "2K (1440p)"),
    ("1080", "Full HD (1080p)"),
    ("720", "HD (720p)"),
    ("480", "SD (480p)"),
    ("360", "Low (360p)"),
]

AUDIO_QUALITY_OPTIONS = [
    ("best", "Best Available"),
    ("lossless", "Lossless (FLAC/Opus copy)"),
    ("320", "320 kbps"),
    ("256", "256 kbps"),
    ("192", "192 kbps"),
    ("128", "128 kbps"),
]

VIDEO_CONTAINERS = ["mp4", "mkv", "webm"]
AUDIO_CONTAINERS = ["mp3", "m4a", "flac", "opus", "ogg", "wav", "aac"]

# yt-dlp format strings for quality ceilings
VIDEO_FORMAT_MAP = {
    "best": "bestvideo+bestaudio/best",
    "2160": "bestvideo[height<=2160]+bestaudio/best[height<=2160]",
    "1440": "bestvideo[height<=1440]+bestaudio/best[height<=1440]",
    "1080": "bestvideo[height<=1080]+bestaudio/best[height<=1080]",
    "720": "bestvideo[height<=720]+bestaudio/best[height<=720]",
    "480": "bestvideo[height<=480]+bestaudio/best[height<=480]",
    "360": "bestvideo[height<=360]+bestaudio/best[height<=360]",
}

AUDIO_FORMAT_MAP = {
    "best": "bestaudio/best",
    "lossless": "bestaudio/best",
    "320": "bestaudio/best",
    "256": "bestaudio/best",
    "192": "bestaudio/best",
    "128": "bestaudio/best",
}

# Filename template variables
FILENAME_TEMPLATE_DEFAULT = "%(title)s.%(ext)s"
FILENAME_TEMPLATE_VARS = [
    "%(title)s", "%(uploader)s", "%(upload_date)s", "%(id)s",
    "%(playlist_index)s", "%(playlist_title)s", "%(duration)s",
    "%(resolution)s", "%(ext)s", "%(channel)s",
]

# Icon preset choices and set definitions
ICON_STYLES = [
    ("badges", "Color Text Badges ([DONE], [DOWN])"),
    ("nerdfont", "Nerd Font Glyphs (󰄬, 󰇚)"),
    ("unicode", "Clean Unicode (✓, ↓, ❚❚)"),
    ("ascii", "Minimal ASCII (+, v, ||)"),
]

ICON_SETS = {
    "badges": {
        "PENDING": "[dim]FETCH[/]",
        "QUEUED": "[cyan]QUEUE[/]",
        "DOWNLOADING": "[bold green]DLOAD[/]",
        "CONVERTING": "[yellow]CONVT[/]",
        "MERGING": "[blue]MERGE[/]",
        "DONE": "[bold green]DONE [/]",
        "PAUSED": "[yellow]PAUSE[/]",
        "ERROR": "[bold red]ERROR[/]",
        "CANCELLED": "[dim red]CANCL[/]",
    },
    "nerdfont": {
        "PENDING": "󰑮",
        "QUEUED": "󰐊",
        "DOWNLOADING": "󰇚",
        "CONVERTING": "󰑤",
        "MERGING": "󰓦",
        "DONE": "󰄬",
        "PAUSED": "󰏤",
        "ERROR": "󰅖",
        "CANCELLED": "󰅙",
    },
    "unicode": {
        "PENDING": "◌",
        "QUEUED": "▸",
        "DOWNLOADING": "↓",
        "CONVERTING": "↻",
        "MERGING": "⇌",
        "DONE": "✓",
        "PAUSED": "❚❚",
        "ERROR": "✗",
        "CANCELLED": "−",
    },
    "ascii": {
        "PENDING": "..",
        "QUEUED": ">",
        "DOWNLOADING": "v",
        "CONVERTING": "~",
        "MERGING": "=",
        "DONE": "+",
        "PAUSED": "||",
        "ERROR": "x",
        "CANCELLED": "-",
    },
}

# Download states
class DownloadState:
    PENDING = "pending"        # shimmer/loading metadata
    QUEUED = "queued"          # metadata resolved, waiting
    DOWNLOADING = "downloading"
    CONVERTING = "converting"
    MERGING = "merging"
    DONE = "done"
    PAUSED = "paused"
    ERROR = "error"

# Commands registry
COMMANDS = {
    "/quality": "Video & audio quality ceilings",
    "/dir": "Target directory settings",
    "/audio": "Switch to audio-only mode",
    "/video": "Switch to video mode",
    "/audio-playlist": "Audio mode + whole-playlist downloads",
    "/video-playlist": "Video mode + whole-playlist downloads",
    "/convert": "Convert a local file",
    "/queue": "Manage the download queue",
    "/history": "Download history",
    "/settings": "Application settings",
    "/bandwidth": "Bandwidth limiting",
    "/limit": "Bandwidth limiting",
    "/vpn": "VPN connection manager",
    "/proxy": "Proxy configuration",
    "/subs": "Subtitle settings",
    "/subtitles": "Subtitle settings",
    "/metadata": "Metadata & tagging",
    "/theme": "Color themes & appearance",
    "/keys": "Keybinding editor",
    "/profile": "Settings profiles",
    "/schedule": "Download scheduling",
    "/hooks": "Post-download hooks",
    "/search": "YouTube search",
    "/clipboard": "Toggle clipboard watcher",
    "/update": "Update yt-dlp engine",
    "/log": "Diagnostics & logs",
    "/stats": "Download statistics",
    "/help": "Help & reference",
    "/?": "Help & reference",
    "/quit": "Exit ytui",
    "/q": "Exit ytui",
    "/clear": "Clear finished items from queue",
    "/pauseall": "Pause all active downloads",
    "/resumeall": "Resume all paused downloads",
    "/stopall": "Cancel all active & queued downloads",
}

# URL patterns
YOUTUBE_PATTERNS = [
    r"(?:https?://)?(?:www\.)?youtube\.com/watch\?v=([\w-]+)",
    r"(?:https?://)?(?:www\.)?youtube\.com/playlist\?list=([\w-]+)",
    r"(?:https?://)?(?:www\.)?youtube\.com/channel/([\w-]+)",
    r"(?:https?://)?(?:www\.)?youtube\.com/@([\w.-]+)",
    r"(?:https?://)?(?:www\.)?youtube\.com/shorts/([\w-]+)",
    r"(?:https?://)?youtu\.be/([\w-]+)",
    r"(?:https?://)?(?:www\.)?youtube\.com/live/([\w-]+)",
    r"(?:https?://)?music\.youtube\.com/watch\?v=([\w-]+)",
]

SUPPORTED_URL_SCHEMES = ("http://", "https://")

# First-run notice
FIRST_RUN_NOTICE = """Welcome to ytui!

This tool uses yt-dlp to download publicly available content from YouTube.
It does not circumvent DRM (Widevine-protected, rental, or premium-only
streams are not supported).

Please only download content you have the rights to use offline.
"""

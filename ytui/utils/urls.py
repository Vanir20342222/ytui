"""URL validation and YouTube URL pattern detection."""

from __future__ import annotations

import re
from urllib.parse import urlparse

from ytui.constants import SUPPORTED_URL_SCHEMES, YOUTUBE_PATTERNS


def is_valid_url(url: str) -> bool:
    """Check if a string is a valid HTTP/HTTPS URL."""
    url = url.strip()
    if not any(url.startswith(scheme) for scheme in SUPPORTED_URL_SCHEMES):
        # Try adding https:// for shorthand URLs like youtu.be/xxx
        if is_youtube_url(f"https://{url}"):
            return True
        return False
    try:
        result = urlparse(url)
        return all([result.scheme, result.netloc])
    except Exception:
        return False


def is_youtube_url(url: str) -> bool:
    """Check if a URL matches any known YouTube pattern."""
    for pattern in YOUTUBE_PATTERNS:
        if re.search(pattern, url):
            return True
    return False


def normalize_url(url: str) -> str:
    """Normalize a URL by adding https:// if missing, skipping search prefixes and local paths."""
    url = url.strip()
    if url.startswith("ytsearch") or url.startswith("/") or url.startswith("."):
        return url
    if not url.startswith(("http://", "https://")):
        url = f"https://{url}"
    return url


def extract_video_id(url: str) -> str | None:
    """Extract the video ID from a YouTube URL."""
    patterns = [
        r"(?:v=|/v/|youtu\.be/|/embed/|/shorts/|/live/)([\w-]{11})",
        r"^([\w-]{11})$",  # bare video ID
    ]
    for pattern in patterns:
        match = re.search(pattern, url)
        if match:
            return match.group(1)
    return None


def extract_playlist_id(url: str) -> str | None:
    """Extract the playlist ID from a YouTube URL."""
    match = re.search(r"list=([\w-]+)", url)
    return match.group(1) if match else None


def is_playlist_url(url: str) -> bool:
    """Check if URL is a YouTube playlist (not an auto-generated radio mix)."""
    if "playlist?list=" in url:
        return True
    if "list=" in url:
        # YouTube auto-generated radio mixes (list=RD...) and "Mix" lists are
        # recommendations, not real playlists — they should be treated as the
        # single referenced video so we don't dump the whole recommendation
        # queue. Real playlists use IDs like PL..., UU..., FL..., LL..., OL...
        playlist_id = extract_playlist_id(url) or ""
        if playlist_id.upper().startswith("RD"):
            return False
        return True
    return False


def strip_playlist_param(url: str) -> str:
    """Strip the `list=` query param from a URL (keep the video)."""
    import re
    return re.sub(r"[?&]list=[^&]*", "", url)


def is_radio_mix(url: str) -> bool:
    """Check if URL references a YouTube auto-generated radio mix (RD...)."""
    playlist_id = extract_playlist_id(url) or ""
    return bool(playlist_id) and playlist_id.upper().startswith("RD")


def is_channel_url(url: str) -> bool:
    """Check if URL is a YouTube channel."""
    return bool(re.search(r"youtube\.com/(?:channel/|@|c/)", url))

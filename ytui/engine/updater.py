"""yt-dlp self-update from official GitHub releases."""

from __future__ import annotations

import logging
import subprocess
import sys

import yt_dlp

logger = logging.getLogger(__name__)


def get_current_version() -> str:
    """Get the currently installed yt-dlp version."""
    try:
        return yt_dlp.version.__version__
    except Exception:
        return "unknown"


async def check_for_update() -> tuple[str, str, bool]:
    """Check for a newer yt-dlp version.
    
    Returns:
        Tuple of (current_version, latest_version, update_available).
    """
    import httpx
    
    current = get_current_version()
    try:
        async with httpx.AsyncClient() as client:
            resp = await client.get(
                "https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest",
                follow_redirects=True,
                timeout=15,
            )
            resp.raise_for_status()
            data = resp.json()
            latest = data.get("tag_name", "").lstrip("v")
            return current, latest, latest != current
    except Exception as e:
        logger.error(f"Update check failed: {e}")
        return current, "unknown", False


def update_ytdlp() -> tuple[bool, str]:
    """Update yt-dlp via pip.
    
    Returns:
        Tuple of (success, message).
    """
    try:
        result = subprocess.run(
            [sys.executable, "-m", "pip", "install", "-U", "yt-dlp"],
            capture_output=True,
            text=True,
            timeout=120,
        )
        if result.returncode == 0:
            msg = "yt-dlp updated successfully. Restart ytui to use the new version."
            logger.info(msg)
            return True, msg
        else:
            msg = f"Update failed: {result.stderr[:200]}"
            logger.error(msg)
            return False, msg
    except subprocess.TimeoutExpired:
        return False, "Update timed out"
    except Exception as e:
        return False, f"Update error: {e}"

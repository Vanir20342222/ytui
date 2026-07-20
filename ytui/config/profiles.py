"""Named settings profiles — save/load/switch bundles of settings."""

from __future__ import annotations

import json
import logging
from pathlib import Path
from typing import Any

from ytui.config.settings import Settings, _config_dir

logger = logging.getLogger(__name__)


def _profiles_dir() -> Path:
    d = _config_dir() / "profiles"
    d.mkdir(parents=True, exist_ok=True)
    return d


def list_profiles() -> list[str]:
    """List all saved profile names."""
    profiles_dir = _profiles_dir()
    return sorted(
        p.stem for p in profiles_dir.glob("*.json")
    )


def _is_safe_name(name: str) -> bool:
    if not name or Path(name).name != name or ".." in name or "/" in name or "\\" in name:
        return False
    return True


def save_profile(name: str, settings: Settings) -> None:
    """Save current settings as a named profile."""
    if not _is_safe_name(name):
        logger.error(f"Invalid or path traversal profile name: '{name}'")
        return
    path = _profiles_dir() / f"{name}.json"
    if path.resolve().parent != _profiles_dir().resolve():
        logger.error(f"Path traversal detected in profile name: '{name}'")
        return
    with open(path, "w") as f:
        json.dump(settings._to_dict(), f, indent=2)
    logger.info(f"Profile '{name}' saved to {path}")


def load_profile(name: str) -> Settings | None:
    """Load a named profile, returning None if not found."""
    if not _is_safe_name(name):
        logger.warning(f"Invalid or path traversal profile name: '{name}'")
        return None
    path = _profiles_dir() / f"{name}.json"
    if path.resolve().parent != _profiles_dir().resolve():
        logger.warning(f"Path traversal detected in profile name: '{name}'")
        return None
    if not path.exists():
        logger.warning(f"Profile '{name}' not found")
        return None
    try:
        with open(path, "r") as f:
            data = json.load(f)
        return Settings._from_dict(data)
    except Exception as e:
        logger.error(f"Failed to load profile '{name}': {e}")
        return None


def delete_profile(name: str) -> bool:
    """Delete a named profile."""
    if not _is_safe_name(name):
        logger.warning(f"Invalid or path traversal profile name: '{name}'")
        return False
    path = _profiles_dir() / f"{name}.json"
    if path.resolve().parent != _profiles_dir().resolve():
        logger.warning(f"Path traversal detected in profile name: '{name}'")
        return False
    if path.exists():
        path.unlink()
        logger.info(f"Profile '{name}' deleted")
        return True
    return False


# Built-in profile presets
BUILTIN_PROFILES: dict[str, dict[str, Any]] = {
    "Archival — Max Quality": {
        "quality": {
            "video_quality": "best",
            "audio_quality": "lossless",
            "video_container": "mkv",
            "audio_container": "flac",
            "prefer_hfr": True,
            "download_mode": "video",
        },
        "metadata": {
            "embed_thumbnail": True,
            "write_metadata": True,
        },
        "subtitles": {
            "enabled": True,
            "embed": True,
        },
    },
    "Quick — Small Files": {
        "quality": {
            "video_quality": "720",
            "audio_quality": "128",
            "video_container": "mp4",
            "audio_container": "mp3",
            "prefer_hfr": False,
            "download_mode": "video",
        },
    },
    "Music Only": {
        "quality": {
            "audio_quality": "320",
            "audio_container": "mp3",
            "download_mode": "audio",
        },
        "metadata": {
            "embed_thumbnail": True,
            "write_metadata": True,
        },
    },
    "Podcast": {
        "quality": {
            "audio_quality": "192",
            "audio_container": "mp3",
            "download_mode": "audio",
        },
    },
}

"""Settings management with OS-correct paths and TOML persistence."""

from __future__ import annotations

import json
import logging
from dataclasses import dataclass, field, asdict
from pathlib import Path
from typing import Any

from platformdirs import user_config_dir, user_data_dir, user_cache_dir

from ytui.constants import APP_NAME, FILENAME_TEMPLATE_DEFAULT

logger = logging.getLogger(__name__)


def _config_dir() -> Path:
    return Path(user_config_dir(APP_NAME, ensure_exists=True))


def _data_dir() -> Path:
    return Path(user_data_dir(APP_NAME, ensure_exists=True))


def _cache_dir() -> Path:
    return Path(user_cache_dir(APP_NAME, ensure_exists=True))


@dataclass
class QualitySettings:
    """Video and audio quality settings."""
    video_quality: str = "best"  # best, 2160, 1440, 1080, 720, 480, 360
    audio_quality: str = "best"  # best, lossless, 320, 256, 192, 128
    video_container: str = "mp4"
    audio_container: str = "mp3"
    prefer_hfr: bool = True  # Prefer high frame rate when available
    download_mode: str = "video"  # "video" or "audio"
    # When True, pasting a playlist URL downloads every item in the playlist.
    # When False, only the referenced video is downloaded (playlist stripped).
    playlist_mode: bool = True
    # Auto-convert downloaded videos to `video_container` via FFmpegVideoConvertor.
    auto_convert_video: bool = False


@dataclass
class DirectorySettings:
    """Download directory settings."""
    video_dir: str = ""
    audio_dir: str = ""
    recent_dirs: list[str] = field(default_factory=list)
    open_after_download: bool = False

    def __post_init__(self):
        if not self.video_dir:
            self.video_dir = str(Path.home() / "Videos" / "ytui")
        if not self.audio_dir:
            self.audio_dir = str(Path.home() / "Music" / "ytui")


@dataclass
class NetworkSettings:
    """Network and bandwidth settings."""
    max_concurrent_downloads: int = 3
    global_bandwidth_limit: int = 0  # KB/s, 0 = unlimited
    per_download_bandwidth_limit: int = 0
    proxy_url: str = ""
    proxy_type: str = ""  # http, socks5
    browser_cookies: str = ""  # Browser name for cookie import


@dataclass
class BandwidthSchedule:
    """A scheduled bandwidth limit."""
    start_hour: int = 9
    end_hour: int = 18
    limit_kbps: int = 500
    enabled: bool = False


@dataclass
class AppearanceSettings:
    """Theme and UI settings."""
    theme: str = "default"
    icon_style: str = "badges"  # badges, nerdfont, unicode, ascii
    ui_density: str = "normal"  # compact, normal, comfortable
    show_thumbnails: bool = True
    animations_enabled: bool = True


@dataclass
class SubtitleSettings:
    """Subtitle download settings."""
    enabled: bool = False
    languages: list[str] = field(default_factory=lambda: ["en"])
    embed: bool = True  # Embed vs sidecar
    format: str = "srt"  # srt, vtt, ass
    auto_translate: bool = False
    auto_translate_lang: str = "en"


@dataclass
class MetadataSettings:
    """Metadata and tagging settings."""
    embed_thumbnail: bool = True
    write_metadata: bool = True
    filename_template: str = FILENAME_TEMPLATE_DEFAULT


@dataclass
class AdvancedSettings:
    """Advanced settings."""
    clipboard_watcher: bool = False
    desktop_notifications: bool = True
    notification_sound: bool = False
    auto_retry_count: int = 3
    retry_delay_seconds: int = 5
    skip_duplicates: bool = True
    sponsor_block: bool = False
    sponsor_block_action: str = "mark"  # mark, skip
    chapter_split: bool = False
    first_run_done: bool = False


@dataclass
class Settings:
    """Complete application settings."""
    quality: QualitySettings = field(default_factory=QualitySettings)
    directories: DirectorySettings = field(default_factory=DirectorySettings)
    network: NetworkSettings = field(default_factory=NetworkSettings)
    bandwidth_schedule: BandwidthSchedule = field(default_factory=BandwidthSchedule)
    appearance: AppearanceSettings = field(default_factory=AppearanceSettings)
    subtitles: SubtitleSettings = field(default_factory=SubtitleSettings)
    metadata: MetadataSettings = field(default_factory=MetadataSettings)
    advanced: AdvancedSettings = field(default_factory=AdvancedSettings)

    @classmethod
    def config_path(cls) -> Path:
        return _config_dir() / "settings.json"

    @classmethod
    def data_path(cls) -> Path:
        return _data_dir()

    @classmethod
    def cache_path(cls) -> Path:
        return _cache_dir()

    @classmethod
    def load(cls) -> Settings:
        """Load settings from disk, returning defaults if not found."""
        path = cls.config_path()
        if not path.exists():
            logger.info("No settings file found, using defaults")
            return cls()

        try:
            with open(path, "r") as f:
                data = json.load(f)
            return cls._from_dict(data)
        except Exception as e:
            logger.error(f"Failed to load settings: {e}")
            return cls()

    def save(self) -> None:
        """Persist settings to disk atomically."""
        path = self.config_path()
        path.parent.mkdir(parents=True, exist_ok=True)
        tmp_path = path.with_suffix(".tmp")
        try:
            with open(tmp_path, "w") as f:
                json.dump(self._to_dict(), f, indent=2)
            import os
            os.replace(tmp_path, path)
            logger.info(f"Settings saved to {path}")
        except Exception as e:
            if tmp_path.exists():
                try:
                    tmp_path.unlink()
                except Exception:
                    pass
            logger.error(f"Failed to save settings: {e}")

    def _to_dict(self) -> dict[str, Any]:
        d = asdict(self)
        try:
            from textual.widgets import Select
            sentinels = (Select.BLANK, Select.NULL)
        except ImportError:
            sentinels = ()

        def _clean(val: Any) -> Any:
            if sentinels and val in sentinels:
                return ""
            if isinstance(val, dict):
                return {k: _clean(v) for k, v in val.items()}
            if isinstance(val, list):
                return [_clean(x) for x in val]
            return val

        return _clean(d)

    @classmethod
    def _from_dict(cls, data: dict[str, Any]) -> Settings:
        """Reconstruct Settings from a dict, filling defaults for missing keys."""
        from dataclasses import fields, MISSING

        def _coerce(value: Any, expected_type: Any, default: Any) -> Any:
            try:
                from textual.widgets import Select
                if value is Select.BLANK or value is Select.NULL:
                    return ""
            except ImportError:
                pass

            origin = getattr(expected_type, "__origin__", expected_type)
            if origin is list:
                if not isinstance(value, list):
                    return default
                args = getattr(expected_type, "__args__", ())
                el_type = args[0] if args else str
                return [_coerce(el, el_type, el_type()) for el in value]
            if origin is bool:
                if isinstance(value, bool):
                    return value
                if isinstance(value, str):
                    return value.lower() in ("true", "1", "yes", "on")
                return bool(value)
            if origin is int:
                try:
                    return int(value)
                except (ValueError, TypeError):
                    return default
            if origin is float:
                try:
                    return float(value)
                except (ValueError, TypeError):
                    return default
            if origin is str:
                if value is None:
                    return default
                return str(value)
            return value

        def _filter_dict(dataclass_cls: type, d: dict[str, Any]) -> dict[str, Any]:
            if not isinstance(d, dict):
                return {}
            res = {}
            for f in fields(dataclass_cls):
                if f.default is not MISSING:
                    default_val = f.default
                elif f.default_factory is not MISSING:
                    default_val = f.default_factory()
                else:
                    default_val = None
                if f.name in d:
                    res[f.name] = _coerce(d[f.name], f.type, default_val)
                else:
                    res[f.name] = default_val
            return res

        return cls(
            quality=QualitySettings(**_filter_dict(QualitySettings, data.get("quality", {}))),
            directories=DirectorySettings(**_filter_dict(DirectorySettings, data.get("directories", {}))),
            network=NetworkSettings(**_filter_dict(NetworkSettings, data.get("network", {}))),
            bandwidth_schedule=BandwidthSchedule(**_filter_dict(BandwidthSchedule, data.get("bandwidth_schedule", {}))),
            appearance=AppearanceSettings(**_filter_dict(AppearanceSettings, data.get("appearance", {}))),
            subtitles=SubtitleSettings(**_filter_dict(SubtitleSettings, data.get("subtitles", {}))),
            metadata=MetadataSettings(**_filter_dict(MetadataSettings, data.get("metadata", {}))),
            advanced=AdvancedSettings(**_filter_dict(AdvancedSettings, data.get("advanced", {}))),
        )

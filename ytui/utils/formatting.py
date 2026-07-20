"""Human-readable formatting utilities."""

from __future__ import annotations

import math
from collections import deque
from datetime import timedelta


def format_size(size_bytes: int | float, precision: int = 1) -> str:
    """Format bytes into human-readable size string.

    Automatically scales: B → KB → MB → GB → TB.
    """
    if size_bytes <= 0:
        return "0 B"
    if size_bytes < 1:
        return f"{size_bytes:.{precision}f} B"

    units = ["B", "KB", "MB", "GB", "TB"]
    exponent = min(max(0, int(math.log(size_bytes, 1024))), len(units) - 1)
    value = size_bytes / (1024 ** exponent)
    return f"{value:.{precision}f} {units[exponent]}"


def format_speed(bytes_per_second: float) -> str:
    """Format download speed with auto-scaling.

    Shows KB/s for speeds < 1 MB/s, MB/s otherwise.
    """
    if bytes_per_second <= 0:
        return "0 KB/s"

    if bytes_per_second < 1024 * 1024:
        return f"{bytes_per_second / 1024:.1f} KB/s"
    return f"{bytes_per_second / (1024 * 1024):.1f} MB/s"


def format_duration(seconds: int | float) -> str:
    """Format seconds into H:MM:SS or M:SS string."""
    if seconds < 0:
        return "--:--"

    seconds = int(seconds)
    if seconds >= 3600:
        h = seconds // 3600
        m = (seconds % 3600) // 60
        s = seconds % 60
        return f"{h}:{m:02d}:{s:02d}"
    else:
        m = seconds // 60
        s = seconds % 60
        return f"{m}:{s:02d}"


def format_eta(seconds: float | None) -> str:
    """Format ETA with 'ETA' prefix."""
    if seconds is None or seconds < 0:
        return "ETA --:--"
    return f"ETA {format_duration(seconds)}"


class SmoothedETA:
    """Rolling-average ETA calculator that avoids jumpy estimates.

    Uses a windowed moving average of recent speeds to produce
    stable ETA values instead of instant math.
    """

    def __init__(self, window_size: int = 10):
        self._speeds: deque[float] = deque(maxlen=window_size)
        self._last_downloaded: float = 0
        self._last_timestamp: float = 0

    def update(self, downloaded: float, total: float, timestamp: float) -> float | None:
        """Update with new progress data and return smoothed ETA in seconds."""
        if total <= 0:
            return None

        if self._last_timestamp > 0:
            dt = timestamp - self._last_timestamp
            if dt > 0:
                speed = (downloaded - self._last_downloaded) / dt
                if speed > 0:
                    self._speeds.append(speed)

        self._last_downloaded = downloaded
        self._last_timestamp = timestamp

        if not self._speeds:
            return None

        avg_speed = sum(self._speeds) / len(self._speeds)
        remaining = total - downloaded
        if avg_speed <= 0:
            return None

        return remaining / avg_speed

    def reset(self) -> None:
        """Reset the ETA calculator."""
        self._speeds.clear()
        self._last_downloaded = 0
        self._last_timestamp = 0


def format_count(count: int, singular: str, plural: str | None = None) -> str:
    """Format a count with proper singular/plural."""
    if plural is None:
        plural = f"{singular}s"
    return f"{count} {singular if count == 1 else plural}"

"""Format resolution and stream parsing helper tools."""

from __future__ import annotations

import re
from typing import Any


def parse_resolution(
    resolution_str: str | None = None,
    height: int | None = None,
    width: int | None = None,
) -> tuple[int, int]:
    """Parse a resolution string or height/width values into (width, height) tuple.

    Handles formats like: "1920x1080", "1080p", "720p60", 1080, etc.
    Returns (0, 0) if unparseable.
    """
    if width and height:
        return (int(width), int(height))

    if height and not width:
        h = int(height)
        w = int(h * 16 / 9)
        return (w, h)

    if not resolution_str or not isinstance(resolution_str, str):
        return (0, 0)

    res_str = resolution_str.strip().lower()

    # Pattern: 1920x1080
    match = re.search(r"(\d{3,5})\s*x\s*(\d{3,5})", res_str)
    if match:
        return (int(match.group(1)), int(match.group(2)))

    # Pattern: 1080p, 720p60
    match = re.search(r"(\d{3,4})p", res_str)
    if match:
        h = int(match.group(1))
        w = int(h * 16 / 9)
        return (w, h)

    return (0, 0)


def format_resolution_label(
    height: int | None = None,
    width: int | None = None,
    fps: int | float | None = None,
) -> str:
    """Generate a clean display label for resolution and FPS.

    Examples:
        2160 -> "4K (2160p)"
        1080, fps=60 -> "1080p60"
        720 -> "720p"
        0 -> "Audio Only"
    """
    if not height or height <= 0:
        return "Audio Only"

    h = int(height)
    fps_suffix = f"{int(fps)}" if (fps and fps >= 45) else ""

    if h >= 2160:
        label = "4K (2160p)"
    elif h >= 1440:
        label = "2K (1440p)"
    elif h >= 1080:
        label = f"1080p{fps_suffix}"
    elif h >= 720:
        label = f"720p{fps_suffix}"
    elif h >= 480:
        label = f"480p{fps_suffix}"
    elif h >= 360:
        label = f"360p{fps_suffix}"
    else:
        label = f"{h}p"

    return label


def get_available_resolutions(formats: list[dict[str, Any]]) -> list[str]:
    """Extract unique available video resolutions from a list of yt-dlp format dicts.

    Returns sorted descending resolution labels (e.g. ["2160p", "1080p", "720p"]).
    """
    heights: set[int] = set()
    for fmt in formats:
        if not isinstance(fmt, dict):
            continue
        vcodec = fmt.get("vcodec", "")
        if vcodec == "none":
            continue
        height = fmt.get("height")
        if height and isinstance(height, int) and height > 0:
            heights.add(height)

    sorted_heights = sorted(heights, reverse=True)
    return [f"{h}p" for h in sorted_heights]


def is_video_only(fmt: dict[str, Any]) -> bool:
    """Check if a format contains video but no audio stream."""
    vcodec = fmt.get("vcodec", "none")
    acodec = fmt.get("acodec", "none")
    return vcodec != "none" and (acodec == "none" or not acodec)


def is_audio_only(fmt: dict[str, Any]) -> bool:
    """Check if a format contains audio but no video stream."""
    vcodec = fmt.get("vcodec", "none")
    acodec = fmt.get("acodec", "none")
    return (vcodec == "none" or not vcodec) and acodec != "none"


def filter_formats(
    formats: list[dict[str, Any]],
    mode: str = "video",
    max_height: int | None = None,
    container: str | None = None,
    prefer_hfr: bool = True,
) -> list[dict[str, Any]]:
    """Filter and rank formats by quality and parameters."""
    filtered: list[dict[str, Any]] = []
    for fmt in formats:
        if not isinstance(fmt, dict):
            continue

        vcodec = fmt.get("vcodec", "none")
        acodec = fmt.get("acodec", "none")

        if mode == "audio":
            if acodec == "none" or not acodec:
                continue
        else:
            if vcodec == "none" or not vcodec:
                continue
            height = fmt.get("height", 0) or 0
            if max_height and height > max_height:
                continue

        filtered.append(fmt)

    def _sort_key(f: dict[str, Any]) -> tuple:
        height = f.get("height", 0) or 0
        fps = (f.get("fps", 0) or 0) if prefer_hfr else 0
        tbr = f.get("tbr", 0) or f.get("vbr", 0) or f.get("abr", 0) or 0
        return (height, fps, tbr)

    return sorted(filtered, key=_sort_key, reverse=True)


def select_best_format(
    formats: list[dict[str, Any]] | None = None,
    target_quality: str = "best",
    mode: str = "video",
    prefer_hfr: bool = True,
) -> str:
    """Construct or resolve optimal yt-dlp format selector string given format list or quality target."""
    if not formats:
        if mode == "audio":
            return "bestaudio/best"
        if target_quality == "best":
            return "bestvideo+bestaudio/best"
        try:
            h = int(target_quality)
            return f"bestvideo[height<={h}]+bestaudio/best[height<={h}]"
        except ValueError:
            return "bestvideo+bestaudio/best"

    max_height = None
    if target_quality != "best":
        try:
            max_height = int(target_quality)
        except ValueError:
            pass

    eligible = filter_formats(
        formats,
        mode=mode,
        max_height=max_height,
        prefer_hfr=prefer_hfr,
    )

    if eligible:
        best_id = eligible[0].get("format_id")
        if best_id:
            if mode == "audio":
                return f"{best_id}/bestaudio/best"
            if is_video_only(eligible[0]):
                return f"{best_id}+bestaudio/best"
            return str(best_id)

    if mode == "audio":
        return "bestaudio/best"
    if max_height:
        return f"bestvideo[height<={max_height}]+bestaudio/best[height<={max_height}]"
    return "bestvideo+bestaudio/best"

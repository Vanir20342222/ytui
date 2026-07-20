"""Filesystem utilities — sanitization, path safety, and OS helpers."""

from __future__ import annotations

import os
import platform
import re
import shutil
import subprocess
import sys
from pathlib import Path


# Characters illegal in filenames on various OSes
_ILLEGAL_CHARS_WINDOWS = re.compile(r'[<>:"/\\|?*\x00-\x1f]')
_ILLEGAL_CHARS_UNIX = re.compile(r'[/\x00]')
_RESERVED_NAMES_WINDOWS = {
    "CON", "PRN", "AUX", "NUL",
    *(f"COM{i}" for i in range(1, 10)),
    *(f"LPT{i}" for i in range(1, 10)),
}


def sanitize_filename(name: str, replacement: str = "_") -> str:
    """Sanitize a filename for the current OS.

    Removes illegal characters, prevents path traversal,
    and handles reserved names on Windows.
    """
    if not name:
        return "unnamed"

    # Remove path separators and traversal attempts
    name = name.replace("..", "")
    name = os.path.basename(name)

    # OS-specific character sanitization
    if platform.system() == "Windows":
        name = _ILLEGAL_CHARS_WINDOWS.sub(replacement, name)
        # Handle reserved names
        stem = Path(name).stem.upper()
        if stem in _RESERVED_NAMES_WINDOWS:
            name = f"_{name}"
    else:
        name = _ILLEGAL_CHARS_UNIX.sub(replacement, name)

    # Trim leading/trailing dots and spaces (Windows issue)
    name = name.strip(". ")

    # Ensure we have something left
    if not name:
        name = "unnamed"

    # Truncate overly long names (max 200 chars, leaving room for extension)
    if len(name) > 200:
        stem = Path(name).stem[:190]
        suffix = Path(name).suffix
        name = f"{stem}{suffix}"

    return name


def safe_path(directory: str | Path, filename: str) -> Path:
    """Create a safe, non-traversal file path.

    Ensures the resulting path is within the target directory.
    """
    directory = Path(directory).resolve()
    safe_name = sanitize_filename(filename)
    full_path = (directory / safe_name).resolve()

    # Verify the path is within the directory (prevent traversal)
    if not str(full_path).startswith(str(directory)):
        raise ValueError(f"Path traversal detected: {filename}")

    return full_path


def ensure_directory(path: str | Path) -> Path:
    """Create a directory if it doesn't exist, return the Path."""
    p = Path(path)
    p.mkdir(parents=True, exist_ok=True)
    return p


def open_in_file_manager(path: str | Path) -> None:
    """Open a file or directory in the OS file manager."""
    path = str(Path(path).resolve())
    system = platform.system()
    try:
        if system == "Darwin":
            subprocess.Popen(["open", path])  # noqa: S603
        elif system == "Windows":
            os.startfile(path)  # type: ignore[attr-defined]  # noqa: S606
        else:  # Linux / BSD
            subprocess.Popen(["xdg-open", path])  # noqa: S603
    except Exception:
        pass  # Silently fail — not critical


def get_available_space(path: str | Path) -> int:
    """Return available disk space in bytes at the given path."""
    try:
        usage = shutil.disk_usage(str(path))
        return usage.free
    except Exception:
        return -1


def find_executable(name: str) -> str | None:
    """Find an executable on the system PATH."""
    return shutil.which(name)


def launch_media_player(path: str | Path) -> tuple[bool, str]:
    """Launch a media file in mpv, vlc, or system default handler (xdg-open/open/os.startfile).

    Returns (success: bool, info_or_error_message: str).
    """
    file_path = Path(path).resolve()
    if not file_path.exists():
        return False, f"File not found: {file_path}"

    path_str = str(file_path)

    # Try dedicated media players first
    for player in ("mpv", "vlc"):
        executable = find_executable(player)
        if executable:
            try:
                subprocess.Popen([executable, path_str])  # noqa: S603
                return True, player
            except Exception as e:
                pass

    # Fallback to OS default handler
    system = platform.system()
    try:
        if system == "Darwin":
            subprocess.Popen(["open", path_str])  # noqa: S603
            return True, "open"
        elif system == "Windows":
            os.startfile(path_str)  # type: ignore[attr-defined] # noqa: S606
            return True, "default player"
        else:  # Linux / BSD
            xdg = find_executable("xdg-open")
            if xdg:
                subprocess.Popen([xdg, path_str])  # noqa: S603
                return True, "xdg-open"
            else:
                return False, "No media player (mpv, vlc, xdg-open) found on system"
    except Exception as e:
        return False, f"Failed to launch player: {e}"


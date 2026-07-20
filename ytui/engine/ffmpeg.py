"""FFmpeg detection and media conversion."""

from __future__ import annotations

import json
import logging
import subprocess
import shutil
from pathlib import Path
from typing import Any

logger = logging.getLogger(__name__)


class FFmpegError(Exception):
    """Raised when an ffmpeg operation fails."""


def find_ffmpeg() -> str | None:
    """Find the ffmpeg executable on the system."""
    return shutil.which("ffmpeg")


def find_ffprobe() -> str | None:
    """Find the ffprobe executable on the system."""
    return shutil.which("ffprobe")


def get_ffmpeg_version() -> str | None:
    """Get the ffmpeg version string."""
    ffmpeg = find_ffmpeg()
    if not ffmpeg:
        return None
    try:
        result = subprocess.run(
            [ffmpeg, "-version"],
            capture_output=True,
            text=True,
            timeout=10,
        )
        first_line = result.stdout.split("\n")[0]
        # Extract version like "ffmpeg version 6.1.1"
        parts = first_line.split()
        if len(parts) >= 3:
            return parts[2]
        return first_line
    except Exception:
        return None


def probe_file(filepath: str) -> dict[str, Any] | None:
    """Get media file info using ffprobe."""
    ffprobe = find_ffprobe()
    if not ffprobe:
        return None
    try:
        result = subprocess.run(
            [
                ffprobe,
                "-v", "quiet",
                "-print_format", "json",
                "-show_format",
                "-show_streams",
                filepath,
            ],
            capture_output=True,
            text=True,
            timeout=30,
        )
        if result.returncode == 0:
            return json.loads(result.stdout)
    except Exception as e:
        logger.error(f"ffprobe failed for {filepath}: {e}")
    return None


def convert_file(
    input_path: str,
    output_path: str,
    codec: str | None = None,
    bitrate: str | None = None,
    extra_args: list[str] | None = None,
    progress_callback: Any = None,
) -> bool:
    """Convert a media file using ffmpeg.
    
    Args:
        input_path: Source file path.
        output_path: Destination file path.
        codec: Output codec (e.g., 'libmp3lame', 'aac', 'libx264').
        bitrate: Target bitrate (e.g., '320k', '192k').
        extra_args: Additional ffmpeg arguments.
        progress_callback: Optional callback for progress updates.
    
    Returns:
        True on success.
    """
    ffmpeg = find_ffmpeg()
    if not ffmpeg:
        raise FFmpegError("ffmpeg not found")

    cmd: list[str] = [ffmpeg, "-i", input_path, "-y"]
    
    if codec:
        cmd.extend(["-c:a" if _is_audio_codec(codec) else "-c:v", codec])
    
    if bitrate:
        cmd.extend(["-b:a", bitrate])
    
    if extra_args:
        cmd.extend(extra_args)
    
    # Ensure output directory exists
    Path(output_path).parent.mkdir(parents=True, exist_ok=True)
    cmd.append(output_path)

    logger.info(f"Converting: {' '.join(cmd)}")

    try:
        result = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            timeout=3600,  # 1 hour max
        )
        if result.returncode != 0:
            logger.error(f"ffmpeg error: {result.stderr}")
            raise FFmpegError(f"Conversion failed: {result.stderr[:200]}")
        return True
    except subprocess.TimeoutExpired:
        raise FFmpegError("Conversion timed out")
    except FFmpegError:
        raise
    except Exception as e:
        raise FFmpegError(f"Conversion error: {e}")


def extract_audio(
    input_path: str,
    output_path: str,
    codec: str = "libmp3lame",
    bitrate: str = "320k",
) -> bool:
    """Extract audio from a video file."""
    return convert_file(
        input_path,
        output_path,
        codec=codec,
        bitrate=bitrate,
        extra_args=["-vn"],  # No video
    )


def _is_audio_codec(codec: str) -> bool:
    """Check if a codec name is an audio codec."""
    audio_codecs = {
        "libmp3lame", "aac", "libvorbis", "libopus",
        "flac", "pcm_s16le", "libfdk_aac", "ac3",
        "opus", "vorbis", "mp3", "wav", "copy", "m4a",
    }
    return codec.lower() in audio_codecs


def get_audio_bitrate(filepath: str) -> int | None:
    """Get the audio bitrate of a file in kbps."""
    info = probe_file(filepath)
    if not info:
        return None
    for stream in info.get("streams", []):
        if stream.get("codec_type") == "audio":
            br = stream.get("bit_rate")
            if br:
                try:
                    return int(br) // 1000
                except (ValueError, TypeError):
                    pass
    # Fallback to format-level bitrate
    fmt = info.get("format", {})
    br = fmt.get("bit_rate")
    if br:
        try:
            return int(br) // 1000
        except (ValueError, TypeError):
            pass
    return None

"""Metadata extraction and music tagging."""

from __future__ import annotations

import logging
from typing import Any

from ytui.queue.models import VideoInfo

logger = logging.getLogger(__name__)


def extract_video_info(info_dict: dict[str, Any]) -> VideoInfo:
    """Convert a yt-dlp info dict to our VideoInfo model."""
    # Determine best resolution from formats
    resolution = info_dict.get("resolution", "")
    if not resolution:
        height = info_dict.get("height", 0)
        width = info_dict.get("width", 0)
        if height:
            resolution = f"{width}x{height}" if width else f"{height}p"

    # Approximate file size
    filesize = (
        info_dict.get("filesize")
        or info_dict.get("filesize_approx")
        or 0
    )

    return VideoInfo(
        video_id=info_dict.get("id", ""),
        title=info_dict.get("title", "Unknown Title"),
        uploader=info_dict.get("uploader", ""),
        channel=info_dict.get("channel", info_dict.get("uploader", "")),
        duration=int(info_dict.get("duration") or 0),
        thumbnail_url=info_dict.get("thumbnail", ""),
        upload_date=info_dict.get("upload_date", ""),
        description=info_dict.get("description", ""),
        view_count=info_dict.get("view_count", 0),
        formats=info_dict.get("formats", []),
        is_playlist="entries" in info_dict,
        playlist_title=info_dict.get("playlist_title", ""),
        playlist_index=info_dict.get("playlist_index", 0) or 0,
        playlist_count=info_dict.get("playlist_count", 0) or 0,
        chapters=info_dict.get("chapters", []),
        subtitles=info_dict.get("subtitles", {}),
        resolution=resolution,
        filesize_approx=filesize,
        ext=info_dict.get("ext", ""),
    )


def tag_audio_file(
    filepath: str,
    info: VideoInfo,
    embed_thumbnail: bool = True,
) -> bool:
    """Tag an audio file with metadata from VideoInfo.
    
    Uses mutagen for MP3/M4A/FLAC tagging.
    Returns True on success.
    """
    try:
        from mutagen import File as MutagenFile
        from mutagen.easyid3 import EasyID3
        from mutagen.id3 import ID3, APIC, TIT2, TPE1, TALB, TDRC, TRCK
        from mutagen.mp4 import MP4, MP4Cover
        from mutagen.flac import FLAC, Picture
    except ImportError:
        logger.warning("mutagen not available for audio tagging")
        return False

    try:
        audio = MutagenFile(filepath)
        if audio is None:
            return False

        ext = filepath.rsplit(".", 1)[-1].lower()

        if ext == "mp3":
            try:
                tags = ID3(filepath)
            except Exception:
                tags = ID3()
            tags["TIT2"] = TIT2(encoding=3, text=[info.title])
            if info.uploader:
                tags["TPE1"] = TPE1(encoding=3, text=[info.uploader])
            if info.upload_date:
                tags["TDRC"] = TDRC(encoding=3, text=[info.upload_date[:4]])
            tags.save(filepath)

        elif ext in ("m4a", "mp4"):
            tags = MP4(filepath)
            tags["\xa9nam"] = [info.title]
            if info.uploader:
                tags["\xa9ART"] = [info.uploader]
            if info.upload_date:
                tags["\xa9day"] = [info.upload_date[:4]]
            tags.save()

        elif ext == "flac":
            tags = FLAC(filepath)
            tags["title"] = info.title
            if info.uploader:
                tags["artist"] = info.uploader
            if info.upload_date:
                tags["date"] = info.upload_date[:4]
            tags.save()

        logger.info(f"Tagged audio file: {filepath}")
        return True

    except Exception as e:
        logger.error(f"Failed to tag {filepath}: {e}")
        return False

"""Data models for the download queue."""

from __future__ import annotations

import time
import uuid
from dataclasses import dataclass, field
from enum import Enum
from typing import Any


class ItemState(str, Enum):
    """Download item states."""
    PENDING = "pending"        # Fetching metadata (shimmer)
    QUEUED = "queued"          # Ready to download
    DOWNLOADING = "downloading"
    CONVERTING = "converting"
    MERGING = "merging"
    DONE = "done"
    PAUSED = "paused"
    ERROR = "error"
    CANCELLED = "cancelled"


class StreamPhase(str, Enum):
    """Current stream being downloaded."""
    VIDEO = "video"
    AUDIO = "audio"
    MERGING = "merging"
    CONVERTING = "converting"
    COMPLETE = "complete"


@dataclass
class ProgressInfo:
    """Progress information for a download."""
    percent: float = 0.0
    downloaded_bytes: int = 0
    total_bytes: int = 0
    speed: float = 0.0  # bytes per second
    eta: float | None = None  # seconds remaining
    phase: StreamPhase = StreamPhase.VIDEO

    @property
    def is_complete(self) -> bool:
        return self.percent >= 100.0


@dataclass
class VideoInfo:
    """Metadata about a video."""
    video_id: str = ""
    title: str = ""
    uploader: str = ""
    channel: str = ""
    duration: int = 0  # seconds
    thumbnail_url: str = ""
    upload_date: str = ""
    description: str = ""
    view_count: int = 0
    formats: list[dict[str, Any]] = field(default_factory=list)
    is_playlist: bool = False
    playlist_title: str = ""
    playlist_index: int = 0
    playlist_count: int = 0
    chapters: list[dict[str, Any]] = field(default_factory=list)
    subtitles: dict[str, list[dict[str, str]]] = field(default_factory=dict)
    # Resolved format info
    resolution: str = ""
    filesize_approx: int = 0
    ext: str = ""


@dataclass
class QueueItem:
    """A single item in the download queue."""
    id: str = field(default_factory=lambda: str(uuid.uuid4())[:8])
    url: str = ""
    state: ItemState = ItemState.PENDING
    info: VideoInfo = field(default_factory=VideoInfo)
    progress: ProgressInfo = field(default_factory=ProgressInfo)
    output_path: str = ""
    error_message: str = ""
    priority: int = 0  # Higher = higher priority
    created_at: float = field(default_factory=time.time)
    started_at: float = 0.0
    completed_at: float = 0.0
    download_mode: str = "video"  # "video" or "audio"
    quality_override: str = ""  # Per-item quality override
    retry_count: int = 0
    max_retries: int = 3
    # Playlist grouping
    playlist_id: str = ""
    parent_group_id: str = ""  # If part of a playlist group

    @property
    def display_title(self) -> str:
        return self.info.title or self.url or "Fetching..."

    @property
    def is_active(self) -> bool:
        return self.state in (
            ItemState.DOWNLOADING,
            ItemState.CONVERTING,
            ItemState.MERGING,
        )

    @property
    def can_retry(self) -> bool:
        return self.state == ItemState.ERROR and self.retry_count < self.max_retries


@dataclass
class PlaylistGroup:
    """A collapsed playlist group in the queue."""
    id: str = field(default_factory=lambda: str(uuid.uuid4())[:8])
    playlist_id: str = ""
    title: str = ""
    url: str = ""
    total_items: int = 0
    completed_items: int = 0
    item_ids: list[str] = field(default_factory=list)
    expanded: bool = False
    selected_indices: list[int] | None = None  # None = all items

    @property
    def pending_items(self) -> int:
        return self.total_items - self.completed_items


@dataclass
class DownloadResult:
    """Result of a completed download."""
    item_id: str
    video_id: str
    title: str
    output_path: str
    file_size: int
    duration: int
    download_mode: str
    quality: str
    completed_at: float = field(default_factory=time.time)
    elapsed_seconds: float = 0.0
    average_speed: float = 0.0

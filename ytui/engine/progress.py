"""Progress tracking with smoothed ETA and phase awareness."""

from __future__ import annotations

import time
from collections import deque
from dataclasses import dataclass, field
from enum import Enum

from ytui.utils.formatting import SmoothedETA


class DownloadPhase(str, Enum):
    """Current phase of the download pipeline."""
    METADATA = "fetching metadata"
    VIDEO = "downloading video"
    AUDIO = "downloading audio"
    MERGING = "merging video+audio"
    CONVERTING = "converting"
    EMBEDDING = "embedding metadata"
    COMPLETE = "complete"
    ERROR = "error"


@dataclass
class ProgressState:
    """Complete progress state for a single download."""
    phase: DownloadPhase = DownloadPhase.METADATA
    percent: float = 0.0
    downloaded_bytes: int = 0
    total_bytes: int = 0
    speed: float = 0.0  # bytes per second
    eta_seconds: float | None = None
    filename: str = ""
    
    # Internal ETA smoother
    _eta_calc: SmoothedETA = field(default_factory=SmoothedETA, repr=False)

    def update_from_hook(self, data: dict) -> None:
        """Update from a yt-dlp progress_hook callback dict."""
        status = data.get("status", "")
        
        if status == "downloading":
            self.downloaded_bytes = data.get("downloaded_bytes", 0)
            self.total_bytes = (
                data.get("total_bytes")
                or data.get("total_bytes_estimate")
                or 0
            )
            self.speed = data.get("speed") or 0.0
            self.filename = data.get("filename", self.filename)
            
            # Calculate percent
            if self.total_bytes > 0:
                self.percent = min(
                    (self.downloaded_bytes / self.total_bytes) * 100, 100.0
                )
            
            # Smoothed ETA
            now = time.time()
            if self.total_bytes > 0:
                self.eta_seconds = self._eta_calc.update(
                    self.downloaded_bytes, self.total_bytes, now
                )
        
        elif status == "finished":
            self.percent = 100.0
            self.downloaded_bytes = self.total_bytes or self.downloaded_bytes
            self.speed = 0.0
            self.eta_seconds = 0.0
            self.filename = data.get("filename", self.filename)
        
        elif status == "error":
            self.phase = DownloadPhase.ERROR

    def update_postprocessor(self, data: dict) -> None:
        """Update from a yt-dlp postprocessor_hook callback."""
        pp_name = data.get("postprocessor", "")
        status = data.get("status", "")
        
        if status == "started":
            if "merge" in pp_name.lower():
                self.phase = DownloadPhase.MERGING
            elif "audio" in pp_name.lower() or "extract" in pp_name.lower():
                self.phase = DownloadPhase.CONVERTING
            elif "metadata" in pp_name.lower() or "thumbnail" in pp_name.lower():
                self.phase = DownloadPhase.EMBEDDING
            else:
                self.phase = DownloadPhase.CONVERTING
            # Reset progress for post-processing phase
            self.percent = 0.0
            self.speed = 0.0
            self.eta_seconds = None
        
        elif status == "finished":
            self.percent = 100.0

    def reset(self) -> None:
        """Reset progress for a new download."""
        self.phase = DownloadPhase.METADATA
        self.percent = 0.0
        self.downloaded_bytes = 0
        self.total_bytes = 0
        self.speed = 0.0
        self.eta_seconds = None
        self._eta_calc.reset()


@dataclass
class AggregateProgress:
    """Aggregate progress across multiple downloads."""
    active_count: int = 0
    paused_count: int = 0
    completed_count: int = 0
    total_count: int = 0
    error_count: int = 0
    total_speed: float = 0.0  # Combined bytes/sec
    total_downloaded: int = 0
    total_size: int = 0
    
    @property
    def overall_percent(self) -> float:
        if self.total_size <= 0:
            return 0.0
        return min((self.total_downloaded / self.total_size) * 100, 100.0)
    
    @property
    def combined_eta(self) -> float | None:
        remaining = self.total_size - self.total_downloaded
        if remaining <= 0 or self.total_speed <= 0:
            return None
        return remaining / self.total_speed

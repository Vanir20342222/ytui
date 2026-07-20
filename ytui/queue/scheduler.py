"""Download scheduling and watch mode."""

from __future__ import annotations

import asyncio
import logging
import time
from dataclasses import dataclass, field
from typing import Any, Callable

logger = logging.getLogger(__name__)


@dataclass
class ScheduledDownload:
    """A download scheduled for later."""
    id: str
    url: str
    start_time: float  # Unix timestamp
    download_mode: str = "video"
    recurring: bool = False
    interval_seconds: int = 3600  # For watch mode
    last_check: float = 0.0
    enabled: bool = True


@dataclass
class OffPeakWindow:
    """Time window for off-peak downloading."""
    start_hour: int = 22  # 10 PM
    end_hour: int = 6     # 6 AM
    enabled: bool = False


class DownloadScheduler:
    """Manages scheduled downloads and watch mode."""

    def __init__(
        self,
        add_url_callback: Callable,
        off_peak: OffPeakWindow | None = None,
    ):
        self._add_url = add_url_callback
        self.scheduled: list[ScheduledDownload] = []
        self.off_peak = off_peak or OffPeakWindow()
        self._running = False
        self._task: asyncio.Task | None = None

    async def start(self) -> None:
        self._running = True
        self._task = asyncio.create_task(self._check_loop())

    async def stop(self) -> None:
        self._running = False
        if self._task:
            self._task.cancel()

    def add_scheduled(
        self,
        url: str,
        start_time: float,
        download_mode: str = "video",
    ) -> ScheduledDownload:
        s = ScheduledDownload(
            id=f"sched_{int(time.time())}_{len(self.scheduled)}",
            url=url,
            start_time=start_time,
            download_mode=download_mode,
        )
        self.scheduled.append(s)
        return s

    def add_watch(
        self,
        url: str,
        interval_seconds: int = 3600,
        download_mode: str = "video",
    ) -> ScheduledDownload:
        s = ScheduledDownload(
            id=f"watch_{int(time.time())}_{len(self.scheduled)}",
            url=url,
            start_time=time.time(),
            download_mode=download_mode,
            recurring=True,
            interval_seconds=interval_seconds,
        )
        self.scheduled.append(s)
        return s

    def is_off_peak(self) -> bool:
        if not self.off_peak.enabled:
            return True  # No restriction
        import datetime
        now = datetime.datetime.now()
        hour = now.hour
        start = self.off_peak.start_hour
        end = self.off_peak.end_hour
        if start < end:
            return start <= hour < end
        else:  # Wraps midnight
            return hour >= start or hour < end

    async def _check_loop(self) -> None:
        while self._running:
            now = time.time()
            for s in self.scheduled:
                if not s.enabled:
                    continue
                if s.recurring:
                    if now - s.last_check >= s.interval_seconds:
                        s.last_check = now
                        try:
                            await self._add_url(s.url, s.download_mode)
                        except Exception as e:
                            logger.error(f"Watch mode error for {s.url}: {e}")
                elif now >= s.start_time and s.start_time > 0:
                    s.start_time = 0  # Mark as fired
                    s.enabled = False
                    try:
                        await self._add_url(s.url, s.download_mode)
                    except Exception as e:
                        logger.error(f"Scheduled download error for {s.url}: {e}")

            await asyncio.sleep(30)  # Check every 30 seconds

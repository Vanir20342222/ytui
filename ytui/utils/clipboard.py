"""Cross-platform clipboard monitoring."""

from __future__ import annotations

import asyncio
import logging
from typing import Callable

logger = logging.getLogger(__name__)


def _get_clipboard_text() -> str:
    """Get current clipboard text content."""
    try:
        import pyperclip
        return pyperclip.paste() or ""
    except Exception:
        return ""


class ClipboardWatcher:
    """Watches the clipboard for new YouTube URLs.

    Polls the clipboard at a configurable interval and calls
    the callback when a new YouTube URL is detected.
    """

    def __init__(
        self,
        callback: Callable[[str], None],
        interval: float = 1.0,
        max_consecutive_errors: int = 10,
    ):
        self._callback = callback
        self._interval = interval
        self._max_consecutive_errors = max_consecutive_errors
        self._consecutive_errors = 0
        self._last_content = ""
        self._running = False
        self._task: asyncio.Task | None = None

    @property
    def is_running(self) -> bool:
        return self._running

    def start(self) -> None:
        """Start watching the clipboard."""
        if self._running:
            return
        self._running = True
        try:
            self._last_content = _get_clipboard_text()
        except Exception:
            self._last_content = ""
        self._task = asyncio.create_task(self._watch_loop())
        logger.info("Clipboard watcher started")

    def stop(self) -> None:
        """Stop watching the clipboard."""
        self._running = False
        if self._task:
            self._task.cancel()
            self._task = None
        logger.info("Clipboard watcher stopped")

    async def _watch_loop(self) -> None:
        """Main polling loop with offloading and exponential backoff."""
        from ytui.utils.urls import is_valid_url, is_youtube_url, normalize_url
        loop = asyncio.get_event_loop()

        while self._running:
            try:
                current_interval = min(
                    self._interval * (2 ** min(self._consecutive_errors, 5)),
                    60.0,
                ) if self._consecutive_errors > 0 else self._interval

                await asyncio.sleep(current_interval)
                if not self._running:
                    break

                content = await loop.run_in_executor(None, _get_clipboard_text)

                if content and content != self._last_content:
                    normalized = normalize_url(content.strip())
                    if is_valid_url(normalized) and is_youtube_url(normalized):
                        logger.info(f"Clipboard URL detected: {normalized}")
                        self._callback(normalized)
                    self._last_content = content
                self._consecutive_errors = 0

            except asyncio.CancelledError:
                break
            except Exception as e:
                self._consecutive_errors += 1
                if self._consecutive_errors >= self._max_consecutive_errors:
                    logger.warning(
                        f"Clipboard watcher encountered {self._consecutive_errors} consecutive errors: {e}"
                    )
                else:
                    logger.debug(f"Clipboard poll error ({self._consecutive_errors}): {e}")

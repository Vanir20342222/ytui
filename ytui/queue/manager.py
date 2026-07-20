"""Download queue manager with concurrent execution."""

from __future__ import annotations

import asyncio
import json
import logging
import time
from pathlib import Path
from typing import Any, Callable


from ytui.config.settings import Settings
from ytui.engine.downloader import DownloadEngine
from ytui.engine.metadata import extract_video_info
from ytui.engine.progress import AggregateProgress, DownloadPhase, ProgressState
from ytui.queue.models import (
    DownloadResult,
    ItemState,
    PlaylistGroup,
    QueueItem,
    VideoInfo,
)
from ytui.queue.persistence import QueueDatabase
from ytui.utils.urls import (
    extract_playlist_id,
    extract_video_id,
    is_playlist_url,
    normalize_url,
    strip_playlist_param,
)

logger = logging.getLogger(__name__)


class ResizableSemaphore:
    """An asyncio semaphore that can be resized dynamically."""
    def __init__(self, value: int = 1):
        self._max_value = value
        self._current_value = value
        self._waiters: list[asyncio.Future] = []

    def resize(self, new_value: int) -> None:
        diff = new_value - self._max_value
        self._max_value = new_value
        self._current_value += diff
        while self._current_value > 0 and self._waiters:
            waiter = self._waiters.pop(0)
            if not waiter.done():
                waiter.set_result(None)
                self._current_value -= 1

    async def acquire(self) -> bool:
        if self._current_value > 0 and not self._waiters:
            self._current_value -= 1
            return True
        loop = asyncio.get_running_loop()
        waiter = loop.create_future()
        self._waiters.append(waiter)
        try:
            await waiter
            return True
        except asyncio.CancelledError:
            if waiter in self._waiters:
                self._waiters.remove(waiter)
            else:
                self.release()
            raise

    def release(self) -> None:
        self._current_value += 1
        while self._current_value > 0 and self._waiters:
            waiter = self._waiters.pop(0)
            if not waiter.done():
                waiter.set_result(None)
                self._current_value -= 1
                break

    async def __aenter__(self) -> None:
        await self.acquire()

    async def __aexit__(self, exc_type: Any, exc_val: Any, exc_tb: Any) -> None:
        self.release()


class QueueManager:
    """Manages the download queue with concurrent execution.
    
    Handles:
    - Adding items (URLs, playlists, search results)
    - Metadata resolution
    - Concurrent download execution
    - Pause/resume/cancel
    - Priority reordering
    - Duplicate detection
    - Persistent state
    """

    def __init__(self, settings: Settings, db_path: str | Path | None = None):
        self.settings = settings
        self.engine = DownloadEngine(settings)
        self.db = QueueDatabase(db_path)
        self.items: list[QueueItem] = []
        self.playlist_groups: dict[str, PlaylistGroup] = {}
        self._workers: dict[str, asyncio.Task] = {}
        self._queue_task: asyncio.Task | None = None
        self._semaphore: ResizableSemaphore | None = None
        self._running = False
        self._loop: asyncio.AbstractEventLoop | None = None
        
        # Callbacks for UI updates
        self.on_item_added: Callable[[QueueItem], None] | None = None
        self.on_item_updated: Callable[[QueueItem], None] | None = None
        self.on_item_completed: Callable[[QueueItem], None] | None = None
        self.on_item_error: Callable[[QueueItem], None] | None = None
        self.on_item_removed: Callable[[QueueItem], None] | None = None
        self.on_progress: Callable[[str, ProgressState], None] | None = None
        self.on_aggregate_update: Callable[[AggregateProgress], None] | None = None

    def _dispatch_callback(self, callback: Callable[..., Any] | None, *args: Any) -> None:
        """Safely invoke callbacks on the main asyncio event loop.

        Never calls a UI callback directly from a worker thread: if the loop is
        not running (e.g. during shutdown), the update is dropped instead.
        """
        if callback is None:
            return
        if self._loop is not None and self._loop.is_running():
            try:
                current_loop = asyncio.get_running_loop()
            except RuntimeError:
                current_loop = None

            if current_loop is self._loop:
                callback(*args)
            else:
                self._loop.call_soon_threadsafe(callback, *args)
        # else: loop not running — drop the callback rather than touch the UI
        # from a non-main thread.

    async def start(self) -> None:
        """Start the queue manager and load persistent queue."""
        self._running = True
        try:
            self._loop = asyncio.get_running_loop()
        except RuntimeError:
            self._loop = asyncio.get_event_loop()
        max_concurrent = max(self.settings.network.max_concurrent_downloads, 1)
        self._semaphore = ResizableSemaphore(max_concurrent)
        
        # Load persistent queue
        saved_items = self.db.load_queue_items()
        # Load persistent playlist groups
        saved_groups = self.db.load_playlist_groups()
        for group in saved_groups:
            self.playlist_groups[group.id] = group

        for item in saved_items:
            # If item was interrupted mid-download, reset to QUEUED so process loop picks it up
            if item.state in (ItemState.DOWNLOADING, ItemState.MERGING, ItemState.CONVERTING):
                item.state = ItemState.QUEUED
                self.db.save_queue_item(item)

            self.items.append(item)
            self._dispatch_callback(self.on_item_added, item)

            # If item was left in PENDING metadata state, resume extraction
            if item.state == ItemState.PENDING:
                asyncio.create_task(self._resolve_metadata(item))

        logger.info(f"Queue manager started, {len(saved_items)} items loaded")
        
        # Start processing queued items
        self._queue_task = asyncio.create_task(self._process_queue())

    async def stop(self) -> None:
        """Stop the queue manager gracefully."""
        self._running = False
        # Signal yt-dlp worker threads to abort on their next progress hook.
        for item_id in list(self._workers.keys()):
            self.engine.cancel(item_id)
        # Cancel and await worker tasks so their cleanup finishes before DB close.
        tasks = list(self._workers.values())
        for task in tasks:
            task.cancel()
        if tasks:
            await asyncio.gather(*tasks, return_exceptions=True)
        self._workers.clear()

        # Cancel the queue processor task.
        if self._queue_task is not None:
            self._queue_task.cancel()
            await asyncio.gather(self._queue_task, return_exceptions=True)
            self._queue_task = None
        
        # Save all items
        for item in self.items:
            self.db.save_queue_item(item)

        # Save all playlist groups
        for group in self.playlist_groups.values():
            self.db.save_playlist_group(group)
        
        self.db.close()
        logger.info("Queue manager stopped")

    async def add_url(self, url: str, mode: str | None = None) -> QueueItem | list[QueueItem]:
        """Add a URL to the queue.

        Returns the created QueueItem(s). For playlists, returns a list.
        """
        url = normalize_url(url)
        download_mode = mode or self.settings.quality.download_mode

        # Search query (ytsearch:...) or genuine playlist URL
        if url.startswith("ytsearch"):
            return await self._add_playlist(url, download_mode)

        # A genuine playlist URL (not an auto-generated radio mix — those are
        # already filtered out by is_playlist_url). Honor the playlist_mode
        # setting: when off, strip the list param and download only the video.
        if is_playlist_url(url) and self.settings.quality.playlist_mode:
            return await self._add_playlist(url, download_mode)
        elif is_playlist_url(url) and not self.settings.quality.playlist_mode:
            url = strip_playlist_param(url)

        # Single video
        item = QueueItem(
            url=url,
            state=ItemState.PENDING,
            download_mode=download_mode,
        )
        self.items.append(item)
        self.db.save_queue_item(item)

        if self.on_item_added:
            self.on_item_added(item)

        # Resolve metadata in background
        asyncio.create_task(self._resolve_metadata(item))
        return item

    async def _add_playlist(
        self, url: str, download_mode: str
    ) -> list[QueueItem]:
        """Add a playlist URL, creating a group."""
        playlist_id = extract_playlist_id(url) or url
        
        # Fast metadata extraction
        info = await asyncio.get_event_loop().run_in_executor(
            None, lambda: self.engine.extract_info(url, flat=True)
        )
        if not info or "entries" not in info:
            # Fallback: treat as single URL
            item = QueueItem(
                url=url, state=ItemState.PENDING, download_mode=download_mode
            )
            self.items.append(item)
            self.db.save_queue_item(item)
            if self.on_item_added:
                self.on_item_added(item)
            asyncio.create_task(self._resolve_metadata(item))
            return [item]

        entries = [e for e in info.get("entries", []) if e is not None]
        group = PlaylistGroup(
            playlist_id=playlist_id,
            title=info.get("title", "Playlist"),
            url=url,
            total_items=len(entries),
        )

        items: list[QueueItem] = []
        for i, entry in enumerate(entries):
            video_url = entry.get("url") or entry.get("webpage_url", "")
            if not video_url and entry.get("id"):
                video_url = f"https://www.youtube.com/watch?v={entry['id']}"
            
            item = QueueItem(
                url=video_url,
                state=ItemState.PENDING,
                download_mode=download_mode,
                playlist_id=playlist_id,
                parent_group_id=group.id,
                info=VideoInfo(
                    video_id=entry.get("id", ""),
                    title=entry.get("title", f"Track {i + 1}"),
                    duration=int(entry.get("duration", 0) or 0),
                    uploader=entry.get("uploader", ""),
                    playlist_title=group.title,
                    playlist_index=i + 1,
                    playlist_count=len(entries),
                ),
            )
            items.append(item)
            group.item_ids.append(item.id)
            self.items.append(item)
            self.db.save_queue_item(item)

        self.playlist_groups[group.id] = group
        self.db.save_playlist_group(group)

        # Notify UI
        for item in items:
            if self.on_item_added:
                self.on_item_added(item)

        # Resolve full metadata for each item
        for item in items:
            asyncio.create_task(self._resolve_metadata(item))

        return items

    async def _resolve_metadata(self, item: QueueItem) -> None:
        """Resolve full metadata for a queue item with timeout protection."""
        try:
            loop = asyncio.get_running_loop()
            info = await asyncio.wait_for(
                loop.run_in_executor(None, lambda: self.engine.extract_info(item.url)),
                timeout=30.0,
            )
            if info:
                # Check for duplicates
                video_id = info.get("id", "")
                if (
                    self.settings.advanced.skip_duplicates
                    and video_id
                    and self.db.is_duplicate(video_id)
                ):
                    if item.state in (ItemState.PENDING, ItemState.PAUSED):
                        item.state = ItemState.ERROR
                        item.error_message = "Already downloaded (duplicate)"
                        self.db.save_queue_item(item)
                        self._dispatch_callback(self.on_item_updated, item)
                    return

                item.info = extract_video_info(info)
                if item.state == ItemState.PENDING:
                    item.state = ItemState.QUEUED
            else:
                if item.state == ItemState.PENDING:
                    item.state = ItemState.ERROR
                    item.error_message = item.error_message or "Failed to fetch metadata"
        except asyncio.TimeoutError:
            if item.state == ItemState.PENDING:
                item.state = ItemState.ERROR
                item.error_message = "Metadata extraction timed out (30s)"
                logger.error(f"Metadata resolution timed out for {item.url}")
        except Exception as e:
            if item.state == ItemState.PENDING:
                item.state = ItemState.ERROR
                item.error_message = str(e)[:150]
                logger.error(f"Metadata resolution failed for {item.url}: {e}")

        self.db.save_queue_item(item)
        self._dispatch_callback(self.on_item_updated, item)

    async def _process_queue(self) -> None:
        """Continuously process queued items."""
        while self._running:
            await asyncio.sleep(0.5)
            
            # Find next queued item
            queued = [
                item for item in self.items
                if item.state == ItemState.QUEUED
            ]
            # Sort by priority (higher first), then by creation time
            queued.sort(key=lambda x: (-x.priority, x.created_at))

            for item in queued:
                if not self._running:
                    break
                if item.id not in self._workers:
                    task = asyncio.create_task(self._download_item(item))
                    self._workers[item.id] = task

    async def _download_item(self, item: QueueItem) -> None:
        """Download a single item with semaphore control."""
        if self._semaphore is None:
            return

        try:
            async with self._semaphore:
                if item.state != ItemState.QUEUED:
                    return

                item.state = ItemState.DOWNLOADING
                item.started_at = time.time()
                progress = ProgressState(phase=DownloadPhase.VIDEO)

                def on_progress(data: dict) -> None:
                    progress.update_from_hook(data)
                    item.progress.percent = progress.percent
                    item.progress.downloaded_bytes = progress.downloaded_bytes
                    item.progress.total_bytes = progress.total_bytes
                    item.progress.speed = progress.speed
                    item.progress.eta = progress.eta_seconds
                    
                    now = time.monotonic()
                    if not hasattr(item, '_last_progress_time'):
                        item._last_progress_time = 0
                    if now - item._last_progress_time >= 0.25 or progress.percent == 100.0:
                        item._last_progress_time = now
                        self._dispatch_callback(self.on_progress, item.id, progress)
                        self._dispatch_callback(self.on_item_updated, item)

                def on_postprocessor(data: dict) -> None:
                    progress.update_postprocessor(data)
                    if progress.phase == DownloadPhase.MERGING:
                        item.state = ItemState.MERGING
                    elif progress.phase in (DownloadPhase.CONVERTING, DownloadPhase.EMBEDDING):
                        item.state = ItemState.CONVERTING
                    # Throttle postprocessor updates to the same 4 FPS as progress
                    # to avoid flooding the event loop during audio conversion.
                    now = time.monotonic()
                    if not hasattr(item, '_last_progress_time'):
                        item._last_progress_time = 0.0
                    if now - item._last_progress_time >= 0.25 or progress.percent == 100.0:
                        item._last_progress_time = now
                        self._dispatch_callback(self.on_item_updated, item)

                self.db.save_queue_item(item)
                self._dispatch_callback(self.on_item_updated, item)

                future = asyncio.get_event_loop().run_in_executor(
                    None,
                    lambda: self.engine.download(item, on_progress, on_postprocessor),
                )
                try:
                    output = await asyncio.shield(future)
                except asyncio.CancelledError:
                    self.engine.cancel(item.id)
                    try:
                        await future
                    except Exception:
                        pass
                    raise

                if output:
                    item.state = ItemState.DONE
                    item.output_path = output
                    item.completed_at = time.time()
                    item.progress.percent = 100.0

                    # Record in history
                    result = DownloadResult(
                        item_id=item.id,
                        video_id=item.info.video_id,
                        title=item.info.title,
                        output_path=output,
                        file_size=item.progress.total_bytes,
                        duration=item.info.duration,
                        download_mode=item.download_mode,
                        quality=item.quality_override or self.settings.quality.video_quality,
                        elapsed_seconds=item.completed_at - item.started_at,
                        average_speed=(
                            item.progress.total_bytes / max(item.completed_at - item.started_at, 0.1)
                        ),
                    )
                    self.db.add_history(result, item.url)

                    # Execute post-download custom script if configured
                    self._run_post_download_script(item, output)

                    # Update playlist group
                    if item.parent_group_id and item.parent_group_id in self.playlist_groups:
                        self.playlist_groups[item.parent_group_id].completed_items += 1
                        self.db.save_playlist_group(self.playlist_groups[item.parent_group_id])

                    if self.on_item_completed:
                        self._dispatch_callback(self.on_item_completed, item)
                else:
                    # Download returned None (failed). can_retry checks state==ERROR,
                    # but state is still DOWNLOADING here, so check retry_count directly.
                    if item.retry_count < item.max_retries:
                        item.retry_count += 1
                        item.state = ItemState.QUEUED
                        item.progress.percent = 0.0
                        item.progress.downloaded_bytes = 0
                        item.progress.speed = 0.0
                        item.progress.eta = None
                        item.error_message = ""
                        logger.info(f"Retrying {item.display_title} ({item.retry_count}/{item.max_retries})")
                    else:
                        item.state = ItemState.ERROR
                        self._dispatch_callback(self.on_item_error, item)

        except Exception as exc:
            import yt_dlp
            if isinstance(exc, yt_dlp.utils.DownloadCancelled):
                # User-initiated cancel via engine.cancel() — treat as pause/cancel.
                if item.state != ItemState.CANCELLED:
                    item.state = ItemState.PAUSED
                item.error_message = ""
            else:
                item.state = ItemState.ERROR
                item.error_message = str(exc)[:150]
                self._dispatch_callback(self.on_item_error, item)
        except asyncio.CancelledError:
            if item.state != ItemState.CANCELLED:
                item.state = ItemState.PAUSED
            raise
        finally:
            if item.state != ItemState.CANCELLED and any(i.id == item.id for i in self.items):
                self.db.save_queue_item(item)
                self._dispatch_callback(self.on_item_updated, item)
            self._workers.pop(item.id, None)

    def pause_item(self, item_id: str) -> None:
        """Pause a download."""
        item = self._find_item(item_id)
        if item and item.state not in (ItemState.DONE, ItemState.ERROR, ItemState.CANCELLED, ItemState.PAUSED):
            task = self._workers.get(item_id)
            if task:
                task.cancel()
            self.engine.cancel(item_id)
            item.state = ItemState.PAUSED
            self.db.save_queue_item(item)
            self._dispatch_callback(self.on_item_updated, item)

    def resume_item(self, item_id: str) -> None:
        """Resume a paused download."""
        item = self._find_item(item_id)
        if item and item.state == ItemState.PAUSED:
            item.state = ItemState.QUEUED
            # Clear stale progress so the resumed run starts fresh.
            item.progress.percent = 0.0
            item.progress.downloaded_bytes = 0
            item.progress.speed = 0.0
            item.progress.eta = None
            self.db.save_queue_item(item)
            self._dispatch_callback(self.on_item_updated, item)

    def cancel_item(self, item_id: str) -> None:
        """Cancel and remove a download."""
        item = self._find_item(item_id)
        if item:
            task = self._workers.get(item_id)
            if task:
                task.cancel()
            self.engine.cancel(item_id)
            item.state = ItemState.CANCELLED
            self.items.remove(item)
            self.db.remove_queue_item(item_id)
            self._workers.pop(item_id, None)
            # Notify the UI to remove the row (not just update it).
            self._dispatch_callback(self.on_item_removed, item)

    def pause_all(self) -> None:
        """Pause all active and queued/pending downloads."""
        for item in self.items:
            if item.state not in (ItemState.DONE, ItemState.ERROR, ItemState.CANCELLED, ItemState.PAUSED):
                self.pause_item(item.id)

    def resume_all(self) -> None:
        """Resume all paused downloads."""
        for item in self.items:
            if item.state == ItemState.PAUSED:
                self.resume_item(item.id)

    def cancel_all(self) -> int:
        """Cancel every active and queued download. Returns the count cancelled."""
        cancelled = 0
        for item in list(self.items):
            if item.is_active or item.state == ItemState.QUEUED:
                self.cancel_item(item.id)
                cancelled += 1
        return cancelled

    def retry_item(self, item_id: str) -> None:
        """Retry a failed download."""
        item = self._find_item(item_id)
        if item and item.state == ItemState.ERROR:
            item.state = ItemState.QUEUED
            item.error_message = ""
            item.retry_count += 1
            # Clear stale cancellation so the retry isn't immediately re-cancelled.
            self.engine._cancelled_items.discard(item.id)
            self.db.save_queue_item(item)
            self._dispatch_callback(self.on_item_updated, item)

    def move_item(self, item_id: str, direction: int) -> None:
        """Move an item up or down in the queue."""
        item = self._find_item(item_id)
        if not item:
            return
        idx = self.items.index(item)
        new_idx = max(0, min(len(self.items) - 1, idx + direction))
        if new_idx != idx:
            self.items.pop(idx)
            self.items.insert(new_idx, item)
            # Re-assign priority levels based on new order
            total = len(self.items)
            for i, it in enumerate(self.items):
                it.priority = total - i
                self.db.save_queue_item(it)
            self._dispatch_callback(self.on_item_updated, item)

    def set_priority(self, item_id: str, priority: int) -> None:
        """Set priority for a queue item."""
        item = self._find_item(item_id)
        if item:
            item.priority = priority
            self.db.save_queue_item(item)

    def get_aggregate_progress(self) -> AggregateProgress:
        """Get combined progress across all items."""
        agg = AggregateProgress(total_count=len(self.items))
        for item in self.items:
            if item.is_active:
                agg.active_count += 1
                agg.total_speed += item.progress.speed
                agg.total_downloaded += item.progress.downloaded_bytes
                agg.total_size += item.progress.total_bytes
            elif item.state == ItemState.PAUSED:
                agg.paused_count += 1
            elif item.state == ItemState.DONE:
                agg.completed_count += 1
            elif item.state == ItemState.ERROR:
                agg.error_count += 1
        return agg

    def _find_item(self, item_id: str) -> QueueItem | None:
        for item in self.items:
            if item.id == item_id:
                return item
        return None

    def export_queue(self, path: str | Path | None = None) -> Path:
        """Export current queue items to a JSON file.

        If path is not specified, exports to default file path in settings data path.
        Returns the absolute Path of the exported file.
        """
        if not path:
            export_path = Settings.data_path() / "queue_export.json"
        else:
            export_path = Path(path).resolve()

        export_path.parent.mkdir(parents=True, exist_ok=True)

        data = []
        for item in self.items:
            data.append({
                "id": item.id,
                "url": item.url,
                "state": item.state.value,
                "download_mode": item.download_mode,
                "quality_override": item.quality_override,
                "priority": item.priority,
                "created_at": item.created_at,
                "output_path": item.output_path,
                "error_message": item.error_message,
                "info": {
                    "video_id": item.info.video_id,
                    "title": item.info.title,
                    "uploader": item.info.uploader,
                    "duration": item.info.duration,
                    "resolution": item.info.resolution,
                },
            })

        with open(export_path, "w", encoding="utf-8") as f:
            json.dump({"version": 1, "count": len(data), "items": data}, f, indent=2)

        return export_path

    async def import_queue(self, path: str | Path) -> int:
        """Import queue items from a JSON export file or text file containing URLs.

        Returns the number of imported items.
        """
        import_path = Path(path).resolve()
        if not import_path.exists():
            raise FileNotFoundError(f"File not found: {import_path}")

        count = 0
        with open(import_path, "r", encoding="utf-8") as f:
            content = f.read()

        # Try parsing as JSON first
        try:
            data = json.loads(content)
            items_data = []
            if isinstance(data, dict) and "items" in data:
                items_data = data["items"]
            elif isinstance(data, list):
                items_data = data

            for entry in items_data:
                if isinstance(entry, dict) and "url" in entry:
                    url = entry["url"]
                    mode = entry.get("download_mode")
                    if url:
                        res = await self.add_url(url, mode=mode)
                        count += len(res) if isinstance(res, list) else 1
                elif isinstance(entry, str) and entry.strip():
                    res = await self.add_url(entry.strip())
                    count += len(res) if isinstance(res, list) else 1
            return count
        except json.JSONDecodeError:
            pass

        # Fallback to line-by-line text format
        lines = content.splitlines()
        for line in lines:
            line = line.strip()
            if line and not line.startswith("#"):
                res = await self.add_url(line)
                count += len(res) if isinstance(res, list) else 1

        return count


    def _run_post_download_script(self, item: QueueItem, output_path: str) -> None:
        """Execute post-download custom script if enabled in settings."""
        if not self.settings.advanced.enable_custom_script or not self.settings.advanced.custom_script:
            return

        script_path = Path(self.settings.advanced.custom_script).expanduser()
        if not script_path.exists():
            logger.warning(f"Post-download script not found: {script_path}")
            return

        def _execute():
            import os
            import subprocess
            try:
                env = os.environ.copy()
                env.update({
                    "YTUI_OUTPUT_PATH": output_path,
                    "YTUI_ITEM_ID": item.id,
                    "YTUI_VIDEO_ID": getattr(item.info, "video_id", ""),
                    "YTUI_TITLE": getattr(item.info, "title", ""),
                    "YTUI_URL": item.url,
                    "YTUI_DOWNLOAD_MODE": item.download_mode,
                })
                cmd = [str(script_path), output_path]
                res = subprocess.run(cmd, env=env, capture_output=True, text=True, timeout=120)
                if res.returncode != 0:
                    logger.warning(f"Post-download script returned exit code {res.returncode}: {res.stderr}")
                else:
                    logger.info(f"Post-download script executed successfully for {output_path}")
            except Exception as exc:
                logger.error(f"Failed to execute post-download script {script_path}: {exc}")

        try:
            loop = asyncio.get_running_loop()
            loop.run_in_executor(None, _execute)
        except RuntimeError:
            _execute()


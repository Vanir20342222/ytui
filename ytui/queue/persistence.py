"""SQLite-backed persistent queue and download history."""

from __future__ import annotations

import json
import logging
import sqlite3
import time
from pathlib import Path
from typing import Any

from ytui.config.settings import Settings
from ytui.queue.models import (
    DownloadResult,
    ItemState,
    PlaylistGroup,
    QueueItem,
    VideoInfo,
)

import threading

logger = logging.getLogger(__name__)


class QueueDatabase:
    """SQLite persistence layer for the download queue and history."""

    def __init__(self, db_path: str | Path | None = None):
        if db_path is None:
            db_path = Settings.data_path() / "queue.db"
        self.db_path = Path(db_path)
        self.db_path.parent.mkdir(parents=True, exist_ok=True)
        self._local = threading.local()
        self._lock = threading.Lock()
        self._connections: set[sqlite3.Connection] = set()
        self._init_db()

    def _get_conn(self) -> sqlite3.Connection:
        if not hasattr(self._local, "conn") or self._local.conn is None:
            conn = sqlite3.connect(str(self.db_path), timeout=30.0)
            conn.row_factory = sqlite3.Row
            conn.execute("PRAGMA journal_mode=WAL")
            conn.execute("PRAGMA synchronous=NORMAL")
            self._local.conn = conn
            with self._lock:
                self._connections.add(conn)
        return self._local.conn

    def _init_db(self) -> None:
        conn = self._get_conn()
        conn.executescript("""
            CREATE TABLE IF NOT EXISTS queue_items (
                id TEXT PRIMARY KEY,
                url TEXT NOT NULL,
                state TEXT NOT NULL DEFAULT 'pending',
                info_json TEXT DEFAULT '{}',
                output_path TEXT DEFAULT '',
                error_message TEXT DEFAULT '',
                priority INTEGER DEFAULT 0,
                created_at REAL NOT NULL,
                started_at REAL DEFAULT 0,
                completed_at REAL DEFAULT 0,
                download_mode TEXT DEFAULT 'video',
                quality_override TEXT DEFAULT '',
                retry_count INTEGER DEFAULT 0,
                max_retries INTEGER DEFAULT 3,
                playlist_id TEXT DEFAULT '',
                parent_group_id TEXT DEFAULT ''
            );

            CREATE TABLE IF NOT EXISTS playlist_groups (
                id TEXT PRIMARY KEY,
                playlist_id TEXT NOT NULL,
                title TEXT DEFAULT '',
                url TEXT NOT NULL,
                total_items INTEGER DEFAULT 0,
                completed_items INTEGER DEFAULT 0,
                item_ids_json TEXT DEFAULT '[]',
                selected_indices_json TEXT DEFAULT NULL
            );

            CREATE TABLE IF NOT EXISTS download_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                item_id TEXT NOT NULL,
                video_id TEXT NOT NULL,
                title TEXT NOT NULL,
                output_path TEXT NOT NULL,
                file_size INTEGER DEFAULT 0,
                duration INTEGER DEFAULT 0,
                download_mode TEXT DEFAULT 'video',
                quality TEXT DEFAULT '',
                completed_at REAL NOT NULL,
                elapsed_seconds REAL DEFAULT 0,
                average_speed REAL DEFAULT 0,
                url TEXT DEFAULT ''
            );

            CREATE TABLE IF NOT EXISTS stats (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_queue_state ON queue_items(state);
            CREATE INDEX IF NOT EXISTS idx_history_video_id ON download_history(video_id);
            CREATE INDEX IF NOT EXISTS idx_history_completed ON download_history(completed_at);
        """)
        conn.commit()

    def save_queue_item(self, item: QueueItem) -> None:
        conn = self._get_conn()
        info_json = json.dumps({
            "video_id": item.info.video_id,
            "title": item.info.title,
            "uploader": item.info.uploader,
            "channel": item.info.channel,
            "duration": item.info.duration,
            "thumbnail_url": item.info.thumbnail_url,
            "upload_date": item.info.upload_date,
            "description": item.info.description,
            "view_count": item.info.view_count,
            "formats": item.info.formats,
            "resolution": item.info.resolution,
            "filesize_approx": item.info.filesize_approx,
            "ext": item.info.ext,
            "is_playlist": item.info.is_playlist,
            "playlist_title": item.info.playlist_title,
            "playlist_index": item.info.playlist_index,
            "playlist_count": item.info.playlist_count,
            "chapters": item.info.chapters,
            "subtitles": item.info.subtitles,
        })
        conn.execute("""
            INSERT OR REPLACE INTO queue_items
            (id, url, state, info_json, output_path, error_message,
             priority, created_at, started_at, completed_at,
             download_mode, quality_override, retry_count, max_retries,
             playlist_id, parent_group_id)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """, (
            item.id, item.url, item.state.value, info_json,
            item.output_path, item.error_message,
            item.priority, item.created_at, item.started_at, item.completed_at,
            item.download_mode, item.quality_override,
            item.retry_count, item.max_retries,
            item.playlist_id, item.parent_group_id,
        ))
        conn.commit()

    def load_queue_items(self) -> list[QueueItem]:
        conn = self._get_conn()
        cursor = conn.execute("""
            SELECT * FROM queue_items
            WHERE state NOT IN ('done', 'cancelled')
            ORDER BY priority DESC, created_at ASC
        """)
        items = []
        for row in cursor:
            try:
                info_data = json.loads(row["info_json"] or "{}")
                video_info_fields = {
                    "video_id": info_data.get("video_id", ""),
                    "title": info_data.get("title", ""),
                    "uploader": info_data.get("uploader", ""),
                    "channel": info_data.get("channel", ""),
                    "duration": info_data.get("duration", 0),
                    "thumbnail_url": info_data.get("thumbnail_url", ""),
                    "upload_date": info_data.get("upload_date", ""),
                    "description": info_data.get("description", ""),
                    "view_count": info_data.get("view_count", 0),
                    "formats": info_data.get("formats", []),
                    "is_playlist": info_data.get("is_playlist", False),
                    "playlist_title": info_data.get("playlist_title", ""),
                    "playlist_index": info_data.get("playlist_index", 0),
                    "playlist_count": info_data.get("playlist_count", 0),
                    "chapters": info_data.get("chapters", []),
                    "subtitles": info_data.get("subtitles", {}),
                    "resolution": info_data.get("resolution", ""),
                    "filesize_approx": info_data.get("filesize_approx", 0),
                    "ext": info_data.get("ext", ""),
                }
                item = QueueItem(
                    id=row["id"],
                    url=row["url"],
                    state=ItemState(row["state"]),
                    info=VideoInfo(**video_info_fields),
                    output_path=row["output_path"] or "",
                    error_message=row["error_message"] or "",
                    priority=row["priority"],
                    created_at=row["created_at"],
                    started_at=row["started_at"],
                    completed_at=row["completed_at"],
                    download_mode=row["download_mode"] or "video",
                    quality_override=row["quality_override"] or "",
                    retry_count=row["retry_count"],
                    max_retries=row["max_retries"],
                    playlist_id=row["playlist_id"] or "",
                    parent_group_id=row["parent_group_id"] or "",
                )
                # Reset active states to queued (app restart recovery)
                if item.state in (ItemState.DOWNLOADING, ItemState.MERGING, ItemState.CONVERTING):
                    item.state = ItemState.QUEUED
                items.append(item)
            except Exception as e:
                logger.error(f"Failed to load queue item {row['id']}: {e}")
                continue
        return items

    def remove_queue_item(self, item_id: str) -> None:
        conn = self._get_conn()
        conn.execute("DELETE FROM queue_items WHERE id = ?", (item_id,))
        conn.commit()

    def add_history(self, result: DownloadResult, url: str = "") -> None:
        conn = self._get_conn()
        conn.execute("""
            INSERT INTO download_history
            (item_id, video_id, title, output_path, file_size, duration,
             download_mode, quality, completed_at, elapsed_seconds, average_speed, url)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """, (
            result.item_id, result.video_id, result.title,
            result.output_path, result.file_size, result.duration,
            result.download_mode, result.quality,
            result.completed_at, result.elapsed_seconds,
            result.average_speed, url,
        ))
        conn.commit()

    def get_history(
        self,
        search: str = "",
        limit: int = 100,
        offset: int = 0,
    ) -> list[dict[str, Any]]:
        conn = self._get_conn()
        if search:
            cursor = conn.execute("""
                SELECT * FROM download_history
                WHERE title LIKE ? OR video_id LIKE ?
                ORDER BY completed_at DESC
                LIMIT ? OFFSET ?
            """, (f"%{search}%", f"%{search}%", limit, offset))
        else:
            cursor = conn.execute("""
                SELECT * FROM download_history
                ORDER BY completed_at DESC
                LIMIT ? OFFSET ?
            """, (limit, offset))
        return [dict(row) for row in cursor]

    def is_duplicate(self, video_id: str) -> bool:
        conn = self._get_conn()
        row = conn.execute(
            "SELECT COUNT(*) as cnt FROM download_history WHERE video_id = ?",
            (video_id,)
        ).fetchone()
        return (row["cnt"] or 0) > 0

    def get_stats(self) -> dict[str, Any]:
        conn = self._get_conn()
        row = conn.execute("""
            SELECT
                COUNT(*) as total_downloads,
                COALESCE(SUM(file_size), 0) as total_bytes,
                COALESCE(SUM(duration), 0) as total_duration,
                COALESCE(SUM(elapsed_seconds), 0) as total_time_spent
            FROM download_history
        """).fetchone()
        return dict(row) if row else {}

    def update_stat(self, key: str, value: str) -> None:
        conn = self._get_conn()
        conn.execute(
            "INSERT OR REPLACE INTO stats (key, value) VALUES (?, ?)",
            (key, value)
        )
        conn.commit()

    def get_stat(self, key: str) -> str | None:
        conn = self._get_conn()
        row = conn.execute("SELECT value FROM stats WHERE key = ?", (key,)).fetchone()
        return row["value"] if row else None

    def save_playlist_group(self, group: PlaylistGroup) -> None:
        conn = self._get_conn()
        conn.execute("""
            INSERT OR REPLACE INTO playlist_groups
            (id, playlist_id, title, url, total_items, completed_items,
             item_ids_json, selected_indices_json)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
        """, (
            group.id, group.playlist_id, group.title, group.url,
            group.total_items, group.completed_items,
            json.dumps(group.item_ids),
            json.dumps(group.selected_indices) if group.selected_indices is not None else None,
        ))
        conn.commit()

    def load_playlist_groups(self) -> list[PlaylistGroup]:
        conn = self._get_conn()
        cursor = conn.execute("SELECT * FROM playlist_groups")
        groups = []
        for row in cursor:
            try:
                selected_indices = row["selected_indices_json"]
                if selected_indices is not None:
                    selected_indices = json.loads(selected_indices)
                group = PlaylistGroup(
                    id=row["id"],
                    playlist_id=row["playlist_id"],
                    title=row["title"] or "",
                    url=row["url"],
                    total_items=row["total_items"] or 0,
                    completed_items=row["completed_items"] or 0,
                    item_ids=json.loads(row["item_ids_json"] or "[]"),
                    selected_indices=selected_indices,
                )
                groups.append(group)
            except Exception as e:
                logger.error(f"Failed to load playlist group {row['id']}: {e}")
                continue
        return groups

    def remove_playlist_group(self, group_id: str) -> None:
        conn = self._get_conn()
        conn.execute("DELETE FROM playlist_groups WHERE id = ?", (group_id,))
        conn.commit()

    def close(self) -> None:
        with self._lock:
            for conn in list(self._connections):
                try:
                    conn.close()
                except Exception:
                    pass
            self._connections.clear()
        if hasattr(self._local, "conn"):
            self._local.conn = None

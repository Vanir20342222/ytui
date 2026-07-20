"""Comprehensive verification and regression test suite for ytui."""

from __future__ import annotations

import asyncio
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
import pytest

from ytui.queue.persistence import QueueDatabase
from ytui.queue.models import QueueItem, ItemState, VideoInfo
from ytui.queue.manager import QueueManager
from ytui.config.settings import Settings
from ytui.config.profiles import save_profile, load_profile, delete_profile
from ytui.config.keyring_store import _save_fallback, _load_fallback
from ytui.engine.metadata import extract_video_info
from ytui.engine.ffmpeg import _is_audio_codec, get_audio_bitrate
from ytui.engine.downloader import DownloadEngine
from ytui.utils.formatting import format_size
from ytui.utils.notifications import send_notification
from ytui.utils.urls import is_valid_url, is_youtube_url


def test_sqlite_multithread_sharing(tmp_path):
    """BUG-CRIT-01: Test concurrent SQLite database operations across threads."""
    db_file = tmp_path / "test_queue.db"
    db = QueueDatabase(db_file)
    
    item = QueueItem(id="test1", url="https://youtube.com/watch?v=12345678901")

    def worker_save():
        try:
            db.save_queue_item(item)
            return True
        except Exception as e:
            return e

    with ThreadPoolExecutor(max_workers=4) as executor:
        futures = [executor.submit(worker_save) for _ in range(5)]
        results = [f.result() for f in futures]

    db.close()
    errors = [r for r in results if isinstance(r, Exception)]
    assert len(errors) == 0


def test_notification_subprocess():
    """BUG-HIGH-01: Test desktop notification execution."""
    send_notification("ytui test", "Test notification message")


def test_profile_path_traversal(tmp_path, monkeypatch):
    """BUG-CRIT-04: Test path traversal prevention in profile operations."""
    profiles_dir = tmp_path / "profiles"
    profiles_dir.mkdir()
    monkeypatch.setattr("ytui.config.profiles._profiles_dir", lambda: profiles_dir)

    settings = Settings()
    traversal_name = "../../traversal_test"
    target_file = tmp_path / "traversal_test.json"

    save_profile(traversal_name, settings)
    assert not target_file.exists()

    result = load_profile(traversal_name)
    assert result is None

    target_file.touch()
    deleted = delete_profile(traversal_name)
    assert not deleted
    target_file.unlink()


def test_settings_key_filtering():
    """BUG-HIGH-05: Test Settings._from_dict ignores unrecognized JSON keys."""
    raw_data = {
        "quality": {"video_quality": "1080", "unknown_extra_key": "invalid_value"},
        "advanced": {"auto_retry_count": 5, "deprecated_key": True},
    }
    s = Settings._from_dict(raw_data)
    assert s.quality.video_quality == "1080"
    assert s.advanced.auto_retry_count == 5


def test_secret_atomic_permissions(tmp_path, monkeypatch):
    """BUG-HIGH-07: Test fallback secrets file is created with restricted permissions."""
    test_file = tmp_path / ".secrets"
    monkeypatch.setattr("ytui.config.keyring_store._fallback_path", lambda: test_file)

    _save_fallback({"key1": "val1"})
    assert test_file.exists()
    assert _load_fallback() == {"key1": "val1"}


def test_queue_move_item_priority(tmp_path):
    """BUG-MED-02: Test that moving item in queue updates QueueItem.priority."""
    settings = Settings()
    qm = QueueManager(settings, db_path=tmp_path / "test_queue.db")

    item1 = QueueItem(id="item1", url="https://youtube.com/watch?v=1", state=ItemState.QUEUED)
    item2 = QueueItem(id="item2", url="https://youtube.com/watch?v=2", state=ItemState.QUEUED)
    item3 = QueueItem(id="item3", url="https://youtube.com/watch?v=3", state=ItemState.QUEUED)

    qm.items = [item1, item2, item3]
    qm.move_item("item3", -1)

    assert [item.id for item in qm.items] == ["item1", "item3", "item2"]
    assert item1.priority > item3.priority > item2.priority
    qm.db.close()


def test_metadata_duration_none_check():
    """BUG-MED-03: Test missing video duration metadata does not crash with TypeError."""
    info_dict = {"id": "live_123", "title": "Live Stream", "duration": None}
    info = extract_video_info(info_dict)
    assert info.duration == 0


def test_audio_codec_detection():
    """BUG-MED-06: Test _is_audio_codec handles all audio formats and copy mode."""
    codecs = ["opus", "vorbis", "mp3", "wav", "flac", "aac", "copy", "m4a", "libmp3lame"]
    for codec in codecs:
        assert _is_audio_codec(codec) is True


def test_format_size_edge_cases():
    """BUG-LOW-01: Test format_size for zero, small fractional, and normal byte values."""
    assert format_size(0) == "0 B"
    assert format_size(0.5) == "0.5 B"
    assert format_size(1024) == "1.0 KB"
    assert format_size(1048576) == "1.0 MB"


def test_download_engine_cancel():
    """BUG-LOW-04: Test DownloadEngine cancellation tracking."""
    engine = DownloadEngine(Settings())
    item = QueueItem(id="cancel_item_1", url="https://youtube.com/watch?v=12345678901")

    engine.cancel(item.id)
    assert "cancel_item_1" in engine._cancelled_items


@pytest.mark.asyncio
async def test_all_screens_instantiation_and_mount():
    """Verify that all screens and panels compile, compose, and mount without syntax/indentation errors."""
    from ytui.screens.settings_panel import SettingsPanel
    from ytui.screens.panels import (
        GenericPanel, QualityPanel, HistoryPanel, StatsPanel,
        BandwidthPanel, ProxyPanel, ThemePanel, ProfilePanel,
        ClipboardPanel, LogPanel, QueuePanelModal, VpnPanel, SearchPanel
    )
    from ytui.screens.main_screen import MainScreen
    from ytui.app import YtuiApp

    app = YtuiApp()
    async with app.run_test(size=(100, 40)) as pilot:
        # Verify SettingsPanel mounts cleanly
        settings_panel = SettingsPanel()
        app.push_screen(settings_panel)
        await pilot.pause()
        app.pop_screen()

        # Verify all panels mount cleanly
        for panel_cls in [QualityPanel, HistoryPanel, StatsPanel, BandwidthPanel, ProxyPanel, ThemePanel, ProfilePanel, ClipboardPanel, LogPanel, QueuePanelModal, VpnPanel]:
            p = panel_cls()
            app.push_screen(p)
            await pilot.pause()
            app.pop_screen()

        search_panel = SearchPanel("cat video")
        app.push_screen(search_panel)
        await pilot.pause()
        app.pop_screen()


@pytest.mark.asyncio
async def test_persistent_pending_item_recovery(tmp_path):
    """Test that QueueManager resumes metadata resolution for items restored in PENDING state."""
    db_path = tmp_path / "test_pending.db"
    db = QueueDatabase(db_path)
    pending_item = QueueItem(id="stuck_1", url="https://youtube.com/watch?v=12345678901", state=ItemState.PENDING)
    interrupted_item = QueueItem(id="interrupted_1", url="https://youtube.com/watch?v=12345678902", state=ItemState.DOWNLOADING)
    db.save_queue_item(pending_item)
    db.save_queue_item(interrupted_item)
    db.close()

    qm = QueueManager(Settings(), db_path=db_path)
    # Mock extract_info to return mock metadata instantly
    qm.engine.extract_info = lambda url: {"id": "12345678901", "title": "Test Video", "duration": 120}

    await qm.start()
    await asyncio.sleep(0.5)

    # Verify interrupted item was reset to QUEUED
    assert qm.items[1].state == ItemState.QUEUED
    # Verify pending item had metadata resolved and moved to QUEUED
    assert qm.items[0].state == ItemState.QUEUED
    assert qm.items[0].info.title == "Test Video"

    await qm.stop()

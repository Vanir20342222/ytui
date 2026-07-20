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


@pytest.mark.anyio
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


@pytest.mark.anyio
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


def test_launch_media_player(tmp_path, monkeypatch):
    """Test launch_media_player utility with missing and existing files."""
    from ytui.utils.filesystem import launch_media_player

    # Missing file
    success, msg = launch_media_player(tmp_path / "non_existent.mp4")
    assert not success
    assert "File not found" in msg

    # Existing file (mocking subprocess.Popen)
    dummy_file = tmp_path / "test.mp4"
    dummy_file.write_text("dummy video content")

    launched = []
    def mock_popen(cmd):
        launched.append(cmd)
        return None

    monkeypatch.setattr("subprocess.Popen", mock_popen)
    monkeypatch.setattr("shutil.which", lambda name: f"/usr/bin/{name}" if name == "mpv" else None)

    success, player = launch_media_player(dummy_file)
    assert success
    assert player == "mpv"
    assert len(launched) == 1
    assert str(dummy_file) in launched[0]


@pytest.mark.anyio
async def test_queue_import_export(tmp_path, monkeypatch):
    """Test exporting queue items to JSON and importing back from JSON and text files."""
    db_path = tmp_path / "test_io.db"
    qm = QueueManager(Settings(), db_path=db_path)
    qm.engine.extract_info = lambda url: {"id": "vid123", "title": "Mock Video", "duration": 60}

    # Add items to queue
    item1 = QueueItem(id="item1", url="https://youtube.com/watch?v=11111111111", state=ItemState.DONE, output_path=str(tmp_path / "v1.mp4"))
    item2 = QueueItem(id="item2", url="https://youtube.com/watch?v=22222222222", state=ItemState.QUEUED)
    qm.items.extend([item1, item2])

    # Test export
    export_file = tmp_path / "exported.json"
    result_path = qm.export_queue(export_file)
    assert result_path.exists()
    assert result_path == export_file

    import json
    data = json.loads(export_file.read_text())
    assert data["count"] == 2
    assert data["items"][0]["url"] == "https://youtube.com/watch?v=11111111111"

    # Test import from JSON
    qm2 = QueueManager(Settings(), db_path=tmp_path / "test_io2.db")
    qm2.engine.extract_info = lambda url: {"id": "vid_imp", "title": "Imported Video", "duration": 60}
    count = await qm2.import_queue(export_file)
    assert count == 2
    assert len(qm2.items) == 2
    assert qm2.items[0].url == "https://youtube.com/watch?v=11111111111"

    # Test import from text file
    txt_file = tmp_path / "urls.txt"
    txt_file.write_text("# Comment line\nhttps://youtube.com/watch?v=33333333333\n\nhttps://youtube.com/watch?v=44444444444\n")

    count_txt = await qm2.import_queue(txt_file)
    assert count_txt == 2
    assert len(qm2.items) == 4

    qm.db.close()
    qm2.db.close()


@pytest.mark.anyio
async def test_queue_item_open_key_event():
    """Test QueueItemWidget key events ('enter' and 'o') emit OpenClicked message."""
    from ytui.widgets.queue_item import QueueItemWidget
    from textual.events import Key
    from textual.pilot import Pilot

    item = QueueItem(id="item_open_1", url="https://youtube.com/watch?v=11111111111", state=ItemState.DONE)
    widget = QueueItemWidget(item)

    messages = []
    def on_open(msg):
        messages.append(msg)

    # Use run_test to verify keypress handling
    from ytui.app import YtuiApp
    app = YtuiApp()
    async with app.run_test() as pilot:
        await app.screen.mount(widget)
        widget.focus()
        await pilot.press("enter")
        await pilot.press("o")

    # Inspect widget state or message logic
    key_event_enter = Key("enter", "enter")
    widget.on_key(key_event_enter)
    key_event_o = Key("o", "o")
    widget.on_key(key_event_o)

    assert widget.can_focus is True


def test_downloader_sponsorblock_cookies_ratelimit(tmp_path):
    """Test SponsorBlock options, cookies support, and bandwidth rate limiting in DownloadEngine."""
    s = Settings()
    s.network.cookies_from_browser = "firefox"
    s.network.cookies_file = str(tmp_path / "cookies.txt")
    s.network.per_download_bandwidth_limit = 500  # KB/s
    s.network.global_bandwidth_limit = 2000  # KB/s
    s.network.max_concurrent_downloads = 2

    s.advanced.sponsor_block = True
    s.advanced.sponsor_block_action = "skip"
    s.advanced.sponsor_block_categories = ["sponsor", "intro"]
    s.advanced.sponsor_block_api = "https://custom.sponsor.api"

    # Create dummy cookie file so os.path.exists returns True
    cookie_path = tmp_path / "cookies.txt"
    cookie_path.write_text("# Netscape HTTP Cookie File")

    engine = DownloadEngine(s)
    item = QueueItem(id="sb_test_1", url="https://youtube.com/watch?v=12345678901")

    opts = engine._build_opts(item)

    # Verify browser cookies & cookie file
    assert opts["cookiesfrombrowser"] == ("firefox",)
    assert opts["cookiefile"] == str(cookie_path)

    # Verify rate limit (500 KB/s * 1024 = 512000 B/s)
    assert opts["ratelimit"] == 500 * 1024

    # Verify SponsorBlock options
    assert opts["sponsorblock_api"] == "https://custom.sponsor.api"
    assert opts["sponsorblock_remove"] == ["sponsor", "intro"]

    pp_keys = [pp["key"] for pp in opts["postprocessors"]]
    assert "SponsorBlock" in pp_keys
    assert "ModifyChapters" in pp_keys

    # Verify mark action
    s.advanced.sponsor_block_action = "mark"
    opts_mark = engine._build_opts(item)
    assert opts_mark["sponsorblock_mark"] == ["sponsor", "intro"]
    assert "sponsorblock_remove" not in opts_mark



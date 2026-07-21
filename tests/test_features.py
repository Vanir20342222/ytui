"""Unit tests for post-download custom scripts, SponsorBlock/Cookie controls, and format resolution tools."""

from __future__ import annotations

import asyncio
from pathlib import Path
import pytest

from ytui.config.settings import Settings
from ytui.engine.downloader import DownloadEngine
from ytui.queue.manager import QueueManager
from ytui.queue.models import QueueItem, ItemState, VideoInfo
from ytui.screens.settings_panel import SettingsPanel
from ytui.utils.format_resolution import (
    filter_formats,
    format_resolution_label,
    get_available_resolutions,
    is_audio_only,
    is_video_only,
    parse_resolution,
    select_best_format,
)


def test_format_resolution_helpers():
    """Test format resolution parsing and selection helper tools."""
    # Test parse_resolution
    assert parse_resolution("1920x1080") == (1920, 1080)
    assert parse_resolution("1080p") == (1920, 1080)
    assert parse_resolution(height=720) == (1280, 720)
    assert parse_resolution(height=720, width=1280) == (1280, 720)
    assert parse_resolution(None) == (0, 0)

    # Test format_resolution_label
    assert format_resolution_label(2160) == "4K (2160p)"
    assert format_resolution_label(1080, fps=60) == "1080p60"
    assert format_resolution_label(720) == "720p"
    assert format_resolution_label(0) == "Audio Only"

    # Test get_available_resolutions
    sample_formats = [
        {"format_id": "137", "vcodec": "avc1", "height": 1080},
        {"format_id": "136", "vcodec": "avc1", "height": 720},
        {"format_id": "140", "vcodec": "none", "acodec": "mp4a", "height": None},
        {"format_id": "313", "vcodec": "vp9", "height": 2160},
    ]
    resolutions = get_available_resolutions(sample_formats)
    assert resolutions == ["2160p", "1080p", "720p"]

    # Test is_video_only and is_audio_only
    video_fmt = {"vcodec": "avc1", "acodec": "none"}
    audio_fmt = {"vcodec": "none", "acodec": "mp4a"}
    combo_fmt = {"vcodec": "avc1", "acodec": "mp4a"}

    assert is_video_only(video_fmt) is True
    assert is_audio_only(video_fmt) is False
    assert is_audio_only(audio_fmt) is True
    assert is_video_only(audio_fmt) is False
    assert is_video_only(combo_fmt) is False
    assert is_audio_only(combo_fmt) is False

    # Test filter_formats and select_best_format
    filtered = filter_formats(sample_formats, mode="video", max_height=1080)
    assert len(filtered) == 2
    assert filtered[0]["format_id"] == "137"

    best_fmt = select_best_format(sample_formats, target_quality="1080", mode="video")
    assert best_fmt == "137+bestaudio/best"


def test_sponsor_block_and_cookie_settings(tmp_path):
    """Test SponsorBlock and Cookie configuration options."""
    s = Settings()
    s.advanced.sponsor_block = True
    s.advanced.sponsor_block_action = "skip"
    s.advanced.sponsor_block_categories = ["sponsor", "intro"]
    s.network.cookie_file = str(tmp_path / "cookies.txt")
    s.network.browser_cookies = "firefox"

    # Create dummy cookie file
    cookie_file = tmp_path / "cookies.txt"
    cookie_file.write_text("# Netscape HTTP Cookie File\n")

    engine = DownloadEngine(s)
    item = QueueItem(id="item1", url="https://youtube.com/watch?v=12345678901")
    opts = engine._build_opts(item)

    assert "postprocessors" in opts
    sb_pp = next((pp for pp in opts["postprocessors"] if pp.get("key") == "SponsorBlock"), None)
    assert sb_pp is not None
    assert sb_pp["categories"] == ["sponsor", "intro"]
    assert opts.get("sponsorblock_remove") == ["sponsor", "intro"]
    assert opts.get("cookiefile") == str(cookie_file)


def test_post_download_script_execution(tmp_path):
    """Test execution of post-download custom script."""
    output_file = tmp_path / "output.txt"

    # Create a simple executable post-download script
    script_file = tmp_path / "post_process.sh"
    script_content = f"#!/bin/sh\necho \"Processed $1\" > \"{output_file}\"\n"
    script_file.write_text(script_content)
    script_file.chmod(0o755)

    settings = Settings()
    settings.advanced.enable_custom_script = True
    settings.advanced.custom_script = str(script_file)

    qm = QueueManager(settings, db_path=tmp_path / "test_script.db")
    item = QueueItem(
        id="test_item_1",
        url="https://youtube.com/watch?v=12345678901",
        download_mode="video",
        info=VideoInfo(video_id="12345678901", title="Test Script Video"),
    )

    downloaded_path = str(tmp_path / "video.mp4")
    qm._run_post_download_script(item, downloaded_path)

    assert output_file.exists()
    assert downloaded_path in output_file.read_text()
    qm.db.close()


@pytest.mark.anyio
async def test_settings_panel_controls_mount():
    """Verify SettingsPanel mounts with new SponsorBlock, Cookie, and script controls."""
    from ytui.app import YtuiApp
    app = YtuiApp()
    async with app.run_test(size=(100, 40)) as pilot:
        panel = SettingsPanel(initial_tab="tab-advanced")
        app.push_screen(panel)
        await pilot.pause()

        # Save settings and ensure no exceptions thrown
        panel.save_settings()
        app.pop_screen()


def test_playlist_group_expanded_persistence(tmp_path):
    """Test saving and loading PlaylistGroup expanded state in QueueDatabase."""
    from ytui.queue.persistence import QueueDatabase
    from ytui.queue.models import PlaylistGroup

    db = QueueDatabase(db_path=tmp_path / "test_group.db")
    group = PlaylistGroup(
        id="grp123",
        playlist_id="PL123",
        title="Test Playlist",
        url="https://youtube.com/playlist?list=PL123",
        expanded=True,
    )
    db.save_playlist_group(group)

    loaded = db.load_playlist_groups()
    assert len(loaded) == 1
    assert loaded[0].id == "grp123"
    assert loaded[0].expanded is True
    db.close()


@pytest.mark.anyio
async def test_local_file_conversion(tmp_path):
    """Test local file conversion metadata resolution and execution in QueueManager."""
    src_file = tmp_path / "sample_video.mkv"
    src_file.write_text("mock video content")

    settings = Settings()
    settings.directories.video_dir = str(tmp_path / "converted_videos")

    qm = QueueManager(settings, db_path=tmp_path / "test_convert.db")

    # Add local file
    res = await qm.add_url(str(src_file))
    await asyncio.sleep(0.1)
    assert isinstance(res, QueueItem)
    assert res.info.uploader == "Local File"
    assert res.state == ItemState.QUEUED

    qm.db.close()


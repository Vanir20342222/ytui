"""yt-dlp self-update from official GitHub releases."""

from __future__ import annotations

import logging
import subprocess
import sys

import yt_dlp

logger = logging.getLogger(__name__)


def get_current_version() -> str:
    """Get the currently installed yt-dlp version."""
    try:
        return yt_dlp.version.__version__
    except Exception:
        return "unknown"


async def check_for_update() -> tuple[str, str, bool]:
    """Check for a newer yt-dlp version.
    
    Returns:
        Tuple of (current_version, latest_version, update_available).
    """
    import httpx
    
    current = get_current_version()
    try:
        async with httpx.AsyncClient() as client:
            resp = await client.get(
                "https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest",
                follow_redirects=True,
                timeout=15,
            )
            resp.raise_for_status()
            data = resp.json()
            latest = data.get("tag_name", "").lstrip("v")
            return current, latest, latest != current
    except Exception as e:
        logger.error(f"Update check failed: {e}")
        return current, "unknown", False


GITHUB_YTUI_REPO = "Vanir20342222/ytui"


async def check_ytui_update() -> tuple[str, str, bool, str]:
    """Check GitHub repository for a newer ytui release or commit.

    Returns:
        Tuple of (current_version, latest_version_or_tag, update_available, release_notes).
    """
    import httpx
    from ytui.constants import APP_VERSION

    current = APP_VERSION
    headers = {"User-Agent": f"ytui/{current}"}

    try:
        async with httpx.AsyncClient(headers=headers, follow_redirects=True, timeout=10) as client:
            # 1. Check latest release tag
            resp = await client.get(f"https://api.github.com/repos/{GITHUB_YTUI_REPO}/releases/latest")
            if resp.status_code == 200:
                data = resp.json()
                latest_tag = data.get("tag_name", "").lstrip("v")
                body = data.get("body", "New release available on GitHub.")
                if latest_tag and latest_tag != current:
                    return current, latest_tag, True, body

            # 2. Check main branch latest commit as fallback
            import asyncio
            proc = await asyncio.create_subprocess_exec(
                "git", "rev-parse", "--short", "HEAD",
                stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.PIPE,
            )
            stdout, _ = await proc.communicate()
            if proc.returncode == 0:
                local_commit = stdout.decode().strip()
                resp = await client.get(f"https://api.github.com/repos/{GITHUB_YTUI_REPO}/commits/main")
                if resp.status_code == 200:
                    data = resp.json()
                    remote_commit = data.get("sha", "")[:7]
                    commit_msg = data.get("commit", {}).get("message", "").split("\n")[0]
                    if remote_commit and local_commit and remote_commit != local_commit:
                        return current, f"commit-{remote_commit}", True, f"New commit on main: {commit_msg}"

            return current, current, False, ""
    except Exception as e:
        logger.error(f"ytui update check failed: {e}")
        return current, current, False, ""


def update_ytui() -> tuple[bool, str]:
    """Execute ytui self-update via git pull or pip install."""
    try:
        res = subprocess.run(
            ["git", "rev-parse", "--is-inside-work-tree"],
            capture_output=True,
            text=True,
            timeout=5,
        )
        if res.returncode == 0 and "true" in res.stdout.strip():
            pull_res = subprocess.run(
                ["git", "pull", "origin", "main"],
                capture_output=True,
                text=True,
                timeout=60,
            )
            if pull_res.returncode == 0:
                subprocess.run([sys.executable, "-m", "pip", "install", "-e", "."], capture_output=True, timeout=60)
                return True, "ytui updated successfully via git pull! Please restart the application."
    except Exception:
        pass

    try:
        res = subprocess.run(
            [sys.executable, "-m", "pip", "install", "-U", f"git+https://github.com/{GITHUB_YTUI_REPO}.git"],
            capture_output=True,
            text=True,
            timeout=120,
        )
        if res.returncode == 0:
            return True, "ytui updated successfully via pip! Please restart the application."
        else:
            return False, f"Update failed: {res.stderr[:200]}"
    except Exception as e:
        return False, f"Update failed: {e}"

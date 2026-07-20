"""Cross-platform desktop notification support."""

from __future__ import annotations

import logging
import platform
import subprocess
import shutil

logger = logging.getLogger(__name__)


def send_notification(
    title: str,
    message: str,
    urgency: str = "normal",
) -> None:
    """Send a desktop notification.

    Args:
        title: Notification title.
        message: Notification body text.
        urgency: One of 'low', 'normal', 'critical'.
    """
    system = platform.system()
    try:
        if system == "Linux":
            _notify_linux(title, message, urgency)
        elif system == "Darwin":
            _notify_macos(title, message)
        elif system == "Windows":
            _notify_windows(title, message)
    except Exception as e:
        logger.debug(f"Notification failed: {e}")


def _notify_linux(title: str, message: str, urgency: str) -> None:
    """Send notification via notify-send on Linux."""
    if shutil.which("notify-send"):
        subprocess.Popen([
            "notify-send",
            "--urgency", urgency,
            "--app-name", "ytui",
            title,
            message,
        ])


def _notify_macos(title: str, message: str) -> None:
    """Send notification via osascript on macOS."""
    script = f'display notification "{message}" with title "{title}"'
    subprocess.Popen(["osascript", "-e", script])


def _notify_windows(title: str, message: str) -> None:
    """Send notification via PowerShell on Windows."""
    # Uses BurntToast or built-in toast if available
    ps_script = f"""
    [Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] > $null
    $template = [Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent([Windows.UI.Notifications.ToastTemplateType]::ToastText02)
    $textNodes = $template.GetElementsByTagName('text')
    $textNodes.Item(0).AppendChild($template.CreateTextNode('{title}')) > $null
    $textNodes.Item(1).AppendChild($template.CreateTextNode('{message}')) > $null
    $toast = [Windows.UI.Notifications.ToastNotification]::new($template)
    [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('ytui').Show($toast)
    """
    try:
        subprocess.Popen(
            ["powershell", "-NoProfile", "-Command", ps_script],
            creationflags=getattr(subprocess, 'CREATE_NO_WINDOW', 0),
        )
    except Exception:
        pass  # Windows notification is best-effort

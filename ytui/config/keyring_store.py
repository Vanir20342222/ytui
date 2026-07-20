"""Secure secret storage via OS keyring with fallback."""

from __future__ import annotations

import json
import logging
import os
import stat
from pathlib import Path

from ytui.config.settings import _config_dir

logger = logging.getLogger(__name__)

SERVICE_NAME = "ytui"


def _keyring_available() -> bool:
    """Check if the OS keyring is accessible."""
    try:
        import keyring
        # Try a test operation
        keyring.get_password(SERVICE_NAME, "__test__")
        return True
    except Exception:
        return False


def _fallback_path() -> Path:
    """Path for the permission-restricted fallback secrets file."""
    return _config_dir() / ".secrets"


def _load_fallback() -> dict[str, str]:
    """Load secrets from fallback file."""
    path = _fallback_path()
    if not path.exists():
        return {}
    try:
        with open(path, "r") as f:
            return json.load(f)
    except Exception:
        return {}


def _save_fallback(data: dict[str, str]) -> None:
    """Save secrets to fallback file with restricted permissions (0600)."""
    path = _fallback_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    flags = os.O_WRONLY | os.O_CREAT | os.O_TRUNC
    fd = os.open(path, flags, 0o600)
    try:
        os.chmod(path, stat.S_IRUSR | stat.S_IWUSR)
    except Exception:
        pass  # Windows doesn't support Unix permissions the same way
    with open(fd, "w") as f:
        json.dump(data, f)


def store_secret(key: str, value: str) -> None:
    """Store a secret securely."""
    if _keyring_available():
        import keyring
        keyring.set_password(SERVICE_NAME, key, value)
        logger.debug(f"Secret '{key}' stored in OS keyring")
    else:
        data = _load_fallback()
        data[key] = value
        _save_fallback(data)
        logger.debug(f"Secret '{key}' stored in fallback file")


def get_secret(key: str) -> str | None:
    """Retrieve a secret."""
    if _keyring_available():
        import keyring
        return keyring.get_password(SERVICE_NAME, key)
    else:
        data = _load_fallback()
        return data.get(key)


def delete_secret(key: str) -> None:
    """Delete a stored secret."""
    if _keyring_available():
        try:
            import keyring
            keyring.delete_password(SERVICE_NAME, key)
        except Exception:
            pass
    else:
        data = _load_fallback()
        data.pop(key, None)
        _save_fallback(data)

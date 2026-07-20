"""Keybinding definitions and remapping support."""

from __future__ import annotations

import json
import logging
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from ytui.config.settings import _config_dir

logger = logging.getLogger(__name__)


# Default keybinding map
DEFAULT_KEYBINDINGS: dict[str, str] = {
    # Navigation
    "focus_input": "slash",
    "scroll_up": "up",
    "scroll_down": "down",
    "page_up": "pageup",
    "page_down": "pagedown",
    "select_item": "enter",

    # Queue operations
    "pause_resume": "space",
    "cancel_item": "delete",
    "move_up": "ctrl+up",
    "move_down": "ctrl+down",
    "pause_all": "ctrl+p",
    "resume_all": "ctrl+r",

    # Panels
    "open_quality": "ctrl+q",
    "open_settings": "ctrl+comma",
    "open_help": "f1",
    "open_log": "ctrl+l",
    "open_search": "ctrl+s",
    "open_history": "ctrl+h",

    # App
    "close_panel": "escape",
    "quit": "ctrl+c",
    "toggle_clipboard": "ctrl+shift+c",
}


@dataclass
class KeybindingConfig:
    """Manages keybinding configuration with persistence."""
    bindings: dict[str, str] = field(default_factory=lambda: dict(DEFAULT_KEYBINDINGS))

    @staticmethod
    def _path() -> Path:
        return _config_dir() / "keybindings.json"

    @classmethod
    def load(cls) -> KeybindingConfig:
        path = cls._path()
        if not path.exists():
            return cls()
        try:
            with open(path, "r") as f:
                data = json.load(f)
            merged = dict(DEFAULT_KEYBINDINGS)
            merged.update(data)
            return cls(bindings=merged)
        except Exception as e:
            logger.error(f"Failed to load keybindings: {e}")
            return cls()

    def save(self) -> None:
        path = self._path()
        path.parent.mkdir(parents=True, exist_ok=True)
        # Only save non-default bindings
        custom = {
            k: v for k, v in self.bindings.items()
            if k not in DEFAULT_KEYBINDINGS or DEFAULT_KEYBINDINGS.get(k) != v
        }
        tmp_path = path.with_suffix(".tmp")
        try:
            with open(tmp_path, "w") as f:
                json.dump(custom, f, indent=2)
            import os
            os.replace(tmp_path, path)
        except Exception as e:
            if tmp_path.exists():
                try:
                    tmp_path.unlink()
                except Exception:
                    pass
            logger.error(f"Failed to save keybindings: {e}")

    def get(self, action: str) -> str:
        return self.bindings.get(action, DEFAULT_KEYBINDINGS.get(action, ""))

    def set(self, action: str, key: str) -> None:
        self.bindings[action] = key

    def reset(self, action: str | None = None) -> None:
        if action:
            self.bindings[action] = DEFAULT_KEYBINDINGS.get(action, "")
        else:
            self.bindings = dict(DEFAULT_KEYBINDINGS)

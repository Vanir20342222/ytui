"""Inline command suggestion dropdown for / commands."""

from __future__ import annotations

import os

from textual.app import ComposeResult
from textual.containers import Vertical
from textual.message import Message
from textual.reactive import reactive
from textual.widget import Widget
from textual.widgets import Static

from ytui.constants import COMMANDS


class CommandSuggestions(Widget):
    """Dropdown widget for / command suggestions with fuzzy matching."""

    DEFAULT_CSS = """
    CommandSuggestions {
        dock: bottom;
        offset-y: -4;
        height: auto;
        max-height: 10;
        background: $surface;
        border: round $border;
        display: none;
        margin: 0 2;
        layer: overlay;
    }
    CommandSuggestions.visible {
        display: block;
    }
    CommandSuggestions .cmd-row {
        height: 1;
        padding: 0 1;
    }
    CommandSuggestions .cmd-row:hover {
        background: $surface;
    }
    CommandSuggestions .cmd-row--selected {
        background: $accent 30%;
        color: white;
    }
    """

    class CommandSelected(Message):
        """Posted when a command is selected."""
        def __init__(self, command: str) -> None:
            super().__init__()
            self.command = command

    filter_text = reactive("", layout=True)
    selected_index = reactive(0)
    _is_open = reactive(False)

    def __init__(self, **kwargs) -> None:
        super().__init__(**kwargs)
        self._filtered_commands: list[tuple[str, str]] = []

    def compose(self) -> ComposeResult:
        yield Vertical(id="suggestions-list")

    def watch_filter_text(self, value: str) -> None:
        """Update filtered commands when filter changes."""
        self._update_suggestions()

    def watch_selected_index(self, value: int) -> None:
        """Highlight the selected suggestion."""
        self._highlight_selected()

    def watch__is_open(self, value: bool) -> None:
        """Toggle visibility class based on is_open state."""
        if value and self._filtered_commands:
            self.add_class("visible")
        else:
            self.remove_class("visible")

    def _update_suggestions(self) -> None:
        """Rebuild the suggestion list based on current filter."""
        query = self.filter_text.lower().lstrip("/")
        
        if not query:
            # Show all commands
            self._filtered_commands = list(COMMANDS.items())
        else:
            # Fuzzy filter: command must contain all query chars in order
            self._filtered_commands = []
            for cmd, desc in COMMANDS.items():
                cmd_lower = cmd.lower()
                if self._fuzzy_match(query, cmd_lower.lstrip("/")):
                    self._filtered_commands.append((cmd, desc))

        # Remove aliases (keep unique descriptions)
        seen_descs: set[str] = set()
        unique: list[tuple[str, str]] = []
        for cmd, desc in self._filtered_commands:
            if desc not in seen_descs:
                seen_descs.add(desc)
                unique.append((cmd, desc))
        self._filtered_commands = unique

        self.selected_index = 0
        self._rebuild_list()

        # Show/hide based on open state and matches
        if self._is_open and self._filtered_commands:
            self.add_class("visible")
        else:
            self.remove_class("visible")

    def _fuzzy_match(self, query: str, text: str) -> bool:
        """Simple fuzzy match — all query chars appear in order in text."""
        qi = 0
        for char in text:
            if qi < len(query) and char == query[qi]:
                qi += 1
        return qi == len(query)

    def _rebuild_list(self) -> None:
        """Rebuild the suggestion widgets."""
        try:
            container = self.query_one("#suggestions-list", Vertical)
            container.remove_children()
            for i, (cmd, desc) in enumerate(self._filtered_commands):
                classes = "cmd-row"
                if i == self.selected_index:
                    classes += " cmd-row--selected"
                row = Static(f"  {cmd:<14} {desc}", classes=classes)
                container.mount(row)
        except Exception:
            pass

    def _highlight_selected(self) -> None:
        """Update highlighting on the selected row."""
        try:
            rows = self.query(".cmd-row")
            for i, row in enumerate(rows):
                if i == self.selected_index:
                    row.add_class("cmd-row--selected")
                else:
                    row.remove_class("cmd-row--selected")
        except Exception:
            pass

    def move_selection(self, direction: int) -> None:
        """Move selection up or down."""
        if not self._filtered_commands:
            return
        new_idx = self.selected_index + direction
        self.selected_index = max(0, min(len(self._filtered_commands) - 1, new_idx))

    def tab_complete(self, current_text: str) -> str | None:
        """Return the completed text for Tab.

        If a single command starts with the typed prefix, return it. If the
        currently-highlighted row matches the prefix, return that one
        (so the user can disambiguate with up/down then Tab). Otherwise return
        the longest common prefix of the matching commands.
        """
        prefix = current_text.lstrip("/")
        if not prefix:
            return None
        matches = [
            cmd for cmd, _ in self._filtered_commands
            if cmd.lstrip("/").startswith(prefix)
        ]
        if not matches:
            return None
        if len(matches) == 1:
            return matches[0]
        # Prefer the highlighted row if it matches the prefix.
        sel = self.get_selected_command()
        if sel and sel.lstrip("/").startswith(prefix):
            return sel
        # Otherwise return the longest common prefix.
        common = os.path.commonprefix(matches)
        return common if common else None

    def get_selected_command(self) -> str | None:
        """Get the currently selected command string."""
        if 0 <= self.selected_index < len(self._filtered_commands):
            return self._filtered_commands[self.selected_index][0]
        return None

    def show(self, filter_text: str = "") -> None:
        """Show suggestions with optional filter."""
        self._is_open = True
        self.filter_text = filter_text

    def hide(self) -> None:
        """Hide the suggestion dropdown."""
        self._is_open = False

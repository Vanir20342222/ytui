"""Main input bar — the primary interaction point.

Always focused, always at the bottom. Handles:
- URL pasting (validated, sent to queue)
- / commands (inline suggestion mode)
- Plain text (YouTube search)
"""

from __future__ import annotations

from textual.app import ComposeResult
from textual.events import Key
from textual.message import Message
from textual.widget import Widget
from textual.widgets import Input, Static

from ytui.utils.urls import is_valid_url, normalize_url
from ytui.widgets.command_suggestions import CommandSuggestions


class InputBar(Widget):
    """The always-visible input bar at the bottom of the screen."""

    DEFAULT_CSS = """
    InputBar {
        dock: bottom;
        height: 4;
        padding: 0 1 0 1;
    }
    InputBar #input-hint {
        height: 1;
        color: $text-muted;
        padding: 0 1;
    }
    InputBar #main-input {
        height: 3;
        border: round $border;
        background: $surface;
        color: $text;
        padding: 0 1;
    }
    InputBar #main-input:focus {
        border: round $accent;
    }
    """

    class UrlSubmitted(Message):
        """A URL was pasted/typed and submitted."""
        def __init__(self, url: str) -> None:
            super().__init__()
            self.url = url

    class CommandSubmitted(Message):
        """A / command was submitted."""
        def __init__(self, command: str) -> None:
            super().__init__()
            self.command = command

    class SearchSubmitted(Message):
        """A search query was submitted."""
        def __init__(self, query: str) -> None:
            super().__init__()
            self.query = query

    def __init__(self, **kwargs) -> None:
        super().__init__(**kwargs)
        self._command_mode = False

    def compose(self) -> ComposeResult:
        yield Static(
            "Paste a link, type a search, or / for commands",
            id="input-hint",
        )
        yield Input(
            placeholder="> ",
            id="main-input",
        )

    def _get_suggestions(self) -> CommandSuggestions | None:
        try:
            return self.screen.query_one("#cmd-suggestions", CommandSuggestions)
        except Exception:
            return None

    def on_mount(self) -> None:
        """Focus the input on mount."""
        self.query_one("#main-input", Input).focus()

    def on_input_changed(self, event: Input.Changed) -> None:
        """Handle input text changes for command suggestion mode."""
        if event.input.id != "main-input":
            return

        value = event.value
        suggestions = self._get_suggestions()
        input_widget = self.query_one("#main-input", Input)

        if value.startswith("/"):
            self._command_mode = True
            if suggestions:
                suggestions.show(value)
            input_widget.remove_class("-invalid")
        else:
            self._command_mode = False
            if suggestions:
                suggestions.hide()
            # Flag malformed URLs in real time (only once it looks like a URL).
            looks_like_url = "://" in value or value.startswith("youtu.be/")
            if looks_like_url and not is_valid_url(value):
                input_widget.add_class("-invalid")
            else:
                input_widget.remove_class("-invalid")

    def on_input_submitted(self, event: Input.Submitted) -> None:
        """Handle Enter key in the input."""
        if event.input.id != "main-input":
            return

        value = event.value.strip()
        if not value:
            return

        input_widget = self.query_one("#main-input", Input)
        suggestions = self._get_suggestions()

        if self._command_mode:
            # Get selected command from suggestions
            selected = suggestions.get_selected_command() if suggestions else None
            cmd = selected or value
            if suggestions:
                suggestions.hide()
            self._command_mode = False
            input_widget.value = ""
            self.post_message(self.CommandSubmitted(cmd))

        elif is_valid_url(value):
            # URL submitted
            url = normalize_url(value)
            input_widget.value = ""
            self.post_message(self.UrlSubmitted(url))

        else:
            # Search query
            input_widget.value = ""
            self.post_message(self.SearchSubmitted(value))

    def on_key(self, event: Key) -> None:
        """Handle arrow keys and Tab for command suggestions."""
        if not self._command_mode:
            return

        suggestions = self._get_suggestions()
        input_widget = self.query_one("#main-input", Input)
        if not suggestions:
            return

        if event.key == "up":
            suggestions.move_selection(-1)
            event.prevent_default()
            event.stop()
        elif event.key == "down":
            suggestions.move_selection(1)
            event.prevent_default()
            event.stop()
        elif event.key == "escape":
            suggestions.hide()
            self._command_mode = False
            input_widget.value = ""
            event.prevent_default()
            event.stop()
        elif event.key == "tab":
            completed = suggestions.tab_complete(input_widget.value)
            if completed:
                input_widget.value = completed
                # Re-filter on the longer prefix so the list narrows.
                suggestions.show(completed)
            event.prevent_default()
            event.stop()

    def focus_input(self) -> None:
        """Focus the main input."""
        self.query_one("#main-input", Input).focus()

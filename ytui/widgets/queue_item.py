"""Queue item widget — displays a single download in the queue.

Handles all visual states: shimmer/pending, queued, downloading,
converting, merging, done, paused, error.
"""

from __future__ import annotations

from textual.app import ComposeResult
from textual.message import Message
from textual.reactive import reactive
from textual.widget import Widget
from textual.widgets import Button, Static

from ytui.constants import ICON_SETS
from ytui.queue.models import ItemState, QueueItem as QueueItemModel
from ytui.utils.formatting import format_duration, format_eta, format_size, format_speed


# Status icons for each state
_STATE_ICONS = {
    ItemState.PENDING: "..",     # shimmer/loading
    ItemState.QUEUED: ">",       # waiting
    ItemState.DOWNLOADING: "v",   # downloading
    ItemState.CONVERTING: "~",   # converting
    ItemState.MERGING: "=",      # merging
    ItemState.DONE: "+",         # complete
    ItemState.PAUSED: "||",      # paused
    ItemState.ERROR: "x",        # error
    ItemState.CANCELLED: "-",    # cancelled
}


class QueueItemWidget(Widget):
    """Visual representation of a queue item."""

    DEFAULT_CSS = """
    QueueItemWidget {
        height: auto;
        min-height: 2;
        max-height: 8;
        padding: 0 1;
        margin: 0;
    }
    QueueItemWidget:hover {
        background: $surface;
    }
    QueueItemWidget #qi-row1 {
        height: auto;
        min-height: 1;
        layout: horizontal;
    }
    QueueItemWidget #qi-icon {
        width: auto;
        min-width: 7;
        margin-right: 1;
        color: $text-muted;
    }
    QueueItemWidget #qi-title {
        width: 1fr;
        text-style: bold;
    }
    QueueItemWidget #qi-meta {
        width: auto;
        min-width: 8;
        text-align: right;
        color: $text-muted;
    }
    QueueItemWidget #qi-row2 {
        height: auto;
        min-height: 1;
        layout: horizontal;
        padding: 0 0 0 3;
        color: $text-muted;
    }
    QueueItemWidget #qi-progress-text {
        width: 1fr;
    }
    QueueItemWidget #qi-error-row {
        height: auto;
        min-height: 1;
        layout: horizontal;
        align: left top;
        padding: 0 0 0 3;
        color: $error;
        display: none;
    }
    QueueItemWidget #qi-error-text {
        width: 1fr;
        text-wrap: wrap;
    }
    QueueItemWidget #qi-retry-btn {
        width: auto;
        height: 1;
        padding: 0 1;
        background: $accent 20%;
        color: $accent;
        text-style: bold;
    }
    QueueItemWidget #qi-retry-btn:hover {
        background: $accent;
        color: $background;
    }
    QueueItemWidget .hidden {
        display: none;
    }
    """

    class RetryClicked(Message):
        def __init__(self, item_id: str) -> None:
            super().__init__()
            self.item_id = item_id

    class DetailsClicked(Message):
        def __init__(self, item_id: str) -> None:
            super().__init__()
            self.item_id = item_id

    item_id = reactive("")

    def __init__(self, item: QueueItemModel, **kwargs) -> None:
        super().__init__(**kwargs)
        self._item = item
        self.item_id = item.id

    def compose(self) -> ComposeResult:
        # Row 1: icon + title + meta (size, resolution)
        with Widget(id="qi-row1"):
            yield Static(id="qi-icon")
            yield Static(id="qi-title")
            yield Static(id="qi-meta")
        # Row 2: progress details
        with Widget(id="qi-row2"):
            yield Static(id="qi-progress-text")
        # Error row (hidden by default)
        with Widget(id="qi-error-row"):
            yield Static(id="qi-error-text")
            yield Static("Retry", id="qi-retry-btn", classes="hidden")

    def on_mount(self) -> None:
        self.refresh_display()

    def refresh_display(self) -> None:
        """Update all display elements from the item model."""
        item = self._item
        state = item.state
        # Get icon set from settings
        icon_style = "badges"
        if hasattr(self.app, "settings") and self.app.settings and hasattr(self.app.settings, "appearance"):
            icon_style = getattr(self.app.settings.appearance, "icon_style", "badges")
            
        icon_map = ICON_SETS.get(icon_style, ICON_SETS["badges"])
        state_key = state.name if hasattr(state, "name") else str(state).upper()
        icon = icon_map.get(state_key, "*")

        # Update CSS classes based on state
        self.remove_class(
            "queue-item--pending", "queue-item--downloading",
            "queue-item--done", "queue-item--error",
            "queue-item--paused"
        )
        if state == ItemState.PENDING:
            self.add_class("queue-item--pending")
        elif state in (ItemState.DOWNLOADING, ItemState.MERGING, ItemState.CONVERTING):
            self.add_class("queue-item--downloading")
        elif state == ItemState.DONE:
            self.add_class("queue-item--done")
        elif state == ItemState.ERROR:
            self.add_class("queue-item--error")
        elif state == ItemState.PAUSED:
            self.add_class("queue-item--paused")

        # Row 1
        try:
            self.query_one("#qi-icon", Static).update(icon)
            self.query_one("#qi-title", Static).update(item.display_title)

            # Meta: size + resolution
            meta_parts = []
            if item.progress.total_bytes > 0:
                meta_parts.append(format_size(item.progress.total_bytes))
            elif item.info.filesize_approx > 0:
                meta_parts.append(f"~{format_size(item.info.filesize_approx)}")
            if item.info.resolution:
                meta_parts.append(item.info.resolution)
            elif item.info.duration > 0:
                meta_parts.append(format_duration(item.info.duration))
            self.query_one("#qi-meta", Static).update("  ".join(meta_parts))
        except Exception:
            pass

        # Row 2: progress
        try:
            row2 = self.query_one("#qi-row2")
            progress_text = self.query_one("#qi-progress-text", Static)

            if state == ItemState.PENDING:
                progress_text.update(".. Fetching metadata...")
                row2.display = True
            elif state == ItemState.QUEUED:
                progress_text.update("Queued")
                row2.display = True
            elif state in (ItemState.DOWNLOADING, ItemState.MERGING, ItemState.CONVERTING):
                p = item.progress
                parts = []
                parts.append(f"{p.percent:.0f}%")

                # Visual progress bar characters
                bar_width = 20
                filled = int(p.percent / 100 * bar_width)
                bar = "▓" * filled + "░" * (bar_width - filled)
                parts.append(bar)

                if p.speed > 0:
                    parts.append(format_speed(p.speed))

                if p.downloaded_bytes > 0 and p.total_bytes > 0:
                    parts.append(
                        f"{format_size(p.downloaded_bytes)}/{format_size(p.total_bytes)}"
                    )

                if p.eta is not None and p.eta > 0:
                    parts.append(format_eta(p.eta))

                # Phase
                if state == ItemState.MERGING:
                    parts.append("· merging video+audio")
                elif state == ItemState.CONVERTING:
                    parts.append("· converting")

                progress_text.update("  ".join(parts))
                row2.display = True
            elif state == ItemState.DONE:
                progress_text.update("")
                row2.display = False
            elif state == ItemState.PAUSED:
                progress_text.update(f"|| Paused at {item.progress.percent:.0f}%")
                row2.display = True
            else:
                progress_text.update("")
                row2.display = False
        except Exception:
            pass

        # Error row
        try:
            error_row = self.query_one("#qi-error-row")
            if state == ItemState.ERROR and item.error_message:
                self.query_one("#qi-error-text", Static).update(
                    f"x {item.error_message}"
                )
                retry_btn = self.query_one("#qi-retry-btn", Static)
                if item.can_retry:
                    retry_btn.remove_class("hidden")
                else:
                    retry_btn.add_class("hidden")
                error_row.display = True
            else:
                error_row.display = False
        except Exception:
            pass

    def update_item(self, item: QueueItemModel) -> None:
        """Update the underlying item model and refresh."""
        self._item = item
        self.refresh_display()

    def on_click(self, event) -> None:
        if event.widget.id == "qi-retry-btn":
            self.post_message(self.RetryClicked(self._item.id))

    def on_button_pressed(self, event: Button.Pressed) -> None:
        if event.button.id == "qi-retry-btn":
            self.post_message(self.RetryClicked(self._item.id))

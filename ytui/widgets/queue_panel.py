"""Queue panel — scrollable list of queue items with aggregate header."""

from __future__ import annotations

from textual.app import ComposeResult
from textual.containers import ScrollableContainer, Vertical
from textual.reactive import reactive
from textual.widget import Widget
from textual.widgets import Static

from ytui.queue.models import QueueItem as QueueItemModel
from ytui.utils.formatting import format_eta, format_size, format_speed
from ytui.widgets.queue_item import QueueItemWidget


class QueuePanel(Widget):
    """The main queue display panel."""

    DEFAULT_CSS = """
    QueuePanel {
        height: 1fr;
        padding: 0 1;
    }
    QueuePanel #queue-header {
        height: 2;
        color: $text-muted;
        padding: 0 0 0 1;
    }
    QueuePanel #queue-title {
        text-style: bold;
        color: $text;
    }
    QueuePanel #queue-aggregate {
        color: $text-muted;
    }
    QueuePanel #queue-scroll {
        height: 1fr;
        scrollbar-size: 1 1;
    }
    QueuePanel #queue-empty {
        text-align: center;
        color: $text-muted;
        margin: 4 0;
    }
    """

    queue_count = reactive(0)

    def __init__(self, **kwargs) -> None:
        super().__init__(**kwargs)
        self._item_widgets: dict[str, QueueItemWidget] = {}
        self._synced = False

    def compose(self) -> ComposeResult:
        with Vertical(id="queue-header"):
            yield Static("QUEUE", id="queue-title")
            yield Static("", id="queue-aggregate")
        yield ScrollableContainer(id="queue-scroll")
        yield Static(
            "\n  Queue is empty \u2014 paste a YouTube link to get started\n",
            id="queue-empty",
        )

    def on_mount(self) -> None:
        """Sync existing queue items from the QueueManager once mounted.

        This handles the boot race: queue_manager.start() loads saved items and
        dispatches on_item_added before this widget is mounted, so those
        dispatches are dropped. Re-syncing here ensures startup items appear.
        """
        if self._synced:
            return
        self._synced = True
        try:
            qm = self.app.queue_manager
            if qm:
                for item in qm.items:
                    self.add_item(item)
        except Exception:
            pass
        # Refresh the header aggregate for the synced items.
        try:
            self.app._update_aggregate()
        except Exception:
            pass

    def watch_queue_count(self, count: int) -> None:
        try:
            self.query_one("#queue-empty").display = count == 0
            self.query_one("#queue-scroll").display = count > 0
        except Exception:
            pass

    def add_item(self, item: QueueItemModel) -> None:
        """Add a new item to the queue display (idempotent)."""
        if item.id in self._item_widgets:
            # Already present (e.g. from on_mount sync) — just refresh it.
            self._item_widgets[item.id].update_item(item)
            return
        widget = QueueItemWidget(item, classes="queue-item")
        self._item_widgets[item.id] = widget
        try:
            self.query_one("#queue-scroll", ScrollableContainer).mount(widget)
        except Exception:
            pass
        self.queue_count = len(self._item_widgets)

    def update_item(self, item: QueueItemModel) -> None:
        """Update an existing item's display."""
        widget = self._item_widgets.get(item.id)
        if widget:
            widget.update_item(item)

    def remove_item(self, item_id: str) -> None:
        """Remove an item from the display."""
        widget = self._item_widgets.pop(item_id, None)
        if widget:
            widget.remove()
        self.queue_count = len(self._item_widgets)

    def update_aggregate(
        self,
        active: int,
        total_speed: float,
        total_downloaded: int,
        total_size: int,
        combined_eta: float | None,
    ) -> None:
        """Update the aggregate progress header."""
        if active == 0:
            text = ""
        else:
            parts = []
            if total_speed > 0:
                parts.append(f"v {format_speed(total_speed)}")
            if total_downloaded > 0 and total_size > 0:
                pct = min(total_downloaded / total_size * 100, 100)
                parts.append(
                    f"{pct:.0f}% · {format_size(total_downloaded)}/{format_size(total_size)}"
                )
            if combined_eta is not None and combined_eta > 0:
                parts.append(format_eta(combined_eta))
            text = "  ".join(parts)
        try:
            self.query_one("#queue-aggregate", Static).update(text)
        except Exception:
            pass

    def clear(self) -> None:
        """Remove all items."""
        for widget in self._item_widgets.values():
            widget.remove()
        self._item_widgets.clear()
        self.queue_count = 0

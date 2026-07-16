"""Shared visual and keyboard language for PhotoSort workflow pages.

The workflow pages intentionally use the same footer shortcut registry, review
header, state banner, and button roles. This keeps presentation code out of the
individual pages and makes a shortcut's visible label and actual binding share
one source of truth.
"""

import sys
from dataclasses import dataclass
from collections.abc import Callable, Iterable, Mapping, Sequence

from PyQt6.QtCore import Qt
from PyQt6.QtGui import QKeySequence, QShortcut
from PyQt6.QtWidgets import (
    QFrame,
    QGridLayout,
    QHBoxLayout,
    QLabel,
    QPushButton,
    QSizePolicy,
    QVBoxLayout,
    QWidget,
)


@dataclass(frozen=True, slots=True)
class WorkflowShortcutSpec:
    """A shortcut definition used by both the UI legend and QShortcut."""

    action: str
    sequences: tuple[str, ...]
    keys: str
    label: str


_PRIMARY_MODIFIER = "⌘" if sys.platform == "darwin" else "Ctrl"
_ALT_MODIFIER = "⌥" if sys.platform == "darwin" else "Alt"


ORGANIZE_SHORTCUTS = (
    WorkflowShortcutSpec(
        "modes",
        ("Alt+1", "Alt+2", "Alt+3", "Alt+4", "Alt+5"),
        f"{_ALT_MODIFIER}1–5",
        "Mode",
    ),
    WorkflowShortcutSpec("rename", ("F2",), "F2", "Rename"),
    WorkflowShortcutSpec(
        "new_folder", ("Ctrl+Shift+N",), f"{_PRIMARY_MODIFIER}⇧N", "New folder"
    ),
    WorkflowShortcutSpec("toggle_delete", ("D",), "D", "Mark"),
    WorkflowShortcutSpec(
        "trash_now", ("Delete", "Backspace"), "Delete", "Trash now"
    ),
    WorkflowShortcutSpec("commit_deletions", ("Shift+D",), "Shift+D", "Commit"),
    WorkflowShortcutSpec(
        "clear_deletions", ("Alt+D",), f"{_ALT_MODIFIER}D", "Clear marks"
    ),
    WorkflowShortcutSpec(
        "apply",
        ("Ctrl+Return", "Ctrl+Enter"),
        f"{_PRIMARY_MODIFIER}↵",
        "Apply",
    ),
)

EASY_DELETE_SHORTCUTS = (
    WorkflowShortcutSpec("select_left", ("1", "Left"), "1 / ←", "Trash left"),
    WorkflowShortcutSpec("select_right", ("2", "Right"), "2 / →", "Trash right"),
    WorkflowShortcutSpec("previous", ("Up",), "↑", "Previous"),
    WorkflowShortcutSpec("next", ("Down",), "↓", "Next"),
    WorkflowShortcutSpec("confirm", ("Return", "Enter"), "Enter", "Confirm"),
    WorkflowShortcutSpec("apply_all", ("A",), "A", "All"),
    WorkflowShortcutSpec("skip", ("Escape",), "Esc", "Skip"),
)

FIX_ROTATION_SHORTCUTS = (
    WorkflowShortcutSpec("toggle", ("R", "Space"), "R / Space", "Rotate"),
    WorkflowShortcutSpec("previous", ("Left", "Up"), "←  ↑", "Previous"),
    WorkflowShortcutSpec("next", ("Right", "Down"), "→  ↓", "Next"),
    WorkflowShortcutSpec("apply", ("A",), "A", "Apply"),
    WorkflowShortcutSpec("primary", ("Return", "Enter"), "Enter", "Continue"),
    WorkflowShortcutSpec("skip", ("Escape",), "Esc", "Skip"),
)

PICK_BEST_SHORTCUTS = (
    WorkflowShortcutSpec("slots", ("1", "2", "3"), "1  2  3", "Choice"),
    WorkflowShortcutSpec("clusters", ("Left", "Right"), "←  →", "Cluster"),
    WorkflowShortcutSpec("sets", ("[", "]"), "[  ]", "Set"),
    WorkflowShortcutSpec("bulk", ("K", "X"), "K / X", "Bulk"),
    WorkflowShortcutSpec("focus", ("C",), "C", "Compare"),
    WorkflowShortcutSpec("info", ("I",), "I", "Details"),
    WorkflowShortcutSpec("continue", ("Return", "Enter"), "Enter", "Continue"),
    WorkflowShortcutSpec("skip", ("Escape",), "Esc", "Skip"),
)

CULL_SHORTCUTS = (
    WorkflowShortcutSpec("focus", tuple(str(i) for i in range(1, 10)), "1–9", "Focus"),
    WorkflowShortcutSpec(
        "browse", ("Left", "Right", "Up", "Down"), "← ↑ → ↓", "Browse"
    ),
    WorkflowShortcutSpec("mark", ("D",), "D", "Mark"),
    WorkflowShortcutSpec("commit", ("Shift+D",), "Shift+D", "Commit"),
    WorkflowShortcutSpec("rotate", ("R",), "R", "Rotate"),
    WorkflowShortcutSpec("details", ("I",), "I", "Details"),
    WorkflowShortcutSpec("fit", ("0",), "0", "Fit"),
    WorkflowShortcutSpec("views", ("F1", "F2"), "F1 / F2", "View"),
)

WORKFLOW_SHORTCUTS = {
    "organize": ORGANIZE_SHORTCUTS,
    "easy_delete": EASY_DELETE_SHORTCUTS,
    "fix_rotation": FIX_ROTATION_SHORTCUTS,
    "pick_best": PICK_BEST_SHORTCUTS,
    "cull": CULL_SHORTCUTS,
}


def _refresh_style(widget: QWidget) -> None:
    style = widget.style()
    style.unpolish(widget)
    style.polish(widget)
    widget.update()


class WorkflowShortcutStrip(QFrame):
    """Compact, consistent key legend used on every workflow page."""

    def __init__(
        self,
        shortcuts: Sequence[WorkflowShortcutSpec],
        max_columns: int | None = None,
        parent: QWidget | None = None,
    ) -> None:
        super().__init__(parent)
        self.setObjectName("workflowShortcutStrip")
        self.setMinimumWidth(0)
        self.setSizePolicy(
            QSizePolicy.Policy.Ignored, QSizePolicy.Policy.Preferred
        )
        layout = QHBoxLayout(self)
        layout.setContentsMargins(8, 4, 8, 4)
        layout.setSpacing(6)

        heading = QLabel("SHORTCUTS")
        heading.setObjectName("workflowShortcutHeading")
        self._heading = heading
        layout.addWidget(heading)

        items = QWidget(self)
        items_layout = QGridLayout(items)
        items_layout.setContentsMargins(0, 0, 0, 0)
        items_layout.setHorizontalSpacing(8)
        items_layout.setVerticalSpacing(3)
        columns = max_columns or max(1, len(shortcuts))
        self._column_limit = columns
        self._current_columns = columns
        self._items_layout = items_layout
        self.shortcut_specs = tuple(shortcuts)
        self._items: list[QWidget] = []
        self._keycaps: list[QLabel] = []
        self._action_labels: list[QLabel] = []

        for index, spec in enumerate(shortcuts):
            item = QWidget(self)
            item.setObjectName("workflowShortcutItem")
            item_layout = QHBoxLayout(item)
            item_layout.setContentsMargins(0, 0, 0, 0)
            item_layout.setSpacing(4)

            keycap = QLabel(spec.keys)
            keycap.setObjectName("workflowKeycap")
            keycap.setAlignment(Qt.AlignmentFlag.AlignCenter)
            keycap.setToolTip(f"{spec.keys} — {spec.label}")
            action = QLabel(spec.label)
            action.setObjectName("workflowShortcutLabel")
            action.setToolTip(f"{spec.keys} — {spec.label}")
            self._items.append(item)
            self._keycaps.append(keycap)
            self._action_labels.append(action)

            item_layout.addWidget(keycap)
            item_layout.addWidget(action)
            items_layout.addWidget(item, index // columns, index % columns)

        layout.addWidget(items)
        layout.addStretch(1)

    def resizeEvent(self, event) -> None:
        super().resizeEvent(event)
        show_labels = self.width() >= 520
        for label in self._action_labels:
            label.setVisible(show_labels)
        self._reflow_shortcuts()

    def _reflow_shortcuts(self) -> None:
        if not self._items:
            return
        margins = self.layout().contentsMargins()
        available_width = max(
            1,
            self.width()
            - margins.left()
            - margins.right()
            - self._heading.sizeHint().width()
            - self.layout().spacing(),
        )
        spacing = self._items_layout.horizontalSpacing()
        item_widths = [item.sizeHint().width() for item in self._items]
        selected_columns = 1
        for columns in range(self._column_limit, 0, -1):
            row_widths = [
                sum(item_widths[start : start + columns])
                + spacing * (min(columns, len(item_widths) - start) - 1)
                for start in range(0, len(item_widths), columns)
            ]
            if max(row_widths, default=0) <= available_width:
                selected_columns = columns
                break
        if selected_columns == self._current_columns:
            return
        self._current_columns = selected_columns
        for item in self._items:
            self._items_layout.removeWidget(item)
        for index, item in enumerate(self._items):
            self._items_layout.addWidget(
                item, index // selected_columns, index % selected_columns
            )


class WorkflowReviewHeader(QFrame):
    """Shared header for analysis/review steps two through four."""

    def __init__(
        self,
        *,
        step_number: int,
        title: str,
        description: str,
        parent: QWidget | None = None,
    ) -> None:
        super().__init__(parent)
        self.setObjectName("workflowReviewHeader")

        root = QVBoxLayout(self)
        root.setContentsMargins(14, 10, 14, 8)
        root.setSpacing(6)

        top = QHBoxLayout()
        top.setContentsMargins(0, 0, 0, 0)
        top.setSpacing(10)

        step = QLabel(f"STEP {step_number} OF 5")
        step.setObjectName("workflowStepPill")
        title_label = QLabel(title)
        title_label.setObjectName("workflowReviewTitle")
        self.summary_label = QLabel()
        self.summary_label.setObjectName("workflowReviewSummary")
        self.summary_label.setAlignment(
            Qt.AlignmentFlag.AlignRight | Qt.AlignmentFlag.AlignVCenter
        )
        self.skip_button = QPushButton("Skip step")
        self.skip_button.setObjectName("workflowGhostButton")

        top.addWidget(step)
        top.addWidget(title_label)
        top.addStretch(1)
        top.addWidget(self.summary_label)
        top.addWidget(self.skip_button)
        root.addLayout(top)

        lower = QHBoxLayout()
        lower.setContentsMargins(0, 0, 0, 0)
        lower.setSpacing(12)
        description_label = QLabel(description)
        description_label.setObjectName("workflowReviewDescription")
        description_label.setWordWrap(True)
        lower.addWidget(description_label, 1)
        root.addLayout(lower)

    def set_summary(self, text: str, tone: str = "neutral") -> None:
        self.summary_label.setText(text)
        self.summary_label.setProperty("tone", tone)
        _refresh_style(self.summary_label)


class WorkflowStateBanner(QFrame):
    """Makes the difference between staged and applied state unmistakable."""

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("workflowStateBanner")
        layout = QHBoxLayout(self)
        layout.setContentsMargins(10, 7, 10, 7)
        layout.setSpacing(8)

        self.icon_label = QLabel("○")
        self.icon_label.setObjectName("workflowStateIcon")
        self.title_label = QLabel()
        self.title_label.setObjectName("workflowStateTitle")
        self.detail_label = QLabel()
        self.detail_label.setObjectName("workflowStateDetail")
        self.detail_label.setWordWrap(True)

        layout.addWidget(self.icon_label)
        layout.addWidget(self.title_label)
        layout.addWidget(self.detail_label, 1)

        self.set_state(
            "Review mode",
            "No files have been changed.",
            tone="neutral",
        )

    def set_state(self, title: str, detail: str, *, tone: str = "neutral") -> None:
        icons = {
            "neutral": "○",
            "info": "i",
            "warning": "!",
            "danger": "×",
            "success": "✓",
        }
        self.setProperty("tone", tone)
        self.icon_label.setText(icons.get(tone, "○"))
        self.title_label.setText(title)
        self.detail_label.setText(detail)
        _refresh_style(self)


def install_workflow_shortcuts(
    owner: QWidget,
    specs: Iterable[WorkflowShortcutSpec],
    handlers: Mapping[str, Callable[[], None]],
) -> list[QShortcut]:
    """Install bindings from the same specs rendered by the shortcut strip."""

    installed: list[QShortcut] = []
    for spec in specs:
        for sequence in spec.sequences:
            handler = handlers.get(f"{spec.action}:{sequence}") or handlers.get(
                spec.action
            )
            if handler is None:
                continue
            shortcut = QShortcut(QKeySequence(sequence), owner)
            shortcut.setContext(Qt.ShortcutContext.WidgetWithChildrenShortcut)
            shortcut.activated.connect(handler)
            installed.append(shortcut)
    return installed

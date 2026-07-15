"""Shared visual and keyboard language for PhotoSort workflow pages.

The workflow pages intentionally use the same header, shortcut registry, state
banner, and button roles.  This keeps presentation code out of the individual
pages and makes a shortcut's visible label and actual binding share one source
of truth.
"""

import sys
from dataclasses import dataclass
from collections.abc import Callable, Iterable, Mapping, Sequence

from PyQt6.QtCore import Qt
from PyQt6.QtGui import QKeySequence, QShortcut
from PyQt6.QtWidgets import (
    QFrame,
    QHBoxLayout,
    QLabel,
    QPushButton,
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
        "Grouping mode",
    ),
    WorkflowShortcutSpec("rename", ("F2",), "F2", "Rename"),
    WorkflowShortcutSpec(
        "new_folder", ("Ctrl+Shift+N",), f"{_PRIMARY_MODIFIER}⇧N", "New folder"
    ),
    WorkflowShortcutSpec("toggle_delete", ("D",), "D", "Mark for Trash"),
    WorkflowShortcutSpec(
        "trash_now", ("Delete", "Backspace"), "Delete", "Move to Trash now"
    ),
    WorkflowShortcutSpec(
        "commit_deletions", ("Shift+D",), "Shift+D", "Commit marked"
    ),
    WorkflowShortcutSpec(
        "clear_deletions", ("Alt+D",), f"{_ALT_MODIFIER}D", "Clear marks"
    ),
    WorkflowShortcutSpec(
        "apply",
        ("Ctrl+Return", "Ctrl+Enter"),
        f"{_PRIMARY_MODIFIER}↵",
        "Review changes",
    ),
)

EASY_DELETE_SHORTCUTS = (
    WorkflowShortcutSpec("toggle", ("X", "Delete"), "X / Del", "Keep / Trash"),
    WorkflowShortcutSpec("previous", ("Left", "Up"), "←  ↑", "Previous"),
    WorkflowShortcutSpec("next", ("Right", "Down"), "→  ↓", "Next"),
    WorkflowShortcutSpec("continue", ("Return", "Enter"), "Enter", "Continue"),
    WorkflowShortcutSpec("skip", ("Escape",), "Esc", "Skip step"),
)

FIX_ROTATION_SHORTCUTS = (
    WorkflowShortcutSpec("toggle", ("R", "Space"), "R / Space", "Rotate / Skip"),
    WorkflowShortcutSpec("previous", ("Left", "Up"), "←  ↑", "Previous"),
    WorkflowShortcutSpec("next", ("Right", "Down"), "→  ↓", "Next"),
    WorkflowShortcutSpec("apply", ("A",), "A", "Apply now"),
    WorkflowShortcutSpec("primary", ("Return", "Enter"), "Enter", "Apply / continue"),
    WorkflowShortcutSpec("skip", ("Escape",), "Esc", "Continue without"),
)

PICK_BEST_SHORTCUTS = (
    WorkflowShortcutSpec("slots", ("1", "2", "3"), "1  2  3", "Set choice"),
    WorkflowShortcutSpec("clusters", ("Left", "Right"), "←  →", "Clusters"),
    WorkflowShortcutSpec("sets", ("[", "]"), "[  ]", "Sets"),
    WorkflowShortcutSpec("bulk", ("K", "X"), "K / X", "Keep / Trash visible"),
    WorkflowShortcutSpec("focus", ("C",), "C", "Compare / Focus"),
    WorkflowShortcutSpec("info", ("I",), "I", "Details"),
    WorkflowShortcutSpec("continue", ("Return", "Enter"), "Enter", "Continue"),
    WorkflowShortcutSpec("skip", ("Escape",), "Esc", "Skip step"),
)


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
        parent: QWidget | None = None,
    ) -> None:
        super().__init__(parent)
        self.setObjectName("workflowShortcutStrip")
        layout = QHBoxLayout(self)
        layout.setContentsMargins(8, 4, 8, 4)
        layout.setSpacing(8)

        heading = QLabel("SHORTCUTS")
        heading.setObjectName("workflowShortcutHeading")
        layout.addWidget(heading)

        for spec in shortcuts:
            item = QWidget(self)
            item.setObjectName("workflowShortcutItem")
            item_layout = QHBoxLayout(item)
            item_layout.setContentsMargins(0, 0, 0, 0)
            item_layout.setSpacing(4)

            keycap = QLabel(spec.keys)
            keycap.setObjectName("workflowKeycap")
            keycap.setAlignment(Qt.AlignmentFlag.AlignCenter)
            action = QLabel(spec.label)
            action.setObjectName("workflowShortcutLabel")

            item_layout.addWidget(keycap)
            item_layout.addWidget(action)
            layout.addWidget(item)

        layout.addStretch(1)


class WorkflowReviewHeader(QFrame):
    """Shared header for analysis/review steps two through four."""

    def __init__(
        self,
        *,
        step_number: int,
        title: str,
        description: str,
        shortcuts: Sequence[WorkflowShortcutSpec],
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
        lower.addWidget(WorkflowShortcutStrip(shortcuts, self), 0)
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

"""Shared visual and keyboard language for PhotoSort workflow pages.

The workflow pages intentionally use the same footer shortcut registry, state
banner, and button roles. This keeps presentation code out of the individual
pages and makes a shortcut's visible label and actual binding share one source
of truth.
"""

import math
import sys
from dataclasses import dataclass
from collections.abc import Callable, Iterable, Mapping, Sequence

from PyQt6.QtCore import Qt, pyqtSignal
from PyQt6.QtGui import QKeySequence, QShortcut
from PyQt6.QtWidgets import (
    QFrame,
    QGridLayout,
    QHBoxLayout,
    QLabel,
    QListWidget,
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

_TOGGLE_LEFT_PANEL_SHORTCUT = WorkflowShortcutSpec(
    "toggle_left_panel",
    ("Ctrl+Shift+L",),
    f"{_PRIMARY_MODIFIER}⇧L",
    "Left panel",
)
_WORKFLOW_STEP_SHORTCUT = WorkflowShortcutSpec(
    "workflow_steps",
    tuple(f"Ctrl+Alt+{index}" for index in range(1, 6)),
    f"{_PRIMARY_MODIFIER}{_ALT_MODIFIER}1–5",
    "Step",
)


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
    WorkflowShortcutSpec("trash_now", ("Delete", "Backspace"), "Delete", "Trash now"),
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
    _TOGGLE_LEFT_PANEL_SHORTCUT,
    _WORKFLOW_STEP_SHORTCUT,
)

EASY_DELETE_SHORTCUTS = (
    WorkflowShortcutSpec("select_left", ("1", "Left"), "1 / ←", "Trash left"),
    WorkflowShortcutSpec("select_right", ("2", "Right"), "2 / →", "Trash right"),
    WorkflowShortcutSpec("previous", ("Up",), "↑", "Previous"),
    WorkflowShortcutSpec("next", ("Down",), "↓", "Next"),
    WorkflowShortcutSpec("confirm", ("Return", "Enter"), "Enter", "Confirm / cancel"),
    WorkflowShortcutSpec(
        "apply", ("Shift+Return", "Shift+Enter"), "Shift+Enter", "Apply"
    ),
    WorkflowShortcutSpec("apply_all", ("A",), "A", "All"),
    WorkflowShortcutSpec("info", ("I",), "I", "Details"),
    _TOGGLE_LEFT_PANEL_SHORTCUT,
    _WORKFLOW_STEP_SHORTCUT,
)

FIX_ROTATION_SHORTCUTS = (
    WorkflowShortcutSpec("rotate_counterclockwise", ("R",), "R", "−90° override"),
    WorkflowShortcutSpec("rotate_clockwise", ("Shift+R",), "Shift+R", "+90° override"),
    WorkflowShortcutSpec("previous", ("Left", "Up"), "←  ↑", "Previous"),
    WorkflowShortcutSpec("next", ("Right", "Down"), "→  ↓", "Next"),
    WorkflowShortcutSpec("primary", ("Return", "Enter"), "Enter", "Confirm"),
    WorkflowShortcutSpec("skip", ("Escape",), "Esc", "Skip"),
    _TOGGLE_LEFT_PANEL_SHORTCUT,
    _WORKFLOW_STEP_SHORTCUT,
)

PICK_BEST_SHORTCUTS = (
    WorkflowShortcutSpec("slots", ("1", "2", "3"), "1  2  3", "Choice"),
    WorkflowShortcutSpec("clusters", ("Left", "Right"), "←  →", "Cluster"),
    WorkflowShortcutSpec("groups", ("Up", "Down"), "↑  ↓", "Comparison / cluster"),
    WorkflowShortcutSpec("focus", ("C",), "C", "Compare"),
    WorkflowShortcutSpec("info", ("I",), "I", "Details"),
    WorkflowShortcutSpec("keep_all", ("K",), "K", "Keep all"),
    WorkflowShortcutSpec("confirm", ("Return", "Enter"), "Enter", "Confirm"),
    WorkflowShortcutSpec("skip", ("Escape",), "Esc", "Skip"),
    _TOGGLE_LEFT_PANEL_SHORTCUT,
    _WORKFLOW_STEP_SHORTCUT,
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
    _TOGGLE_LEFT_PANEL_SHORTCUT,
    _WORKFLOW_STEP_SHORTCUT,
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


class WorkflowDecisionCard(QFrame):
    """Shared keep/trash card shown separately from workflow image previews."""

    activated = pyqtSignal()

    def __init__(
        self,
        slot_number: int,
        parent: QWidget | None = None,
        *,
        filename_in_header: bool = False,
    ) -> None:
        super().__init__(parent)
        self._slot_number = slot_number
        self.setObjectName("workflowCompareCard")
        self.setFrameShape(QFrame.Shape.StyledPanel)
        self.setSizePolicy(QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Preferred)
        self.setCursor(Qt.CursorShape.PointingHandCursor)
        self._focused = False
        self._border_color = "#3A4654"
        self._background = "#20252C"

        self._content_layout = QVBoxLayout(self)
        self._content_layout.setContentsMargins(12, 10, 12, 10)
        self._content_layout.setSpacing(8)

        top_row = QHBoxLayout()
        top_row.setContentsMargins(0, 0, 0, 0)
        top_row.setSpacing(8)

        self._slot_label = QLabel(str(slot_number))
        self._slot_label.setObjectName("workflowCompareSlot")
        self._slot_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self._slot_label.setFixedSize(22, 22)
        top_row.addWidget(self._slot_label, alignment=Qt.AlignmentFlag.AlignLeft)

        self._name_label = QLabel()
        self._name_label.setObjectName("workflowCompareName")
        self._name_label.setWordWrap(False)
        if filename_in_header:
            top_row.addWidget(self._name_label, stretch=1)

        self._state_label = QLabel()
        self._state_label.setObjectName("workflowCompareState")
        self._state_label.setAlignment(
            Qt.AlignmentFlag.AlignRight | Qt.AlignmentFlag.AlignVCenter
        )
        top_row.addWidget(self._state_label, stretch=0 if filename_in_header else 1)
        self._content_layout.addLayout(top_row)

        if not filename_in_header:
            self._content_layout.addWidget(self._name_label)

        self._details_grid = QGridLayout()
        self._details_grid.setContentsMargins(0, 0, 0, 0)
        self._details_grid.setHorizontalSpacing(10)
        self._details_grid.setVerticalSpacing(4)
        self._details_grid.setColumnStretch(1, 1)
        self._content_layout.addLayout(self._details_grid)

        self._details_visible = True
        self._detail_rows: list[tuple[QLabel, QLabel]] = []
        for row in range(6):
            key = QLabel()
            key.setObjectName("workflowCompareMetaKey")
            value = QLabel()
            value.setObjectName("workflowCompareMetaValue")
            value.setWordWrap(True)
            value.setTextInteractionFlags(Qt.TextInteractionFlag.TextSelectableByMouse)
            self._details_grid.addWidget(key, row, 0)
            self._details_grid.addWidget(value, row, 1)
            self._detail_rows.append((key, value))

        self._hint_label = QLabel()
        self._hint_label.setObjectName("workflowCompareHint")
        self._content_layout.addWidget(self._hint_label)

    @property
    def state_label(self) -> QLabel:
        return self._state_label

    def set_details(self, details: Sequence[tuple[str, str]]) -> None:
        for index, (key_label, value_label) in enumerate(self._detail_rows):
            has_value = index < len(details)
            if has_value:
                key, value = details[index]
                key_label.setText(key)
                value_label.setText(value)
                value_label.setToolTip(value)
            key_label.setVisible(self._details_visible and has_value)
            value_label.setVisible(self._details_visible and has_value)

    def set_details_visible(self, visible: bool) -> None:
        self._details_visible = visible
        for key_label, value_label in self._detail_rows:
            has_value = bool(key_label.text() or value_label.text())
            key_label.setVisible(visible and has_value)
            value_label.setVisible(visible and has_value)

    def set_decision(
        self,
        *,
        filename: str,
        state: str,
        state_color: str,
        border_color: str,
        hint: str,
        background: str = "#20252C",
    ) -> None:
        self._name_label.setText(filename)
        self._state_label.setText(state)
        self._state_label.setStyleSheet(
            f"font-size: 11px; font-weight: bold; color: {state_color};"
        )
        self._hint_label.setText(hint)
        self._border_color = border_color
        self._background = background
        self._apply_card_style()

    def set_focused(self, focused: bool) -> None:
        if self._focused == focused:
            return
        self._focused = focused
        self._apply_card_style()

    def _apply_card_style(self) -> None:
        border_width = 4 if self._focused else 2
        self.setStyleSheet(
            f"WorkflowDecisionCard, QFrame#workflowCompareCard {{"
            f"border: {border_width}px solid {self._border_color};"
            "border-radius: 10px;"
            f"background: {self._background};"
            "}"
        )

    def mousePressEvent(self, event) -> None:
        if event.button() == Qt.MouseButton.LeftButton:
            self.activated.emit()
            event.accept()
            return
        super().mousePressEvent(event)


class WorkflowShortcutStrip(QFrame):
    """Compact, consistent key legend used on every workflow page."""

    def __init__(
        self,
        shortcuts: Sequence[WorkflowShortcutSpec],
        max_rows: int = 3,
        parent: QWidget | None = None,
    ) -> None:
        super().__init__(parent)
        self.setObjectName("workflowShortcutStrip")
        self.setMinimumWidth(0)
        self.setSizePolicy(QSizePolicy.Policy.Ignored, QSizePolicy.Policy.Preferred)
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
        columns = max(1, len(shortcuts))
        self._column_limit = columns
        self._minimum_columns = max(1, math.ceil(len(shortcuts) / max(1, max_rows)))
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
        selected_columns = max(selected_columns, self._minimum_columns)
        if selected_columns == self._current_columns:
            return
        self._current_columns = selected_columns
        for item in self._items:
            self._items_layout.removeWidget(item)
        for index, item in enumerate(self._items):
            self._items_layout.addWidget(
                item, index // selected_columns, index % selected_columns
            )


class WorkflowReviewListPanel(QFrame):
    """Compact shared queue used by the review workflows."""

    def __init__(
        self,
        *,
        bulk_action_text: str | None,
        title_text: str = "Review queue",
        count_noun: str = "item",
        parent: QWidget | None = None,
    ) -> None:
        super().__init__(parent)
        self.setObjectName("workflowReviewListPanel")
        self.setFrameShape(QFrame.Shape.NoFrame)
        self.setMinimumWidth(220)
        self.setMaximumWidth(310)

        root = QVBoxLayout(self)
        root.setContentsMargins(10, 10, 10, 10)
        root.setSpacing(5)

        header = QWidget()
        header.setObjectName("workflowReviewListHeader")
        header_layout = QHBoxLayout(header)
        header_layout.setContentsMargins(4, 0, 4, 2)
        header_layout.setSpacing(6)

        self._count_noun = count_noun
        title = QLabel(title_text)
        title.setObjectName("workflowReviewListTitle")
        self.count_label = QLabel("0 items")
        self.count_label.setObjectName("workflowReviewListCount")
        self.count_label.setAlignment(
            Qt.AlignmentFlag.AlignRight | Qt.AlignmentFlag.AlignVCenter
        )
        header_layout.addWidget(title)
        header_layout.addStretch(1)
        header_layout.addWidget(self.count_label)
        root.addWidget(header)

        self.filters = QWidget()
        self.filters.setObjectName("workflowReviewListFilters")
        self.filters_layout = QGridLayout(self.filters)
        self.filters_layout.setContentsMargins(4, 2, 4, 3)
        self.filters_layout.setHorizontalSpacing(8)
        self.filters_layout.setVerticalSpacing(2)
        self.filters.hide()
        root.addWidget(self.filters)

        self.list_widget = QListWidget()
        self.list_widget.setObjectName("workflowReviewList")
        self.list_widget.setFocusPolicy(Qt.FocusPolicy.NoFocus)
        root.addWidget(self.list_widget, 1)

        self.footer = QWidget()
        self.footer.setObjectName("workflowReviewListFooter")
        footer_layout = QHBoxLayout(self.footer)
        footer_layout.setContentsMargins(0, 2, 0, 0)
        self.bulk_button = QPushButton(bulk_action_text or "")
        self.bulk_button.setObjectName("workflowGhostButton")
        footer_layout.addWidget(self.bulk_button)
        self.footer.setVisible(bulk_action_text is not None)
        root.addWidget(self.footer)

    def set_count(self, visible: int, total: int | None = None) -> None:
        if total is not None and visible != total:
            text = f"{visible} of {total}"
        else:
            text = f"{visible} {self._count_noun}{'s' if visible != 1 else ''}"
        self.count_label.setText(text)


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
    """Install bindings that remain active while their workflow page is visible."""

    installed: list[QShortcut] = []
    for spec in specs:
        for sequence in spec.sequences:
            handler = handlers.get(f"{spec.action}:{sequence}") or handlers.get(
                spec.action
            )
            if handler is None:
                continue
            shortcut = QShortcut(QKeySequence(sequence), owner)
            shortcut.setContext(Qt.ShortcutContext.WindowShortcut)
            shortcut.activated.connect(handler)
            installed.append(shortcut)
    return installed

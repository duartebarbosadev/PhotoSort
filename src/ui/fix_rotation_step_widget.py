from __future__ import annotations

import logging
import os
from typing import Dict, List, Optional

from PyQt6.QtCore import Qt, QTimer, pyqtSignal
from PyQt6.QtGui import QColor, QPixmap, QTransform
from PyQt6.QtWidgets import (
    QButtonGroup,
    QHBoxLayout,
    QLabel,
    QListWidget,
    QListWidgetItem,
    QProgressBar,
    QPushButton,
    QSizePolicy,
    QSplitter,
    QStackedWidget,
    QVBoxLayout,
    QWidget,
)

from ui.workflow_review_components import (
    FIX_ROTATION_SHORTCUTS,
    WorkflowReviewHeader,
    WorkflowStateBanner,
    install_workflow_shortcuts,
)

logger = logging.getLogger(__name__)

_ANGLE_LABELS: Dict[int, tuple] = {
    90: ("90° CW", "#00D4FF"),
    180: ("180°", "#F5B700"),
    -90: ("90° CCW", "#00D4FF"),
}

_UNMARKED_COLOR = "#00D4FF"
_MARKED_COLOR = "#66BB6A"
_SKIP_COLOR = "#607080"


class _RotatedImageLabel(QLabel):
    """QLabel that displays an image and an optional rotation-preview overlay."""

    def __init__(self, parent: Optional[QWidget] = None) -> None:
        super().__init__(parent)
        self._source_pixmap: Optional[QPixmap] = None
        self._preview_angle: int = 0
        self.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.setMinimumSize(80, 80)
        self.setSizePolicy(QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Expanding)
        self.setStyleSheet("background: #0D1117;")

    def set_pixmap_and_angle(
        self, pixmap: Optional[QPixmap], preview_angle: int = 0
    ) -> None:
        self._source_pixmap = pixmap
        self._preview_angle = preview_angle
        self._refresh()

    def resizeEvent(self, event) -> None:
        super().resizeEvent(event)
        self._refresh()

    def _refresh(self) -> None:
        if not self._source_pixmap or self._source_pixmap.isNull():
            super().setPixmap(QPixmap())
            return

        px = self._source_pixmap
        if self._preview_angle != 0:
            transform = QTransform().rotate(self._preview_angle)
            px = px.transformed(transform, Qt.TransformationMode.SmoothTransformation)

        scaled = px.scaled(
            self.size(),
            Qt.AspectRatioMode.KeepAspectRatio,
            Qt.TransformationMode.SmoothTransformation,
        )
        super().setPixmap(scaled)


class FixRotationStepWidget(QWidget):
    """Step 3: Detect and fix wrongly-rotated images before culling."""

    apply_rotations_requested = pyqtSignal(dict)  # {path: angle_degrees}
    proceed_requested = pyqtSignal()
    skip_requested = pyqtSignal()

    def __init__(self, parent: Optional[QWidget] = None) -> None:
        super().__init__(parent)
        self._suggestions: Dict[str, int] = {}  # path -> suggested angle
        self._marked: Dict[str, bool] = {}  # path -> True if marked for rotation
        self._ordered_paths: List[str] = []
        self._current_index: int = -1
        self._image_pipeline = None
        self._applying = False
        self._submitted_paths: set[str] = set()
        self._successful_paths: set[str] = set()
        self._failed_paths: set[str] = set()
        self.setFocusPolicy(Qt.FocusPolicy.ClickFocus)
        self._setup_ui()
        self._shortcuts = install_workflow_shortcuts(
            self,
            FIX_ROTATION_SHORTCUTS,
            {
                "toggle": self._on_mark_toggle,
                "previous": self._on_prev,
                "next": self._on_next,
                "apply": self._on_apply,
                "primary": self._on_primary_action,
                "skip": self._on_skip,
            },
        )

    def set_image_pipeline(self, pipeline) -> None:
        self._image_pipeline = pipeline

    # ------------------------------------------------------------------
    # Public state-machine API
    # ------------------------------------------------------------------

    def show_loading(self, message: str = "", percent: int = -1) -> None:
        self._loading_label.setText(message or "Analyzing rotation…")
        if percent < 0:
            self._progress_bar.setRange(0, 0)
        else:
            self._progress_bar.setRange(0, 100)
            self._progress_bar.setValue(percent)
        self._content_stack.setCurrentIndex(0)

    def show_error(self, message: str) -> None:
        self._loading_label.setText(f"Error: {message}")
        self._progress_bar.setRange(0, 100)
        self._progress_bar.setValue(0)
        self._content_stack.setCurrentIndex(0)

    def show_model_not_found(self, message: str) -> None:
        self._loading_label.setText(
            "Rotation model not found — skip this step or install the model.\n\n"
            + message
        )
        self._progress_bar.setRange(0, 100)
        self._progress_bar.setValue(0)
        self._content_stack.setCurrentIndex(0)

    def show_results(self, suggestions: Dict[str, int]) -> None:
        self._suggestions = dict(suggestions)
        # Pre-mark all suggestions for rotation
        self._marked = dict.fromkeys(suggestions, True)
        self._ordered_paths = sorted(suggestions.keys(), key=os.path.basename)
        self._current_index = -1
        self._applying = False
        self._submitted_paths.clear()
        self._successful_paths.clear()
        self._failed_paths.clear()

        if self._ordered_paths:
            self._populate_list()
            self._content_stack.setCurrentIndex(1)
            self._navigate_to(0)
            self.setFocus(Qt.FocusReason.OtherFocusReason)
        else:
            self._configure_empty_state(
                "All photos are correctly oriented",
                "No rotation corrections are needed.",
            )
            self._content_stack.setCurrentIndex(2)

    def show_applying(self, current: int, total: int, filename: str) -> None:
        self._applying = True
        self._apply_btn.setEnabled(False)
        self._apply_btn.setText(f"Applying… ({current}/{total})")
        if total > 0:
            self._progress_bar.setRange(0, total)
            self._progress_bar.setValue(current)
        self._status_label.setText(f"Rotating {filename}…")
        self._state_banner.set_state(
            "Changing files now",
            f"Applying rotation {current} of {total}. Please keep PhotoSort open.",
            tone="warning",
        )

    def record_apply_result(self, path: str, success: bool) -> None:
        """Record per-file worker results so completed rows cannot be re-applied."""

        if success:
            self._successful_paths.add(path)
            self._failed_paths.discard(path)
        else:
            self._failed_paths.add(path)

    def show_apply_complete(self, successful: int, failed: int) -> None:
        self._applying = False
        total = successful + failed
        if successful and not self._successful_paths and failed == 0:
            self._successful_paths.update(self._submitted_paths)

        completed = self._successful_paths & set(self._ordered_paths)
        if completed:
            for path in completed:
                self._suggestions.pop(path, None)
                self._marked.pop(path, None)
            self._ordered_paths = [
                path for path in self._ordered_paths if path not in completed
            ]

        self._submitted_paths.clear()
        self._successful_paths.clear()

        if not self._ordered_paths:
            self._configure_empty_state(
                "Rotations applied",
                f"{successful} photo{'s' if successful != 1 else ''} updated successfully.",
            )
            self._content_stack.setCurrentIndex(2)
            return

        self._populate_list()
        self._navigate_to(min(self._current_index, len(self._ordered_paths) - 1))
        if failed:
            self._status_label.setText(
                f"{successful} applied · {failed} failed — review and retry"
            )
            self._state_banner.set_state(
                "Some rotations failed",
                "Failed photos remain queued. Review them and try Apply again.",
                tone="danger",
            )
        else:
            self._status_label.setText(f"Applied {successful}/{total} rotations")
        self._failed_paths.clear()
        self._refresh_apply_button()

    def _configure_empty_state(self, title: str, subtitle: str) -> None:
        self._empty_title.setText(title)
        self._empty_subtitle.setText(subtitle)

    # ------------------------------------------------------------------
    # Private helpers
    # ------------------------------------------------------------------

    def _populate_list(self) -> None:
        self._items_list.clear()
        for path in self._ordered_paths:
            angle = self._suggestions.get(path, 0)
            badge, _ = _ANGLE_LABELS.get(angle, (f"{angle}°", "#888"))
            state = "QUEUED" if self._marked.get(path, False) else "SKIP"
            item_text = f"{state}  ·  [{badge}]  {os.path.basename(path)}"
            item = QListWidgetItem(item_text)
            item.setData(Qt.ItemDataRole.UserRole, path)
            self._items_list.addItem(item)

        self._update_summary()
        self._refresh_list_colors()

    def _update_summary(self) -> None:
        total = len(self._ordered_paths)
        marked = sum(1 for v in self._marked.values() if v)
        self._summary_label.setText(
            f"{total} suggestion{'s' if total != 1 else ''} · {marked} queued"
        )
        self._review_header.set_summary(
            f"{marked} queued  ·  {total - marked} leave as-is",
            "warning" if marked else "neutral",
        )

    def _refresh_list_colors(self) -> None:
        for i in range(self._items_list.count()):
            item = self._items_list.item(i)
            path = item.data(Qt.ItemDataRole.UserRole)
            is_marked = self._marked.get(path, False)
            color = _MARKED_COLOR if is_marked else _SKIP_COLOR
            item.setForeground(QColor(color))
            angle = self._suggestions.get(path, 0)
            badge, _ = _ANGLE_LABELS.get(angle, (f"{angle}°", "#888"))
            state = "QUEUED" if is_marked else "SKIP"
            item.setText(f"{state}  ·  [{badge}]  {os.path.basename(path)}")

    def _navigate_to(self, index: int) -> None:
        if not self._ordered_paths:
            return
        index = max(0, min(index, len(self._ordered_paths) - 1))
        self._current_index = index

        self._items_list.blockSignals(True)
        self._items_list.setCurrentRow(index)
        self._items_list.blockSignals(False)

        self._show_current()
        self._refresh_controls()

    def _show_current(self) -> None:
        if self._current_index < 0 or self._current_index >= len(self._ordered_paths):
            return
        path = self._ordered_paths[self._current_index]
        angle = self._suggestions.get(path, 0)
        is_marked = self._marked.get(path, False)

        badge, color = _ANGLE_LABELS.get(angle, (f"{angle}°", "#888888"))

        pixmap = self._load_pixmap(path)
        if pixmap is None:
            request = getattr(self.window(), "request_interactive_previews", None)
            if callable(request):
                request([path])

        # Left panel: current (as-is) — no rotation applied
        self._current_img.set_pixmap_and_angle(pixmap, 0)
        self._current_hdr.setText("ORIGINAL · unchanged")

        # Right panel: preview after suggested rotation
        preview_angle = angle if is_marked else 0
        self._preview_img.set_pixmap_and_angle(pixmap, preview_angle)
        if is_marked:
            self._preview_hdr.setText(f"ROTATED PREVIEW · {badge}")
            self._state_banner.set_state(
                f"Queued: rotate {badge}",
                f"{os.path.basename(path)} is only previewed. Press Apply to change the file.",
                tone="warning",
            )
        else:
            self._preview_hdr.setText("LEAVE AS-IS · no change")
            self._state_banner.set_state(
                "Leaving this photo as-is",
                f"{os.path.basename(path)} is not queued and will not be changed.",
                tone="neutral",
            )

        self._angle_label.setText(
            f"<b style='color:{color}'>[{badge}]</b>  Suggested rotation: <b>{badge}</b>"
        )

    def _load_pixmap(self, path: str) -> Optional[QPixmap]:
        try:
            if self._image_pipeline:
                pixmap = self._image_pipeline.get_cached_analysis_qpixmap(
                    path,
                    memory_only=True,
                )
                if pixmap is None:
                    pixmap = self._image_pipeline.get_cached_preview_qpixmap(
                        path,
                        memory_only=True,
                    )
                if pixmap is None:
                    pixmap = self._image_pipeline.get_cached_thumbnail_qpixmap(
                        path,
                        memory_only=True,
                    )
                if pixmap is not None and not pixmap.isNull():
                    return pixmap
        except Exception as exc:
            logger.debug(
                "FixRotation: pixmap load failed for %s: %s",
                os.path.basename(path),
                exc,
            )
        return None

    def handle_preview_ready(self, path: str) -> None:
        if not (0 <= self._current_index < len(self._ordered_paths)):
            return
        if self._ordered_paths[self._current_index] == path:
            self._show_current()

    def _refresh_controls(self) -> None:
        total = len(self._ordered_paths)
        if total == 0:
            return
        self._counter_label.setText(f"{self._current_index + 1} of {total}")
        self._prev_btn.setEnabled(self._current_index > 0)
        self._next_btn.setEnabled(self._current_index < total - 1)

        path = self._ordered_paths[self._current_index]
        is_marked = self._marked.get(path, False)
        angle = self._suggestions.get(path, 0)
        badge, _ = _ANGLE_LABELS.get(angle, (f"{angle}°", "#888"))
        self._decision_group.blockSignals(True)
        self._mark_btn.setText(f"Rotate {badge}  [R]")
        self._mark_btn.setChecked(is_marked)
        self._keep_btn.setChecked(not is_marked)
        self._decision_group.blockSignals(False)

        self._refresh_apply_button()
        self._refresh_list_colors()
        self._update_summary()

    def _refresh_apply_button(self) -> None:
        marked_count = sum(1 for v in self._marked.values() if v)
        self._apply_btn.setEnabled(marked_count > 0 and not self._applying)
        self._apply_btn.setText(
            f"Apply {marked_count} Rotation{'s' if marked_count != 1 else ''} Now  [A]"
            if marked_count > 0
            else "Nothing to Apply"
        )

    # ------------------------------------------------------------------
    # Event handlers
    # ------------------------------------------------------------------

    def _on_item_clicked(self, item: QListWidgetItem) -> None:
        row = self._items_list.row(item)
        if row != self._current_index:
            self._current_index = row
            self._show_current()
            self._refresh_controls()

    def _on_prev(self) -> None:
        self._navigate_to(self._current_index - 1)

    def _on_next(self) -> None:
        self._navigate_to(self._current_index + 1)

    def _on_mark_toggle(self) -> None:
        if self._current_index < 0 or not self._ordered_paths:
            return
        path = self._ordered_paths[self._current_index]
        self._set_current_marked(not self._marked.get(path, False))

    def _set_current_marked(self, marked: bool) -> None:
        if self._current_index < 0 or not self._ordered_paths:
            return
        path = self._ordered_paths[self._current_index]
        self._marked[path] = marked
        self._show_current()
        QTimer.singleShot(0, self._refresh_controls)

    def _on_mark_all(self) -> None:
        for path in self._ordered_paths:
            self._marked[path] = True
        if self._current_index >= 0:
            self._show_current()
        self._refresh_controls()

    def _on_unmark_all(self) -> None:
        for path in self._ordered_paths:
            self._marked[path] = False
        if self._current_index >= 0:
            self._show_current()
        self._refresh_controls()

    def _on_apply(self) -> None:
        rotations = {
            p: a for p, a in self._suggestions.items() if self._marked.get(p, False)
        }
        if rotations:
            self._submitted_paths = set(rotations)
            self._successful_paths.clear()
            self._failed_paths.clear()
            self._applying = True
            self._refresh_apply_button()
            self._state_banner.set_state(
                "Starting file changes",
                f"Preparing to rotate {len(rotations)} photo{'s' if len(rotations) != 1 else ''}.",
                tone="warning",
            )
            self.apply_rotations_requested.emit(rotations)

    def _on_primary_action(self) -> None:
        if self._applying:
            return
        if any(self._marked.values()):
            self._on_apply()
        else:
            self._on_proceed()

    def _on_proceed(self) -> None:
        self.proceed_requested.emit()

    def _on_skip(self) -> None:
        self.skip_requested.emit()

    # ------------------------------------------------------------------
    # UI construction
    # ------------------------------------------------------------------

    def _setup_ui(self) -> None:
        root = QVBoxLayout(self)
        root.setContentsMargins(0, 0, 0, 0)
        root.setSpacing(0)

        self._content_stack = QStackedWidget()
        root.addWidget(self._content_stack, 1)

        self._content_stack.addWidget(self._build_loading_page())  # 0
        self._content_stack.addWidget(self._build_results_page())  # 1
        self._content_stack.addWidget(self._build_empty_page())  # 2
        self._content_stack.setCurrentIndex(0)

    def _build_loading_page(self) -> QWidget:
        page = QWidget()
        layout = QVBoxLayout(page)
        layout.setAlignment(Qt.AlignmentFlag.AlignCenter)
        layout.setSpacing(16)

        title = QLabel("Fix Rotation")
        title.setAlignment(Qt.AlignmentFlag.AlignCenter)
        title.setStyleSheet("font-size: 20px; font-weight: bold; margin-bottom: 4px;")

        self._loading_label = QLabel("Analyzing rotation…")
        self._loading_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self._loading_label.setWordWrap(True)
        self._loading_label.setStyleSheet(
            "font-size: 13px; color: #aaaaaa; margin-bottom: 12px;"
        )

        self._progress_bar = QProgressBar()
        self._progress_bar.setRange(0, 100)
        self._progress_bar.setFixedWidth(320)
        self._progress_bar.setTextVisible(True)

        layout.addWidget(title)
        layout.addWidget(self._loading_label)
        layout.addWidget(self._progress_bar, alignment=Qt.AlignmentFlag.AlignCenter)
        return page

    def _build_results_page(self) -> QWidget:
        page = QWidget()
        page.setObjectName("workflowReviewPage")
        page_layout = QVBoxLayout(page)
        page_layout.setContentsMargins(0, 0, 0, 0)
        page_layout.setSpacing(0)

        self._review_header = WorkflowReviewHeader(
            step_number=3,
            title="Fix Rotation",
            description=(
                "Compare the original with the corrected preview. Queued rotations "
                "do not change files until you press Apply."
            ),
            shortcuts=FIX_ROTATION_SHORTCUTS,
        )
        self._review_header.skip_button.clicked.connect(self._on_skip)
        page_layout.addWidget(self._review_header)

        content = QWidget()
        content_layout = QVBoxLayout(content)
        content_layout.setContentsMargins(12, 10, 12, 10)
        content_layout.setSpacing(8)

        self._summary_label = QLabel()
        self._summary_label.hide()

        # Main splitter: list | dual-preview
        splitter = QSplitter(Qt.Orientation.Horizontal)
        splitter.setHandleWidth(4)
        splitter.setChildrenCollapsible(False)

        # Left: image list
        list_pane = QWidget()
        list_pane.setMinimumWidth(230)
        list_pane.setMaximumWidth(330)
        list_layout = QVBoxLayout(list_pane)
        list_layout.setContentsMargins(0, 0, 0, 0)
        list_layout.setSpacing(0)

        self._items_list = QListWidget()
        self._items_list.setFocusPolicy(Qt.FocusPolicy.NoFocus)
        self._items_list.setStyleSheet(
            "QListWidget { background: #242729; border: none; border-right: 1px solid #3C3F41; color: #A9B7C6; }"
            "QListWidget::item { padding: 7px 10px; border-bottom: 1px solid #303538; font-size: 11px; }"
            "QListWidget::item:selected { background: #1E3F62; color: #E0E8F0; }"
            "QListWidget::item:hover { background: #2C3438; }"
        )
        self._items_list.itemClicked.connect(self._on_item_clicked)
        list_layout.addWidget(self._items_list, 1)

        batch_actions = QHBoxLayout()
        batch_actions.setContentsMargins(6, 6, 6, 0)
        batch_actions.setSpacing(6)
        mark_all_btn = QPushButton("Queue all")
        mark_all_btn.setObjectName("workflowGhostButton")
        mark_all_btn.clicked.connect(self._on_mark_all)
        unmark_all_btn = QPushButton("Leave all as-is")
        unmark_all_btn.setObjectName("workflowGhostButton")
        unmark_all_btn.clicked.connect(self._on_unmark_all)
        batch_actions.addWidget(mark_all_btn)
        batch_actions.addWidget(unmark_all_btn)
        list_layout.addLayout(batch_actions)

        splitter.addWidget(list_pane)

        # Right: preview area + controls
        right_pane = QWidget()
        right_layout = QVBoxLayout(right_pane)
        right_layout.setContentsMargins(8, 0, 0, 0)
        right_layout.setSpacing(6)

        # Dual image view: current vs. proposed
        image_row = QWidget()
        image_row_layout = QHBoxLayout(image_row)
        image_row_layout.setContentsMargins(0, 0, 0, 0)
        image_row_layout.setSpacing(6)

        # Current panel
        current_pane = QWidget()
        current_pane_layout = QVBoxLayout(current_pane)
        current_pane_layout.setContentsMargins(0, 0, 0, 0)
        current_pane_layout.setSpacing(3)

        self._current_hdr = QLabel("CURRENT")
        self._current_hdr.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self._current_hdr.setStyleSheet(
            "font-size: 11px; color: #808080; letter-spacing: 1px;"
        )

        self._current_img = _RotatedImageLabel()
        current_pane_layout.addWidget(self._current_hdr)
        current_pane_layout.addWidget(self._current_img, 1)

        # After-rotation panel
        preview_pane = QWidget()
        preview_pane_layout = QVBoxLayout(preview_pane)
        preview_pane_layout.setContentsMargins(0, 0, 0, 0)
        preview_pane_layout.setSpacing(3)

        self._preview_hdr = QLabel("AFTER ROTATION")
        self._preview_hdr.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self._preview_hdr.setStyleSheet(
            "font-size: 11px; color: #4B6EAF; font-weight: bold; letter-spacing: 1px;"
        )

        self._preview_img = _RotatedImageLabel()
        preview_pane_layout.addWidget(self._preview_hdr)
        preview_pane_layout.addWidget(self._preview_img, 1)

        image_row_layout.addWidget(current_pane, 1)
        image_row_layout.addWidget(preview_pane, 1)
        right_layout.addWidget(image_row, 1)

        # Info + status row
        info_row = QHBoxLayout()
        info_row.setContentsMargins(0, 0, 0, 0)
        info_row.setSpacing(8)
        self._angle_label = QLabel()
        self._angle_label.setStyleSheet("font-size: 12px; color: #A9B7C6;")
        self._status_label = QLabel()
        self._status_label.setStyleSheet("font-size: 11px; color: #808080;")
        info_row.addWidget(self._angle_label)
        info_row.addStretch(1)
        info_row.addWidget(self._status_label)
        right_layout.addLayout(info_row)

        self._state_banner = WorkflowStateBanner()
        right_layout.addWidget(self._state_banner)

        # Action bar
        action = QHBoxLayout()
        action.setSpacing(6)

        self._prev_btn = QPushButton("← Prev")
        self._prev_btn.setObjectName("workflowGhostButton")
        self._prev_btn.setFixedWidth(70)
        self._prev_btn.setToolTip("Previous  [←]")
        self._prev_btn.clicked.connect(self._on_prev)

        self._counter_label = QLabel("0 of 0")
        self._counter_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self._counter_label.setStyleSheet("font-size: 12px; color: #808080;")
        self._counter_label.setFixedWidth(72)

        self._next_btn = QPushButton("Next →")
        self._next_btn.setObjectName("workflowGhostButton")
        self._next_btn.setFixedWidth(70)
        self._next_btn.setToolTip("Next  [→]")
        self._next_btn.clicked.connect(self._on_next)

        self._decision_group = QButtonGroup(self)
        self._decision_group.setExclusive(True)

        self._keep_btn = QPushButton("Leave as-is")
        self._keep_btn.setObjectName("workflowDecisionKeep")
        self._keep_btn.setCheckable(True)
        self._keep_btn.setMinimumWidth(100)
        self._keep_btn.clicked.connect(lambda: self._set_current_marked(False))
        self._decision_group.addButton(self._keep_btn)

        self._mark_btn = QPushButton("Rotate  [R]")
        self._mark_btn.setObjectName("workflowDecisionRotate")
        self._mark_btn.setCheckable(True)
        self._mark_btn.setMinimumWidth(140)
        self._mark_btn.setToolTip("Queue this rotation [R or Space]")
        self._mark_btn.clicked.connect(lambda: self._set_current_marked(True))
        self._decision_group.addButton(self._mark_btn)

        self._apply_btn = QPushButton("Apply Marked Rotations  [A]")
        self._apply_btn.setObjectName("workflowPrimaryButton")
        self._apply_btn.setEnabled(False)
        self._apply_btn.clicked.connect(self._on_apply)

        proceed_btn = QPushButton("Continue without applying  →")
        proceed_btn.setObjectName("workflowGhostButton")
        proceed_btn.setToolTip("Leave queued previews unapplied and continue")
        proceed_btn.clicked.connect(self._on_proceed)

        action.addWidget(self._prev_btn)
        action.addWidget(self._counter_label)
        action.addWidget(self._next_btn)
        action.addSpacing(8)
        action.addWidget(self._keep_btn)
        action.addWidget(self._mark_btn)
        action.addStretch(1)
        action.addWidget(proceed_btn)
        action.addWidget(self._apply_btn)
        right_layout.addLayout(action)

        splitter.addWidget(right_pane)
        content_layout.addWidget(splitter, 1)
        page_layout.addWidget(content, 1)
        return page

    def _build_empty_page(self) -> QWidget:
        page = QWidget()
        layout = QVBoxLayout(page)
        layout.setAlignment(Qt.AlignmentFlag.AlignCenter)
        layout.setSpacing(16)

        self._empty_title = QLabel("All photos are correctly oriented")
        self._empty_title.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self._empty_title.setStyleSheet(
            "font-size: 18px; font-weight: bold; color: #66BB6A;"
        )

        self._empty_subtitle = QLabel("No rotation corrections are needed.")
        self._empty_subtitle.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self._empty_subtitle.setStyleSheet("font-size: 13px; color: #aaaaaa;")

        proceed_btn = QPushButton("Continue to Pick Best →")
        proceed_btn.setObjectName("acceptButton")
        proceed_btn.setFixedWidth(220)
        proceed_btn.clicked.connect(self._on_proceed)

        layout.addWidget(self._empty_title)
        layout.addWidget(self._empty_subtitle)
        layout.addWidget(proceed_btn, alignment=Qt.AlignmentFlag.AlignCenter)
        return page

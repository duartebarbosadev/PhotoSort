from __future__ import annotations

import logging
import os
from typing import Dict, List, Optional

from PyQt6.QtCore import Qt, QTimer, pyqtSignal
from PyQt6.QtGui import QColor, QImage, QKeyEvent, QPixmap, QTransform
from PyQt6.QtWidgets import (
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
        self.setFocusPolicy(Qt.FocusPolicy.ClickFocus)
        self._setup_ui()

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
        self._marked = {p: True for p in suggestions}
        self._ordered_paths = sorted(suggestions.keys(), key=os.path.basename)
        self._current_index = -1

        if self._ordered_paths:
            self._populate_list()
            self._content_stack.setCurrentIndex(1)
            self._navigate_to(0)
            self.setFocus(Qt.FocusReason.OtherFocusReason)
        else:
            self._content_stack.setCurrentIndex(2)

    def show_applying(self, current: int, total: int, filename: str) -> None:
        self._applying = True
        self._apply_btn.setEnabled(False)
        self._apply_btn.setText(f"Applying… ({current}/{total})")
        if total > 0:
            self._progress_bar.setRange(0, total)
            self._progress_bar.setValue(current)
        self._status_label.setText(f"Rotating {filename}…")

    def show_apply_complete(self, successful: int, failed: int) -> None:
        self._applying = False
        total = successful + failed
        msg = f"Applied {successful}/{total} rotations"
        if failed > 0:
            msg += f" ({failed} failed)"
        self._status_label.setText(msg)
        self._apply_btn.setText("Apply Marked Rotations")
        self._refresh_apply_button()

    # ------------------------------------------------------------------
    # Private helpers
    # ------------------------------------------------------------------

    def _populate_list(self) -> None:
        self._items_list.clear()
        for path in self._ordered_paths:
            angle = self._suggestions.get(path, 0)
            badge, _ = _ANGLE_LABELS.get(angle, (f"{angle}°", "#888"))
            item_text = f"[{badge}]  {os.path.basename(path)}"
            item = QListWidgetItem(item_text)
            item.setData(Qt.ItemDataRole.UserRole, path)
            self._items_list.addItem(item)

        self._update_summary()
        self._refresh_list_colors()

    def _update_summary(self) -> None:
        total = len(self._ordered_paths)
        marked = sum(1 for v in self._marked.values() if v)
        self._summary_label.setText(
            f"{total} image{'s' if total != 1 else ''} with rotation suggestions — "
            f"{marked} marked"
        )

    def _refresh_list_colors(self) -> None:
        for i in range(self._items_list.count()):
            item = self._items_list.item(i)
            path = item.data(Qt.ItemDataRole.UserRole)
            is_marked = self._marked.get(path, False)
            color = _MARKED_COLOR if is_marked else _SKIP_COLOR
            item.setForeground(QColor(color))

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

        # Left panel: current (as-is) — no rotation applied
        self._current_img.set_pixmap_and_angle(self._load_pixmap(path), 0)
        self._current_hdr.setText(
            "<span style='color:#8899AA; font-size:11px'>CURRENT</span>"
        )

        # Right panel: preview after suggested rotation
        preview_angle = angle if is_marked else 0
        self._preview_img.set_pixmap_and_angle(self._load_pixmap(path), preview_angle)
        if is_marked:
            self._preview_hdr.setText(
                f"<b style='color:{color}'>AFTER ROTATION ({badge})</b>"
            )
        else:
            self._preview_hdr.setText(
                f"<span style='color:{_SKIP_COLOR}'>SKIPPED — will not rotate</span>"
            )

        self._angle_label.setText(
            f"<b style='color:{color}'>[{badge}]</b>  Suggested rotation: <b>{badge}</b>"
        )

    def _load_pixmap(self, path: str) -> Optional[QPixmap]:
        try:
            if self._image_pipeline:
                pil_img = self._image_pipeline.get_preview_image(path)
                if pil_img:
                    rgb = pil_img.convert("RGB")
                    data = rgb.tobytes("raw", "RGB")
                    qimg = QImage(
                        data,
                        rgb.width,
                        rgb.height,
                        rgb.width * 3,
                        QImage.Format.Format_RGB888,
                    )
                    return QPixmap.fromImage(qimg)
            px = QPixmap(path)
            if not px.isNull():
                return px
        except Exception as exc:
            logger.debug(
                "FixRotation: pixmap load failed for %s: %s",
                os.path.basename(path),
                exc,
            )
        return None

    def _refresh_controls(self) -> None:
        total = len(self._ordered_paths)
        if total == 0:
            return
        self._counter_label.setText(f"{self._current_index + 1} of {total}")
        self._prev_btn.setEnabled(self._current_index > 0)
        self._next_btn.setEnabled(self._current_index < total - 1)

        path = self._ordered_paths[self._current_index]
        is_marked = self._marked.get(path, False)
        if is_marked:
            self._mark_btn.setText("Skip This (don't rotate)  [R]")
            self._mark_btn.setStyleSheet(
                "background: qlineargradient(x1:0,y1:0,x2:0,y2:1,stop:0 #178B4A,stop:1 #0F6D39);"
                " color: #FFFFFF; border: 1px solid #1C9A53; padding: 4px 14px;"
            )
        else:
            self._mark_btn.setText("Mark for Rotation  [R]")
            self._mark_btn.setStyleSheet(
                "background: qlineargradient(x1:0,y1:0,x2:0,y2:1,stop:0 #4B6EAF,stop:1 #3D5A8A);"
                " color: #FFFFFF; border: 1px solid #5580BB; padding: 4px 14px;"
            )

        self._refresh_apply_button()
        self._refresh_list_colors()
        self._update_summary()

    def _refresh_apply_button(self) -> None:
        marked_count = sum(1 for v in self._marked.values() if v)
        self._apply_btn.setEnabled(marked_count > 0 and not self._applying)
        self._apply_btn.setText(
            f"Apply {marked_count} Rotation{'s' if marked_count != 1 else ''}  [A]"
            if marked_count > 0
            else "No Rotations Marked"
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
        self._marked[path] = not self._marked.get(path, False)
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
            self.apply_rotations_requested.emit(rotations)

    def _on_proceed(self) -> None:
        self.proceed_requested.emit()

    def _on_skip(self) -> None:
        self.skip_requested.emit()

    def keyPressEvent(self, event: QKeyEvent) -> None:
        key = event.key()
        if key == Qt.Key.Key_R or key == Qt.Key.Key_Space:
            self._on_mark_toggle()
        elif key in (Qt.Key.Key_Left, Qt.Key.Key_Up):
            self._on_prev()
        elif key in (Qt.Key.Key_Right, Qt.Key.Key_Down):
            self._on_next()
        elif key == Qt.Key.Key_A:
            self._on_apply()
        elif key == Qt.Key.Key_Return:
            self._on_proceed()
        elif key == Qt.Key.Key_Escape:
            self._on_skip()
        else:
            super().keyPressEvent(event)

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
        page_layout = QVBoxLayout(page)
        page_layout.setContentsMargins(10, 8, 10, 8)
        page_layout.setSpacing(8)

        # Header row
        header = QWidget()
        hl = QHBoxLayout(header)
        hl.setContentsMargins(0, 0, 0, 0)
        hl.setSpacing(8)

        self._summary_label = QLabel()
        self._summary_label.setStyleSheet("font-size: 13px; font-weight: bold;")

        hint = QLabel(
            "R — toggle  ·  A — apply  ·  ←/→ — navigate  ·  Enter — proceed  ·  Esc — skip"
        )
        hint.setStyleSheet("font-size: 11px; color: #888888;")

        skip_btn = QPushButton("Skip Step")
        skip_btn.setFixedWidth(80)
        skip_btn.clicked.connect(self._on_skip)

        hl.addWidget(self._summary_label)
        hl.addStretch(1)
        hl.addWidget(hint)
        hl.addWidget(skip_btn)
        page_layout.addWidget(header)

        # Main splitter: list | dual-preview
        splitter = QSplitter(Qt.Orientation.Horizontal)
        splitter.setHandleWidth(4)
        splitter.setChildrenCollapsible(False)

        # Left: image list
        list_pane = QWidget()
        list_pane.setMinimumWidth(180)
        list_pane.setMaximumWidth(280)
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

        # Action bar
        action = QHBoxLayout()
        action.setSpacing(6)

        self._prev_btn = QPushButton("← Prev")
        self._prev_btn.setFixedWidth(70)
        self._prev_btn.setToolTip("Previous  [←]")
        self._prev_btn.clicked.connect(self._on_prev)

        self._counter_label = QLabel("0 of 0")
        self._counter_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self._counter_label.setStyleSheet("font-size: 12px; color: #808080;")
        self._counter_label.setFixedWidth(72)

        self._next_btn = QPushButton("Next →")
        self._next_btn.setFixedWidth(70)
        self._next_btn.setToolTip("Next  [→]")
        self._next_btn.clicked.connect(self._on_next)

        self._mark_btn = QPushButton("Mark for Rotation  [R]")
        self._mark_btn.setMinimumWidth(160)
        self._mark_btn.setToolTip("Toggle rotation mark  [R or Space]")
        self._mark_btn.clicked.connect(self._on_mark_toggle)

        mark_all_btn = QPushButton("Mark All")
        mark_all_btn.setFixedWidth(72)
        mark_all_btn.setToolTip("Mark all for rotation")
        mark_all_btn.clicked.connect(self._on_mark_all)

        unmark_all_btn = QPushButton("Clear All")
        unmark_all_btn.setFixedWidth(72)
        unmark_all_btn.setToolTip("Remove all rotation marks")
        unmark_all_btn.clicked.connect(self._on_unmark_all)

        self._apply_btn = QPushButton("Apply Marked Rotations  [A]")
        self._apply_btn.setEnabled(False)
        self._apply_btn.setStyleSheet(
            "background: qlineargradient(x1:0,y1:0,x2:0,y2:1,stop:0 #178B4A,stop:1 #0F6D39);"
            " color: #FFFFFF; border: 1px solid #1C9A53; padding: 4px 16px; font-weight: bold;"
        )
        self._apply_btn.clicked.connect(self._on_apply)

        proceed_btn = QPushButton("Continue →")
        proceed_btn.setObjectName("acceptButton")
        proceed_btn.setToolTip("Proceed to next step  [Enter]")
        proceed_btn.clicked.connect(self._on_proceed)

        action.addWidget(self._prev_btn)
        action.addWidget(self._counter_label)
        action.addWidget(self._next_btn)
        action.addSpacing(8)
        action.addWidget(self._mark_btn)
        action.addWidget(mark_all_btn)
        action.addWidget(unmark_all_btn)
        action.addStretch(1)
        action.addWidget(self._apply_btn)
        action.addWidget(proceed_btn)
        right_layout.addLayout(action)

        splitter.addWidget(right_pane)
        page_layout.addWidget(splitter, 1)
        return page

    def _build_empty_page(self) -> QWidget:
        page = QWidget()
        layout = QVBoxLayout(page)
        layout.setAlignment(Qt.AlignmentFlag.AlignCenter)
        layout.setSpacing(16)

        title = QLabel("All images are correctly oriented")
        title.setAlignment(Qt.AlignmentFlag.AlignCenter)
        title.setStyleSheet("font-size: 18px; font-weight: bold; color: #66BB6A;")

        subtitle = QLabel("No rotation corrections needed.")
        subtitle.setAlignment(Qt.AlignmentFlag.AlignCenter)
        subtitle.setStyleSheet("font-size: 13px; color: #aaaaaa;")

        proceed_btn = QPushButton("Continue to Pick Best →")
        proceed_btn.setObjectName("acceptButton")
        proceed_btn.setFixedWidth(220)
        proceed_btn.clicked.connect(self._on_proceed)

        layout.addWidget(title)
        layout.addWidget(subtitle)
        layout.addWidget(proceed_btn, alignment=Qt.AlignmentFlag.AlignCenter)
        return page

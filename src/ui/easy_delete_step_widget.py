from __future__ import annotations

import logging
import os
from typing import Callable, Dict, List, Optional

from PyQt6.QtCore import Qt, QTimer, pyqtSignal
from PyQt6.QtGui import QColor, QImage, QKeyEvent, QPixmap
from PyQt6.QtWidgets import (
    QCheckBox,
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

_ISSUE_LABELS: Dict[str, tuple] = {
    "blur": ("BLUR", "#FF6B6B"),
    "dark": ("DARK", "#4A90D9"),
    "white": ("WHITE", "#F5B700"),
    "duplicate": ("DUP", "#A78BFA"),
}
_ISSUE_ORDER = ("duplicate", "blur", "dark", "white")
_CATEGORY_NAMES: Dict[str, str] = {
    "blur": "Blurry",
    "dark": "Near-black",
    "white": "Overexposed",
    "duplicate": "Duplicates",
}

_MARKED_COLOR = "#E53935"


class _ScaledImageLabel(QLabel):
    """QLabel that scales a stored pixmap to fill its size while keeping aspect ratio."""

    def __init__(self, parent: Optional[QWidget] = None) -> None:
        super().__init__(parent)
        self._source_pixmap: Optional[QPixmap] = None
        self.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.setMinimumSize(80, 80)
        self.setSizePolicy(QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Expanding)
        self.setStyleSheet("background: #232628;")

    def set_pixmap(self, pixmap: Optional[QPixmap]) -> None:
        self._source_pixmap = pixmap
        self._refresh()

    def resizeEvent(self, event) -> None:
        super().resizeEvent(event)
        self._refresh()

    def _refresh(self) -> None:
        if self._source_pixmap and not self._source_pixmap.isNull():
            scaled = self._source_pixmap.scaled(
                self.size(),
                Qt.AspectRatioMode.KeepAspectRatio,
                Qt.TransformationMode.SmoothTransformation,
            )
            super().setPixmap(scaled)
        else:
            super().setPixmap(QPixmap())


class EasyDeleteStepWidget(QWidget):
    """Step 2: Review and mark obviously bad / duplicate images for deletion."""

    proceed_to_pick_best_requested = pyqtSignal()
    skip_requested = pyqtSignal()
    mark_for_deletion_requested = pyqtSignal(list)
    unmark_for_deletion_requested = pyqtSignal(list)

    def __init__(self, parent: Optional[QWidget] = None) -> None:
        super().__init__(parent)
        self._results: Dict[str, dict] = {}
        self._flagged_paths: List[str] = []
        self._current_index: int = -1
        self._category_counts: Dict[str, int] = {}
        self._enabled_categories: Dict[str, bool] = {}
        self._category_checkboxes: Dict[str, QCheckBox] = {}
        self._updating_category_toggles = False
        self._is_marked_func: Optional[Callable[[str], bool]] = None
        self._has_any_marked_func: Optional[Callable[[], bool]] = None
        self._image_pipeline = None
        self.setFocusPolicy(Qt.FocusPolicy.ClickFocus)
        self._setup_ui()

    def set_is_marked_func(self, fn: Callable[[str], bool]) -> None:
        self._is_marked_func = fn

    def set_has_any_marked_func(self, fn: Callable[[], bool]) -> None:
        self._has_any_marked_func = fn

    def set_image_pipeline(self, pipeline) -> None:
        self._image_pipeline = pipeline

    # ------------------------------------------------------------------
    # Public state-machine API
    # ------------------------------------------------------------------

    def show_loading(self, message: str = "", percent: int = -1) -> None:
        self._loading_label.setText(message or "Analyzing images…")
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

    def show_results(self, results: Dict[str, dict]) -> None:
        self._results = results
        self._category_counts = self._build_category_counts(results)
        self._enabled_categories = {
            issue_type: True
            for issue_type in _ISSUE_ORDER
            if self._category_counts.get(issue_type, 0) > 0
        }
        self._flagged_paths = self._build_ordered_paths(results)
        self._current_index = -1
        self._refresh_category_controls()

        counts = {}
        for path in self._flagged_paths:
            t = self._results.get(path, {}).get("type", "?")
            counts[t] = counts.get(t, 0) + 1
        logger.info(
            f"EasyDelete results: {len(self._flagged_paths)} flagged — "
            f"{', '.join(f'{v} {k}' for k, v in sorted(counts.items()))}"
        )

        if self._flagged_paths:
            self._populate_list()
            self._content_stack.setCurrentIndex(1)
            self._navigate_to(0)
            self.setFocus(Qt.FocusReason.OtherFocusReason)
        else:
            self._content_stack.setCurrentIndex(2)

    # ------------------------------------------------------------------
    # Private helpers
    # ------------------------------------------------------------------

    def _build_ordered_paths(self, results: dict) -> List[str]:
        return self._build_ordered_paths_for_issue_types(
            results, self._enabled_issue_types()
        )

    def _build_ordered_paths_for_issue_types(
        self, results: dict, issue_types: List[str] | tuple[str, ...]
    ) -> List[str]:
        ordered: List[str] = []
        seen: set = set()
        for issue_type in issue_types:
            for path, entry in results.items():
                if (
                    path not in seen
                    and entry["type"] == issue_type
                    and entry["suggest_delete"]
                ):
                    ordered.append(path)
                    seen.add(path)
                    if entry.get("pair_path"):
                        seen.add(entry["pair_path"])
        return ordered

    def _build_category_counts(self, results: dict) -> Dict[str, int]:
        counts: Dict[str, int] = {}
        ordered_paths = self._build_ordered_paths_for_issue_types(results, _ISSUE_ORDER)
        for path in ordered_paths:
            issue_type = results.get(path, {}).get("type", "")
            counts[issue_type] = counts.get(issue_type, 0) + 1
        return counts

    def _enabled_issue_types(self) -> List[str]:
        return [
            issue_type
            for issue_type in _ISSUE_ORDER
            if self._enabled_categories.get(issue_type, False)
        ]

    def _issue_types_with_counts(self) -> List[str]:
        return [
            issue_type
            for issue_type in _ISSUE_ORDER
            if self._category_counts.get(issue_type, 0) > 0
        ]

    def _refresh_category_controls(self) -> None:
        issue_types = self._issue_types_with_counts()
        summary_parts = [
            f"{_CATEGORY_NAMES.get(issue_type, issue_type.title())}: {self._category_counts[issue_type]}"
            for issue_type in issue_types
        ]
        self._category_summary_label.setText(" · ".join(summary_parts))
        self._category_summary_label.setVisible(bool(summary_parts))

        while self._category_toggle_layout.count():
            item = self._category_toggle_layout.takeAt(0)
            widget = item.widget()
            if widget:
                widget.deleteLater()

        self._category_checkboxes = {}
        self._updating_category_toggles = True
        for issue_type in issue_types:
            count = self._category_counts[issue_type]
            checkbox = QCheckBox(
                f"{_CATEGORY_NAMES.get(issue_type, issue_type.title())} ({count})"
            )
            checkbox.setChecked(self._enabled_categories.get(issue_type, True))
            checkbox.setStyleSheet("font-size: 11px; color: #A9B7C6;")
            checkbox.toggled.connect(
                lambda checked, issue_type=issue_type: self._on_category_toggled(
                    issue_type, checked
                )
            )
            self._category_checkboxes[issue_type] = checkbox
            self._category_toggle_layout.addWidget(checkbox)
        self._category_toggle_layout.addStretch()
        self._updating_category_toggles = False
        self._category_toggle_container.setVisible(bool(issue_types))

    def _populate_list(self) -> None:
        self._items_list.clear()
        for path in self._flagged_paths:
            entry = self._results.get(path, {})
            issue_type = entry.get("type", "")
            badge, _ = _ISSUE_LABELS.get(issue_type, ("?", "#888"))

            if issue_type == "duplicate":
                pair = entry.get("pair_path", "")
                pair_name = os.path.basename(pair) if pair else ""
                item_text = f"[{badge}] {os.path.basename(path)} ↔ {pair_name}"
            else:
                item_text = f"[{badge}] {os.path.basename(path)}"

            item = QListWidgetItem(item_text)
            item.setData(Qt.ItemDataRole.UserRole, path)
            self._items_list.addItem(item)

        total = len(self._flagged_paths)
        all_total = sum(self._category_counts.values())
        if total == all_total:
            self._summary_label.setText(
                f"{total} image{'s' if total != 1 else ''} flagged for review"
            )
        else:
            self._summary_label.setText(
                f"{total} of {all_total} flagged image{'s' if all_total != 1 else ''} visible"
            )
        self._refresh_list_colors()

    def _refresh_list_colors(self) -> None:
        for i in range(self._items_list.count()):
            item = self._items_list.item(i)
            path = item.data(Qt.ItemDataRole.UserRole)
            is_marked = self._is_marked_func(path) if self._is_marked_func else False
            entry = self._results.get(path, {})
            issue_type = entry.get("type", "")
            _, color = _ISSUE_LABELS.get(issue_type, ("?", "#888888"))
            item.setForeground(QColor(_MARKED_COLOR if is_marked else color))

    def _navigate_to(self, index: int) -> None:
        if not self._flagged_paths:
            return
        index = max(0, min(index, len(self._flagged_paths) - 1))
        self._current_index = index

        self._items_list.blockSignals(True)
        self._items_list.setCurrentRow(index)
        self._items_list.blockSignals(False)

        self._show_current()
        self._refresh_controls()

    def _show_current(self) -> None:
        if self._current_index < 0 or self._current_index >= len(self._flagged_paths):
            return
        path = self._flagged_paths[self._current_index]
        entry = self._results.get(path, {})
        issue_type = entry.get("type", "")
        pair_path = entry.get("pair_path")

        if issue_type == "duplicate" and pair_path:
            self._show_pair(path, pair_path, entry)
        else:
            self._show_single(path, entry)

    def _show_single(self, path: str, entry: dict) -> None:
        self._image_stack.setCurrentIndex(0)
        self._load_into(path, self._single_img)
        issue_type = entry.get("type", "")
        label, color = _ISSUE_LABELS.get(issue_type, ("ISSUE", "#888"))
        reason = entry.get("reason", "")
        self._issue_label.setText(f"<b style='color:{color}'>[{label}]</b>  {reason}")
        self._suggestion_label.hide()
        logger.info(f"Showing [{label}] {os.path.basename(path)} — {reason}")

    def _show_pair(self, path: str, pair_path: str, entry: dict) -> None:
        self._image_stack.setCurrentIndex(1)
        self._load_into(path, self._pair_left_img)
        self._load_into(pair_path, self._pair_right_img)

        suggest_delete = entry.get("suggest_delete", True)
        left_name = os.path.basename(path)
        right_name = os.path.basename(pair_path)
        if suggest_delete:
            self._pair_left_hdr.setText(
                f"<b style='color:#E53935'>✕ Delete suggested:</b> {left_name}"
            )
            self._pair_right_hdr.setText(
                f"<b style='color:#66BB6A'>✓ Keep suggested:</b> {right_name}"
            )
        else:
            self._pair_left_hdr.setText(
                f"<b style='color:#66BB6A'>✓ Keep suggested:</b> {left_name}"
            )
            self._pair_right_hdr.setText(
                f"<b style='color:#E53935'>✕ Delete suggested:</b> {right_name}"
            )

        reason = entry.get("reason", "")
        self._suggestion_label.setText(reason)
        self._suggestion_label.show()
        _, color = _ISSUE_LABELS.get("duplicate", ("DUP", "#A78BFA"))
        self._issue_label.setText(
            f"<b style='color:{color}'>[DUP]</b>  Near-duplicate pair"
        )
        keep_path = pair_path if suggest_delete else path
        logger.info(
            "EasyDelete duplicate: keeping %s over %s — %s",
            os.path.basename(keep_path),
            os.path.basename(pair_path if suggest_delete else path),
            reason,
        )

    def _load_into(self, path: str, label: _ScaledImageLabel) -> None:
        label.set_pixmap(self._load_pixmap(path))

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
                "EasyDelete: could not load pixmap for %s: %s",
                os.path.basename(path),
                exc,
            )
        return None

    def _refresh_controls(self) -> None:
        total = len(self._flagged_paths)
        if total == 0:
            self._counter_label.setText("0 of 0")
            self._prev_btn.setEnabled(False)
            self._next_btn.setEnabled(False)
            self._mark_btn.setEnabled(False)
            return
        self._mark_btn.setEnabled(True)
        self._counter_label.setText(f"{self._current_index + 1} of {total}")
        self._prev_btn.setEnabled(self._current_index > 0)
        self._next_btn.setEnabled(self._current_index < total - 1)

        path = self._flagged_paths[self._current_index]
        is_marked = self._is_marked_func(path) if self._is_marked_func else False
        if is_marked:
            self._mark_btn.setText("Unmark for Deletion  [X]")
            self._mark_btn.setStyleSheet(
                "background: qlineargradient(x1:0,y1:0,x2:0,y2:1,stop:0 #8C2E35,stop:1 #6E2228);"
                " color: #FFFFFF; border: 1px solid #A63B42; padding: 4px 12px;"
            )
        else:
            self._mark_btn.setText("Mark for Deletion  [X]")
            self._mark_btn.setStyleSheet(
                "background: qlineargradient(x1:0,y1:0,x2:0,y2:1,stop:0 #A63B42,stop:1 #8C2E35);"
                " color: #FFFFFF; border: 1px solid #B84A52; padding: 4px 12px;"
            )
        self._refresh_list_colors()

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
        if self._current_index < 0 or not self._flagged_paths:
            return
        path = self._flagged_paths[self._current_index]
        is_marked = self._is_marked_func(path) if self._is_marked_func else False
        if is_marked:
            self.unmark_for_deletion_requested.emit([path])
        else:
            self.mark_for_deletion_requested.emit([path])
        QTimer.singleShot(0, self._refresh_controls)

    def _on_category_toggled(self, issue_type: str, checked: bool) -> None:
        if self._updating_category_toggles:
            return
        self._enabled_categories[issue_type] = checked
        self._apply_category_filter()

    def _apply_category_filter(self) -> None:
        current_path = None
        if 0 <= self._current_index < len(self._flagged_paths):
            current_path = self._flagged_paths[self._current_index]

        self._flagged_paths = self._build_ordered_paths(self._results)
        self._populate_list()
        self._content_stack.setCurrentIndex(1)

        if not self._flagged_paths:
            self._show_no_enabled_categories()
            return

        if current_path in self._flagged_paths:
            next_index = self._flagged_paths.index(current_path)
        else:
            next_index = 0
        self._navigate_to(next_index)

    def _show_no_enabled_categories(self) -> None:
        self._current_index = -1
        self._items_list.clearSelection()
        self._single_img.set_pixmap(None)
        self._pair_left_img.set_pixmap(None)
        self._pair_right_img.set_pixmap(None)
        self._image_stack.setCurrentIndex(0)
        self._issue_label.setText(
            "No enabled categories. Re-enable a category on the left to review those images."
        )
        self._suggestion_label.hide()
        self._refresh_controls()

    def _on_done(self) -> None:
        self.proceed_to_pick_best_requested.emit()

    def _on_skip(self) -> None:
        self.skip_requested.emit()

    def keyPressEvent(self, event: QKeyEvent) -> None:
        key = event.key()
        if key in (Qt.Key.Key_X, Qt.Key.Key_Delete):
            self._on_mark_toggle()
        elif key in (Qt.Key.Key_Left, Qt.Key.Key_Up):
            self._on_prev()
        elif key in (Qt.Key.Key_Right, Qt.Key.Key_Down):
            self._on_next()
        elif key == Qt.Key.Key_Return:
            self._on_done()
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

        self._content_stack.addWidget(self._build_loading_page())
        self._content_stack.addWidget(self._build_results_page())
        self._content_stack.addWidget(self._build_empty_page())
        self._content_stack.setCurrentIndex(0)

    def _build_loading_page(self) -> QWidget:
        page = QWidget()
        layout = QVBoxLayout(page)
        layout.setAlignment(Qt.AlignmentFlag.AlignCenter)
        layout.setSpacing(16)

        title = QLabel("Easy Delete")
        title.setAlignment(Qt.AlignmentFlag.AlignCenter)
        title.setStyleSheet("font-size: 20px; font-weight: bold; margin-bottom: 4px;")

        self._loading_label = QLabel("Analyzing images…")
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

        hint = QLabel("X — mark/unmark  ·  ←/→ — navigate  ·  Enter — proceed")
        hint.setStyleSheet("font-size: 11px; color: #888888;")

        skip_btn = QPushButton("Skip Step")
        skip_btn.setFixedWidth(80)
        skip_btn.clicked.connect(self._on_skip)

        hl.addWidget(self._summary_label)
        hl.addStretch()
        hl.addWidget(hint)
        hl.addWidget(skip_btn)
        page_layout.addWidget(header)

        # Main split: list | viewer
        splitter = QSplitter(Qt.Orientation.Horizontal)
        splitter.setHandleWidth(4)
        splitter.setChildrenCollapsible(False)

        # Left list
        left = QWidget()
        left.setMinimumWidth(200)
        left.setMaximumWidth(300)
        left_layout = QVBoxLayout(left)
        left_layout.setContentsMargins(0, 0, 0, 0)
        left_layout.setSpacing(4)

        self._category_summary_label = QLabel()
        self._category_summary_label.setWordWrap(True)
        self._category_summary_label.setStyleSheet(
            "font-size: 11px; color: #A9B7C6; padding: 0 4px 2px 4px;"
        )
        left_layout.addWidget(self._category_summary_label)

        self._category_toggle_container = QWidget()
        self._category_toggle_layout = QHBoxLayout(self._category_toggle_container)
        self._category_toggle_layout.setContentsMargins(4, 0, 4, 4)
        self._category_toggle_layout.setSpacing(6)
        left_layout.addWidget(self._category_toggle_container)

        self._items_list = QListWidget()
        self._items_list.setFocusPolicy(Qt.FocusPolicy.NoFocus)
        self._items_list.setStyleSheet(
            "QListWidget { background: #242729; border: none; border-right: 1px solid #3C3F41; color: #A9B7C6; }"
            "QListWidget::item { padding: 7px 12px; border-bottom: 1px solid #303538; font-size: 11px; }"
            "QListWidget::item:selected { background: #1E3F62; color: #E0E8F0; }"
            "QListWidget::item:hover { background: #2C3438; }"
        )
        self._items_list.itemClicked.connect(self._on_item_clicked)
        left_layout.addWidget(self._items_list)

        # Right viewer
        right = QWidget()
        right_layout = QVBoxLayout(right)
        right_layout.setContentsMargins(8, 4, 8, 4)
        right_layout.setSpacing(6)

        self._issue_label = QLabel()
        self._issue_label.setWordWrap(True)
        self._issue_label.setStyleSheet("font-size: 12px; color: #A9B7C6;")
        right_layout.addWidget(self._issue_label)

        # Image display: 0=single, 1=pair
        self._image_stack = QStackedWidget()

        # Single image view
        single = QWidget()
        sl = QVBoxLayout(single)
        sl.setContentsMargins(0, 0, 0, 0)
        self._single_img = _ScaledImageLabel()
        sl.addWidget(self._single_img, 1)

        # Pair image view
        pair = QWidget()
        pl = QVBoxLayout(pair)
        pl.setContentsMargins(0, 0, 0, 0)
        pair_splitter = QSplitter(Qt.Orientation.Horizontal)

        lp = QWidget()
        ll = QVBoxLayout(lp)
        ll.setContentsMargins(0, 0, 4, 0)
        ll.setSpacing(3)
        self._pair_left_hdr = QLabel()
        self._pair_left_hdr.setWordWrap(True)
        self._pair_left_hdr.setStyleSheet("font-size: 11px;")
        self._pair_left_img = _ScaledImageLabel()
        ll.addWidget(self._pair_left_hdr)
        ll.addWidget(self._pair_left_img, 1)

        rp = QWidget()
        rl = QVBoxLayout(rp)
        rl.setContentsMargins(4, 0, 0, 0)
        rl.setSpacing(3)
        self._pair_right_hdr = QLabel()
        self._pair_right_hdr.setWordWrap(True)
        self._pair_right_hdr.setStyleSheet("font-size: 11px;")
        self._pair_right_img = _ScaledImageLabel()
        rl.addWidget(self._pair_right_hdr)
        rl.addWidget(self._pair_right_img, 1)

        pair_splitter.addWidget(lp)
        pair_splitter.addWidget(rp)
        pl.addWidget(pair_splitter, 1)

        self._image_stack.addWidget(single)  # 0
        self._image_stack.addWidget(pair)  # 1
        right_layout.addWidget(self._image_stack, 1)

        # Suggestion banner (duplicate hint)
        self._suggestion_label = QLabel()
        self._suggestion_label.setWordWrap(True)
        self._suggestion_label.setStyleSheet(
            "background: #2C2616; color: #F5B700; border: 1px solid #4A3B00;"
            " border-radius: 4px; padding: 6px 10px; font-size: 11px;"
        )
        self._suggestion_label.hide()
        right_layout.addWidget(self._suggestion_label)

        # Action bar
        action = QHBoxLayout()
        action.setSpacing(6)

        self._prev_btn = QPushButton("← Prev")
        self._prev_btn.setFixedWidth(70)
        self._prev_btn.clicked.connect(self._on_prev)

        self._counter_label = QLabel("0 of 0")
        self._counter_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self._counter_label.setStyleSheet("font-size: 12px; color: #808080;")
        self._counter_label.setFixedWidth(72)

        self._next_btn = QPushButton("Next →")
        self._next_btn.setFixedWidth(70)
        self._next_btn.clicked.connect(self._on_next)

        self._mark_btn = QPushButton("Mark for Deletion  [X]")
        self._mark_btn.setMinimumWidth(160)
        self._mark_btn.clicked.connect(self._on_mark_toggle)

        self._done_btn = QPushButton("Done → Fix Rotation")
        self._done_btn.setObjectName("acceptButton")
        self._done_btn.clicked.connect(self._on_done)

        action.addWidget(self._prev_btn)
        action.addWidget(self._counter_label)
        action.addWidget(self._next_btn)
        action.addStretch()
        action.addWidget(self._mark_btn)
        action.addWidget(self._done_btn)
        right_layout.addLayout(action)

        splitter.addWidget(left)
        splitter.addWidget(right)
        splitter.setStretchFactor(0, 1)
        splitter.setStretchFactor(1, 3)
        page_layout.addWidget(splitter, 1)
        return page

    def _build_empty_page(self) -> QWidget:
        page = QWidget()
        layout = QVBoxLayout(page)
        layout.setAlignment(Qt.AlignmentFlag.AlignCenter)
        layout.setSpacing(16)

        lbl = QLabel("No obvious issues detected — all images look good!")
        lbl.setAlignment(Qt.AlignmentFlag.AlignCenter)
        lbl.setStyleSheet("font-size: 16px; color: #66BB6A;")

        btn = QPushButton("Continue to Fix Rotation →")
        btn.setObjectName("acceptButton")
        btn.setFixedWidth(230)
        btn.clicked.connect(self._on_done)

        layout.addWidget(lbl)
        layout.addWidget(btn, alignment=Qt.AlignmentFlag.AlignCenter)
        return page

import logging
import os
from collections.abc import Callable
from typing import override

from PyQt6.QtCore import Qt, pyqtSignal
from PyQt6.QtGui import QColor, QPixmap
from PyQt6.QtWidgets import (
    QButtonGroup,
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

from ui.workflow_review_components import (
    EASY_DELETE_SHORTCUTS,
    WorkflowReviewHeader,
    WorkflowStateBanner,
    install_workflow_shortcuts,
)

logger = logging.getLogger(__name__)

_ISSUE_LABELS: dict[str, tuple] = {
    "blur": ("BLUR", "#FF6B6B"),
    "dark": ("DARK", "#4A90D9"),
    "white": ("WHITE", "#F5B700"),
    "duplicate": ("DUP", "#A78BFA"),
}
_ISSUE_ORDER = ("duplicate", "blur", "dark", "white")
_CATEGORY_NAMES: dict[str, str] = {
    "blur": "Blurry",
    "dark": "Near-black",
    "white": "Overexposed",
    "duplicate": "Duplicates",
}

_CONFIRMED_COLOR = "#66BB6A"


class _ScaledImageLabel(QLabel):
    """QLabel that scales a stored pixmap to fill its size while keeping aspect ratio."""

    clicked = pyqtSignal()

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self._source_pixmap: QPixmap | None = None
        self.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.setMinimumSize(80, 80)
        self.setSizePolicy(QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Expanding)
        self.setStyleSheet("background: #232628;")

    def set_pixmap(self, pixmap: QPixmap | None) -> None:
        self._source_pixmap = pixmap
        self._refresh()

    @override
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

    @override
    def mousePressEvent(self, event) -> None:
        if event.button() == Qt.MouseButton.LeftButton:
            self.clicked.emit()
        super().mousePressEvent(event)


class EasyDeleteStepWidget(QWidget):
    """Step 2: Review and mark obviously bad / duplicate images for deletion."""

    proceed_to_pick_best_requested = pyqtSignal()
    skip_requested = pyqtSignal()
    mark_for_deletion_requested = pyqtSignal(list)
    unmark_for_deletion_requested = pyqtSignal(list)

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self._results: dict[str, dict] = {}
        self._flagged_paths: list[str] = []
        self._current_index: int = -1
        self._category_counts: dict[str, int] = {}
        self._enabled_categories: dict[str, bool] = {}
        self._category_checkboxes: dict[str, QCheckBox] = {}
        self._updating_category_toggles = False
        self._is_marked_func: Callable[[str], bool] | None = None
        self._has_any_marked_func: Callable[[], bool] | None = None
        self._pending_delete_by_review: dict[str, str | None] = {}
        self._confirmed_reviews: set[str] = set()
        self._image_pipeline = None
        self.setFocusPolicy(Qt.FocusPolicy.ClickFocus)
        self._setup_ui()
        self._shortcuts = install_workflow_shortcuts(
            self,
            EASY_DELETE_SHORTCUTS,
            {
                "select_left": lambda: self._select_pair_side(0),
                "select_right": lambda: self._select_pair_side(1),
                "previous": self._on_prev,
                "next": self._on_next,
                "confirm": self._on_confirm,
                "apply_all": self._on_apply_all,
                "skip": self._on_skip,
            },
        )

    def set_is_marked_func(self, fn: Callable[[str], bool]) -> None:
        self._is_marked_func = fn

    def set_has_any_marked_func(self, fn: Callable[[], bool]) -> None:
        self._has_any_marked_func = fn

    def set_image_pipeline(self, pipeline) -> None:
        self._image_pipeline = pipeline

    def refresh_deletion_state(self) -> None:
        """Refresh staged decisions after another workflow changes shared state."""

        if not hasattr(self, "_items_list"):
            return
        self._discard_stale_confirmations()
        self._refresh_controls()

    def _discard_stale_confirmations(self) -> None:
        if not self._is_marked_func:
            return
        for review_path in list(self._confirmed_reviews):
            entry = self._results.get(review_path, {})
            selected_delete = self._pending_delete_by_review.get(review_path)
            pair_path = entry.get("pair_path")
            candidates = [review_path] + ([pair_path] if pair_path else [])
            matches_shared_state = all(
                self._is_marked_func(candidate) == (candidate == selected_delete)
                for candidate in candidates
            )
            if not matches_shared_state:
                self._confirmed_reviews.discard(review_path)

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

    def show_results(self, results: dict[str, dict]) -> None:
        if results != self._results:
            self._pending_delete_by_review.clear()
            self._confirmed_reviews.clear()
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

    def _build_ordered_paths(self, results: dict) -> list[str]:
        return self._build_ordered_paths_for_issue_types(
            results, self._enabled_issue_types()
        )

    def _build_ordered_paths_for_issue_types(
        self, results: dict, issue_types: list[str] | tuple[str, ...]
    ) -> list[str]:
        ordered: list[str] = []
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

    def _build_category_counts(self, results: dict) -> dict[str, int]:
        counts: dict[str, int] = {}
        ordered_paths = self._build_ordered_paths_for_issue_types(results, _ISSUE_ORDER)
        for path in ordered_paths:
            issue_type = results.get(path, {}).get("type", "")
            counts[issue_type] = counts.get(issue_type, 0) + 1
        return counts

    def _enabled_issue_types(self) -> list[str]:
        return [
            issue_type
            for issue_type in _ISSUE_ORDER
            if self._enabled_categories.get(issue_type, False)
        ]

    def _issue_types_with_counts(self) -> list[str]:
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

    def _item_text(self, path: str) -> str:
        entry = self._results.get(path, {})
        issue_type = entry.get("type", "")
        badge, _ = _ISSUE_LABELS.get(issue_type, ("?", "#888"))
        state = "CONFIRMED" if path in self._confirmed_reviews else "REVIEW"
        if issue_type == "duplicate":
            pair = entry.get("pair_path", "")
            pair_name = os.path.basename(pair) if pair else ""
            return f"{state}  ·  [{badge}] {os.path.basename(path)}  ↔  {pair_name}"
        return f"{state}  ·  [{badge}] {os.path.basename(path)}"

    def _populate_list(self) -> None:
        self._items_list.clear()
        for path in self._flagged_paths:
            item = QListWidgetItem(self._item_text(path))
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
            item.setForeground(
                QColor(
                    _CONFIRMED_COLOR
                    if path in self._confirmed_reviews
                    else "#A9B7C6"
                )
            )
            item.setText(self._item_text(path))

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
        if not self._load_into(path, self._single_img):
            self._request_previews([path])
        issue_type = entry.get("type", "")
        label, color = _ISSUE_LABELS.get(issue_type, ("ISSUE", "#888"))
        reason = entry.get("reason", "")
        self._issue_label.setText(f"<b style='color:{color}'>[{label}]</b>  {reason}")
        self._suggestion_label.hide()
        self._update_decision_presentation(path, entry)
        logger.info(f"Showing [{label}] {os.path.basename(path)} — {reason}")

    def _show_pair(self, path: str, pair_path: str, entry: dict) -> None:
        self._image_stack.setCurrentIndex(1)
        missing_paths = []
        if not self._load_into(path, self._pair_left_img):
            missing_paths.append(path)
        if not self._load_into(pair_path, self._pair_right_img):
            missing_paths.append(pair_path)
        self._request_previews(missing_paths)

        left_name = os.path.basename(path)
        right_name = os.path.basename(pair_path)
        self._pair_left_hdr.setToolTip(left_name)
        self._pair_right_hdr.setToolTip(right_name)

        reason = entry.get("reason", "")
        self._suggestion_label.hide()
        _, color = _ISSUE_LABELS.get("duplicate", ("DUP", "#A78BFA"))
        duplicate_kind = entry.get("duplicate_kind", "near")
        classification = "Exact duplicate" if duplicate_kind == "exact" else "Near-duplicate"
        self._issue_label.setText(
            f"<b style='color:{color}'>{classification}</b>  ·  {reason}"
        )
        self._update_decision_presentation(path, entry)
        logger.info(
            "EasyDelete duplicate review: %s and %s — %s",
            os.path.basename(path),
            os.path.basename(pair_path),
            reason,
        )

    def _update_decision_presentation(self, path: str, entry: dict) -> None:
        filename = os.path.basename(path)
        selected_delete = self._pending_delete(path, entry)
        confirmed = path in self._confirmed_reviews
        if confirmed:
            self._state_banner.set_state(
                "Decision confirmed",
                "The selected photo is marked for Trash, but no file has been moved or deleted.",
                tone="success",
            )
        else:
            self._state_banner.set_state(
                "Choose, then confirm",
                "This is only a selection. Nothing changes until you press Confirm.",
                tone="warning",
            )

        if entry.get("type") == "duplicate" and entry.get("pair_path"):
            pair_path = entry["pair_path"]
            pair_name = os.path.basename(pair_path)
            left_is_delete = selected_delete == path
            if confirmed:
                delete_state = "MARKED FOR TRASH · staged"
                keep_state = "KEEP"
            else:
                delete_state = "SELECTED FOR TRASH · not confirmed"
                keep_state = "SELECTED TO KEEP · not confirmed"
            self._pair_left_hdr.setText(
                f"<b style='color:{'#FF7B86' if left_is_delete else '#66BB6A'}'>"
                f"{delete_state if left_is_delete else keep_state}</b><br>LEFT · {filename}"
            )
            self._pair_right_hdr.setText(
                f"<b style='color:{'#66BB6A' if left_is_delete else '#FF7B86'}'>"
                f"{keep_state if left_is_delete else delete_state}</b><br>RIGHT · {pair_name}"
            )
            self._pair_left_img.setStyleSheet(
                self._image_choice_style(left_is_delete)
            )
            self._pair_right_img.setStyleSheet(
                self._image_choice_style(not left_is_delete)
            )
            self._keep_btn.setText("Trash left")
            self._mark_btn.setText("Trash right")
            self._set_button_role(self._keep_btn, "workflowDecisionTrash")
            self._keep_btn.setChecked(left_is_delete)
            self._mark_btn.setChecked(not left_is_delete)
        else:
            delete_selected = selected_delete == path
            self._keep_btn.setText("Keep")
            self._mark_btn.setText("Trash")
            self._set_button_role(self._keep_btn, "workflowDecisionKeep")
            self._keep_btn.setChecked(not delete_selected)
            self._mark_btn.setChecked(delete_selected)

    @staticmethod
    def _set_button_role(button: QPushButton, object_name: str) -> None:
        if button.objectName() == object_name:
            return
        button.setObjectName(object_name)
        style = button.style()
        style.unpolish(button)
        style.polish(button)
        button.update()

    @staticmethod
    def _image_choice_style(selected_for_delete: bool) -> str:
        color = "#E53935" if selected_for_delete else "#2E7D32"
        return f"background: #232628; border: 3px solid {color}; border-radius: 4px;"

    def _pending_delete(self, path: str, entry: dict) -> str | None:
        if path not in self._pending_delete_by_review:
            pair_path = entry.get("pair_path")
            if pair_path and self._is_marked_func and self._is_marked_func(pair_path):
                selected = pair_path
            elif self._is_marked_func and self._is_marked_func(path):
                selected = path
            else:
                selected = path if entry.get("suggest_delete", True) else None
            self._pending_delete_by_review[path] = selected
        return self._pending_delete_by_review[path]

    def _load_into(self, path: str, label: _ScaledImageLabel) -> bool:
        pixmap = self._load_pixmap(path)
        label.set_pixmap(pixmap)
        return pixmap is not None and not pixmap.isNull()

    def _request_previews(self, paths: list[str]) -> None:
        request = getattr(self.window(), "request_interactive_previews", None)
        if paths and callable(request):
            request(paths)

    def _load_pixmap(self, path: str) -> QPixmap | None:
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
                "EasyDelete: could not load pixmap for %s: %s",
                os.path.basename(path),
                exc,
            )
        return None

    def handle_preview_ready(self, path: str) -> None:
        """Upgrade only the currently visible card when its preview arrives."""
        if not (0 <= self._current_index < len(self._flagged_paths)):
            return
        current_path = self._flagged_paths[self._current_index]
        entry = self._results.get(current_path, {})
        pair_path = entry.get("pair_path")
        if path == current_path:
            target = (
                self._pair_left_img
                if entry.get("type") == "duplicate" and pair_path
                else self._single_img
            )
            self._load_into(path, target)
        elif path == pair_path:
            self._load_into(path, self._pair_right_img)

    def _refresh_controls(self) -> None:
        total = len(self._flagged_paths)
        if total == 0:
            self._counter_label.setText("0 of 0")
            self._prev_btn.setEnabled(False)
            self._next_btn.setEnabled(False)
            self._mark_btn.setEnabled(False)
            self._keep_btn.setEnabled(False)
            self._confirm_btn.setEnabled(False)
            self._apply_all_btn.setEnabled(False)
            self._review_header.set_summary("No suggestions visible")
            return
        self._mark_btn.setEnabled(True)
        self._keep_btn.setEnabled(True)
        self._confirm_btn.setEnabled(True)
        self._apply_all_btn.setEnabled(True)
        self._counter_label.setText(f"{self._current_index + 1} of {total}")
        self._prev_btn.setEnabled(self._current_index > 0)
        self._next_btn.setEnabled(self._current_index < total - 1)

        path = self._flagged_paths[self._current_index]
        self._decision_group.blockSignals(True)
        self._update_decision_presentation(path, self._results.get(path, {}))
        self._decision_group.blockSignals(False)

        confirmed_here = sum(
            candidate in self._confirmed_reviews for candidate in self._flagged_paths
        )
        self._review_header.set_summary(
            f"{total} suggestions  ·  {confirmed_here} confirmed",
            "success" if confirmed_here == total else "neutral",
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

    def _select_pair_side(self, side: int) -> None:
        if self._current_index < 0 or not self._flagged_paths:
            return
        path = self._flagged_paths[self._current_index]
        entry = self._results.get(path, {})
        pair_path = entry.get("pair_path")
        if not pair_path:
            self._select_current_delete(path if side == 1 else None)
            return
        self._select_current_delete(path if side == 0 else pair_path)

    def _select_current_delete(self, selected_path: str | None) -> None:
        if self._current_index < 0 or not self._flagged_paths:
            return
        review_path = self._flagged_paths[self._current_index]
        self._pending_delete_by_review[review_path] = selected_path
        self._confirmed_reviews.discard(review_path)
        self._refresh_controls()

    def _on_confirm(self) -> None:
        if self._current_index < 0 or not self._flagged_paths:
            return
        review_path = self._flagged_paths[self._current_index]
        entry = self._results.get(review_path, {})
        selected_delete = self._pending_delete(review_path, entry)
        pair_path = entry.get("pair_path")
        candidates = [review_path] + ([pair_path] if pair_path else [])
        to_mark = [
            candidate
            for candidate in candidates
            if candidate == selected_delete
            and self._is_marked_func
            and not self._is_marked_func(candidate)
        ]
        to_unmark = [
            candidate
            for candidate in candidates
            if candidate != selected_delete
            and self._is_marked_func
            and self._is_marked_func(candidate)
        ]
        if to_mark:
            self.mark_for_deletion_requested.emit(to_mark)
        if to_unmark:
            self.unmark_for_deletion_requested.emit(to_unmark)
        self._confirmed_reviews.add(review_path)
        next_index = next(
            (
                i
                for i in range(self._current_index + 1, len(self._flagged_paths))
                if self._flagged_paths[i] not in self._confirmed_reviews
            ),
            None,
        )
        if next_index is None:
            self._refresh_controls()
        else:
            self._navigate_to(next_index)

    def _on_apply_all(self) -> None:
        to_mark: list[str] = []
        to_unmark: list[str] = []
        for review_path in self._flagged_paths:
            entry = self._results.get(review_path, {})
            suggested_delete = review_path if entry.get("suggest_delete", True) else None
            pair_path = entry.get("pair_path")
            candidates = [review_path] + ([pair_path] if pair_path else [])
            for candidate in candidates:
                is_marked = bool(
                    self._is_marked_func and self._is_marked_func(candidate)
                )
                if candidate == suggested_delete and not is_marked:
                    to_mark.append(candidate)
                elif candidate != suggested_delete and is_marked:
                    to_unmark.append(candidate)
            self._pending_delete_by_review[review_path] = suggested_delete
            self._confirmed_reviews.add(review_path)
        if to_mark:
            self.mark_for_deletion_requested.emit(list(dict.fromkeys(to_mark)))
        if to_unmark:
            self.unmark_for_deletion_requested.emit(list(dict.fromkeys(to_unmark)))
        self._refresh_controls()

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
        page.setObjectName("workflowReviewPage")
        page_layout = QVBoxLayout(page)
        page_layout.setContentsMargins(0, 0, 0, 0)
        page_layout.setSpacing(0)

        self._review_header = WorkflowReviewHeader(
            step_number=2,
            title="Easy Delete",
            description=(
                "Review automated suggestions. Marking a photo is reversible; "
                "nothing is moved until you confirm deletion in Cull."
            ),
        )
        self._review_header.skip_button.clicked.connect(self._on_skip)
        page_layout.addWidget(self._review_header)

        content = QWidget()
        content_layout = QVBoxLayout(content)
        content_layout.setContentsMargins(12, 10, 12, 10)
        content_layout.setSpacing(8)

        self._summary_label = QLabel()
        self._summary_label.hide()

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
        right_layout.setContentsMargins(10, 0, 0, 0)
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
        self._pair_left_img.setCursor(Qt.CursorShape.PointingHandCursor)
        self._pair_left_img.clicked.connect(lambda: self._select_pair_side(0))
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
        self._pair_right_img.setCursor(Qt.CursorShape.PointingHandCursor)
        self._pair_right_img.clicked.connect(lambda: self._select_pair_side(1))
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

        self._state_banner = WorkflowStateBanner()
        right_layout.addWidget(self._state_banner)

        # Action bar
        action = QHBoxLayout()
        action.setSpacing(6)

        self._prev_btn = QPushButton("← Prev")
        self._prev_btn.setObjectName("workflowGhostButton")
        self._prev_btn.setFixedWidth(70)
        self._prev_btn.clicked.connect(self._on_prev)

        self._counter_label = QLabel("0 of 0")
        self._counter_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self._counter_label.setStyleSheet("font-size: 12px; color: #808080;")
        self._counter_label.setFixedWidth(72)

        self._next_btn = QPushButton("Next →")
        self._next_btn.setObjectName("workflowGhostButton")
        self._next_btn.setFixedWidth(70)
        self._next_btn.clicked.connect(self._on_next)

        self._decision_group = QButtonGroup(self)
        self._decision_group.setExclusive(True)

        self._keep_btn = QPushButton("Keep")
        self._keep_btn.setObjectName("workflowDecisionKeep")
        self._keep_btn.setCheckable(True)
        self._keep_btn.setMinimumWidth(90)
        self._keep_btn.clicked.connect(lambda: self._select_pair_side(0))
        self._decision_group.addButton(self._keep_btn)

        self._mark_btn = QPushButton("Trash")
        self._mark_btn.setObjectName("workflowDecisionTrash")
        self._mark_btn.setCheckable(True)
        self._mark_btn.setMinimumWidth(150)
        self._mark_btn.clicked.connect(lambda: self._select_pair_side(1))
        self._decision_group.addButton(self._mark_btn)

        self._apply_all_btn = QPushButton("Apply suggestions to all visible")
        self._apply_all_btn.setObjectName("workflowGhostButton")
        self._apply_all_btn.clicked.connect(self._on_apply_all)

        self._confirm_btn = QPushButton("Confirm  →")
        self._confirm_btn.setObjectName("workflowPrimaryButton")
        self._confirm_btn.setMinimumWidth(110)
        self._confirm_btn.clicked.connect(self._on_confirm)

        self._done_btn = QPushButton("Continue to Fix Rotation")
        self._done_btn.setObjectName("workflowPrimaryButton")
        self._done_btn.clicked.connect(self._on_done)

        action.addWidget(self._prev_btn)
        action.addWidget(self._counter_label)
        action.addWidget(self._next_btn)
        action.addStretch()
        action.addWidget(self._keep_btn)
        action.addWidget(self._mark_btn)
        action.addWidget(self._apply_all_btn)
        action.addWidget(self._confirm_btn)
        action.addWidget(self._done_btn)
        right_layout.addLayout(action)

        splitter.addWidget(left)
        splitter.addWidget(right)
        splitter.setStretchFactor(0, 1)
        splitter.setStretchFactor(1, 3)
        content_layout.addWidget(splitter, 1)
        page_layout.addWidget(content, 1)
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

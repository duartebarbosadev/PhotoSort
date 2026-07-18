import logging
import os
from collections.abc import Callable
from typing import override

from PyQt6.QtCore import QSize, Qt, pyqtSignal
from PyQt6.QtGui import QColor, QPixmap
from PyQt6.QtWidgets import (
    QCheckBox,
    QHBoxLayout,
    QLabel,
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
    WorkflowDecisionCard,
    WorkflowReviewListPanel,
    WorkflowStateBanner,
    install_workflow_shortcuts,
)
from ui.workflow_metadata import build_workflow_metadata_rows

logger = logging.getLogger(__name__)

_ISSUE_LABELS: dict[str, tuple] = {
    "blur": ("BLUR", "#FF6B6B"),
    "dark": ("DARK", "#4A90D9"),
    "white": ("WHITE", "#F5B700"),
    "duplicate": ("DUP", "#A78BFA"),
}
_CATEGORY_ORDER = ("exact_duplicate", "near_duplicate", "blur", "dark", "white")
_CATEGORY_NAMES: dict[str, str] = {
    "exact_duplicate": "Duplicates",
    "near_duplicate": "Near-duplicates",
    "blur": "Blur",
    "dark": "Dark",
    "white": "Bright",
}
_CATEGORY_HEADER_NAMES: dict[str, str] = {
    "exact_duplicate": "DUPLICATES",
    "near_duplicate": "NEAR-DUPLICATES",
    "blur": "BLURRY PHOTOS",
    "dark": "DARK PHOTOS",
    "white": "BRIGHT PHOTOS",
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

    apply_requested = pyqtSignal()
    skip_requested = pyqtSignal()
    mark_for_deletion_requested = pyqtSignal(list)
    unmark_for_deletion_requested = pyqtSignal(list)
    active_image_changed = pyqtSignal(str)

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self._results: dict[str, dict] = {}
        self._shown_results: dict[str, dict] | None = None
        self._flagged_paths: list[str] = []
        self._current_index: int = -1
        self._focused_path: str | None = None
        self._syncing_active_image = False
        self._category_counts: dict[str, int] = {}
        self._enabled_categories: dict[str, bool] = {}
        self._category_checkboxes: dict[str, QCheckBox] = {}
        self._list_row_by_path: dict[str, int] = {}
        self._updating_category_toggles = False
        self._is_marked_func: Callable[[str], bool] | None = None
        self._has_any_marked_func: Callable[[], bool] | None = None
        self._pending_delete_by_review: dict[str, str | None] = {}
        self._confirmed_reviews: set[str] = set()
        self._marks_before_confirmation: dict[str, set[str]] = {}
        self._info_visible = False
        self._image_pipeline = None
        self._exif_disk_cache = None
        self._metadata_cache: dict[str, list[tuple[str, str]]] = {}
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
                "info": self._toggle_info,
                "skip": self._on_skip,
            },
        )

    def set_is_marked_func(self, fn: Callable[[str], bool]) -> None:
        self._is_marked_func = fn

    def set_has_any_marked_func(self, fn: Callable[[], bool]) -> None:
        self._has_any_marked_func = fn

    def set_image_pipeline(self, pipeline) -> None:
        self._image_pipeline = pipeline

    def set_exif_disk_cache(self, cache) -> None:
        self._exif_disk_cache = cache
        self._metadata_cache.clear()

    def refresh_deletion_state(self) -> None:
        """Refresh staged decisions after another workflow changes shared state."""

        if not hasattr(self, "_items_list"):
            return
        self._discard_stale_confirmations()
        self._refresh_controls()

    def discard_pending_decisions(self) -> None:
        """Forget local review confirmations after shared Trash marks are cleared."""
        self._pending_delete_by_review.clear()
        self._confirmed_reviews.clear()
        self._marks_before_confirmation.clear()
        if hasattr(self, "_items_list"):
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
                self._marks_before_confirmation.pop(review_path, None)

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
        if self._shown_results is not None and results == self._shown_results:
            self.refresh_deletion_state()
            if self._flagged_paths:
                self._content_stack.setCurrentIndex(1)
                self._show_current()
                self._refresh_controls()
            else:
                self._content_stack.setCurrentIndex(2)
            self.setFocus(Qt.FocusReason.OtherFocusReason)
            return
        if results != self._results:
            self._pending_delete_by_review.clear()
            self._confirmed_reviews.clear()
            self._marks_before_confirmation.clear()
            self._metadata_cache.clear()
        self._shown_results = results
        self._results = results
        self._category_counts = self._build_category_counts(results)
        self._enabled_categories = {
            category: True
            for category in _CATEGORY_ORDER
            if self._category_counts.get(category, 0) > 0
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
            self._syncing_active_image = True
            try:
                self._navigate_to(0)
            finally:
                self._syncing_active_image = False
            self.setFocus(Qt.FocusReason.OtherFocusReason)
        else:
            self._content_stack.setCurrentIndex(2)

    # ------------------------------------------------------------------
    # Private helpers
    # ------------------------------------------------------------------

    def _build_ordered_paths(self, results: dict) -> list[str]:
        return self._build_ordered_paths_for_categories(
            results, self._enabled_categories_in_order()
        )

    @staticmethod
    def _entry_category(entry: dict) -> str:
        if entry.get("type") != "duplicate":
            return entry.get("type", "")
        if entry.get("duplicate_kind", "near") == "exact":
            return "exact_duplicate"
        return "near_duplicate"

    def _build_ordered_paths_for_categories(
        self, results: dict, categories: list[str] | tuple[str, ...]
    ) -> list[str]:
        ordered: list[str] = []
        seen: set = set()
        for category in categories:
            category_entries = [
                (path, entry)
                for path, entry in results.items()
                if self._entry_category(entry) == category
                and entry["suggest_delete"]
            ]
            for path, entry in category_entries:
                if path in seen:
                    continue
                ordered.append(path)
                seen.add(path)
                if entry.get("pair_path"):
                    seen.add(entry["pair_path"])
        return ordered

    def _build_category_counts(self, results: dict) -> dict[str, int]:
        counts: dict[str, int] = {}
        ordered_paths = self._build_ordered_paths_for_categories(
            results, _CATEGORY_ORDER
        )
        for path in ordered_paths:
            category = self._entry_category(results.get(path, {}))
            counts[category] = counts.get(category, 0) + 1
        return counts

    def _enabled_categories_in_order(self) -> list[str]:
        return [
            category
            for category in _CATEGORY_ORDER
            if self._enabled_categories.get(category, False)
        ]

    def _categories_with_counts(self) -> list[str]:
        return [
            category
            for category in _CATEGORY_ORDER
            if self._category_counts.get(category, 0) > 0
        ]

    def _refresh_category_controls(self) -> None:
        categories = self._categories_with_counts()
        while self._category_toggle_layout.count():
            item = self._category_toggle_layout.takeAt(0)
            widget = item.widget()
            if widget:
                widget.deleteLater()

        self._category_checkboxes = {}
        self._updating_category_toggles = True
        for index, category in enumerate(categories):
            count = self._category_counts[category]
            checkbox = QCheckBox(
                f"{_CATEGORY_NAMES.get(category, category.title())} ({count})"
            )
            checkbox.setObjectName("workflowReviewFilter")
            checkbox.setChecked(self._enabled_categories.get(category, True))
            checkbox.toggled.connect(
                lambda checked, category=category: self._on_category_toggled(
                    category, checked
                )
            )
            self._category_checkboxes[category] = checkbox
            self._category_toggle_layout.addWidget(checkbox, index // 2, index % 2)
        self._updating_category_toggles = False
        self._review_list_panel.filters.setVisible(bool(categories))

    def _item_text(self, path: str) -> str:
        entry = self._results.get(path, {})
        issue_type = entry.get("type", "")
        prefix = "✓  " if path in self._confirmed_reviews else ""
        if issue_type == "duplicate":
            pair = entry.get("pair_path", "")
            pair_name = os.path.basename(pair) if pair else ""
            return f"{prefix}{os.path.basename(path)}  ↔  {pair_name}"
        return f"{prefix}{os.path.basename(path)}"

    def _add_category_header(self, category: str, count: int) -> None:
        """Add a visual-only heading without making it part of queue navigation."""

        heading = _CATEGORY_HEADER_NAMES.get(
            category, category.upper()
        )
        header = QListWidgetItem(f"{heading}  ·  {count}")
        header.setFlags(Qt.ItemFlag.NoItemFlags)
        header.setForeground(QColor("#8795A1"))
        header.setBackground(QColor("#282D31"))
        font = header.font()
        font.setBold(True)
        font.setPointSizeF(8.0)
        header.setFont(font)
        header.setSizeHint(QSize(0, 30))
        self._items_list.addItem(header)

    def _populate_list(self) -> None:
        self._items_list.clear()
        self._list_row_by_path.clear()
        for category in self._enabled_categories_in_order():
            category_paths = [
                path
                for path in self._flagged_paths
                if self._entry_category(self._results.get(path, {})) == category
            ]
            if not category_paths:
                continue
            self._add_category_header(category, len(category_paths))
            for path in category_paths:
                item = QListWidgetItem(self._item_text(path))
                item.setData(Qt.ItemDataRole.UserRole, path)
                self._items_list.addItem(item)
                self._list_row_by_path[path] = self._items_list.row(item)

        self._review_list_panel.set_count(
            len(self._flagged_paths), sum(self._category_counts.values())
        )
        self._refresh_list_colors()

    def _refresh_list_colors(self) -> None:
        for i in range(self._items_list.count()):
            item = self._items_list.item(i)
            path = item.data(Qt.ItemDataRole.UserRole)
            if not path:
                continue
            item.setForeground(
                QColor(
                    _CONFIRMED_COLOR if path in self._confirmed_reviews else "#A9B7C6"
                )
            )
            item.setText(self._item_text(path))

    def _navigate_to(self, index: int) -> None:
        if not self._flagged_paths:
            return
        index = max(0, min(index, len(self._flagged_paths) - 1))
        self._current_index = index
        review_path = self._flagged_paths[index]
        pair_path = self._results.get(review_path, {}).get("pair_path")
        if self._focused_path not in {review_path, pair_path}:
            self._focused_path = review_path

        self._items_list.blockSignals(True)
        self._items_list.setCurrentRow(self._list_row_by_path[review_path])
        self._items_list.blockSignals(False)

        self._show_current()
        self._refresh_controls()
        if not self._syncing_active_image and self._focused_path:
            self.active_image_changed.emit(self._focused_path)

    def focus_image(self, path: str) -> bool:
        """Navigate to a matching review or duplicate side without changing decisions."""

        for index, review_path in enumerate(self._flagged_paths):
            pair_path = self._results.get(review_path, {}).get("pair_path")
            if path not in {review_path, pair_path}:
                continue
            self._focused_path = path
            self._syncing_active_image = True
            try:
                self._navigate_to(index)
            finally:
                self._syncing_active_image = False
            return True
        return False

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
        classification = (
            "Exact duplicate" if duplicate_kind == "exact" else "Near-duplicate"
        )
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
            self._set_choice_card(
                self._pair_left_card,
                self._pair_left_hdr,
                path=path,
                entry=entry,
                filename=filename,
                selected_for_delete=left_is_delete,
                suggested_for_delete=True,
                confirmed=confirmed,
                slot=1,
            )
            self._set_choice_card(
                self._pair_right_card,
                self._pair_right_hdr,
                path=pair_path,
                entry=entry,
                filename=pair_name,
                selected_for_delete=not left_is_delete,
                suggested_for_delete=False,
                confirmed=confirmed,
                slot=2,
            )
            self._pair_left_card.set_focused(self._focused_path == path)
            self._pair_right_card.set_focused(self._focused_path == pair_path)
        else:
            delete_selected = selected_delete == path
            self._set_choice_card(
                self._single_card,
                self._single_hdr,
                path=path,
                entry=entry,
                filename=filename,
                selected_for_delete=delete_selected,
                suggested_for_delete=None,
                confirmed=confirmed,
                slot=1,
            )
            self._single_card.set_focused(self._focused_path == path)

    def _set_choice_card(
        self,
        card: WorkflowDecisionCard,
        state_label: QLabel,
        *,
        path: str,
        entry: dict,
        filename: str,
        selected_for_delete: bool,
        suggested_for_delete: bool | None,
        confirmed: bool,
        slot: int,
    ) -> None:
        if selected_for_delete:
            state = (
                "MARKED FOR TRASH · staged"
                if confirmed
                else "SELECTED FOR TRASH · not confirmed"
            )
            color = "#FF7B86"
            border = "#E53935"
        else:
            state = "KEEP" if confirmed else "SELECTED TO KEEP · not confirmed"
            color = "#66BB6A"
            border = "#2E7D32"
        state_label.setText(state)
        display_parts = [filename]
        if suggested_for_delete is not None:
            if suggested_for_delete:
                suggestion = "Suggested for trash"
                suggestion_reason = entry.get("delete_suggestion_reason")
            else:
                suggestion = "Suggested to keep"
                suggestion_reason = entry.get("keep_suggestion_reason")
            display_parts.append(suggestion)
            if suggestion_reason:
                display_parts.append(suggestion_reason)
        card.set_decision(
            filename=" · ".join(display_parts),
            state=state,
            state_color=color,
            border_color=border,
            hint=f"Click image/card or press {slot} to change the choice · I toggles details",
        )
        card.set_details(self._metadata_rows_for_path(path))

    def _metadata_rows_for_path(self, path: str) -> list[tuple[str, str]]:
        if path not in self._metadata_cache:
            try:
                rows = build_workflow_metadata_rows(path, self._exif_disk_cache)
            except Exception:
                logger.debug("Cached EXIF lookup failed for %s", path, exc_info=True)
                rows = [("Metadata", "No EXIF details available")]
            self._metadata_cache[path] = rows
        return self._metadata_cache[path]

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
                pixmap = self._image_pipeline.get_cached_review_qpixmap(path)
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
            self._confirm_btn.setEnabled(False)
            self._apply_all_btn.setEnabled(False)
            return
        self._confirm_btn.setEnabled(True)
        self._apply_all_btn.setEnabled(True)
        self._counter_label.setText(f"{self._current_index + 1} of {total}")
        self._prev_btn.setEnabled(self._current_index > 0)
        self._next_btn.setEnabled(self._current_index < total - 1)

        path = self._flagged_paths[self._current_index]
        self._confirm_btn.setText(
            "Cancel confirmation" if path in self._confirmed_reviews else "Confirm  →"
        )
        self._update_decision_presentation(path, self._results.get(path, {}))

        self._refresh_list_colors()

    # ------------------------------------------------------------------
    # Event handlers
    # ------------------------------------------------------------------

    def _on_item_clicked(self, item: QListWidgetItem) -> None:
        path = item.data(Qt.ItemDataRole.UserRole)
        if not path:
            return
        index = self._flagged_paths.index(path)
        if index != self._current_index:
            self._navigate_to(index)

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
            self._publish_active_image(path)
            self._select_current_delete(path if side == 1 else None)
            return
        self._publish_active_image(path if side == 0 else pair_path)
        self._select_current_delete(path if side == 0 else pair_path)

    def _toggle_single_choice(self) -> None:
        if self._current_index < 0 or not self._flagged_paths:
            return
        review_path = self._flagged_paths[self._current_index]
        self._publish_active_image(review_path)
        entry = self._results.get(review_path, {})
        selected_delete = self._pending_delete(review_path, entry)
        self._select_current_delete(
            None if selected_delete == review_path else review_path
        )

    def _publish_active_image(self, path: str) -> None:
        self._focused_path = path
        if not self._syncing_active_image:
            self.active_image_changed.emit(path)

    def _toggle_info(self) -> None:
        self._info_visible = not self._info_visible
        for card in (self._single_card, self._pair_left_card, self._pair_right_card):
            card.set_details_visible(self._info_visible)

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
        pair_path = entry.get("pair_path")
        candidates = [review_path] + ([pair_path] if pair_path else [])
        if review_path in self._confirmed_reviews:
            self._cancel_confirmation(review_path, candidates)
            return

        selected_delete = self._pending_delete(review_path, entry)
        self._marks_before_confirmation[review_path] = {
            candidate
            for candidate in candidates
            if self._is_marked_func and self._is_marked_func(candidate)
        }
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

    def _cancel_confirmation(
        self, review_path: str, candidates: list[str]
    ) -> None:
        """Undo a confirmed review and restore its prior shared deletion marks."""

        prior_marks = self._marks_before_confirmation.pop(review_path, set())
        to_mark = [
            candidate
            for candidate in candidates
            if candidate in prior_marks
            and self._is_marked_func
            and not self._is_marked_func(candidate)
        ]
        to_unmark = [
            candidate
            for candidate in candidates
            if candidate not in prior_marks
            and self._is_marked_func
            and self._is_marked_func(candidate)
        ]
        self._confirmed_reviews.discard(review_path)
        if to_mark:
            self.mark_for_deletion_requested.emit(to_mark)
        if to_unmark:
            self.unmark_for_deletion_requested.emit(to_unmark)
        self._refresh_controls()

    def _on_apply_all(self) -> None:
        to_mark: list[str] = []
        to_unmark: list[str] = []
        for review_path in self._flagged_paths:
            entry = self._results.get(review_path, {})
            suggested_delete = (
                review_path if entry.get("suggest_delete", True) else None
            )
            pair_path = entry.get("pair_path")
            candidates = [review_path] + ([pair_path] if pair_path else [])
            if review_path not in self._confirmed_reviews:
                self._marks_before_confirmation[review_path] = {
                    candidate
                    for candidate in candidates
                    if self._is_marked_func and self._is_marked_func(candidate)
                }
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

    def _on_category_toggled(self, category: str, checked: bool) -> None:
        if self._updating_category_toggles:
            return
        self._enabled_categories[category] = checked
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

    def _on_apply(self) -> None:
        self.apply_requested.emit()

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

        content = QWidget()
        content_layout = QVBoxLayout(content)
        content_layout.setContentsMargins(12, 10, 12, 10)
        content_layout.setSpacing(8)

        # Main split: list | viewer
        splitter = QSplitter(Qt.Orientation.Horizontal)
        splitter.setHandleWidth(4)
        splitter.setChildrenCollapsible(False)

        self._review_list_panel = WorkflowReviewListPanel(
            bulk_action_text="Confirm visible"
        )
        self._category_toggle_layout = self._review_list_panel.filters_layout
        self._items_list = self._review_list_panel.list_widget
        self._items_list.itemClicked.connect(self._on_item_clicked)
        self._apply_all_btn = self._review_list_panel.bulk_button
        self._apply_all_btn.setToolTip(
            "Confirms suggestions only for currently visible categories. For example, "
            "if Duplicates is enabled and Blurry is disabled, only duplicate "
            "suggestions are confirmed. You can still review or revise them afterward."
        )
        self._apply_all_btn.clicked.connect(self._on_apply_all)

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
        self._single_img.setCursor(Qt.CursorShape.PointingHandCursor)
        self._single_img.clicked.connect(self._toggle_single_choice)
        sl.addWidget(self._single_img, 1)
        self._single_card = WorkflowDecisionCard(1)
        self._single_card.set_details_visible(self._info_visible)
        self._single_card.activated.connect(self._toggle_single_choice)
        self._single_hdr = self._single_card.state_label
        sl.addWidget(self._single_card)

        # Pair image view
        pair = QWidget()
        pl = QVBoxLayout(pair)
        pl.setContentsMargins(0, 0, 0, 0)
        pair_splitter = QSplitter(Qt.Orientation.Horizontal)

        lp = QWidget()
        ll = QVBoxLayout(lp)
        ll.setContentsMargins(0, 0, 4, 0)
        ll.setSpacing(3)
        self._pair_left_img = _ScaledImageLabel()
        self._pair_left_img.setCursor(Qt.CursorShape.PointingHandCursor)
        self._pair_left_img.clicked.connect(lambda: self._select_pair_side(0))
        ll.addWidget(self._pair_left_img, 1)
        self._pair_left_card = WorkflowDecisionCard(1, filename_in_header=True)
        self._pair_left_card.set_details_visible(self._info_visible)
        self._pair_left_card.activated.connect(lambda: self._select_pair_side(0))
        self._pair_left_hdr = self._pair_left_card.state_label
        ll.addWidget(self._pair_left_card)

        rp = QWidget()
        rl = QVBoxLayout(rp)
        rl.setContentsMargins(4, 0, 0, 0)
        rl.setSpacing(3)
        self._pair_right_img = _ScaledImageLabel()
        self._pair_right_img.setCursor(Qt.CursorShape.PointingHandCursor)
        self._pair_right_img.clicked.connect(lambda: self._select_pair_side(1))
        rl.addWidget(self._pair_right_img, 1)
        self._pair_right_card = WorkflowDecisionCard(2, filename_in_header=True)
        self._pair_right_card.set_details_visible(self._info_visible)
        self._pair_right_card.activated.connect(lambda: self._select_pair_side(1))
        self._pair_right_hdr = self._pair_right_card.state_label
        rl.addWidget(self._pair_right_card)

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
        self._action_layout = action
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

        self._confirm_btn = QPushButton("Confirm  →")
        self._confirm_btn.setObjectName("workflowPrimaryButton")
        self._confirm_btn.setMinimumWidth(110)
        self._confirm_btn.clicked.connect(self._on_confirm)

        self._apply_btn = QPushButton("Apply")
        self._apply_btn.setObjectName("workflowPrimaryButton")
        self._apply_btn.setToolTip(
            "Review and apply pending changes without leaving Easy Delete."
        )
        self._apply_btn.clicked.connect(self._on_apply)

        action.addWidget(self._prev_btn)
        action.addWidget(self._counter_label)
        action.addWidget(self._next_btn)
        action.addWidget(self._confirm_btn)
        action.addStretch()
        action.addWidget(self._apply_btn)
        right_layout.addLayout(action)

        splitter.addWidget(self._review_list_panel)
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

        btn = QPushButton("Apply")
        btn.setObjectName("acceptButton")
        btn.setFixedWidth(230)
        btn.setToolTip("Review and apply pending changes without leaving Easy Delete.")
        btn.clicked.connect(self._on_apply)

        layout.addWidget(lbl)
        layout.addWidget(btn, alignment=Qt.AlignmentFlag.AlignCenter)
        return page

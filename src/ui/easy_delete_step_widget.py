import logging
import os
from copy import deepcopy
from collections.abc import Callable

from PyQt6.QtCore import QSize, Qt, pyqtSignal
from PyQt6.QtGui import QColor, QPixmap
from PyQt6.QtWidgets import (
    QCheckBox,
    QHBoxLayout,
    QLabel,
    QListWidgetItem,
    QProgressBar,
    QPushButton,
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
    show_confirm_or_reset_notice,
)
from ui.workflow_metadata import build_workflow_metadata_rows
from ui.advanced_image_viewer import SynchronizedImageViewer
from ui.controllers.image_inspection_controller import InspectionImageSpec

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


class EasyDeleteStepWidget(QWidget):
    """Step 2: Review and mark obviously bad / duplicate images for deletion."""

    apply_requested = pyqtSignal()
    skip_requested = pyqtSignal()
    mark_for_deletion_requested = pyqtSignal(list)
    unmark_for_deletion_requested = pyqtSignal(list)
    deletion_state_requested = pyqtSignal(dict)
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
        self._pending_keep_by_review: dict[str, dict[str, bool]] = {}
        self._confirmed_reviews: set[str] = set()
        self._confirmation_order: list[str] = []
        self._marks_before_confirmation: dict[str, set[str]] = {}
        self._publishing_confirmation = False
        self._info_visible = False
        self._image_pipeline = None
        self._exif_disk_cache = None
        self._metadata_cache: dict[str, list[tuple[str, str]]] = {}
        self._visible_image_paths: tuple[str, ...] = ()
        self._fallback_detail_requested = False
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
                "reset": self.reset_current_to_default,
                "reset_all": self.reset_all_to_default,
                "apply": self._on_apply,
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
        self._pending_keep_by_review.clear()
        self._confirmed_reviews.clear()
        self._confirmation_order.clear()
        self._marks_before_confirmation.clear()
        if hasattr(self, "_items_list"):
            self._refresh_controls()

    def has_unconfirmed_changes(self) -> bool:
        if self._current_index < 0 or not self._flagged_paths:
            return False
        review_path = self._flagged_paths[self._current_index]
        if review_path in self._confirmed_reviews:
            return False
        entry = self._results.get(review_path, {})
        return self._pending_keep_state(review_path, entry) != self._default_keep_state(
            review_path, entry
        )

    def show_confirm_or_reset_required(self) -> None:
        show_confirm_or_reset_notice(
            self,
            confirm=self._on_confirm,
            reset=self.reset_current_to_default,
            reset_all=self.reset_all_to_default,
        )

    def reset_current_to_default(self) -> None:
        if self._current_index < 0 or not self._flagged_paths:
            return
        review_path = self._flagged_paths[self._current_index]
        entry = self._results.get(review_path, {})
        candidates = self._review_candidates(review_path, entry)
        if review_path in self._confirmed_reviews:
            self._cancel_confirmation(review_path, candidates, refresh=False)
        self._pending_keep_by_review[review_path] = self._default_keep_state(
            review_path, entry
        )
        self._refresh_controls()

    def reset_all_to_default(self) -> None:
        """Unconfirm every review and restore all detector suggestions."""

        restored_mark_state: dict[str, bool] = {}
        ordered_confirmations = list(reversed(self._confirmation_order))
        ordered_confirmations.extend(
            review_path
            for review_path in self._confirmed_reviews
            if review_path not in self._confirmation_order
        )
        for review_path in ordered_confirmations:
            entry = self._results.get(review_path, {})
            prior_marks = self._marks_before_confirmation.get(review_path, set())
            restored_mark_state.update(
                {
                    candidate: candidate in prior_marks
                    for candidate in self._review_candidates(review_path, entry)
                }
            )
        self._confirmed_reviews.clear()
        self._confirmation_order.clear()
        self._marks_before_confirmation.clear()
        for review_path, entry in self._results.items():
            self._pending_keep_by_review[review_path] = self._default_keep_state(
                review_path, entry
            )
        if restored_mark_state:
            self._publish_mark_state(restored_mark_state)
        self._refresh_controls()

    def _allow_review_departure(self) -> bool:
        if not self.has_unconfirmed_changes():
            return True
        self.show_confirm_or_reset_required()
        return False

    def _discard_stale_confirmations(self) -> None:
        if self._publishing_confirmation or not self._is_marked_func:
            return
        for review_path in list(self._confirmed_reviews):
            entry = self._results.get(review_path, {})
            keep_by_path = self._pending_keep_state(review_path, entry)
            candidates = self._review_candidates(review_path, entry)
            matches_shared_state = all(
                self._is_marked_func(candidate)
                == (not keep_by_path.get(candidate, True))
                for candidate in candidates
            )
            if not matches_shared_state:
                self._confirmed_reviews.discard(review_path)
                if review_path in self._confirmation_order:
                    self._confirmation_order.remove(review_path)
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
        self._pending_keep_by_review.clear()
        self._confirmed_reviews.clear()
        self._confirmation_order.clear()
        self._marks_before_confirmation.clear()
        self._metadata_cache.clear()
        # Keep a value snapshot. AppState owns and mutates the live result mapping
        # when files move, so retaining the same object here would hide changes.
        self._shown_results = deepcopy(results)
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
            self._clear_viewer_images()
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
                if self._entry_category(entry) == category and entry["suggest_delete"]
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
        confirmed = path in self._confirmed_reviews
        prefix = "✓  " if confirmed else ""
        if issue_type == "duplicate":
            pair = entry.get("pair_path", "")
            pair_name = os.path.basename(pair) if pair else ""
            summary = ""
            if confirmed:
                keep_by_path = self._pending_keep_state(path, entry)
                kept_count = sum(
                    keep_by_path.get(candidate, True)
                    for candidate in self._review_candidates(path, entry)
                )
                summary = f"\nComplete · {kept_count} kept"
            return f"{prefix}{os.path.basename(path)}  ↔  {pair_name}{summary}"
        if confirmed:
            kept = self._pending_keep_state(path, entry).get(path, True)
            return f"{prefix}{os.path.basename(path)}\nComplete · {'Keep' if kept else 'Trash'}"
        return os.path.basename(path)

    def _add_category_header(self, category: str, count: int) -> None:
        """Add a visual-only heading without making it part of queue navigation."""

        heading = _CATEGORY_HEADER_NAMES.get(category, category.upper())
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
        if index != self._current_index and not self._allow_review_departure():
            return
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
            if index != self._current_index and not self._allow_review_departure():
                return False
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
        self._set_viewer_images([path])
        self._decision_stack.setCurrentIndex(0)
        issue_type = entry.get("type", "")
        label, color = _ISSUE_LABELS.get(issue_type, ("ISSUE", "#888"))
        reason = entry.get("reason", "")
        self._issue_label.setText(f"<b style='color:{color}'>[{label}]</b>  {reason}")
        self._suggestion_label.hide()
        self._update_decision_presentation(path, entry)
        logger.info(f"Showing [{label}] {os.path.basename(path)} — {reason}")

    def _show_pair(self, path: str, pair_path: str, entry: dict) -> None:
        self._set_viewer_images([path, pair_path])
        self._decision_stack.setCurrentIndex(1)

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
        keep_by_path = self._pending_keep_state(path, entry)
        confirmed = path in self._confirmed_reviews
        candidates = self._review_candidates(path, entry)
        kept_count = sum(keep_by_path.get(candidate, True) for candidate in candidates)
        if self.has_unconfirmed_changes():
            self._state_banner.set_state(
                "Set each photo, then confirm",
                "Your Keep/Trash changes are local until you press Confirm.",
                tone="warning",
            )
        elif confirmed:
            self._state_banner.set_state(
                "Decisions confirmed",
                (
                    f"{kept_count} of {len(candidates)} photos kept. Trash choices are "
                    "marked for deletion, but no file has been moved or deleted."
                ),
                tone="success" if kept_count else "warning",
            )
        else:
            self._state_banner.set_state(
                "Set each photo, then confirm",
                "Toggle each Keep/Trash state independently. Nothing changes until you press Confirm.",
                tone="warning",
            )

        if entry.get("type") == "duplicate" and entry.get("pair_path"):
            pair_path = entry["pair_path"]
            pair_name = os.path.basename(pair_path)
            self._set_choice_card(
                self._pair_left_card,
                self._pair_left_hdr,
                path=path,
                entry=entry,
                filename=filename,
                selected_for_delete=not keep_by_path.get(path, True),
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
                selected_for_delete=not keep_by_path.get(pair_path, True),
                suggested_for_delete=False,
                confirmed=confirmed,
                slot=2,
            )
            self._pair_left_card.set_focused(self._focused_path == path)
            self._pair_right_card.set_focused(self._focused_path == pair_path)
        else:
            self._set_choice_card(
                self._single_card,
                self._single_hdr,
                path=path,
                entry=entry,
                filename=filename,
                selected_for_delete=not keep_by_path.get(path, True),
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
            state = "TRASH"
            color = "#FF7B86"
            border = "#E53935"
        else:
            state = "KEEP"
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
            hint=(
                f"Click image/card or press {slot} to toggle only this photo "
                "between Keep and Trash · I toggles details"
            ),
        )
        card.set_details(self._metadata_rows_for_path(path))

    def _metadata_rows_for_path(self, path: str) -> list[tuple[str, str]]:
        if path not in self._metadata_cache:
            try:
                rows = build_workflow_metadata_rows(path, self._exif_disk_cache)
            except Exception:
                logger.debug("Cached EXIF lookup failed for %s", path, exc_info=True)
                rows = [("Metadata", "No EXIF details available")]
            rows = [("Path", path), *rows]
            self._metadata_cache[path] = rows
        return self._metadata_cache[path]

    @staticmethod
    def _review_candidates(path: str, entry: dict) -> list[str]:
        pair_path = entry.get("pair_path")
        return [path] + ([pair_path] if pair_path else [])

    def _default_keep_state(self, path: str, entry: dict) -> dict[str, bool]:
        candidates = self._review_candidates(path, entry)
        return {
            candidate: candidate != path or not entry.get("suggest_delete", True)
            for candidate in candidates
        }

    def _pending_keep_state(self, path: str, entry: dict) -> dict[str, bool]:
        if path not in self._pending_keep_by_review:
            self._pending_keep_by_review[path] = self._default_keep_state(path, entry)
        return self._pending_keep_by_review[path]

    def _set_viewer_images(self, paths: list[str]) -> None:
        visible_paths = tuple(paths)
        if visible_paths == self._visible_image_paths and all(
            self._sync_viewer.displays_path(path) for path in visible_paths
        ):
            return
        if visible_paths != self._visible_image_paths:
            self._fallback_detail_requested = False
        self._visible_image_paths = visible_paths
        activate = getattr(self.window(), "activate_image_inspection", None)
        if callable(activate):
            activate(
                self._sync_viewer,
                [InspectionImageSpec(path=path) for path in paths],
            )
        else:
            images_data = [
                {"path": path, "pixmap": self._load_pixmap(path), "rating": 0}
                for path in paths
            ]
            self._sync_viewer.set_images_data(images_data)
            request = getattr(self.window(), "request_interactive_previews", None)
            if paths and callable(request):
                request(paths)
        for viewer in self._sync_viewer.image_viewers:
            viewer.control_bar.hide()

    def _load_pixmap(self, path: str) -> QPixmap | None:
        try:
            if self._image_pipeline:
                pixmap, _ = self._image_pipeline.get_immediate_review_qpixmap(path)
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
        if path not in self._visible_image_paths or self._image_pipeline is None:
            return
        pixmap = self._image_pipeline.get_cached_preview_qpixmap(path, memory_only=True)
        if pixmap is not None and not pixmap.isNull():
            self._sync_viewer.update_image_pixmap(path, pixmap, preserve_view=True)

    def _request_detail_images(self, reason: str) -> None:
        """Compatibility path for embedding the widget without the app controller."""
        if callable(getattr(self.window(), "activate_image_inspection", None)):
            return
        if self._fallback_detail_requested:
            return
        request = getattr(self.window(), "request_interactive_details", None)
        if callable(request) and self._visible_image_paths:
            self._fallback_detail_requested = True
            request(list(self._visible_image_paths))

    def handle_detail_ready(self, path: str, pixmap: QPixmap) -> None:
        if path in self._visible_image_paths and not pixmap.isNull():
            self._sync_viewer.update_image_pixmap(path, pixmap, preserve_view=True)

    def handle_detail_failed(self, path: str) -> None:
        if path in self._visible_image_paths:
            self._fallback_detail_requested = False

    def _refresh_controls(self) -> None:
        total = len(self._flagged_paths)
        if total == 0:
            self._counter_label.setText("0 of 0")
            self._prev_btn.setEnabled(False)
            self._next_btn.setEnabled(False)
            self._confirm_btn.setEnabled(False)
            self._reset_btn.setEnabled(False)
            self._apply_all_btn.setEnabled(False)
            return
        self._confirm_btn.setEnabled(True)
        path = self._flagged_paths[self._current_index]
        self._reset_btn.setEnabled(
            self.has_unconfirmed_changes() or path in self._confirmed_reviews
        )
        self._apply_all_btn.setEnabled(True)
        self._counter_label.setText(f"{self._current_index + 1} of {total}")
        self._prev_btn.setEnabled(self._current_index > 0)
        self._next_btn.setEnabled(self._current_index < total - 1)

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
        if index != self._current_index and not self._allow_review_departure():
            self._items_list.blockSignals(True)
            self._items_list.setCurrentRow(
                self._list_row_by_path[self._flagged_paths[self._current_index]]
            )
            self._items_list.blockSignals(False)
            return
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
            if side == 0:
                self._publish_active_image(path)
                self._toggle_current_path(path)
            return
        selected_path = path if side == 0 else pair_path
        self._publish_active_image(selected_path)
        self._toggle_current_path(selected_path)

    def _on_viewer_image_clicked(self, _slot: int, path: str) -> None:
        if len(self._visible_image_paths) == 1:
            self._toggle_single_choice()
            return
        try:
            side = self._visible_image_paths.index(path)
        except ValueError:
            return
        self._select_pair_side(side)

    def _toggle_single_choice(self) -> None:
        if self._current_index < 0 or not self._flagged_paths:
            return
        review_path = self._flagged_paths[self._current_index]
        self._publish_active_image(review_path)
        self._toggle_current_path(review_path)

    def _publish_active_image(self, path: str) -> None:
        self._focused_path = path
        if not self._syncing_active_image:
            self.active_image_changed.emit(path)

    def _toggle_info(self) -> None:
        self._info_visible = not self._info_visible
        for card in (self._single_card, self._pair_left_card, self._pair_right_card):
            card.set_details_visible(self._info_visible)

    def _toggle_current_path(self, selected_path: str) -> None:
        if self._current_index < 0 or not self._flagged_paths:
            return
        review_path = self._flagged_paths[self._current_index]
        entry = self._results.get(review_path, {})
        candidates = self._review_candidates(review_path, entry)
        if selected_path not in candidates:
            return
        if review_path in self._confirmed_reviews:
            self._cancel_confirmation(review_path, candidates, refresh=False)
        keep_by_path = self._pending_keep_state(review_path, entry)
        keep_by_path[selected_path] = not keep_by_path.get(selected_path, True)
        self._refresh_controls()

    def _publish_mark_state(self, mark_state: dict[str, bool]) -> None:
        if self.receivers(self.deletion_state_requested):
            self._publishing_confirmation = True
            try:
                self.deletion_state_requested.emit(mark_state)
            finally:
                self._publishing_confirmation = False
            return
        to_mark = [
            path
            for path, marked in mark_state.items()
            if marked and (not self._is_marked_func or not self._is_marked_func(path))
        ]
        to_unmark = [
            path
            for path, marked in mark_state.items()
            if not marked and self._is_marked_func and self._is_marked_func(path)
        ]
        self._publishing_confirmation = True
        try:
            if to_mark:
                self.mark_for_deletion_requested.emit(to_mark)
            if to_unmark:
                self.unmark_for_deletion_requested.emit(to_unmark)
        finally:
            self._publishing_confirmation = False

    def _on_confirm(self) -> None:
        if self._current_index < 0 or not self._flagged_paths:
            return
        review_path = self._flagged_paths[self._current_index]
        entry = self._results.get(review_path, {})
        candidates = self._review_candidates(review_path, entry)
        if review_path in self._confirmed_reviews:
            self._cancel_confirmation(review_path, candidates)
            return

        keep_by_path = self._pending_keep_state(review_path, entry)
        self._marks_before_confirmation[review_path] = {
            candidate
            for candidate in candidates
            if self._is_marked_func and self._is_marked_func(candidate)
        }
        self._publish_mark_state(
            {
                candidate: not keep_by_path.get(candidate, True)
                for candidate in candidates
            }
        )
        self._confirmed_reviews.add(review_path)
        self._confirmation_order.append(review_path)
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
        self, review_path: str, candidates: list[str], *, refresh: bool = True
    ) -> None:
        """Undo a confirmed review and restore its prior shared deletion marks."""

        prior_marks = self._marks_before_confirmation.pop(review_path, set())
        self._confirmed_reviews.discard(review_path)
        if review_path in self._confirmation_order:
            self._confirmation_order.remove(review_path)
        self._publish_mark_state(
            {candidate: candidate in prior_marks for candidate in candidates}
        )
        if refresh:
            self._refresh_controls()

    def _on_apply_all(self) -> None:
        if not self._allow_review_departure():
            return
        desired_mark_state: dict[str, bool] = {}
        for review_path in self._flagged_paths:
            entry = self._results.get(review_path, {})
            candidates = self._review_candidates(review_path, entry)
            if review_path not in self._confirmed_reviews:
                self._marks_before_confirmation[review_path] = {
                    candidate
                    for candidate in candidates
                    if self._is_marked_func and self._is_marked_func(candidate)
                }
            keep_by_path = self._default_keep_state(review_path, entry)
            self._pending_keep_by_review[review_path] = keep_by_path
            desired_mark_state.update(
                {
                    candidate: not keep_by_path.get(candidate, True)
                    for candidate in candidates
                }
            )
            self._confirmed_reviews.add(review_path)
            if review_path not in self._confirmation_order:
                self._confirmation_order.append(review_path)
        self._publish_mark_state(desired_mark_state)
        self._refresh_controls()

    def _on_category_toggled(self, category: str, checked: bool) -> None:
        if self._updating_category_toggles:
            return
        if not self._allow_review_departure():
            self._updating_category_toggles = True
            try:
                self._category_checkboxes[category].setChecked(
                    self._enabled_categories.get(category, False)
                )
            finally:
                self._updating_category_toggles = False
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
        self._clear_viewer_images()
        self._decision_stack.setCurrentIndex(0)
        self._issue_label.setText(
            "No enabled categories. Re-enable a category on the left to review those images."
        )
        self._suggestion_label.hide()
        self._refresh_controls()

    def _clear_viewer_images(self) -> None:
        clear_inspection = getattr(self.window(), "clear_image_inspection", None)
        if callable(clear_inspection):
            clear_inspection(self._sync_viewer)
        self._visible_image_paths = ()
        self._fallback_detail_requested = False
        self._sync_viewer.clear()

    def _on_apply(self) -> None:
        if not self._allow_review_departure():
            return
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
            "Confirm the detector's independent Keep/Trash suggestions only for "
            "currently visible categories. For example, if Duplicates is enabled and "
            "Blurry is disabled, only duplicate suggestions are confirmed. You can "
            "still review or revise each photo afterward."
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

        self._sync_viewer = SynchronizedImageViewer()
        self._sync_viewer.configure_toolbar(show_view_modes=False)
        self._sync_viewer.imageClicked.connect(self._on_viewer_image_clicked)
        self._sync_viewer.detail_requested.connect(self._request_detail_images)
        right_layout.addWidget(self._sync_viewer, 1)

        # Decision cards remain separate from the reusable image viewer.
        self._decision_stack = QStackedWidget()
        single = QWidget()
        sl = QVBoxLayout(single)
        sl.setContentsMargins(0, 0, 0, 0)
        self._single_card = WorkflowDecisionCard(1)
        self._single_card.set_details_visible(self._info_visible)
        self._single_card.activated.connect(self._toggle_single_choice)
        self._single_hdr = self._single_card.state_label
        sl.addWidget(self._single_card)

        pair = QWidget()
        pl = QHBoxLayout(pair)
        pl.setContentsMargins(0, 0, 0, 0)
        pl.setSpacing(8)
        self._pair_left_card = WorkflowDecisionCard(1, filename_in_header=True)
        self._pair_left_card.set_details_visible(self._info_visible)
        self._pair_left_card.activated.connect(lambda: self._select_pair_side(0))
        self._pair_left_hdr = self._pair_left_card.state_label
        self._pair_right_card = WorkflowDecisionCard(2, filename_in_header=True)
        self._pair_right_card.set_details_visible(self._info_visible)
        self._pair_right_card.activated.connect(lambda: self._select_pair_side(1))
        self._pair_right_hdr = self._pair_right_card.state_label
        pl.addWidget(self._pair_left_card, 1)
        pl.addWidget(self._pair_right_card, 1)

        self._decision_stack.addWidget(single)
        self._decision_stack.addWidget(pair)
        right_layout.addWidget(self._decision_stack)

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

        self._reset_btn = QPushButton("Reset default")
        self._reset_btn.setObjectName("workflowGhostButton")
        self._reset_btn.setToolTip(
            "Reset this review (R), or reset every review (Shift+R)"
        )
        self._reset_btn.clicked.connect(self.reset_current_to_default)

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
        action.addWidget(self._reset_btn)
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

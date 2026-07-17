import logging
import os
from fractions import Fraction
from collections.abc import Callable

from PyQt6.QtCore import Qt, pyqtSignal
from PyQt6.QtGui import QColor
from PyQt6.QtWidgets import (
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

from core.metadata_processor import (
    DATE_TAGS_PREFERENCE,
    MetadataProcessor,
    _parse_exif_date,
)
from core.best_photo_finder.payloads import PickBestClusterResult, PickBestResults
from ui.advanced_image_viewer import SynchronizedImageViewer
from ui.workflow_review_components import (
    PICK_BEST_SHORTCUTS,
    WorkflowDecisionCard,
    WorkflowReviewListPanel,
    WorkflowStateBanner,
    install_workflow_shortcuts,
)
import contextlib

logger = logging.getLogger(__name__)

WINNER_BORDER_COLOR = "#F5B700"
MARKED_BORDER_COLOR = "#E53935"
KEEP_BORDER_COLOR = "#66BB6A"
FOCUSED_BORDER_COLOR = "#4FC3F7"
CARD_BG = "#20252C"
CARD_BG_WINNER = "#2C2616"
CARD_BORDER_COLOR = "#3A434C"


def _first_present(metadata: dict, *keys: str):
    for key in keys:
        value = metadata.get(key)
        if value not in (None, "", "None"):
            return value
    return None


def _fraction_text(value: object) -> str | None:
    if value in (None, ""):
        return None
    text = str(value).strip()
    try:
        if "/" in text:
            frac = Fraction(text)
            if frac >= 1:
                return f"{float(frac):.1f}s"
            return f"1/{round(1 / float(frac))}s"
        numeric = float(text)
    except TypeError, ValueError, ZeroDivisionError:
        return text
    if numeric >= 1:
        return f"{numeric:.1f}s"
    if numeric <= 0:
        return text
    return f"1/{round(1 / numeric)}s"


def _float_text(
    value: object, prefix: str = "", suffix: str = "", digits: int = 1
) -> str | None:
    if value in (None, ""):
        return None
    try:
        return f"{prefix}{float(value):.{digits}f}{suffix}"
    except TypeError, ValueError:
        return f"{prefix}{value}{suffix}"


def _format_capture_date(metadata: dict) -> str | None:
    for key in DATE_TAGS_PREFERENCE:
        raw_value = metadata.get(key)
        if raw_value in (None, "", "None"):
            continue
        parsed = _parse_exif_date(str(raw_value))
        if parsed is not None:
            if parsed.hour == 0 and parsed.minute == 0 and parsed.second == 0:
                return parsed.strftime("%Y-%m-%d")
            return parsed.strftime("%Y-%m-%d %H:%M")
        return str(raw_value)
    return None


class CompareCard(WorkflowDecisionCard):
    toggled = pyqtSignal(str, bool)

    def __init__(self, slot_number: int, parent: QWidget | None = None) -> None:
        super().__init__(slot_number, parent)
        self.path: str = ""
        self.is_winner = False
        self._marked = False
        self._confirmed = False
        self._focused = False
        self._slot_number = slot_number

        self._score_label = QLabel("")
        self._score_label.setObjectName("workflowCompareScore")
        self._score_label.setStyleSheet("font-size: 11px; color: #AAB4BE;")
        self._content_layout.insertWidget(2, self._score_label)

        self._meta_grid = self._details_grid
        self._meta_rows = self._detail_rows

        self.activated.connect(self._toggle)
        self._update_style()

    def configure(
        self,
        *,
        path: str,
        is_winner: bool,
        marked: bool,
        confirmed: bool,
        score: float | None,
        failure_reason: str | None,
        metadata_rows: list[tuple[str, str]],
    ) -> None:
        self.path = path
        self.is_winner = is_winner
        self._marked = marked
        self._confirmed = confirmed
        self._name_label.setText(os.path.basename(path))
        if score is None:
            self._score_label.setText("Score unavailable")
            self._score_label.setToolTip(failure_reason or "")
        else:
            self._score_label.setText(f"Final score {score:.3f}")
            self._score_label.setToolTip("")

        self.set_details(metadata_rows)

        self._update_style()

    def set_marked(self, marked: bool) -> None:
        self._marked = marked
        self._update_style()

    def set_confirmed(self, confirmed: bool) -> None:
        self._confirmed = confirmed
        self._update_style()

    def set_focused(self, focused: bool) -> None:
        self._focused = focused
        self._update_style()

    def _toggle(self) -> None:
        if not self.path:
            return
        self._marked = not self._marked
        self._update_style()
        self.toggled.emit(self.path, self._marked)

    def set_info_visible(self, visible: bool) -> None:
        self._score_label.setVisible(visible)
        self._hint_label.setVisible(visible)
        self.set_details_visible(visible)

    def _update_style(self) -> None:
        if self._marked:
            border_color = MARKED_BORDER_COLOR
            status = (
                "MARKED FOR TRASH · staged"
                if self._confirmed
                else "SELECTED FOR TRASH · not confirmed"
            )
            color = "#FF7B86"
        elif self.is_winner:
            border_color = WINNER_BORDER_COLOR
            status = (
                "AI PICK · KEEP · confirmed"
                if self._confirmed
                else "AI PICK · KEEP · not confirmed"
            )
            color = WINNER_BORDER_COLOR
        else:
            border_color = FOCUSED_BORDER_COLOR if self._focused else CARD_BORDER_COLOR
            status = "KEEP · confirmed" if self._confirmed else "KEEP · not confirmed"
            color = KEEP_BORDER_COLOR

        bg = CARD_BG_WINNER if self.is_winner else CARD_BG
        if self.is_winner:
            hint = f"AI pick. Click or press {self._slot_number} to change the choice"
        else:
            hint = f"Click image/card or press {self._slot_number} to change the choice"
        self.set_decision(
            filename=os.path.basename(self.path) if self.path else "",
            state=status,
            state_color=color,
            border_color=border_color,
            background=bg,
            hint=hint,
        )


class PickBestStepWidget(QWidget):
    skip_requested = pyqtSignal()
    proceed_to_cull_requested = pyqtSignal()
    mark_for_deletion_requested = pyqtSignal(list)
    unmark_for_deletion_requested = pyqtSignal(list)
    active_image_changed = pyqtSignal(str)

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self._clusters: list[PickBestClusterResult] = []
        self._shown_results: PickBestResults | None = None
        self._cluster_index = 0
        self._subset_index = 0
        self._subset_paths: list[str] = []
        self._compare_cards: list[CompareCard] = []
        self._focused_slot_index = 0
        self._syncing_active_image = False
        self._current_winner_path = ""
        self._current_all_paths: list[str] = []
        self._cluster_ordered_paths: list[str] = []
        self._cluster_mark_state: dict[str, bool] = {}
        self._confirmed_clusters: set[int] = set()
        self._publishing_confirmation = False
        self._metadata_cache: dict[str, list[tuple[str, str]]] = {}
        self._focus_mode = False
        self._current_images_data: list[dict] = []
        self._info_visible = True
        self._is_marked_func: Callable[[str], bool] | None = None
        self._has_any_marked_func: Callable[[], bool] | None = None
        self._create_widgets()
        self._connect_signals()
        self._create_shortcuts()

    def show_loading(self, message: str = "Analysing…", percent: int = 0) -> None:
        self._stack.setCurrentWidget(self._page_loading)
        self._loading_label.setText(message)
        if percent is None or percent < 0:
            self._progress_bar.setRange(0, 0)
        else:
            self._progress_bar.setRange(0, 100)
            self._progress_bar.setValue(percent)

    def show_error(self, message: str) -> None:
        self._stack.setCurrentWidget(self._page_loading)
        self._loading_label.setText(f"Error: {message}")
        self._progress_bar.setValue(0)

    def show_results(self, results: PickBestResults) -> None:
        if self._shown_results is not None and results == self._shown_results:
            self.refresh_deletion_state()
            if self._clusters:
                self._stack.setCurrentWidget(self._page_review)
                self.setFocus()
            return
        self._shown_results = results
        self._clusters = [r for r in results.values() if r.get("winner_path")]
        self._confirmed_clusters = {
            index
            for index, cluster in enumerate(self._clusters)
            if cluster.get("_confirmed") is True
        }
        self._metadata_cache.clear()
        if not self._clusters:
            self.show_loading(
                "No comparable clusters found.\nClick 'Done' to continue to Cull."
            )
            self._skip_btn_loading.setText("Done: Go to Cull →")
            with contextlib.suppress(TypeError):
                self._skip_btn_loading.clicked.disconnect()
            self._skip_btn_loading.clicked.connect(self.proceed_to_cull_requested)
            return

        self._cluster_index = 0
        self._populate_cluster_list()
        self._load_cluster(0)
        self._stack.setCurrentWidget(self._page_review)
        self.setFocus()

    def set_is_marked_func(self, func: Callable[[str], bool]) -> None:
        self._is_marked_func = func
        self._sync_viewer.set_is_marked_for_deletion_func(
            lambda path: self._cluster_mark_state.get(path, bool(func(path)))
        )

    def set_has_any_marked_func(self, func: Callable[[], bool]) -> None:
        self._has_any_marked_func = func
        self._sync_viewer.set_has_any_marked_for_deletion_func(
            lambda: any(self._cluster_mark_state.values()) or bool(func())
        )

    def refresh_deletion_state(self) -> None:
        """Synchronize cards when marks are changed from another workflow."""

        if (
            self._publishing_confirmation
            or not self._is_marked_func
            or not self._cluster_mark_state
            or self._cluster_index not in self._confirmed_clusters
        ):
            return
        for path in list(self._cluster_mark_state):
            self._cluster_mark_state[path] = bool(self._is_marked_func(path))
        for card in self._compare_cards:
            if card.path:
                card.set_marked(self._cluster_mark_state.get(card.path, False))
                card.set_confirmed(self._cluster_index in self._confirmed_clusters)
        self._update_cluster_header_only()

    def discard_pending_decisions(self) -> None:
        """Clear saved per-cluster marks so revisiting cannot stage them again."""
        for cluster in self._clusters:
            cluster.pop("_mark_state", None)
            cluster.pop("_confirmed", None)
        for cluster in self._shown_results.values() if self._shown_results else ():
            cluster.pop("_mark_state", None)
            cluster.pop("_confirmed", None)
        self._confirmed_clusters.clear()
        self._populate_cluster_list()
        if self._clusters:
            self._load_cluster(min(self._cluster_index, len(self._clusters) - 1))

    def _create_widgets(self) -> None:
        main_layout = QVBoxLayout(self)
        main_layout.setContentsMargins(0, 0, 0, 0)
        main_layout.setSpacing(0)

        self._stack = QStackedWidget()
        main_layout.addWidget(self._stack)

        self._page_loading = QWidget()
        loading_layout = QVBoxLayout(self._page_loading)
        loading_layout.setAlignment(Qt.AlignmentFlag.AlignCenter)

        title = QLabel("Pick Best Photos")
        title.setStyleSheet("font-size: 20px; font-weight: bold; margin-bottom: 8px;")
        title.setAlignment(Qt.AlignmentFlag.AlignCenter)
        loading_layout.addWidget(title)

        self._loading_label = QLabel("Starting analysis…")
        self._loading_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self._loading_label.setWordWrap(True)
        self._loading_label.setStyleSheet(
            "font-size: 13px; color: #aaaaaa; margin-bottom: 12px;"
        )
        loading_layout.addWidget(self._loading_label)

        self._progress_bar = QProgressBar()
        self._progress_bar.setRange(0, 100)
        self._progress_bar.setValue(0)
        self._progress_bar.setFixedWidth(320)
        self._progress_bar.setTextVisible(True)
        loading_layout.addWidget(
            self._progress_bar, alignment=Qt.AlignmentFlag.AlignCenter
        )

        self._skip_btn_loading = QPushButton("Skip Step")
        self._skip_btn_loading.setFixedWidth(160)
        self._skip_btn_loading.setStyleSheet("margin-top: 16px;")
        loading_layout.addWidget(
            self._skip_btn_loading, alignment=Qt.AlignmentFlag.AlignCenter
        )
        self._stack.addWidget(self._page_loading)

        self._page_review = QWidget()
        self._page_review.setObjectName("workflowReviewPage")
        review_layout = QVBoxLayout(self._page_review)
        review_layout.setContentsMargins(0, 0, 0, 0)
        review_layout.setSpacing(0)

        review_content = QWidget()
        content_layout = QVBoxLayout(review_content)
        content_layout.setContentsMargins(12, 10, 12, 10)
        content_layout.setSpacing(8)

        review_splitter = QSplitter(Qt.Orientation.Horizontal)
        review_splitter.setHandleWidth(4)
        review_splitter.setChildrenCollapsible(False)
        self._review_list_panel = WorkflowReviewListPanel(
            bulk_action_text="Confirm all"
        )
        self._items_list = self._review_list_panel.list_widget
        self._confirm_all_btn = self._review_list_panel.bulk_button
        review_splitter.addWidget(self._review_list_panel)
        review_splitter.addWidget(review_content)
        review_splitter.setStretchFactor(0, 0)
        review_splitter.setStretchFactor(1, 1)

        cluster_bar = QWidget()
        cluster_bar.setObjectName("workflowClusterBar")
        header_layout = QHBoxLayout(cluster_bar)
        header_layout.setContentsMargins(0, 0, 0, 0)
        header_layout.setSpacing(8)

        self._prev_cluster_btn = QPushButton("◀ Prev Cluster")
        self._prev_cluster_btn.setObjectName("workflowGhostButton")
        self._prev_cluster_btn.setFixedWidth(110)
        header_layout.addWidget(self._prev_cluster_btn)

        self._cluster_info_label = QLabel()
        self._cluster_info_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self._cluster_info_label.setStyleSheet("font-size: 13px; font-weight: bold;")
        header_layout.addWidget(self._cluster_info_label, stretch=1)

        self._next_cluster_btn = QPushButton("Next Cluster ▶")
        self._next_cluster_btn.setObjectName("workflowGhostButton")
        self._next_cluster_btn.setFixedWidth(110)
        header_layout.addWidget(self._next_cluster_btn)
        content_layout.addWidget(cluster_bar)

        self._subset_info_label = QLabel()
        self._subset_info_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self._subset_info_label.setStyleSheet("font-size: 11px; color: #92A0AD;")
        content_layout.addWidget(self._subset_info_label)

        self._hint_label = QLabel(
            "The AI pick stays on the right as a reference, and every choice remains editable."
        )
        self._hint_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self._hint_label.setWordWrap(True)
        self._hint_label.setStyleSheet("font-size: 11px; color: #888888;")
        content_layout.addWidget(self._hint_label)

        self._sync_viewer = SynchronizedImageViewer()
        self._sync_viewer.controls_frame.hide()
        content_layout.addWidget(self._sync_viewer, stretch=1)

        self._state_banner = WorkflowStateBanner()
        self._state_banner.set_state(
            "Choose, then confirm",
            "Choices remain local to this cluster until you confirm them.",
            tone="warning",
        )
        content_layout.addWidget(self._state_banner)

        self._cards_row = QWidget()
        self._cards_layout = QHBoxLayout(self._cards_row)
        self._cards_layout.setContentsMargins(0, 0, 0, 0)
        self._cards_layout.setSpacing(10)
        for slot in range(3):
            card = CompareCard(slot + 1, self._cards_row)
            self._cards_layout.addWidget(card)
            self._compare_cards.append(card)
        content_layout.addWidget(self._cards_row)

        action_bar = QWidget()
        action_layout = QHBoxLayout(action_bar)
        action_layout.setContentsMargins(0, 2, 0, 2)
        action_layout.setSpacing(8)

        self._prev_set_btn = QPushButton("◀ Prev Set")
        self._prev_set_btn.setObjectName("workflowGhostButton")
        action_layout.addWidget(self._prev_set_btn)

        self._keep_all_btn = QPushButton("Keep visible  [K]")
        self._keep_all_btn.setObjectName("workflowDecisionKeep")
        self._keep_all_btn.setToolTip("Keep every currently visible photo")
        action_layout.addWidget(self._keep_all_btn)

        self._mark_rest_btn = QPushButton("Mark visible for Trash  [X]")
        self._mark_rest_btn.setObjectName("workflowDecisionTrash")
        self._mark_rest_btn.setToolTip("Stage every currently visible photo for Trash")
        action_layout.addWidget(self._mark_rest_btn)

        self._next_set_btn = QPushButton("Next Set ▶")
        self._next_set_btn.setObjectName("workflowGhostButton")
        action_layout.addWidget(self._next_set_btn)

        self._confirm_btn = QPushButton("Confirm  →")
        self._confirm_btn.setObjectName("workflowPrimaryButton")
        action_layout.addWidget(self._confirm_btn)

        action_layout.addStretch()

        self._done_btn = QPushButton("Continue to Cull  →")
        self._done_btn.setObjectName("workflowPrimaryButton")
        action_layout.addWidget(self._done_btn)

        content_layout.addWidget(action_bar)
        review_layout.addWidget(review_splitter, 1)
        self._stack.addWidget(self._page_review)
        self._stack.setCurrentWidget(self._page_loading)

    def _connect_signals(self) -> None:
        self._skip_btn_loading.clicked.connect(self.skip_requested)
        self._done_btn.clicked.connect(self._on_done)
        self._confirm_btn.clicked.connect(self._on_confirm)
        self._confirm_all_btn.clicked.connect(self._on_confirm_all)
        self._items_list.itemClicked.connect(self._on_cluster_item_clicked)
        self._prev_cluster_btn.clicked.connect(self._prev_cluster)
        self._next_cluster_btn.clicked.connect(self._next_cluster)
        self._prev_set_btn.clicked.connect(self._prev_subset)
        self._next_set_btn.clicked.connect(self._next_subset)
        self._keep_all_btn.clicked.connect(self._keep_visible)
        self._mark_rest_btn.clicked.connect(self._delete_visible)

        self._sync_viewer.markAsDeletedRequested.connect(self._on_viewer_mark)
        self._sync_viewer.unmarkAsDeletedRequested.connect(self._on_viewer_unmark)
        self._sync_viewer.markOthersAsDeletedRequested.connect(
            self._on_viewer_mark_others
        )
        self._sync_viewer.unmarkOthersAsDeletedRequested.connect(
            self._on_viewer_unmark_others
        )
        self._sync_viewer.imageClicked.connect(self._on_viewer_clicked)
        self._sync_viewer.installEventFilter(self)

        for card in self._compare_cards:
            card.toggled.connect(self._on_card_toggled)

    def _create_shortcuts(self) -> None:
        self._shortcuts = install_workflow_shortcuts(
            self,
            PICK_BEST_SHORTCUTS,
            {
                "slots:1": lambda: self._activate_slot_shortcut(0),
                "slots:2": lambda: self._activate_slot_shortcut(1),
                "slots:3": lambda: self._activate_slot_shortcut(2),
                "clusters:Left": self._prev_cluster,
                "clusters:Up": self._prev_cluster,
                "clusters:Right": self._next_cluster,
                "clusters:Down": self._next_cluster,
                "sets:[": self._prev_subset,
                "sets:]": self._next_subset,
                "bulk:K": self._keep_visible,
                "bulk:X": self._delete_visible,
                "focus": self._toggle_focus_mode,
                "info": self._toggle_info,
                "confirm": self._on_confirm,
                "skip": self.skip_requested.emit,
            },
        )

    def _cluster_item_text(self, index: int) -> str:
        cluster = self._clusters[index]
        count = len(cluster.get("all_paths", []))
        prefix = "Confirmed  ·  " if index in self._confirmed_clusters else ""
        return f"{prefix}Cluster {index + 1}  ·  {count} photo{'s' if count != 1 else ''}"

    def _populate_cluster_list(self) -> None:
        self._items_list.clear()
        for index in range(len(self._clusters)):
            item = QListWidgetItem(self._cluster_item_text(index))
            item.setData(Qt.ItemDataRole.UserRole, index)
            item.setForeground(
                QColor("#66BB6A" if index in self._confirmed_clusters else "#A9B7C6")
            )
            self._items_list.addItem(item)
        self._review_list_panel.set_count(len(self._clusters))

    def _refresh_cluster_list(self) -> None:
        for index in range(self._items_list.count()):
            item = self._items_list.item(index)
            item.setText(self._cluster_item_text(index))
            item.setForeground(
                QColor("#66BB6A" if index in self._confirmed_clusters else "#A9B7C6")
            )
        self._items_list.blockSignals(True)
        self._items_list.setCurrentRow(self._cluster_index)
        self._items_list.blockSignals(False)

    def _pending_state_for_cluster(
        self, cluster: PickBestClusterResult
    ) -> dict[str, bool]:
        winner_path = cluster.get("winner_path", "")
        all_paths: list[str] = cluster.get("all_paths", [])
        saved_mark_state = cluster.get("_mark_state")
        if isinstance(saved_mark_state, dict):
            return {
                path: bool(saved_mark_state.get(path, path != winner_path))
                for path in all_paths
            }
        return {
            path: (
                bool(self._is_marked_func(path))
                if self._is_marked_func and self._is_marked_func(path)
                else path != winner_path
            )
            for path in all_paths
        }

    def _save_current_cluster_state(self) -> None:
        if 0 <= self._cluster_index < len(self._clusters):
            self._clusters[self._cluster_index]["_mark_state"] = dict(
                self._cluster_mark_state
            )

    def _on_cluster_item_clicked(self, item: QListWidgetItem) -> None:
        index = item.data(Qt.ItemDataRole.UserRole)
        if not isinstance(index, int) or index == self._cluster_index:
            return
        self._exit_focus_mode()
        self._save_current_cluster_state()
        self._load_cluster(index)
        self._publish_focused_path()

    def _load_cluster(self, index: int) -> None:
        if not self._clusters:
            return

        self._cluster_index = index
        cluster = self._clusters[index]
        winner_path = cluster.get("winner_path", "")
        all_paths: list[str] = cluster.get("all_paths", [])

        self._current_winner_path = winner_path
        self._current_all_paths = list(all_paths)

        score_by_path, failure_reason_by_path = self._cluster_score_maps(cluster)

        non_winners = [path for path in all_paths if path != winner_path]
        non_winners.sort(
            key=lambda path: (
                score_by_path.get(path) is None,
                -(score_by_path.get(path) or 0.0),
                os.path.basename(path).lower(),
            )
        )
        self._cluster_ordered_paths = non_winners + (
            [winner_path] if winner_path else []
        )
        pending_state = self._pending_state_for_cluster(cluster)
        self._cluster_mark_state = {
            path: pending_state.get(path, path != winner_path)
            for path in self._cluster_ordered_paths
        }
        self._subset_index = 0
        self._update_cluster_ui(score_by_path, failure_reason_by_path)
        self._refresh_cluster_list()
        self._refresh_confirmation_controls()

    def focus_image(self, path: str) -> bool:
        """Open and focus the comparison containing path without changing decisions."""

        cluster_index = next(
            (
                index
                for index, cluster in enumerate(self._clusters)
                if path in cluster.get("all_paths", [])
            ),
            None,
        )
        if cluster_index is None:
            return False

        self._syncing_active_image = True
        try:
            if cluster_index != self._cluster_index:
                self._save_current_cluster_state()
            self._load_cluster(cluster_index)
            if path != self._current_winner_path:
                non_winners = [
                    candidate
                    for candidate in self._cluster_ordered_paths
                    if candidate != self._current_winner_path
                ]
                self._subset_index = non_winners.index(path) // 2
                cluster = self._clusters[self._cluster_index]
                score_map, failure_map = self._cluster_score_maps(cluster)
                self._update_cluster_ui(score_map, failure_map)
            if path in self._subset_paths:
                self._focused_slot_index = self._subset_paths.index(path)
                self._update_focus_state()
                if self._focus_mode:
                    self._sync_viewer.set_focused_viewer(self._focused_slot_index)
        finally:
            self._syncing_active_image = False
        return True

    def _update_cluster_ui(
        self,
        score_by_path: dict[str, float | None],
        failure_reason_by_path: dict[str, str],
    ) -> None:
        self._show_subset(score_by_path, failure_reason_by_path)
        total_clusters = len(self._clusters)
        total_sets = self._subset_count()
        self._cluster_info_label.setText(
            f"Cluster {self._cluster_index + 1} of {total_clusters}  ·  {len(self._current_all_paths)} photos"
        )
        self._subset_info_label.setText(
            f"Set {self._subset_index + 1} of {total_sets}  ·  challengers on the left, editable AI pick on the right"
        )
        self._prev_cluster_btn.setEnabled(self._cluster_index > 0)
        self._next_cluster_btn.setEnabled(self._cluster_index < total_clusters - 1)
        self._prev_set_btn.setEnabled(self._subset_index > 0)
        self._next_set_btn.setEnabled(self._subset_index < total_sets - 1)

    def _show_subset(
        self,
        score_by_path: dict[str, float | None],
        failure_reason_by_path: dict[str, str],
    ) -> None:
        non_winners = [
            path
            for path in self._cluster_ordered_paths
            if path != self._current_winner_path
        ]
        start = self._subset_index * 2
        subset_non_winners = non_winners[start : start + 2]
        subset_paths = list(subset_non_winners)
        if self._current_winner_path:
            subset_paths.append(self._current_winner_path)
        self._subset_paths = subset_paths

        image_pipeline = getattr(self.window(), "image_pipeline", None)
        images_data = []
        missing_preview_paths = []
        for path in subset_paths:
            pixmap = None
            if image_pipeline is not None:
                try:
                    pixmap = image_pipeline.get_cached_review_qpixmap(
                        path,
                        thumbnail_apply_orientation=True,
                    )
                except Exception as exc:
                    logger.debug("Could not load preview for %s: %s", path, exc)
            if pixmap is None or pixmap.isNull():
                missing_preview_paths.append(path)
            images_data.append({"path": path, "pixmap": pixmap, "rating": 0})

        self._current_images_data = images_data
        self._sync_viewer.set_images_data(images_data)
        request = getattr(self.window(), "request_interactive_previews", None)
        if missing_preview_paths and callable(request):
            request(missing_preview_paths)
        for viewer in self._sync_viewer.image_viewers:
            viewer.control_bar.hide()

        for index, card in enumerate(self._compare_cards):
            if index < len(subset_paths):
                path = subset_paths[index]
                card.show()
                card.configure(
                    path=path,
                    is_winner=path == self._current_winner_path,
                    marked=self._cluster_mark_state.get(path, False),
                    confirmed=self._cluster_index in self._confirmed_clusters,
                    score=score_by_path.get(path),
                    failure_reason=failure_reason_by_path.get(path),
                    metadata_rows=self._metadata_rows_for_path(
                        path, failure_reason=failure_reason_by_path.get(path)
                    ),
                )
            else:
                card.hide()

        if not self._info_visible:
            for card in self._compare_cards:
                if card.isVisible():
                    card.set_info_visible(False)

        self._focused_slot_index = min(
            self._focused_slot_index, max(0, len(subset_paths) - 1)
        )
        self._update_focus_state()
        if self._focus_mode:
            self._sync_viewer.set_focused_viewer(self._focused_slot_index)
        self.setFocus()

    def handle_preview_ready(self, path: str) -> None:
        if path not in self._subset_paths:
            return
        image_pipeline = getattr(self.window(), "image_pipeline", None)
        if image_pipeline is None:
            return
        pixmap = image_pipeline.get_cached_preview_qpixmap(
            path,
            memory_only=True,
        )
        if pixmap is None or pixmap.isNull():
            return
        for image_data in self._current_images_data:
            if image_data.get("path") == path:
                image_data["pixmap"] = pixmap
        self._sync_viewer.update_image_pixmap(path, pixmap)

    def _metadata_rows_for_path(
        self, path: str, *, failure_reason: str | None = None
    ) -> list[tuple[str, str]]:
        if path not in self._metadata_cache:
            self._metadata_cache[path] = self._build_metadata_rows(path)

        rows = list(self._metadata_cache[path])
        if failure_reason:
            rows.insert(0, ("Scoring", failure_reason))
        return rows[:6]

    def _build_metadata_rows(self, path: str) -> list[tuple[str, str]]:
        rows: list[tuple[str, str]] = []
        metadata = None
        app_state = getattr(self.window(), "app_state", None)
        cache = getattr(app_state, "exif_disk_cache", None) if app_state else None
        try:
            metadata = MetadataProcessor.get_cached_detailed_metadata(path, cache)
        except Exception:
            logger.debug("Cached EXIF lookup failed for %s", path, exc_info=True)

        if isinstance(metadata, dict):
            capture_date = _format_capture_date(metadata)
            if capture_date:
                rows.append(("Date", capture_date))

            camera_make = _first_present(
                metadata, "Exif.Image.Make", "Xmp.tiff.Make", "Make"
            )
            camera_model = _first_present(
                metadata, "Exif.Image.Model", "Xmp.tiff.Model", "Model"
            )
            if camera_make or camera_model:
                camera_text = " ".join(
                    str(part).strip() for part in (camera_make, camera_model) if part
                )
                rows.append(("Camera", camera_text))

            lens = _first_present(
                metadata,
                "Exif.Photo.LensModel",
                "Xmp.aux.Lens",
                "LensModel",
                "LensInfo",
            )
            if lens:
                rows.append(("Lens", str(lens)))

            focal = _float_text(
                _first_present(metadata, "Exif.Photo.FocalLength", "FocalLength"),
                suffix=" mm",
                digits=0,
            )
            aperture = _float_text(
                _first_present(
                    metadata,
                    "Exif.Photo.FNumber",
                    "Exif.Photo.ApertureValue",
                    "FNumber",
                ),
                prefix="f/",
                digits=1,
            )
            if focal or aperture:
                rows.append(
                    ("Lens", "  ".join(part for part in (focal, aperture) if part))
                )

            shutter = _fraction_text(
                _first_present(
                    metadata,
                    "Exif.Photo.ExposureTime",
                    "ExposureTime",
                    "Exif.Photo.ShutterSpeedValue",
                )
            )
            iso = _first_present(
                metadata,
                "Exif.Photo.ISOSpeedRatings",
                "ISO",
                "EXIF:ISO",
                "EXIF:ISOSpeedRatings",
            )
            if shutter or iso:
                iso_text = f"ISO {iso}" if iso not in (None, "") else None
                rows.append(
                    (
                        "Exposure",
                        "  ".join(part for part in (shutter, iso_text) if part),
                    )
                )

            width = _first_present(
                metadata,
                "pixel_width",
                "Exif.Photo.PixelXDimension",
                "Exif.Image.ImageWidth",
            )
            height = _first_present(
                metadata,
                "pixel_height",
                "Exif.Photo.PixelYDimension",
                "Exif.Image.ImageLength",
            )
            if width and height:
                rows.append(("Size", f"{width} × {height}"))

        if not rows:
            rows.append(("Metadata", "No EXIF details available"))

        self._metadata_cache[path] = rows[:6]
        return self._metadata_cache[path]

    def _cluster_score_maps(
        self, cluster: PickBestClusterResult
    ) -> tuple[dict[str, float | None], dict[str, str]]:
        ranked: list[dict] = cluster.get("ranked", [])
        failed: list[dict] = cluster.get("failed", [])
        score_by_path: dict[str, float | None] = {
            entry["path"]: entry.get("final_score") for entry in ranked
        }
        failure_reason_by_path: dict[str, str] = {}
        for entry in failed:
            path = entry.get("path")
            reason = entry.get("failure_reason")
            if path and reason:
                failure_reason_by_path[path] = str(reason)
        return score_by_path, failure_reason_by_path

    def _update_focus_state(self) -> None:
        for index, card in enumerate(self._compare_cards):
            if card.isVisible():
                card.set_focused(index == self._focused_slot_index)

    def _set_path_marked(self, path: str, marked: bool) -> None:
        if not path:
            return
        if self._cluster_mark_state.get(path) == marked:
            return
        self._invalidate_current_confirmation()
        self._cluster_mark_state[path] = marked
        self._save_current_cluster_state()
        for card in self._compare_cards:
            if card.isVisible() and card.path == path:
                card.set_marked(marked)
        self._update_cluster_header_only()

    def _invalidate_current_confirmation(self) -> None:
        if self._cluster_index not in self._confirmed_clusters:
            return
        self._confirmed_clusters.discard(self._cluster_index)
        self._clusters[self._cluster_index]["_confirmed"] = False
        for card in self._compare_cards:
            if card.path:
                card.set_confirmed(False)
        self._refresh_cluster_list()
        self._refresh_confirmation_controls()

    def _refresh_confirmation_controls(self) -> None:
        confirmed = self._cluster_index in self._confirmed_clusters
        all_confirmed = bool(self._clusters) and len(self._confirmed_clusters) == len(
            self._clusters
        )
        self._confirm_btn.setEnabled(not confirmed)
        self._confirm_btn.setText("Confirmed" if confirmed else "Confirm  →")
        self._confirm_all_btn.setEnabled(not all_confirmed)
        self._done_btn.setEnabled(all_confirmed)
        if confirmed:
            marked_count = sum(self._cluster_mark_state.values())
            self._state_banner.set_state(
                "Decision confirmed",
                f"{marked_count} photo{'s' if marked_count != 1 else ''} marked for Trash; no files have moved.",
                tone="success",
            )
        else:
            self._state_banner.set_state(
                "Choose, then confirm",
                "Choices remain local to this cluster until you confirm them.",
                tone="warning",
            )

    def _update_cluster_header_only(self) -> None:
        total_clusters = len(self._clusters)
        self._cluster_info_label.setText(
            f"Cluster {self._cluster_index + 1} of {total_clusters}  ·  {len(self._current_all_paths)} photos"
        )

    def _subset_count(self) -> int:
        non_winner_count = len(
            [
                path
                for path in self._cluster_ordered_paths
                if path != self._current_winner_path
            ]
        )
        return max(1, (non_winner_count + 1) // 2)

    def _visible_paths(self) -> list[str]:
        return [path for path in self._subset_paths if path]

    def _publish_confirmed_state(self, mark_state: dict[str, bool]) -> None:
        to_mark = [path for path, marked in mark_state.items() if marked]
        to_unmark = [path for path, marked in mark_state.items() if not marked]
        self._publishing_confirmation = True
        try:
            if to_mark:
                self.mark_for_deletion_requested.emit(to_mark)
            if to_unmark:
                self.unmark_for_deletion_requested.emit(to_unmark)
        finally:
            self._publishing_confirmation = False

    def _on_confirm(self) -> None:
        if not self._clusters or self._cluster_index in self._confirmed_clusters:
            return
        self._save_current_cluster_state()
        self._confirmed_clusters.add(self._cluster_index)
        self._clusters[self._cluster_index]["_confirmed"] = True
        self._publish_confirmed_state(dict(self._cluster_mark_state))
        for card in self._compare_cards:
            if card.path:
                card.set_confirmed(True)
        self._refresh_cluster_list()
        self._refresh_confirmation_controls()

        next_unconfirmed = next(
            (
                index
                for index in range(self._cluster_index + 1, len(self._clusters))
                if index not in self._confirmed_clusters
            ),
            None,
        )
        if next_unconfirmed is not None:
            self._load_cluster(next_unconfirmed)
            self._publish_focused_path()

    def _on_confirm_all(self) -> None:
        if not self._clusters:
            return
        self._save_current_cluster_state()
        combined_state: dict[str, bool] = {}
        for index, cluster in enumerate(self._clusters):
            state = self._pending_state_for_cluster(cluster)
            cluster["_mark_state"] = dict(state)
            cluster["_confirmed"] = True
            self._confirmed_clusters.add(index)
            combined_state.update(state)
        self._publish_confirmed_state(combined_state)
        for card in self._compare_cards:
            if card.path:
                card.set_confirmed(True)
        self._refresh_cluster_list()
        self._refresh_confirmation_controls()

    def _next_cluster(self) -> None:
        if self._cluster_index < len(self._clusters) - 1:
            self._exit_focus_mode()
            self._save_current_cluster_state()
            self._load_cluster(self._cluster_index + 1)
            self._publish_focused_path()

    def _prev_cluster(self) -> None:
        if self._cluster_index > 0:
            self._exit_focus_mode()
            self._save_current_cluster_state()
            self._load_cluster(self._cluster_index - 1)
            self._publish_focused_path()

    def _next_subset(self) -> None:
        max_subset = self._subset_count() - 1
        if self._subset_index < max_subset:
            self._subset_index += 1
            cluster = self._clusters[self._cluster_index]
            score_by_path, failure_reason_by_path = self._cluster_score_maps(cluster)
            self._update_cluster_ui(score_by_path, failure_reason_by_path)
            self._publish_focused_path()

    def _prev_subset(self) -> None:
        if self._subset_index > 0:
            self._subset_index -= 1
            cluster = self._clusters[self._cluster_index]
            score_by_path, failure_reason_by_path = self._cluster_score_maps(cluster)
            self._update_cluster_ui(score_by_path, failure_reason_by_path)
            self._publish_focused_path()

    def _keep_visible(self) -> None:
        paths = self._visible_paths()
        for path in paths:
            self._set_path_marked(path, False)

    def _delete_visible(self) -> None:
        paths = self._visible_paths()
        for path in paths:
            self._set_path_marked(path, True)

    def _toggle_slot(self, slot_index: int) -> None:
        if not (0 <= slot_index < len(self._subset_paths)):
            return
        path = self._subset_paths[slot_index]
        if not path:
            return
        new_marked = not self._cluster_mark_state.get(path, True)
        self._set_path_marked(path, new_marked)

    def _on_card_toggled(self, path: str, is_marked: bool) -> None:
        self._publish_active_image(path)
        self._set_path_marked(path, is_marked)

    def _on_viewer_clicked(self, slot_index: int, path: str) -> None:
        self._focused_slot_index = slot_index
        self._update_focus_state()
        self._publish_active_image(path)
        self._toggle_slot(slot_index)

    def _on_viewer_mark(self, path: str) -> None:
        self._set_path_marked(path, True)

    def _on_viewer_unmark(self, path: str) -> None:
        self._set_path_marked(path, False)

    def _on_viewer_mark_others(self, keeper_path: str) -> None:
        paths = [path for path in self._visible_paths() if path != keeper_path]
        for path in paths:
            self._set_path_marked(path, True)

    def _on_viewer_unmark_others(self, keeper_path: str) -> None:
        paths = [path for path in self._visible_paths() if path != keeper_path]
        for path in paths:
            self._set_path_marked(path, False)

    def _on_done(self) -> None:
        if len(self._confirmed_clusters) != len(self._clusters):
            self._state_banner.set_state(
                "Confirm every cluster first",
                "Use Confirm for each cluster, or Confirm all in the queue.",
                tone="warning",
            )
            return
        self.proceed_to_cull_requested.emit()

    def _activate_slot_shortcut(self, slot_index: int) -> None:
        self._focused_slot_index = slot_index
        self._update_focus_state()
        self._publish_focused_path()
        if self._focus_mode:
            self._sync_viewer.set_focused_viewer(slot_index)
        else:
            self._toggle_slot(slot_index)

    def _publish_focused_path(self) -> None:
        if 0 <= self._focused_slot_index < len(self._subset_paths):
            self._publish_active_image(self._subset_paths[self._focused_slot_index])

    def _publish_active_image(self, path: str) -> None:
        if path and not self._syncing_active_image:
            self.active_image_changed.emit(path)

    def _exit_focus_mode(self) -> None:
        """Reset to compare mode (flag + hint). No-op if already in compare mode."""
        if not self._focus_mode:
            return
        self._focus_mode = False
        self._hint_label.setText(
            "The AI pick stays on the right as a reference, and every choice remains editable."
        )

    def _toggle_focus_mode(self) -> None:
        if not self._focus_mode:
            self._focus_mode = True
            slot = min(self._focused_slot_index, max(0, len(self._subset_paths) - 1))
            self._sync_viewer.set_focused_viewer(slot)
            self._hint_label.setText(
                "Focus mode · press 1, 2, or 3 to switch photos · press C to compare again."
            )
        else:
            self._exit_focus_mode()
            self._sync_viewer.set_images_data(self._current_images_data)
            for viewer in self._sync_viewer.image_viewers:
                viewer.control_bar.hide()

    def _toggle_info(self) -> None:
        self._info_visible = not self._info_visible
        for card in self._compare_cards:
            if card.isVisible():
                card.set_info_visible(self._info_visible)

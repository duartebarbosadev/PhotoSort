import logging
import os
from dataclasses import dataclass, field
from collections.abc import Callable

from PyQt6.QtCore import Qt, pyqtSignal
from PyQt6.QtGui import QColor
from PyQt6.QtWidgets import (
    QAbstractItemView,
    QHBoxLayout,
    QLabel,
    QListWidgetItem,
    QProgressBar,
    QPushButton,
    QSplitter,
    QStackedWidget,
    QStyledItemDelegate,
    QStyleOptionViewItem,
    QVBoxLayout,
    QWidget,
)

from core.best_photo_finder.payloads import PickBestClusterResult, PickBestResults
from core.app_settings import (
    EASY_DELETE_SAME_FRAME_SIMILARITY,
    get_easy_delete_duplicate_distance,
)
from core.similarity_utils import cosine_similarity
from ui.advanced_image_viewer import SynchronizedImageViewer
from ui.controllers.image_inspection_controller import InspectionImageSpec
from ui.workflow_review_components import (
    PICK_BEST_SHORTCUTS,
    WorkflowDecisionCard,
    WorkflowReviewListPanel,
    WorkflowStateBanner,
    install_workflow_shortcuts,
)
from ui.workflow_metadata import build_workflow_metadata_rows

logger = logging.getLogger(__name__)

MARKED_BORDER_COLOR = "#E53935"
KEEP_BORDER_COLOR = "#66BB6A"
CARD_BG = "#20252C"
CARD_BG_WINNER = "#2C2616"
LIST_SECTION_ROLE = int(Qt.ItemDataRole.UserRole) + 1
LIST_CLUSTER_ROLE = int(Qt.ItemDataRole.UserRole) + 2
LIST_DEPTH_ROLE = int(Qt.ItemDataRole.UserRole) + 3


class TournamentItemDelegate(QStyledItemDelegate):
    """Paint comparison rows as children of their cluster heading."""

    CHILD_INDENT = 20

    def _indented_option(self, option, index) -> QStyleOptionViewItem:
        adjusted = QStyleOptionViewItem(option)
        if index.data(LIST_DEPTH_ROLE) == 1:
            adjusted.rect = adjusted.rect.adjusted(self.CHILD_INDENT, 0, 0, 0)
        return adjusted

    def paint(self, painter, option, index) -> None:
        super().paint(painter, self._indented_option(option, index), index)


class CompareCard(WorkflowDecisionCard):
    chosen = pyqtSignal(str)

    def __init__(self, slot_number: int, parent: QWidget | None = None) -> None:
        super().__init__(slot_number, parent, filename_in_header=True)
        self.path: str = ""
        self.is_ai_pick = False
        self._kept = False
        self._group_confirmed = False
        self._focused = False
        self._slot_number = slot_number
        self._display_name = ""

        self._meta_grid = self._details_grid
        self._meta_rows = self._detail_rows

        self.activated.connect(self._choose)
        self._update_style()

    def configure(
        self,
        *,
        path: str,
        is_ai_pick: bool,
        kept: bool,
        group_confirmed: bool,
        score: float | None,
        failure_reason: str | None,
        metadata_rows: list[tuple[str, str]],
    ) -> None:
        self.path = path
        self.is_ai_pick = is_ai_pick
        self._kept = kept
        self._group_confirmed = group_confirmed
        name = os.path.basename(path)
        name_parts = [name]
        if is_ai_pick:
            name_parts.append("AI suggestion")
        if score is None:
            name_parts.append("score unavailable")
            self._name_label.setToolTip(failure_reason or "")
        else:
            name_parts.append(f"score {score:.3f}")
            self._name_label.setToolTip("")
        self._display_name = " · ".join(name_parts)

        self.set_details(metadata_rows)

        self._update_style()

    def set_kept(self, kept: bool) -> None:
        self._kept = kept
        self._update_style()

    def set_group_confirmed(self, confirmed: bool) -> None:
        self._group_confirmed = confirmed
        self._update_style()

    def set_focused(self, focused: bool) -> None:
        self._focused = focused
        self._update_style()

    def _choose(self) -> None:
        if not self.path:
            return
        self.chosen.emit(self.path)

    def set_info_visible(self, visible: bool) -> None:
        self._hint_label.setVisible(visible)
        self.set_details_visible(visible)

    def _update_style(self) -> None:
        if self._kept:
            border_color = KEEP_BORDER_COLOR
            status = "KEEP"
            color = KEEP_BORDER_COLOR
            background = CARD_BG_WINNER
        else:
            border_color = MARKED_BORDER_COLOR
            status = "TRASH"
            color = "#FF7B86"
            background = CARD_BG

        action = "Trash" if self._kept else "Keep"
        if self._group_confirmed:
            hint = f"Confirmed {status}. Toggle to revise this comparison"
        else:
            hint = (
                f"Click image/card or press {self._slot_number} to change to {action}"
            )
        self.set_decision(
            filename=self._display_name,
            state=status,
            state_color=color,
            border_color=border_color,
            background=background,
            hint=hint,
        )


@dataclass(slots=True)
class TournamentGroup:
    paths: list[str]
    ai_pick: str
    keep_by_path: dict[str, bool]
    advancing_path: str
    confirmed: bool = False

    @property
    def selected_path(self) -> str:
        """Compatibility alias for the photo carried into the next comparison."""

        return self.advancing_path

    @property
    def keep_all(self) -> bool:
        """Return whether every photo in this comparison is currently kept."""

        return bool(self.paths) and all(
            self.keep_by_path.get(path, False) for path in self.paths
        )


@dataclass(slots=True)
class TournamentRound:
    groups: list[TournamentGroup]


@dataclass(slots=True)
class ClusterTournament:
    cluster_key: object
    payload: PickBestClusterResult
    rounds: list[TournamentRound] = field(default_factory=list)
    current_round: int = 0
    current_group: int = 0
    final_advancing_path: str | None = None
    finalized: bool = False
    prior_marks: dict[str, bool] | None = None
    next_path_index: int = 0

    @property
    def final_winner(self) -> str | None:
        """Compatibility alias for the final comparison carrier."""

        return self.final_advancing_path


class PickBestStepWidget(QWidget):
    apply_requested = pyqtSignal()
    mark_for_deletion_requested = pyqtSignal(list)
    unmark_for_deletion_requested = pyqtSignal(list)
    active_image_changed = pyqtSignal(str)

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self._clusters: list[PickBestClusterResult] = []
        self._cluster_keys: list[object] = []
        self._tournaments: list[ClusterTournament] = []
        self._shown_results: PickBestResults | None = None
        self._cluster_index = 0
        self._subset_index = 0  # compatibility alias for the current group index
        self._subset_paths: list[str] = []
        self._compare_cards: list[CompareCard] = []
        self._focused_slot_index = 0
        self._syncing_active_image = False
        self._current_winner_path = ""
        self._current_all_paths: list[str] = []
        self._cluster_ordered_paths: list[str] = []
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

    def show_results(
        self, results: PickBestResults, *, restore_prior_marks: bool = True
    ) -> None:
        if self._shown_results is not None and results == self._shown_results:
            if self._tournaments:
                self._stack.setCurrentWidget(self._page_review)
                self._load_cluster(self._cluster_index)
                self.setFocus()
            return
        if restore_prior_marks:
            for tournament in self._tournaments:
                if tournament.prior_marks is not None:
                    self._publish_confirmed_state(dict(tournament.prior_marks))
        self._shown_results = results
        cluster_entries = [
            (key, payload)
            for key, payload in results.items()
            if payload.get("winner_path")
        ]
        self._cluster_keys = [key for key, _payload in cluster_entries]
        self._clusters = [payload for _key, payload in cluster_entries]
        self._tournaments = [
            self._build_tournament(key, payload) for key, payload in cluster_entries
        ]
        self._metadata_cache.clear()
        if not self._tournaments:
            self.show_loading(
                "No comparable clusters found.\nUse the workflow footer to continue."
            )
            return

        self._cluster_index = 0
        self._load_cluster(0)
        self._stack.setCurrentWidget(self._page_review)
        self.setFocus()

    def sync_results_after_file_mutation(self, results: PickBestResults) -> None:
        """Rebuild review state after shared file operations invalidate results."""

        self._shown_results = None
        self.show_results(results, restore_prior_marks=False)

    def set_is_marked_func(self, func: Callable[[str], bool]) -> None:
        self._is_marked_func = func
        self._sync_viewer.set_is_marked_for_deletion_func(lambda _path: False)

    def set_has_any_marked_func(self, func: Callable[[], bool]) -> None:
        self._has_any_marked_func = func
        self._sync_viewer.set_has_any_marked_for_deletion_func(lambda: False)

    def refresh_deletion_state(self) -> None:
        """Tournament choices remain local until a final winner is confirmed."""

    def discard_pending_decisions(self) -> None:
        """Reset every local tournament while retaining worker analysis results."""
        for tournament in self._tournaments:
            if tournament.prior_marks is not None:
                self._publish_confirmed_state(dict(tournament.prior_marks))
        self._tournaments = [
            self._build_tournament(key, payload)
            for key, payload in zip(self._cluster_keys, self._clusters, strict=True)
        ]
        if self._tournaments:
            self._cluster_index = min(self._cluster_index, len(self._tournaments) - 1)
            self._load_cluster(self._cluster_index)

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
            bulk_action_text=None,
            title_text="",
            count_noun="photo",
        )
        self._items_list = self._review_list_panel.list_widget
        self._tournament_item_delegate = TournamentItemDelegate(self._items_list)
        self._items_list.setItemDelegate(self._tournament_item_delegate)
        self._items_list.setHorizontalScrollBarPolicy(
            Qt.ScrollBarPolicy.ScrollBarAlwaysOff
        )
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

        round_bar = QWidget()
        round_layout = QHBoxLayout(round_bar)
        round_layout.setContentsMargins(0, 0, 0, 0)
        round_layout.setSpacing(8)
        self._prev_round_btn = QPushButton("◀ Previous Comparison")
        self._prev_round_btn.setObjectName("workflowGhostButton")
        self._next_round_btn = QPushButton("Next Comparison ▶")
        self._next_round_btn.setObjectName("workflowGhostButton")
        round_layout.addWidget(self._prev_round_btn)
        round_layout.addStretch(1)
        round_layout.addWidget(self._next_round_btn)
        content_layout.addWidget(round_bar)

        self._sync_viewer = SynchronizedImageViewer()
        self._sync_viewer.configure_toolbar(show_view_modes=False)
        content_layout.addWidget(self._sync_viewer, stretch=1)

        self._state_banner = WorkflowStateBanner()
        self._state_banner.set_state(
            "Choose, then confirm",
            "Confirm applies the Keep and Trash marks immediately.",
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

        self._prev_set_btn = QPushButton("◀ Prev Group")
        self._prev_set_btn.setObjectName("workflowGhostButton")
        action_layout.addWidget(self._prev_set_btn)

        self._next_set_btn = QPushButton("Next Group ▶")
        self._next_set_btn.setObjectName("workflowGhostButton")
        action_layout.addWidget(self._next_set_btn)
        self._prev_set_btn.hide()
        self._next_set_btn.hide()

        self._confirm_btn = QPushButton("Confirm  →")
        self._confirm_btn.setObjectName("workflowPrimaryButton")
        action_layout.addWidget(self._confirm_btn)

        self._keep_all_btn = QPushButton("Keep all")
        self._keep_all_btn.setObjectName("workflowGhostButton")
        self._keep_all_btn.setToolTip(
            "Keep every photo in this group and continue without eliminating one (K)"
        )
        action_layout.addWidget(self._keep_all_btn)

        action_layout.addStretch()

        self._done_btn = QPushButton("Apply")
        self._done_btn.setObjectName("workflowPrimaryButton")
        action_layout.addWidget(self._done_btn)

        content_layout.addWidget(action_bar)
        review_layout.addWidget(review_splitter, 1)
        self._stack.addWidget(self._page_review)
        self._stack.setCurrentWidget(self._page_loading)

    def _connect_signals(self) -> None:
        self._done_btn.clicked.connect(self._on_apply)
        self._confirm_btn.clicked.connect(self._on_confirm)
        self._keep_all_btn.clicked.connect(self._on_keep_all)
        self._items_list.itemClicked.connect(self._on_photo_item_clicked)
        self._prev_cluster_btn.clicked.connect(self._prev_cluster)
        self._next_cluster_btn.clicked.connect(self._next_cluster)
        self._prev_set_btn.clicked.connect(self._prev_group)
        self._next_set_btn.clicked.connect(self._next_group)
        self._prev_round_btn.clicked.connect(self._prev_round)
        self._next_round_btn.clicked.connect(self._next_round)
        self._sync_viewer.imageClicked.connect(self._on_viewer_clicked)
        self._sync_viewer.installEventFilter(self)

        for card in self._compare_cards:
            card.chosen.connect(self._select_path)

    def _create_shortcuts(self) -> None:
        self._shortcuts = install_workflow_shortcuts(
            self,
            PICK_BEST_SHORTCUTS,
            {
                "slots:1": lambda: self._activate_slot_shortcut(0),
                "slots:2": lambda: self._activate_slot_shortcut(1),
                "slots:3": lambda: self._activate_slot_shortcut(2),
                "clusters:Left": self._prev_cluster,
                "clusters:Right": self._next_cluster,
                "groups:Up": self._prev_group,
                "groups:Down": self._next_group,
                "focus": self._toggle_focus_mode,
                "info": self._toggle_info,
                "keep_all": self._on_keep_all,
                "confirm": self._on_confirm,
                "apply": self._on_apply,
            },
        )

    def _make_round(
        self,
        paths: list[str],
        payload: PickBestClusterResult,
        *,
        carried_decisions: dict[str, bool] | None = None,
    ) -> TournamentRound:
        score_by_path, _failures = self._cluster_score_maps(payload)
        ai_pick = max(
            paths,
            key=lambda path: (
                score_by_path.get(path) is not None,
                score_by_path.get(path) or float("-inf"),
                -paths.index(path),
            ),
        )
        keep_by_path = dict(carried_decisions or {})
        for path in paths:
            keep_by_path.setdefault(path, path == ai_pick)
        return TournamentRound(
            [
                TournamentGroup(
                    paths=list(paths),
                    ai_pick=ai_pick,
                    keep_by_path=keep_by_path,
                    advancing_path=self._resolve_advancing_path(
                        paths, keep_by_path, ai_pick
                    ),
                )
            ]
        )

    @staticmethod
    def _resolve_advancing_path(
        paths: list[str], keep_by_path: dict[str, bool], ai_pick: str
    ) -> str:
        kept_paths = [path for path in paths if keep_by_path.get(path, False)]
        return kept_paths[0] if len(kept_paths) == 1 else ai_pick

    def _build_tournament(
        self, cluster_key: object, payload: PickBestClusterResult
    ) -> ClusterTournament:
        paths = list(dict.fromkeys(payload.get("all_paths", [])))
        tournament = ClusterTournament(
            cluster_key=cluster_key,
            payload=payload,
            next_path_index=min(2, len(paths)),
        )
        if len(paths) >= 2:
            tournament.rounds.append(self._make_round(paths[:2], payload))
        elif paths:
            tournament.final_advancing_path = paths[0]
            tournament.finalized = True
        return tournament

    @staticmethod
    def _total_round_count(photo_count: int) -> int:
        return max(0, photo_count - 1)

    def _current_tournament(self) -> ClusterTournament:
        return self._tournaments[self._cluster_index]

    def _current_group(self) -> TournamentGroup:
        tournament = self._current_tournament()
        return tournament.rounds[tournament.current_round].groups[
            tournament.current_group
        ]

    @staticmethod
    def _kept_paths(tournament: ClusterTournament) -> set[str]:
        kept_paths: set[str] = set()
        for round_ in tournament.rounds:
            for group in round_.groups:
                if not group.confirmed:
                    continue
                for path in group.paths:
                    if group.keep_by_path.get(path, False):
                        kept_paths.add(path)
                    else:
                        kept_paths.discard(path)
        return kept_paths

    @staticmethod
    def _comparison_item_presentation(
        tournament: ClusterTournament, round_index: int
    ) -> tuple[str, QColor, QColor]:
        """Describe one rolling comparison as the pair users see on screen."""

        group = tournament.rounds[round_index].groups[0]
        pair_text = "  ↔  ".join(os.path.basename(path) for path in group.paths)
        is_current = round_index == tournament.current_round
        if not group.confirmed:
            state = "Current comparison" if is_current else "Needs review"
            foreground = QColor("#F4C95D")
        else:
            kept_count = sum(
                group.keep_by_path.get(path, False) for path in group.paths
            )
            state = f"Complete · {kept_count} kept"
            foreground = QColor("#78D58A" if kept_count else "#FF7B86")
        background = QColor("#2B3035" if is_current else Qt.GlobalColor.transparent)
        return f"{pair_text}\n{state}", foreground, background

    def _populate_photo_list(self) -> None:
        self._refresh_photo_list()

    def _cluster_summary(self, index: int) -> tuple[str, QColor, QColor]:
        tournament = self._tournaments[index]
        paths = list(tournament.payload.get("all_paths", []))
        kept_paths = self._kept_paths(tournament)
        retained_count = len(kept_paths)
        has_progress = any(
            group.confirmed for round_ in tournament.rounds for group in round_.groups
        )

        if tournament.finalized:
            status = f"Complete · {retained_count} kept"
            foreground = QColor("#78D58A")
        elif index == self._cluster_index:
            comparison_number = tournament.current_round + 1
            comparison_total = max(1, len(paths) - 1)
            status = f"Current · comparison {comparison_number} of {comparison_total}"
            foreground = QColor("#D8E6F2")
        elif has_progress:
            comparison_number = tournament.current_round + 1
            comparison_total = max(1, len(paths) - 1)
            status = (
                f"In progress · comparison {comparison_number} of {comparison_total}"
            )
            foreground = QColor("#F4C95D")
        else:
            status = "Not started"
            foreground = QColor("#8D99A3")
        background = QColor(
            "#2B3035" if index == self._cluster_index else "transparent"
        )
        return (
            f"Cluster {index + 1} · {len(paths)} photos\n{status}",
            foreground,
            background,
        )

    def _sync_photo_sections(self) -> dict[int, QListWidgetItem]:
        comparison_items: dict[tuple[int, int], QListWidgetItem] = {}
        waiting_items: dict[tuple[int, str], QListWidgetItem] = {}
        cluster_items: dict[int, QListWidgetItem] = {}
        self._items_list.setUpdatesEnabled(False)
        while self._items_list.count():
            item = self._items_list.takeItem(0)
            path = item.data(Qt.ItemDataRole.UserRole)
            cluster_index = item.data(LIST_CLUSTER_ROLE)
            round_index = item.data(LIST_SECTION_ROLE)
            if isinstance(cluster_index, int) and isinstance(round_index, int):
                comparison_items[(cluster_index, round_index)] = item
            elif isinstance(cluster_index, int) and isinstance(path, str):
                waiting_items[(cluster_index, path)] = item
            elif isinstance(cluster_index, int):
                cluster_items[cluster_index] = item

        try:
            for cluster_index, tournament in enumerate(self._tournaments):
                cluster_item = cluster_items.get(cluster_index, QListWidgetItem())
                text, foreground, background = self._cluster_summary(cluster_index)
                cluster_item.setText(text)
                cluster_item.setData(Qt.ItemDataRole.UserRole, None)
                cluster_item.setData(LIST_SECTION_ROLE, None)
                cluster_item.setData(LIST_CLUSTER_ROLE, cluster_index)
                cluster_item.setData(LIST_DEPTH_ROLE, 0)
                cluster_item.setFlags(
                    Qt.ItemFlag.ItemIsEnabled | Qt.ItemFlag.ItemIsSelectable
                )
                cluster_item.setForeground(foreground)
                cluster_item.setBackground(background)
                cluster_item.setToolTip(
                    f"Open cluster {cluster_index + 1} of {len(self._tournaments)}"
                )
                font = cluster_item.font()
                font.setBold(cluster_index == self._cluster_index)
                cluster_item.setFont(font)
                self._items_list.addItem(cluster_item)

                if cluster_index == self._cluster_index:
                    for round_index, round_ in enumerate(tournament.rounds):
                        group = round_.groups[0]
                        item = comparison_items.get(
                            (cluster_index, round_index), QListWidgetItem()
                        )
                        item.setData(Qt.ItemDataRole.UserRole, tuple(group.paths))
                        item.setData(LIST_SECTION_ROLE, round_index)
                        item.setData(LIST_CLUSTER_ROLE, cluster_index)
                        item.setData(LIST_DEPTH_ROLE, 1)
                        item.setToolTip("\n↔\n".join(group.paths))
                        item.setFlags(
                            Qt.ItemFlag.ItemIsEnabled | Qt.ItemFlag.ItemIsSelectable
                        )
                        self._items_list.addItem(item)

                    all_paths = list(tournament.payload.get("all_paths", []))
                    for path in all_paths[tournament.next_path_index :]:
                        item = waiting_items.get(
                            (cluster_index, path), QListWidgetItem()
                        )
                        item.setText(f"Up next · {os.path.basename(path)}")
                        item.setData(Qt.ItemDataRole.UserRole, path)
                        item.setData(LIST_SECTION_ROLE, None)
                        item.setData(LIST_CLUSTER_ROLE, cluster_index)
                        item.setData(LIST_DEPTH_ROLE, 1)
                        item.setToolTip(path)
                        item.setFlags(Qt.ItemFlag.ItemIsEnabled)
                        item.setForeground(QColor("#8D99A3"))
                        item.setBackground(QColor(Qt.GlobalColor.transparent))
                        self._items_list.addItem(item)
        finally:
            self._items_list.setUpdatesEnabled(True)
        return {
            item.data(LIST_SECTION_ROLE): item
            for index in range(self._items_list.count())
            if isinstance(
                (item := self._items_list.item(index)).data(LIST_SECTION_ROLE), int
            )
        }

    def _refresh_photo_list(self) -> None:
        tournament = self._current_tournament()
        comparison_items = self._sync_photo_sections()
        for round_index, item in comparison_items.items():
            text, foreground, background = self._comparison_item_presentation(
                tournament, round_index
            )
            item.setText(text)
            item.setForeground(foreground)
            item.setBackground(background)
        self._review_list_panel.count_label.setText(
            f"{sum(tournament.finalized for tournament in self._tournaments)}/{len(self._tournaments)} done"
        )
        current_item = comparison_items.get(tournament.current_round)
        if current_item is None:
            self._items_list.setCurrentRow(-1)
            return
        self._items_list.setCurrentItem(current_item)
        self._items_list.scrollToItem(
            current_item,
            QAbstractItemView.ScrollHint.EnsureVisible,
        )

    def _on_photo_item_clicked(self, item: QListWidgetItem) -> None:
        cluster_index = item.data(LIST_CLUSTER_ROLE)
        round_index = item.data(LIST_SECTION_ROLE)
        if isinstance(cluster_index, int):
            if cluster_index != self._cluster_index:
                self._exit_focus_mode()
                self._load_cluster(cluster_index)
            if isinstance(round_index, int):
                tournament = self._current_tournament()
                tournament.current_round = max(
                    0, min(round_index, len(tournament.rounds) - 1)
                )
                tournament.current_group = 0
                self._subset_index = 0
                self._refresh_photo_list()
                self._show_current_group()
            self._publish_focused_path()
            return

    def _load_cluster(self, index: int) -> None:
        if not self._tournaments:
            return
        self._cluster_index = max(0, min(index, len(self._tournaments) - 1))
        tournament = self._current_tournament()
        all_paths = list(tournament.payload.get("all_paths", []))
        self._current_all_paths = all_paths
        self._cluster_ordered_paths = all_paths
        if tournament.final_advancing_path and not tournament.rounds:
            self._current_winner_path = tournament.final_advancing_path
            return
        tournament.current_round = min(
            tournament.current_round, len(tournament.rounds) - 1
        )
        groups = tournament.rounds[tournament.current_round].groups
        tournament.current_group = min(tournament.current_group, len(groups) - 1)
        self._subset_index = tournament.current_group
        self._populate_photo_list()
        self._show_current_group()

    def focus_image(self, path: str) -> bool:
        """Open and focus the comparison containing path without changing decisions."""

        cluster_index = next(
            (
                index
                for index, tournament in enumerate(self._tournaments)
                if path in tournament.payload.get("all_paths", [])
            ),
            None,
        )
        if cluster_index is None:
            return False

        self._syncing_active_image = True
        try:
            self._load_cluster(cluster_index)
            tournament = self._current_tournament()
            match = next(
                (
                    (round_index, group_index)
                    for round_index in range(len(tournament.rounds) - 1, -1, -1)
                    for group_index, group in enumerate(
                        tournament.rounds[round_index].groups
                    )
                    if path in group.paths
                ),
                None,
            )
            if match is not None:
                tournament.current_round, tournament.current_group = match
                self._subset_index = tournament.current_group
                self._refresh_photo_list()
                self._show_current_group()
            if path in self._subset_paths:
                self._focused_slot_index = self._subset_paths.index(path)
                self._update_focus_state()
                if self._focus_mode:
                    self._sync_viewer.set_focused_viewer(self._focused_slot_index)
        finally:
            self._syncing_active_image = False
        return True

    def _show_current_group(self) -> None:
        tournament = self._current_tournament()
        group = self._current_group()
        subset_paths = list(group.paths)
        self._subset_paths = subset_paths
        self._current_winner_path = group.advancing_path
        score_by_path, failure_reason_by_path = self._cluster_score_maps(
            tournament.payload
        )

        activate = getattr(self.window(), "activate_image_inspection", None)
        if callable(activate):
            activate(
                self._sync_viewer,
                [InspectionImageSpec(path=path) for path in subset_paths],
            )
            self._current_images_data = [
                {"path": path, "pixmap": None, "rating": 0} for path in subset_paths
            ]
        else:
            image_pipeline = getattr(self.window(), "image_pipeline", None)
            images_data = []
            for path in subset_paths:
                pixmap = None
                if image_pipeline is not None:
                    try:
                        pixmap, _ = image_pipeline.get_immediate_review_qpixmap(
                            path,
                            thumbnail_apply_orientation=True,
                        )
                    except Exception as exc:
                        logger.debug("Could not load preview for %s: %s", path, exc)
                images_data.append({"path": path, "pixmap": pixmap, "rating": 0})
            self._current_images_data = images_data
            self._sync_viewer.set_images_data(images_data)
            request = getattr(self.window(), "request_interactive_previews", None)
            if subset_paths and callable(request):
                request(subset_paths)
        for viewer in self._sync_viewer.image_viewers:
            viewer.control_bar.hide()

        for index, card in enumerate(self._compare_cards):
            if index < len(subset_paths):
                path = subset_paths[index]
                card.show()
                card.configure(
                    path=path,
                    is_ai_pick=path == group.ai_pick,
                    kept=group.keep_by_path.get(path, False),
                    group_confirmed=group.confirmed,
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
        self._refresh_photo_list()
        self._update_tournament_controls()
        self.setFocus()

    def _update_tournament_controls(self) -> None:
        tournament = self._current_tournament()
        group = self._current_group()
        similarity_detail = self._comparison_similarity_detail(group.paths)
        self._cluster_info_label.setText(
            f"Cluster {self._cluster_index + 1} of {len(self._tournaments)}  ·  {len(self._current_all_paths)} photos"
        )
        self._prev_cluster_btn.setEnabled(self._cluster_index > 0)
        self._next_cluster_btn.setEnabled(
            self._cluster_index < len(self._tournaments) - 1
        )
        self._prev_round_btn.setEnabled(tournament.current_round > 0)
        self._next_round_btn.setEnabled(
            tournament.current_round + 1 < len(tournament.rounds)
        )
        self._prev_set_btn.setEnabled(tournament.current_round > 0)
        self._next_set_btn.setEnabled(
            tournament.current_round + 1 < len(tournament.rounds)
        )
        self._confirm_btn.setEnabled(not group.confirmed)
        self._confirm_btn.setText("Confirmed" if group.confirmed else "Confirm  →")
        self._keep_all_btn.setEnabled(not (group.confirmed and group.keep_all))
        self._keep_all_btn.setText(
            "All kept"
            if group.confirmed and group.keep_all
            else f"Keep all {len(group.paths)}"
        )
        has_completed_cluster = any(
            candidate.finalized for candidate in self._tournaments
        )
        self._done_btn.setEnabled(has_completed_cluster)
        kept_count = sum(group.keep_by_path.get(path, False) for path in group.paths)
        if group.confirmed:
            advancing_name = os.path.basename(group.advancing_path)
            self._state_banner.set_state(
                "Decisions confirmed",
                self._append_similarity_detail(
                    f"{kept_count} of {len(group.paths)} photos kept. "
                    f"{advancing_name} continues to the next comparison.",
                    similarity_detail,
                ),
                tone="success" if kept_count else "warning",
            )
        else:
            self._state_banner.set_state(
                "Set each photo, then confirm",
                self._append_similarity_detail(
                    "Click a photo or press its number to toggle only that Keep/Trash decision.",
                    similarity_detail,
                ),
                tone="warning",
            )

    @staticmethod
    def _append_similarity_detail(message: str, similarity_detail: str) -> str:
        return f"{message} {similarity_detail}" if similarity_detail else message

    def _comparison_similarity_detail(self, paths: list[str]) -> str:
        if len(paths) != 2:
            return ""
        app_state = getattr(self.window(), "app_state", None)
        embeddings = getattr(app_state, "embeddings_cache", {}) if app_state else {}
        first = embeddings.get(paths[0])
        second = embeddings.get(paths[1])
        if first is None or second is None:
            return ""
        similarity = cosine_similarity(first, second)
        if similarity is None:
            return ""
        distance = max(0.0, 1.0 - similarity)
        cutoff = get_easy_delete_duplicate_distance()
        cutoff_result = "inside" if distance < cutoff else "outside"
        return (
            f"Cosine similarity {similarity:.4f} ({similarity * 100:.2f}%) · "
            f"distance {distance:.4f} · Easy Delete cosine cutoff < {cutoff:.4f} "
            f"({cutoff_result} cutoff). Easy Delete also accepts unchanged framing "
            f"at ≥ {EASY_DELETE_SAME_FRAME_SIMILARITY:.2f} structural similarity."
        )

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
        app_state = getattr(self.window(), "app_state", None)
        cache = getattr(app_state, "exif_disk_cache", None) if app_state else None
        try:
            rows = build_workflow_metadata_rows(path, cache)
        except Exception:
            logger.debug("Cached EXIF lookup failed for %s", path, exc_info=True)
            rows = [("Metadata", "No EXIF details available")]

        self._metadata_cache[path] = rows
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

    def _restore_prior_marks(self, tournament: ClusterTournament) -> None:
        if tournament.prior_marks is not None:
            self._publish_confirmed_state(dict(tournament.prior_marks))
        tournament.prior_marks = None
        tournament.final_advancing_path = None
        tournament.finalized = False

    def _invalidate_later_rounds(self, tournament: ClusterTournament) -> None:
        if tournament.prior_marks is not None:
            replayed_state = dict(tournament.prior_marks)
            for round_ in tournament.rounds[: tournament.current_round]:
                group = round_.groups[0]
                if not group.confirmed:
                    continue
                for path in group.paths:
                    replayed_state[path] = not group.keep_by_path.get(path, False)
            self._publish_confirmed_state(replayed_state)
        tournament.rounds = tournament.rounds[: tournament.current_round + 1]
        all_paths = list(tournament.payload.get("all_paths", []))
        path_indices = {path: index for index, path in enumerate(all_paths)}
        reviewed_indices = [
            path_indices[path]
            for round_ in tournament.rounds
            for group in round_.groups
            for path in group.paths
            if path in path_indices
        ]
        tournament.next_path_index = (
            max(reviewed_indices) + 1 if reviewed_indices else 0
        )
        tournament.final_advancing_path = None
        tournament.finalized = False

    def _ensure_prior_marks(self, tournament: ClusterTournament) -> None:
        if tournament.prior_marks is not None:
            return
        tournament.prior_marks = {
            path: bool(self._is_marked_func(path)) if self._is_marked_func else False
            for path in tournament.payload.get("all_paths", [])
        }

    def _publish_group_decision(self, group: TournamentGroup) -> None:
        self._publish_confirmed_state(
            {path: not group.keep_by_path.get(path, False) for path in group.paths}
        )

    def _select_path(self, path: str) -> None:
        group = self._current_group()
        if path not in group.paths:
            return
        if group.confirmed:
            self._invalidate_later_rounds(self._current_tournament())
            group.confirmed = False
        group.keep_by_path[path] = not group.keep_by_path.get(path, False)
        group.advancing_path = self._resolve_advancing_path(
            group.paths, group.keep_by_path, group.ai_pick
        )
        self._focused_slot_index = group.paths.index(path)
        self._show_current_group()
        self._refresh_photo_list()
        self._publish_active_image(path)

    def _finalize_current_tournament(self, advancing_path: str) -> None:
        tournament = self._current_tournament()
        tournament.final_advancing_path = advancing_path
        tournament.finalized = True

    def _advance_after_confirmation(self) -> None:
        # A confirmation finishes the current decision. Do not carry the
        # single-photo inspection mode into the next matchup, where it would make
        # a valid two-photo comparison appear to contain only one image.
        self._exit_focus_mode()
        tournament = self._current_tournament()
        group = self._current_group()
        advancing_path = group.advancing_path
        all_paths = list(tournament.payload.get("all_paths", []))
        if tournament.next_path_index >= len(all_paths):
            self._finalize_current_tournament(advancing_path)
            next_cluster = next(
                (
                    index
                    for index in range(self._cluster_index + 1, len(self._tournaments))
                    if not self._tournaments[index].finalized
                ),
                None,
            )
            if next_cluster is not None:
                self._load_cluster(next_cluster)
            else:
                self._show_current_group()
            return

        tournament.rounds = tournament.rounds[: tournament.current_round + 1]
        remaining_count = len(all_paths) - tournament.next_path_index
        kept_count = sum(group.keep_by_path.get(path, False) for path in group.paths)
        carried_decisions: dict[str, bool] | None
        if kept_count == 0 and remaining_count >= 2:
            next_paths = all_paths[
                tournament.next_path_index : tournament.next_path_index + 2
            ]
            tournament.next_path_index += 2
            carried_decisions = None
        else:
            challenger = all_paths[tournament.next_path_index]
            tournament.next_path_index += 1
            next_paths = [advancing_path, challenger]
            carried_decisions = {
                advancing_path: group.keep_by_path.get(advancing_path, False)
            }
        tournament.rounds.append(
            self._make_round(
                next_paths,
                tournament.payload,
                carried_decisions=carried_decisions,
            )
        )
        tournament.current_round += 1
        tournament.current_group = 0
        self._subset_index = 0
        self._refresh_photo_list()
        self._show_current_group()

    def _on_confirm(self) -> None:
        if not self._tournaments:
            return
        group = self._current_group()
        if group.confirmed:
            return
        tournament = self._current_tournament()
        self._ensure_prior_marks(tournament)
        group.advancing_path = self._resolve_advancing_path(
            group.paths, group.keep_by_path, group.ai_pick
        )
        group.confirmed = True
        self._publish_group_decision(group)
        self._refresh_photo_list()
        self._advance_after_confirmation()

    def _on_keep_all(self) -> None:
        if not self._tournaments:
            return
        tournament = self._current_tournament()
        group = self._current_group()
        if group.confirmed and group.keep_all:
            return
        if group.confirmed or tournament.finalized:
            self._invalidate_later_rounds(tournament)
        self._ensure_prior_marks(tournament)
        group.keep_by_path.update(dict.fromkeys(group.paths, True))
        group.advancing_path = self._resolve_advancing_path(
            group.paths, group.keep_by_path, group.ai_pick
        )
        group.confirmed = True
        self._publish_group_decision(group)
        self._refresh_photo_list()
        self._advance_after_confirmation()

    def _next_cluster(self) -> None:
        if self._cluster_index < len(self._tournaments) - 1:
            self._exit_focus_mode()
            self._load_cluster(self._cluster_index + 1)
            self._publish_focused_path()

    def _prev_cluster(self) -> None:
        if self._cluster_index > 0:
            self._exit_focus_mode()
            self._load_cluster(self._cluster_index - 1)
            self._publish_focused_path()

    def _next_group(self) -> None:
        """Move down within comparison history, then into the next cluster."""

        if not self._tournaments:
            return
        tournament = self._current_tournament()
        if tournament.current_round + 1 < len(tournament.rounds):
            self._next_round()
        else:
            self._next_cluster()

    def _prev_group(self) -> None:
        """Move up within comparison history, then into the previous cluster."""

        if not self._tournaments:
            return
        if self._current_tournament().current_round > 0:
            self._prev_round()
        else:
            self._prev_cluster()

    def _next_round(self) -> None:
        tournament = self._current_tournament()
        if tournament.current_round + 1 < len(tournament.rounds):
            tournament.current_round += 1
            tournament.current_group = 0
            self._subset_index = 0
            self._refresh_photo_list()
            self._show_current_group()

    def _prev_round(self) -> None:
        tournament = self._current_tournament()
        if tournament.current_round > 0:
            tournament.current_round -= 1
            tournament.current_group = 0
            self._subset_index = 0
            self._refresh_photo_list()
            self._show_current_group()

    def _on_viewer_clicked(self, slot_index: int, path: str) -> None:
        self._focused_slot_index = slot_index
        self._update_focus_state()
        self._publish_active_image(path)
        self._select_path(path)

    def _on_apply(self) -> None:
        if not self._tournaments or not any(
            tournament.finalized for tournament in self._tournaments
        ):
            self._state_banner.set_state(
                "Complete one cluster first",
                "Finish every comparison in at least one cluster before applying.",
                tone="warning",
            )
            return
        self.apply_requested.emit()

    def _activate_slot_shortcut(self, slot_index: int) -> None:
        if not 0 <= slot_index < len(self._subset_paths):
            return
        self._focused_slot_index = slot_index
        self._update_focus_state()
        self._publish_focused_path()
        if self._focus_mode:
            self._sync_viewer.set_focused_viewer(slot_index)
        self._select_path(self._subset_paths[slot_index])

    def _publish_focused_path(self) -> None:
        if 0 <= self._focused_slot_index < len(self._subset_paths):
            self._publish_active_image(self._subset_paths[self._focused_slot_index])

    def _publish_active_image(self, path: str) -> None:
        if path and not self._syncing_active_image:
            self.active_image_changed.emit(path)

    def _exit_focus_mode(self) -> None:
        """Reset to compare mode. No-op if already in compare mode."""
        if not self._focus_mode:
            return
        self._focus_mode = False

    def _toggle_focus_mode(self) -> None:
        if not self._focus_mode:
            self._focus_mode = True
            slot = min(self._focused_slot_index, max(0, len(self._subset_paths) - 1))
            self._sync_viewer.set_focused_viewer(slot)
        else:
            self._exit_focus_mode()
            self._sync_viewer.show_comparison()

    def _toggle_info(self) -> None:
        self._info_visible = not self._info_visible
        for card in self._compare_cards:
            if card.isVisible():
                card.set_info_visible(self._info_visible)

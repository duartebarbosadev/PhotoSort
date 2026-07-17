import logging
import os
from dataclasses import dataclass, field
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
LIST_SECTION_ROLE = int(Qt.ItemDataRole.UserRole) + 1
LIST_CLUSTER_ROLE = int(Qt.ItemDataRole.UserRole) + 2


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
    chosen = pyqtSignal(str)

    def __init__(self, slot_number: int, parent: QWidget | None = None) -> None:
        super().__init__(slot_number, parent)
        self.path: str = ""
        self.is_ai_pick = False
        self._selected = False
        self._group_confirmed = False
        self._group_keep_all = False
        self._protected = False
        self._focused = False
        self._slot_number = slot_number

        self._score_label = QLabel("")
        self._score_label.setObjectName("workflowCompareScore")
        self._score_label.setStyleSheet("font-size: 11px; color: #AAB4BE;")
        self._content_layout.insertWidget(2, self._score_label)

        self._meta_grid = self._details_grid
        self._meta_rows = self._detail_rows

        self.activated.connect(self._choose)
        self._update_style()

    def configure(
        self,
        *,
        path: str,
        is_ai_pick: bool,
        selected: bool,
        group_confirmed: bool,
        group_keep_all: bool,
        protected: bool,
        score: float | None,
        failure_reason: str | None,
        metadata_rows: list[tuple[str, str]],
    ) -> None:
        self.path = path
        self.is_ai_pick = is_ai_pick
        self._selected = selected
        self._group_confirmed = group_confirmed
        self._group_keep_all = group_keep_all
        self._protected = protected
        self._name_label.setText(os.path.basename(path))
        if score is None:
            self._score_label.setText("Score unavailable")
            self._score_label.setToolTip(failure_reason or "")
        else:
            prefix = "AI suggestion · " if is_ai_pick else ""
            self._score_label.setText(f"{prefix}score {score:.3f}")
            self._score_label.setToolTip("")

        self.set_details(metadata_rows)

        self._update_style()

    def set_selected(self, selected: bool) -> None:
        self._selected = selected
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
        self._score_label.setVisible(visible)
        self._hint_label.setVisible(visible)
        self.set_details_visible(visible)

    def _update_style(self) -> None:
        if self._protected and not self._group_keep_all:
            border_color = KEEP_BORDER_COLOR
            status = "KEPT · protected"
            color = KEEP_BORDER_COLOR
        elif self._group_confirmed and self._group_keep_all:
            border_color = KEEP_BORDER_COLOR
            status = "KEPT · confirmed"
            color = KEEP_BORDER_COLOR
        elif self._group_confirmed and self._selected:
            border_color = KEEP_BORDER_COLOR
            status = "KEPT · confirmed"
            color = KEEP_BORDER_COLOR
        elif self._group_confirmed:
            border_color = MARKED_BORDER_COLOR
            status = "TRASH · confirmed"
            color = "#FF7B86"
        elif self._selected:
            border_color = WINNER_BORDER_COLOR
            status = "KEEP · not confirmed"
            color = WINNER_BORDER_COLOR
        else:
            border_color = FOCUSED_BORDER_COLOR if self._focused else CARD_BORDER_COLOR
            status = "TRASH · not confirmed"
            color = "#FF9AA3"

        bg = CARD_BG_WINNER if self._selected or self._group_keep_all else CARD_BG
        if self._protected and not self._group_keep_all:
            hint = "Already kept; it will not be sent to Trash"
        elif self._group_keep_all:
            hint = "Kept. Select a photo to change this decision"
        elif self._selected:
            hint = "Selected. Click another photo or press 1–3 to change the choice"
        else:
            hint = f"Click image/card or press {self._slot_number} to select"
        self.set_decision(
            filename=os.path.basename(self.path) if self.path else "",
            state=status,
            state_color=color,
            border_color=border_color,
            background=bg,
            hint=hint,
        )


@dataclass(slots=True)
class TournamentGroup:
    paths: list[str]
    ai_pick: str
    selected_path: str
    confirmed: bool = False
    keep_all: bool = False


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
    final_winner: str | None = None
    finalized: bool = False
    prior_marks: dict[str, bool] | None = None
    next_path_index: int = 0


class PickBestStepWidget(QWidget):
    skip_requested = pyqtSignal()
    proceed_to_cull_requested = pyqtSignal()
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

    def show_results(self, results: PickBestResults) -> None:
        if self._shown_results is not None and results == self._shown_results:
            if self._tournaments:
                self._stack.setCurrentWidget(self._page_review)
                self._load_cluster(self._cluster_index)
                self.setFocus()
            return
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
                "No comparable clusters found.\nClick 'Done' to continue to Cull."
            )
            self._skip_btn_loading.setText("Done: Go to Cull →")
            with contextlib.suppress(TypeError):
                self._skip_btn_loading.clicked.disconnect()
            self._skip_btn_loading.clicked.connect(self.proceed_to_cull_requested)
            return

        self._cluster_index = 0
        self._load_cluster(0)
        self._stack.setCurrentWidget(self._page_review)
        self.setFocus()

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
            bulk_action_text=None,
            title_text="Tournament",
            count_noun="photo",
        )
        self._items_list = self._review_list_panel.list_widget
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
        self._round_info_label = QLabel()
        self._round_info_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self._round_info_label.setStyleSheet("font-size: 12px; font-weight: bold;")
        self._next_round_btn = QPushButton("Next Comparison ▶")
        self._next_round_btn.setObjectName("workflowGhostButton")
        round_layout.addWidget(self._prev_round_btn)
        round_layout.addWidget(self._round_info_label, 1)
        round_layout.addWidget(self._next_round_btn)
        content_layout.addWidget(round_bar)

        self._subset_info_label = QLabel()
        self._subset_info_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self._subset_info_label.setStyleSheet("font-size: 11px; color: #92A0AD;")
        content_layout.addWidget(self._subset_info_label)

        self._hint_label = QLabel(
            "Confirm keeps the selection and sends the other photo to Trash. Keep all protects both."
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
            "Keep every photo in this group and continue without eliminating one"
        )
        action_layout.addWidget(self._keep_all_btn)

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
                "confirm": self._on_confirm,
                "skip": self.skip_requested.emit,
            },
        )

    def _make_round(
        self, paths: list[str], payload: PickBestClusterResult
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
        return TournamentRound(
            [
                TournamentGroup(
                    paths=list(paths),
                    ai_pick=ai_pick,
                    selected_path=ai_pick,
                )
            ]
        )

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
            tournament.final_winner = paths[0]
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
    def _path_matchup_count(tournament: ClusterTournament, path: str) -> int:
        return sum(
            path in group.paths
            for round_ in tournament.rounds
            for group in round_.groups
        )

    @staticmethod
    def _path_elimination_round(
        tournament: ClusterTournament, path: str
    ) -> int | None:
        protected_paths: set[str] = set()
        for round_index, round_ in enumerate(tournament.rounds):
            for group in round_.groups:
                if group.confirmed and group.keep_all:
                    protected_paths.update(group.paths)
                if (
                    group.confirmed
                    and not group.keep_all
                    and path in group.paths
                    and path != group.selected_path
                    and path not in protected_paths
                ):
                    return round_index + 1
        return None

    @staticmethod
    def _kept_paths(tournament: ClusterTournament) -> set[str]:
        return {
            path
            for round_ in tournament.rounds
            for group in round_.groups
            if group.confirmed and group.keep_all
            for path in group.paths
        }

    def _photo_item_presentation(self, path: str) -> tuple[str, QColor, QColor]:
        tournament = self._current_tournament()
        group = self._current_group()
        matchup_count = self._path_matchup_count(tournament, path)
        elimination_round = self._path_elimination_round(tournament, path)
        kept_paths = self._kept_paths(tournament)

        if path in kept_paths:
            state = "Kept"
            foreground = QColor("#78D58A")
            background = QColor("#203529")
        elif tournament.final_winner == path:
            state = "Winner"
            foreground = QColor("#78D58A")
            background = QColor("#203529")
        elif elimination_round is not None:
            state = "Trash"
            foreground = QColor("#7F8B96")
            background = QColor(Qt.GlobalColor.transparent)
        elif path in group.paths:
            if group.confirmed:
                state = (
                    "Advanced"
                    if path == group.selected_path
                    else "Trash"
                )
            else:
                state = "Current"
            foreground = QColor(
                "#F4C95D" if path == group.selected_path else "#C9D8E5"
            )
            background = QColor("#2B3035")
        else:
            has_advanced = any(
                candidate.confirmed
                and not candidate.keep_all
                and candidate.selected_path == path
                for round_ in tournament.rounds
                for candidate in round_.groups
            )
            state = "Advanced" if matchup_count > 1 or has_advanced else "Waiting"
            foreground = QColor("#A9B7C6")
            background = QColor(Qt.GlobalColor.transparent)

        text = f"{os.path.basename(path)}\n{state}"
        return text, foreground, background

    def _populate_photo_list(self) -> None:
        self._refresh_photo_list()

    def _photo_sections(self) -> list[tuple[str, str, list[str]]]:
        tournament = self._current_tournament()
        paths = list(tournament.payload.get("all_paths", []))
        current_paths = list(self._current_group().paths)
        current_set = set(current_paths)
        kept_paths = self._kept_paths(tournament)
        decided_paths = [
            path
            for path in paths
            if path not in current_set
            and (
                path in kept_paths
                or path == tournament.final_winner
                or self._path_elimination_round(tournament, path) is not None
            )
        ]
        decided_set = set(decided_paths)
        in_play_paths = [
            path
            for path in paths
            if path not in current_set and path not in decided_set
        ]
        sections = [
            ("current", "CURRENT GROUP", current_paths),
            ("in_play", "STILL IN PLAY", in_play_paths),
            ("decided", "DECIDED", decided_paths),
        ]
        return [section for section in sections if section[2]]

    def _cluster_summary(self, index: int) -> tuple[str, QColor, QColor]:
        tournament = self._tournaments[index]
        paths = list(tournament.payload.get("all_paths", []))
        kept_paths = self._kept_paths(tournament)
        eliminated_count = sum(
            self._path_elimination_round(tournament, path) is not None
            for path in paths
        )
        active_count = len(paths) - len(kept_paths) - eliminated_count
        retained_paths = set(kept_paths)
        if tournament.final_winner is not None:
            retained_paths.add(tournament.final_winner)
        retained_count = len(retained_paths)
        has_progress = any(
            group.confirmed for round_ in tournament.rounds for group in round_.groups
        )

        if tournament.finalized:
            status = f"Complete · {retained_count} kept"
            foreground = QColor("#78D58A")
        elif index == self._cluster_index:
            status = f"Current · {active_count} active"
            foreground = QColor("#D8E6F2")
        elif has_progress:
            status = f"In progress · {active_count} active"
            foreground = QColor("#F4C95D")
        else:
            status = "Not started"
            foreground = QColor("#8D99A3")
        background = QColor("#2B3035" if index == self._cluster_index else "transparent")
        return f"Cluster {index + 1} · {len(paths)} photos\n{status}", foreground, background

    def _sync_photo_sections(self) -> dict[str, QListWidgetItem]:
        photo_items: dict[str, QListWidgetItem] = {}
        header_items: dict[str, QListWidgetItem] = {}
        cluster_items: dict[int, QListWidgetItem] = {}
        self._items_list.setUpdatesEnabled(False)
        while self._items_list.count():
            item = self._items_list.takeItem(0)
            path = item.data(Qt.ItemDataRole.UserRole)
            section_key = item.data(LIST_SECTION_ROLE)
            cluster_index = item.data(LIST_CLUSTER_ROLE)
            if isinstance(path, str):
                photo_items[path] = item
            elif isinstance(cluster_index, int):
                cluster_items[cluster_index] = item
            elif isinstance(section_key, str):
                header_items[section_key] = item

        try:
            for cluster_index, _tournament in enumerate(self._tournaments):
                cluster_item = cluster_items.get(cluster_index, QListWidgetItem())
                text, foreground, background = self._cluster_summary(cluster_index)
                cluster_item.setText(text)
                cluster_item.setData(Qt.ItemDataRole.UserRole, None)
                cluster_item.setData(LIST_SECTION_ROLE, None)
                cluster_item.setData(LIST_CLUSTER_ROLE, cluster_index)
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

            for section_key, title, section_paths in self._photo_sections():
                owned_section_key = f"{self._cluster_index}:{section_key}"
                header = header_items.get(owned_section_key, QListWidgetItem())
                header.setText(f"{title}  ·  {len(section_paths)}")
                header.setData(Qt.ItemDataRole.UserRole, None)
                header.setData(LIST_SECTION_ROLE, owned_section_key)
                header.setData(LIST_CLUSTER_ROLE, None)
                header.setFlags(Qt.ItemFlag.NoItemFlags)
                header.setForeground(QColor("#7F8B96"))
                header.setBackground(QColor("#24272A"))
                header_font = header.font()
                header_font.setBold(True)
                header.setFont(header_font)
                self._items_list.addItem(header)
                for path in section_paths:
                    item = photo_items.get(path, QListWidgetItem())
                    item.setData(Qt.ItemDataRole.UserRole, path)
                    item.setData(LIST_SECTION_ROLE, None)
                    item.setData(LIST_CLUSTER_ROLE, self._cluster_index)
                    item.setToolTip(path)
                    item.setFlags(
                        Qt.ItemFlag.ItemIsEnabled | Qt.ItemFlag.ItemIsSelectable
                    )
                    self._items_list.addItem(item)
        finally:
            self._items_list.setUpdatesEnabled(True)
        return {
            item.data(Qt.ItemDataRole.UserRole): item
            for index in range(self._items_list.count())
            if isinstance(
                (item := self._items_list.item(index)).data(Qt.ItemDataRole.UserRole),
                str,
            )
        }

    def _row_for_path(self, path: str) -> int | None:
        return next(
            (
                index
                for index in range(self._items_list.count())
                if self._items_list.item(index).data(Qt.ItemDataRole.UserRole) == path
            ),
            None,
        )

    def _refresh_photo_list(self) -> None:
        tournament = self._current_tournament()
        paths = list(tournament.payload.get("all_paths", []))
        photo_items = self._sync_photo_sections()
        for path in paths:
            item = photo_items[path]
            text, foreground, background = self._photo_item_presentation(path)
            item.setText(text)
            item.setForeground(foreground)
            item.setBackground(background)
        self._review_list_panel.count_label.setText(
            f"{sum(tournament.finalized for tournament in self._tournaments)}/{len(self._tournaments)} done"
        )
        selected_path = self._current_group().selected_path
        selected_row = self._row_for_path(selected_path)
        if selected_row is not None:
            self._items_list.setCurrentRow(selected_row)

    def _on_photo_item_clicked(self, item: QListWidgetItem) -> None:
        cluster_index = item.data(LIST_CLUSTER_ROLE)
        path = item.data(Qt.ItemDataRole.UserRole)
        if isinstance(cluster_index, int) and not isinstance(path, str):
            if cluster_index != self._cluster_index:
                self._exit_focus_mode()
                self._load_cluster(cluster_index)
                self._publish_focused_path()
            return
        tournament = self._current_tournament()
        if not isinstance(path, str):
            return
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
        if match is None:
            return
        tournament.current_round, tournament.current_group = match
        self._subset_index = tournament.current_group
        self._show_current_group()
        if path in self._subset_paths:
            self._focused_slot_index = self._subset_paths.index(path)
            self._update_focus_state()
            self._publish_active_image(path)

    def _load_cluster(self, index: int) -> None:
        if not self._tournaments:
            return
        self._cluster_index = max(0, min(index, len(self._tournaments) - 1))
        tournament = self._current_tournament()
        all_paths = list(tournament.payload.get("all_paths", []))
        self._current_all_paths = all_paths
        self._cluster_ordered_paths = all_paths
        if tournament.final_winner and not tournament.rounds:
            self._current_winner_path = tournament.final_winner
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
                index for index, tournament in enumerate(self._tournaments)
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
        self._current_winner_path = group.selected_path
        score_by_path, failure_reason_by_path = self._cluster_score_maps(
            tournament.payload
        )
        protected_paths = self._kept_paths(tournament)

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
                    is_ai_pick=path == group.ai_pick,
                    selected=path == group.selected_path,
                    group_confirmed=group.confirmed,
                    group_keep_all=group.keep_all,
                    protected=path in protected_paths,
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
        total_rounds = self._total_round_count(len(self._current_all_paths))
        self._cluster_info_label.setText(
            f"Cluster {self._cluster_index + 1} of {len(self._tournaments)}  ·  {len(self._current_all_paths)} photos"
        )
        self._round_info_label.setText(
            f"Comparison {tournament.current_round + 1} of {total_rounds}"
        )
        self._subset_info_label.setText(
            "Choose the photo that continues"
        )
        selected_row = self._row_for_path(group.selected_path)
        if selected_row is not None:
            self._items_list.setCurrentRow(selected_row)
        self._prev_cluster_btn.setEnabled(self._cluster_index > 0)
        self._next_cluster_btn.setEnabled(self._cluster_index < len(self._tournaments) - 1)
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
            "All kept" if group.confirmed and group.keep_all else f"Keep all {len(group.paths)}"
        )
        all_complete = bool(self._tournaments) and all(
            candidate.finalized for candidate in self._tournaments
        )
        self._done_btn.setEnabled(all_complete)
        if group.confirmed and group.keep_all:
            self._state_banner.set_state(
                "All kept",
                "Both photos are kept; the selected photo continues.",
                tone="success",
            )
        elif group.confirmed:
            self._state_banner.set_state(
                "Choice confirmed",
                f"{os.path.basename(group.selected_path)} stays; the other photo is now marked Trash.",
                tone="success",
            )
        else:
            self._state_banner.set_state(
                "Choose, then confirm",
                "Confirm applies Keep and Trash immediately.",
                tone="warning",
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
        tournament.final_winner = None
        tournament.finalized = False

    def _invalidate_later_rounds(self, tournament: ClusterTournament) -> None:
        if tournament.prior_marks is not None:
            replayed_state = dict(tournament.prior_marks)
            protected_paths: set[str] = set()
            for round_ in tournament.rounds[: tournament.current_round]:
                group = round_.groups[0]
                if not group.confirmed:
                    continue
                if group.keep_all:
                    protected_paths.update(group.paths)
                for path in group.paths:
                    replayed_state[path] = (
                        False
                        if path in protected_paths or path == group.selected_path
                        else True
                    )
            self._publish_confirmed_state(replayed_state)
        tournament.rounds = tournament.rounds[: tournament.current_round + 1]
        tournament.next_path_index = tournament.current_round + 2
        tournament.final_winner = None
        tournament.finalized = False

    def _ensure_prior_marks(self, tournament: ClusterTournament) -> None:
        if tournament.prior_marks is not None:
            return
        tournament.prior_marks = {
            path: bool(self._is_marked_func(path)) if self._is_marked_func else False
            for path in tournament.payload.get("all_paths", [])
        }

    def _publish_group_decision(
        self, tournament: ClusterTournament, group: TournamentGroup
    ) -> None:
        protected_paths = self._kept_paths(tournament)
        self._publish_confirmed_state(
            {
                path: False
                if path in protected_paths or path == group.selected_path
                else True
                for path in group.paths
            }
        )

    def _select_path(self, path: str) -> None:
        group = self._current_group()
        if path not in group.paths:
            return
        if group.confirmed and group.keep_all:
            self._invalidate_later_rounds(self._current_tournament())
            group.confirmed = False
            group.keep_all = False
        elif path == group.selected_path:
            return
        if group.confirmed:
            self._invalidate_later_rounds(self._current_tournament())
            group.confirmed = False
        group.selected_path = path
        self._focused_slot_index = group.paths.index(path)
        self._show_current_group()
        self._refresh_photo_list()
        self._publish_active_image(path)

    def _finalize_current_tournament(self, winner_path: str) -> None:
        tournament = self._current_tournament()
        tournament.final_winner = winner_path
        tournament.finalized = True

    def _advance_after_confirmation(self) -> None:
        tournament = self._current_tournament()
        winner_path = self._current_group().selected_path
        all_paths = list(tournament.payload.get("all_paths", []))
        if tournament.next_path_index >= len(all_paths):
            self._finalize_current_tournament(winner_path)
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
        challenger = all_paths[tournament.next_path_index]
        tournament.next_path_index += 1
        tournament.rounds.append(
            self._make_round([winner_path, challenger], tournament.payload)
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
        if group.confirmed or not group.selected_path:
            return
        tournament = self._current_tournament()
        self._ensure_prior_marks(tournament)
        group.keep_all = False
        group.confirmed = True
        self._publish_group_decision(tournament, group)
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
        group.keep_all = True
        group.confirmed = True
        self._publish_group_decision(tournament, group)
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
        self._next_round()

    def _prev_group(self) -> None:
        self._prev_round()

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

    def _on_done(self) -> None:
        if not self._tournaments or not all(
            tournament.finalized for tournament in self._tournaments
        ):
            self._state_banner.set_state(
                "Confirm every cluster first",
                "Finish every rolling comparison before continuing.",
                tone="warning",
            )
            return
        self.proceed_to_cull_requested.emit()

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
        """Reset to compare mode (flag + hint). No-op if already in compare mode."""
        if not self._focus_mode:
            return
        self._focus_mode = False
        self._hint_label.setText(
            "Choose one photo from this group. The AI selection is only a suggestion."
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

from __future__ import annotations

import logging
import os
from typing import Dict, List, Optional

from PyQt6.QtCore import Qt, pyqtSignal
from PyQt6.QtGui import QImage, QKeyEvent, QPixmap
from PyQt6.QtWidgets import (
    QFrame,
    QGridLayout,
    QHBoxLayout,
    QLabel,
    QProgressBar,
    QPushButton,
    QScrollArea,
    QStackedWidget,
    QVBoxLayout,
    QWidget,
)

logger = logging.getLogger(__name__)

THUMB_SIZE = 180
CARDS_PER_ROW = 4
WINNER_BORDER_COLOR = "#F5B700"
MARKED_BORDER_COLOR = "#E53935"
NEUTRAL_BORDER_COLOR = "#555555"
FOCUSED_BORDER_COLOR = "#4FC3F7"


def _load_pixmap(path: str, size: int = THUMB_SIZE) -> Optional[QPixmap]:
    try:
        from PIL import Image, ImageOps

        img = Image.open(path)
        img = ImageOps.exif_transpose(img)
        img = img.convert("RGB")
        img.thumbnail((size, size), Image.Resampling.LANCZOS)
        w, h = img.size
        data = img.tobytes("raw", "RGB")
        qimg = QImage(data, w, h, w * 3, QImage.Format.Format_RGB888)
        return QPixmap.fromImage(qimg)
    except Exception as exc:
        logger.debug(f"Could not load thumbnail for {path}: {exc}")
        return None


class ThumbnailCard(QFrame):
    """A clickable thumbnail card showing one image in the cluster."""

    toggled = pyqtSignal(str, bool)  # path, is_marked

    def __init__(
        self,
        path: str,
        is_winner: bool,
        score: Optional[float],
        pixmap: Optional[QPixmap] = None,
        parent: Optional[QWidget] = None,
    ) -> None:
        super().__init__(parent)
        self.path = path
        self.is_winner = is_winner
        self._marked = False
        self._focused = False

        self.setFixedSize(THUMB_SIZE + 16, THUMB_SIZE + 44)
        self.setFrameShape(QFrame.Shape.StyledPanel)
        self.setCursor(
            Qt.CursorShape.PointingHandCursor
            if not is_winner
            else Qt.CursorShape.ArrowCursor
        )

        layout = QVBoxLayout(self)
        layout.setContentsMargins(4, 4, 4, 4)
        layout.setSpacing(2)

        # Image area
        self._img_label = QLabel()
        self._img_label.setFixedSize(THUMB_SIZE, THUMB_SIZE)
        self._img_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self._img_label.setStyleSheet("background-color: #1a1a1a;")
        layout.addWidget(self._img_label, alignment=Qt.AlignmentFlag.AlignCenter)

        # Filename + score row
        basename = os.path.basename(path)
        score_str = f"  ({score:.2f})" if score is not None else ""
        name_label = QLabel(basename + score_str)
        name_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        name_label.setWordWrap(False)
        name_label.setStyleSheet("font-size: 10px; color: #cccccc;")
        name_label.setMaximumWidth(THUMB_SIZE + 8)
        layout.addWidget(name_label)

        # Badge label (winner / marked)
        self._badge_label = QLabel()
        self._badge_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self._badge_label.setFixedHeight(16)
        layout.addWidget(self._badge_label)

        self._update_style()

        # Load thumbnail in-place (fast enough for review step)
        if pixmap is not None:
            self._img_label.setPixmap(pixmap)
        else:
            fallback_pixmap = _load_pixmap(path, THUMB_SIZE)
            if fallback_pixmap is not None:
                self._img_label.setPixmap(fallback_pixmap)
            else:
                self._img_label.setText("?")

    def set_marked(self, marked: bool) -> None:
        if self.is_winner:
            return
        self._marked = marked
        self._update_style()

    def set_focused(self, focused: bool) -> None:
        self._focused = focused
        self._update_style()

    def _update_style(self) -> None:
        if self.is_winner:
            border_color = WINNER_BORDER_COLOR
            self._badge_label.setText("★ BEST")
            self._badge_label.setStyleSheet(
                f"color: {WINNER_BORDER_COLOR}; font-weight: bold; font-size: 11px;"
            )
        elif self._marked:
            border_color = MARKED_BORDER_COLOR
            self._badge_label.setText("✗ DELETE")
            self._badge_label.setStyleSheet(
                f"color: {MARKED_BORDER_COLOR}; font-weight: bold; font-size: 11px;"
            )
        else:
            border_color = (
                FOCUSED_BORDER_COLOR if self._focused else NEUTRAL_BORDER_COLOR
            )
            self._badge_label.setText("✓ KEEP")
            self._badge_label.setStyleSheet(
                "color: #66BB6A; font-weight: bold; font-size: 11px;"
                if not self._focused
                else "color: #4FC3F7; font-weight: bold; font-size: 11px;"
            )

        self.setStyleSheet(
            f"ThumbnailCard {{ border: 2px solid {border_color}; border-radius: 6px; "
            f"background-color: #2a2a2a; }}"
        )

    def mousePressEvent(self, event) -> None:
        if not self.is_winner:
            self._marked = not self._marked
            self._update_style()
            self.toggled.emit(self.path, self._marked)
        super().mousePressEvent(event)


class PickBestStepWidget(QWidget):
    """
    Step 2: Pick Best Photos.

    Shows similarity clusters with the AI-scored winner highlighted.
    Non-winners are pre-marked for deletion. User un-marks the ones to keep.
    Emits signals to integrate with app_controller.
    """

    skip_requested = pyqtSignal()
    proceed_to_cull_requested = pyqtSignal()
    mark_for_deletion_requested = pyqtSignal(list)  # list[str] paths
    unmark_for_deletion_requested = pyqtSignal(list)  # list[str] paths

    def __init__(self, parent: Optional[QWidget] = None) -> None:
        super().__init__(parent)
        self._clusters: List[Dict] = []  # list of cluster result dicts
        self._cluster_index: int = 0
        self._cards: List[ThumbnailCard] = []
        self._focused_card_index: int = 0
        self._create_widgets()
        self._connect_signals()

    # ------------------------------------------------------------------ #
    # Public API                                                           #
    # ------------------------------------------------------------------ #

    def show_loading(self, message: str = "Analysing…", percent: int = 0) -> None:
        self._stack.setCurrentWidget(self._page_loading)
        self._loading_label.setText(message)
        self._progress_bar.setValue(percent)

    def show_error(self, message: str) -> None:
        self._stack.setCurrentWidget(self._page_loading)
        self._loading_label.setText(f"Error: {message}")
        self._progress_bar.setValue(0)

    def show_results(self, results: Dict[int, dict]) -> None:
        """
        Populate the review page.

        ``results`` maps cluster_id → {'winner_path', 'ranked', 'failed', 'all_paths'}.
        Only clusters with a winner_path (i.e. ≥2 scored images) are shown.
        """
        self._clusters = [r for r in results.values() if r.get("winner_path")]
        if not self._clusters:
            self.show_loading(
                "No comparable clusters found.\nClick 'Done' to continue to Cull."
            )
            self._skip_btn_loading.setText("Done: Go to Cull →")
            self._skip_btn_loading.clicked.disconnect()
            self._skip_btn_loading.clicked.connect(self.proceed_to_cull_requested)
            return

        self._cluster_index = 0
        self._load_cluster(0)
        self._stack.setCurrentWidget(self._page_review)

    # ------------------------------------------------------------------ #
    # Private — widget construction                                        #
    # ------------------------------------------------------------------ #

    def _create_widgets(self) -> None:
        main_layout = QVBoxLayout(self)
        main_layout.setContentsMargins(0, 0, 0, 0)
        main_layout.setSpacing(0)

        self._stack = QStackedWidget()
        main_layout.addWidget(self._stack)

        # -- Loading page -- #
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

        # -- Review page -- #
        self._page_review = QWidget()
        review_layout = QVBoxLayout(self._page_review)
        review_layout.setContentsMargins(12, 8, 12, 8)
        review_layout.setSpacing(8)

        # Header bar
        header = QWidget()
        header_layout = QHBoxLayout(header)
        header_layout.setContentsMargins(0, 0, 0, 0)

        self._prev_btn = QPushButton("◀ Prev")
        self._prev_btn.setFixedWidth(80)
        header_layout.addWidget(self._prev_btn)

        self._cluster_info_label = QLabel()
        self._cluster_info_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self._cluster_info_label.setStyleSheet("font-size: 13px; font-weight: bold;")
        header_layout.addWidget(self._cluster_info_label, stretch=1)

        self._next_btn = QPushButton("Next ▶")
        self._next_btn.setFixedWidth(80)
        header_layout.addWidget(self._next_btn)

        review_layout.addWidget(header)

        # Keyboard hint
        hint = QLabel(
            "Click or D = toggle keep/delete  ·  ← → = navigate  ·  N = next cluster  ·  Ctrl+↵ = commit & next"
        )
        hint.setAlignment(Qt.AlignmentFlag.AlignCenter)
        hint.setStyleSheet("font-size: 11px; color: #888888;")
        review_layout.addWidget(hint)

        # Thumbnail scroll area
        self._scroll_area = QScrollArea()
        self._scroll_area.setWidgetResizable(True)
        self._scroll_area.setFrameShape(QFrame.Shape.NoFrame)
        self._scroll_area.setHorizontalScrollBarPolicy(
            Qt.ScrollBarPolicy.ScrollBarAsNeeded
        )
        self._scroll_area.setVerticalScrollBarPolicy(
            Qt.ScrollBarPolicy.ScrollBarAsNeeded
        )

        self._thumbs_container = QWidget()
        self._thumbs_grid = QGridLayout(self._thumbs_container)
        self._thumbs_grid.setContentsMargins(4, 4, 4, 4)
        self._thumbs_grid.setSpacing(8)
        self._scroll_area.setWidget(self._thumbs_container)
        review_layout.addWidget(self._scroll_area, stretch=1)

        # Action bar
        action_bar = QWidget()
        action_layout = QHBoxLayout(action_bar)
        action_layout.setContentsMargins(0, 4, 0, 4)

        self._keep_all_btn = QPushButton("Keep All")
        self._keep_all_btn.setToolTip("Unmark all images in this cluster")
        action_layout.addWidget(self._keep_all_btn)

        self._mark_rest_btn = QPushButton("Mark Non-Winners for Deletion")
        self._mark_rest_btn.setToolTip("Re-mark all non-winners for deletion")
        action_layout.addWidget(self._mark_rest_btn)

        self._delete_marked_btn = QPushButton("✗ Apply Deletions in Cluster")
        self._delete_marked_btn.setToolTip(
            "Commit marked deletions for this cluster and continue"
        )
        self._delete_marked_btn.setStyleSheet("color: #E53935; font-weight: bold;")
        action_layout.addWidget(self._delete_marked_btn)

        action_layout.addStretch()

        self._skip_btn_review = QPushButton("Skip Step →")
        action_layout.addWidget(self._skip_btn_review)

        self._done_btn = QPushButton("Done: Go to Cull →")
        self._done_btn.setStyleSheet("font-weight: bold;")
        action_layout.addWidget(self._done_btn)

        review_layout.addWidget(action_bar)

        self._stack.addWidget(self._page_review)

        # Start in loading state
        self._stack.setCurrentWidget(self._page_loading)

    def _connect_signals(self) -> None:
        self._skip_btn_loading.clicked.connect(self.skip_requested)
        self._skip_btn_review.clicked.connect(self.skip_requested)
        self._done_btn.clicked.connect(self._on_done)
        self._prev_btn.clicked.connect(self._prev_cluster)
        self._next_btn.clicked.connect(self._next_cluster)
        self._keep_all_btn.clicked.connect(self._keep_all)
        self._mark_rest_btn.clicked.connect(self._mark_non_winners)
        self._delete_marked_btn.clicked.connect(self._apply_and_next)

    # ------------------------------------------------------------------ #
    # Private — cluster review logic                                       #
    # ------------------------------------------------------------------ #

    def _load_cluster(self, index: int) -> None:
        if not self._clusters:
            return

        self._cluster_index = index
        cluster = self._clusters[index]
        winner_path = cluster.get("winner_path", "")
        all_paths: List[str] = cluster.get("all_paths", [])
        ranked: List[dict] = cluster.get("ranked", [])

        # Build score lookup
        score_by_path: Dict[str, Optional[float]] = {
            r["path"]: r.get("final_score") for r in ranked
        }

        # Sort: winner first, then by score desc, then unsupported at end
        def _sort_key(p: str):
            if p == winner_path:
                return (0, -(score_by_path.get(p) or 0.0))
            score = score_by_path.get(p)
            if score is not None:
                return (1, -score)
            return (2, 0.0)

        sorted_paths = sorted(all_paths, key=_sort_key)

        # Clear old grid
        self._cards.clear()
        while self._thumbs_grid.count():
            item = self._thumbs_grid.takeAt(0)
            if item.widget():
                item.widget().deleteLater()

        # Populate grid
        for i, path in enumerate(sorted_paths):
            is_winner = path == winner_path
            score = score_by_path.get(path)
            pixmap = None
            image_pipeline = getattr(self.window(), "image_pipeline", None)
            if image_pipeline is not None:
                try:
                    pixmap = image_pipeline.get_thumbnail_qpixmap(
                        path, apply_orientation=True
                    )
                except Exception as exc:
                    logger.debug("Could not load thumbnail via image pipeline for %s: %s", path, exc)
            card = ThumbnailCard(
                path,
                is_winner=is_winner,
                score=score,
                pixmap=pixmap,
                parent=self._thumbs_container,
            )
            # Pre-mark non-winners for deletion
            if not is_winner:
                card.set_marked(True)
            card.toggled.connect(self._on_card_toggled)
            row, col = divmod(i, CARDS_PER_ROW)
            self._thumbs_grid.addWidget(card, row, col)
            self._cards.append(card)

        # Update header
        total_clusters = len(self._clusters)
        n_images = len(all_paths)
        self._cluster_info_label.setText(
            f"Cluster {index + 1} / {total_clusters}  •  {n_images} photos"
        )
        self._prev_btn.setEnabled(index > 0)
        self._next_btn.setEnabled(index < total_clusters - 1)

        # Set keyboard focus
        self._focused_card_index = 0
        self._update_focus()
        self.setFocus()

    def _update_focus(self) -> None:
        for i, card in enumerate(self._cards):
            card.set_focused(i == self._focused_card_index)

    def _on_card_toggled(self, path: str, is_marked: bool) -> None:
        """Called when a card is clicked."""
        if is_marked:
            self.mark_for_deletion_requested.emit([path])
        else:
            self.unmark_for_deletion_requested.emit([path])

    def _keep_all(self) -> None:
        paths_to_unmark = []
        for card in self._cards:
            if not card.is_winner and card._marked:
                paths_to_unmark.append(card.path)
                card.set_marked(False)
        if paths_to_unmark:
            self.unmark_for_deletion_requested.emit(paths_to_unmark)

    def _mark_non_winners(self) -> None:
        paths_to_mark = []
        for card in self._cards:
            if not card.is_winner and not card._marked:
                paths_to_mark.append(card.path)
                card.set_marked(True)
        if paths_to_mark:
            self.mark_for_deletion_requested.emit(paths_to_mark)

    def _commit_current_cluster_marks(self) -> None:
        """Emit mark signals for the current state of all cards."""
        to_mark = [c.path for c in self._cards if not c.is_winner and c._marked]
        to_unmark = [c.path for c in self._cards if not c.is_winner and not c._marked]
        if to_mark:
            self.mark_for_deletion_requested.emit(to_mark)
        if to_unmark:
            self.unmark_for_deletion_requested.emit(to_unmark)

    def _apply_and_next(self) -> None:
        self._commit_current_cluster_marks()
        self._next_cluster()

    def _next_cluster(self) -> None:
        if self._cluster_index < len(self._clusters) - 1:
            self._load_cluster(self._cluster_index + 1)
        # If on the last cluster, do nothing (user presses Done)

    def _prev_cluster(self) -> None:
        if self._cluster_index > 0:
            self._load_cluster(self._cluster_index - 1)

    def _on_done(self) -> None:
        self._commit_current_cluster_marks()
        self.proceed_to_cull_requested.emit()

    # ------------------------------------------------------------------ #
    # Keyboard handling                                                    #
    # ------------------------------------------------------------------ #

    def keyPressEvent(self, event: QKeyEvent) -> None:
        key = event.key()
        modifiers = event.modifiers()

        if key == Qt.Key.Key_D or key == Qt.Key.Key_Space:
            self._toggle_focused_card()
        elif key == Qt.Key.Key_Right or key == Qt.Key.Key_Tab:
            if key == Qt.Key.Key_Tab and modifiers & Qt.KeyboardModifier.ShiftModifier:
                self._move_focus(-1)
            else:
                self._move_focus(1)
        elif key == Qt.Key.Key_Left or key == Qt.Key.Key_Backtab:
            self._move_focus(-1)
        elif key == Qt.Key.Key_N:
            self._next_cluster()
        elif key == Qt.Key.Key_P:
            self._prev_cluster()
        elif (
            key == Qt.Key.Key_Return and modifiers & Qt.KeyboardModifier.ControlModifier
        ):
            self._apply_and_next()
        else:
            super().keyPressEvent(event)

    def _toggle_focused_card(self) -> None:
        if 0 <= self._focused_card_index < len(self._cards):
            card = self._cards[self._focused_card_index]
            if not card.is_winner:
                new_marked = not card._marked
                card.set_marked(new_marked)
                if new_marked:
                    self.mark_for_deletion_requested.emit([card.path])
                else:
                    self.unmark_for_deletion_requested.emit([card.path])

    def _move_focus(self, delta: int) -> None:
        if not self._cards:
            return
        new_index = max(0, min(len(self._cards) - 1, self._focused_card_index + delta))
        if new_index != self._focused_card_index:
            self._focused_card_index = new_index
            self._update_focus()

from __future__ import annotations

import logging
import os
from fractions import Fraction
from typing import Callable, Dict, List, Optional

from PyQt6.QtCore import Qt, pyqtSignal
from PyQt6.QtGui import QKeyEvent
from PyQt6.QtGui import QKeySequence, QShortcut
from PyQt6.QtWidgets import (
    QFrame,
    QGridLayout,
    QHBoxLayout,
    QLabel,
    QProgressBar,
    QPushButton,
    QSizePolicy,
    QStackedWidget,
    QVBoxLayout,
    QWidget,
)

from core.metadata_processor import MetadataProcessor
from ui.advanced_image_viewer import SynchronizedImageViewer

logger = logging.getLogger(__name__)

WINNER_BORDER_COLOR = "#F5B700"
MARKED_BORDER_COLOR = "#E53935"
KEEP_BORDER_COLOR = "#66BB6A"
FOCUSED_BORDER_COLOR = "#4FC3F7"
CARD_BG = "#20252C"
CARD_BG_WINNER = "#2C2616"


def _first_present(metadata: dict, *keys: str):
    for key in keys:
        value = metadata.get(key)
        if value not in (None, "", "None"):
            return value
    return None


def _fraction_text(value: object) -> Optional[str]:
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
    except (TypeError, ValueError, ZeroDivisionError):
        return text
    if numeric >= 1:
        return f"{numeric:.1f}s"
    if numeric <= 0:
        return text
    return f"1/{round(1 / numeric)}s"


def _float_text(
    value: object, prefix: str = "", suffix: str = "", digits: int = 1
) -> Optional[str]:
    if value in (None, ""):
        return None
    try:
        return f"{prefix}{float(value):.{digits}f}{suffix}"
    except (TypeError, ValueError):
        return f"{prefix}{value}{suffix}"


class CompareCard(QFrame):
    toggled = pyqtSignal(str, bool)

    def __init__(self, slot_number: int, parent: Optional[QWidget] = None) -> None:
        super().__init__(parent)
        self.path: str = ""
        self.is_winner = False
        self._marked = False
        self._focused = False
        self._slot_number = slot_number

        self.setFrameShape(QFrame.Shape.StyledPanel)
        self.setSizePolicy(QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Preferred)
        self.setCursor(Qt.CursorShape.PointingHandCursor)

        layout = QVBoxLayout(self)
        layout.setContentsMargins(12, 10, 12, 10)
        layout.setSpacing(8)

        top_row = QHBoxLayout()
        top_row.setContentsMargins(0, 0, 0, 0)
        top_row.setSpacing(8)

        self._slot_label = QLabel(f"{slot_number}")
        self._slot_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self._slot_label.setFixedSize(22, 22)
        self._slot_label.setStyleSheet(
            "border-radius: 11px; background: #11161C; color: #B8C2CC; font-weight: bold;"
        )
        top_row.addWidget(self._slot_label, alignment=Qt.AlignmentFlag.AlignLeft)

        self._state_label = QLabel()
        self._state_label.setAlignment(
            Qt.AlignmentFlag.AlignRight | Qt.AlignmentFlag.AlignVCenter
        )
        top_row.addWidget(self._state_label, stretch=1)
        layout.addLayout(top_row)

        self._name_label = QLabel("")
        self._name_label.setWordWrap(False)
        self._name_label.setStyleSheet("font-size: 12px; font-weight: 600; color: #F5F7FA;")
        layout.addWidget(self._name_label)

        self._score_label = QLabel("")
        self._score_label.setStyleSheet("font-size: 11px; color: #AAB4BE;")
        layout.addWidget(self._score_label)

        self._meta_grid = QGridLayout()
        self._meta_grid.setContentsMargins(0, 0, 0, 0)
        self._meta_grid.setHorizontalSpacing(10)
        self._meta_grid.setVerticalSpacing(4)
        layout.addLayout(self._meta_grid)

        self._meta_rows: list[tuple[QLabel, QLabel]] = []
        for row in range(5):
            key = QLabel("")
            key.setStyleSheet("font-size: 10px; color: #7D8792;")
            value = QLabel("")
            value.setStyleSheet("font-size: 10px; color: #D5DBE1;")
            self._meta_grid.addWidget(key, row, 0)
            self._meta_grid.addWidget(value, row, 1)
            self._meta_rows.append((key, value))

        self._hint_label = QLabel("Click image/card or press number key")
        self._hint_label.setStyleSheet("font-size: 10px; color: #6D7782;")
        layout.addWidget(self._hint_label)

        self._update_style()

    def configure(
        self,
        *,
        path: str,
        is_winner: bool,
        marked: bool,
        score: Optional[float],
        failure_reason: Optional[str],
        metadata_rows: list[tuple[str, str]],
    ) -> None:
        self.path = path
        self.is_winner = is_winner
        self._marked = marked
        self._name_label.setText(os.path.basename(path))
        if score is None:
            self._score_label.setText("Score unavailable")
            self._score_label.setToolTip(failure_reason or "")
        else:
            self._score_label.setText(f"Final score {score:.3f}")
            self._score_label.setToolTip("")

        for idx, (key_label, value_label) in enumerate(self._meta_rows):
            if idx < len(metadata_rows):
                key_text, value_text = metadata_rows[idx]
                key_label.setText(key_text)
                value_label.setText(value_text)
                key_label.show()
                value_label.show()
            else:
                key_label.hide()
                value_label.hide()

        self._update_style()

    def set_marked(self, marked: bool) -> None:
        self._marked = marked
        self._update_style()

    def set_focused(self, focused: bool) -> None:
        self._focused = focused
        self._update_style()

    def mousePressEvent(self, event) -> None:
        if event.button() == Qt.MouseButton.LeftButton and self.path:
            self._toggle()
            event.accept()
            return
        super().mousePressEvent(event)

    def _toggle(self) -> None:
        if not self.path:
            return
        self._marked = not self._marked
        self._update_style()
        self.toggled.emit(self.path, self._marked)

    def set_info_visible(self, visible: bool) -> None:
        self._score_label.setVisible(visible)
        self._hint_label.setVisible(visible)
        for key_label, value_label in self._meta_rows:
            key_label.setVisible(visible)
            value_label.setVisible(visible)

    def _update_style(self) -> None:
        if self.is_winner:
            border_color = WINNER_BORDER_COLOR
            bg = CARD_BG_WINNER
            status = "BEST DELETE" if self._marked else "BEST KEEP"
            color = MARKED_BORDER_COLOR if self._marked else WINNER_BORDER_COLOR
            self._state_label.setText(status)
            self._state_label.setStyleSheet(
                f"font-size: 11px; font-weight: bold; color: {color};"
            )
            self._hint_label.setText(
                f"Best candidate on the right. Click or press {self._slot_number} to toggle"
            )
            self.setCursor(Qt.CursorShape.PointingHandCursor)
        else:
            border_color = MARKED_BORDER_COLOR if self._marked else KEEP_BORDER_COLOR
            if self._focused:
                border_color = FOCUSED_BORDER_COLOR
            bg = CARD_BG
            status = "DELETE" if self._marked else "KEEP"
            color = MARKED_BORDER_COLOR if self._marked else KEEP_BORDER_COLOR
            self._state_label.setText(status)
            self._state_label.setStyleSheet(
                f"font-size: 11px; font-weight: bold; color: {color};"
            )
            self._hint_label.setText(
                f"Click image/card or press {self._slot_number} to toggle"
            )
            self.setCursor(Qt.CursorShape.PointingHandCursor)

        self.setStyleSheet(
            f"CompareCard {{"
            f"border: 2px solid {border_color};"
            f"border-radius: 10px;"
            f"background: {bg};"
            f"}}"
        )


class PickBestStepWidget(QWidget):
    skip_requested = pyqtSignal()
    proceed_to_cull_requested = pyqtSignal()
    mark_for_deletion_requested = pyqtSignal(list)
    unmark_for_deletion_requested = pyqtSignal(list)

    def __init__(self, parent: Optional[QWidget] = None) -> None:
        super().__init__(parent)
        self._clusters: List[Dict] = []
        self._cluster_index = 0
        self._subset_index = 0
        self._subset_paths: List[str] = []
        self._compare_cards: List[CompareCard] = []
        self._focused_slot_index = 0
        self._current_winner_path = ""
        self._current_all_paths: List[str] = []
        self._cluster_ordered_paths: List[str] = []
        self._cluster_mark_state: Dict[str, bool] = {}
        self._metadata_cache: Dict[str, list[tuple[str, str]]] = {}
        self._focus_mode = False
        self._current_images_data: List[Dict] = []
        self._info_visible = True
        self._create_widgets()
        self._connect_signals()
        self._create_shortcuts()

    def show_loading(self, message: str = "Analysing…", percent: int = 0) -> None:
        self._stack.setCurrentWidget(self._page_loading)
        self._loading_label.setText(message)
        self._progress_bar.setValue(percent)

    def show_error(self, message: str) -> None:
        self._stack.setCurrentWidget(self._page_loading)
        self._loading_label.setText(f"Error: {message}")
        self._progress_bar.setValue(0)

    def show_results(self, results: Dict[int, dict]) -> None:
        self._clusters = [r for r in results.values() if r.get("winner_path")]
        self._metadata_cache.clear()
        if not self._clusters:
            self.show_loading(
                "No comparable clusters found.\nClick 'Done' to continue to Cull."
            )
            self._skip_btn_loading.setText("Done: Go to Cull →")
            try:
                self._skip_btn_loading.clicked.disconnect()
            except TypeError:
                pass
            self._skip_btn_loading.clicked.connect(self.proceed_to_cull_requested)
            return

        self._cluster_index = 0
        self._load_cluster(0)
        self._stack.setCurrentWidget(self._page_review)

    def set_is_marked_func(self, func: Callable[[str], bool]) -> None:
        self._sync_viewer.set_is_marked_for_deletion_func(func)

    def set_has_any_marked_func(self, func: Callable[[], bool]) -> None:
        self._sync_viewer.set_has_any_marked_for_deletion_func(func)

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
        review_layout = QVBoxLayout(self._page_review)
        review_layout.setContentsMargins(10, 8, 10, 8)
        review_layout.setSpacing(8)

        header = QWidget()
        header_layout = QHBoxLayout(header)
        header_layout.setContentsMargins(0, 0, 0, 0)
        header_layout.setSpacing(8)

        self._prev_cluster_btn = QPushButton("◀ Prev Cluster")
        self._prev_cluster_btn.setFixedWidth(110)
        header_layout.addWidget(self._prev_cluster_btn)

        self._cluster_info_label = QLabel()
        self._cluster_info_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self._cluster_info_label.setStyleSheet("font-size: 13px; font-weight: bold;")
        header_layout.addWidget(self._cluster_info_label, stretch=1)

        self._next_cluster_btn = QPushButton("Next Cluster ▶")
        self._next_cluster_btn.setFixedWidth(110)
        header_layout.addWidget(self._next_cluster_btn)
        review_layout.addWidget(header)

        self._subset_info_label = QLabel()
        self._subset_info_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self._subset_info_label.setStyleSheet("font-size: 11px; color: #92A0AD;")
        review_layout.addWidget(self._subset_info_label)

        self._hint_label = QLabel(
            "Up to 3 images per round. Winner stays on the right."
            " Press 1, 2, 3 to toggle keep/delete — C to focus an image, i to hide/show info."
        )
        self._hint_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self._hint_label.setWordWrap(True)
        self._hint_label.setStyleSheet("font-size: 11px; color: #888888;")
        review_layout.addWidget(self._hint_label)

        self._sync_viewer = SynchronizedImageViewer()
        self._sync_viewer.controls_frame.hide()
        review_layout.addWidget(self._sync_viewer, stretch=1)

        self._cards_row = QWidget()
        self._cards_layout = QHBoxLayout(self._cards_row)
        self._cards_layout.setContentsMargins(0, 0, 0, 0)
        self._cards_layout.setSpacing(10)
        for slot in range(3):
            card = CompareCard(slot + 1, self._cards_row)
            self._cards_layout.addWidget(card)
            self._compare_cards.append(card)
        review_layout.addWidget(self._cards_row)

        action_bar = QWidget()
        action_layout = QHBoxLayout(action_bar)
        action_layout.setContentsMargins(0, 2, 0, 2)
        action_layout.setSpacing(8)

        self._prev_set_btn = QPushButton("◀ Prev Set")
        action_layout.addWidget(self._prev_set_btn)

        self._keep_all_btn = QPushButton("Keep Visible")
        self._keep_all_btn.setToolTip("Unmark the visible non-winners")
        action_layout.addWidget(self._keep_all_btn)

        self._mark_rest_btn = QPushButton("Delete Visible")
        self._mark_rest_btn.setToolTip("Mark the visible non-winners for deletion")
        action_layout.addWidget(self._mark_rest_btn)

        self._next_set_btn = QPushButton("Next Set ▶")
        action_layout.addWidget(self._next_set_btn)

        action_layout.addStretch()

        self._skip_btn_review = QPushButton("Skip Step →")
        action_layout.addWidget(self._skip_btn_review)

        self._done_btn = QPushButton("Done: Go to Cull →")
        self._done_btn.setStyleSheet("font-weight: bold;")
        action_layout.addWidget(self._done_btn)

        review_layout.addWidget(action_bar)
        self._stack.addWidget(self._page_review)
        self._stack.setCurrentWidget(self._page_loading)

    def _connect_signals(self) -> None:
        self._skip_btn_loading.clicked.connect(self.skip_requested)
        self._skip_btn_review.clicked.connect(self.skip_requested)
        self._done_btn.clicked.connect(self._on_done)
        self._prev_cluster_btn.clicked.connect(self._prev_cluster)
        self._next_cluster_btn.clicked.connect(self._next_cluster)
        self._prev_set_btn.clicked.connect(self._prev_subset)
        self._next_set_btn.clicked.connect(self._next_subset)
        self._keep_all_btn.clicked.connect(self._keep_visible)
        self._mark_rest_btn.clicked.connect(self._delete_visible)

        self._sync_viewer.markAsDeletedRequested.connect(self._on_viewer_mark)
        self._sync_viewer.unmarkAsDeletedRequested.connect(self._on_viewer_unmark)
        self._sync_viewer.markOthersAsDeletedRequested.connect(self._on_viewer_mark_others)
        self._sync_viewer.unmarkOthersAsDeletedRequested.connect(
            self._on_viewer_unmark_others
        )
        self._sync_viewer.imageClicked.connect(self._on_viewer_clicked)
        self._sync_viewer.installEventFilter(self)

        for card in self._compare_cards:
            card.toggled.connect(self._on_card_toggled)
    
    def _create_shortcuts(self) -> None:
        self._shortcuts: List[QShortcut] = []
        bindings = [
            ("1", lambda: self._activate_slot_shortcut(0)),
            ("2", lambda: self._activate_slot_shortcut(1)),
            ("3", lambda: self._activate_slot_shortcut(2)),
            ("Left", self._prev_cluster),
            ("Right", self._next_cluster),
            ("C", self._toggle_focus_mode),
            ("i", self._toggle_info),
        ]
        for key_text, handler in bindings:
            shortcut = QShortcut(QKeySequence(key_text), self)
            shortcut.setContext(Qt.ShortcutContext.WidgetWithChildrenShortcut)
            shortcut.activated.connect(handler)
            self._shortcuts.append(shortcut)

    def _load_cluster(self, index: int) -> None:
        if not self._clusters:
            return

        self._cluster_index = index
        cluster = self._clusters[index]
        winner_path = cluster.get("winner_path", "")
        all_paths: List[str] = cluster.get("all_paths", [])
        ranked: List[dict] = cluster.get("ranked", [])

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
        saved_mark_state = cluster.get("_mark_state")
        if isinstance(saved_mark_state, dict):
            self._cluster_mark_state = {
                path: bool(saved_mark_state.get(path, path != winner_path))
                for path in self._cluster_ordered_paths
            }
        else:
            self._cluster_mark_state = {
                path: path != winner_path for path in self._cluster_ordered_paths
            }
        self._subset_index = 0
        self._update_cluster_ui(score_by_path, failure_reason_by_path)

    def _update_cluster_ui(
        self,
        score_by_path: Dict[str, Optional[float]],
        failure_reason_by_path: Dict[str, str],
    ) -> None:
        self._show_subset(score_by_path, failure_reason_by_path)
        total_clusters = len(self._clusters)
        total_sets = self._subset_count()
        visible_kept = sum(
            1
            for path, marked in self._cluster_mark_state.items()
            if path != self._current_winner_path and not marked
        )
        visible_deleted = sum(
            1
            for path, marked in self._cluster_mark_state.items()
            if path != self._current_winner_path and marked
        )
        self._cluster_info_label.setText(
            f"Cluster {self._cluster_index + 1} / {total_clusters}  •  {len(self._current_all_paths)} photos  •  {visible_kept} keep / {visible_deleted} delete"
        )
        self._subset_info_label.setText(
            f"Set {self._subset_index + 1} / {total_sets}  •  top challengers on the left, winner locked on the right"
        )
        self._prev_cluster_btn.setEnabled(self._cluster_index > 0)
        self._next_cluster_btn.setEnabled(self._cluster_index < total_clusters - 1)
        self._prev_set_btn.setEnabled(self._subset_index > 0)
        self._next_set_btn.setEnabled(self._subset_index < total_sets - 1)

    def _show_subset(
        self,
        score_by_path: Dict[str, Optional[float]],
        failure_reason_by_path: Dict[str, str],
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
        for path in subset_paths:
            pixmap = None
            if image_pipeline is not None:
                try:
                    pixmap = image_pipeline.get_cached_preview_qpixmap(path)
                    if pixmap is None:
                        pixmap = image_pipeline.get_preview_qpixmap(
                            path, display_max_size=None
                        )
                    if pixmap is None:
                        pixmap = image_pipeline.get_thumbnail_qpixmap(
                            path, apply_orientation=True
                        )
                except Exception as exc:
                    logger.debug("Could not load preview for %s: %s", path, exc)
            images_data.append({"path": path, "pixmap": pixmap, "rating": 0})

        self._current_images_data = images_data
        self._sync_viewer.set_images_data(images_data)
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

    def _metadata_rows_for_path(
        self, path: str, *, failure_reason: Optional[str] = None
    ) -> list[tuple[str, str]]:
        if path not in self._metadata_cache:
            self._metadata_cache[path] = self._build_metadata_rows(path)

        rows = list(self._metadata_cache[path])
        if failure_reason:
            rows.insert(0, ("Scoring", failure_reason))
        return rows[:5]

    def _build_metadata_rows(self, path: str) -> list[tuple[str, str]]:
        rows: list[tuple[str, str]] = []
        metadata = None
        app_state = getattr(self.window(), "app_state", None)
        cache = getattr(app_state, "exif_disk_cache", None) if app_state else None
        try:
            metadata = MetadataProcessor.get_detailed_metadata(path, cache)
        except Exception:
            logger.debug("EXIF lookup failed for %s", path, exc_info=True)

        if isinstance(metadata, dict):
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
                rows.append(("Lens", "  ".join(part for part in (focal, aperture) if part)))

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
                    ("Exposure", "  ".join(part for part in (shutter, iso_text) if part))
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

        self._metadata_cache[path] = rows[:5]
        return self._metadata_cache[path]

    def _cluster_score_maps(
        self, cluster: Dict
    ) -> tuple[Dict[str, Optional[float]], Dict[str, str]]:
        ranked: List[dict] = cluster.get("ranked", [])
        failed: List[dict] = cluster.get("failed", [])
        score_by_path: Dict[str, Optional[float]] = {
            entry["path"]: entry.get("final_score") for entry in ranked
        }
        failure_reason_by_path: Dict[str, str] = {}
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
        self._cluster_mark_state[path] = marked
        for card in self._compare_cards:
            if card.isVisible() and card.path == path:
                card.set_marked(marked)
        self._update_cluster_header_only()

    def _update_cluster_header_only(self) -> None:
        total_clusters = len(self._clusters)
        kept = sum(
            1
            for path, marked in self._cluster_mark_state.items()
            if path != self._current_winner_path and not marked
        )
        deleted = sum(
            1
            for path, marked in self._cluster_mark_state.items()
            if path != self._current_winner_path and marked
        )
        self._cluster_info_label.setText(
            f"Cluster {self._cluster_index + 1} / {total_clusters}  •  {len(self._current_all_paths)} photos  •  {kept} keep / {deleted} delete"
        )

    def _subset_count(self) -> int:
        non_winner_count = len(
            [path for path in self._cluster_ordered_paths if path != self._current_winner_path]
        )
        return max(1, (non_winner_count + 1) // 2)

    def _visible_paths(self) -> List[str]:
        return [path for path in self._subset_paths if path]

    def _commit_current_cluster_marks(self) -> None:
        if 0 <= self._cluster_index < len(self._clusters):
            self._clusters[self._cluster_index]["_mark_state"] = dict(
                self._cluster_mark_state
            )
        to_mark = [
            path
            for path, marked in self._cluster_mark_state.items()
            if marked
        ]
        to_unmark = [
            path
            for path, marked in self._cluster_mark_state.items()
            if not marked
        ]
        if to_mark:
            self.mark_for_deletion_requested.emit(to_mark)
        if to_unmark:
            self.unmark_for_deletion_requested.emit(to_unmark)

    def _next_cluster(self) -> None:
        if self._cluster_index < len(self._clusters) - 1:
            self._exit_focus_mode()
            self._commit_current_cluster_marks()
            self._load_cluster(self._cluster_index + 1)

    def _prev_cluster(self) -> None:
        if self._cluster_index > 0:
            self._exit_focus_mode()
            self._commit_current_cluster_marks()
            self._load_cluster(self._cluster_index - 1)

    def _next_subset(self) -> None:
        max_subset = self._subset_count() - 1
        if self._subset_index < max_subset:
            self._subset_index += 1
            cluster = self._clusters[self._cluster_index]
            score_by_path, failure_reason_by_path = self._cluster_score_maps(cluster)
            self._update_cluster_ui(score_by_path, failure_reason_by_path)

    def _prev_subset(self) -> None:
        if self._subset_index > 0:
            self._subset_index -= 1
            cluster = self._clusters[self._cluster_index]
            score_by_path, failure_reason_by_path = self._cluster_score_maps(cluster)
            self._update_cluster_ui(score_by_path, failure_reason_by_path)

    def _keep_visible(self) -> None:
        paths = self._visible_paths()
        for path in paths:
            self._set_path_marked(path, False)
        if paths:
            self.unmark_for_deletion_requested.emit(paths)

    def _delete_visible(self) -> None:
        paths = self._visible_paths()
        for path in paths:
            self._set_path_marked(path, True)
        if paths:
            self.mark_for_deletion_requested.emit(paths)

    def _toggle_slot(self, slot_index: int) -> None:
        if not (0 <= slot_index < len(self._subset_paths)):
            return
        path = self._subset_paths[slot_index]
        if not path:
            return
        new_marked = not self._cluster_mark_state.get(path, True)
        self._set_path_marked(path, new_marked)
        if new_marked:
            self.mark_for_deletion_requested.emit([path])
        else:
            self.unmark_for_deletion_requested.emit([path])

    def _on_card_toggled(self, path: str, is_marked: bool) -> None:
        self._set_path_marked(path, is_marked)
        if is_marked:
            self.mark_for_deletion_requested.emit([path])
        else:
            self.unmark_for_deletion_requested.emit([path])

    def _on_viewer_clicked(self, slot_index: int, _path: str) -> None:
        self._focused_slot_index = slot_index
        self._update_focus_state()
        self._toggle_slot(slot_index)

    def _on_viewer_mark(self, path: str) -> None:
        self._set_path_marked(path, True)
        self.mark_for_deletion_requested.emit([path])

    def _on_viewer_unmark(self, path: str) -> None:
        self._set_path_marked(path, False)
        self.unmark_for_deletion_requested.emit([path])

    def _on_viewer_mark_others(self, keeper_path: str) -> None:
        paths = [path for path in self._visible_paths() if path != keeper_path]
        for path in paths:
            self._set_path_marked(path, True)
        if paths:
            self.mark_for_deletion_requested.emit(paths)

    def _on_viewer_unmark_others(self, keeper_path: str) -> None:
        paths = [path for path in self._visible_paths() if path != keeper_path]
        for path in paths:
            self._set_path_marked(path, False)
        if paths:
            self.unmark_for_deletion_requested.emit(paths)

    def _on_done(self) -> None:
        self._commit_current_cluster_marks()
        self.proceed_to_cull_requested.emit()

    def keyPressEvent(self, event: QKeyEvent) -> None:
        key = event.key()

        if key in (Qt.Key.Key_1, Qt.Key.Key_2, Qt.Key.Key_3):
            slot = key - Qt.Key.Key_1
            self._focused_slot_index = slot
            self._update_focus_state()
            if self._focus_mode:
                self._sync_viewer.set_focused_viewer(slot)
            else:
                self._toggle_slot(slot)
        elif key == Qt.Key.Key_Left:
            self._prev_cluster()
        elif key == Qt.Key.Key_Right:
            self._next_cluster()
        elif key == Qt.Key.Key_C:
            self._toggle_focus_mode()
        elif key == Qt.Key.Key_I:
            self._toggle_info()
        else:
            super().keyPressEvent(event)

    def _activate_slot_shortcut(self, slot_index: int) -> None:
        self._focused_slot_index = slot_index
        self._update_focus_state()
        if self._focus_mode:
            self._sync_viewer.set_focused_viewer(slot_index)
        else:
            self._toggle_slot(slot_index)

    def _exit_focus_mode(self) -> None:
        """Reset to compare mode (flag + hint). No-op if already in compare mode."""
        if not self._focus_mode:
            return
        self._focus_mode = False
        self._hint_label.setText(
            "Up to 3 images per round. Winner stays on the right."
            " Press 1, 2, 3 to toggle keep/delete — C to focus an image, i to hide/show info."
        )

    def _toggle_focus_mode(self) -> None:
        if not self._focus_mode:
            self._focus_mode = True
            slot = min(self._focused_slot_index, max(0, len(self._subset_paths) - 1))
            self._sync_viewer.set_focused_viewer(slot)
            self._hint_label.setText(
                "Focus mode: press 1, 2, 3 to switch between images."
                " Press C to return to side-by-side compare."
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

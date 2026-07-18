import logging
import os
from typing import override

from PyQt6.QtCore import Qt, QTimer, pyqtSignal, QUrl
from PyQt6.QtGui import QColor, QPixmap, QTransform, QDesktopServices
from PyQt6.QtWidgets import (
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
from core.runtime_paths import get_app_models_dir, is_frozen_runtime
from core.app_settings import ROTATION_MODEL_DOWNLOAD_URL

from ui.workflow_review_components import (
    FIX_ROTATION_SHORTCUTS,
    WorkflowReviewListPanel,
    WorkflowStateBanner,
    install_workflow_shortcuts,
)

logger = logging.getLogger(__name__)

_ANGLE_LABELS: dict[int, tuple] = {
    90: ("90° CW", "#00D4FF"),
    180: ("180°", "#F5B700"),
    -90: ("90° CCW", "#00D4FF"),
}

_UNMARKED_COLOR = "#B9C2C9"
_MARKED_COLOR = "#66BB6A"
_SKIP_COLOR = "#607080"


class _RotatedImageLabel(QLabel):
    """QLabel that displays an image and an optional rotation-preview overlay."""

    clicked = pyqtSignal()

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self._source_pixmap: QPixmap | None = None
        self._preview_angle: int = 0
        self.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.setMinimumSize(80, 80)
        self.setSizePolicy(QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Expanding)
        self.setStyleSheet("background: #0D1117;")

    def set_pixmap_and_angle(
        self, pixmap: QPixmap | None, preview_angle: int = 0
    ) -> None:
        self._source_pixmap = pixmap
        self._preview_angle = preview_angle
        self._refresh()

    @override
    def resizeEvent(self, event) -> None:
        super().resizeEvent(event)
        self._refresh()

    @override
    def mouseReleaseEvent(self, event) -> None:
        if event.button() == Qt.MouseButton.LeftButton:
            self.clicked.emit()
        super().mouseReleaseEvent(event)

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
    active_image_changed = pyqtSignal(str)
    proceed_requested = pyqtSignal()
    skip_requested = pyqtSignal()
    retry_requested = pyqtSignal()

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self._suggestions: dict[str, int] = {}  # path -> suggested angle
        self._shown_suggestions: dict[str, int] | None = None
        self._angle_overrides: dict[str, int] = {}  # path -> manual preview angle
        self._marked: dict[str, bool] = {}  # path -> currently selected choice
        self._confirmed: set[str] = set()
        self._ordered_paths: list[str] = []
        self._current_index: int = -1
        self._syncing_active_image = False
        self._image_pipeline = None
        self._applying = False
        self._submitted_paths: set[str] = set()
        self._successful_paths: set[str] = set()
        self._failed_paths: set[str] = set()
        self.setFocusPolicy(Qt.FocusPolicy.ClickFocus)
        self._setup_ui()
        self._shortcuts = install_workflow_shortcuts(
            self,
            FIX_ROTATION_SHORTCUTS,
            {
                "toggle": self._on_mark_toggle,
                "rotate_clockwise": self._on_rotate_clockwise,
                "previous": self._on_prev,
                "next": self._on_next,
                "primary": self._on_confirm,
                "skip": self._on_skip,
            },
        )

    def set_image_pipeline(self, pipeline) -> None:
        self._image_pipeline = pipeline

    def pending_rotations(self) -> dict[str, int]:
        """Return the currently queued, unapplied rotation changes."""
        if self._applying:
            return {}
        return {
            path: self._selected_angle(path)
            for path in self._ordered_paths
            if path in self._confirmed
            and self._marked.get(path, False)
            and self._selected_angle(path) != 0
        }

    def discard_pending_rotations(self) -> None:
        """Keep detection results while clearing every queued file mutation."""
        for path in self._ordered_paths:
            self._marked[path] = False
        self._confirmed.clear()
        if self._ordered_paths and self._current_index >= 0:
            self._show_current()
        self._refresh_controls()

    def apply_pending_rotations(self) -> None:
        """Apply the current queue through the widget's normal state machine."""
        self._on_apply()

    # ------------------------------------------------------------------
    # Public state-machine API
    # ------------------------------------------------------------------

    def show_loading(self, message: str = "", percent: int = -1) -> None:
        self._loading_label.setText(message or "Analyzing rotation…")
        self._missing_model_widget.setVisible(False)
        self._progress_bar.setVisible(True)
        if percent < 0:
            self._progress_bar.setRange(0, 0)
        else:
            self._progress_bar.setRange(0, 100)
            self._progress_bar.setValue(percent)
        self._content_stack.setCurrentIndex(0)

    def show_error(self, message: str) -> None:
        self._loading_label.setText(f"Error: {message}")
        self._missing_model_widget.setVisible(False)
        self._progress_bar.setVisible(True)
        self._progress_bar.setRange(0, 100)
        self._progress_bar.setValue(0)
        self._content_stack.setCurrentIndex(0)

    def show_model_not_found(self, message: str) -> None:
        self._loading_label.setText(
            "Rotation model not found. Follow the instructions below to install it."
        )
        self._model_path_label.setText(message)
        
        instructions_text = (
            "<p style='color: #a9b7c6; font-size: 13px; font-weight: bold; margin-bottom: 8px; text-align: left;'>"
            "Follow these steps to enable this feature:</p>"
            "<ol style='color: #bbbbbb; font-size: 12px; line-height: 1.6; margin-left: 20px; text-align: left;'>"
            "<li>Click <b>Download Model</b> to open the GitHub releases page.</li>"
            "<li>Download the latest <b>orientation_model.onnx</b> file.</li>"
            "<li>Click <b>Open Models Folder</b> and place the downloaded file there.</li>"
            "<li><b>Restart</b> the application or re-run the rotation analysis.</li>"
            "</ol>"
        )
        self._instructions_label.setText(instructions_text)
        
        self._missing_model_widget.setVisible(True)
        self._progress_bar.setVisible(False)
        self._content_stack.setCurrentIndex(0)

    def show_results(self, suggestions: dict[str, int]) -> None:
        if (
            self._shown_suggestions is not None
            and suggestions == self._shown_suggestions
        ):
            if self._ordered_paths:
                self._content_stack.setCurrentIndex(1)
                self._show_current()
                self._refresh_controls()
            else:
                self._content_stack.setCurrentIndex(2)
            self.setFocus(Qt.FocusReason.OtherFocusReason)
            return
        self._shown_suggestions = suggestions
        self._suggestions = dict(suggestions)
        self._angle_overrides.clear()
        # Preview the suggested choice, but require confirmation before queueing it.
        self._marked = dict.fromkeys(suggestions, True)
        self._confirmed.clear()
        self._ordered_paths = sorted(suggestions.keys(), key=os.path.basename)
        self._current_index = -1
        self._applying = False
        self._submitted_paths.clear()
        self._successful_paths.clear()
        self._failed_paths.clear()

        if self._ordered_paths:
            self._populate_list()
            self._content_stack.setCurrentIndex(1)
            self._syncing_active_image = True
            try:
                self._navigate_to(0)
            finally:
                self._syncing_active_image = False
            self.setFocus(Qt.FocusReason.OtherFocusReason)
        else:
            self._configure_empty_state(
                "All photos are correctly oriented",
                "No rotation corrections are needed.",
            )
            self._content_stack.setCurrentIndex(2)

    def show_applying(self, current: int, total: int, filename: str) -> None:
        self._applying = True
        self._apply_btn.setEnabled(False)
        self._apply_btn.setText(f"Applying… ({current}/{total})")
        if total > 0:
            self._progress_bar.setRange(0, total)
            self._progress_bar.setValue(current)
        self._status_label.setText(f"Rotating {filename}…")
        self._state_banner.set_state(
            "Changing files now",
            f"Applying rotation {current} of {total}. Please keep PhotoSort open.",
            tone="warning",
        )

    def record_apply_result(self, path: str, success: bool) -> None:
        """Record per-file worker results so completed rows cannot be re-applied."""

        if success:
            self._successful_paths.add(path)
            self._failed_paths.discard(path)
        else:
            self._failed_paths.add(path)

    def show_apply_complete(self, successful: int, failed: int) -> None:
        self._applying = False
        total = successful + failed
        if successful and not self._successful_paths and failed == 0:
            self._successful_paths.update(self._submitted_paths)

        completed = self._successful_paths & set(self._ordered_paths)
        if completed:
            for path in completed:
                self._suggestions.pop(path, None)
                self._angle_overrides.pop(path, None)
                self._marked.pop(path, None)
                self._confirmed.discard(path)
            self._ordered_paths = [
                path for path in self._ordered_paths if path not in completed
            ]

        self._submitted_paths.clear()
        self._successful_paths.clear()

        if not self._ordered_paths:
            self._configure_empty_state(
                "Rotations applied",
                f"{successful} photo{'s' if successful != 1 else ''} updated successfully.",
            )
            self._content_stack.setCurrentIndex(2)
            return

        self._populate_list()
        self._navigate_to(min(self._current_index, len(self._ordered_paths) - 1))
        if failed:
            self._status_label.setText(
                f"{successful} applied · {failed} failed — review and retry"
            )
            self._state_banner.set_state(
                "Some rotations failed",
                "Failed photos remain queued. Review them and try Apply again.",
                tone="danger",
            )
        else:
            self._status_label.setText(f"Applied {successful}/{total} rotations")
        self._failed_paths.clear()
        self._refresh_apply_button()

    def _configure_empty_state(self, title: str, subtitle: str) -> None:
        self._empty_title.setText(title)
        self._empty_subtitle.setText(subtitle)

    # ------------------------------------------------------------------
    # Private helpers
    # ------------------------------------------------------------------

    def _selected_angle(self, path: str) -> int:
        """Return the manual override when present, otherwise the model angle."""

        return self._angle_overrides.get(path, self._suggestions.get(path, 0))

    @staticmethod
    def _clockwise_angle(angle: int) -> int:
        """Advance an image preview clockwise, keeping worker-supported angles."""

        return {0: 90, 90: 180, 180: -90, -90: 0}.get(angle % 360, 0)

    def _item_text(self, path: str) -> str:
        angle = self._selected_angle(path)
        orientation, _ = _ANGLE_LABELS.get(angle, (f"{angle}°", "#888"))
        prefix = "Confirmed  ·  " if path in self._confirmed else ""
        override = "Manual  ·  " if path in self._angle_overrides else ""
        return f"{prefix}{override}{os.path.basename(path)}  ·  {orientation}"

    def _populate_list(self) -> None:
        self._items_list.clear()
        for path in self._ordered_paths:
            item = QListWidgetItem(self._item_text(path))
            item.setData(Qt.ItemDataRole.UserRole, path)
            self._items_list.addItem(item)

        self._review_list_panel.set_count(len(self._ordered_paths))
        self._refresh_list_colors()

    def _refresh_list_colors(self) -> None:
        for i in range(self._items_list.count()):
            item = self._items_list.item(i)
            path = item.data(Qt.ItemDataRole.UserRole)
            is_marked = self._marked.get(path, False)
            confirmed = path in self._confirmed
            color = (
                _MARKED_COLOR
                if confirmed and is_marked
                else _SKIP_COLOR
                if confirmed
                else _UNMARKED_COLOR
            )
            item.setForeground(QColor(color))
            item.setText(self._item_text(path))

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
        if not self._syncing_active_image:
            self.active_image_changed.emit(self._ordered_paths[index])

    def focus_image(self, path: str) -> bool:
        """Navigate to a rotation suggestion without modifying its queued state."""

        try:
            index = self._ordered_paths.index(path)
        except ValueError:
            return False
        self._syncing_active_image = True
        try:
            self._navigate_to(index)
        finally:
            self._syncing_active_image = False
        return True

    def _show_current(self) -> None:
        if self._current_index < 0 or self._current_index >= len(self._ordered_paths):
            return
        path = self._ordered_paths[self._current_index]
        suggested_angle = self._suggestions.get(path, 0)
        angle = self._selected_angle(path)
        is_marked = self._marked.get(path, False)
        confirmed = path in self._confirmed

        badge, color = _ANGLE_LABELS.get(angle, (f"{angle}°", "#888888"))

        pixmap = self._load_pixmap(path)
        if pixmap is None:
            request = getattr(self.window(), "request_interactive_previews", None)
            if callable(request):
                request([path])

        # Left panel: current (as-is) — no rotation applied
        self._current_img.set_pixmap_and_angle(pixmap, 0)
        self._current_hdr.setText(
            "ORIGINAL · SELECTED" if not is_marked else "ORIGINAL · unchanged"
        )

        # Right panel: preview after suggested rotation
        preview_angle = angle if is_marked else 0
        self._preview_img.set_pixmap_and_angle(pixmap, preview_angle)
        if is_marked:
            selected = " · SELECTED" if not confirmed else ""
            self._preview_hdr.setText(f"ROTATED PREVIEW · {badge}{selected}")
        else:
            self._preview_hdr.setText("ROTATED PREVIEW · not selected")

        if not confirmed:
            self._state_banner.set_state(
                "Choose, then confirm",
                "This is only a preview selection. Nothing is queued until you press Confirm.",
                tone="warning",
            )
        elif is_marked:
            self._state_banner.set_state(
                "Decision confirmed",
                f"{os.path.basename(path)} is queued to rotate {badge}. Press Apply to change the file.",
                tone="success",
            )
        else:
            self._state_banner.set_state(
                "Decision confirmed",
                f"{os.path.basename(path)} will remain unchanged.",
                tone="success",
            )

        suggested_badge, _ = _ANGLE_LABELS.get(
            suggested_angle, (f"{suggested_angle}°", "#888888")
        )
        if path in self._angle_overrides:
            self._angle_label.setText(
                f"<b style='color:{color}'>[{badge}]</b>  Manual override: "
                f"<b>{badge}</b> · Suggested: {suggested_badge}"
            )
        else:
            self._angle_label.setText(
                f"<b style='color:{color}'>[{badge}]</b>  Suggested rotation: <b>{badge}</b>"
            )

    def _load_pixmap(self, path: str) -> QPixmap | None:
        try:
            if self._image_pipeline:
                pixmap = self._image_pipeline.get_cached_review_qpixmap(path)
                if pixmap is not None and not pixmap.isNull():
                    return pixmap
        except Exception as exc:
            logger.debug(
                "FixRotation: pixmap load failed for %s: %s",
                os.path.basename(path),
                exc,
            )
        return None

    def handle_preview_ready(self, path: str) -> None:
        if not (0 <= self._current_index < len(self._ordered_paths)):
            return
        if self._ordered_paths[self._current_index] == path:
            self._show_current()

    def _refresh_controls(self) -> None:
        total = len(self._ordered_paths)
        if total == 0:
            return
        self._counter_label.setText(f"{self._current_index + 1} of {total}")
        self._prev_btn.setEnabled(self._current_index > 0)
        self._next_btn.setEnabled(self._current_index < total - 1)

        path = self._ordered_paths[self._current_index]
        self._confirm_btn.setText(
            "Cancel confirmation"
            if path in self._confirmed
            else "Confirm  →"
        )

        self._refresh_apply_button()
        self._refresh_list_colors()

    def _refresh_apply_button(self) -> None:
        marked_count = sum(
            1
            for path in self._ordered_paths
            if path in self._confirmed and self._marked.get(path, False)
        )
        self._apply_btn.setEnabled(marked_count > 0 and not self._applying)
        self._apply_btn.setText(
            f"Apply {marked_count} Rotation{'s' if marked_count != 1 else ''} Now"
            if marked_count > 0
            else "Nothing to Apply"
        )

    # ------------------------------------------------------------------
    # Event handlers
    # ------------------------------------------------------------------

    def _on_item_clicked(self, item: QListWidgetItem) -> None:
        row = self._items_list.row(item)
        if row != self._current_index:
            self._navigate_to(row)

    def _on_prev(self) -> None:
        self._navigate_to(self._current_index - 1)

    def _on_next(self) -> None:
        self._navigate_to(self._current_index + 1)

    def _on_mark_toggle(self) -> None:
        if self._current_index < 0 or not self._ordered_paths:
            return
        path = self._ordered_paths[self._current_index]
        self._set_current_marked(not self._marked.get(path, False))

    def _set_current_marked(self, marked: bool) -> None:
        if self._current_index < 0 or not self._ordered_paths:
            return
        path = self._ordered_paths[self._current_index]
        if marked and self._selected_angle(path) == 0:
            # A zero-degree manual choice is the original image. Selecting the
            # rotated side again restores the model suggestion.
            self._angle_overrides.pop(path, None)
        self._marked[path] = marked
        self._confirmed.discard(path)
        self._show_current()
        QTimer.singleShot(0, self._refresh_controls)

    def _on_rotate_clockwise(self) -> None:
        """Override the model by rotating the selected preview 90° clockwise."""

        if self._current_index < 0 or not self._ordered_paths:
            return
        path = self._ordered_paths[self._current_index]
        current_angle = (
            self._selected_angle(path) if self._marked.get(path, False) else 0
        )
        angle = self._clockwise_angle(current_angle)
        if angle == self._suggestions.get(path, 0):
            self._angle_overrides.pop(path, None)
        else:
            self._angle_overrides[path] = angle
        self._marked[path] = angle != 0
        self._confirmed.discard(path)
        self._show_current()
        QTimer.singleShot(0, self._refresh_controls)

    def _on_confirm_all(self) -> None:
        for path in self._ordered_paths:
            if self._selected_angle(path) == 0:
                self._angle_overrides.pop(path, None)
            self._marked[path] = True
        self._confirmed.update(self._ordered_paths)
        if self._current_index >= 0:
            self._show_current()
        self._refresh_controls()

    def _on_confirm(self) -> None:
        if self._current_index < 0 or not self._ordered_paths:
            return
        path = self._ordered_paths[self._current_index]
        if path in self._confirmed:
            self._confirmed.discard(path)
            self._show_current()
            self._refresh_controls()
            return

        self._confirmed.add(path)
        next_index = next(
            (
                index
                for index in range(self._current_index + 1, len(self._ordered_paths))
                if self._ordered_paths[index] not in self._confirmed
            ),
            None,
        )
        if next_index is None:
            self._show_current()
            self._refresh_controls()
        else:
            self._navigate_to(next_index)

    def _on_apply(self) -> None:
        rotations = self.pending_rotations()
        if rotations:
            self._submitted_paths = set(rotations)
            self._successful_paths.clear()
            self._failed_paths.clear()
            self._applying = True
            self._refresh_apply_button()
            self._state_banner.set_state(
                "Starting file changes",
                f"Preparing to rotate {len(rotations)} photo{'s' if len(rotations) != 1 else ''}.",
                tone="warning",
            )
            self.apply_rotations_requested.emit(rotations)

    def _on_proceed(self) -> None:
        self.proceed_requested.emit()

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

        # Container for the missing model view
        self._missing_model_widget = QWidget()
        self._missing_model_widget.setObjectName("missingModelWidget")
        missing_layout = QVBoxLayout(self._missing_model_widget)
        missing_layout.setContentsMargins(0, 0, 0, 0)
        missing_layout.setSpacing(12)
        missing_layout.setAlignment(Qt.AlignmentFlag.AlignCenter)

        self._error_summary_label = QLabel("The automatic rotation feature requires a model file that was not found:")
        self._error_summary_label.setObjectName("errorSummaryLabel")
        self._error_summary_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self._error_summary_label.setWordWrap(True)

        self._model_path_label = QLabel()
        self._model_path_label.setObjectName("modelPathLabel")
        self._model_path_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self._model_path_label.setWordWrap(True)
        self._model_path_label.setFixedWidth(520)

        self._instructions_label = QLabel()
        self._instructions_label.setObjectName("instructionsLabel")
        self._instructions_label.setAlignment(Qt.AlignmentFlag.AlignLeft)
        self._instructions_label.setWordWrap(True)
        self._instructions_label.setFixedWidth(520)

        # Download buttons container widget
        self._download_btn_container = QWidget()
        btn_layout = QHBoxLayout(self._download_btn_container)
        btn_layout.setContentsMargins(0, 12, 0, 0)
        btn_layout.setSpacing(16)

        self._download_model_btn = QPushButton("Download Model")
        self._download_model_btn.setObjectName("downloadModelBtn")
        self._download_model_btn.clicked.connect(
            lambda: QDesktopServices.openUrl(QUrl(ROTATION_MODEL_DOWNLOAD_URL))
        )

        self._open_models_btn = QPushButton("Open Models Folder")
        self._open_models_btn.setObjectName("openModelsBtn")
        self._open_models_btn.clicked.connect(self._open_models_folder_clicked)

        self._retry_btn = QPushButton("Check Again")
        self._retry_btn.setObjectName("retryBtn")
        self._retry_btn.clicked.connect(self.retry_requested.emit)

        btn_layout.addWidget(self._download_model_btn)
        btn_layout.addWidget(self._open_models_btn)
        btn_layout.addWidget(self._retry_btn)

        missing_layout.addWidget(self._error_summary_label)
        missing_layout.addWidget(self._model_path_label, alignment=Qt.AlignmentFlag.AlignCenter)
        missing_layout.addWidget(self._instructions_label, alignment=Qt.AlignmentFlag.AlignCenter)
        missing_layout.addWidget(self._download_btn_container, alignment=Qt.AlignmentFlag.AlignCenter)

        self._missing_model_widget.setVisible(False)

        self._progress_bar = QProgressBar()
        self._progress_bar.setRange(0, 100)
        self._progress_bar.setFixedWidth(320)
        self._progress_bar.setTextVisible(True)

        layout.addWidget(title)
        layout.addWidget(self._loading_label)
        layout.addWidget(self._missing_model_widget)
        layout.addWidget(self._progress_bar, alignment=Qt.AlignmentFlag.AlignCenter)
        return page

    def _open_models_folder_clicked(self) -> None:
        try:
            if not is_frozen_runtime():
                target = os.path.abspath("./models")
                os.makedirs(target, exist_ok=True)
            else:
                target = get_app_models_dir()
            url = QUrl.fromLocalFile(os.path.abspath(target))
            QDesktopServices.openUrl(url)
        except Exception as e:
            logger.error(f"Failed to open models folder: {e}", exc_info=True)

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

        # Main splitter: list | dual-preview
        splitter = QSplitter(Qt.Orientation.Horizontal)
        splitter.setHandleWidth(4)
        splitter.setChildrenCollapsible(False)

        self._review_list_panel = WorkflowReviewListPanel(
            bulk_action_text="Confirm all"
        )
        self._items_list = self._review_list_panel.list_widget
        self._items_list.itemClicked.connect(self._on_item_clicked)
        self._confirm_all_btn = self._review_list_panel.bulk_button
        self._confirm_all_btn.setToolTip(
            "Confirm every suggested rotation. You can still review or revise each choice."
        )
        self._confirm_all_btn.clicked.connect(self._on_confirm_all)
        splitter.addWidget(self._review_list_panel)

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
        self._current_img.setCursor(Qt.CursorShape.PointingHandCursor)
        self._current_img.clicked.connect(lambda: self._set_current_marked(False))
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
        self._preview_img.setCursor(Qt.CursorShape.PointingHandCursor)
        self._preview_img.clicked.connect(lambda: self._set_current_marked(True))
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

        self._state_banner = WorkflowStateBanner()
        right_layout.addWidget(self._state_banner)

        # Action bar
        action = QHBoxLayout()
        self._action_layout = action
        action.setSpacing(6)

        self._prev_btn = QPushButton("← Prev")
        self._prev_btn.setObjectName("workflowGhostButton")
        self._prev_btn.setFixedWidth(70)
        self._prev_btn.setToolTip("Previous  [←]")
        self._prev_btn.clicked.connect(self._on_prev)

        self._counter_label = QLabel("0 of 0")
        self._counter_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self._counter_label.setStyleSheet("font-size: 12px; color: #808080;")
        self._counter_label.setFixedWidth(72)

        self._next_btn = QPushButton("Next →")
        self._next_btn.setObjectName("workflowGhostButton")
        self._next_btn.setFixedWidth(70)
        self._next_btn.setToolTip("Next  [→]")
        self._next_btn.clicked.connect(self._on_next)

        self._confirm_btn = QPushButton("Confirm  →")
        self._confirm_btn.setObjectName("workflowPrimaryButton")
        self._confirm_btn.setMinimumWidth(110)
        self._confirm_btn.clicked.connect(self._on_confirm)

        self._rotate_clockwise_btn = QPushButton("Rotate +90°  [Shift+R]")
        self._rotate_clockwise_btn.setObjectName("workflowGhostButton")
        self._rotate_clockwise_btn.setToolTip(
            "Override the suggestion and rotate the preview 90° clockwise"
        )
        self._rotate_clockwise_btn.clicked.connect(self._on_rotate_clockwise)

        self._apply_btn = QPushButton("Apply Marked Rotations")
        self._apply_btn.setObjectName("workflowPrimaryButton")
        self._apply_btn.setEnabled(False)
        self._apply_btn.clicked.connect(self._on_apply)

        action.addWidget(self._prev_btn)
        action.addWidget(self._counter_label)
        action.addWidget(self._next_btn)
        action.addWidget(self._confirm_btn)
        action.addWidget(self._rotate_clockwise_btn)
        action.addStretch(1)
        action.addWidget(self._apply_btn)
        right_layout.addLayout(action)

        splitter.addWidget(right_pane)
        content_layout.addWidget(splitter, 1)
        page_layout.addWidget(content, 1)
        return page

    def _build_empty_page(self) -> QWidget:
        page = QWidget()
        layout = QVBoxLayout(page)
        layout.setAlignment(Qt.AlignmentFlag.AlignCenter)
        layout.setSpacing(16)

        self._empty_title = QLabel("All photos are correctly oriented")
        self._empty_title.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self._empty_title.setStyleSheet(
            "font-size: 18px; font-weight: bold; color: #66BB6A;"
        )

        self._empty_subtitle = QLabel("No rotation corrections are needed.")
        self._empty_subtitle.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self._empty_subtitle.setStyleSheet("font-size: 13px; color: #aaaaaa;")

        proceed_btn = QPushButton("Continue to Pick Best →")
        proceed_btn.setObjectName("acceptButton")
        proceed_btn.setFixedWidth(220)
        proceed_btn.clicked.connect(self._on_proceed)

        layout.addWidget(self._empty_title)
        layout.addWidget(self._empty_subtitle)
        layout.addWidget(proceed_btn, alignment=Qt.AlignmentFlag.AlignCenter)
        return page

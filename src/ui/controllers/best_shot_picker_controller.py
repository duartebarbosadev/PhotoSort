"""
Best Shot Picker Controller
Manages the AI-powered best shot selection feature.
"""

import logging
from pathlib import Path
from typing import Optional, TYPE_CHECKING

from PyQt6.QtCore import Qt, pyqtSignal, QItemSelectionModel
from PyQt6.QtWidgets import (
    QMessageBox,
    QDialog,
    QLabel,
    QPushButton,
    QVBoxLayout,
    QHBoxLayout,
)

from core.ai.best_shot_picker import BestShotResult

if TYPE_CHECKING:
    from ui.main_window import MainWindow

logger = logging.getLogger(__name__)


class _BestShotProgressDialog(QDialog):
    """Frameless, styled dialog for long-running AI analysis."""

    cancelled = pyqtSignal()

    def __init__(self, parent=None):
        super().__init__(parent)
        self.setModal(True)
        self.setWindowTitle("AI Best Shot Picker")
        self.setObjectName("aiBestShotProgressDialog")
        self.setAttribute(Qt.WidgetAttribute.WA_DeleteOnClose)
        self.setWindowFlags(self.windowFlags() | Qt.WindowType.FramelessWindowHint)
        self.setMinimumWidth(360)

        layout = QVBoxLayout(self)
        layout.setSpacing(16)
        layout.setContentsMargins(24, 24, 24, 24)

        title_label = QLabel("Analyzing Selection")
        title_label.setObjectName("aiBestShotProgressTitle")
        title_label.setStyleSheet("font-weight: bold; font-size: 14px;")
        layout.addWidget(title_label)

        self.status_label = QLabel("Preparing analysis...")
        self.status_label.setObjectName("aiBestShotProgressStatus")
        self.status_label.setWordWrap(True)
        self.status_label.setStyleSheet("color: #767676; font-size: 11px;")
        layout.addWidget(self.status_label)

        layout.addStretch()

        button_row = QHBoxLayout()
        button_row.addStretch()
        self.cancel_button = QPushButton("Cancel Analysis")
        self.cancel_button.setObjectName("aiBestShotProgressCancelButton")
        self.cancel_button.clicked.connect(self.cancelled.emit)
        button_row.addWidget(self.cancel_button)
        layout.addLayout(button_row)

    def update_status(self, message: str):
        if message:
            self.status_label.setText(message)

    def set_cancel_enabled(self, enabled: bool):
        self.cancel_button.setEnabled(enabled)


class BestShotPickerController:
    """
    Controller for AI-powered best shot selection.

    Manages the workflow of:
    1. Getting selected images from the UI
    2. Running the AI analysis in a background thread
    3. Presenting the results to the user
    4. Optionally selecting the best image in the UI
    """

    def __init__(self, main_window: "MainWindow"):
        """Initialize the controller."""
        self.main_window = main_window
        self.worker_manager = self.main_window.worker_manager
        self.current_image_paths: list[str] = []
        self.progress_dialog: Optional[_BestShotProgressDialog] = None

        self.worker_manager.best_shot_progress.connect(self._on_progress)
        self.worker_manager.best_shot_result_ready.connect(self._on_result_ready)
        self.worker_manager.best_shot_error.connect(self._on_error)
        self.worker_manager.best_shot_finished.connect(self._on_finished)

    def can_pick_best_shot(self) -> bool:
        """Return True when 2+ images are selected and no analysis is running."""
        if self.worker_manager.is_best_shot_running():
            return False

        selected_paths = self.main_window.selection_controller.get_selected_file_paths()
        return len(selected_paths) >= 2

    def start_analysis(self):
        """Start the best shot analysis for the current selection."""
        if self.worker_manager.is_best_shot_running():
            QMessageBox.information(
                self.main_window,
                "Analysis In Progress",
                "AI best shot analysis is already running. Please wait for it to finish.",
            )
            return

        selected_paths = self.main_window.selection_controller.get_selected_file_paths()

        if len(selected_paths) < 2:
            QMessageBox.warning(
                self.main_window,
                "Not Enough Images",
                "Please select at least 2 images to pick the best shot.",
            )
            return

        logger.info(f"Starting best shot analysis for {len(selected_paths)} images")
        logger.info("Images retrieved from selection (in order):")
        for idx, path in enumerate(selected_paths, 1):
            logger.info("  Selection position %d: %s", idx, Path(path).name)

        self.current_image_paths = list(selected_paths)

        self._show_progress_dialog()
        self._set_action_enabled(False)

        try:
            self.worker_manager.start_best_shot_analysis(self.current_image_paths)
        except ValueError as exc:
            logger.error("Failed to start best shot analysis: %s", exc)
            self._close_progress_dialog()
            self._set_action_enabled(True)
            QMessageBox.critical(self.main_window, "Analysis Failed", str(exc))

    def _show_progress_dialog(self):
        self._close_progress_dialog()
        self.progress_dialog = _BestShotProgressDialog(self.main_window)
        self.progress_dialog.cancelled.connect(self._on_cancel)
        self.progress_dialog.update_status("Testing API connection...")
        self.progress_dialog.show()

    def _on_progress(self, message: str):
        """Display progress updates from the worker in the dialog."""
        if self.progress_dialog and message:
            self.progress_dialog.update_status(message)

    def _on_result_ready(self, result: BestShotResult):
        """Handle analysis result and show it to the user."""
        logger.info("Best shot selected: %s", result.best_image_path)
        self.main_window.update_best_shot_labels(
            [result.best_image_path], replace=True
        )
        self._close_progress_dialog()
        self._show_result_dialog(result)

    def _on_error(self, error_message: str):
        """Handle an error emitted by the worker."""
        logger.error("Best shot analysis error: %s", error_message)
        self._close_progress_dialog()
        QMessageBox.critical(
            self.main_window,
            "Analysis Failed",
            f"Failed to analyze images:\n\n{error_message}",
        )
        self._set_action_enabled(True)

    def _on_finished(self, success: bool):
        """Handle worker completion regardless of result."""
        logger.info("Best shot analysis finished (success: %s)", success)
        self._set_action_enabled(True)
        if not success:
            self.main_window.statusBar().showMessage(
                "AI best shot analysis cancelled.", 5000
            )
        self._close_progress_dialog()
        self.current_image_paths = []

    def _on_cancel(self):
        """Handle user cancelling the dialog."""
        self.worker_manager.stop_best_shot_analysis()
        self.main_window.statusBar().showMessage(
            "Cancelling AI best shot analysis...", 3000
        )
        if self.progress_dialog:
            self.progress_dialog.set_cancel_enabled(False)
            self.progress_dialog.update_status("Cancelling analysis...")

    def cleanup(self):
        """Cleanup resources when shutting down."""
        self.worker_manager.stop_best_shot_analysis()
        self._close_progress_dialog()
        self.current_image_paths = []
        self._set_action_enabled(True)

    def _close_progress_dialog(self):
        if self.progress_dialog:
            try:
                self.progress_dialog.close()
            finally:
                self.progress_dialog.deleteLater()
                self.progress_dialog = None

    def _set_action_enabled(self, enabled: bool):
        try:
            action = self.main_window.menu_manager.pick_best_shot_action
            action.setEnabled(enabled)
        except Exception:  # pragma: no cover - defensive
            logger.debug("Failed to toggle best shot action state", exc_info=True)

    def _show_result_dialog(self, result: BestShotResult):
        """Show a dialog with the analysis results."""
        image_name = Path(result.best_image_path).name
        image_num = result.best_image_index + 1
        total_images = len(self.current_image_paths)

        message = f"""<h3>Best Image Selected</h3>
<p><b>Image:</b> {image_name}<br>
<b>Position:</b> {image_num} of {total_images}<br>
<b>Confidence:</b> {result.confidence}</p>

<p><b>Reasoning:</b><br>
{result.reasoning}</p>

<p>Would you like to select this image in the viewer?</p>"""

        reply = QMessageBox.question(
            self.main_window,
            "Best Shot Selected",
            message,
            QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.No,
            QMessageBox.StandardButton.Yes,
        )

        if reply == QMessageBox.StandardButton.Yes:
            self._select_image_in_ui(result.best_image_path)

    def _select_image_in_ui(self, image_path: str):
        """Select the specified image in the UI."""
        try:
            logger.info("Selecting image in UI: %s", image_path)

            active_view = self.main_window._get_active_file_view()
            if not active_view:
                logger.warning("No active view available")
                return

            proxy_index = self.main_window._find_proxy_index_for_path(image_path)
            if not proxy_index or not proxy_index.isValid():
                logger.warning("Could not find index for path: %s", image_path)
                QMessageBox.information(
                    self.main_window,
                    "Navigation Failed",
                    "Could not find the selected image in the current view.\n\n"
                    "The image may be filtered out or in a different folder.",
                )
                return

            selection_model = active_view.selectionModel()
            if selection_model:
                selection_model.setCurrentIndex(
                    proxy_index,
                    QItemSelectionModel.SelectionFlag.ClearAndSelect,
                )
                active_view.scrollTo(proxy_index)
                logger.info("Successfully selected best image in UI")
            else:
                logger.warning("No selection model available")

        except Exception as exc:  # pragma: no cover - UI safety
            logger.error("Failed to select image in UI: %s", exc)
            QMessageBox.warning(
                self.main_window,
                "Selection Failed",
                f"Could not select the image in the UI:\n{exc}",
            )


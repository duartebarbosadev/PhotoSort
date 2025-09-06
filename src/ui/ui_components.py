from PyQt6.QtWidgets import (
    QWidget,
    QVBoxLayout,
    QLabel,
    QTreeView,
    QStyledItemDelegate,
    QStyleOptionViewItem,
    QListView,
)
from PyQt6.QtCore import Qt, QObject, pyqtSignal, QModelIndex
from PyQt6.QtGui import (
    QPainter,
    QPalette,
    QDragEnterEvent,
    QDragMoveEvent,
    QDropEvent,
    QStandardItem,
)
from typing import List, Optional
import os

from core.image_pipeline import ImagePipeline
from core.image_features.blur_detector import (
    BlurDetector,
)
from core.caching.exif_cache import ExifCache
from core.image_processing.raw_image_processor import is_raw_extension
import logging

logger = logging.getLogger(__name__)


# --- Custom Tree View for Drag and Drop ---
class DroppableTreeView(QTreeView):
    def __init__(self, model, main_window, parent=None):
        super().__init__(parent)
        self.setModel(model)
        self.main_window = main_window  # To access AppState
        self.viewport().setAcceptDrops(False)  # Disable drag and drop
        # self.setDefaultDropAction(Qt.DropAction.MoveAction) # Disable drag and drop
        self.highlighted_drop_target_index = None
        self.original_item_brush = None

    def dragEnterEvent(self, event: Optional[QDragEnterEvent]):
        if event:
            event.ignore()  # Disable drag and drop

    def _clear_drop_highlight(self):
        if (
            self.highlighted_drop_target_index
            and self.highlighted_drop_target_index.isValid()
        ):
            item = self.model().itemFromIndex(self.highlighted_drop_target_index)
            if item:
                item.setBackground(
                    self.original_item_brush
                    if self.original_item_brush
                    else QStandardItem().background()
                )
        self.highlighted_drop_target_index = None
        self.original_item_brush = None

    def dragMoveEvent(self, event: Optional[QDragMoveEvent]):
        if event:
            event.ignore()  # Disable drag and drop

    def dragLeaveEvent(self, event):
        self._clear_drop_highlight()
        super().dragLeaveEvent(event)

    def dropEvent(self, event: Optional[QDropEvent]):
        if event:
            event.ignore()  # Disable drag and drop

    # Use default selection semantics (allow Ctrl+click multi-select on Windows again)
    def selectionCommand(self, index, event=None):  # type: ignore[override]
        return super().selectionCommand(index, event)


class NoCtrlListView(QListView):
    """QListView with default Ctrl+click multi-select behavior restored."""

    def selectionCommand(self, index, event=None):  # type: ignore[override]
        return super().selectionCommand(index, event)


# --- Custom Delegate for Highlighting Focused Image ---
class FocusHighlightDelegate(QStyledItemDelegate):
    def __init__(self, app_state, main_window, parent=None):
        super().__init__(parent)
        self.app_state = app_state
        self.main_window = main_window

    def paint(
        self,
        painter: Optional[QPainter],
        option: Optional[QStyleOptionViewItem],
        index: Optional[QModelIndex],
    ):
        # Let the base class handle the default painting (selection, text, icon)
        super().paint(painter, option, index)

        if not self.app_state.focused_image_path:
            return

        active_view = self.main_window._get_active_file_view()
        if not active_view:
            return

        # Only draw the underline if more than one item is selected (i.e., we are in a "split" context)
        num_selected = len(active_view.selectionModel().selectedIndexes())
        if num_selected <= 1:
            return

        # Check if the current item is the one that is focused in the viewer
        item_data = index.data(Qt.ItemDataRole.UserRole)
        if (
            isinstance(item_data, dict)
            and item_data.get("path") == self.app_state.focused_image_path
        ):
            painter.save()

            # Use the theme's highlight color for a more integrated look.
            pen_color = option.palette.color(QPalette.ColorRole.Highlight)
            pen = painter.pen()
            pen.setColor(pen_color)
            pen.setWidth(3)  # A bit thicker for better visibility
            pen.setCapStyle(Qt.PenCapStyle.RoundCap)  # Softer edges
            painter.setPen(pen)

            # Position the underline at the bottom of the item's rectangle
            rect = option.rect
            # Position 2px from the bottom, and inset the line horizontally
            y = rect.bottom() - 2
            painter.drawLine(rect.left() + 5, y, rect.right() - 5, y)

            painter.restore()


# --- Loading Overlay ---
class LoadingOverlay(QWidget):
    def __init__(self, parent=None):
        super().__init__(parent)
        self.setAttribute(Qt.WidgetAttribute.WA_TranslucentBackground)
        self.setAttribute(Qt.WidgetAttribute.WA_DeleteOnClose)

        main_layout = QVBoxLayout(self)
        main_layout.setContentsMargins(0, 0, 0, 0)
        main_layout.setSpacing(0)

        self.bg_widget = QWidget(self)
        # Stylesheet will be applied from dark_theme.qss based on object name or class
        # self.bg_widget.setStyleSheet("background-color: rgba(0, 0, 0, 0.7);")
        main_layout.addWidget(self.bg_widget)

        content_layout = QVBoxLayout(self.bg_widget)
        content_layout.setAlignment(Qt.AlignmentFlag.AlignCenter)
        content_layout.setContentsMargins(20, 20, 20, 20)

        self.text_label = QLabel("Loading...", self)
        self.text_label.setObjectName("loading_text_label")  # For styling
        self.text_label.setAlignment(Qt.AlignmentFlag.AlignCenter)

        content_layout.addWidget(self.text_label)
        self.hide()

    def setText(self, text):
        self.text_label.setText(text)
        self.text_label.adjustSize()

    def showEvent(self, event):
        if self.parentWidget():
            self.setGeometry(self.parentWidget().rect())
        super().showEvent(event)

    def hideEvent(self, event):
        super().hideEvent(event)

    def update_position(self):
        if self.parentWidget() and self.isVisible():
            self.setGeometry(self.parentWidget().rect())
            self.raise_()


# --- Preview Preloader Worker ---
class PreviewPreloaderWorker(QObject):
    progress_update = pyqtSignal(int, str)
    finished = pyqtSignal()
    error = pyqtSignal(str)

    def __init__(
        self,
        image_paths,
        max_size,
        image_pipeline_instance: ImagePipeline,
        parent=None,
    ):
        super().__init__(parent)
        self._image_paths = image_paths
        self._max_size = max_size
        self.image_pipeline = image_pipeline_instance
        self._is_running = True

    def stop(self):
        self._is_running = False

    def _should_continue(self):
        return self._is_running

    def _report_progress(self, count, total):
        if total > 0:
            percentage = int((count / total) * 100)
            # Report progress more frequently or at key milestones
            if percentage % 5 == 0 or count == total or count == 1:
                self.progress_update.emit(
                    percentage, f"Preloading previews ({count}/{total})..."
                )

    def run_preload(self):
        self._is_running = True
        try:
            self.image_pipeline.preload_previews(
                self._image_paths,
                progress_callback=self._report_progress,
                should_continue_callback=self._should_continue,
            )
        except Exception as e:
            err_msg = f"Error during preview preloading thread: {e}"
            logger.error(err_msg, exc_info=True)
            self.error.emit(err_msg)
        finally:
            if self._is_running:
                self.progress_update.emit(100, "Preview preloading complete.")
            else:
                self.progress_update.emit(100, "Preview preloading cancelled.")
            self.finished.emit()


# --- Blur Detection Worker ---
class BlurDetectionWorker(QObject):
    progress_update = pyqtSignal(int, int, str)  # current, total, basename
    blur_status_updated = pyqtSignal(str, bool)  # image_path, is_blurred
    finished = pyqtSignal()
    error = pyqtSignal(str)

    def __init__(
        self,
        image_paths: List[str],
        blur_threshold: float,
        apply_auto_edits_for_raw: bool,
        parent=None,
    ):
        super().__init__(parent)
        self._image_paths = image_paths  # Changed from image_data_list
        self._blur_threshold = blur_threshold
        self._apply_auto_edits = apply_auto_edits_for_raw
        self._is_running = True

    def stop(self):
        self._is_running = False

    def _should_continue(self) -> bool:
        return self._is_running

    def run_detection(self):
        self._is_running = True
        try:
            BlurDetector.detect_blur_in_batch(
                image_paths=self._image_paths,
                threshold=self._blur_threshold,
                apply_auto_edits_for_raw_preview=self._apply_auto_edits,
                status_update_callback=self.blur_status_updated.emit,  # Pass signal emitter directly
                progress_callback=self.progress_update.emit,  # Pass signal emitter directly
                should_continue_callback=self._should_continue,
            )
        except Exception as e:
            err_msg = f"Error during batch blur detection: {e}"
            logger.error(err_msg, exc_info=True)
            self.error.emit(err_msg)
        finally:
            if (
                not self._is_running and not self.signalsBlocked()
            ):  # If stopped, error might have been emitted by batch
                pass  # Avoid double emitting error if already cancelled and handled by batch
            elif (
                self.signalsBlocked()
            ):  # If signals were blocked (e.g. due to deletion)
                pass
            else:  # Normal finish
                self.finished.emit()


# --- Rotation Detection Worker ---
class RotationDetectionWorker(QObject):
    """Worker thread for detecting rotation suggestions in images."""

    progress_update = pyqtSignal(int, int, str)  # current, total, basename
    rotation_detected = pyqtSignal(str, int)  # image_path, suggested_rotation
    model_not_found = pyqtSignal(str)  # model_path
    finished = pyqtSignal()
    error = pyqtSignal(str)

    def __init__(
        self,
        image_paths: List[str],
        image_pipeline: ImagePipeline,
        exif_cache: "ExifCache",
        parent=None,
    ):
        super().__init__(parent)
        self.image_paths = image_paths
        self.image_pipeline = image_pipeline
        self.exif_cache = exif_cache
        self._should_stop = False

    def stop(self):
        """Request the worker to stop."""
        self._should_stop = True

    def run(self):
        """Run the rotation detection process."""
        try:
            from core.image_features.rotation_detector import RotationDetector
            from core.image_features.model_rotation_detector import (
                ModelNotFoundError,
            )

            def result_callback(image_path: str, suggested_rotation: int):
                if not self._should_stop:
                    self.rotation_detected.emit(image_path, suggested_rotation)

            def progress_callback(current: int, total: int, basename: str):
                if not self._should_stop:
                    self.progress_update.emit(current, total, basename)

            def should_continue_callback() -> bool:
                return not self._should_stop

            # Pass the image pipeline instance to the detector
            detector = RotationDetector(self.image_pipeline, self.exif_cache)
            detector.detect_rotation_in_batch(
                image_paths=self.image_paths,
                result_callback=result_callback,
                progress_callback=progress_callback,
                should_continue_callback=should_continue_callback,
            )

            if not self._should_stop:
                self.finished.emit()

        except ModelNotFoundError as e:
            logger.error(f"Rotation model not found during worker execution: {e}")
            if not self._should_stop:
                self.model_not_found.emit(str(e))  # Emit the model path
        except Exception as e:
            logger.error(f"Error in rotation detection worker: {e}")
            if not self._should_stop:
                self.error.emit(str(e))


# --- Similarity Engine Worker ---
class SimilarityWorker(QObject):
    """Worker for running similarity analysis in the background."""

    progress_update = pyqtSignal(int, str)
    embeddings_generated = pyqtSignal(object)  # Using object to pass dict
    clustering_complete = pyqtSignal(object)  # Using object to pass dict
    error = pyqtSignal(str)
    finished = pyqtSignal()

    def __init__(self, file_paths: List[str], parent=None):
        super().__init__(parent)
        self.file_paths = file_paths
        self._is_running = True
        self.similarity_engine = None

    def _has_raw_images(self) -> bool:
        """Check if any of the file paths are RAW image files."""
        for path in self.file_paths:
            if not path:
                continue
            ext = os.path.splitext(path)[1].lower()
            if is_raw_extension(ext):
                return True
        return False

    def stop(self):
        self._is_running = False
        if self.similarity_engine:
            self.similarity_engine.stop()

    def run(self):
        """The main method that will be executed in the new thread."""
        self._is_running = True
        try:
            from core.similarity_engine import SimilarityEngine

            # 1. Instantiate the engine inside the worker thread
            self.similarity_engine = SimilarityEngine()

            # 2. Connect its signals to this worker's signals
            self.similarity_engine.progress_update.connect(self.progress_update)
            self.similarity_engine.embeddings_generated.connect(
                self.embeddings_generated
            )
            self.similarity_engine.clustering_complete.connect(self.clustering_complete)
            self.similarity_engine.error.connect(self.error)

            # 3. Connect the final signal to this worker's finished signal
            self.similarity_engine.clustering_complete.connect(self.finished)
            self.similarity_engine.error.connect(self.finished)

            # 4. Start the process
            self.similarity_engine.generate_embeddings_for_files(self.file_paths)

        except Exception as e:
            logger.error(
                f"Error initializing or running SimilarityEngine: {e}", exc_info=True
            )
            self.error.emit(str(e))
            self.finished.emit()


# --- CUDA Detection Worker ---
class CudaDetectionWorker(QObject):
    finished = pyqtSignal(bool)  # cuda_available

    def run(self):
        from core.app_settings import is_pytorch_cuda_available

        try:
            available = is_pytorch_cuda_available()
            self.finished.emit(available)
        except Exception as e:
            logger.error(f"Error during CUDA detection: {e}", exc_info=True)
            self.finished.emit(False)  # default to CPU on error

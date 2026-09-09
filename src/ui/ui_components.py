from PyQt6.QtWidgets import (
    QWidget,
    QVBoxLayout,
    QTreeView,
    QStyledItemDelegate,
    QStyleOptionViewItem,
    QListView,
)
from PyQt6.QtCore import Qt, QObject, pyqtSignal, QModelIndex, QPoint, QRect, QSize
from PyQt6.QtGui import (
    QPainter,
    QPalette,
    QDragEnterEvent,
    QDragMoveEvent,
    QDropEvent,
    QStandardItem,
)
import os
import re
from typing import override

from core.image_pipeline import ImagePipeline
from core.image_processing.raw_image_processor import is_raw_extension
from core.media_utils import is_image_extension
from core.similarity_cache import parse_cluster_id
from ui.workflow_review_components import WorkflowProgressView
import logging

logger = logging.getLogger(__name__)


# --- Custom Tree View for Drag and Drop ---
class DroppableTreeView(QTreeView):
    """
    Custom QTreeView that prevents single-letter shortcuts from being consumed by type-ahead search.
    Also supports drag-and-drop between clusters in similarity mode.

    Architecture for keyboard shortcuts in PhotoSort:

    1. Single-letter QAction shortcuts (D, R, A, I, S, F):
       - Defined in MenuManager with ApplicationShortcut context
       - This class ignores them in keyPressEvent() to prevent type-ahead search
       - Qt's QAction system then processes them normally

    2. Navigation keys (arrows):
       - Handled by MainWindow.eventFilter() -> HotkeyController

    3. Modified shortcuts (Ctrl+S, Alt+1, Shift+R, etc.):
       - Work automatically via QActions, no special handling needed

    This architecture is:
    - Simple: One place for each type of shortcut
    - Future-proof: Adding new single-letter shortcuts only requires updating shortcut_keys set
    - Maintainable: Clear separation between navigation, single-key, and modified shortcuts
    """

    def __init__(self, model, main_window, parent=None):
        super().__init__(parent)
        self.setModel(model)
        self.main_window = main_window  # To access AppState
        self.viewport().setAcceptDrops(True)  # Enable drops
        self.setDragEnabled(True)  # Enable dragging
        self.setDropIndicatorShown(True)
        self.highlighted_drop_target_index = None
        self.original_item_brush = None

    @override
    def keyPressEvent(self, event):
        """
        Override to prevent type-ahead search from consuming single-letter shortcuts.

        When adding a new single-letter shortcut:
        1. Add it to MenuManager with setShortcutContext(Qt.ShortcutContext.ApplicationShortcut)
        2. Add the Qt.Key constant to the shortcut_keys set below
        """
        key = event.key()
        modifiers = event.modifiers()

        # Single-key shortcuts that should NOT be consumed by type-ahead search
        # Note: Only truly unmodified single keys need to be listed here
        # Modified shortcuts (Ctrl+S, Alt+1, etc.) are already handled correctly by Qt
        shortcut_keys = {
            Qt.Key.Key_A,  # Actual size zoom
            Qt.Key.Key_D,  # Mark for deletion
            Qt.Key.Key_F,  # Toggle folder view
            Qt.Key.Key_I,  # Toggle metadata sidebar
            Qt.Key.Key_R,  # Rotate clockwise
            Qt.Key.Key_S,  # Group by similarity
        }

        # If it's an unmodified single-letter shortcut, ignore it
        # This prevents QTreeView's type-ahead search from consuming it
        if modifiers == Qt.KeyboardModifier.NoModifier and key in shortcut_keys:
            event.ignore()
            return

        # For all other keys, use default QTreeView behavior
        super().keyPressEvent(event)

    def _is_cluster_drop_valid(self, event) -> bool:
        """Check if drop target is a cluster header and we're in similarity mode."""
        if not self.main_window.group_by_similarity_mode:
            return False

        app_state = self.main_window.app_state
        cluster_results = (
            app_state.cluster_results_for_workflow()
            if hasattr(app_state, "cluster_results_for_workflow")
            else app_state.cluster_results
        )
        if not cluster_results:
            return False

        pos = event.position().toPoint()
        proxy_index = self.indexAt(pos)
        if not proxy_index.isValid():
            return False

        source_index = self.main_window.proxy_model.mapToSource(proxy_index)
        item = self.main_window.file_system_model.itemFromIndex(source_index)
        if not item:
            return False

        item_data = item.data(Qt.ItemDataRole.UserRole)
        if not (isinstance(item_data, str) and item_data.startswith("cluster_header_")):
            return False

        return self._get_cluster_id_from_index(proxy_index) is not None

    def _get_cluster_id_from_index(self, proxy_index) -> int | None:
        """Extract cluster ID from a cluster header item."""
        if not proxy_index.isValid():
            return None

        source_index = self.main_window.proxy_model.mapToSource(proxy_index)
        item = self.main_window.file_system_model.itemFromIndex(source_index)
        if not item:
            return None

        item_data = item.data(Qt.ItemDataRole.UserRole)
        if isinstance(item_data, str) and item_data.startswith("cluster_header_"):
            try:
                return int(item_data.replace("cluster_header_", ""))
            except ValueError:
                return None
        return None

    def _move_dragged_items_to_cluster(self, target_cluster_id: int):
        """Move currently selected items to the target cluster."""
        selected_paths = [
            path
            for path in self.main_window._get_selected_file_paths_from_view()
            if is_image_extension(path)
        ]
        if not selected_paths:
            return

        app_state = self.main_window.app_state
        folder_path = app_state.current_folder_path
        cluster_results = app_state.cluster_results_for_workflow()

        overrides_to_save = {}
        for path in selected_paths:
            cluster_results[path] = target_cluster_id
            overrides_to_save[path] = target_cluster_id

        # Persist to cache
        if folder_path:
            app_state.analysis_cache.save_manual_cluster_overrides(
                folder_path,
                overrides_to_save,
                namespace=app_state.manual_override_namespace_for_workflow(),
            )

        # Update UI - extract cluster IDs for display
        cluster_ids = set()
        for value in cluster_results.values():
            parsed_id = parse_cluster_id(value)
            if parsed_id is not None:
                cluster_ids.add(parsed_id)
        sorted_cluster_ids = sorted(cluster_ids)

        self.main_window.cluster_filter_combo.clear()
        self.main_window.cluster_filter_combo.addItems(
            ["All Clusters"] + [f"Cluster {cid}" for cid in sorted_cluster_ids]
        )
        self.main_window.menu_manager.update_cluster_filter_menu(sorted_cluster_ids)
        self.main_window._rebuild_model_view()

        logger.info(
            "Moved %d image(s) to cluster %d via drag-drop",
            len(selected_paths),
            target_cluster_id,
        )

    @override
    def dragEnterEvent(self, event: QDragEnterEvent | None):
        if event and self._is_cluster_drop_valid(event):
            event.acceptProposedAction()
        elif event:
            event.ignore()

    def _clear_drop_highlight(self):
        if (
            self.highlighted_drop_target_index
            and self.highlighted_drop_target_index.isValid()
        ):
            item = self.main_window.file_system_model.itemFromIndex(
                self.highlighted_drop_target_index
            )
            if item:
                item.setBackground(
                    self.original_item_brush
                    if self.original_item_brush
                    else QStandardItem().background()
                )
        self.highlighted_drop_target_index = None
        self.original_item_brush = None

    def _highlight_drop_target(self, event):
        """Highlight the cluster header being hovered over."""
        pos = event.position().toPoint()
        proxy_index = self.indexAt(pos)

        if not proxy_index.isValid():
            self._clear_drop_highlight()
            return

        source_index = self.main_window.proxy_model.mapToSource(proxy_index)

        # Clear previous highlight if different
        if self.highlighted_drop_target_index != source_index:
            self._clear_drop_highlight()

        item = self.main_window.file_system_model.itemFromIndex(source_index)
        if not item:
            return

        item_data = item.data(Qt.ItemDataRole.UserRole)
        if isinstance(item_data, str) and item_data.startswith("cluster_header_"):
            if self.highlighted_drop_target_index != source_index:
                self.original_item_brush = item.background()
                self.highlighted_drop_target_index = source_index
                # Use a light highlight color
                from PyQt6.QtGui import QColor, QBrush

                item.setBackground(QBrush(QColor(100, 150, 200, 100)))

    def dragMoveEvent(self, event: QDragMoveEvent | None):
        if event and self._is_cluster_drop_valid(event):
            self._highlight_drop_target(event)
            event.acceptProposedAction()
        elif event:
            self._clear_drop_highlight()
            event.ignore()

    def dragLeaveEvent(self, event):
        self._clear_drop_highlight()
        super().dragLeaveEvent(event)

    @override
    def dropEvent(self, event: QDropEvent | None):
        if not event:
            return

        if not self._is_cluster_drop_valid(event):
            self._clear_drop_highlight()
            event.ignore()
            return

        pos = event.position().toPoint()
        proxy_index = self.indexAt(pos)
        target_cluster_id = self._get_cluster_id_from_index(proxy_index)

        self._clear_drop_highlight()

        if target_cluster_id is not None:
            self._move_dragged_items_to_cluster(target_cluster_id)
            event.acceptProposedAction()
        else:
            event.ignore()

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

    def sizeHint(
        self,
        option: QStyleOptionViewItem | None,
        index: QModelIndex | None,
    ) -> QSize:
        """Reserve stable row height before asynchronous thumbnails arrive."""
        size = super().sizeHint(option, index)
        view = option.widget if option is not None else None
        if view is None or not hasattr(view, "iconSize"):
            return size
        icon_size = view.iconSize()
        if icon_size.isValid() and icon_size.height() > 0:
            size.setHeight(max(size.height(), icon_size.height() + 4))
        return size

    def paint(
        self,
        painter: QPainter | None,
        option: QStyleOptionViewItem | None,
        index: QModelIndex | None,
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
        self._base_flags = self.windowFlags()
        self._floating = False

        main_layout = QVBoxLayout(self)
        main_layout.setContentsMargins(0, 0, 0, 0)
        main_layout.setSpacing(0)

        self.progress_view = WorkflowProgressView(
            "Working on your library",
            default_message="Loading…",
            parent=self,
        )
        self.bg_widget = self.progress_view
        self.text_label = self.progress_view.message_label
        main_layout.addWidget(self.progress_view)
        self.hide()

    def setText(self, text):
        match = re.search(r"\((\d+)%\)", text)
        percent = int(match.group(1)) if match else -1
        self.progress_view.update_progress(text, percent)

    @override
    def showEvent(self, event):
        if self.parentWidget():
            self.setGeometry(self.parentWidget().rect())
        super().showEvent(event)

    @override
    def hideEvent(self, event):
        self.progress_view.mark_finished()
        super().hideEvent(event)

    def update_position(self):
        if self.parentWidget() and self.isVisible():
            parent = self.parentWidget()
            if self._floating:
                top_left = parent.mapToGlobal(QPoint(0, 0))
                self.setGeometry(QRect(top_left, parent.size()))
            else:
                self.setGeometry(parent.rect())
            self.raise_()

    def set_floating(self, enabled: bool):
        if self._floating == enabled:
            return
        self._floating = enabled
        if enabled:
            self.setWindowFlags(
                Qt.WindowType.Tool
                | Qt.WindowType.FramelessWindowHint
                | Qt.WindowType.WindowStaysOnTopHint
            )
        else:
            self.setWindowFlags(self._base_flags)
        if self.isVisible():
            self.hide()
            self.show()


# --- Similarity Engine Worker ---
class SimilarityWorker(QObject):
    """Worker for running similarity analysis in the background."""

    progress_update = pyqtSignal(int, str)
    embeddings_generated = pyqtSignal(object)  # Using object to pass dict
    regional_embeddings_generated = pyqtSignal(object)
    clustering_complete = pyqtSignal(object)  # Using object to pass dict
    error = pyqtSignal(str)
    finished = pyqtSignal()

    def __init__(
        self,
        file_paths: list[str],
        allow_model_download: bool = False,
        image_pipeline: ImagePipeline | None = None,
        folder_path: str | None = None,
        analysis_cache=None,
        fingerprints: dict[str, tuple[int, int]] | None = None,
        parent=None,
    ):
        super().__init__(parent)
        self.file_paths = file_paths
        self.allow_model_download = allow_model_download
        self._is_running = True
        self.similarity_engine = None
        self.image_pipeline = image_pipeline
        self.folder_path = folder_path
        self.analysis_cache = analysis_cache
        self.fingerprints = fingerprints or {}
        self._similarity_signature = ""

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
        if not self._is_running:
            self.finished.emit()
            return
        try:
            from core.similarity_engine import SimilarityEngine
            from core.similarity_cache import (
                SimilarityClusteringResult,
                normalize_fingerprints,
            )
            from core.similarity_clustering import build_signature, load_cached_clusters

            # 1. Instantiate the engine inside the worker thread
            self.similarity_engine = SimilarityEngine(
                allow_model_download=self.allow_model_download,
                image_pipeline=self.image_pipeline,
            )
            if not self._is_running:
                self.similarity_engine.stop()
                self.finished.emit()
                return

            normalized_fingerprints = normalize_fingerprints(
                self.file_paths, self.fingerprints
            )
            self._similarity_signature = build_signature(
                self.similarity_engine, self.file_paths, normalized_fingerprints
            )
            cached_clusters = load_cached_clusters(
                self.analysis_cache,
                self.folder_path,
                signature=self._similarity_signature,
                expected_paths=set(self.file_paths),
            )

            # 2. Connect its signals to this worker's signals
            self.similarity_engine.progress_update.connect(self.progress_update)
            self.similarity_engine.embeddings_generated.connect(
                self.embeddings_generated
            )
            self.similarity_engine.regional_embeddings_generated.connect(
                self.regional_embeddings_generated
            )
            self.similarity_engine.clustering_complete.connect(
                self._handle_clustering_complete
            )
            self.similarity_engine.error.connect(self.error)

            # 3. Connect the final error signal to this worker's finished signal
            self.similarity_engine.error.connect(self.finished)

            # 4. Start the process
            self.similarity_engine.generate_embeddings_for_files(
                self.file_paths,
                fingerprints=normalized_fingerprints,
                perform_clustering=cached_clusters is None,
            )
            if cached_clusters is not None and self._is_running:
                logger.info(
                    "Reusing %d cached similarity cluster assignments.",
                    len(cached_clusters),
                )
                self.clustering_complete.emit(
                    SimilarityClusteringResult(
                        clusters=cached_clusters,
                        signature=self._similarity_signature,
                        reused=True,
                    )
                )
                self.finished.emit()

        except Exception as e:
            logger.error(
                f"Error initializing or running SimilarityEngine: {e}", exc_info=True
            )
            self.error.emit(str(e))
            self.finished.emit()

    def _handle_clustering_complete(self, cluster_results: dict[str, int]) -> None:
        """Apply and persist analysis-cache state before returning to the UI."""

        if not self._is_running:
            self.finished.emit()
            return

        from core.similarity_cache import SimilarityClusteringResult
        from core.similarity_clustering import persist_clusters

        results = persist_clusters(
            self.analysis_cache,
            self.folder_path,
            cluster_results,
            signature=self._similarity_signature,
        )
        self.clustering_complete.emit(
            SimilarityClusteringResult(
                clusters=results,
                signature=self._similarity_signature,
                reused=False,
            )
        )
        self.finished.emit()

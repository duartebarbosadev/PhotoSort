import time
import logging # Added for startup logging
from PyQt6.QtWidgets import QStyledItemDelegate, QStyleOptionViewItem
from PyQt6.QtGui import QPainter, QMovie # For animated GIFs
from PyQt6.QtWidgets import (
    QMainWindow, QWidget, QVBoxLayout, QHBoxLayout, QSplitter, QLabel, QFrame,
    QFileDialog, QTreeView, # Replaced QListWidget with QTreeView
    QPushButton, QListView, QComboBox,
    QLineEdit, # For search input
    QStyle, # For standard icons
    QAbstractItemView, QMessageBox, QApplication # For selection and edit triggersor dialogs
)
import re # For regular expressions in filtering
import os # <-- Add import os at the top level
import send2trash # <-- Import send2trash for moving files to trash
import traceback # For detailed error logging
from datetime import date as date_obj, datetime # For date type hinting and objects
from typing import List, Dict, Optional, Any, Tuple # Import List and Dict for type hinting, Optional, Any, Tuple
from PyQt6.QtCore import Qt, QThread, QSize, QModelIndex, QMimeData, QUrl, QSortFilterProxyModel, QObject, pyqtSignal, QTimer, QPersistentModelIndex, QItemSelectionModel, QEvent # Import QEvent for eventFilter
from PyQt6.QtGui import QColor # Import QColor for highlighting
from PyQt6.QtGui import QAction, QKeySequence, QPixmap, QKeyEvent, QIcon, QStandardItemModel, QStandardItem, QResizeEvent, QDragEnterEvent, QDropEvent, QDragMoveEvent # Import model classes and event types
import numpy as np
from sklearn.decomposition import PCA
from sklearn.metrics.pairwise import cosine_similarity # Add cosine_similarity import
# from src.core.file_scanner import FileScanner # Now managed by WorkerManager
# from src.core.similarity_engine import SimilarityEngine # Now managed by WorkerManager
# from src.core.similarity_engine import PYTORCH_CUDA_AVAILABLE # Import PyTorch CUDA info <-- ENSURE REMOVED
from src.core.image_pipeline import ImagePipeline
from src.core.image_file_ops import ImageFileOperations
# from src.core.image_features.blur_detector import BlurDetector # Now managed by WorkerManager
from src.core.rating_handler import MetadataHandler # Renamed from RatingHandler
from src.core.app_settings import get_preview_cache_size_gb, set_preview_cache_size_gb, get_preview_cache_size_bytes, get_exif_cache_size_mb, set_exif_cache_size_mb, get_exiftool_executable_path, set_exiftool_executable_path, get_default_exiftool_name # Import settings
from PyQt6.QtWidgets import QFormLayout, QComboBox, QSizePolicy # For cache dialog
from src.ui.app_state import AppState # Import AppState
from src.core.caching.rating_cache import RatingCache # Import RatingCache for type hinting
from src.core.caching.exif_cache import ExifCache # Import ExifCache for type hinting and methods
from src.ui.ui_components import LoadingOverlay # PreviewPreloaderWorker, BlurDetectionWorker are used by WorkerManager
from src.ui.worker_manager import WorkerManager # Import WorkerManager
from src.core.file_scanner import SUPPORTED_EXTENSIONS # Import from file_scanner


# --- Custom Tree View for Drag and Drop ---
class DroppableTreeView(QTreeView):
    def __init__(self, model, main_window, parent=None):
        super().__init__(parent)
        self.setModel(model)
        self.main_window = main_window # To access AppState
        self.viewport().setAcceptDrops(False) # Disable drag and drop
        # self.setDefaultDropAction(Qt.DropAction.MoveAction) # Disable drag and drop
        self.highlighted_drop_target_index = None
        self.original_item_brush = None

    def dragEnterEvent(self, event: QDragEnterEvent):
        event.ignore() # Disable drag and drop

    def _clear_drop_highlight(self):
        if self.highlighted_drop_target_index and self.highlighted_drop_target_index.isValid():
            item = self.model().itemFromIndex(self.highlighted_drop_target_index)
            if item:
                item.setBackground(self.original_item_brush if self.original_item_brush else QStandardItem().background())
        self.highlighted_drop_target_index = None
        self.original_item_brush = None

    def dragMoveEvent(self, event: QDragMoveEvent):
        event.ignore() # Disable drag and drop

    def dragLeaveEvent(self, event):
        self._clear_drop_highlight()
        super().dragLeaveEvent(event)


    def dropEvent(self, event: QDropEvent):
        event.ignore() # Disable drag and drop

# --- Custom Proxy Model for Filtering ---
class CustomFilterProxyModel(QSortFilterProxyModel):
    def __init__(self, parent=None):
        super().__init__(parent)
        self.current_rating_filter = "Show All"
        self.current_cluster_filter_id = -1
        self.app_state_ref: Optional[AppState] = None 
        self.show_folders_mode_ref = False
        self.current_view_mode_ref = "list"

    def _check_item_passes_filter(self, item: QStandardItem) -> bool:
        item_user_data = item.data(Qt.ItemDataRole.UserRole)
        item_text = item.text()
        
        is_image_item = isinstance(item_user_data, dict) and 'path' in item_user_data
        
        if not is_image_item:
            return False

        file_path = item_user_data['path']
        if not os.path.exists(file_path): 
            return False

        search_text = self.filterRegularExpression().pattern().lower()
        search_match = search_text in item_text.lower()
        if not search_match: return False

        if not self.app_state_ref: return True 

        current_rating = self.app_state_ref.rating_cache.get(file_path, 0) # Uses in-memory cache, populated by RatingLoaderWorker
        rating_filter = self.current_rating_filter
        rating_passes = (
            rating_filter == "Show All" or
            (rating_filter == "Unrated (0)" and current_rating == 0) or
            (rating_filter == "1 Star +" and current_rating >= 1) or
            (rating_filter == "2 Stars +" and current_rating >= 2) or
            (rating_filter == "3 Stars +" and current_rating >= 3) or
            (rating_filter == "4 Stars +" and current_rating >= 4) or
            (rating_filter == "5 Stars" and current_rating == 5)
        )
        if not rating_passes: return False

        cluster_filter_id = self.current_cluster_filter_id
        cluster_passes = (
            cluster_filter_id == -1 or
            (self.app_state_ref.cluster_results.get(file_path) == cluster_filter_id)
        )
        if not cluster_passes: return False
        return True

    def filterAcceptsRow(self, source_row: int, source_parent: QModelIndex) -> bool:
        source_model = self.sourceModel()
        source_index = source_model.index(source_row, 0, source_parent)
        if not source_index.isValid():
            return False

        item = source_model.itemFromIndex(source_index)
        if not item:
            return False

        if self._check_item_passes_filter(item):
            return True

        if item.hasChildren():
            for i in range(item.rowCount()):
                if self.filterAcceptsRow(i, source_index): 
                    return True
        return False


class MainWindow(QMainWindow):
    """Main application window."""

    def __init__(self):
        super().__init__()
        init_start_time = time.perf_counter()
        logging.info("MainWindow.__init__ - Start")

        self.image_pipeline = ImagePipeline()
        logging.info(f"MainWindow.__init__ - ImagePipeline instantiated: {time.perf_counter() - init_start_time:.4f}s")
        self.image_file_ops = ImageFileOperations()
        logging.info(f"MainWindow.__init__ - ImageFileOperations instantiated: {time.perf_counter() - init_start_time:.4f}s")
        self.app_state = AppState()
        logging.info(f"MainWindow.__init__ - AppState instantiated: {time.perf_counter() - init_start_time:.4f}s")
        self.worker_manager = WorkerManager(image_pipeline_instance=self.image_pipeline, parent=self)
        logging.info(f"MainWindow.__init__ - WorkerManager instantiated: {time.perf_counter() - init_start_time:.4f}s")
        
        self.setWindowTitle("PhotoRanker")
        self.setGeometry(100, 100, 1200, 800)
 
        self.loading_overlay = None
        
        self.thumbnail_delegate = None
        self.current_view_mode = None
        self.show_folders_mode = False
        self.group_by_similarity_mode = False
        self.apply_auto_edits_enabled = False
        self.blur_detection_threshold = 100.0
 
        section_start_time = time.perf_counter()
        self._create_menu()
        logging.info(f"MainWindow.__init__ - _create_menu done: {time.perf_counter() - section_start_time:.4f}s (Total: {time.perf_counter() - init_start_time:.4f}s)")
        
        section_start_time = time.perf_counter()
        self._create_widgets()
        logging.info(f"MainWindow.__init__ - _create_widgets done: {time.perf_counter() - section_start_time:.4f}s (Total: {time.perf_counter() - init_start_time:.4f}s)")
        
        section_start_time = time.perf_counter()
        self._create_layout()
        logging.info(f"MainWindow.__init__ - _create_layout done: {time.perf_counter() - section_start_time:.4f}s (Total: {time.perf_counter() - init_start_time:.4f}s)")
        
        section_start_time = time.perf_counter()
        self._create_loading_overlay()
        logging.info(f"MainWindow.__init__ - _create_loading_overlay done: {time.perf_counter() - section_start_time:.4f}s (Total: {time.perf_counter() - init_start_time:.4f}s)")
        
        section_start_time = time.perf_counter()
        self._create_actions()
        logging.info(f"MainWindow.__init__ - _create_actions done: {time.perf_counter() - section_start_time:.4f}s (Total: {time.perf_counter() - init_start_time:.4f}s)")
        
        section_start_time = time.perf_counter()
        self._connect_signals()
        logging.info(f"MainWindow.__init__ - _connect_signals done: {time.perf_counter() - section_start_time:.4f}s (Total: {time.perf_counter() - init_start_time:.4f}s)")
        
        section_start_time = time.perf_counter()
        self._set_view_mode_list()
        logging.info(f"MainWindow.__init__ - _set_view_mode_list done: {time.perf_counter() - section_start_time:.4f}s (Total: {time.perf_counter() - init_start_time:.4f}s)")
        
        self._update_image_info_label() # Set initial info label text
        logging.info(f"MainWindow.__init__ - _update_image_info_label done: {time.perf_counter() - section_start_time:.4f}s (Total: {time.perf_counter() - init_start_time:.4f}s)")

        logging.info(f"MainWindow.__init__ - End (Total: {time.perf_counter() - init_start_time:.4f}s)")

    # Helper method to update the image information status label
    def _update_image_info_label(self, status_message_override: Optional[str] = None):
        if not hasattr(self, 'image_info_label'): # Widget might not be created yet
            return

        if status_message_override:
            self.image_info_label.setText(status_message_override)
            return

        num_images = 0
        total_size_mb = 0.0
        folder_name_display = "N/A"
        # Default text if no folder is loaded yet
        status_text = "No folder loaded. Open a folder to begin."

        if self.app_state.current_folder_path:
            folder_name_display = os.path.basename(self.app_state.current_folder_path)
            if not folder_name_display: # Handles "C:/"
                folder_name_display = self.app_state.current_folder_path

            # Determine if scan is considered "active" based on UI elements
            # open_folder_action is disabled during the scan process.
            scan_logically_active = not self.open_folder_action.isEnabled()

            if scan_logically_active:
                # Scan is in progress
                num_images_found_so_far = len(self.app_state.image_files_data) # Current count during scan
                status_text = f"Folder: {folder_name_display}  |  Scanning... ({num_images_found_so_far} files found)"
            elif self.app_state.image_files_data: # Scan is finished and there's data
                num_images = len(self.app_state.image_files_data)
                current_files_size_bytes = 0
                for file_data in self.app_state.image_files_data:
                    try:
                        if 'path' in file_data and os.path.exists(file_data['path']):
                             current_files_size_bytes += os.path.getsize(file_data['path'])
                    except OSError as e:
                        # Log lightly, this can be noisy if many files are temporarily unavailable
                        logging.debug(f"Could not get size for {file_data.get('path')} for info label: {e}")
                total_size_mb = current_files_size_bytes / (1024 * 1024)
                status_text = f"Folder: {folder_name_display}  |  Images: {num_images} ({total_size_mb:.2f} MB)"
            else: # Folder path set, scan finished (or not started if folder just selected), no image data
                status_text = f"Folder: {folder_name_display}  |  Images: 0 (0.00 MB)"
        
        self.image_info_label.setText(status_text)

    def _create_loading_overlay(self):
        start_time = time.perf_counter()
        logging.debug("MainWindow._create_loading_overlay - Start")
        parent_for_overlay = self
        if parent_for_overlay:
            self.loading_overlay = LoadingOverlay(parent_for_overlay)
            self.loading_overlay.hide()
        else:
            print("Warning: Could not create loading overlay, parent widget not available yet.")
        logging.debug(f"MainWindow._create_loading_overlay - End: {time.perf_counter() - start_time:.4f}s")

    def show_loading_overlay(self, text="Loading..."):
        if self.loading_overlay:
            self.loading_overlay.setText(text)
            self.loading_overlay.update_position() 
            self.loading_overlay.show()
            QApplication.processEvents() 

    def update_loading_text(self, text):
        if self.loading_overlay and self.loading_overlay.isVisible():
            self.loading_overlay.setText(text)
            QApplication.processEvents()

    def hide_loading_overlay(self):
        if self.loading_overlay:
            self.loading_overlay.hide()
            QApplication.processEvents()

    def _create_menu(self):
        start_time = time.perf_counter()
        logging.debug("MainWindow._create_menu - Start")
        menu_bar = self.menuBar()
        file_menu = menu_bar.addMenu("&File")
        self.open_folder_action = QAction("&Open Folder...", self)
        self.open_folder_action.setShortcut(QKeySequence.StandardKey.Open)
        file_menu.addAction(self.open_folder_action)
        exit_action = QAction("&Exit", self)
        exit_action.setShortcut(QKeySequence.StandardKey.Quit)
        exit_action.triggered.connect(self.close)
        file_menu.addAction(exit_action)

        view_menu = menu_bar.addMenu("&View")
        self.toggle_folder_view_action = QAction("Show Images in Folders", self)
        self.toggle_folder_view_action.setCheckable(True)
        self.toggle_folder_view_action.setChecked(self.show_folders_mode)
        view_menu.addAction(self.toggle_folder_view_action)
        self.group_by_similarity_action = QAction("Group by Similarity", self)
        self.group_by_similarity_action.setCheckable(True)
        self.group_by_similarity_action.setChecked(False)
        self.group_by_similarity_action.setEnabled(False)
        view_menu.addAction(self.group_by_similarity_action)
        view_menu.addSeparator()
        self.toggle_thumbnails_action = QAction("Show Thumbnails", self)
        self.toggle_thumbnails_action.setCheckable(True)
        self.toggle_thumbnails_action.setChecked(True)
        view_menu.addAction(self.toggle_thumbnails_action)
        view_menu.addSeparator()
        self.analyze_similarity_action = QAction("Analyze Similarity", self)
        self.analyze_similarity_action.setToolTip("Generate image embeddings and find similar groups (can be slow)")
        self.analyze_similarity_action.setEnabled(False)
        view_menu.addAction(self.analyze_similarity_action)

        self.detect_blur_action = QAction("Detect Blurriness", self)
        self.detect_blur_action.setToolTip("Analyze images for blurriness (can be slow for many images)")
        self.detect_blur_action.setEnabled(False) 
        view_menu.addAction(self.detect_blur_action)

        help_menu = menu_bar.addMenu("&Help")
        about_action = QAction("&About", self)
        about_action.triggered.connect(self._show_about_dialog)
        help_menu.addAction(about_action)
        self._create_settings_menu()
        logging.debug(f"MainWindow._create_menu - End: {time.perf_counter() - start_time:.4f}s")

    def _create_settings_menu(self):
        start_time = time.perf_counter()
        logging.debug("MainWindow._create_settings_menu - Start")
        settings_menu = self.menuBar().addMenu("&Settings")
        manage_cache_action = QAction("Manage Cache", self)
        manage_cache_action.triggered.connect(self._show_cache_management_dialog)
        settings_menu.addAction(manage_cache_action)
        settings_menu.addSeparator()
        self.toggle_auto_edits_action = QAction("Enable Auto RAW Edits", self)
        self.toggle_auto_edits_action.setCheckable(True)
        self.toggle_auto_edits_action.setChecked(self.apply_auto_edits_enabled)
        self.toggle_auto_edits_action.setToolTip("Apply automatic brightness, contrast, and color adjustments to RAW previews and thumbnails.")
        settings_menu.addAction(self.toggle_auto_edits_action)
        settings_menu.addSeparator() # Separator before ExifTool path setting
        self.set_exiftool_path_action = QAction("Set ExifTool Path...", self)
        self.set_exiftool_path_action.setToolTip("Specify the location of the ExifTool executable.")
        settings_menu.addAction(self.set_exiftool_path_action)
        logging.debug(f"MainWindow._create_settings_menu - End: {time.perf_counter() - start_time:.4f}s")

    def _show_set_exiftool_path_dialog(self):
        current_path = get_exiftool_executable_path()
        default_name = get_default_exiftool_name()

        dialog_text = "Select the ExifTool executable.\n"
        if current_path:
            dialog_text += f"Current: {current_path}\n"
        else:
            dialog_text += "Current: Using system PATH (ExifTool not explicitly set).\n"
        dialog_text += "If cleared, PhotoRanker will try to find ExifTool in your system's PATH."

        file_path, _ = QFileDialog.getOpenFileName(
            self,
            "Select ExifTool Executable",
            os.path.dirname(current_path) if current_path else "", # Start in current dir if set
            f"ExifTool Executable ({default_name});;All Files (*)"
        )

        if file_path:
            if os.path.isfile(file_path) and (os.access(file_path, os.X_OK) or file_path.endswith(".exe")):
                set_exiftool_executable_path(file_path)
                self.statusBar().showMessage(f"ExifTool path set to: {file_path}", 5000)
                logging.info(f"[MainWindow] ExifTool path set to: {file_path}")
                # Optional: Ping ExifTool or re-verify functionality if critical
            else:
                QMessageBox.warning(self, "Invalid ExifTool Path",
                                    f"The selected file '{file_path}' is not a valid or executable file.")
                logging.warning(f"[MainWindow] Invalid ExifTool path selected: {file_path}")
        elif file_path == "": # User pressed cancel or closed dialog
            # Offer to clear the path if one was set
            if current_path:
                reply = QMessageBox.question(self, "Clear ExifTool Path?",
                                             "No file selected. Do you want to clear the current ExifTool path and revert to using the system PATH?",
                                             QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.No,
                                             QMessageBox.StandardButton.No)
                if reply == QMessageBox.StandardButton.Yes:
                    set_exiftool_executable_path(None)
                    self.statusBar().showMessage("ExifTool path cleared. Will use system PATH.", 5000)
                    logging.info("[MainWindow] ExifTool path cleared by user.")
            else:
                self.statusBar().showMessage("ExifTool path selection cancelled. No changes made.", 3000)


    def _show_cache_management_dialog(self):
        from PyQt6.QtWidgets import QDialog, QVBoxLayout, QLabel, QPushButton, QFrame, QGridLayout, QSpacerItem

        dialog = QDialog(self)
        dialog.setWindowTitle("Cache Management")
        dialog.setObjectName("cacheManagementDialog")
        main_layout = QVBoxLayout(dialog)
        main_layout.setSpacing(15)

        thumb_section_title = QLabel("Thumbnail Cache")
        thumb_section_title.setObjectName("cacheSectionTitle")
        main_layout.addWidget(thumb_section_title)

        thumb_frame = QFrame()
        thumb_frame.setObjectName("cacheSectionFrame")
        thumb_layout = QGridLayout(thumb_frame) 

        self.thumb_cache_usage_label = QLabel() 
        thumb_layout.addWidget(QLabel("Current Disk Usage:"), 0, 0)
        thumb_layout.addWidget(self.thumb_cache_usage_label, 0, 1)

        delete_thumb_cache_button = QPushButton("Clear Thumbnail Cache")
        delete_thumb_cache_button.setObjectName("deleteThumbnailCacheButton")
        delete_thumb_cache_button.clicked.connect(self._clear_thumbnail_cache_action)
        thumb_layout.addWidget(delete_thumb_cache_button, 1, 0, 1, 2)
        
        main_layout.addWidget(thumb_frame)

        preview_section_title = QLabel("Preview Image Cache")
        preview_section_title.setObjectName("cacheSectionTitle")
        main_layout.addWidget(preview_section_title)

        preview_frame = QFrame()
        preview_frame.setObjectName("cacheSectionFrame")
        preview_layout = QGridLayout(preview_frame)

        self.preview_cache_configured_limit_label = QLabel()
        preview_layout.addWidget(QLabel("Configured Size Limit:"), 0, 0)
        preview_layout.addWidget(self.preview_cache_configured_limit_label, 0, 1)

        self.preview_cache_usage_label = QLabel()
        preview_layout.addWidget(QLabel("Current Disk Usage:"), 1, 0)
        preview_layout.addWidget(self.preview_cache_usage_label, 1, 1)

        preview_layout.addWidget(QLabel("Set New Limit (GB):"), 2, 0)
        self.preview_cache_size_combo = QComboBox()
        self.preview_cache_size_combo.setObjectName("previewCacheSizeCombo")
        self.preview_cache_size_options_gb = [0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0]
        self.preview_cache_size_combo.addItems([f"{size:.2f} GB" for size in self.preview_cache_size_options_gb])
        
        current_conf_gb = get_preview_cache_size_gb()
        try:
            current_index = self.preview_cache_size_options_gb.index(current_conf_gb)
            self.preview_cache_size_combo.setCurrentIndex(current_index)
        except ValueError:
            self.preview_cache_size_combo.addItem(f"{current_conf_gb:.2f} GB (Custom)")
            self.preview_cache_size_combo.setCurrentIndex(self.preview_cache_size_combo.count() - 1)

        preview_layout.addWidget(self.preview_cache_size_combo, 2, 1)

        apply_preview_limit_button = QPushButton("Apply New Limit")
        apply_preview_limit_button.setObjectName("applyPreviewLimitButton")
        apply_preview_limit_button.clicked.connect(self._apply_preview_cache_limit_action)
        preview_layout.addWidget(apply_preview_limit_button, 3, 0, 1, 2)

        delete_preview_cache_button = QPushButton("Clear Preview Cache")
        delete_preview_cache_button.setObjectName("deletePreviewCacheButton")
        delete_preview_cache_button.clicked.connect(self._clear_preview_cache_action)
        preview_layout.addWidget(delete_preview_cache_button, 4, 0, 1, 2)

        main_layout.addWidget(preview_frame)

        # --- EXIF Cache Section ---
        exif_section_title = QLabel("EXIF Metadata Cache")
        exif_section_title.setObjectName("cacheSectionTitle")
        main_layout.addWidget(exif_section_title)

        exif_frame = QFrame()
        exif_frame.setObjectName("cacheSectionFrame")
        exif_layout = QGridLayout(exif_frame)

        self.exif_cache_configured_limit_label = QLabel()
        exif_layout.addWidget(QLabel("Configured Size Limit:"), 0, 0)
        exif_layout.addWidget(self.exif_cache_configured_limit_label, 0, 1)

        self.exif_cache_usage_label = QLabel()
        exif_layout.addWidget(QLabel("Current Disk Usage:"), 1, 0)
        exif_layout.addWidget(self.exif_cache_usage_label, 1, 1)

        exif_layout.addWidget(QLabel("Set New Limit (MB):"), 2, 0)
        self.exif_cache_size_combo = QComboBox()
        self.exif_cache_size_combo.setObjectName("exifCacheSizeCombo") # Unique object name
        self.exif_cache_size_options_mb = [64, 128, 256, 512, 1024] # MB options
        self.exif_cache_size_combo.addItems([f"{size} MB" for size in self.exif_cache_size_options_mb])

        current_exif_conf_mb = get_exif_cache_size_mb()
        try:
            current_exif_index = self.exif_cache_size_options_mb.index(current_exif_conf_mb)
            self.exif_cache_size_combo.setCurrentIndex(current_exif_index)
        except ValueError:
            self.exif_cache_size_combo.addItem(f"{current_exif_conf_mb} MB (Custom)")
            self.exif_cache_size_combo.setCurrentIndex(self.exif_cache_size_combo.count() - 1)
        exif_layout.addWidget(self.exif_cache_size_combo, 2, 1)

        apply_exif_limit_button = QPushButton("Apply New EXIF Limit")
        apply_exif_limit_button.setObjectName("applyExifLimitButton")
        apply_exif_limit_button.clicked.connect(self._apply_exif_cache_limit_action)
        exif_layout.addWidget(apply_exif_limit_button, 3, 0, 1, 2)

        delete_exif_cache_button = QPushButton("Clear EXIF Cache")
        delete_exif_cache_button.setObjectName("deleteExifCacheButton") # Unique object name
        delete_exif_cache_button.clicked.connect(self._clear_exif_cache_action)
        exif_layout.addWidget(delete_exif_cache_button, 4, 0, 1, 2)

        main_layout.addWidget(exif_frame)
        
        main_layout.addSpacerItem(QSpacerItem(20, 40, QSizePolicy.Policy.Minimum, QSizePolicy.Policy.Expanding))
        close_button = QPushButton("Close")
        close_button.setObjectName("cacheDialogCloseButton")
        close_button.clicked.connect(dialog.accept)
        main_layout.addWidget(close_button)

        self._update_cache_dialog_labels() 
        dialog.setLayout(main_layout)
        dialog.exec()

    def _update_cache_dialog_labels(self):
        thumb_usage_bytes = self.image_pipeline.thumbnail_cache.volume()
        self.thumb_cache_usage_label.setText(f"{thumb_usage_bytes / (1024*1024):.2f} MB")

        configured_gb = get_preview_cache_size_gb() 
        self.preview_cache_configured_limit_label.setText(f"{configured_gb:.2f} GB")
        
        preview_usage_bytes = self.image_pipeline.preview_cache.volume()
        self.preview_cache_usage_label.setText(f"{preview_usage_bytes / (1024*1024):.2f} MB")

        # Update EXIF cache labels
        if hasattr(self, 'app_state') and self.app_state.exif_disk_cache:
            exif_configured_mb = self.app_state.exif_disk_cache.get_current_size_limit_mb()
            self.exif_cache_configured_limit_label.setText(f"{exif_configured_mb} MB")
            exif_usage_bytes = self.app_state.exif_disk_cache.volume()
            self.exif_cache_usage_label.setText(f"{exif_usage_bytes / (1024*1024):.2f} MB")
        else: # Fallback if app_state or exif_disk_cache is not yet fully initialized
            self.exif_cache_configured_limit_label.setText("N/A")
            self.exif_cache_usage_label.setText("N/A")

    def _clear_thumbnail_cache_action(self):
        self.image_pipeline.thumbnail_cache.clear()
        self.statusBar().showMessage("Thumbnail cache cleared.", 5000)
        self._update_cache_dialog_labels()
        self._refresh_visible_items_icons()

    def _clear_preview_cache_action(self):
        self.image_pipeline.preview_cache.clear() 
        self.statusBar().showMessage("Preview cache cleared. Previews will regenerate.", 5000)
        self._update_cache_dialog_labels()
        self._refresh_current_selection_preview()

    def _apply_preview_cache_limit_action(self):
        selected_index = self.preview_cache_size_combo.currentIndex()
        new_size_gb = 0
        if self.preview_cache_size_combo.itemText(selected_index).endswith("(Custom)"):
            new_size_gb = float(self.preview_cache_size_combo.itemText(selected_index).split(" ")[0])
        elif 0 <= selected_index < len(self.preview_cache_size_options_gb):
            new_size_gb = self.preview_cache_size_options_gb[selected_index]
        else:
            self.statusBar().showMessage("Invalid selection for cache size.", 3000)
            return

        current_size_gb = get_preview_cache_size_gb()
        if new_size_gb != current_size_gb:
            set_preview_cache_size_gb(new_size_gb)
            self.image_pipeline.reinitialize_preview_cache_from_settings()
            self.statusBar().showMessage(f"Preview cache limit set to {new_size_gb:.2f} GB. Cache reinitialized.", 5000)
        else:
            self.statusBar().showMessage(f"Preview cache limit is already {new_size_gb:.2f} GB.", 3000)
        self._update_cache_dialog_labels()

    def _clear_exif_cache_action(self):
        if self.app_state.exif_disk_cache:
            self.app_state.exif_disk_cache.clear()
            self.statusBar().showMessage("EXIF cache cleared.", 5000)
            self._update_cache_dialog_labels()
            # No direct visual refresh needed for EXIF data itself in list/grid,
            # but metadata display for current image might need update
            self._refresh_current_selection_preview() # This will re-fetch metadata

    def _apply_exif_cache_limit_action(self):
        selected_index = self.exif_cache_size_combo.currentIndex()
        new_size_mb = 0
        if self.exif_cache_size_combo.itemText(selected_index).endswith("(Custom)"):
            new_size_mb = int(self.exif_cache_size_combo.itemText(selected_index).split(" ")[0])
        elif 0 <= selected_index < len(self.exif_cache_size_options_mb):
            new_size_mb = self.exif_cache_size_options_mb[selected_index]
        else:
            self.statusBar().showMessage("Invalid selection for EXIF cache size.", 3000)
            return

        if self.app_state.exif_disk_cache:
            current_size_mb = self.app_state.exif_disk_cache.get_current_size_limit_mb()
            if new_size_mb != current_size_mb:
                set_exif_cache_size_mb(new_size_mb) # Update app_settings
                self.app_state.exif_disk_cache.reinitialize_from_settings() # Reinitialize ExifCache
                self.statusBar().showMessage(f"EXIF cache limit set to {new_size_mb} MB. Cache reinitialized.", 5000)
            else:
                self.statusBar().showMessage(f"EXIF cache limit is already {new_size_mb} MB.", 3000)
        self._update_cache_dialog_labels()
        
    def _refresh_visible_items_icons(self):
        active_view = self._get_active_file_view()
        if active_view:
            active_view.viewport().update()


    def _refresh_current_selection_preview(self):
        active_view = self._get_active_file_view()
        if active_view and active_view.currentIndex().isValid():
            self._handle_file_selection_changed() 

    def _show_about_dialog(self):
        from src.core.app_settings import is_pytorch_cuda_available, get_exiftool_executable_path # <-- IMPORT FROM APP_SETTINGS
        clustering_info = "Clustering Algorithm: DBSCAN (scikit-learn)"
        
        exiftool_path_status = "Using system PATH"
        configured_exiftool_path = get_exiftool_executable_path()
        if configured_exiftool_path:
            exiftool_path_status = f"Configured: {configured_exiftool_path}"
        
        about_text = (
            "PhotoRanker\n"
            "Version: 1.0b\n"
            "Author: Duarte Barbosa\n\n"
            "Technology Used:\n"
            f"  - Embeddings: SentenceTransformer (CLIP) on {'GPU (CUDA)' if is_pytorch_cuda_available() else 'CPU'}\n"
            f"  - {clustering_info}\n"
            f"  - ExifTool: {exiftool_path_status}"
        )
        QMessageBox.information(self, "About PhotoRanker", about_text)

    def _create_widgets(self):
        """Create the UI widgets."""
        start_time = time.perf_counter()
        logging.debug("MainWindow._create_widgets - Start")
        self.file_system_model = QStandardItemModel()
        self.proxy_model = CustomFilterProxyModel(self)
        self.proxy_model.setSourceModel(self.file_system_model)
        self.proxy_model.app_state_ref = self.app_state # Link AppState to proxy model

        self.tree_display_view = DroppableTreeView(self.proxy_model, self)
        self.tree_display_view.setHeaderHidden(True)
        self.tree_display_view.setIndentation(15)
        self.tree_display_view.setSelectionMode(QAbstractItemView.SelectionMode.ExtendedSelection)
        self.tree_display_view.setEditTriggers(QAbstractItemView.EditTrigger.NoEditTriggers)
        self.tree_display_view.setContextMenuPolicy(Qt.ContextMenuPolicy.CustomContextMenu)
        self.tree_display_view.setMinimumWidth(300)
        self.tree_display_view.setDragEnabled(False) # Disable drag and drop
        self.tree_display_view.setAcceptDrops(False) # Disable drag and drop
        self.tree_display_view.setDropIndicatorShown(False) # Disable drag and drop

        self.grid_display_view = QListView()
        self.grid_display_view.setModel(self.proxy_model)
        self.grid_display_view.setViewMode(QListView.ViewMode.IconMode)
        self.grid_display_view.setFlow(QListView.Flow.LeftToRight)
        self.grid_display_view.setWrapping(True)
        self.grid_display_view.setResizeMode(QListView.ResizeMode.Adjust)
        self.grid_display_view.setSelectionMode(QAbstractItemView.SelectionMode.ExtendedSelection)
        self.grid_display_view.setEditTriggers(QAbstractItemView.EditTrigger.NoEditTriggers)
        self.grid_display_view.setContextMenuPolicy(Qt.ContextMenuPolicy.CustomContextMenu)
        self.grid_display_view.setMinimumWidth(300)
        self.grid_display_view.setVisible(False)
        self.grid_display_view.setDragEnabled(True)

        self.center_pane_container = QWidget()
        self.center_pane_container.setObjectName("center_pane_container")
        center_pane_layout = QVBoxLayout(self.center_pane_container)
        center_pane_layout.setContentsMargins(0, 0, 0, 0) 
        center_pane_layout.setSpacing(0) 

        self.image_view = QLabel("Select an image to view") 
        self.image_view.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.image_view.setObjectName("image_view")
        self.image_view.setMinimumSize(400, 300) 
        center_pane_layout.addWidget(self.image_view, 1) 

        self.star_buttons = []
        self.rating_widget = QWidget() 
        self.rating_widget.setObjectName("rating_widget")
        rating_layout = QHBoxLayout(self.rating_widget)
        rating_layout.setContentsMargins(0,0,0,0) 
        rating_layout.setSpacing(2) 

        for i in range(1, 6):
            btn = QPushButton("☆")
            btn.setProperty("ratingValue", i)
            font = btn.font()
            font.setPointSize(14) 
            btn.setFont(font)
            rating_layout.addWidget(btn)
            self.star_buttons.append(btn)

        self.clear_rating_button = QPushButton("X")
        self.clear_rating_button.setToolTip("Clear rating (0 stars)")
        rating_layout.addWidget(self.clear_rating_button)

        self.color_buttons = {}
        self.color_widget = QWidget() 
        self.color_widget.setObjectName("color_widget")
        color_layout = QHBoxLayout(self.color_widget)
        color_layout.setContentsMargins(0,0,0,0) 
        color_layout.setSpacing(3) 
        colors = ["Red", "Yellow", "Green", "Blue", "Purple"]
        color_map = {"Red": "#C92C2C", "Yellow": "#E1C340", "Green": "#3F9142", "Blue": "#3478BC", "Purple": "#8E44AD"}

        for color_name in colors:
            btn = QPushButton("")
            btn.setToolTip(f"Set label to {color_name}")
            btn.setProperty("labelValue", color_name)
            hex_color = color_map.get(color_name, "#FFFFFF")
            btn.setProperty("originalHexColor", hex_color)
            btn.setStyleSheet(f"QPushButton {{ background-color: {hex_color}; }}") 
            color_layout.addWidget(btn)
            self.color_buttons[color_name] = btn

        self.clear_color_label_button = QPushButton("X")
        self.clear_color_label_button.setToolTip("Clear color label")
        color_layout.addWidget(self.clear_color_label_button)

        self.image_action_bar = QWidget()
        self.image_action_bar.setObjectName("image_action_bar")
        image_action_bar_layout = QHBoxLayout(self.image_action_bar)
        image_action_bar_layout.setContentsMargins(10, 8, 10, 8) 
        image_action_bar_layout.setSpacing(15) 

        image_action_bar_layout.addStretch(1) 
        image_action_bar_layout.addWidget(self.rating_widget)
        image_action_bar_layout.addWidget(self.color_widget)
        image_action_bar_layout.addStretch(1) 

        center_pane_layout.addWidget(self.image_action_bar) 

        self.bottom_bar = QWidget()
        self.bottom_bar.setObjectName("bottom_bar")
        bottom_layout = QHBoxLayout(self.bottom_bar)
        bottom_layout.setContentsMargins(8, 5, 8, 5) 
        bottom_layout.setSpacing(10) 

        self.nav_widget = QWidget() 
        self.nav_widget.setObjectName("nav_widget")
        nav_layout = QHBoxLayout(self.nav_widget)
        nav_layout.setContentsMargins(0,0,0,0)
        nav_layout.setSpacing(5)

        self.prev_button = QPushButton()
        prev_icon = self.style().standardIcon(QStyle.StandardPixmap.SP_ArrowBack)
        self.prev_button.setIcon(prev_icon)
        self.prev_button.setToolTip("Previous Image (Left Arrow)")
        nav_layout.addWidget(self.prev_button)

        self.next_button = QPushButton()
        next_icon = self.style().standardIcon(QStyle.StandardPixmap.SP_ArrowForward)
        self.next_button.setIcon(next_icon)
        self.next_button.setToolTip("Next Image (Right Arrow)")
        nav_layout.addWidget(self.next_button)
        bottom_layout.addWidget(self.nav_widget) 

        bottom_layout.addStretch(1) 

        self.filter_widget = QWidget() 
        self.filter_widget.setObjectName("filter_widget")
        filter_layout = QHBoxLayout(self.filter_widget)
        filter_layout.setContentsMargins(0,0,0,0)
        filter_layout.setSpacing(5) 
        filter_layout.addWidget(QLabel("Filter:"))
        self.filter_combo = QComboBox()
        self.filter_combo.addItems([
            "Show All", "Unrated (0)", "1 Star +", "2 Stars +",
            "3 Stars +", "4 Stars +", "5 Stars"
        ])
        filter_layout.addWidget(self.filter_combo)
        
        self.cluster_filter_combo = QComboBox()
        self.cluster_filter_combo.addItems(["All Clusters"])
        self.cluster_filter_combo.setEnabled(False)
        self.cluster_filter_combo.setToolTip("Filter images by similarity cluster")
        filter_layout.addWidget(QLabel(" Cluster:"))
        filter_layout.addWidget(self.cluster_filter_combo)
        bottom_layout.addWidget(self.filter_widget) 

        self.cluster_sort_label = QLabel("Sort Clusters By:")
        self.cluster_sort_combo = QComboBox()
        self.cluster_sort_combo.addItems(["Default", "Time", "Similarity then Time"])
        self.cluster_sort_combo.setEnabled(False)
        self.cluster_sort_combo.setToolTip("Order of clusters when 'Group by Similarity' is active")
        bottom_layout.addWidget(self.cluster_sort_label)
        bottom_layout.addWidget(self.cluster_sort_combo)
        self.cluster_sort_label.setVisible(False)
        self.cluster_sort_combo.setVisible(False)
        
        bottom_layout.addStretch(1) 

        self.view_mode_widget = QWidget() 
        self.view_mode_widget.setObjectName("view_mode_widget")
        view_mode_layout = QHBoxLayout(self.view_mode_widget)
        view_mode_layout.setContentsMargins(0,0,0,0)
        view_mode_layout.setSpacing(5)
        view_mode_layout.addWidget(QLabel("View:"))
        self.view_list_button = QPushButton("List")
        self.view_icons_button = QPushButton("Icons")
        self.view_grid_button = QPushButton("Grid")
        self.view_date_button = QPushButton("Date")
        view_mode_layout.addWidget(self.view_list_button)
        view_mode_layout.addWidget(self.view_icons_button)
        view_mode_layout.addWidget(self.view_grid_button)
        view_mode_layout.addWidget(self.view_date_button)
        bottom_layout.addWidget(self.view_mode_widget) 

        bottom_layout.addStretch(2) 

        self.search_widget = QWidget() 
        self.search_widget.setObjectName("search_widget")
        search_layout = QHBoxLayout(self.search_widget)
        search_layout.setContentsMargins(0,0,0,0)
        search_layout.setSpacing(5)
        search_layout.addWidget(QLabel("Search:"))
        self.search_input = QLineEdit()
        self.search_input.setPlaceholderText("Filename...")
        self.search_input.setFixedWidth(180) 
        search_layout.addWidget(self.search_input)
        bottom_layout.addWidget(self.search_widget)

        self.image_info_label = QLabel() # Initial text set by _update_image_info_label in __init__
        self.image_info_label.setObjectName("imageInfoLabel")
        self.image_info_label.setAlignment(Qt.AlignmentFlag.AlignRight)
        bottom_layout.addWidget(self.image_info_label, 0, Qt.AlignmentFlag.AlignRight)

        self.statusBar().showMessage("Ready")
        logging.debug(f"MainWindow._create_widgets - End: {time.perf_counter() - start_time:.4f}s")

    def _create_layout(self):
        """Set up the main window layout."""
        start_time = time.perf_counter()
        logging.debug("MainWindow._create_layout - Start")
        central_widget = QWidget()
        main_layout = QVBoxLayout(central_widget)
        main_layout.setContentsMargins(0, 0, 0, 0)
        main_layout.setSpacing(0)

        main_splitter = QSplitter(Qt.Orientation.Horizontal)
        main_splitter.setObjectName("main_splitter") 

        self.left_pane_widget = QWidget()
        self.left_pane_widget.setObjectName("left_pane_widget")
        left_pane_layout = QVBoxLayout(self.left_pane_widget)
        left_pane_layout.setContentsMargins(0,0,0,0)
        left_pane_layout.setSpacing(0)
        left_pane_layout.addWidget(self.tree_display_view)
        left_pane_layout.addWidget(self.grid_display_view) 
        main_splitter.addWidget(self.left_pane_widget)

        main_splitter.addWidget(self.center_pane_container) 

        main_splitter.setStretchFactor(0, 1) 
        main_splitter.setStretchFactor(1, 3) 
        main_splitter.setSizes([350, 850]) 

        main_layout.addWidget(main_splitter)
        main_layout.addWidget(self.bottom_bar)

        self.setCentralWidget(central_widget)
        logging.debug(f"MainWindow._create_layout - End: {time.perf_counter() - start_time:.4f}s")

    def _connect_signals(self):
        start_time = time.perf_counter()
        logging.debug("MainWindow._connect_signals - Start")
        self.open_folder_action.triggered.connect(self._open_folder_dialog)
        
        # Install event filters on views
        self.tree_display_view.installEventFilter(self)
        self.grid_display_view.installEventFilter(self)

        self.tree_display_view.selectionModel().selectionChanged.connect(self._handle_file_selection_changed)
        self.grid_display_view.selectionModel().selectionChanged.connect(self._handle_file_selection_changed)
        for btn in self.star_buttons:
            btn.clicked.connect(self._set_rating_from_button)
        self.clear_rating_button.clicked.connect(self._clear_rating)
        for btn in self.color_buttons.values():
            btn.clicked.connect(self._set_label_from_button)
        self.clear_color_label_button.clicked.connect(self._clear_label)
        self.prev_button.clicked.connect(self._navigate_up_sequential) # Renamed
        self.next_button.clicked.connect(self._navigate_down_sequential) # Renamed
        self.filter_combo.currentIndexChanged.connect(self._apply_filter)
        self.cluster_filter_combo.currentIndexChanged.connect(self._apply_filter)
        self.cluster_sort_combo.currentIndexChanged.connect(self._cluster_sort_changed)
        self.search_input.textChanged.connect(self._apply_filter)
        self._connect_rating_actions()
        self.tree_display_view.collapsed.connect(self._handle_item_collapsed)
        self.view_list_button.clicked.connect(self._set_view_mode_list)
        self.view_icons_button.clicked.connect(self._set_view_mode_icons)
        self.view_grid_button.clicked.connect(self._set_view_mode_grid)
        self.view_date_button.clicked.connect(self._set_view_mode_date)
        self.toggle_folder_view_action.toggled.connect(self._toggle_folder_visibility)
        self.group_by_similarity_action.toggled.connect(self._toggle_group_by_similarity)
        self.find_action.triggered.connect(self._focus_search_input)
        self.analyze_similarity_action.triggered.connect(self._start_similarity_analysis)
        self.detect_blur_action.triggered.connect(self._start_blur_detection_analysis)
        self.toggle_auto_edits_action.toggled.connect(self._handle_toggle_auto_edits)
        self.set_exiftool_path_action.triggered.connect(self._show_set_exiftool_path_dialog) # Connect new action

        # Connect signals from WorkerManager
        self.worker_manager.file_scan_found_files.connect(self._handle_files_found)
        self.worker_manager.file_scan_thumbnail_preload_finished.connect(self._handle_thumbnail_preload_finished)
        self.worker_manager.file_scan_finished.connect(self._handle_scan_finished)
        self.worker_manager.file_scan_error.connect(self._handle_scan_error)

        self.worker_manager.similarity_progress.connect(self._handle_similarity_progress)
        self.worker_manager.similarity_embeddings_generated.connect(self._handle_embeddings_generated)
        self.worker_manager.similarity_clustering_complete.connect(self._handle_clustering_complete)
        self.worker_manager.similarity_error.connect(self._handle_similarity_error)

        self.worker_manager.preview_preload_progress.connect(self._handle_preview_progress)
        self.worker_manager.preview_preload_finished.connect(self._handle_preview_finished)
        self.worker_manager.preview_preload_error.connect(self._handle_preview_error)

        self.worker_manager.blur_detection_progress.connect(self._handle_blur_detection_progress)
        self.worker_manager.blur_detection_status_updated.connect(self._handle_blur_status_updated)
        self.worker_manager.blur_detection_finished.connect(self._handle_blur_detection_finished)
        self.worker_manager.blur_detection_error.connect(self._handle_blur_detection_error)

        # Connect signals from WorkerManager for RatingLoader
        self.worker_manager.rating_load_progress.connect(self._handle_rating_load_progress)
        self.worker_manager.rating_load_metadata_batch_loaded.connect(self._handle_metadata_batch_loaded) # New batched signal
        self.worker_manager.rating_load_finished.connect(self._handle_rating_load_finished)
        self.worker_manager.rating_load_error.connect(self._handle_rating_load_error)
        logging.debug(f"MainWindow._connect_signals - End: {time.perf_counter() - start_time:.4f}s")
    def _create_actions(self):
        start_time = time.perf_counter()
        logging.debug("MainWindow._create_actions - Start")
        self.rating_actions = {}
        key_map = {
            0: Qt.Key.Key_0, 1: Qt.Key.Key_1, 2: Qt.Key.Key_2,
            3: Qt.Key.Key_3, 4: Qt.Key.Key_4, 5: Qt.Key.Key_5
        }
        for rating_value in range(6):
            action = QAction(self)
            # Add ControlModifier for rating shortcuts
            action.setShortcut(QKeySequence(Qt.KeyboardModifier.ControlModifier | key_map[rating_value]))
            action.setData(rating_value)
            action.setShortcutContext(Qt.ShortcutContext.ApplicationShortcut)
            self.addAction(action)
            self.rating_actions[rating_value] = action
        self.find_action = QAction("Find", self)
        self.find_action.setShortcut(QKeySequence.StandardKey.Find)
        self.find_action.setShortcutContext(Qt.ShortcutContext.ApplicationShortcut)
        self.addAction(self.find_action)
        logging.debug(f"MainWindow._create_actions - End: {time.perf_counter() - start_time:.4f}s")

    def _connect_rating_actions(self):
        for rating_value, action in self.rating_actions.items():
            action.triggered.connect(self._apply_rating_from_action)

    def _apply_rating_from_action(self):
        sender_action = self.sender()
        if isinstance(sender_action, QAction):
            rating = sender_action.data()
            if rating is not None:
                self._apply_rating(rating)

    def _open_folder_dialog(self):
        folder_path = QFileDialog.getExistingDirectory(
            self, "Select Folder", "",
            QFileDialog.Option.ShowDirsOnly | QFileDialog.Option.DontResolveSymlinks
        )
        if folder_path:
            self._load_folder(folder_path)
        else:
            self.statusBar().showMessage("Folder selection cancelled.")

    def _calculate_folder_image_size(self, folder_path: str) -> int:
        """Calculates the total size of supported image files in a folder (recursive)."""
        total_size_bytes = 0
        try:
            for root, _, files in os.walk(folder_path):
                for filename in files:
                    ext = os.path.splitext(filename)[1].lower()
                    if ext in SUPPORTED_EXTENSIONS:
                        try:
                            full_path = os.path.join(root, filename)
                            total_size_bytes += os.path.getsize(full_path)
                        except OSError:
                            pass # Ignore files that can't be accessed or no longer exist
        except Exception as e:
            logging.error(f"Error calculating folder image size for {folder_path}: {e}")
        return total_size_bytes

    def _load_folder(self, folder_path):
        load_folder_start_time = time.perf_counter()
        logging.info(f"MainWindow._load_folder - Start for: {folder_path}")
        self.show_loading_overlay(f"Preparing to scan folder...")

        # Check cache size vs folder size
        estimated_folder_image_size_bytes = self._calculate_folder_image_size(folder_path)
        preview_cache_limit_bytes = get_preview_cache_size_bytes()

        PREVIEW_ESTIMATED_SIZE_FACTOR = 0.20 # Estimate previews take 20% of original image size
        estimated_preview_data_needed_for_folder_bytes = int(estimated_folder_image_size_bytes * PREVIEW_ESTIMATED_SIZE_FACTOR)

        if preview_cache_limit_bytes > 0 and \
           estimated_preview_data_needed_for_folder_bytes > preview_cache_limit_bytes:
            warning_msg = (
                f"The images in the selected folder are estimated to require approximately "
                f"{estimated_preview_data_needed_for_folder_bytes / (1024*1024):.2f} MB for their previews. "
                f"Your current preview cache limit is "
                f"{preview_cache_limit_bytes / (1024*1024*1024):.2f} GB.\n\n"
                "This might exceed your cache capacity, potentially leading to frequent cache evictions "
                "and slower performance as previews are regenerated.\n\n"
                "Consider increasing the 'Preview Image Cache' size in "
                "Settings > Manage Cache for a smoother experience, or select a smaller folder."
            )
            QMessageBox.warning(self, "Potential Cache Overflow", warning_msg)
        
        section_start_time = time.perf_counter()
        self.worker_manager.stop_all_workers() # Use WorkerManager to stop all
        logging.info(f"MainWindow._load_folder - stop_all_workers done: {time.perf_counter() - section_start_time:.4f}s")
 
        section_start_time = time.perf_counter()
        self.app_state.clear_all_file_specific_data()
        self.app_state.current_folder_path = folder_path
        folder_display_name = os.path.basename(folder_path) if folder_path else "Selected Folder"
        self._update_image_info_label(status_message_override=f"Folder: {folder_display_name} | Preparing scan...")
        logging.info(f"MainWindow._load_folder - clear_all_file_specific_data & set path done: {time.perf_counter() - section_start_time:.4f}s")
        
        section_start_time = time.perf_counter()
        self.cluster_filter_combo.clear()
        self.cluster_filter_combo.addItems(["All Clusters"])
        self.cluster_filter_combo.setEnabled(False)
        self.cluster_sort_label.setVisible(False)
        self.cluster_sort_combo.setEnabled(False)
        self.cluster_sort_combo.setVisible(False)
        self.cluster_sort_combo.setCurrentIndex(0)
        self.group_by_similarity_action.setEnabled(False)
        self.group_by_similarity_action.setChecked(False)
        logging.info(f"MainWindow._load_folder - UI reset (cluster, group by) done: {time.perf_counter() - section_start_time:.4f}s")

        section_start_time = time.perf_counter()
        self.file_system_model.clear()
        self.file_system_model.setColumnCount(1)
        logging.info(f"MainWindow._load_folder - file_system_model cleared: {time.perf_counter() - section_start_time:.4f}s")
        
        self.update_loading_text(f"Scanning folder: {os.path.basename(folder_path)}...")
        self.open_folder_action.setEnabled(False)
        self.analyze_similarity_action.setEnabled(False)
        self.detect_blur_action.setEnabled(False)
        
        # Delegate to WorkerManager
        logging.info(f"MainWindow._load_folder - Preparing to call start_file_scan. Total time before call: {time.perf_counter() - load_folder_start_time:.4f}s")
        self.worker_manager.start_file_scan(
            folder_path,
            apply_auto_edits=self.apply_auto_edits_enabled,
            perform_blur_detection=False, # Initial scan doesn't do blur
            blur_threshold=self.blur_detection_threshold
        )
        logging.info(f"MainWindow._load_folder - start_file_scan called. Total time for _load_folder (sync part): {time.perf_counter() - load_folder_start_time:.4f}s")
    # _stop_scanner, _reset_scanner_state are now implicitly handled by WorkerManager
    # and MainWindow's reaction to WorkerManager's signals (e.g., file_scan_finished)

    # Slot for WorkerManager's file_scan_found_files signal
    def _handle_files_found(self, batch_of_file_data: List[Dict[str, any]]):
        self.app_state.image_files_data.extend(batch_of_file_data)
        self.update_loading_text(f"Scanning... {len(self.app_state.image_files_data)} images found")
        self._update_image_info_label() # Update bottom bar info


    # Slot for WorkerManager's file_scan_finished signal
    def _handle_scan_finished(self):
        self.update_loading_text("Scan finished. Populating view and starting background loads...")
        # Enable actions now that scan is complete
        self.open_folder_action.setEnabled(True)
        self.analyze_similarity_action.setEnabled(bool(self.app_state.image_files_data))
        self.detect_blur_action.setEnabled(bool(self.app_state.image_files_data))
        
        self._rebuild_model_view() # Populate view with basic file info first
        
        # Start background loading of ratings and then previews
        if self.app_state.image_files_data:
            self.update_loading_text("Loading Exiftool data...")
            self.worker_manager.start_rating_load(
                self.app_state.image_files_data.copy(), # Pass a copy of the list
                self.app_state.rating_disk_cache,
                self.app_state
            )
            # Preview preloading will be chained after rating loading finishes
        else:
            self.hide_loading_overlay() # No data to load further
        
        self._update_image_info_label() # Update with final counts
        # WorkerManager handles file_scanner thread cleanup
 
    # Slot for WorkerManager's file_scan_error signal
    def _handle_scan_error(self, message):
        self.statusBar().showMessage(f"Scan Error: {message}")
        self.open_folder_action.setEnabled(True) # Re-enable in case of error
        
        error_folder_display = "N/A"
        if self.app_state.current_folder_path:
            error_folder_display = os.path.basename(self.app_state.current_folder_path)
            if not error_folder_display: error_folder_display = self.app_state.current_folder_path
        self._update_image_info_label(status_message_override=f"Folder: {error_folder_display} | Scan error.")
        
        self.hide_loading_overlay()
        # WorkerManager handles thread cleanup
 
    def _rebuild_model_view(self):
        self.update_loading_text("Rebuilding view...")
        QApplication.processEvents()
        self.file_system_model.clear()
        root_item = self.file_system_model.invisibleRootItem()
        active_view = self._get_active_file_view()

        if not self.app_state.image_files_data:
            self.statusBar().showMessage("No images loaded.", 3000)
            return

        if self.group_by_similarity_mode:
            if not self.app_state.cluster_results:
                no_cluster_item = QStandardItem("Run 'Analyze Similarity' to group.")
                no_cluster_item.setEditable(False); root_item.appendRow(no_cluster_item)
                return

            images_by_cluster = self._group_images_by_cluster()
            if not images_by_cluster:
                 no_images_in_clusters = QStandardItem("No images assigned to clusters."); no_images_in_clusters.setEditable(False); root_item.appendRow(no_images_in_clusters)
                 return

            sorted_cluster_ids = list(images_by_cluster.keys())
            current_sort_method = self.cluster_sort_combo.currentText()
            if current_sort_method == "Time":
                cluster_timestamps = self._get_cluster_timestamps(images_by_cluster, self.app_state.date_cache) 
                sorted_cluster_ids.sort(key=lambda cid: cluster_timestamps.get(cid, date_obj.max))
            elif current_sort_method == "Similarity then Time":
                if not self.app_state.embeddings_cache: 
                    cluster_timestamps = self._get_cluster_timestamps(images_by_cluster, self.app_state.date_cache) 
                    sorted_cluster_ids.sort(key=lambda cid: cluster_timestamps.get(cid, date_obj.max))
                else:
                    sorted_cluster_ids = self._sort_clusters_by_similarity_time( 
                        images_by_cluster, self.app_state.embeddings_cache, self.app_state.date_cache
                    )
            else: # Default sort
                sorted_cluster_ids.sort()

            total_clustered_images = 0
            for cluster_id in sorted_cluster_ids:
                cluster_item = QStandardItem(f"Group {cluster_id}")
                cluster_item.setEditable(False); cluster_item.setData(f"cluster_header_{cluster_id}", Qt.ItemDataRole.UserRole)
                cluster_item.setForeground(QColor(Qt.GlobalColor.gray))
                root_item.appendRow(cluster_item)
                files_in_cluster = images_by_cluster[cluster_id] 
                total_clustered_images += len(files_in_cluster)
                if self.current_view_mode == "date":
                    self._populate_model_by_date(cluster_item, files_in_cluster) 
                else:
                    self._populate_model_standard(cluster_item, files_in_cluster) 
            self.statusBar().showMessage(f"Grouped {total_clustered_images} images into {len(sorted_cluster_ids)} clusters.", 3000)
        else: # Not grouping by similarity
            if self.current_view_mode == "date":
                self._populate_model_by_date(root_item, self.app_state.image_files_data)
            else:
                self._populate_model_standard(root_item, self.app_state.image_files_data)
            self.statusBar().showMessage(f"View populated with {len(self.app_state.image_files_data)} images.", 3000)

        self._apply_filter() 
        if self.group_by_similarity_mode and isinstance(active_view, QTreeView):
            proxy_root = QModelIndex()
            for i in range(self.proxy_model.rowCount(proxy_root)):
                proxy_cluster_index = self.proxy_model.index(i, 0, proxy_root)
                if proxy_cluster_index.isValid():
                    source_cluster_index = self.proxy_model.mapToSource(proxy_cluster_index)
                    item = self.file_system_model.itemFromIndex(source_cluster_index) 
                    if item:
                        item_user_data = item.data(Qt.ItemDataRole.UserRole)
                        if isinstance(item_user_data, str) and item_user_data.startswith("cluster_header_"):
                            if not active_view.isRowHidden(proxy_cluster_index.row(), proxy_cluster_index.parent()):
                                active_view.expand(proxy_cluster_index)
 
        if active_view:
            active_view.updateGeometries()
            active_view.viewport().update()
            first_index = self._find_first_visible_item()
            if first_index.isValid():
                active_view.setCurrentIndex(first_index)
                active_view.scrollTo(first_index, QAbstractItemView.ScrollHint.EnsureVisible)
                current_parent = first_index.parent()
                expand_list = []
                while current_parent.isValid() and current_parent != QModelIndex():
                    expand_list.append(current_parent)
                    current_parent = current_parent.parent()
                if isinstance(active_view, QTreeView):
                    for idx_to_expand in reversed(expand_list):
                        active_view.expand(idx_to_expand)
            else: # No items visible after filter
                 self.image_view.clear(); self.image_view.setText("No items match filter")
                 self._update_rating_display(0); self._update_label_display(None)
                 self.statusBar().showMessage("No items match current filter.")

    def _group_images_by_cluster(self) -> Dict[int, List[Dict[str, any]]]:
        images_by_cluster: Dict[int, List[Dict[str, any]]] = {}
        image_data_map = {img_data['path']: img_data for img_data in self.app_state.image_files_data}

        for file_path, cluster_id in self.app_state.cluster_results.items():
            if file_path in image_data_map:
                if cluster_id not in images_by_cluster:
                    images_by_cluster[cluster_id] = []
                images_by_cluster[cluster_id].append(image_data_map[file_path])
        return images_by_cluster

    def _populate_model_standard(self, parent_item: QStandardItem, image_data_list: List[Dict[str, any]]):
        if not image_data_list: return

        if self.show_folders_mode and not self.group_by_similarity_mode:
            files_by_folder: Dict[str, List[Dict[str, any]]] = {}
            for file_data in image_data_list:
                f_path = file_data['path']
                folder = os.path.dirname(f_path)
                if folder not in files_by_folder: files_by_folder[folder] = []
                files_by_folder[folder].append(file_data)
            
            for folder_path in sorted(files_by_folder.keys()):
                folder_name = os.path.basename(folder_path) if folder_path else "Root"
                folder_item = QStandardItem(folder_name)
                folder_item.setEditable(False); folder_item.setData(folder_path, Qt.ItemDataRole.UserRole) 
                folder_item.setIcon(self.style().standardIcon(QStyle.StandardPixmap.SP_DirIcon))
                parent_item.appendRow(folder_item)
                for file_data in sorted(files_by_folder[folder_path], key=lambda fd: os.path.basename(fd['path'])):
                    image_item = self._create_standard_item(file_data) 
                    folder_item.appendRow(image_item)
        else: # Not showing folders, or grouping by similarity (which creates its own top-level groups)
            image_sort_key_func = lambda fd: os.path.basename(fd['path'])
            parent_data = parent_item.data(Qt.ItemDataRole.UserRole)
            is_cluster_header = isinstance(parent_data, str) and parent_data.startswith("cluster_header_")

            if self.group_by_similarity_mode and is_cluster_header:
                current_cluster_sort_method = self.cluster_sort_combo.currentText()
                if current_cluster_sort_method == "Time" or current_cluster_sort_method == "Similarity then Time":
                    image_sort_key_func = lambda fd: (self.app_state.date_cache.get(fd['path'], date_obj.max), os.path.basename(fd['path']))
            
            for file_data in sorted(image_data_list, key=image_sort_key_func):
                 image_item = self._create_standard_item(file_data) 
                 parent_item.appendRow(image_item)

    def _set_rating_from_button(self):
        sender_button = self.sender()
        if sender_button:
            rating = sender_button.property("ratingValue")
            if rating is not None:
                self._apply_rating(rating)
    
    def _clear_rating(self):
        self._apply_rating(0)

    def _apply_rating(self, rating: int):
        active_view = self._get_active_file_view()
        if not active_view: return
        current_index = active_view.currentIndex()
        if not current_index.isValid(): return
        source_index = self.proxy_model.mapToSource(current_index)
        if not source_index.isValid(): return
        item = self.file_system_model.itemFromIndex(source_index) 
        if not item: return
        
        item_data = item.data(Qt.ItemDataRole.UserRole)
        if not isinstance(item_data, dict) or 'path' not in item_data: return 
        file_path = item_data['path']
        
        if not os.path.exists(file_path): return

        success = MetadataHandler.set_rating(file_path, rating, self.app_state.rating_disk_cache, self.app_state.exif_disk_cache)
        if success:
            self._update_rating_display(rating)
            self.app_state.rating_cache[file_path] = rating # Update in-memory cache as well
            self._apply_filter() # Re-apply filter in case rating affects visibility
        else:
            self.statusBar().showMessage(f"Failed to set rating for {os.path.basename(file_path)}", 5000)

    def _log_qmodelindex(self, index: QModelIndex, prefix: str = "") -> str:
        if not hasattr(self, 'proxy_model') or not hasattr(self, 'file_system_model'): # Models might not be initialized yet
            if not index.isValid():
                return f"{prefix} Invalid QModelIndex (models not ready)"
            return f"{prefix} QModelIndex(row={index.row()}, col={index.column()}, valid={index.isValid()}) (models not ready)"

        if not index.isValid():
            return f"{prefix} Invalid QModelIndex"
        
        source_index = self.proxy_model.mapToSource(index)
        item_text = "N/A (source invalid)"
        user_data_str = "N/A (source invalid)"

        if source_index.isValid():
            item = self.file_system_model.itemFromIndex(source_index)
            if item:
                item_text = item.text()
                user_data = item.data(Qt.ItemDataRole.UserRole)
                if isinstance(user_data, dict) and 'path' in user_data:
                    user_data_str = f"path: {os.path.basename(user_data['path'])}"
                elif isinstance(user_data, str): # For group headers etc.
                    user_data_str = f"str_data: '{user_data}'"
                elif user_data is None:
                    user_data_str = "None"
                else:
                    user_data_str = f"type: {type(user_data)}"
            else: # item is None
                item_text = "N/A (item is None)"
                user_data_str = "N/A (item is None)"
        
        return f"{prefix} QModelIndex(proxy_row={index.row()}, proxy_col={index.column()}, text='{item_text}', user_data='{user_data_str}', proxy_valid={index.isValid()}, source_valid={source_index.isValid()})"

    def _is_valid_image_item(self, proxy_index: QModelIndex) -> bool:
        # logging.debug(f"IS_VALID_IMAGE_ITEM: Checking {self._log_qmodelindex(proxy_index)}")
        if not proxy_index.isValid():
            # logging.debug(f"IS_VALID_IMAGE_ITEM: Proxy index invalid.")
            return False
        
        source_index = self.proxy_model.mapToSource(proxy_index)
        if not source_index.isValid():
            # logging.debug(f"IS_VALID_IMAGE_ITEM: Source index invalid for proxy {proxy_index.row()}.")
            return False
            
        item = self.file_system_model.itemFromIndex(source_index)
        if not item:
            # logging.debug(f"IS_VALID_IMAGE_ITEM: Item from source index is None.")
            return False
            
        item_user_data = item.data(Qt.ItemDataRole.UserRole)
        is_image = isinstance(item_user_data, dict) and \
                   'path' in item_user_data and \
                   os.path.isfile(item_user_data['path'])
        
        # logging.debug(f"IS_VALID_IMAGE_ITEM: Result for {os.path.basename(item_user_data.get('path', 'N/A')) if isinstance(item_user_data, dict) else 'NonDictUserData'}: {is_image}")
        return is_image

    def _get_active_file_view(self):
        # Simplified: Grid view is only active if NOT grouping by similarity
        if hasattr(self, 'current_view_mode') and self.current_view_mode == "grid" and not self.group_by_similarity_mode:
            return self.grid_display_view
        else: # list, icons, date views, or grid view when grouped by similarity, use tree_display_view
            return self.tree_display_view

    # resizeEvent needs to be defined before it's called by super() or other events
    def resizeEvent(self, event: QResizeEvent):
        super().resizeEvent(event)
        if hasattr(self, 'current_view_mode') and self.current_view_mode == "grid" and not self.group_by_similarity_mode:
            if hasattr(self, '_update_grid_view_layout'):
                self._update_grid_view_layout()
        
        if self.loading_overlay: 
            self.loading_overlay.update_position()

        # event.accept() # Not needed for resizeEvent in QMainWindow from my recall

    def keyPressEvent(self, event: QKeyEvent):
        # Arrow key and Delete navigation is now handled by the eventFilter for the views.
        # MainWindow.keyPressEvent will handle other application-wide shortcuts
        # or fallbacks if focus is not on the views.

        key = event.key()
        
        # Escape key to clear focus from search input (if it has focus)
        if key == Qt.Key.Key_Escape:
            if self.search_input.hasFocus():
                self.search_input.clearFocus()
                active_view = self._get_active_file_view()
                if (active_view): active_view.setFocus() # Return focus to the view
                event.accept(); return
        
        # Other global shortcuts for MainWindow could be here.
        # e.g. Ctrl+F is handled by QAction self.find_action

        super().keyPressEvent(event) # Pass to super for any other default handling

    def _focus_search_input(self):
        self.search_input.setFocus()
        self.search_input.selectAll()

    def _move_current_image_to_trash(self):
        active_view = self._get_active_file_view()
        if not active_view: return

        # --- Pre-deletion information gathering ---
        active_view = self._get_active_file_view() # Get active_view once
        if not active_view: return

        visible_paths_before_delete = self._get_all_visible_image_paths()
        logging.debug(f"MDIT: Visible paths before delete ({len(visible_paths_before_delete)}): {visible_paths_before_delete[:5]}...")
        
        # This uses selectionModel, which is fine before deletions alter the model structure too much.
        # It's important this is called BEFORE items are removed from the model.
        deleted_file_paths = self._get_selected_file_paths_from_view()
        
        if not deleted_file_paths: # Renamed from selected_file_paths for clarity post-selection
            self.statusBar().showMessage("No image(s) selected to delete.", 3000)
            return

        num_selected = len(deleted_file_paths) # Corrected variable name here
        
        dialog = QMessageBox(self)
        dialog.setWindowTitle("Confirm Delete")
        if num_selected == 1:
            file_name = os.path.basename(deleted_file_paths[0]) # Use deleted_file_paths here
            dialog.setText(f"Are you sure you want to move '{file_name}' to the trash?")
        else:
            dialog.setText(f"Are you sure you want to move {num_selected} images to the trash?")
        dialog.setIcon(QMessageBox.Icon.Warning)
        dialog.setStandardButtons(QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.No)
        dialog.setDefaultButton(QMessageBox.StandardButton.Yes) 

        yes_button = dialog.button(QMessageBox.StandardButton.Yes)
        if yes_button:
            yes_button.setObjectName("confirmDeleteYesButton")
            
        no_button = dialog.button(QMessageBox.StandardButton.No)
        if no_button:
            no_button.setObjectName("confirmDeleteNoButton")

        # Apply QSS for QMessageBox and its buttons
        dialog.setStyleSheet("""
            QMessageBox {
                background-color: #2B2B2B; 
                color: #D1D1D1;          
                font-family: "Segoe UI", system-ui, -apple-system, BlinkMacSystemFont, sans-serif;
                font-size: 9pt;
            }
            QMessageBox QLabel { 
                color: #D1D1D1;
                background-color: transparent; 
                padding-bottom: 10px; 
            }
            QMessageBox QPushButton { 
                background-color: #333333;
                color: #C0C0C0;
                border: 1px solid #404040;
                padding: 6px 15px; 
                border-radius: 4px;
                min-height: 28px; 
                min-width: 80px;  
            }
            QMessageBox QPushButton:hover {
                background-color: #3D3D3D;
                border-color: #505050;
                color: #FFFFFF;
            }
            QMessageBox QPushButton:pressed {
                background-color: #2A2A2A;
            }

            QMessageBox QPushButton#confirmDeleteYesButton {
                background-color: #C92C2C; 
                color: #FFFFFF;
                border: 1px solid #A02020;
                font-weight: bold;
            }
            QMessageBox QPushButton#confirmDeleteYesButton:hover {
                background-color: #E04040; 
                border-color: #B03030;
            }
            QMessageBox QPushButton#confirmDeleteYesButton:pressed {
                background-color: #B02020; 
            }

            QMessageBox QPushButton#confirmDeleteNoButton {
                background-color: #383838; 
                color: #D1D1D1;
                border: 1px solid #484848;
            }
            QMessageBox QPushButton#confirmDeleteNoButton:hover {
                background-color: #454545;
                border-color: #555555;
                color: #FFFFFF;
            }
            QMessageBox QPushButton#confirmDeleteNoButton:pressed {
                background-color: #303030;
            }
        """)

        reply = dialog.exec()

        if reply == QMessageBox.StandardButton.No:
            return

        # Store the proxy index of the initially focused item to try and select something near it later.
        # This is tricky with multiple selections across different parents, so we'll simplify.
        # We'll try to select the item that was *next* to the *first* item in the original selection,
        # or the first visible item if that fails.
        
        first_selected_proxy_idx = QModelIndex()
        active_selection = active_view.selectionModel().selectedIndexes()
        if active_selection:
            first_selected_proxy_idx = active_selection[0] # Assuming column 0 if multiple columns selected

        # We need to map proxy indices to source indices *before* deleting,
        # as proxy indices will become invalid once source items are removed.
        source_indices_to_delete = []
        for proxy_idx_to_delete in active_view.selectionModel().selectedIndexes():
            if proxy_idx_to_delete.column() == 0: # Process only one index per row
                source_idx = self.proxy_model.mapToSource(proxy_idx_to_delete)
                if source_idx.isValid():
                    source_indices_to_delete.append(source_idx)
        
        # Sort source indices in reverse order by row, then by parent.
        # This ensures that when we remove items from a parent, the row numbers
        # of subsequent items in that same parent are not affected.
        source_indices_to_delete.sort(key=lambda idx: (idx.parent().internalId(), idx.row()), reverse=True)

        deleted_count = 0
        affected_source_parent_items = [] # Store unique QStandardItem objects of parents

        for source_idx_to_delete in source_indices_to_delete:
            item_to_delete = self.file_system_model.itemFromIndex(source_idx_to_delete)
            if not item_to_delete: continue

            item_data = item_to_delete.data(Qt.ItemDataRole.UserRole)
            if not isinstance(item_data, dict) or 'path' not in item_data: continue
            
            file_path_to_delete = item_data['path']
            if not os.path.isfile(file_path_to_delete): continue
            
            file_name_to_delete = os.path.basename(file_path_to_delete)
            try:
                send2trash.send2trash(file_path_to_delete)
                self.app_state.remove_data_for_path(file_path_to_delete)

                source_parent_idx = source_idx_to_delete.parent()
                source_parent_item = self.file_system_model.itemFromIndex(source_parent_idx) \
                    if source_parent_idx.isValid() else self.file_system_model.invisibleRootItem()
                
                if source_parent_item:
                    source_parent_item.takeRow(source_idx_to_delete.row())
                    if source_parent_item not in affected_source_parent_items:
                        affected_source_parent_items.append(source_parent_item) # Add if unique
                deleted_count += 1
            except Exception as e:
                # Log the error to terminal as well for easier debugging
                logging.error(f"Error moving '{file_name_to_delete}' to trash: {e}", exc_info=True)
                QMessageBox.warning(self, "Delete Error", f"Error moving '{file_name_to_delete}' to trash: {e}")
                # Optionally, break or continue if one file fails
        
        if deleted_count > 0:
            # Check and remove empty group headers
            # Iterate over a list copy as we might modify the underlying structure
            parents_to_check_for_emptiness = list(affected_source_parent_items)
            logging.debug(f"MDIT: Checking {len(parents_to_check_for_emptiness)} parent items for emptiness after deletions.")

            for parent_item_candidate in parents_to_check_for_emptiness:
                if parent_item_candidate == self.file_system_model.invisibleRootItem(): # Skip root
                    continue
                if parent_item_candidate.model() is None: # Already removed (e.g. child of another removed empty group)
                    logging.debug(f"MDIT: Parent candidate '{parent_item_candidate.text()}' no longer in model, skipping.")
                    continue

                is_eligible_group_header = False
                parent_user_data = parent_item_candidate.data(Qt.ItemDataRole.UserRole)

                if isinstance(parent_user_data, str):
                    if parent_user_data.startswith("cluster_header_") or \
                       parent_user_data.startswith("date_header_"):
                        is_eligible_group_header = True
                    elif self.show_folders_mode and not self.group_by_similarity_mode and os.path.isdir(parent_user_data): # Folder item
                        is_eligible_group_header = True
                
                if is_eligible_group_header and parent_item_candidate.rowCount() == 0:
                    item_row = parent_item_candidate.row() # Get row before potential parent() call alters context
                    # parent() of a QStandardItem returns its QStandardItem parent, or None if it's a top-level item.
                    actual_parent_qstandarditem = parent_item_candidate.parent()

                    parent_to_operate_on = None
                    parent_display_name_for_log = ""

                    if actual_parent_qstandarditem is None: # It's a top-level item in the model
                        parent_to_operate_on = self.file_system_model.invisibleRootItem()
                        parent_display_name_for_log = "invisibleRootItem"
                    else: # It's a child of another QStandardItem
                        parent_to_operate_on = actual_parent_qstandarditem
                        parent_display_name_for_log = f"'{actual_parent_qstandarditem.text()}'"
                    
                    logging.debug(f"MDIT: Attempting to remove empty group header: '{parent_item_candidate.text()}' (item_row {item_row}) from parent {parent_display_name_for_log}")
                    
                    # Use takeRow on the QStandardItem that is the actual parent in the model hierarchy
                    removed_items_list = parent_to_operate_on.takeRow(item_row)
                    
                    if removed_items_list: # takeRow returns a list of QStandardItems removed
                        logging.debug(f"MDIT: Successfully removed '{parent_item_candidate.text()}'.")
                    else:
                        logging.warning(f"MDIT: takeRow failed to remove '{parent_item_candidate.text()}' from parent {parent_display_name_for_log} at row {item_row}.")
            
            self.statusBar().showMessage(f"{deleted_count} image(s) moved to trash.", 5000)
            active_view.selectionModel().clearSelection() # Clear old selection to avoid issues

            logging.debug(f"MDIT: --- New Post Deletion Selection Logic ---")
            visible_paths_after_delete = self._get_all_visible_image_paths()
            logging.debug(f"MDIT: Visible paths after delete ({len(visible_paths_after_delete)}): {visible_paths_after_delete[:5]}...")

            next_item_to_select_proxy_idx = QModelIndex()

            if not visible_paths_after_delete:
                logging.debug("MDIT: No visible image items left after deletion.")
                self.image_view.clear(); self.image_view.setText("No images")
                self._update_rating_display(0); self._update_label_display(None)
                self.statusBar().showMessage("No images left or visible.")
            else:
                # Determine the index in the *original* visible list of the first deleted item
                first_deleted_path_idx_in_visible_list = -1
                if visible_paths_before_delete and deleted_file_paths: # deleted_file_paths from earlier
                    try:
                        first_deleted_path_idx_in_visible_list = visible_paths_before_delete.index(deleted_file_paths[0])
                    except ValueError:
                        logging.warning(f"MDIT: First deleted path {deleted_file_paths[0]} not found in pre-delete visible list. Defaulting to 0.")
                        first_deleted_path_idx_in_visible_list = 0 # Fallback
                elif visible_paths_before_delete: # If deleted_file_paths was empty for some reason, but we had original paths
                    first_deleted_path_idx_in_visible_list = 0
                else: # No visible items before, or no deleted items tracked (should not happen if deleted_count > 0)
                    first_deleted_path_idx_in_visible_list = 0
                
                logging.debug(f"MDIT: Index of first deleted item in original visible list: {first_deleted_path_idx_in_visible_list}")

                # Target index in the new list: try to select item at same original visual position, or new last if original was last
                target_idx_in_new_list = min(first_deleted_path_idx_in_visible_list, len(visible_paths_after_delete) - 1)
                # Ensure target_idx is non-negative
                target_idx_in_new_list = max(0, target_idx_in_new_list)


                logging.debug(f"MDIT: Target index in new visible list: {target_idx_in_new_list}")

                if 0 <= target_idx_in_new_list < len(visible_paths_after_delete):
                    path_to_select = visible_paths_after_delete[target_idx_in_new_list]
                    logging.debug(f"MDIT: Path to select: {os.path.basename(path_to_select)}")
                    next_item_to_select_proxy_idx = self._find_proxy_index_for_path(path_to_select)
                    logging.debug(f"MDIT: Proxy index for path: {self._log_qmodelindex(next_item_to_select_proxy_idx)}")
                else:
                    logging.warning(f"MDIT: target_idx_in_new_list ({target_idx_in_new_list}) is out of bounds for visible_paths_after_delete (len {len(visible_paths_after_delete)}).")


            if next_item_to_select_proxy_idx.isValid() and self._is_valid_image_item(next_item_to_select_proxy_idx):
                logging.debug(f"MDIT: Selecting item: {self._log_qmodelindex(next_item_to_select_proxy_idx)}")
                active_view.setCurrentIndex(next_item_to_select_proxy_idx)
                active_view.selectionModel().select(next_item_to_select_proxy_idx, QItemSelectionModel.SelectionFlag.ClearAndSelect)
                active_view.scrollTo(next_item_to_select_proxy_idx, QAbstractItemView.ScrollHint.EnsureVisible)
                self._handle_file_selection_changed()
            else:
                # This case handles if visible_paths_after_delete was empty OR find_proxy_index_for_path failed
                logging.debug("MDIT: No valid image item to select after deletion based on new logic. Clearing UI.")
                self.image_view.clear(); self.image_view.setText("No images")
                self._update_rating_display(0); self._update_label_display(None)
                if not visible_paths_after_delete : self.statusBar().showMessage("No images left or visible.")
                else: self.statusBar().showMessage("Could not determine next item to select.")
            
            self._update_image_info_label()
        elif num_selected > 0 : # No items were actually deleted, but some were selected
             self.statusBar().showMessage("No valid image files were deleted from selection.", 3000)
 
    def _is_row_hidden_in_tree_if_applicable(self, active_view, proxy_idx: QModelIndex) -> bool:
        if isinstance(active_view, QTreeView):
            # Ensure proxy_idx is valid and has a valid parent for isRowHidden
            if proxy_idx.isValid() and proxy_idx.parent().isValid():
                 return active_view.isRowHidden(proxy_idx.row(), proxy_idx.parent())
            elif proxy_idx.isValid(): # Top-level item
                 return active_view.isRowHidden(proxy_idx.row(), QModelIndex()) # Parent is root
        return False

    def _is_expanded_group_header(self, active_view, proxy_idx: QModelIndex) -> bool:
        if not proxy_idx.isValid() or not isinstance(active_view, QTreeView):
            return False
        
        source_idx = self.proxy_model.mapToSource(proxy_idx)
        item = self.file_system_model.itemFromIndex(source_idx)
        if not item: return False

        user_data = item.data(Qt.ItemDataRole.UserRole)
        is_group = False
        if isinstance(user_data, str):
            if user_data.startswith("cluster_header_") or user_data.startswith("date_header_"):
                is_group = True
            elif self.show_folders_mode and not self.group_by_similarity_mode and os.path.isdir(user_data):
                 is_group = True
        
        return is_group and active_view.isExpanded(proxy_idx)

    def _find_last_visible_image_item_in_subtree(self, parent_proxy_idx: QModelIndex) -> QModelIndex:
        active_view = self._get_active_file_view()
        # Ensure active_view and its model are valid
        if not active_view or not active_view.model() or not isinstance(active_view.model(), QSortFilterProxyModel):
            return QModelIndex()
        
        proxy_model = active_view.model() # This should be self.proxy_model

        for i in range(proxy_model.rowCount(parent_proxy_idx) - 1, -1, -1): # Iterate children in reverse
            child_proxy_idx = proxy_model.index(i, 0, parent_proxy_idx)
            if not child_proxy_idx.isValid():
                continue

            # If this child is an expanded group (QTreeView only), recurse
            if isinstance(active_view, QTreeView) and self._is_expanded_group_header(active_view, child_proxy_idx):
                found_in_child_group = self._find_last_visible_image_item_in_subtree(child_proxy_idx)
                if found_in_child_group.isValid():
                    return found_in_child_group
            
            # If not an expanded group where an item was found, check if the child itself is a visible image
            if self._is_valid_image_item(child_proxy_idx) and \
               not self._is_row_hidden_in_tree_if_applicable(active_view, child_proxy_idx):
                return child_proxy_idx
                     
        return QModelIndex() # No visible image item found in this subtree

    def closeEvent(self, event):
        logging.info("MainWindow.closeEvent - Stopping all workers.")
        self.worker_manager.stop_all_workers() # Use WorkerManager to stop all
        event.accept()

    def _get_current_group_sibling_images(self, current_image_proxy_idx: QModelIndex) -> Tuple[Optional[QModelIndex], List[QModelIndex], int]:
        """
        Finds the parent group of the current image and all its visible sibling image items.
        Returns (parent_group_proxy_idx, list_of_sibling_image_proxy_indices, local_idx_of_current_image).
        If not in a group (top-level), parent_group_proxy_idx is root QModelIndex().
        """
        active_view = self._get_active_file_view()
        if not active_view or not current_image_proxy_idx.isValid():
            return QModelIndex(), [], -1

        proxy_model = active_view.model()
        if not isinstance(proxy_model, QSortFilterProxyModel): # Should be self.proxy_model
            return QModelIndex(), [], -1

        parent_proxy_idx = current_image_proxy_idx.parent() 

        sibling_image_items = []
        current_item_local_idx = -1

        for i in range(proxy_model.rowCount(parent_proxy_idx)):
            sibling_idx = proxy_model.index(i, 0, parent_proxy_idx)
            if not sibling_idx.isValid():
                continue
            
            if self._is_valid_image_item(sibling_idx) and \
               not self._is_row_hidden_in_tree_if_applicable(active_view, sibling_idx):
                sibling_image_items.append(sibling_idx)
                if sibling_idx == current_image_proxy_idx:
                    current_item_local_idx = len(sibling_image_items) - 1
        
        return parent_proxy_idx, sibling_image_items, current_item_local_idx

    def _navigate_left_in_group(self):
        active_view = self._get_active_file_view()
        if not active_view: return
        current_proxy_idx = active_view.currentIndex()
        if not current_proxy_idx.isValid() or not self._is_valid_image_item(current_proxy_idx):
            # If no valid image selected, try to select the first one in view
            first_item = self._find_first_visible_item()
            if first_item.isValid(): active_view.setCurrentIndex(first_item)
            return

        _parent_group_idx, group_images, local_idx = self._get_current_group_sibling_images(current_proxy_idx)

        if not group_images or local_idx == -1:
            logging.debug("NAV_LEFT_IN_GROUP: No group images or current item not found in its group.")
            return 

        new_local_idx = local_idx - 1
        if new_local_idx < 0: # Was first, wrap to last
            new_local_idx = len(group_images) - 1
        
        if 0 <= new_local_idx < len(group_images):
            new_selection_candidate = group_images[new_local_idx]
            active_view.setCurrentIndex(new_selection_candidate)
            active_view.scrollTo(new_selection_candidate, QAbstractItemView.ScrollHint.EnsureVisible)
            logging.debug(f"NAV_LEFT_IN_GROUP: Set current index to {self._log_qmodelindex(new_selection_candidate)}")
        else:
            logging.debug(f"NAV_LEFT_IN_GROUP: Failed to select new item. local_idx={local_idx}, new_local_idx={new_local_idx}, group_size={len(group_images)}")


    def _navigate_right_in_group(self):
        active_view = self._get_active_file_view()
        if not active_view: return
        current_proxy_idx = active_view.currentIndex()
        if not current_proxy_idx.isValid() or not self._is_valid_image_item(current_proxy_idx):
            first_item = self._find_first_visible_item()
            if first_item.isValid(): active_view.setCurrentIndex(first_item)
            return

        _parent_group_idx, group_images, local_idx = self._get_current_group_sibling_images(current_proxy_idx)

        if not group_images or local_idx == -1:
            logging.debug("NAV_RIGHT_IN_GROUP: No group images or current item not found in its group.")
            return

        new_local_idx = local_idx + 1
        if new_local_idx >= len(group_images): # Was last, wrap to first
            new_local_idx = 0
            
        if 0 <= new_local_idx < len(group_images): # Ensure index is still valid after wrap
            new_selection_candidate = group_images[new_local_idx]
            active_view.setCurrentIndex(new_selection_candidate)
            active_view.scrollTo(new_selection_candidate, QAbstractItemView.ScrollHint.EnsureVisible)
            logging.debug(f"NAV_RIGHT_IN_GROUP: Set current index to {self._log_qmodelindex(new_selection_candidate)}")
        else:
             logging.debug(f"NAV_RIGHT_IN_GROUP: Failed to select new item. local_idx={local_idx}, new_local_idx={new_local_idx}, group_size={len(group_images)}")

    def _navigate_up_sequential(self): # Renamed from _navigate_previous
        active_view = self._get_active_file_view()
        if not active_view:
            logging.debug("NAV_UP_SEQ: No active view.")
            return
        
        current_proxy_idx = active_view.currentIndex()
        logging.debug(f"NAV_UP_SEQ: Start. Current Index: {self._log_qmodelindex(current_proxy_idx, 'current_proxy_idx')}")

        if not current_proxy_idx.isValid():
            last_item_index = self._find_last_visible_item()
            if last_item_index.isValid():
                active_view.setCurrentIndex(last_item_index)
                active_view.scrollTo(last_item_index, QAbstractItemView.ScrollHint.EnsureVisible)
            return

        new_selection_candidate = QModelIndex()
        iter_idx = current_proxy_idx

        max_iterations = (self.proxy_model.rowCount(QModelIndex()) + sum(self.proxy_model.rowCount(self.proxy_model.index(r,0,QModelIndex())) for r in range(self.proxy_model.rowCount(QModelIndex())))) * 2
        if max_iterations == 0 and self.app_state and self.app_state.image_files_data: max_iterations = len(self.app_state.image_files_data) * 5
        if max_iterations == 0: max_iterations = 5000

        for iteration_count in range(max_iterations):
            prev_visual_idx = active_view.indexAbove(iter_idx)

            if not prev_visual_idx.isValid():
                logging.debug("NAV_UP_SEQ: Reached top of view (indexAbove returned invalid).")
                break

            if self._is_valid_image_item(prev_visual_idx) and \
               not self._is_row_hidden_in_tree_if_applicable(active_view, prev_visual_idx):
                new_selection_candidate = prev_visual_idx
                logging.debug(f"NAV_UP_SEQ: Found image item directly above: {self._log_qmodelindex(new_selection_candidate)}")
                break
            elif isinstance(active_view, QTreeView) and self._is_expanded_group_header(active_view, prev_visual_idx):
                last_in_group = self._find_last_visible_image_item_in_subtree(prev_visual_idx)
                if last_in_group.isValid():
                    new_selection_candidate = last_in_group
                    logging.debug(f"NAV_UP_SEQ: Found last image in group above: {self._log_qmodelindex(new_selection_candidate)}")
                    break
            
            iter_idx = prev_visual_idx
            if iteration_count == max_iterations -1:
                logging.warning("NAV_UP_SEQ: Max iterations reached during search.")

        if new_selection_candidate.isValid():
            active_view.setCurrentIndex(new_selection_candidate)
            active_view.scrollTo(new_selection_candidate, QAbstractItemView.ScrollHint.EnsureVisible)
            logging.debug(f"NAV_UP_SEQ: Set current index to {self._log_qmodelindex(new_selection_candidate)}")
        else:
            logging.debug("NAV_UP_SEQ: No valid previous image item found after search.")

    def _navigate_down_sequential(self): # Renamed from _navigate_next
        active_view = self._get_active_file_view()
        if not active_view:
            logging.debug("NAV_DOWN_SEQ: No active view.")
            return

        current_index = active_view.currentIndex()
        logging.debug(f"NAV_DOWN_SEQ: Start. Current Index: {self._log_qmodelindex(current_index, 'current_index')}")

        if not current_index.isValid():
            first_item_index = self._find_first_visible_item()
            logging.debug(f"NAV_DOWN_SEQ: Current index invalid, found first item: {self._log_qmodelindex(first_item_index, 'first_item_index')}")
            if first_item_index.isValid():
                active_view.setCurrentIndex(first_item_index)
                active_view.scrollTo(first_item_index, QAbstractItemView.ScrollHint.EnsureVisible)
            return
            
        next_item_index = QModelIndex()
        temp_index = current_index
        iteration_count = 0
        
        max_iterations = (self.proxy_model.rowCount(QModelIndex()) + sum(self.proxy_model.rowCount(self.proxy_model.index(r,0,QModelIndex())) for r in range(self.proxy_model.rowCount(QModelIndex())))) * 2
        if max_iterations == 0 and self.app_state.image_files_data: safety_iteration_limit = len(self.app_state.image_files_data) * 2
        if max_iterations == 0 : safety_iteration_limit = 5000
        else: safety_iteration_limit = max_iterations


        while temp_index.isValid() and iteration_count < safety_iteration_limit:
            iteration_count +=1
            temp_index = active_view.indexBelow(temp_index)
            if not temp_index.isValid():
                logging.debug("NAV_DOWN_SEQ: temp_index became invalid (end of list/parent).")
                break

            is_image_item = self._is_valid_image_item(temp_index)
            if is_image_item:
                is_hidden = False
                if isinstance(active_view, QTreeView):
                    is_hidden = active_view.isRowHidden(temp_index.row(), temp_index.parent())
                if not is_hidden:
                    next_item_index = temp_index
                    logging.debug(f"NAV_DOWN_SEQ: Found valid next image item: {self._log_qmodelindex(next_item_index, 'next_item_index')}")
                    break
        
        if iteration_count >= safety_iteration_limit:
            logging.warning(f"NAV_DOWN_SEQ: Hit safety iteration limit ({safety_iteration_limit}).")

        if next_item_index.isValid():
            active_view.setCurrentIndex(next_item_index)
            active_view.scrollTo(next_item_index, QAbstractItemView.ScrollHint.EnsureVisible)
            logging.debug(f"NAV_DOWN_SEQ: Set current index to {self._log_qmodelindex(next_item_index)}")
        else:
            logging.debug("NAV_DOWN_SEQ: No valid next image item found.")

    def _find_first_visible_item(self) -> QModelIndex:
        active_view = self._get_active_file_view()
        if not active_view: return QModelIndex()
        logging.debug("FIND_FIRST: Start")
        proxy_model = active_view.model()
        if not isinstance(proxy_model, QSortFilterProxyModel):
            logging.debug("FIND_FIRST: Model is not QSortFilterProxyModel.")
            return QModelIndex()
        
        root_proxy_index = QModelIndex()

        if isinstance(active_view, QTreeView):
            q = [proxy_model.index(r, 0, root_proxy_index) for r in range(proxy_model.rowCount(root_proxy_index))]
            # logging.debug(f"FIND_FIRST (Tree): Initial queue size: {len(q)}")
            head = 0
            while head < len(q):
                current_proxy_idx = q[head]; head += 1
                if not current_proxy_idx.isValid(): continue
                
                # logging.debug(f"FIND_FIRST (Tree): Dequeued {self._log_qmodelindex(current_proxy_idx)}")
                if not active_view.isRowHidden(current_proxy_idx.row(), current_proxy_idx.parent()):
                    if self._is_valid_image_item(current_proxy_idx):
                        logging.debug(f"FIND_FIRST (Tree): Found first: {self._log_qmodelindex(current_proxy_idx)}")
                        return current_proxy_idx
                    
                    # Check if source item exists before calling hasChildren / isExpanded
                    source_idx_for_children_check = proxy_model.mapToSource(current_proxy_idx)
                    item_for_children_check = None
                    if source_idx_for_children_check.isValid():
                        item_for_children_check = proxy_model.sourceModel().itemFromIndex(source_idx_for_children_check)

                    if item_for_children_check and proxy_model.hasChildren(current_proxy_idx) and active_view.isExpanded(current_proxy_idx):
                        # logging.debug(f"FIND_FIRST (Tree): Item {current_proxy_idx.row()} is expanded, adding children.")
                        for child_row in range(proxy_model.rowCount(current_proxy_idx)):
                             q.append(proxy_model.index(child_row, 0, current_proxy_idx))
                # else:
                    # logging.debug(f"FIND_FIRST (Tree): Item {current_proxy_idx.row()} is hidden.")
            logging.debug("FIND_FIRST (Tree): No visible image item found after BFS.")
            return QModelIndex()
        elif isinstance(active_view, QListView):
            for r in range(proxy_model.rowCount(root_proxy_index)):
                proxy_idx = proxy_model.index(r, 0, root_proxy_index)
                # logging.debug(f"FIND_FIRST (List): Checking row {r}: {self._log_qmodelindex(proxy_idx)}")
                if self._is_valid_image_item(proxy_idx):
                    logging.debug(f"FIND_FIRST (List): Found first at row {r}: {self._log_qmodelindex(proxy_idx)}")
                    return proxy_idx
            logging.debug("FIND_FIRST (List): No visible image item found.")
            return QModelIndex()
        
        logging.debug("FIND_FIRST: Unknown view type or scenario.")
        return QModelIndex()

    def _find_last_visible_item(self) -> QModelIndex:
        active_view = self._get_active_file_view()
        if not active_view: return QModelIndex()
        logging.debug("FIND_LAST: Start")
        proxy_model = active_view.model()
        if not isinstance(proxy_model, QSortFilterProxyModel):
            logging.debug("FIND_LAST: Model is not QSortFilterProxyModel.")
            return QModelIndex()
        
        root_proxy_index = QModelIndex()

        if isinstance(active_view, QTreeView):
            # DFS-like approach, exploring last children first
            # Stack stores (index_to_visit, has_been_expanded_and_children_queued)
            # This is a bit complex to do purely iteratively backwards for DFS.
            # Let's try a simpler reversed BFS-like approach on expanded items.
            
            # Iterate all items in display order and pick the last valid one.
            # This is less efficient but simpler to implement correctly than reverse DFS.
            # We can optimize if needed, but correctness first.
            
            last_found_valid_image = QModelIndex()
            
            # Queue for BFS-like traversal
            q = [proxy_model.index(r, 0, root_proxy_index) for r in range(proxy_model.rowCount(root_proxy_index))]
            head = 0
            while head < len(q):
                current_proxy_idx = q[head]; head += 1
                if not current_proxy_idx.isValid(): continue

                if not active_view.isRowHidden(current_proxy_idx.row(), current_proxy_idx.parent()):
                    if self._is_valid_image_item(current_proxy_idx):
                        last_found_valid_image = current_proxy_idx # Update if this one is valid
                    
                    source_idx_for_children_check = proxy_model.mapToSource(current_proxy_idx)
                    item_for_children_check = None
                    if source_idx_for_children_check.isValid():
                        item_for_children_check = proxy_model.sourceModel().itemFromIndex(source_idx_for_children_check)

                    if item_for_children_check and proxy_model.hasChildren(current_proxy_idx) and active_view.isExpanded(current_proxy_idx):
                        for child_row in range(proxy_model.rowCount(current_proxy_idx)):
                             q.append(proxy_model.index(child_row, 0, current_proxy_idx))
            logging.debug(f"FIND_LAST (Tree): Traversed all, last valid found: {self._log_qmodelindex(last_found_valid_image)}")
            return last_found_valid_image

        elif isinstance(active_view, QListView):
            for r in range(proxy_model.rowCount(root_proxy_index) - 1, -1, -1): # Iterate backwards
                proxy_idx = proxy_model.index(r, 0, root_proxy_index)
                # logging.debug(f"FIND_LAST (List): Checking row {r}: {self._log_qmodelindex(proxy_idx)}")
                if self._is_valid_image_item(proxy_idx):
                    logging.debug(f"FIND_LAST (List): Found last at row {r}: {self._log_qmodelindex(proxy_idx)}")
                    return proxy_idx
            logging.debug("FIND_LAST (List): No visible image item found.")
            return QModelIndex()
            
        logging.debug("FIND_LAST: Unknown view type or scenario.")
        return QModelIndex()

    def _get_all_visible_image_paths(self) -> List[str]:
        """Gets an ordered list of file paths for all currently visible image items."""
        paths = []
        active_view = self._get_active_file_view()
        if not active_view: return paths
        
        proxy_model = active_view.model()
        # Ensure model is a QSortFilterProxyModel, as it holds the filtered/sorted view
        if not isinstance(proxy_model, QSortFilterProxyModel):
            logging.warning("_get_all_visible_image_paths: Active view's model is not QSortFilterProxyModel.")
            return paths

        # Traversal logic needs to handle both QTreeView (hierarchical) and QListView (flat)
        # We build a queue of proxy indices to visit in display order.
        queue = []
        root_proxy_parent_idx = QModelIndex() # Parent for top-level items in the proxy model

        for r in range(proxy_model.rowCount(root_proxy_parent_idx)):
            queue.append(proxy_model.index(r, 0, root_proxy_parent_idx))

        head = 0
        while head < len(queue):
            current_proxy_idx = queue[head]; head += 1
            if not current_proxy_idx.isValid(): continue

            # Check if the item itself is a valid image item
            if self._is_valid_image_item(current_proxy_idx):
                source_idx = proxy_model.mapToSource(current_proxy_idx)
                item = self.file_system_model.itemFromIndex(source_idx) # Use source_model here
                if item:
                    item_data = item.data(Qt.ItemDataRole.UserRole)
                    if isinstance(item_data, dict) and 'path' in item_data:
                        paths.append(item_data['path'])
            
            # If it's a QTreeView and the current item is expanded and has children, add them to the queue
            if isinstance(active_view, QTreeView):
                # We need to check against the source item for hasChildren, but expansion against proxy index
                source_idx_for_children_check = proxy_model.mapToSource(current_proxy_idx)
                # Ensure source_idx is valid before using it with source model
                if source_idx_for_children_check.isValid():
                    item_for_children_check = self.file_system_model.itemFromIndex(source_idx_for_children_check)
                    if item_for_children_check and item_for_children_check.hasChildren() and active_view.isExpanded(current_proxy_idx):
                        for child_row in range(proxy_model.rowCount(current_proxy_idx)): # Children from proxy model
                            queue.append(proxy_model.index(child_row, 0, current_proxy_idx))
        return paths

    def _find_proxy_index_for_path(self, target_path: str) -> QModelIndex:
        """Finds the QModelIndex in the current proxy model for a given file path."""
        active_view = self._get_active_file_view()
        if not active_view: return QModelIndex()
        
        proxy_model = active_view.model()
        if not isinstance(proxy_model, QSortFilterProxyModel):
            logging.warning("_find_proxy_index_for_path: Active view's model is not QSortFilterProxyModel.")
            return QModelIndex()

        # Similar traversal as _get_all_visible_image_paths
        queue = []
        root_proxy_parent_idx = QModelIndex()
        for r in range(proxy_model.rowCount(root_proxy_parent_idx)):
            queue.append(proxy_model.index(r, 0, root_proxy_parent_idx))
        
        head = 0
        while head < len(queue):
            current_proxy_idx = queue[head]; head += 1
            if not current_proxy_idx.isValid(): continue

            if self._is_valid_image_item(current_proxy_idx):
                source_idx = proxy_model.mapToSource(current_proxy_idx)
                item = self.file_system_model.itemFromIndex(source_idx) # Use source_model
                if item:
                    item_data = item.data(Qt.ItemDataRole.UserRole)
                    if isinstance(item_data, dict) and item_data.get('path') == target_path:
                        return current_proxy_idx # Found it

            if isinstance(active_view, QTreeView):
                source_idx_for_children_check = proxy_model.mapToSource(current_proxy_idx)
                if source_idx_for_children_check.isValid():
                    item_for_children_check = self.file_system_model.itemFromIndex(source_idx_for_children_check)
                    if item_for_children_check and item_for_children_check.hasChildren() and active_view.isExpanded(current_proxy_idx):
                        for child_row in range(proxy_model.rowCount(current_proxy_idx)): # Children from proxy model
                            queue.append(proxy_model.index(child_row, 0, current_proxy_idx))
        return QModelIndex() # Not found

    def _update_rating_display(self, rating: int):
        for i, btn in enumerate(self.star_buttons):
            star_value = i + 1
            if star_value <= rating: btn.setText("★")
            else: btn.setText("☆")

    def _get_selected_file_paths_from_view(self) -> List[str]:
        """Helper to get valid, unique, existing file paths from the current selection."""
        active_view = self._get_active_file_view()
        if not active_view:
            return []

        selected_indexes = active_view.selectionModel().selectedIndexes()
        selected_file_paths = []
        for proxy_index in selected_indexes:
            if proxy_index.column() == 0:
                source_index = self.proxy_model.mapToSource(proxy_index)
                if source_index.isValid():
                    item = self.file_system_model.itemFromIndex(source_index)
                    if item:
                        item_user_data = item.data(Qt.ItemDataRole.UserRole)
                        if isinstance(item_user_data, dict) and 'path' in item_user_data:
                            file_path = item_user_data['path']
                            if os.path.isfile(file_path): 
                                if file_path not in selected_file_paths: 
                                    selected_file_paths.append(file_path)
        return selected_file_paths

    def _get_cached_metadata_for_selection(self, file_path: str) -> Optional[Dict[str, Any]]:
        """Gets metadata from AppState caches. Assumes caches are populated by RatingLoaderWorker."""
        if not os.path.isfile(file_path):
            logging.warning(f"[_get_cached_metadata_for_selection] File not found: {file_path}")
            return None
            
        # Data should have been populated by RatingLoaderWorker into AppState caches
        # os.path.normpath is important for cache key consistency.
        # RatingLoaderWorker stores with normalized paths.
        normalized_path = os.path.normpath(file_path)

        current_rating = self.app_state.rating_cache.get(normalized_path, 0)
        current_label = self.app_state.label_cache.get(normalized_path)
        current_date = self.app_state.date_cache.get(normalized_path)

        return {'rating': current_rating, 'label': current_label, 'date': current_date}

    def _display_single_image_preview(self, file_path: str, file_data_from_model: Optional[Dict[str, Any]]):
        """Handles displaying preview and info for a single selected image."""
        overall_start_time = time.perf_counter()
        logging.debug(f"[PERF] _display_single_image_preview START for: {os.path.basename(file_path)}")

        metadata_start_time = time.perf_counter()
        # Get metadata from AppState caches
        metadata = self._get_cached_metadata_for_selection(file_path)
        metadata_end_time = time.perf_counter()
        logging.debug(f"[PERF] Metadata retrieval from cache took: {metadata_end_time - metadata_start_time:.4f}s")

        if not metadata:
            self.image_view.setText(f"File not found or metadata error:\n{os.path.basename(file_path)}")
            self.statusBar().showMessage(f"Error accessing file: {os.path.basename(file_path)}", 5000)
            self._update_rating_display(0)
            self._update_label_display(None)
            logging.error(f"[PERF] _display_single_image_preview END (metadata error) for: {os.path.basename(file_path)} in {time.perf_counter() - overall_start_time:.4f}s")
            return

        current_rating = metadata['rating']
        current_label = metadata['label']
        current_date = metadata['date']
        current_cluster = self.app_state.cluster_results.get(file_path, "N/A")
        
        is_blurred_val = None
        if file_data_from_model and isinstance(file_data_from_model, dict):
            is_blurred_val = file_data_from_model.get('is_blurred')

        blur_status_text = ""
        if is_blurred_val is True: blur_status_text = " | Blurred: Yes"
        elif is_blurred_val is False: blur_status_text = " | Blurred: No"

        ui_update_start_time = time.perf_counter()
        self._update_rating_display(current_rating)
        self._update_label_display(current_label)
        ui_update_end_time = time.perf_counter()
        logging.debug(f"[PERF] Rating/label display update took: {ui_update_end_time - ui_update_start_time:.4f}s")

        pixmap_load_start_time = time.perf_counter()
        label_size = self.image_view.size()
        preview_pixmap = None
        thumbnail_pixmap = None
        pixmap_set = False

        try:
            get_preview_start_time = time.perf_counter()
            preview_pixmap = self.image_pipeline.get_preview_qpixmap(
                file_path,
                display_max_size=(label_size.width(), label_size.height()),
                apply_auto_edits=self.apply_auto_edits_enabled
            )
            get_preview_end_time = time.perf_counter()
            logging.debug(f"[PERF] get_preview_qpixmap for {os.path.basename(file_path)} took: {get_preview_end_time - get_preview_start_time:.4f}s. Pixmap is None: {preview_pixmap is None}")

            if preview_pixmap:
                set_pixmap_start_time = time.perf_counter()
                self.image_view.setPixmap(preview_pixmap)
                set_pixmap_end_time = time.perf_counter()
                logging.debug(f"[PERF] setPixmap (preview) for {os.path.basename(file_path)} took: {set_pixmap_end_time - set_pixmap_start_time:.4f}s")
                pixmap_set = True
            else:
                logging.warning(f"[PERF] Preview pixmap was None for {os.path.basename(file_path)}. Trying thumbnail.")
                get_thumb_start_time = time.perf_counter()
                thumbnail_pixmap = self.image_pipeline.get_thumbnail_qpixmap(
                    file_path, apply_auto_edits=self.apply_auto_edits_enabled
                )
                get_thumb_end_time = time.perf_counter()
                logging.debug(f"[PERF] get_thumbnail_qpixmap for {os.path.basename(file_path)} took: {get_thumb_end_time - get_thumb_start_time:.4f}s. Thumbnail is None: {thumbnail_pixmap is None}")

                if thumbnail_pixmap:
                    set_pixmap_start_time = time.perf_counter()
                    self.image_view.setPixmap(thumbnail_pixmap)
                    set_pixmap_end_time = time.perf_counter()
                    logging.debug(f"[PERF] setPixmap (thumbnail) for {os.path.basename(file_path)} took: {set_pixmap_end_time - set_pixmap_start_time:.4f}s")
                    pixmap_set = True
                else:
                    self.image_view.setText(f"Failed to load preview/thumbnail:\n{os.path.basename(file_path)}")
                    logging.error(f"[PERF] Both preview and thumbnail failed for {os.path.basename(file_path)}")

            pixmap_load_end_time = time.perf_counter()
            logging.debug(f"[PERF] Total pixmap loading and setting for {os.path.basename(file_path)} took: {pixmap_load_end_time - pixmap_load_start_time:.4f}s")

            status_bar_start_time = time.perf_counter()
            label_text = current_label if current_label else "None"
            date_text = current_date.strftime("%Y-%m-%d") if current_date else "Unknown"
            cluster_text = f"C: {current_cluster}" if current_cluster != "N/A" else ""
            try:
                original_size = os.path.getsize(file_path) // 1024
                size_text = f"Size: {original_size} KB"
            except OSError as ose:
                size_text = "Size: N/A"
                logging.warning(f"Could not get size for {file_path}: {ose}")


            status_message = (f"{os.path.basename(file_path)} | R: {current_rating} | "
                              f"L: {label_text} | D: {date_text} {cluster_text} | "
                              f"{size_text}{blur_status_text}")
            self.statusBar().showMessage(status_message)
            status_bar_end_time = time.perf_counter()
            logging.debug(f"[PERF] Status bar update took: {status_bar_end_time - status_bar_start_time:.4f}s")

        except Exception as e: # Catch any exception during pixmap handling or UI update
            error_message = f"Critical error displaying image {os.path.basename(file_path)}: {e}"
            logging.error(f"[MainWindow] _display_single_image_preview: {error_message}", exc_info=True)
            self.image_view.setText(f"Error displaying image:\n{os.path.basename(file_path)}")
            self.statusBar().showMessage(error_message, 7000)
        
        logging.debug(f"[PERF] _display_single_image_preview END for: {os.path.basename(file_path)} in {time.perf_counter() - overall_start_time:.4f}s\n")

    def _display_multi_selection_info(self, selected_paths: List[str]):
        """Handles UI updates when multiple (2 or more) images are selected."""
        self.image_view.clear()
        self._update_rating_display(0) 
        self._update_label_display(None)

        if len(selected_paths) == 2:
            self.image_view.setText("Comparing 2 images...")
            path1, path2 = selected_paths[0], selected_paths[1]
            emb1 = self.app_state.embeddings_cache.get(path1)
            emb2 = self.app_state.embeddings_cache.get(path2)

            if emb1 is not None and emb2 is not None:
                try:
                    emb1_np = np.array(emb1).reshape(1, -1)
                    emb2_np = np.array(emb2).reshape(1, -1)
                    similarity = cosine_similarity(emb1_np, emb2_np)[0][0]
                    self.statusBar().showMessage(f"Similarity between {os.path.basename(path1)} and {os.path.basename(path2)}: {similarity:.4f}")
                except Exception as e:
                    self.statusBar().showMessage(f"Error calculating similarity: {e}", 5000)
                    logging.error(f"[MainWindow] Error calculating similarity: {e}", exc_info=True)
            else:
                missing_msg_parts = []
                if emb1 is None: missing_msg_parts.append(f"{os.path.basename(path1)}")
                if emb2 is None: missing_msg_parts.append(f"{os.path.basename(path2)}")
                self.statusBar().showMessage(f"Embeddings not found for: {', '.join(missing_msg_parts)}. Analyze similarity first.", 5000)
        else: # More than 2 items selected
            self.image_view.setText(f"{len(selected_paths)} items selected.")
            self.statusBar().showMessage(f"{len(selected_paths)} items selected. Select 1 for preview or 2 for similarity.")

    def _handle_no_selection_or_non_image(self):
        """Handles UI updates when no valid image is selected or focus is on a non-image item."""
        active_view = self._get_active_file_view()
        if not active_view:
            self.image_view.clear()
            self.image_view.setText("No image selected or multiple items.")
            self._update_rating_display(0)
            self._update_label_display(None)
            self.statusBar().showMessage("Ready")
            return

        current_focused_index = active_view.currentIndex()
        if current_focused_index.isValid():
            source_index = self.proxy_model.mapToSource(current_focused_index)
            item = self.file_system_model.itemFromIndex(source_index)
            if item:
                item_user_data = item.data(Qt.ItemDataRole.UserRole)
                is_image_item = isinstance(item_user_data, dict) and \
                                'path' in item_user_data and \
                                os.path.isfile(item_user_data['path'])
                if not is_image_item: # Focused on a folder or non-image
                    self.image_view.clear()
                    self.image_view.setText(f"{item.text()} selected (not an image file).")
                    self.statusBar().showMessage(f"{item.text()} selected.")
                    self._update_rating_display(0)
                    self._update_label_display(None)
                    return
        
        # Default state if no specific non-image item is clearly focused
        self.image_view.clear()
        self.image_view.setText("No image selected or multiple items.")
        self._update_rating_display(0)
        self._update_label_display(None)
        self.statusBar().showMessage("Ready")

    def _handle_file_selection_changed(self, selected=None, deselected=None):
        selected_file_paths = self._get_selected_file_paths_from_view()
        
        file_data_from_model = None
        if len(selected_file_paths) == 1:
            active_view = self._get_active_file_view()
            if active_view:
                current_proxy_idx = active_view.currentIndex()
                if current_proxy_idx.isValid():
                    source_idx = self.proxy_model.mapToSource(current_proxy_idx)
                    item = self.file_system_model.itemFromIndex(source_idx)
                    if item:
                        file_data_from_model = item.data(Qt.ItemDataRole.UserRole)

        if len(selected_file_paths) == 1:
            self._display_single_image_preview(selected_file_paths[0], file_data_from_model)
        elif len(selected_file_paths) >= 2:
            self._display_multi_selection_info(selected_file_paths)
        else: 
            self._handle_no_selection_or_non_image()

    def _apply_filter(self):
        search_text = self.search_input.text().lower()
        selected_filter_text = self.filter_combo.currentText()
        selected_cluster_text = self.cluster_filter_combo.currentText()
        target_cluster_id = -1
        if self.cluster_filter_combo.isEnabled() and selected_cluster_text != "All Clusters":
            try: target_cluster_id = int(selected_cluster_text.split(" ")[-1])
            except ValueError: pass
        
        self.proxy_model.app_state_ref = self.app_state
        
        self.proxy_model.current_rating_filter = selected_filter_text
        self.proxy_model.current_cluster_filter_id = target_cluster_id
        self.proxy_model.show_folders_mode_ref = self.show_folders_mode
        self.proxy_model.current_view_mode_ref = self.current_view_mode
        self.proxy_model.setFilterRegularExpression(search_text)
        self.proxy_model.setFilterKeyColumn(-1)
        self.proxy_model.setFilterRole(Qt.ItemDataRole.UserRole) 
        self.proxy_model.invalidateFilter()
        active_view = self._get_active_file_view()
        if active_view:
            current_proxy_index = active_view.currentIndex()
            current_source_index = QModelIndex()
            if current_proxy_index.isValid():
                current_source_index = self.proxy_model.mapToSource(current_proxy_index)
            new_proxy_index = self.proxy_model.mapFromSource(current_source_index)
            if not new_proxy_index.isValid():
                first_visible_index = self._find_first_visible_item()
                if first_visible_index.isValid():
                    active_view.setCurrentIndex(first_visible_index)
                    active_view.scrollTo(first_visible_index, QAbstractItemView.ScrollHint.EnsureVisible)
                else:
                    self.image_view.clear(); self.image_view.setText("No images match filter")
                    self._update_rating_display(0); self._update_label_display(None)


    def _start_preview_preloader(self, image_data_list: List[Dict[str, any]]):
        logging.info(f"[MainWindow] <<< ENTRY >>> _start_preview_preloader called with {len(image_data_list)} items.")
        if not image_data_list:
            logging.info("[MainWindow] _start_preview_preloader: image_data_list is empty. Hiding overlay.")
            self.hide_loading_overlay()
            return
        
        paths_for_preloader = [fd['path'] for fd in image_data_list if fd and isinstance(fd, dict) and 'path' in fd]
        logging.info(f"[MainWindow] _start_preview_preloader: Extracted {len(paths_for_preloader)} paths for preloader.")

        if not paths_for_preloader:
            logging.info("[MainWindow] _start_preview_preloader: No valid paths_for_preloader. Hiding overlay.")
            self.hide_loading_overlay()
            return
 
        self.update_loading_text(f"Preloading previews ({len(paths_for_preloader)} images)...")
        logging.info(f"[MainWindow] _start_preview_preloader: Calling worker_manager.start_preview_preload for {len(paths_for_preloader)} paths.")
        try:
            logging.info(f"[MainWindow] _start_preview_preloader: --- CALLING --- worker_manager.start_preview_preload for {len(paths_for_preloader)} paths.")
            self.worker_manager.start_preview_preload(paths_for_preloader, self.apply_auto_edits_enabled)
            logging.info("[MainWindow] _start_preview_preloader: --- RETURNED --- worker_manager.start_preview_preload call successful.")
        except Exception as e_preview_preload:
            logging.error(f"[MainWindow] _start_preview_preloader: Error calling worker_manager.start_preview_preload: {e_preview_preload}", exc_info=True)
            self.hide_loading_overlay() # Ensure overlay is hidden on error
        logging.info(f"[MainWindow] <<< EXIT >>> _start_preview_preloader.")
  
    # Slot for WorkerManager's file_scan_thumbnail_preload_finished signal
    # This signal is now deprecated in favor of chaining after rating load.
    # Keeping the method signature for now in case it's used elsewhere, but logic is changed.
    def _handle_thumbnail_preload_finished(self, all_file_data: List[Dict[str, any]]):
        # This was previously used to kick off preview preloading.
        # Now, preview preloading is kicked off after rating loading finishes.
        # self.update_loading_text("Thumbnails preloaded. Starting preview preloading...")
        # self._start_preview_preloader(all_file_data)
        logging.debug("[MainWindow] _handle_thumbnail_preload_finished called (now largely deprecated by rating load chain)")
        pass # Intentionally do nothing here, preview starts after rating load now

    # --- Rating Loader Worker Handlers ---
    def _handle_rating_load_progress(self, current: int, total: int, basename: str):
        percentage = int((current / total) * 100) if total > 0 else 0
        logging.debug(f"[MainWindow] Rating load progress: {percentage}% ({current}/{total}) - {basename}")
        self.update_loading_text(f"Loading ratings: {percentage}% ({current}/{total}) - {basename}")

    def _handle_metadata_batch_loaded(self, metadata_batch: List[Dict[str, Any]]):
        logging.debug(f"[MainWindow] Metadata batch loaded with {len(metadata_batch)} items.")
        # AppState caches (rating_cache, label_cache, date_cache) are already updated by RatingLoaderWorker.
        
        active_view = self._get_active_file_view()
        currently_selected_path = None
        if active_view and active_view.currentIndex().isValid():
            current_proxy_idx = active_view.currentIndex()
            source_idx = self.proxy_model.mapToSource(current_proxy_idx)
            item = self.file_system_model.itemFromIndex(source_idx)
            if item:
                item_data = item.data(Qt.ItemDataRole.UserRole)
                if isinstance(item_data, dict) and 'path' in item_data:
                    currently_selected_path = item_data.get('path')

        needs_active_selection_refresh = False
        for image_path, metadata in metadata_batch:
            logging.debug(f"[MainWindow] Processing metadata from batch for {os.path.basename(image_path)}: {metadata}")
            if image_path == currently_selected_path:
                logging.debug(f"[MainWindow] Batch contains currently selected item: {os.path.basename(image_path)}. Marking for UI refresh.")
                # Update rating buttons, label buttons directly using the new metadata
                self._update_rating_display(metadata.get('rating', 0))
                self._update_label_display(metadata.get('label'))
                # Mark that the full selection handler needs to be called once after the loop
                # to update status bar and potentially preview if other aspects changed.
                needs_active_selection_refresh = True
        
        if needs_active_selection_refresh:
            logging.debug(f"[MainWindow] Triggering _handle_file_selection_changed after processing batch due to active item update.")
            self._handle_file_selection_changed()
            
        # Optionally, if items being visible depends on metadata (e.g. rating filter),
        # you might need to call self._apply_filter() here or after all batches are done.
        # For now, individual item updates are prioritized.

    def _handle_rating_load_finished(self):
        logging.info("[MainWindow] _handle_rating_load_finished: Received RatingLoaderWorker.finished signal.")
        self.statusBar().showMessage("Background rating loading finished.", 3000)
        
        if not self.app_state.image_files_data:
            logging.info("[MainWindow] _handle_rating_load_finished: No image files data found in app_state. Hiding loading overlay.")
            self.hide_loading_overlay()
            return

        logging.info("[MainWindow] _handle_rating_load_finished: image_files_data found. Preparing to start preview preloader.")
        self.update_loading_text("Ratings loaded. Preloading previews...")
        try:
            logging.info("[MainWindow] _handle_rating_load_finished: --- CALLING --- _start_preview_preloader.")
            self._start_preview_preloader(self.app_state.image_files_data.copy()) # Pass a copy
            logging.info("[MainWindow] _handle_rating_load_finished: --- RETURNED --- _start_preview_preloader call completed.")
        except Exception as e_start_preview:
            logging.error(f"[MainWindow] _handle_rating_load_finished: Error calling _start_preview_preloader: {e_start_preview}", exc_info=True)
            self.hide_loading_overlay() # Ensure overlay is hidden on error
        logging.info("[MainWindow] <<< EXIT >>> _handle_rating_load_finished.")

    def _handle_rating_load_error(self, message: str):
        logging.error(f"[MainWindow] Rating Load Error: {message}")
        self.statusBar().showMessage(f"Rating Load Error: {message}", 5000)
        # Still proceed to preview preloading even if rating load had errors for some files
        if self.app_state.image_files_data:
            self.update_loading_text("Rating load errors. Preloading previews...")
            self._start_preview_preloader(self.app_state.image_files_data.copy()) # Pass a copy
        else:
            self.hide_loading_overlay()


    # Slot for WorkerManager's preview_preload_progress signal
    def _handle_preview_progress(self, percentage: int, message: str):
        logging.info(f"[MainWindow] <<< ENTRY >>> _handle_preview_progress: {percentage}% - {message}")
        self.update_loading_text(message)
        logging.info(f"[MainWindow] <<< EXIT >>> _handle_preview_progress.")

    # Slot for WorkerManager's preview_preload_finished signal
    def _handle_preview_finished(self):
        logging.info("[MainWindow] <<< ENTRY >>> _handle_preview_finished: Received PreviewPreloaderWorker.finished signal.")
        self.statusBar().showMessage("Preview preloading finished.", 5000)
        self.hide_loading_overlay()
        logging.info("[MainWindow] _handle_preview_finished: Loading overlay hidden.")
        # WorkerManager handles thread cleanup
        logging.info("[MainWindow] <<< EXIT >>> _handle_preview_finished.")

    # Slot for WorkerManager's preview_preload_error signal
    def _handle_preview_error(self, message: str):
        logging.info(f"[MainWindow] <<< ENTRY >>> _handle_preview_error: {message}")
        logging.error(f"[MainWindow] Preview Preload Error: {message}")
        self.statusBar().showMessage(f"Preview Preload Error: {message}", 5000)
        self.hide_loading_overlay()
        # WorkerManager handles thread cleanup
        logging.info(f"[MainWindow] <<< EXIT >>> _handle_preview_error.")
 
    def _set_label_from_button(self):
        sender_button = self.sender()
        if sender_button:
            label = sender_button.property("labelValue")
            if label: self._apply_label(label)

    def _clear_label(self):
        self._apply_label(None)

    def _apply_label(self, label: str | None):
        active_view = self._get_active_file_view()
        if not active_view: return
        current_index = active_view.currentIndex()
        if not current_index.isValid(): return
        source_index = self.proxy_model.mapToSource(current_index)
        if not source_index.isValid(): return
        item = self.file_system_model.itemFromIndex(source_index) 
        if not item: return

        item_data = item.data(Qt.ItemDataRole.UserRole)
        if not isinstance(item_data, dict) or 'path' not in item_data: return 
        file_path = item_data['path']
        
        if not os.path.exists(file_path): return

        success = MetadataHandler.set_label(file_path, label, self.app_state.exif_disk_cache)
        if success:
            self.app_state.label_cache[file_path] = label
            self._update_label_display(label)
        else:
            self.statusBar().showMessage(f"Failed to set label for {os.path.basename(file_path)}", 5000)

    def _update_label_display(self, label: str | None):
        selected_border_style = "border: 2px solid #FFFFFF;" 
        default_border_style = "border: 1px solid #33353a;" 

        for color_name, btn in self.color_buttons.items():
            original_hex = btn.property("originalHexColor")
            base_style = f"background-color: {original_hex};" 
            if color_name == label:
                btn.setStyleSheet(base_style + selected_border_style)
            else:
                btn.setStyleSheet(base_style + default_border_style)
        
        clear_button_base_style = self.clear_color_label_button.styleSheet().split("border:")[0] 
        if label is None:
            self.clear_color_label_button.setStyleSheet(clear_button_base_style + selected_border_style)
        else:
             self.clear_color_label_button.setStyleSheet(clear_button_base_style + default_border_style)


    def _set_view_mode_list(self):
        self.current_view_mode = "list"
        self.tree_display_view.setVisible(True)
        self.grid_display_view.setVisible(False)
        self.tree_display_view.setIconSize(QSize(16, 16))
        self.tree_display_view.setIndentation(10)
        self.tree_display_view.setRootIsDecorated(self.show_folders_mode or self.group_by_similarity_mode)
        self.tree_display_view.setItemsExpandable(self.show_folders_mode or self.group_by_similarity_mode)
        if self.tree_display_view.itemDelegate() is self.thumbnail_delegate: 
            self.tree_display_view.setItemDelegate(None)
        self._rebuild_model_view()
        self.tree_display_view.setFocus()

    def _set_view_mode_icons(self):
        self.current_view_mode = "icons"
        self.tree_display_view.setVisible(True)
        self.grid_display_view.setVisible(False)
        self.tree_display_view.setIconSize(QSize(64, 64))
        self.tree_display_view.setIndentation(20)
        self.tree_display_view.setRootIsDecorated(self.show_folders_mode or self.group_by_similarity_mode)
        self.tree_display_view.setItemsExpandable(self.show_folders_mode or self.group_by_similarity_mode)
        if self.tree_display_view.itemDelegate() is self.thumbnail_delegate: 
             self.tree_display_view.setItemDelegate(None)
        self._rebuild_model_view()
        self.tree_display_view.setFocus()

    def _update_grid_view_layout(self):
        if not self.grid_display_view.isVisible(): return
        TARGET_GRID_COLUMNS = 4; GRID_ITEM_HORIZONTAL_SPACING = 10
        MIN_ICON_SIZE = 64; MAX_ICON_SIZE = 192
        viewport_width = self.grid_display_view.viewport().width()
        if viewport_width > 0 and TARGET_GRID_COLUMNS > 0:
            available_width_for_icons = viewport_width - ((TARGET_GRID_COLUMNS -1) * GRID_ITEM_HORIZONTAL_SPACING)
            calculated_icon_width = available_width_for_icons / TARGET_GRID_COLUMNS
            icon_size = int(max(MIN_ICON_SIZE, min(calculated_icon_width, MAX_ICON_SIZE)))
        else: icon_size = MIN_ICON_SIZE
        self.grid_display_view.setIconSize(QSize(icon_size, icon_size))
        self.grid_display_view.setSpacing(GRID_ITEM_HORIZONTAL_SPACING // 2)
        self.grid_display_view.updateGeometries()
        self.grid_display_view.viewport().update()

    def _toggle_folder_visibility(self, checked):
        self.show_folders_mode = checked
        self._rebuild_model_view() 
        if self.current_view_mode == "list": self._set_view_mode_list()
        elif self.current_view_mode == "icons": self._set_view_mode_icons()
        elif self.current_view_mode == "date": self._set_view_mode_date()

    def _toggle_group_by_similarity(self, checked):
        if not self.app_state.cluster_results and checked:
            self.group_by_similarity_action.setChecked(False)
            self.statusBar().showMessage("Cannot group: Run 'Analyze Similarity' first.", 3000)
            return
        self.group_by_similarity_mode = checked
        if checked and self.app_state.cluster_results:
            self.cluster_sort_label.setVisible(True)
            self.cluster_sort_combo.setEnabled(True)
            self.cluster_sort_combo.setVisible(True)
        else:
            self.cluster_sort_label.setVisible(False)
            self.cluster_sort_combo.setEnabled(False)
            self.cluster_sort_combo.setVisible(False)
            if checked and not self.app_state.cluster_results: # Should not happen if initial check passed
                self.group_by_similarity_action.setChecked(False)
                self.group_by_similarity_mode = False
        if self.current_view_mode == "list": self._set_view_mode_list()
        elif self.current_view_mode == "icons": self._set_view_mode_icons()
        elif self.current_view_mode == "grid": self._set_view_mode_grid()
        elif self.current_view_mode == "date": self._set_view_mode_date()
        else: self._rebuild_model_view()

    def _set_view_mode_grid(self):
        self.current_view_mode = "grid"
        if self.group_by_similarity_mode: # Grid view not supported when grouping by similarity
            self.tree_display_view.setVisible(True)
            self.grid_display_view.setVisible(False)
            # Use a suitable icon size for tree when grid would have been active
            self.tree_display_view.setIconSize(QSize(96, 96)) 
            self.tree_display_view.setIndentation(20)
            self.tree_display_view.setRootIsDecorated(True)
            self.tree_display_view.setItemsExpandable(True)
            if self.tree_display_view.itemDelegate() is self.thumbnail_delegate: 
                 self.tree_display_view.setItemDelegate(None)
            self._rebuild_model_view()
            self.tree_display_view.setFocus()
        else:
            self.tree_display_view.setVisible(False)
            self.grid_display_view.setVisible(True)
            self.grid_display_view.setViewMode(QListView.ViewMode.IconMode)
            self.grid_display_view.setFlow(QListView.Flow.LeftToRight)
            self.grid_display_view.setWrapping(True)
            self.grid_display_view.setResizeMode(QListView.ResizeMode.Adjust)
            self._rebuild_model_view() # Populate model first
            self._update_grid_view_layout() # Then adjust layout
            self.grid_display_view.setFocus()

    def _set_view_mode_date(self):
        self.current_view_mode = "date"
        self.tree_display_view.setVisible(True)
        self.grid_display_view.setVisible(False)
        self.tree_display_view.setIconSize(QSize(16, 16))
        self.tree_display_view.setIndentation(20)
        self.tree_display_view.setRootIsDecorated(True)
        self.tree_display_view.setItemsExpandable(True)
        if self.tree_display_view.itemDelegate() is self.thumbnail_delegate: 
            self.tree_display_view.setItemDelegate(None)
        self._rebuild_model_view()
        self.tree_display_view.setFocus()

    def _populate_model_by_date(self, parent_item: QStandardItem, image_data_list: List[Dict[str, any]]):
        if not image_data_list: return

        images_by_year_month: Dict[any, Dict[int, List[Dict[str, any]]]] = {}
        unknown_date_key = "Unknown Date"

        for file_data in image_data_list:
            file_path = file_data['path']
            img_date: date_obj | None = self.app_state.date_cache.get(file_path)
            year = img_date.year if img_date else unknown_date_key
            month = img_date.month if img_date else 1 # Default to 1 if unknown, for sorting

            if year not in images_by_year_month: images_by_year_month[year] = {}
            if month not in images_by_year_month[year]: images_by_year_month[year][month] = []
            images_by_year_month[year][month].append(file_data) 

        sorted_years = sorted(images_by_year_month.keys(), key=lambda y: (y == unknown_date_key, y))
        for year_val in sorted_years:
            year_item = QStandardItem(str(year_val))
            year_item.setEditable(False); year_item.setData(f"date_header_{year_val}", Qt.ItemDataRole.UserRole)
            font = year_item.font(); font.setBold(True); year_item.setFont(font)
            parent_item.appendRow(year_item)

            sorted_months = sorted(images_by_year_month[year_val].keys())
            for month_val in sorted_months:
                parent_for_images = year_item # Default to year item if month is unknown
                if year_val != unknown_date_key: # Only create month sub-item if year is known
                    month_name = date_obj(1900, month_val, 1).strftime("%B")
                    month_item = QStandardItem(month_name)
                    month_item.setEditable(False); month_item.setData(f"date_header_{year_val}-{month_val}", Qt.ItemDataRole.UserRole)
                    year_item.appendRow(month_item)
                    parent_for_images = month_item
                
                files_in_group_data = sorted(
                    images_by_year_month[year_val][month_val],
                    key=lambda fd: (self.app_state.date_cache.get(fd['path']) or date_obj.min, os.path.basename(fd['path']))
                )
                for file_data in files_in_group_data:
                    image_item = self._create_standard_item(file_data) 
                    parent_for_images.appendRow(image_item)

    def _create_standard_item(self, file_data: Dict[str, any]):
        file_path = file_data['path']
        is_blurred = file_data.get('is_blurred')

        item_text = os.path.basename(file_path)
        item = QStandardItem(item_text)
        item.setData(file_data, Qt.ItemDataRole.UserRole)
        item.setEditable(False)

        # Icon logic depends on toggle_thumbnails_action and view mode
        if self.toggle_thumbnails_action.isChecked():
            thumbnail_pixmap = self.image_pipeline.get_thumbnail_qpixmap(file_path, apply_auto_edits=self.apply_auto_edits_enabled)
            if thumbnail_pixmap:
                item.setIcon(QIcon(thumbnail_pixmap))
        
        if is_blurred is True:
            item.setForeground(QColor(Qt.GlobalColor.red))
            item.setText(item_text + " (Blurred)")
        elif is_blurred is False:
            # Use default text color from the application's palette
            item.setForeground(QApplication.palette().text().color()) 
            item.setText(item_text) # No (Not Blurred) suffix needed
        else: # is_blurred is None (unknown)
            item.setForeground(QApplication.palette().text().color()) 
            item.setText(item_text)

        return item

    def _start_similarity_analysis(self):
        logging.info("[MainWindow] _start_similarity_analysis called.")
        if self.worker_manager.is_similarity_worker_running():
            self.statusBar().showMessage("Similarity analysis is already in progress.", 3000)
            return
        
        if not self.app_state.image_files_data:
            self.hide_loading_overlay() 
            self.statusBar().showMessage("No images loaded to analyze similarity.", 3000)
            return
        
        paths_for_similarity = [fd['path'] for fd in self.app_state.image_files_data]
        if not paths_for_similarity:
            self.hide_loading_overlay()
            self.statusBar().showMessage("No valid image paths for similarity analysis.", 3000)
            return
 
        self.show_loading_overlay("Starting similarity analysis...")
        self.analyze_similarity_action.setEnabled(False) 
        self.worker_manager.start_similarity_analysis(paths_for_similarity, self.apply_auto_edits_enabled)
  
    # Slot for WorkerManager's similarity_progress signal
    def _handle_similarity_progress(self, percentage, message):
        self.update_loading_text(f"Similarity: {message} ({percentage}%)")

    # Slot for WorkerManager's similarity_embeddings_generated signal
    def _handle_embeddings_generated(self, embeddings_dict):
        self.app_state.embeddings_cache = embeddings_dict
        self.update_loading_text("Embeddings generated. Clustering...")

    # Slot for WorkerManager's similarity_clustering_complete signal
    def _handle_clustering_complete(self, cluster_results_dict: Dict[str, int]):
        self.app_state.cluster_results = cluster_results_dict
        self.analyze_similarity_action.setEnabled(bool(self.app_state.image_files_data)) 

        if not self.app_state.cluster_results:
            self.hide_loading_overlay()
            self.statusBar().showMessage("Clustering did not produce results.", 3000)
            return

        self.update_loading_text("Clustering complete. Updating view...")
        cluster_ids = sorted(list(set(self.app_state.cluster_results.values())))
        self.cluster_filter_combo.clear()
        self.cluster_filter_combo.addItems(["All Clusters"] + [f"Cluster {cid}" for cid in cluster_ids])
        self.cluster_filter_combo.setEnabled(True)
        self.group_by_similarity_action.setEnabled(True)
        self.group_by_similarity_action.setChecked(True) # Automatically switch to group by similarity view
        if self.group_by_similarity_action.isChecked() and self.app_state.cluster_results:
            self.cluster_sort_label.setVisible(True)
            self.cluster_sort_combo.setEnabled(True)
            self.cluster_sort_combo.setVisible(True)
        if self.group_by_similarity_mode: self._rebuild_model_view()
        self.hide_loading_overlay()

    # Slot for WorkerManager's similarity_error signal
    def _handle_similarity_error(self, message):
        self.statusBar().showMessage(f"Similarity Error: {message}", 8000)
        self.analyze_similarity_action.setEnabled(bool(self.app_state.image_files_data)) 
        self.hide_loading_overlay()
 
    def _reload_current_folder(self):
        if self.app_state.image_files_data: 
            if self.app_state.image_files_data[0] and 'path' in self.app_state.image_files_data[0]:
                current_dir = os.path.dirname(self.app_state.image_files_data[0]['path'])
                if os.path.isdir(current_dir):
                    self._load_folder(current_dir)
                    return
        self.statusBar().showMessage("No folder context to reload.", 3000)


    def _handle_item_collapsed(self, proxy_index: QModelIndex):
        if self.group_by_similarity_mode and proxy_index.isValid():
            active_view = self.tree_display_view
            source_index = self.proxy_model.mapToSource(proxy_index)
            item = self.file_system_model.itemFromIndex(source_index)
            if item:
                item_data = item.data(Qt.ItemDataRole.UserRole)
                if isinstance(item_data, str) and item_data.startswith("cluster_header_"):
                    QTimer.singleShot(0, lambda: active_view.expand(proxy_index))

    def _cluster_sort_changed(self):
        if self.group_by_similarity_mode and self.app_state.cluster_results: 
            self._rebuild_model_view()

    def _get_cluster_timestamps(self, images_by_cluster: Dict[int, List[Dict[str, any]]], date_cache: Dict[str, Optional[date_obj]]) -> Dict[int, date_obj]: 
        cluster_timestamps = {}
        for cluster_id, file_data_list in images_by_cluster.items():
            earliest_date = date_obj.max; found_date = False
            for file_data in file_data_list:
                img_date = date_cache.get(file_data['path'])
                if img_date and img_date < earliest_date:
                    earliest_date = img_date; found_date = True
            cluster_timestamps[cluster_id] = earliest_date if found_date else date_obj.max
        return cluster_timestamps

    def _calculate_cluster_centroids(self, images_by_cluster: Dict[int, List[Dict[str, any]]], embeddings_cache: Dict[str, List[float]]) -> Dict[int, np.ndarray]:
        centroids = {}
        if not embeddings_cache: return centroids
        for cluster_id, file_data_list in images_by_cluster.items():
            cluster_embeddings = []
            for file_data in file_data_list:
                embedding = embeddings_cache.get(file_data['path'])
                if embedding is not None:
                    if isinstance(embedding, np.ndarray): cluster_embeddings.append(embedding)
                    elif isinstance(embedding, list): cluster_embeddings.append(np.array(embedding))
            if cluster_embeddings:
                try:
                    # Ensure all embeddings are numpy arrays before stacking for mean calculation
                    if all(isinstance(emb, np.ndarray) for emb in cluster_embeddings):
                        if cluster_embeddings: # Ensure list is not empty
                             # Explicitly cast to float32 if not already, for consistency
                            centroids[cluster_id] = np.mean(np.array(cluster_embeddings, dtype=np.float32), axis=0)
                except Exception as e: # Catch potential errors in np.mean, like empty list or dtype issues
                    print(f"[MainWindow] Error calculating centroid for cluster {cluster_id}: {e}")
                    pass 
        return centroids

    def _sort_clusters_by_similarity_time(self,
                                          images_by_cluster: Dict[int, List[Dict[str, any]]],
                                          embeddings_cache: Dict[str, List[float]],
                                          date_cache: Dict[str, Optional[date_obj]]) -> List[int]:
        cluster_ids = list(images_by_cluster.keys())
        if not cluster_ids: return []

        centroids = self._calculate_cluster_centroids(images_by_cluster, embeddings_cache)
        valid_cluster_ids_for_pca = [cid for cid in cluster_ids if cid in centroids and centroids[cid] is not None and centroids[cid].size > 0]

        if not valid_cluster_ids_for_pca or len(valid_cluster_ids_for_pca) < 2: 
            cluster_timestamps_for_fallback = self._get_cluster_timestamps(images_by_cluster, date_cache)
            return sorted(list(images_by_cluster.keys()), key=lambda cid_orig: cluster_timestamps_for_fallback.get(cid_orig, date_obj.max))

        valid_centroid_list = [centroids[cid] for cid in valid_cluster_ids_for_pca]
        if not valid_centroid_list: 
             cluster_timestamps_for_fallback = self._get_cluster_timestamps(images_by_cluster, date_cache)
             return sorted(list(images_by_cluster.keys()), key=lambda cid_orig: cluster_timestamps_for_fallback.get(cid_orig, date_obj.max))

        centroid_matrix = np.array(valid_centroid_list)
        
        pca_scores = {}
        # Ensure matrix is 2D and has enough samples/features for PCA
        if centroid_matrix.ndim == 2 and centroid_matrix.shape[0] > 1 and centroid_matrix.shape[1] > 0:
            try:
                # n_components for PCA must be less than min(n_samples, n_features)
                n_components_pca = min(1, centroid_matrix.shape[0] -1 if centroid_matrix.shape[0] > 1 else 1, centroid_matrix.shape[1])
                if n_components_pca > 0 : # Ensure n_components is at least 1
                    pca = PCA(n_components=n_components_pca)
                    transformed_centroids = pca.fit_transform(centroid_matrix)
                    for i, cid in enumerate(valid_cluster_ids_for_pca): 
                        pca_scores[cid] = transformed_centroids[i, 0] if transformed_centroids.ndim > 1 else transformed_centroids[i]
            except Exception as e: 
                print(f"Error during PCA for cluster sorting: {e}")
        
        cluster_timestamps = self._get_cluster_timestamps(images_by_cluster, date_cache)
        sortable_clusters = []
        for cid in cluster_ids: 
            pca_val = pca_scores.get(cid, float('inf')) # Default to inf if PCA score not found
            ts_val = cluster_timestamps.get(cid, date_obj.max)
            sortable_clusters.append((cid, pca_val, ts_val))
        sortable_clusters.sort(key=lambda x: (x[1], x[2])) # Sort by PCA score, then timestamp
        return [item[0] for item in sortable_clusters]

    def _handle_toggle_auto_edits(self, checked: bool):
        self.apply_auto_edits_enabled = checked
        self.image_pipeline.clear_all_image_caches() 
        self._rebuild_model_view() 
        
        if self.app_state.image_files_data:
            self.worker_manager.start_preview_preload(
                [fd['path'] for fd in self.app_state.image_files_data],
                self.apply_auto_edits_enabled
            )

        active_view = self._get_active_file_view()
        if active_view:
            current_proxy_idx = active_view.currentIndex()
            if current_proxy_idx.isValid():
                try:
                    active_view.selectionModel().selectionChanged.disconnect(self._handle_file_selection_changed)
                except TypeError: pass 
                
                self._handle_file_selection_changed() 
                
                try:
                    active_view.selectionModel().selectionChanged.connect(self._handle_file_selection_changed)
                except TypeError: pass 
            else:
                first_visible_item = self._find_first_visible_item()
                if first_visible_item.isValid(): active_view.setCurrentIndex(first_visible_item)
            self.statusBar().showMessage(f"Auto RAW edits {'enabled' if checked else 'disabled'}. Caches cleared, view refreshed.", 3000)

    def _start_blur_detection_analysis(self):
        logging.info("[MainWindow] _start_blur_detection_analysis called.")
        if not self.app_state.image_files_data:
            self.statusBar().showMessage("No images loaded to analyze for blurriness.", 3000)
            return
        
        if self.worker_manager.is_blur_detection_running():
            self.statusBar().showMessage("Blur detection is already in progress.", 3000)
            return
 
        self.show_loading_overlay("Starting blur detection...")
        self.detect_blur_action.setEnabled(False)
 
        self.worker_manager.start_blur_detection(
            self.app_state.image_files_data.copy(),
            self.blur_detection_threshold,
            self.apply_auto_edits_enabled
        )
 
    # Slot for WorkerManager's blur_detection_progress signal
    def _handle_blur_detection_progress(self, current: int, total: int, path_basename: str):
        percentage = int((current / total) * 100) if total > 0 else 0
        self.update_loading_text(f"Detecting blur: {percentage}% ({current}/{total}) - {path_basename}")

    # Slot for WorkerManager's blur_detection_status_updated signal
    def _handle_blur_status_updated(self, image_path: str, is_blurred: bool):
        self.app_state.update_blur_status(image_path, is_blurred) 

        source_model = self.file_system_model
        proxy_model = self.proxy_model
        active_view = self._get_active_file_view()

        item_to_update = None
        # This search needs to be through the source model, not the proxy,
        # because the item might be filtered out in the proxy.
        # We iterate through the source model to find the QStandardItem.
        for r_top in range(source_model.rowCount()):
            top_item = source_model.item(r_top)
            if not top_item: continue
            
            # Check top-level item
            top_item_data = top_item.data(Qt.ItemDataRole.UserRole)
            if isinstance(top_item_data, dict) and top_item_data.get('path') == image_path:
                item_to_update = top_item
                break

            if top_item.hasChildren(): # Check children if it's a folder/group
                for r_child in range(top_item.rowCount()):
                    child_item = top_item.child(r_child)
                    if not child_item: continue

                    child_item_data = child_item.data(Qt.ItemDataRole.UserRole)
                    if isinstance(child_item_data, dict) and child_item_data.get('path') == image_path:
                        item_to_update = child_item
                        break
                    
                    # Potentially check grandchildren if structure is deeper (e.g., date view inside cluster view)
                    if child_item.hasChildren(): 
                        for r_grandchild in range(child_item.rowCount()):
                            grandchild_item = child_item.child(r_grandchild)
                            if not grandchild_item: continue
                            grandchild_item_data = grandchild_item.data(Qt.ItemDataRole.UserRole)
                            if isinstance(grandchild_item_data, dict) and grandchild_item_data.get('path') == image_path:
                                item_to_update = grandchild_item
                                break
                        if item_to_update: break
                if item_to_update: break
        
        if item_to_update:
            original_text = os.path.basename(image_path)
            # Update the UserRole data in the source model item
            item_user_data = item_to_update.data(Qt.ItemDataRole.UserRole)
            if isinstance(item_user_data, dict):
                item_user_data['is_blurred'] = is_blurred # Update existing dict
                item_to_update.setData(item_user_data, Qt.ItemDataRole.UserRole)
            else: # Should not happen if item was created correctly
                 item_to_update.setData({'path': image_path, 'is_blurred': is_blurred}, Qt.ItemDataRole.UserRole) 

            # Update display text and color
            if is_blurred is True:
                item_to_update.setForeground(QColor(Qt.GlobalColor.red))
                item_to_update.setText(original_text + " (Blurred)")
            elif is_blurred is False:
                default_text_color = QApplication.palette().text().color()
                item_to_update.setForeground(default_text_color)
                item_to_update.setText(original_text) 
            else: # is_blurred is None
                default_text_color = QApplication.palette().text().color()
                item_to_update.setForeground(default_text_color)
                item_to_update.setText(original_text)

            # If the updated item is currently selected, refresh the main image view and status bar
            if active_view and active_view.currentIndex().isValid():
                current_proxy_idx = active_view.currentIndex()
                current_source_idx = proxy_model.mapToSource(current_proxy_idx)
                selected_item = source_model.itemFromIndex(current_source_idx)
                if selected_item == item_to_update:
                    self._handle_file_selection_changed() # Re-process selection to update main view
        else:
            logging.warning(f"Could not find QStandardItem for {image_path} to update blur status in UI.")


    # Slot for WorkerManager's blur_detection_finished signal
    def _handle_blur_detection_finished(self):
        self.hide_loading_overlay()
        self.statusBar().showMessage("Blur detection complete.", 5000)
        self.detect_blur_action.setEnabled(bool(self.app_state.image_files_data)) # Re-enable
        # WorkerManager handles thread cleanup

    def _perform_group_selection_from_key(self, key: int, active_view_from_event: QWidget) -> bool:
        """
        Handles selection of an image within the current cluster based on a numeric key (1-9).
        Called by eventFilter or potentially keyPressEvent.
        Returns True if event was handled, False otherwise.
        """
        if not self.group_by_similarity_mode: # Guard: only if in correct mode
            return False

        target_image_index_in_cluster = key - Qt.Key.Key_1 # 0-indexed
        
        # active_view_from_event is the QTreeView or QListView that received the event
        # Ensure it's the one we expect for current UI state.
        active_view = self._get_active_file_view()
        if active_view_from_event is not active_view:
            # This could happen if the event filter is somehow triggered for a non-active view
            # or if keyPressEvent calls this when focus is not on the primary view.
            logging.warning("_perform_group_selection_from_key: Event source mismatch with _get_active_file_view().")
            return False # Don't handle if view context is mismatched

        if not active_view:
            return False

        current_proxy_idx = active_view.currentIndex()
        if not current_proxy_idx.isValid():
            return False # No current item to determine cluster from

        # Determine the cluster ID of the currently selected/focused item's group
        determined_cluster_id = None
        search_idx = current_proxy_idx
        while search_idx.isValid():
            s_idx = self.proxy_model.mapToSource(search_idx)
            item_at_search = self.file_system_model.itemFromIndex(s_idx)
            if not item_at_search: break
            current_item_user_data = item_at_search.data(Qt.ItemDataRole.UserRole)

            if isinstance(current_item_user_data, dict) and 'path' in current_item_user_data:
                image_path = current_item_user_data['path']
                if os.path.exists(image_path): # Check path exists before cache lookup
                    determined_cluster_id = self.app_state.cluster_results.get(image_path)
                    break
            elif isinstance(current_item_user_data, str) and current_item_user_data.startswith("cluster_header_"):
                try: determined_cluster_id = int(current_item_user_data.split("_")[-1])
                except ValueError: pass
                break
            
            parent_of_search_idx = search_idx.parent()
            if not parent_of_search_idx.isValid() and search_idx.isValid(): # Reached top-level proxy item
                break
            search_idx = parent_of_search_idx
        
        if determined_cluster_id is None:
            return False # Could not determine current cluster

        images_by_cluster_map = self._group_images_by_cluster()
        images_in_target_cluster = images_by_cluster_map.get(determined_cluster_id, [])
        if not images_in_target_cluster:
            return False

        # Sort images within the cluster consistent with display order
        current_cluster_sort_method = self.cluster_sort_combo.currentText()
        if current_cluster_sort_method == "Time" or current_cluster_sort_method == "Similarity then Time":
            image_sort_key_func = lambda fd: (self.app_state.date_cache.get(fd['path'], date_obj.max), os.path.basename(fd['path']))
        else: # Default sort (by basename)
            image_sort_key_func = lambda fd: os.path.basename(fd['path'])
        sorted_images_in_cluster_data = sorted(images_in_target_cluster, key=image_sort_key_func)

        if 0 <= target_image_index_in_cluster < len(sorted_images_in_cluster_data):
            target_file_data_dict = sorted_images_in_cluster_data[target_image_index_in_cluster]
            target_file_path = target_file_data_dict['path']

            # Find the proxy QModelIndex for the target_file_path within its cluster header
            proxy_root = QModelIndex()
            cluster_header_proxy_idx = QModelIndex()
            for r in range(self.proxy_model.rowCount(proxy_root)):
                idx = self.proxy_model.index(r, 0, proxy_root)
                s_idx_header = self.proxy_model.mapToSource(idx)
                header_item = self.file_system_model.itemFromIndex(s_idx_header)
                if header_item:
                    header_data = header_item.data(Qt.ItemDataRole.UserRole)
                    if isinstance(header_data, str) and header_data == f"cluster_header_{determined_cluster_id}":
                        cluster_header_proxy_idx = idx
                        break
            
            if not cluster_header_proxy_idx.isValid():
                return False # Cluster header not found in proxy model

            target_proxy_idx_to_select = QModelIndex()
            for r_child in range(self.proxy_model.rowCount(cluster_header_proxy_idx)):
                child_proxy_idx = self.proxy_model.index(r_child, 0, cluster_header_proxy_idx)
                child_source_idx = self.proxy_model.mapToSource(child_proxy_idx)
                child_item = self.file_system_model.itemFromIndex(child_source_idx)
                if child_item:
                    child_item_user_data = child_item.data(Qt.ItemDataRole.UserRole)
                    if isinstance(child_item_user_data, dict) and child_item_user_data.get('path') == target_file_path:
                        target_proxy_idx_to_select = child_proxy_idx
                        break
            
            if target_proxy_idx_to_select.isValid():
                active_view.selectionModel().clearSelection()
                active_view.setCurrentIndex(target_proxy_idx_to_select)
                active_view.selectionModel().select(target_proxy_idx_to_select, QItemSelectionModel.SelectionFlag.ClearAndSelect)
                active_view.scrollTo(target_proxy_idx_to_select, QAbstractItemView.ScrollHint.EnsureVisible)
                return True # Event handled successfully
        
        return False # Index out of bounds or other failure


    def eventFilter(self, obj: QObject, event: QEvent) -> bool:
        if event.type() == QEvent.Type.KeyPress:
            # Ensure the event is for one of our views
            if (obj is self.tree_display_view or obj is self.grid_display_view):
                
                key_event: QKeyEvent = event # Cast event to QKeyEvent
                key = key_event.key()
                
                search_has_focus = self.search_input.hasFocus()

                # Handle Arrow Keys & Delete for navigation (if search input doesn't have focus on the view itself)
                if not search_has_focus: # Only act if search input doesn't have focus
                    if key == Qt.Key.Key_Left or key == Qt.Key.Key_A:
                        self._navigate_left_in_group()
                        return True
                    elif key == Qt.Key.Key_Right or key == Qt.Key.Key_D:
                        self._navigate_right_in_group()
                        return True
                    elif key == Qt.Key.Key_Up or key == Qt.Key.Key_W:
                        self._navigate_up_sequential()
                        return True
                    elif key == Qt.Key.Key_Down or key == Qt.Key.Key_S:
                        self._navigate_down_sequential()
                        return True
                    elif key == Qt.Key.Key_Delete:
                        self._move_current_image_to_trash()
                        return True

                # Handle 1-9 for group selection (existing logic, also conditional on search_input focus)
                if not search_has_focus and \
                   key_event.modifiers() == Qt.KeyboardModifier.NoModifier and \
                   self.group_by_similarity_mode and \
                   key >= Qt.Key.Key_1 and key <= Qt.Key.Key_9:
                    
                    current_active_view_for_logic = self._get_active_file_view()
                    # Ensure the event source is the logically active view for this specific shortcut
                    if obj is current_active_view_for_logic:
                        if self._perform_group_selection_from_key(key, obj):
                            return True # Event consumed
        
        # Pass unhandled events to the base class
        return super().eventFilter(obj, event)

    # Slot for WorkerManager's blur_detection_error signal
    def _handle_blur_detection_error(self, message: str):
        self.hide_loading_overlay()
        self.statusBar().showMessage(f"Blur Detection Error: {message}", 8000)
        self.detect_blur_action.setEnabled(bool(self.app_state.image_files_data)) # Re-enable
        # WorkerManager handles thread cleanup

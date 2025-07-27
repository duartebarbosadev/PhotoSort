import time
import logging

logger = logging.getLogger(__name__)
from src.ui.advanced_image_viewer import SynchronizedImageViewer
from PyQt6.QtWidgets import (
    QMainWindow,
    QWidget,
    QVBoxLayout,
    QHBoxLayout,
    QSplitter,
    QFileDialog,
    QTreeView,  # Replaced QListWidget with QTreeView
    QPushButton,
    QListView,
    QComboBox,
    QStyle,  # For standard icons
    QAbstractItemView,
    QApplication,  # For selection and edit triggersor dialogs
)
import os  # <-- Add import os at the top level
from datetime import date as date_obj  # For date type hinting and objects
from typing import (
    List,
    Dict,
    Optional,
    Any,
    Tuple,
)  # Import List and Dict for type hinting, Optional, Any, Tuple
from PyQt6.QtCore import (
    Qt,
    QModelIndex,
    QSortFilterProxyModel,
    QObject,
    QTimer,
    QItemSelectionModel,
    QEvent,
    QItemSelection,
)
from PyQt6.QtGui import (
    QColor,
    QAction,
    QKeyEvent,
    QIcon,
    QStandardItemModel,
    QStandardItem,
    QResizeEvent,
)
import numpy as np
from sklearn.decomposition import PCA
from sklearn.metrics.pairwise import cosine_similarity  # Add cosine_similarity import

# from src.core.file_scanner import FileScanner # Now managed by WorkerManager
# from src.core.similarity_engine import SimilarityEngine # Now managed by WorkerManager
# from src.core.similarity_engine import PYTORCH_CUDA_AVAILABLE # Import PyTorch CUDA info <-- ENSURE REMOVED
from src.core.image_pipeline import ImagePipeline

# from src.core.image_features.blur_detector import BlurDetector # Now managed by WorkerManager
from src.core.metadata_processor import MetadataProcessor  # New metadata processor
from src.core.app_settings import (
    get_preview_cache_size_gb,
    set_preview_cache_size_gb,
    set_exif_cache_size_mb,
    get_auto_edit_photos,
    set_auto_edit_photos,
    get_mark_for_deletion_mode,
    set_mark_for_deletion_mode,
    DEFAULT_BLUR_DETECTION_THRESHOLD,
    DEFAULT_MAX_ITERATIONS,
    DEFAULT_SAFETY_ITERATION_MULTIPLIER,
    LEFT_PANEL_STRETCH,
    CENTER_PANEL_STRETCH,
    RIGHT_PANEL_STRETCH,
)
from src.ui.app_state import AppState
from src.ui.ui_components import LoadingOverlay
from src.ui.worker_manager import WorkerManager
from src.ui.metadata_sidebar import MetadataSidebar
from src.ui.dialog_manager import DialogManager
from src.ui.left_panel import LeftPanel
from src.ui.app_controller import AppController
from src.ui.menu_manager import MenuManager


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

        is_image_item = isinstance(item_user_data, dict) and "path" in item_user_data

        if not is_image_item:
            return False

        file_path = item_user_data["path"]
        if not os.path.exists(file_path):
            return False

        search_text = self.filterRegularExpression().pattern().lower()
        search_match = search_text in item_text.lower()
        if not search_match:
            return False

        if not self.app_state_ref:
            return True

        current_rating = self.app_state_ref.rating_cache.get(
            file_path, 0
        )  # Uses in-memory cache, populated by RatingLoaderWorker
        rating_filter = self.current_rating_filter
        rating_passes = (
            rating_filter == "Show All"
            or (rating_filter == "Unrated (0)" and current_rating == 0)
            or (rating_filter == "1 Star +" and current_rating >= 1)
            or (rating_filter == "2 Stars +" and current_rating >= 2)
            or (rating_filter == "3 Stars +" and current_rating >= 3)
            or (rating_filter == "4 Stars +" and current_rating >= 4)
            or (rating_filter == "5 Stars" and current_rating == 5)
        )
        if not rating_passes:
            return False

        cluster_filter_id = self.current_cluster_filter_id
        cluster_passes = cluster_filter_id == -1 or (
            self.app_state_ref.cluster_results.get(file_path) == cluster_filter_id
        )
        if not cluster_passes:
            return False
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

    def __init__(self, initial_folder=None):
        super().__init__()
        init_start_time = time.perf_counter()
        logger.debug("Initializing MainWindow...")
        self.initial_folder = initial_folder
        self._is_syncing_selection = False
        self._left_panel_views = set()
        self._image_viewer_views = set()

        self.image_pipeline = ImagePipeline()
        self.app_state = AppState()
        self.worker_manager = WorkerManager(
            image_pipeline_instance=self.image_pipeline, parent=self
        )
        self.dialog_manager = DialogManager(self)
        self.app_controller = AppController(
            main_window=self,
            app_state=self.app_state,
            worker_manager=self.worker_manager,
            parent=self,
        )
        logger.debug("Core components initialized.")

        self.setWindowTitle("PhotoSort")
        self.setGeometry(100, 100, 1200, 800)

        self.loading_overlay = None
        self.metadata_sidebar = None
        self.sidebar_visible = False
        self.thumbnail_delegate = None
        self.show_folders_mode = False
        self.group_by_similarity_mode = False
        self.apply_auto_edits_enabled = get_auto_edit_photos()
        self.mark_for_deletion_mode_enabled = get_mark_for_deletion_mode()
        self.blur_detection_threshold = DEFAULT_BLUR_DETECTION_THRESHOLD
        self.rotation_suggestions = {}

        self.filter_combo = QComboBox()
        self.filter_combo.addItems(
            [
                "Show All",
                "Unrated (0)",
                "1 Star +",
                "2 Stars +",
                "3 Stars +",
                "4 Stars +",
                "5 Stars",
            ]
        )
        self.cluster_filter_combo = QComboBox()
        self.cluster_filter_combo.addItems(["All Clusters"])
        self.cluster_filter_combo.setEnabled(False)
        self.cluster_filter_combo.setToolTip("Filter images by similarity cluster")
        self.cluster_sort_combo = QComboBox()
        self.cluster_sort_combo.addItems(["Time", "Similarity then Time"])
        self.cluster_sort_combo.setEnabled(False)
        self.cluster_sort_combo.setToolTip(
            "Order of clusters when 'Group by Similarity' is active"
        )
        logger.debug("Filter controls created.")

        self.menu_manager = MenuManager(self)
        self.menu_manager.create_menus(self.menuBar())
        self._create_widgets()
        self._create_layout()
        self._create_loading_overlay()
        self.left_panel.thumbnail_delegate = self.thumbnail_delegate
        self._connect_signals()
        self._update_image_info_label()
        logger.debug("UI components and signals initialized.")

        logger.info(
            f"MainWindow initialization complete in {time.perf_counter() - init_start_time:.2f}s."
        )

        # Hide rotation view by default
        self._hide_rotation_view()

        # Load initial folder if provided
        if self.initial_folder and os.path.isdir(self.initial_folder):
            QTimer.singleShot(
                0, lambda: self.app_controller.load_folder(self.initial_folder)
            )

    # Helper method to update the image information in status bar
    def _update_image_info_label(self, status_message_override: Optional[str] = None):
        if status_message_override:
            self.statusBar().showMessage(status_message_override)
            return

        num_images = 0
        total_size_mb = 0.0
        folder_name_display = "N/A"
        # Default text if no folder is loaded yet
        status_text = "No folder loaded. Open a folder to begin."
        scan_logically_active = False  # Initialize to avoid UnboundLocalError

        if self.app_state.current_folder_path:
            folder_name_display = os.path.basename(self.app_state.current_folder_path)
            if not folder_name_display:  # Handles "C:/"
                folder_name_display = self.app_state.current_folder_path

            # Determine if scan is considered "active" based on UI elements
            # open_folder_action is disabled during the scan process.
            scan_logically_active = not self.menu_manager.open_folder_action.isEnabled()

            if scan_logically_active:
                # Scan is in progress
                num_images_found_so_far = len(
                    self.app_state.image_files_data
                )  # Current count during scan
                status_text = f"Folder: {folder_name_display}  |  Scanning... ({num_images_found_so_far} files found)"
            elif self.app_state.image_files_data:  # Scan is finished and there's data
                num_images = len(self.app_state.image_files_data)
                current_files_size_bytes = 0
                for file_data in self.app_state.image_files_data:
                    try:
                        if "path" in file_data and os.path.exists(file_data["path"]):
                            current_files_size_bytes += os.path.getsize(
                                file_data["path"]
                            )
                    except OSError as e:
                        # Log lightly, this can be noisy if many files are temporarily unavailable
                        logger.warning(
                            f"Could not get size for '{file_data.get('path', 'Unknown')}' for info label: {e}"
                        )
                total_size_mb = current_files_size_bytes / (1024 * 1024)

                # Add cache size information to the status text
                preview_cache_size_bytes = self.image_pipeline.preview_cache.volume()
                preview_cache_size_mb = preview_cache_size_bytes / (1024 * 1024)

                status_text = (
                    f"Folder: {folder_name_display} | "
                    f"Images: {num_images} ({total_size_mb:.2f} MB) | "
                    f"Preview Cache: {preview_cache_size_mb:.2f} MB"
                )
            else:  # Folder path set, scan finished (or not started if folder just selected), no image data
                status_text = f"Folder: {folder_name_display}  |  Images: 0 (0.00 MB)"

        self.statusBar().showMessage(status_text)

    def _create_loading_overlay(self):
        start_time = time.perf_counter()
        logger.debug("Creating loading overlay...")
        parent_for_overlay = self
        if parent_for_overlay:
            self.loading_overlay = LoadingOverlay(parent_for_overlay)
            self.loading_overlay.hide()
        else:
            logger.warning(
                "Could not create loading overlay: parent widget not available."
            )
        logger.debug(
            f"Loading overlay created in {time.perf_counter() - start_time:.4f}s"
        )

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

    def _update_cache_dialog_labels(self):
        thumb_usage_bytes = self.image_pipeline.thumbnail_cache.volume()
        self.thumb_cache_usage_label.setText(
            f"{thumb_usage_bytes / (1024 * 1024):.2f} MB"
        )

        configured_gb = get_preview_cache_size_gb()
        self.preview_cache_configured_limit_label.setText(f"{configured_gb:.2f} GB")

        preview_usage_bytes = self.image_pipeline.preview_cache.volume()
        self.preview_cache_usage_label.setText(
            f"{preview_usage_bytes / (1024 * 1024):.2f} MB"
        )

        # Update EXIF cache labels
        if hasattr(self, "app_state") and self.app_state.exif_disk_cache:
            exif_configured_mb = (
                self.app_state.exif_disk_cache.get_current_size_limit_mb()
            )
            self.exif_cache_configured_limit_label.setText(f"{exif_configured_mb} MB")
            exif_usage_bytes = self.app_state.exif_disk_cache.volume()
            self.exif_cache_usage_label.setText(
                f"{exif_usage_bytes / (1024 * 1024):.2f} MB"
            )
        else:  # Fallback if app_state or exif_disk_cache is not yet fully initialized
            self.exif_cache_configured_limit_label.setText("N/A")
            self.exif_cache_usage_label.setText("N/A")

    def _clear_thumbnail_cache_action(self):
        self.image_pipeline.thumbnail_cache.clear()
        self.statusBar().showMessage("Thumbnail cache cleared.", 5000)
        self._update_cache_dialog_labels()
        self._refresh_visible_items_icons()

    def _clear_preview_cache_action(self):
        self.image_pipeline.preview_cache.clear()
        self.statusBar().showMessage(
            "Preview cache cleared. Previews will regenerate.", 5000
        )
        self._update_cache_dialog_labels()
        self._refresh_current_selection_preview()

    def _apply_preview_cache_limit_action(self):
        selected_index = self.preview_cache_size_combo.currentIndex()
        new_size_gb = 0
        if self.preview_cache_size_combo.itemText(selected_index).endswith("(Custom)"):
            new_size_gb = float(
                self.preview_cache_size_combo.itemText(selected_index).split(" ")[0]
            )
        elif 0 <= selected_index < len(self.preview_cache_size_options_gb):
            new_size_gb = self.preview_cache_size_options_gb[selected_index]
        else:
            self.statusBar().showMessage("Invalid selection for cache size.", 3000)
            return

        current_size_gb = get_preview_cache_size_gb()
        if new_size_gb != current_size_gb:
            set_preview_cache_size_gb(new_size_gb)
            self.image_pipeline.reinitialize_preview_cache_from_settings()
            self.statusBar().showMessage(
                f"Preview cache limit set to {new_size_gb:.2f} GB. Cache reinitialized.",
                5000,
            )
        else:
            self.statusBar().showMessage(
                f"Preview cache limit is already {new_size_gb:.2f} GB.", 3000
            )
        self._update_cache_dialog_labels()

    def _clear_exif_cache_action(self):
        if self.app_state.exif_disk_cache:
            self.app_state.exif_disk_cache.clear()
            self.app_state.rating_disk_cache.clear()
            self.statusBar().showMessage("EXIF and rating caches cleared.", 5000)
            self._update_cache_dialog_labels()
            # No direct visual refresh needed for EXIF data itself in list/grid,
            # but metadata display for current image might need update
            self._refresh_current_selection_preview()  # This will re-fetch metadata

    def _apply_exif_cache_limit_action(self):
        selected_index = self.exif_cache_size_combo.currentIndex()
        new_size_mb = 0
        if self.exif_cache_size_combo.itemText(selected_index).endswith("(Custom)"):
            new_size_mb = int(
                self.exif_cache_size_combo.itemText(selected_index).split(" ")[0]
            )
        elif 0 <= selected_index < len(self.exif_cache_size_options_mb):
            new_size_mb = self.exif_cache_size_options_mb[selected_index]
        else:
            self.statusBar().showMessage("Invalid selection for EXIF cache size.", 3000)
            return

        if self.app_state.exif_disk_cache:
            current_size_mb = self.app_state.exif_disk_cache.get_current_size_limit_mb()
            if new_size_mb != current_size_mb:
                set_exif_cache_size_mb(new_size_mb)  # Update app_settings
                self.app_state.exif_disk_cache.reinitialize_from_settings()  # Reinitialize ExifCache
                self.statusBar().showMessage(
                    f"EXIF cache limit set to {new_size_mb} MB. Cache reinitialized.",
                    5000,
                )
            else:
                self.statusBar().showMessage(
                    f"EXIF cache limit is already {new_size_mb} MB.", 3000
                )
        self._update_cache_dialog_labels()

    def _refresh_visible_items_icons(self):
        active_view = self._get_active_file_view()
        if active_view:
            active_view.viewport().update()

    def _refresh_current_selection_preview(self):
        active_view = self._get_active_file_view()
        if active_view and active_view.currentIndex().isValid():
            self._handle_file_selection_changed()

    def _create_widgets(self):
        """Create the UI widgets."""
        start_time = time.perf_counter()
        logger.debug("Creating widgets...")
        self.file_system_model = QStandardItemModel()
        self.proxy_model = CustomFilterProxyModel(self)
        self.proxy_model.setSourceModel(self.file_system_model)
        self.proxy_model.app_state_ref = self.app_state  # Link AppState to proxy model

        self.left_panel = LeftPanel(self.proxy_model, self.app_state, self)

        self._left_panel_views = {
            self.left_panel.tree_display_view,
            self.left_panel.grid_display_view,
            self.left_panel.rotation_suggestions_view,
        }

        self.center_pane_container = QWidget()
        self.center_pane_container.setObjectName("center_pane_container")
        center_pane_layout = QVBoxLayout(self.center_pane_container)
        center_pane_layout.setContentsMargins(0, 0, 0, 0)
        center_pane_layout.setSpacing(0)

        # Advanced image viewer instead of simple QLabel
        self.advanced_image_viewer = SynchronizedImageViewer()
        self.advanced_image_viewer.setObjectName("advanced_image_viewer")
        self.advanced_image_viewer.setMinimumSize(400, 300)
        center_pane_layout.addWidget(self.advanced_image_viewer, 1)

        # Keep a reference to the first viewer for backward compatibility if needed,
        # but primary interaction is now with the SynchronizedImageViewer itself.
        self.image_view = self.advanced_image_viewer.image_viewers[0].image_view
        self._image_viewer_views = {
            v.image_view for v in self.advanced_image_viewer.image_viewers
        }

        # The rating and color controls are now part of the IndividualViewer
        # widgets inside SynchronizedImageViewer, so they are no longer created here.

        self.accept_all_button = QPushButton("Accept All")
        self.accept_all_button.setObjectName("acceptAllButton")
        self.accept_all_button.setVisible(False)
        self.accept_button = QPushButton("Accept")
        self.accept_button.setObjectName("acceptButton")
        self.accept_button.setVisible(False)
        self.refuse_button = QPushButton("Refuse")
        self.refuse_button.setObjectName("refuseButton")
        self.refuse_button.setVisible(False)
        self.refuse_all_button = QPushButton("Refuse All")
        self.refuse_all_button.setObjectName("refuseAllButton")
        self.refuse_all_button.setVisible(False)

        button_layout = QHBoxLayout()
        button_layout.addStretch(1)
        button_layout.addWidget(self.accept_button)
        button_layout.addWidget(self.accept_all_button)
        button_layout.addSpacing(20)  # Add space between button groups
        button_layout.addWidget(self.refuse_button)
        button_layout.addWidget(self.refuse_all_button)
        button_layout.addStretch(1)

        center_pane_layout.addLayout(button_layout)

        # Create dummy view mode buttons for compatibility (not displayed)
        self.view_list_button = QPushButton("List")
        self.view_list_button.setCheckable(True)
        self.view_list_button.setVisible(False)
        self.view_icons_button = QPushButton("Icons")
        self.view_icons_button.setCheckable(True)
        self.view_icons_button.setVisible(False)
        self.view_grid_button = QPushButton("Grid")
        self.view_grid_button.setCheckable(True)
        self.view_grid_button.setVisible(False)
        self.view_date_button = QPushButton("Date")
        self.view_date_button.setCheckable(True)
        self.view_date_button.setVisible(False)

        # No bottom bar - image info will be shown in status bar only

        self.statusBar().showMessage("Ready")
        logger.debug(f"Widgets created in {time.perf_counter() - start_time:.4f}s.")

    def _create_layout(self):
        """Set up the main window layout."""
        start_time = time.perf_counter()
        logger.debug("Creating layout...")
        central_widget = QWidget()
        main_layout = QVBoxLayout(central_widget)
        main_layout.setContentsMargins(0, 0, 0, 0)
        main_layout.setSpacing(0)

        main_splitter = QSplitter(Qt.Orientation.Horizontal)
        main_splitter.setObjectName("main_splitter")

        main_splitter.addWidget(self.left_panel)
        main_splitter.addWidget(self.center_pane_container)

        # Create metadata sidebar and add to splitter
        self.metadata_sidebar = MetadataSidebar(self)
        self.metadata_sidebar.hide_requested.connect(self._hide_metadata_sidebar)
        main_splitter.addWidget(self.metadata_sidebar)

        # Set stretch factors: left=1, center=3, right=1 (when visible)
        main_splitter.setStretchFactor(0, LEFT_PANEL_STRETCH)  # Left pane
        main_splitter.setStretchFactor(1, CENTER_PANEL_STRETCH)  # Center pane
        main_splitter.setStretchFactor(2, RIGHT_PANEL_STRETCH)  # Right pane (sidebar)

        # Initially hide the sidebar by setting its size to 0
        main_splitter.setSizes([350, 850, 0])
        self.main_splitter = main_splitter  # Store reference for sidebar toggling

        main_layout.addWidget(main_splitter)

        self.setCentralWidget(central_widget)
        logger.debug(f"Layout created in {time.perf_counter() - start_time:.4f}s.")

    def _connect_signals(self):
        start_time = time.perf_counter()
        logger.debug("Connecting signals...")
        # Connect to the new signals from the advanced viewer
        self.advanced_image_viewer.ratingChanged.connect(self._apply_rating)
        self.advanced_image_viewer.focused_image_changed.connect(
            self._handle_focused_image_changed
        )
        self.advanced_image_viewer.side_by_side_availability_changed.connect(
            self._on_side_by_side_availability_changed
        )

        # Connect UI component signals
        self.left_panel.tree_display_view.installEventFilter(self)
        self.left_panel.grid_display_view.installEventFilter(self)
        self.left_panel.rotation_suggestions_view.installEventFilter(self)
        for viewer in self.advanced_image_viewer.image_viewers:
            viewer.image_view.installEventFilter(self)
        self.left_panel.tree_display_view.clicked.connect(self._handle_tree_view_click)
        self.left_panel.tree_display_view.customContextMenuRequested.connect(
            self.menu_manager.show_image_context_menu
        )
        self.left_panel.grid_display_view.customContextMenuRequested.connect(
            self.menu_manager.show_image_context_menu
        )
        self.left_panel.tree_display_view.selectionModel().selectionChanged.connect(
            self._handle_file_selection_changed
        )
        self.left_panel.grid_display_view.selectionModel().selectionChanged.connect(
            self._handle_file_selection_changed
        )
        self.filter_combo.currentIndexChanged.connect(self._apply_filter)
        self.cluster_filter_combo.currentIndexChanged.connect(self._apply_filter)
        self.cluster_sort_combo.currentIndexChanged.connect(self._cluster_sort_changed)
        self.left_panel.search_input.textChanged.connect(self._apply_filter)
        self.left_panel.tree_display_view.collapsed.connect(self._handle_item_collapsed)
        self.left_panel.connect_signals()

        # Connect MenuManager signals
        self.menu_manager.connect_signals()

        # Delegate signal connections to the AppController
        self.app_controller.connect_signals()

        self.accept_all_button.clicked.connect(self._accept_all_rotations)
        self.accept_button.clicked.connect(self._accept_current_rotation)
        self.refuse_button.clicked.connect(self._refuse_current_rotation)
        logger.debug(f"Signals connected in {time.perf_counter() - start_time:.4f}s.")

    # def _connect_rating_actions(self):
    #     for rating_value, action in self.menu_manager.rating_actions.items():
    #         action.triggered.connect(self._apply_rating_from_action)

    def _apply_rating_from_action(self):
        sender_action = self.sender()
        if isinstance(sender_action, QAction):
            rating = sender_action.data()
            if rating is not None:
                # In side-by-side mode, this keyboard shortcut could apply to one or both images.
                # For now, we apply it to all selected images for simplicity.
                self._apply_rating_to_selection(rating)

    def _open_folder_dialog(self):
        folder_path = QFileDialog.getExistingDirectory(
            self,
            "Select Folder",
            "",
            QFileDialog.Option.ShowDirsOnly | QFileDialog.Option.DontResolveSymlinks,
        )
        if folder_path:
            self.app_controller.load_folder(folder_path)
        else:
            self.statusBar().showMessage("Folder selection cancelled.")

    def _toggle_thumbnail_view(self, checked):
        self._rebuild_model_view()

    def _rebuild_model_view(
        self,
        preserved_selection_paths: Optional[List[str]] = None,
        preserved_focused_path: Optional[str] = None,
    ):
        if preserved_selection_paths is None:
            preserved_selection_paths = self._get_selected_file_paths_from_view()
        if preserved_focused_path is None:
            preserved_focused_path = self.app_state.focused_image_path

        self.update_loading_text("Rebuilding view...")
        QApplication.processEvents()
        self.file_system_model.clear()
        root_item = self.file_system_model.invisibleRootItem()
        active_view = self._get_active_file_view()

        if not self.app_state.image_files_data:
            self.statusBar().showMessage("No images loaded.", 3000)
            return

        if self.left_panel.current_view_mode == "rotation":
            self._rebuild_rotation_view()
        elif self.group_by_similarity_mode:
            if not self.app_state.cluster_results:
                no_cluster_item = QStandardItem("Run 'Analyze Similarity' to group.")
                no_cluster_item.setEditable(False)
                root_item.appendRow(no_cluster_item)
                return

            images_by_cluster = self._group_images_by_cluster()
            if not images_by_cluster:
                no_images_in_clusters = QStandardItem("No images assigned to clusters.")
                no_images_in_clusters.setEditable(False)
                root_item.appendRow(no_images_in_clusters)
                return

            sorted_cluster_ids = list(images_by_cluster.keys())
            current_sort_method = self.cluster_sort_combo.currentText()
            if current_sort_method == "Time":
                cluster_timestamps = self._get_cluster_timestamps(
                    images_by_cluster, self.app_state.date_cache
                )
                sorted_cluster_ids.sort(
                    key=lambda cid: cluster_timestamps.get(cid, date_obj.max)
                )
            elif current_sort_method == "Similarity then Time":
                if not self.app_state.embeddings_cache:
                    cluster_timestamps = self._get_cluster_timestamps(
                        images_by_cluster, self.app_state.date_cache
                    )
                    sorted_cluster_ids.sort(
                        key=lambda cid: cluster_timestamps.get(cid, date_obj.max)
                    )
                else:
                    sorted_cluster_ids = self._sort_clusters_by_similarity_time(
                        images_by_cluster,
                        self.app_state.embeddings_cache,
                        self.app_state.date_cache,
                    )
            else:  # Default sort
                sorted_cluster_ids.sort()

            total_clustered_images = 0
            for cluster_id in sorted_cluster_ids:
                cluster_item = QStandardItem(f"Group {cluster_id}")
                cluster_item.setEditable(False)
                cluster_item.setData(
                    f"cluster_header_{cluster_id}", Qt.ItemDataRole.UserRole
                )
                cluster_item.setForeground(QColor(Qt.GlobalColor.gray))
                root_item.appendRow(cluster_item)
                files_in_cluster = images_by_cluster[cluster_id]
                total_clustered_images += len(files_in_cluster)
                if self.left_panel.current_view_mode == "date":
                    self._populate_model_by_date(cluster_item, files_in_cluster)
                else:
                    self._populate_model_standard(cluster_item, files_in_cluster)
            self.statusBar().showMessage(
                f"Grouped {total_clustered_images} images into {len(sorted_cluster_ids)} clusters.",
                3000,
            )
        else:  # Not grouping by similarity
            if self.left_panel.current_view_mode == "date":
                self._populate_model_by_date(root_item, self.app_state.image_files_data)
            else:
                self._populate_model_standard(
                    root_item, self.app_state.image_files_data
                )
            self.statusBar().showMessage(
                f"View populated with {len(self.app_state.image_files_data)} images.",
                3000,
            )

        self._apply_filter()
        if self.group_by_similarity_mode and isinstance(active_view, QTreeView):
            proxy_root = QModelIndex()
            for i in range(self.proxy_model.rowCount(proxy_root)):
                proxy_cluster_index = self.proxy_model.index(i, 0, proxy_root)
                if proxy_cluster_index.isValid():
                    source_cluster_index = self.proxy_model.mapToSource(
                        proxy_cluster_index
                    )
                    item = self.file_system_model.itemFromIndex(source_cluster_index)
                    if item:
                        item_user_data = item.data(Qt.ItemDataRole.UserRole)
                        if isinstance(
                            item_user_data, str
                        ) and item_user_data.startswith("cluster_header_"):
                            if not active_view.isRowHidden(
                                proxy_cluster_index.row(), proxy_cluster_index.parent()
                            ):
                                active_view.expand(proxy_cluster_index)

        if active_view:
            active_view.updateGeometries()
            active_view.viewport().update()

            focused_proxy_idx = (
                self._find_proxy_index_for_path(preserved_focused_path)
                if preserved_focused_path
                else QModelIndex()
            )

            selection_to_restore = QItemSelection()
            for path in preserved_selection_paths:
                proxy_idx = self._find_proxy_index_for_path(path)
                if proxy_idx.isValid():
                    selection_to_restore.select(proxy_idx, proxy_idx)

            if not selection_to_restore.isEmpty():
                active_view.selectionModel().select(
                    selection_to_restore,
                    QItemSelectionModel.SelectionFlag.ClearAndSelect,
                )
                # If a specific item was focused, make it the current index
                if focused_proxy_idx.isValid():
                    active_view.setCurrentIndex(focused_proxy_idx)
                    active_view.scrollTo(
                        focused_proxy_idx, QAbstractItemView.ScrollHint.EnsureVisible
                    )
                else:  # Otherwise, scroll to the first selected item
                    first_selected_idx = selection_to_restore.indexes()[0]
                    active_view.setCurrentIndex(first_selected_idx)
                    active_view.scrollTo(
                        first_selected_idx, QAbstractItemView.ScrollHint.EnsureVisible
                    )
            else:
                # If no selection to restore, fall back to selecting the first visible item
                first_index = self._find_first_visible_item()
                if first_index.isValid():
                    active_view.setCurrentIndex(first_index)
                    active_view.scrollTo(
                        first_index, QAbstractItemView.ScrollHint.EnsureVisible
                    )
                else:  # No items visible after filter
                    self.image_view.clear()
                    self.image_view.setText("No items match filter")
                    self.advanced_image_viewer.clear()
                    self.statusBar().showMessage("No items match current filter.")

            # Ensure any necessary parent items are expanded to show the selection
            final_focus_index = active_view.currentIndex()
            if final_focus_index.isValid():
                current_parent = final_focus_index.parent()
                expand_list = []
                while current_parent.isValid() and current_parent != QModelIndex():
                    expand_list.append(current_parent)
                    current_parent = current_parent.parent()
                if isinstance(active_view, QTreeView):
                    for idx_to_expand in reversed(expand_list):
                        active_view.expand(idx_to_expand)

    def _reload_current_folder(self):
        self.app_controller.reload_current_folder()

    def _rebuild_rotation_view(self):
        self.file_system_model.clear()
        root_item = self.file_system_model.invisibleRootItem()

        if not self.rotation_suggestions:
            no_suggestions_item = QStandardItem("No rotation suggestions available.")
            no_suggestions_item.setEditable(False)
            root_item.appendRow(no_suggestions_item)
            return

        for path, rotation in self.rotation_suggestions.items():
            item = QStandardItem(os.path.basename(path))
            item.setData({"path": path, "rotation": rotation}, Qt.ItemDataRole.UserRole)
            root_item.appendRow(item)

    def _group_images_by_cluster(self) -> Dict[int, List[Dict[str, any]]]:
        images_by_cluster: Dict[int, List[Dict[str, any]]] = {}
        image_data_map = {
            img_data["path"]: img_data for img_data in self.app_state.image_files_data
        }

        for file_path, cluster_id in self.app_state.cluster_results.items():
            if file_path in image_data_map:
                if cluster_id not in images_by_cluster:
                    images_by_cluster[cluster_id] = []
                images_by_cluster[cluster_id].append(image_data_map[file_path])
        return images_by_cluster

    def _populate_model_standard(
        self, parent_item: QStandardItem, image_data_list: List[Dict[str, any]]
    ):
        if not image_data_list:
            return

        if self.show_folders_mode and not self.group_by_similarity_mode:
            files_by_folder: Dict[str, List[Dict[str, any]]] = {}
            for file_data in image_data_list:
                f_path = file_data["path"]
                folder = os.path.dirname(f_path)
                if folder not in files_by_folder:
                    files_by_folder[folder] = []
                files_by_folder[folder].append(file_data)

            for folder_path in sorted(files_by_folder.keys()):
                folder_name = os.path.basename(folder_path) if folder_path else "Root"
                folder_item = QStandardItem(folder_name)
                folder_item.setEditable(False)
                folder_item.setData(folder_path, Qt.ItemDataRole.UserRole)
                folder_item.setIcon(
                    self.style().standardIcon(QStyle.StandardPixmap.SP_DirIcon)
                )
                parent_item.appendRow(folder_item)
                for file_data in sorted(
                    files_by_folder[folder_path],
                    key=lambda fd: os.path.basename(fd["path"]),
                ):
                    image_item = self._create_standard_item(file_data)
                    folder_item.appendRow(image_item)
        else:  # Not showing folders, or grouping by similarity (which creates its own top-level groups)

            def image_sort_key_func(fd):
                return os.path.basename(fd["path"])

            parent_data = parent_item.data(Qt.ItemDataRole.UserRole)
            is_cluster_header = isinstance(parent_data, str) and parent_data.startswith(
                "cluster_header_"
            )

            if self.group_by_similarity_mode and is_cluster_header:
                current_cluster_sort_method = self.cluster_sort_combo.currentText()
                if (
                    current_cluster_sort_method == "Time"
                    or current_cluster_sort_method == "Similarity then Time"
                ):

                    def image_sort_key_func(fd):
                        return (
                            self.app_state.date_cache.get(fd["path"], date_obj.max),
                            os.path.basename(fd["path"]),
                        )

            for file_data in sorted(image_data_list, key=image_sort_key_func):
                image_item = self._create_standard_item(file_data)
                parent_item.appendRow(image_item)

    def _apply_rating(self, file_path: str, rating: int):
        """Apply rating to a specific file path, called by signal."""
        if not os.path.exists(file_path):
            return

        success = MetadataProcessor.set_rating(
            file_path,
            rating,
            self.app_state.rating_disk_cache,
            self.app_state.exif_disk_cache,
        )

        if success:
            self.app_state.rating_cache[file_path] = rating
            self._apply_filter()
        else:
            self.statusBar().showMessage(
                f"Failed to set rating for {os.path.basename(file_path)}", 5000
            )

    def _apply_rating_to_selection(self, rating: int):
        """Applies a rating to all currently selected images."""
        selected_paths = self._get_selected_file_paths_from_view()
        if not selected_paths:
            # If no selection, apply to the currently displayed image(s) in the advanced viewer
            for viewer in self.advanced_image_viewer.image_viewers:
                if viewer.isVisible() and viewer._file_path:
                    self._apply_rating(viewer._file_path, rating)
                    viewer.update_rating_display(rating)
            return

        for path in selected_paths:
            self._apply_rating(path, rating)
            for viewer in self.advanced_image_viewer.image_viewers:
                if viewer.isVisible() and viewer._file_path == path:
                    viewer.update_rating_display(rating)

    def _log_qmodelindex(self, index: QModelIndex, prefix: str = "") -> str:
        if not hasattr(self, "proxy_model") or not hasattr(self, "file_system_model"):
            return f"{prefix} Invalid QModelIndex (models not initialized)"

        if not index.isValid():
            return f"{prefix} Invalid QModelIndex"

        source_index = self.proxy_model.mapToSource(index)
        item_text = "N/A"

        if source_index.isValid():
            item = self.file_system_model.itemFromIndex(source_index)
            if item:
                item_text = item.text()

        return f"{prefix} (Row: {index.row()}, Text: '{item_text}')"

    def _is_valid_image_item(self, proxy_index: QModelIndex) -> bool:
        if not proxy_index.isValid():
            return False

        source_index = self.proxy_model.mapToSource(proxy_index)
        if not source_index.isValid():
            return False

        item = self.file_system_model.itemFromIndex(source_index)
        if not item:
            return False

        item_user_data = item.data(Qt.ItemDataRole.UserRole)
        is_image = (
            isinstance(item_user_data, dict)
            and "path" in item_user_data
            and os.path.isfile(item_user_data["path"])
        )

        return is_image

    def _get_active_file_view(self):
        return self.left_panel.get_active_view() if self.left_panel else None

    def resizeEvent(self, event: QResizeEvent):
        super().resizeEvent(event)
        if (
            hasattr(self, "left_panel")
            and self.left_panel.current_view_mode == "grid"
            and not self.group_by_similarity_mode
        ):
            if hasattr(self, "left_panel"):
                self.left_panel.update_grid_view_layout()

        if self.loading_overlay:
            self.loading_overlay.update_position()

        # Sidebar positioning is now handled by the splitter automatically
        # No need for manual position updates

    def keyPressEvent(self, event: QKeyEvent):
        # Arrow key and Delete navigation is now handled by the eventFilter for the views.
        # MainWindow.keyPressEvent will handle other application-wide shortcuts
        # or fallbacks if focus is not on the views.

        key = event.key()

        # Escape key to clear focus from search input (if it has focus)
        if key == Qt.Key.Key_Escape:
            if self.left_panel.search_input.hasFocus():
                self.left_panel.search_input.clearFocus()
                active_view = self._get_active_file_view()
                if active_view:
                    active_view.setFocus()  # Return focus to the view
                event.accept()
                return

        # Other global shortcuts for MainWindow could be here.
        # e.g. Ctrl+F is handled by QAction self.find_action

        super().keyPressEvent(event)  # Pass to super for any other default handling

    def _handle_image_focus_shortcut(self):
        """Handles the triggered signal from the 1-9 QAction shortcuts."""
        sender = self.sender()
        if not isinstance(sender, QAction):
            return

        index = sender.data()
        if index is None:
            return

        selected_paths = self._get_selected_file_paths_from_view()
        num_selected = len(selected_paths)

        # CASE 1: In group mode with a single image selected (or no selection),
        # numbers select the Nth image within the current item's group.
        if self.group_by_similarity_mode and num_selected <= 1:
            key = index + Qt.Key.Key_1
            active_view = self._get_active_file_view()
            if active_view and self._perform_group_selection_from_key(key, active_view):
                return  # Handled

        # CASE 2: In all other cases (not group mode, OR group mode with multi-select),
        # numbers switch focus among the selected images in the viewer (basic action).
        else:
            num_images = sum(
                1 for v in self.advanced_image_viewer.image_viewers if v.has_image()
            )
            # It is considered multi-image if more than one image is loaded into the viewer,
            # even if only one is currently visible (focused mode).
            if num_images > 1 and index < num_images:
                self.advanced_image_viewer.set_focused_viewer(index)
                return  # Handled

    def _focus_search_input(self):
        self.left_panel.search_input.setFocus()
        self.left_panel.search_input.selectAll()

    def _handle_delete_action(self):
        if self.mark_for_deletion_mode_enabled:
            self._mark_selection_for_deletion()
        else:
            self._move_current_image_to_trash()

    def _move_current_image_to_trash(self):
        active_view = self._get_active_file_view()
        if not active_view:
            return

        # This function is complex. Let's add more targeted debug logs.
        logger.debug("Initiating file deletion process.")

        # --- Pre-deletion information gathering ---
        self.original_selection_paths = self._get_selected_file_paths_from_view()
        focused_path_to_delete = (
            self.advanced_image_viewer.get_focused_image_path_if_any()
        )

        # If a single image is focused in the viewer from a multi-selection,
        # we prioritize deleting only that focused image.
        if focused_path_to_delete:
            deleted_file_paths = [focused_path_to_delete]
            self.was_focused_delete = True
            logger.debug(
                f"Deleting focused image: {os.path.basename(focused_path_to_delete)}"
            )
        else:
            # Otherwise, delete the entire selection from the list/grid view.
            deleted_file_paths = self.original_selection_paths
            self.was_focused_delete = False

        if not deleted_file_paths:
            self.statusBar().showMessage("No image(s) selected to delete.", 3000)
            return

        if not self.dialog_manager.show_confirm_delete_dialog(deleted_file_paths):
            return

        # Store the proxy index of the initially focused item to try and select something near it later.
        # This is tricky with multiple selections across different parents, so we'll simplify.
        # We'll try to select the item that was *next* to the *first* item in the original selection,
        # or the first visible item if that fails.

        active_selection = active_view.selectionModel().selectedIndexes()
        if active_selection:
            pass

        # This part now finds the source indices for the file paths we've decided to delete.
        # This correctly handles deleting a single focused file or a whole selection.
        source_indices_to_delete = []
        for path_to_delete in deleted_file_paths:
            proxy_idx_to_delete = self._find_proxy_index_for_path(path_to_delete)
            if proxy_idx_to_delete.isValid():
                source_idx = self.proxy_model.mapToSource(proxy_idx_to_delete)
                if source_idx.isValid() and source_idx not in source_indices_to_delete:
                    source_indices_to_delete.append(source_idx)

        # Sort source indices in reverse order by row, then by parent.
        # This ensures that when we remove items from a parent, the row numbers
        # of subsequent items in that same parent are not affected.
        source_indices_to_delete.sort(
            key=lambda idx: (idx.parent().internalId(), idx.row()), reverse=True
        )

        deleted_count = 0
        affected_source_parent_items = []  # Store unique QStandardItem objects of parents

        for source_idx_to_delete in source_indices_to_delete:
            item_to_delete = self.file_system_model.itemFromIndex(source_idx_to_delete)
            if not item_to_delete:
                continue

            item_data = item_to_delete.data(Qt.ItemDataRole.UserRole)
            if not isinstance(item_data, dict) or "path" not in item_data:
                continue

            file_path_to_delete = item_data["path"]
            if not os.path.isfile(file_path_to_delete):
                continue

            file_name_to_delete = os.path.basename(file_path_to_delete)
            try:
                self.app_controller.move_to_trash(file_path_to_delete)
                self.app_state.remove_data_for_path(file_path_to_delete)

                source_parent_idx = source_idx_to_delete.parent()
                source_parent_item = (
                    self.file_system_model.itemFromIndex(source_parent_idx)
                    if source_parent_idx.isValid()
                    else self.file_system_model.invisibleRootItem()
                )

                if source_parent_item:
                    source_parent_item.takeRow(source_idx_to_delete.row())
                    if source_parent_item not in affected_source_parent_items:
                        affected_source_parent_items.append(
                            source_parent_item
                        )  # Add if unique
                deleted_count += 1
            except Exception as e:
                # Log the error to terminal as well for easier debugging
                logger.error(f"Error moving file to trash: {e}", exc_info=True)
                self.dialog_manager.show_error_dialog(
                    "Delete Error", f"Could not move {file_name_to_delete} to trash."
                )
                # Optionally, break or continue if one file fails

        if deleted_count > 0:
            # Check and remove empty group headers
            # Iterate over a list copy as we might modify the underlying structure
            parents_to_check_for_emptiness = list(affected_source_parent_items)
            logger.debug(
                f"Checking {len(parents_to_check_for_emptiness)} parent groups for emptiness after deletion."
            )

            for parent_item_candidate in parents_to_check_for_emptiness:
                if (
                    parent_item_candidate == self.file_system_model.invisibleRootItem()
                ):  # Skip root
                    continue
                if (
                    parent_item_candidate.model() is None
                ):  # Already removed (e.g. child of another removed empty group)
                    logger.debug(
                        f"Parent candidate '{parent_item_candidate.text()}' is no longer in the model, skipping."
                    )
                    continue

                is_eligible_group_header = False
                parent_user_data = parent_item_candidate.data(Qt.ItemDataRole.UserRole)

                if isinstance(parent_user_data, str):
                    if parent_user_data.startswith(
                        "cluster_header_"
                    ) or parent_user_data.startswith("date_header_"):
                        is_eligible_group_header = True
                    elif (
                        self.show_folders_mode
                        and not self.group_by_similarity_mode
                        and os.path.isdir(parent_user_data)
                    ):  # Folder item
                        is_eligible_group_header = True

                if is_eligible_group_header and parent_item_candidate.rowCount() == 0:
                    item_row = (
                        parent_item_candidate.row()
                    )  # Get row before potential parent() call alters context
                    # parent() of a QStandardItem returns its QStandardItem parent, or None if it's a top-level item.
                    actual_parent_qstandarditem = parent_item_candidate.parent()

                    parent_to_operate_on = None
                    parent_display_name_for_log = ""

                    if (
                        actual_parent_qstandarditem is None
                    ):  # It's a top-level item in the model
                        parent_to_operate_on = (
                            self.file_system_model.invisibleRootItem()
                        )
                        parent_display_name_for_log = "invisibleRootItem"
                    else:  # It's a child of another QStandardItem
                        parent_to_operate_on = actual_parent_qstandarditem
                        parent_display_name_for_log = (
                            f"'{actual_parent_qstandarditem.text()}'"
                        )

                    logger.debug(
                        f"Removing empty group header '{parent_item_candidate.text()}' (row {item_row}) from parent {parent_display_name_for_log}"
                    )

                    # Use takeRow on the QStandardItem that is the actual parent in the model hierarchy
                    removed_items_list = parent_to_operate_on.takeRow(item_row)

                    if (
                        removed_items_list
                    ):  # takeRow returns a list of QStandardItems removed
                        logger.debug(
                            f"Removed empty group: '{parent_item_candidate.text()}'."
                        )
                    else:
                        logger.warning(
                            f"Failed to remove empty group '{parent_item_candidate.text()}' from parent {parent_display_name_for_log} at row {item_row}."
                        )

            self.statusBar().showMessage(
                f"{deleted_count} image(s) moved to trash.", 5000
            )
            active_view.selectionModel().clearSelection()  # Clear old selection to avoid issues

            # Get the state of the view AFTER deletions have occurred.
            visible_paths_after_delete = self._get_all_visible_image_paths()
            logger.debug(
                f"{len(visible_paths_after_delete)} visible paths remaining after deletion."
            )

            # This flag will be used to determine if our special focused-delete logic handled the selection.
            selection_handled_by_focus_logic = False

            if self.was_focused_delete:
                remaining_selection_paths = [
                    p
                    for p in self.original_selection_paths
                    if p in visible_paths_after_delete
                ]
                logger.debug(
                    f"Found {len(remaining_selection_paths)} remaining items from original selection."
                )

                if remaining_selection_paths:
                    self._handle_file_selection_changed(
                        override_selected_paths=remaining_selection_paths
                    )

                    selection = QItemSelection()
                    first_proxy_idx_to_select = QModelIndex()

                    for path in remaining_selection_paths:
                        proxy_idx = self._find_proxy_index_for_path(path)
                        if proxy_idx.isValid():
                            selection.select(proxy_idx, proxy_idx)
                            if not first_proxy_idx_to_select.isValid():
                                first_proxy_idx_to_select = proxy_idx

                    if not selection.isEmpty():
                        selection_model = active_view.selectionModel()
                        selection_model.blockSignals(True)
                        selection_model.select(
                            selection, QItemSelectionModel.SelectionFlag.ClearAndSelect
                        )
                        selection_model.blockSignals(False)

                        if first_proxy_idx_to_select.isValid():
                            active_view.scrollTo(
                                first_proxy_idx_to_select,
                                QAbstractItemView.ScrollHint.EnsureVisible,
                            )

                    selection_handled_by_focus_logic = True

            # Fallback logic for standard deletion or if focused-delete logic fails to find items
            if not selection_handled_by_focus_logic:
                if not visible_paths_after_delete:
                    logger.debug("No visible image items left after deletion.")
                    self.advanced_image_viewer.clear()
                    self.advanced_image_viewer.setText("No images left to display.")
                    self.statusBar().showMessage("No images left or visible.")
                else:
                    first_deleted_path_idx_in_visible_list = -1
                    if visible_paths_before_delete and deleted_file_paths:
                        try:
                            first_deleted_path_idx_in_visible_list = (
                                visible_paths_before_delete.index(deleted_file_paths[0])
                            )
                        except ValueError:
                            first_deleted_path_idx_in_visible_list = 0
                    elif visible_paths_before_delete:
                        first_deleted_path_idx_in_visible_list = 0

                    target_idx_in_new_list = min(
                        first_deleted_path_idx_in_visible_list,
                        len(visible_paths_after_delete) - 1,
                    )
                    target_idx_in_new_list = max(0, target_idx_in_new_list)

                    next_item_to_select_proxy_idx = self._find_proxy_index_for_path(
                        visible_paths_after_delete[target_idx_in_new_list]
                    )

                    if next_item_to_select_proxy_idx.isValid():
                        active_view.setCurrentIndex(next_item_to_select_proxy_idx)
                        active_view.selectionModel().select(
                            next_item_to_select_proxy_idx,
                            QItemSelectionModel.SelectionFlag.ClearAndSelect,
                        )
                        active_view.scrollTo(
                            next_item_to_select_proxy_idx,
                            QAbstractItemView.ScrollHint.EnsureVisible,
                        )
                        # The selection change will trigger _handle_file_selection_changed automatically.
                    else:
                        logger.debug(
                            "Fallback failed. No valid item to select. Clearing UI."
                        )
                        self.advanced_image_viewer.clear()
                        self.advanced_image_viewer.setText("No valid image to select.")

            self._update_image_info_label()
        elif (
            deleted_count == 0 and len(self.original_selection_paths) > 0
        ):  # No items were actually deleted, but some were selected
            self.statusBar().showMessage(
                "No valid image files were deleted from selection.", 3000
            )

    def _is_row_hidden_in_tree_if_applicable(
        self, active_view, proxy_idx: QModelIndex
    ) -> bool:
        if isinstance(active_view, QTreeView):
            # Ensure proxy_idx is valid and has a valid parent for isRowHidden
            if proxy_idx.isValid() and proxy_idx.parent().isValid():
                return active_view.isRowHidden(proxy_idx.row(), proxy_idx.parent())
            elif proxy_idx.isValid():  # Top-level item
                return active_view.isRowHidden(
                    proxy_idx.row(), QModelIndex()
                )  # Parent is root
        return False

    def _is_expanded_group_header(self, active_view, proxy_idx: QModelIndex) -> bool:
        if not proxy_idx.isValid() or not isinstance(active_view, QTreeView):
            return False

        source_idx = self.proxy_model.mapToSource(proxy_idx)
        item = self.file_system_model.itemFromIndex(source_idx)
        if not item:
            return False

        user_data = item.data(Qt.ItemDataRole.UserRole)
        is_group = False
        if isinstance(user_data, str):
            if user_data.startswith("cluster_header_") or user_data.startswith(
                "date_header_"
            ):
                is_group = True
            elif (
                self.show_folders_mode
                and not self.group_by_similarity_mode
                and os.path.isdir(user_data)
            ):
                is_group = True

        return is_group and active_view.isExpanded(proxy_idx)

    def _find_last_visible_image_item_in_subtree(
        self, parent_proxy_idx: QModelIndex, skip_deleted: bool = True
    ) -> QModelIndex:
        active_view = self._get_active_file_view()
        # Ensure active_view and its model are valid
        if (
            not active_view
            or not active_view.model()
            or not isinstance(active_view.model(), QSortFilterProxyModel)
        ):
            return QModelIndex()

        proxy_model = active_view.model()  # This should be self.proxy_model

        for i in range(
            proxy_model.rowCount(parent_proxy_idx) - 1, -1, -1
        ):  # Iterate children in reverse
            child_proxy_idx = proxy_model.index(i, 0, parent_proxy_idx)
            if not child_proxy_idx.isValid():
                continue

            # If this child is an expanded group (QTreeView only), recurse
            if isinstance(active_view, QTreeView) and self._is_expanded_group_header(
                active_view, child_proxy_idx
            ):
                found_in_child_group = self._find_last_visible_image_item_in_subtree(
                    child_proxy_idx
                )
                if found_in_child_group.isValid():
                    return found_in_child_group

            # If not an expanded group where an item was found, check if the child itself is a visible image
            if self._is_valid_image_item(
                child_proxy_idx
            ) and not self._is_row_hidden_in_tree_if_applicable(
                active_view, child_proxy_idx
            ):
                source_idx = self.proxy_model.mapToSource(child_proxy_idx)
                item = self.file_system_model.itemFromIndex(source_idx)
                item_data = item.data(Qt.ItemDataRole.UserRole) if item else None
                path = item_data.get("path") if isinstance(item_data, dict) else None

                if skip_deleted and path and self._is_marked_for_deletion(path):
                    continue  # Skip this item and keep searching backwards in the loop

                return child_proxy_idx

        return QModelIndex()  # No visible image item found in this subtree

    def closeEvent(self, event):
        logger.info("Stopping all workers on application close.")
        self.worker_manager.stop_all_workers()  # Use WorkerManager to stop all
        event.accept()

    def _get_current_group_sibling_images(
        self, current_image_proxy_idx: QModelIndex
    ) -> Tuple[Optional[QModelIndex], List[QModelIndex], int]:
        """
        Finds the parent group of the current image and all its visible sibling image items.
        Returns (parent_group_proxy_idx, list_of_sibling_image_proxy_indices, local_idx_of_current_image).
        If not in a group (top-level), parent_group_proxy_idx is root QModelIndex().
        """
        active_view = self._get_active_file_view()
        if not active_view or not current_image_proxy_idx.isValid():
            return QModelIndex(), [], -1

        proxy_model = active_view.model()
        if not isinstance(
            proxy_model, QSortFilterProxyModel
        ):  # Should be self.proxy_model
            return QModelIndex(), [], -1

        parent_proxy_idx = current_image_proxy_idx.parent()

        sibling_image_items = []
        current_item_local_idx = -1

        for i in range(proxy_model.rowCount(parent_proxy_idx)):
            sibling_idx = proxy_model.index(i, 0, parent_proxy_idx)
            if not sibling_idx.isValid():
                continue

            if self._is_valid_image_item(
                sibling_idx
            ) and not self._is_row_hidden_in_tree_if_applicable(
                active_view, sibling_idx
            ):
                source_idx = self.proxy_model.mapToSource(sibling_idx)
                item = self.file_system_model.itemFromIndex(source_idx)
                item_data = item.data(Qt.ItemDataRole.UserRole) if item else None
                path = item_data.get("path") if isinstance(item_data, dict) else None

                if path and self._is_marked_for_deletion(path):
                    continue  # Do not add this marked item to the list of siblings

                sibling_image_items.append(sibling_idx)
                if sibling_idx == current_image_proxy_idx:
                    current_item_local_idx = len(sibling_image_items) - 1

        return parent_proxy_idx, sibling_image_items, current_item_local_idx

    def _validate_and_select_image_candidate(
        self, candidate_idx: QModelIndex, direction: str, skip_deleted: bool
    ) -> bool:
        """
        Validates if a QModelIndex is a selectable image item, and if so, selects it.
        This includes checking if the item is marked for deletion.

        Args:
            candidate_idx: The QModelIndex of the potential item.
            direction: A string ("left", "right", "up", "down") for logging.
            skip_deleted: If True, items marked for deletion will be skipped.

        Returns:
            True if the item was valid and selected (signaling to stop searching).
            False if the item was skipped or invalid (signaling to continue searching).
        """
        active_view = self._get_active_file_view()
        if not self._is_valid_image_item(
            candidate_idx
        ) or self._is_row_hidden_in_tree_if_applicable(active_view, candidate_idx):
            return False

        source_idx = self.proxy_model.mapToSource(candidate_idx)
        item = self.file_system_model.itemFromIndex(source_idx)
        item_data = item.data(Qt.ItemDataRole.UserRole) if item else None
        path = item_data.get("path") if isinstance(item_data, dict) else None

        logger.info(f"Navigate {direction}: Checking candidate item - Path: {path}")

        if skip_deleted and path and self._is_marked_for_deletion(path):
            logger.debug(
                f"Navigate {direction}: Skipping deleted item: {os.path.basename(path)}"
            )
            return False

        if skip_deleted:
            logger.debug(
                f"Navigate {direction}: Found valid item: {os.path.basename(path) if path else 'Unknown'}"
            )
        else:
            logger.debug(
                f"Navigate {direction} (bypass deleted): Moving to: {os.path.basename(path) if path else 'Unknown'}"
            )

        active_view.setCurrentIndex(candidate_idx)
        active_view.scrollTo(candidate_idx, QAbstractItemView.ScrollHint.EnsureVisible)
        if item:
            logger.debug(f"Navigated {direction} to: {item.text()}")

        return True

    def _navigate_left_in_group(self, skip_deleted=True):
        active_view = self._get_active_file_view()
        if not active_view:
            return
        current_proxy_idx = active_view.currentIndex()
        if not current_proxy_idx.isValid() or not self._is_valid_image_item(
            current_proxy_idx
        ):
            first_item = self._find_first_visible_item()
            if first_item.isValid():
                active_view.setCurrentIndex(first_item)
            return

        _parent_group_idx, group_images, local_idx = (
            self._get_current_group_sibling_images(current_proxy_idx)
        )

        if not group_images or local_idx == -1:
            logger.debug("Navigate left: No sibling images found in the current group.")
            return

        num_items = len(group_images)
        if num_items == 0:
            return

        # If not skipping deleted items, just move to the next item directly.
        if not skip_deleted:
            candidate_local_idx = (local_idx - 1 + num_items) % num_items
            candidate_idx = group_images[candidate_local_idx]
            self._validate_and_select_image_candidate(candidate_idx, "left", False)
            return

        # If skipping, iterate backwards from the current position to find the next valid item.
        # This loop runs at most `num_items` times. The complexity is linear (O(k)) with respect to group size,
        # as the inner validation is a fast O(1) operation.
        for i in range(1, num_items + 1):
            # Calculate the index of the candidate item, moving circularly backwards.
            candidate_local_idx = (local_idx - i + num_items) % num_items
            candidate_idx = group_images[candidate_local_idx]

            # Attempt to select the candidate. If it's valid, the function returns True.
            if self._validate_and_select_image_candidate(candidate_idx, "left", True):
                return  # Found a valid item, so we exit.

        logger.debug("Navigate left: All items in group are marked for deletion.")

    def _navigate_right_in_group(self, skip_deleted=True):
        active_view = self._get_active_file_view()
        if not active_view:
            return
        current_proxy_idx = active_view.currentIndex()
        if not current_proxy_idx.isValid() or not self._is_valid_image_item(
            current_proxy_idx
        ):
            first_item = self._find_first_visible_item()
            if first_item.isValid():
                active_view.setCurrentIndex(first_item)
            return

        _parent_group_idx, group_images, local_idx = (
            self._get_current_group_sibling_images(current_proxy_idx)
        )

        if not group_images or local_idx == -1:
            logger.debug(
                "Navigate right: No sibling images found in the current group."
            )
            return

        num_items = len(group_images)
        if num_items == 0:
            return

        if not skip_deleted:
            candidate_local_idx = (local_idx + 1) % num_items
            candidate_idx = group_images[candidate_local_idx]
            self._validate_and_select_image_candidate(candidate_idx, "right", False)
            return

        # If skipping, iterate forwards from the current position to find the next valid item.
        # The complexity is linear (O(k)) with respect to group size.
        for i in range(1, num_items + 1):
            candidate_local_idx = (local_idx + i) % num_items
            candidate_idx = group_images[candidate_local_idx]

            if self._validate_and_select_image_candidate(candidate_idx, "right", True):
                return

        logger.debug("Navigate right: All items in group are marked for deletion.")

    def _navigate_up_sequential(self, skip_deleted=True):
        active_view = self._get_active_file_view()
        if not active_view:
            logger.debug("Navigate up: No active view found.")
            return

        current_proxy_idx = active_view.currentIndex()

        if not current_proxy_idx.isValid():
            last_item_index = self._find_last_visible_item()
            if last_item_index.isValid():
                active_view.setCurrentIndex(last_item_index)
                active_view.scrollTo(
                    last_item_index, QAbstractItemView.ScrollHint.EnsureVisible
                )
            return

        iter_idx = current_proxy_idx

        max_iterations = (
            self.proxy_model.rowCount(QModelIndex())
            + sum(
                self.proxy_model.rowCount(self.proxy_model.index(r, 0, QModelIndex()))
                for r in range(self.proxy_model.rowCount(QModelIndex()))
            )
        ) * 2
        if max_iterations == 0 and self.app_state and self.app_state.image_files_data:
            max_iterations = len(self.app_state.image_files_data) * 5
        if max_iterations == 0:
            max_iterations = DEFAULT_MAX_ITERATIONS

        for iteration_count in range(max_iterations):
            prev_visual_idx = active_view.indexAbove(iter_idx)

            if not prev_visual_idx.isValid():
                break

            if self._validate_and_select_image_candidate(
                prev_visual_idx, "up", skip_deleted
            ):
                return

            if isinstance(active_view, QTreeView) and self._is_expanded_group_header(
                active_view, prev_visual_idx
            ):
                if iter_idx.parent() != prev_visual_idx:
                    last_in_group = self._find_last_visible_image_item_in_subtree(
                        prev_visual_idx, skip_deleted=skip_deleted
                    )
                    # Validate the item before selecting it
                    if (
                        last_in_group.isValid()
                        and self._validate_and_select_image_candidate(
                            last_in_group, "up", skip_deleted
                        )
                    ):
                        return

            iter_idx = prev_visual_idx
            if iteration_count == max_iterations - 1:  # Safety break
                logger.warning("Navigate up: Max iterations reached, aborting.")

        logger.debug("Navigate up: No previous image found.")

    def _navigate_down_sequential(self, skip_deleted=True):
        active_view = self._get_active_file_view()
        if not active_view:
            logger.debug("Navigate down: No active view found.")
            return

        current_index = active_view.currentIndex()
        if not current_index.isValid():
            first_item_index = self._find_first_visible_item()
            if first_item_index.isValid():
                active_view.setCurrentIndex(first_item_index)
                active_view.scrollTo(
                    first_item_index, QAbstractItemView.ScrollHint.EnsureVisible
                )
            return

        temp_index = current_index
        iteration_count = 0

        # Determine a safe iteration limit to prevent infinite loops in unexpected scenarios
        safety_iteration_limit = (
            self.proxy_model.rowCount(QModelIndex())
            * DEFAULT_SAFETY_ITERATION_MULTIPLIER
        )
        if self.app_state.image_files_data:
            safety_iteration_limit = max(
                safety_iteration_limit,
                len(self.app_state.image_files_data)
                * DEFAULT_SAFETY_ITERATION_MULTIPLIER,
            )
        if safety_iteration_limit == 0:
            safety_iteration_limit = DEFAULT_MAX_ITERATIONS

        while temp_index.isValid() and iteration_count < safety_iteration_limit:
            iteration_count += 1
            # 1. Get the item visually below the current one
            temp_index = active_view.indexBelow(temp_index)

            if not temp_index.isValid():
                break

            if self._validate_and_select_image_candidate(
                temp_index, "down", skip_deleted
            ):
                return

        if iteration_count >= safety_iteration_limit:
            logger.warning("Navigate down: Max iterations reached, aborting.")

        logger.debug("Navigate down: No next image found.")

    def _find_first_visible_item(self) -> QModelIndex:
        active_view = self._get_active_file_view()
        if not active_view:
            return QModelIndex()

        proxy_model = active_view.model()
        if not isinstance(proxy_model, QSortFilterProxyModel):
            return QModelIndex()

        root_proxy_index = QModelIndex()
        proxy_row_count = proxy_model.rowCount(root_proxy_index)
        if isinstance(active_view, QTreeView):
            q = [
                proxy_model.index(r, 0, root_proxy_index)
                for r in range(proxy_row_count)
            ]

            head = 0
            while head < len(q):
                current_proxy_idx = q[head]
                head += 1
                if not current_proxy_idx.isValid():
                    continue

                if not active_view.isRowHidden(
                    current_proxy_idx.row(), current_proxy_idx.parent()
                ):
                    if self._is_valid_image_item(current_proxy_idx):
                        return current_proxy_idx
                    source_idx_for_children_check = proxy_model.mapToSource(
                        current_proxy_idx
                    )
                    item_for_children_check = None
                    if source_idx_for_children_check.isValid():
                        item_for_children_check = (
                            proxy_model.sourceModel().itemFromIndex(
                                source_idx_for_children_check
                            )
                        )
                    if (
                        item_for_children_check
                        and proxy_model.hasChildren(current_proxy_idx)
                        and active_view.isExpanded(current_proxy_idx)
                    ):
                        for child_row in range(proxy_model.rowCount(current_proxy_idx)):
                            q.append(proxy_model.index(child_row, 0, current_proxy_idx))
            return QModelIndex()
        elif isinstance(active_view, QListView):
            for r in range(proxy_row_count):
                proxy_idx = proxy_model.index(r, 0, root_proxy_index)
                if self._is_valid_image_item(proxy_idx):
                    return proxy_idx
            return QModelIndex()
        return QModelIndex()

    def _find_last_visible_item(self) -> QModelIndex:
        active_view = self._get_active_file_view()
        if not active_view:
            return QModelIndex()
        proxy_model = active_view.model()
        if not isinstance(proxy_model, QSortFilterProxyModel):
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
            q = [
                proxy_model.index(r, 0, root_proxy_index)
                for r in range(proxy_model.rowCount(root_proxy_index))
            ]
            head = 0
            while head < len(q):
                current_proxy_idx = q[head]
                head += 1
                if not current_proxy_idx.isValid():
                    continue

                if not active_view.isRowHidden(
                    current_proxy_idx.row(), current_proxy_idx.parent()
                ):
                    if self._is_valid_image_item(current_proxy_idx):
                        last_found_valid_image = (
                            current_proxy_idx  # Update if this one is valid
                        )

                    source_idx_for_children_check = proxy_model.mapToSource(
                        current_proxy_idx
                    )
                    item_for_children_check = None
                    if source_idx_for_children_check.isValid():
                        item_for_children_check = (
                            proxy_model.sourceModel().itemFromIndex(
                                source_idx_for_children_check
                            )
                        )

                    if (
                        item_for_children_check
                        and proxy_model.hasChildren(current_proxy_idx)
                        and active_view.isExpanded(current_proxy_idx)
                    ):
                        for child_row in range(proxy_model.rowCount(current_proxy_idx)):
                            q.append(proxy_model.index(child_row, 0, current_proxy_idx))
            return last_found_valid_image

        elif isinstance(active_view, QListView):
            for r in range(
                proxy_model.rowCount(root_proxy_index) - 1, -1, -1
            ):  # Iterate backwards
                proxy_idx = proxy_model.index(r, 0, root_proxy_index)
                if self._is_valid_image_item(proxy_idx):
                    return proxy_idx
            logger.debug("Find last item (List): No visible image item found.")
            return QModelIndex()

        logger.debug("Find last item: Unknown view type or scenario.")
        return QModelIndex()

    def _get_all_visible_image_paths(self) -> List[str]:
        """Gets an ordered list of file paths for all currently visible image items."""
        paths = []
        active_view = self._get_active_file_view()
        if not active_view:
            return paths

        proxy_model = active_view.model()
        # Ensure model is a QSortFilterProxyModel, as it holds the filtered/sorted view
        if not isinstance(proxy_model, QSortFilterProxyModel):
            logger.warning(
                "Cannot get visible paths: Active view's model is not a QSortFilterProxyModel."
            )
            return paths

        # Traversal logic needs to handle both QTreeView (hierarchical) and QListView (flat)
        # We build a queue of proxy indices to visit in display order.
        queue = []
        root_proxy_parent_idx = (
            QModelIndex()
        )  # Parent for top-level items in the proxy model

        for r in range(proxy_model.rowCount(root_proxy_parent_idx)):
            queue.append(proxy_model.index(r, 0, root_proxy_parent_idx))

        head = 0
        while head < len(queue):
            current_proxy_idx = queue[head]
            head += 1
            if not current_proxy_idx.isValid():
                continue

            # Check if the item itself is a valid image item
            if self._is_valid_image_item(current_proxy_idx):
                source_idx = proxy_model.mapToSource(current_proxy_idx)
                item = self.file_system_model.itemFromIndex(
                    source_idx
                )  # Use source_model here
                if item:
                    item_data = item.data(Qt.ItemDataRole.UserRole)
                    if isinstance(item_data, dict) and "path" in item_data:
                        paths.append(item_data["path"])

            # If it's a QTreeView and the current item is expanded and has children, add them to the queue
            if isinstance(active_view, QTreeView):
                # We need to check against the source item for hasChildren, but expansion against proxy index
                source_idx_for_children_check = proxy_model.mapToSource(
                    current_proxy_idx
                )
                # Ensure source_idx is valid before using it with source model
                if source_idx_for_children_check.isValid():
                    item_for_children_check = self.file_system_model.itemFromIndex(
                        source_idx_for_children_check
                    )
                    if (
                        item_for_children_check
                        and item_for_children_check.hasChildren()
                        and active_view.isExpanded(current_proxy_idx)
                    ):
                        for child_row in range(
                            proxy_model.rowCount(current_proxy_idx)
                        ):  # Children from proxy model
                            queue.append(
                                proxy_model.index(child_row, 0, current_proxy_idx)
                            )
        return paths

    def _find_proxy_index_for_path(self, target_path: str) -> QModelIndex:
        """Finds the QModelIndex in the current proxy model for a given file path."""
        active_view = self._get_active_file_view()
        if not active_view:
            return QModelIndex()

        proxy_model = active_view.model()
        if not isinstance(proxy_model, QSortFilterProxyModel):
            logger.warning(
                "Cannot find proxy index: Active view's model is not a QSortFilterProxyModel."
            )
            return QModelIndex()

        # Similar traversal as _get_all_visible_image_paths
        queue = []
        root_proxy_parent_idx = QModelIndex()
        for r in range(proxy_model.rowCount(root_proxy_parent_idx)):
            queue.append(proxy_model.index(r, 0, root_proxy_parent_idx))

        head = 0
        while head < len(queue):
            current_proxy_idx = queue[head]
            head += 1
            if not current_proxy_idx.isValid():
                continue

            if self._is_valid_image_item(current_proxy_idx):
                source_idx = proxy_model.mapToSource(current_proxy_idx)
                item = self.file_system_model.itemFromIndex(
                    source_idx
                )  # Use source_model
                if item:
                    item_data = item.data(Qt.ItemDataRole.UserRole)
                    if (
                        isinstance(item_data, dict)
                        and item_data.get("path") == target_path
                    ):
                        return current_proxy_idx  # Found it

            if isinstance(active_view, QTreeView):
                source_idx_for_children_check = proxy_model.mapToSource(
                    current_proxy_idx
                )
                if source_idx_for_children_check.isValid():
                    item_for_children_check = self.file_system_model.itemFromIndex(
                        source_idx_for_children_check
                    )
                    if (
                        item_for_children_check
                        and item_for_children_check.hasChildren()
                        and active_view.isExpanded(current_proxy_idx)
                    ):
                        for child_row in range(
                            proxy_model.rowCount(current_proxy_idx)
                        ):  # Children from proxy model
                            queue.append(
                                proxy_model.index(child_row, 0, current_proxy_idx)
                            )
        return QModelIndex()  # Not found

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
                        if (
                            isinstance(item_user_data, dict)
                            and "path" in item_user_data
                        ):
                            file_path = item_user_data["path"]
                            if os.path.isfile(file_path):
                                if file_path not in selected_file_paths:
                                    selected_file_paths.append(file_path)
        return selected_file_paths

    def _get_cached_metadata_for_selection(
        self, file_path: str
    ) -> Optional[Dict[str, Any]]:
        """Gets metadata from AppState caches. Assumes caches are populated by RatingLoaderWorker."""
        if not os.path.isfile(file_path):
            logger.warning(
                f"[_get_cached_metadata_for_selection] File not found: {file_path}"
            )
            return None

        # Data should have been populated by RatingLoaderWorker into AppState caches
        # os.path.normpath is important for cache key consistency.
        # RatingLoaderWorker stores with normalized paths.
        normalized_path = os.path.normpath(file_path)

        current_rating = self.app_state.rating_cache.get(normalized_path, 0)
        current_date = self.app_state.date_cache.get(normalized_path)

        return {"rating": current_rating, "date": current_date}

    def _display_single_image_preview(
        self, file_path: str, file_data_from_model: Optional[Dict[str, Any]]
    ):
        """Handles displaying preview and info for a single selected image."""
        logger.debug(f"Displaying single image preview: {os.path.basename(file_path)}")
        if not os.path.exists(file_path):
            self.advanced_image_viewer.clear()
            self.statusBar().showMessage(
                f"Error: File not found - {os.path.basename(file_path)}", 5000
            )
            return

        metadata = self._get_cached_metadata_for_selection(file_path)
        if not metadata:
            self.advanced_image_viewer.setText("Metadata unavailable")
            self.statusBar().showMessage(
                f"Error accessing metadata: {os.path.basename(file_path)}", 5000
            )
            return

        pixmap = self.image_pipeline.get_preview_qpixmap(
            file_path,
            display_max_size=(8000, 8000),
            apply_auto_edits=self.apply_auto_edits_enabled,
        )

        if not pixmap or pixmap.isNull():
            pixmap = self.image_pipeline.get_thumbnail_qpixmap(
                file_path, apply_auto_edits=self.apply_auto_edits_enabled
            )

        if pixmap and not pixmap.isNull():
            image_data = {
                "pixmap": pixmap,
                "path": file_path,
                "rating": metadata.get("rating", 0),
            }
            self.advanced_image_viewer.set_image_data(image_data)
            self._update_status_bar_for_image(
                file_path, metadata, pixmap, file_data_from_model
            )
        else:
            self.advanced_image_viewer.setText("Failed to load image")
            self.statusBar().showMessage(
                f"Error: Could not load image data for {os.path.basename(file_path)}",
                7000,
            )

        if self.sidebar_visible:
            self._update_sidebar_with_current_selection()

    def _display_rotated_image_preview(
        self,
        file_path: str,
        file_data_from_model: Optional[Dict[str, Any]],
        preserve_side_by_side: bool,
    ):
        """Handles displaying preview after rotation, preserving view mode."""
        logger.debug(f"Displaying rotated image preview: {os.path.basename(file_path)}")
        if not os.path.exists(file_path):
            self.advanced_image_viewer.clear()
            self.statusBar().showMessage(
                f"Error: File not found - {os.path.basename(file_path)}", 5000
            )
            return

        if preserve_side_by_side:
            # In side-by-side mode, we need to refresh the entire view to show the updated image
            # Get all currently selected paths to rebuild the side-by-side view
            selected_paths = self._get_selected_file_paths_from_view()
            if len(selected_paths) >= 2:
                # Refresh the entire side-by-side view with updated images
                self._display_multi_selection_info(selected_paths)
                return
            else:
                # Fallback to single image if selection changed
                preserve_side_by_side = False

        # Single image mode or fallback
        metadata = self._get_cached_metadata_for_selection(file_path)
        if not metadata:
            self.advanced_image_viewer.setText("Metadata unavailable")
            self.statusBar().showMessage(
                f"Error accessing metadata: {os.path.basename(file_path)}", 5000
            )
            return

        pixmap = self.image_pipeline.get_preview_qpixmap(
            file_path,
            display_max_size=(8000, 8000),
            apply_auto_edits=self.apply_auto_edits_enabled,
        )

        if not pixmap or pixmap.isNull():
            pixmap = self.image_pipeline.get_thumbnail_qpixmap(
                file_path, apply_auto_edits=self.apply_auto_edits_enabled
            )

        if pixmap and not pixmap.isNull():
            image_data = {
                "pixmap": pixmap,
                "path": file_path,
                "rating": metadata.get("rating", 0),
            }
            self.advanced_image_viewer.set_image_data(
                image_data, preserve_view_mode=preserve_side_by_side
            )
            self._update_status_bar_for_image(
                file_path, metadata, pixmap, file_data_from_model
            )
        else:
            self.advanced_image_viewer.setText("Failed to load image")
            self.statusBar().showMessage(
                f"Error: Could not load image data for {os.path.basename(file_path)}",
                7000,
            )

        if self.sidebar_visible:
            self._update_sidebar_with_current_selection()

    def _update_status_bar_for_image(
        self, file_path, metadata, pixmap, file_data_from_model
    ):
        """Helper to compose and set the status bar message for an image."""
        filename = os.path.basename(file_path)
        rating_text = f"R: {metadata.get('rating', 0)}"
        date_obj = metadata.get("date")
        date_text = f"D: {date_obj.strftime('%Y-%m-%d')}" if date_obj else "D: Unknown"
        cluster = self.app_state.cluster_results.get(file_path)
        cluster_text = f" | C: {cluster}" if cluster is not None else ""
        try:
            size_text = f" | Size: {os.path.getsize(file_path) // 1024} KB"
        except OSError:
            size_text = " | Size: N/A"
        dimensions_text = f" | {pixmap.width()}x{pixmap.height()}"
        is_blurred = (
            file_data_from_model.get("is_blurred") if file_data_from_model else None
        )
        blur_status_text = (
            " | Blurred: Yes"
            if is_blurred is True
            else (" | Blurred: No" if is_blurred is False else "")
        )

        status_message = f"{filename} | {rating_text} | {date_text}{cluster_text}{size_text}{dimensions_text}{blur_status_text}"
        self.statusBar().showMessage(status_message)

    def _display_multi_selection_info(self, selected_paths: List[str]):
        """Handles UI updates when multiple images are selected."""
        logger.debug(f"Displaying {len(selected_paths)} images side-by-side.")
        if not selected_paths:
            self.advanced_image_viewer.clear()
            self.statusBar().showMessage("No items selected.")
            return

        images_data_for_viewer = []
        metadata_for_sidebar = []

        for path in selected_paths:
            pixmap = self.image_pipeline.get_preview_qpixmap(
                path,
                display_max_size=(8000, 8000),
                apply_auto_edits=self.apply_auto_edits_enabled,
            )
            if not pixmap or pixmap.isNull():
                pixmap = self.image_pipeline.get_thumbnail_qpixmap(
                    path, apply_auto_edits=self.apply_auto_edits_enabled
                )

            if pixmap:
                basic_metadata = self._get_cached_metadata_for_selection(path)
                raw_exif = MetadataProcessor.get_detailed_metadata(
                    path, self.app_state.exif_disk_cache
                )

                images_data_for_viewer.append(
                    {
                        "pixmap": pixmap,
                        "path": path,
                        "rating": basic_metadata.get("rating", 0)
                        if basic_metadata
                        else 0,
                        "label": basic_metadata.get("label")
                        if basic_metadata
                        else None,
                    }
                )

                combined_meta = (basic_metadata or {}).copy()
                combined_meta["raw_exif"] = (raw_exif or {}).copy()
                metadata_for_sidebar.append(combined_meta)

        if images_data_for_viewer:
            self.advanced_image_viewer.set_images_data(images_data_for_viewer)

            if self.sidebar_visible:
                if len(images_data_for_viewer) >= 2:
                    self.metadata_sidebar.update_comparison(
                        [d["path"] for d in images_data_for_viewer],
                        metadata_for_sidebar,
                    )
                else:
                    # Fallback to single selection view if something goes wrong
                    self._update_sidebar_with_current_selection()

            if len(images_data_for_viewer) >= 2:
                # Show similarity for the first two images in a multi-selection
                path1, path2 = (
                    images_data_for_viewer[0]["path"],
                    images_data_for_viewer[1]["path"],
                )
                emb1, emb2 = (
                    self.app_state.embeddings_cache.get(path1),
                    self.app_state.embeddings_cache.get(path2),
                )
                if emb1 is not None and emb2 is not None:
                    try:
                        similarity = cosine_similarity([emb1], [emb2])[0][0]
                        self.statusBar().showMessage(
                            f"Comparing {len(images_data_for_viewer)} images. Similarity (first 2): {similarity:.4f}"
                        )
                    except Exception as e:
                        logger.error(f"Error calculating similarity: {e}")
                else:
                    self.statusBar().showMessage(
                        f"Comparing {len(images_data_for_viewer)} images."
                    )
            else:
                self.statusBar().showMessage(
                    f"Displaying {len(images_data_for_viewer)} image(s)."
                )
        else:
            self.statusBar().showMessage(
                "Could not load any selected images for display.", 3000
            )
            self.advanced_image_viewer.clear()
            if self.sidebar_visible:
                self.metadata_sidebar.show_placeholder()

    def _handle_no_selection_or_non_image(self):
        """Handles UI updates when no valid image is selected."""
        if not self.app_state.image_files_data:
            return

        # Clear focused image path and repaint view to remove underline
        if self.app_state.focused_image_path:
            self.app_state.focused_image_path = None
            self._get_active_file_view().viewport().update()

        self.advanced_image_viewer.clear()
        self.advanced_image_viewer.setText("Select an image to view details.")
        self.statusBar().showMessage("Ready")

    def _handle_file_selection_changed(
        self,
        selected=None,
        deselected=None,
        override_selected_paths: Optional[List[str]] = None,
    ):
        # If the user is typing in the search bar, filtering the model can
        # trigger selection changes. We want to ignore these automated changes
        # to prevent the image preview from updating and stealing focus from the search bar.
        if self.left_panel.search_input.hasFocus():
            logger.info(
                "Search input is active, skipping selection change handler to maintain focus."
            )
            return

        if self._is_syncing_selection and override_selected_paths is None:
            return

        if override_selected_paths is not None:
            selected_file_paths = override_selected_paths
            logger.debug(
                f"_handle_file_selection_changed: Using overridden selection of {len(selected_file_paths)} paths."
            )
        else:
            selected_file_paths = self._get_selected_file_paths_from_view()

        if not self.app_state.image_files_data:
            return

        # In rotation view, update the accept/refuse buttons based on selection
        if self.left_panel.current_view_mode == "rotation":
            num_suggestions = len(self.rotation_suggestions)
            self.accept_all_button.setVisible(num_suggestions > 1)
            self.refuse_all_button.setVisible(num_suggestions > 1)
            num_selected = len(selected_file_paths)

            self.accept_button.setVisible(num_selected > 0)
            self.refuse_button.setVisible(num_selected > 0)

            if num_selected > 0:
                all_selected_have_suggestion = all(
                    p in self.rotation_suggestions for p in selected_file_paths
                )
                self.accept_button.setEnabled(all_selected_have_suggestion)
                self.refuse_button.setEnabled(all_selected_have_suggestion)

                if num_selected == 1:
                    self.accept_button.setText("Accept (Y)")
                    self.refuse_button.setText("Refuse (N)")
                    self._display_side_by_side_comparison(selected_file_paths[0])
                else:
                    self.accept_button.setText(f"Accept ({num_selected})")
                    self.refuse_button.setText(f"Refuse ({num_selected})")
                    self.advanced_image_viewer.clear()
                    self.advanced_image_viewer.setText(
                        f"{num_selected} items selected for rotation approval."
                    )
            else:
                self.advanced_image_viewer.clear()
            return
        else:
            self.accept_all_button.setVisible(False)
            self.accept_button.setVisible(False)
            self.refuse_button.setVisible(False)
            self.refuse_all_button.setVisible(False)

        # When selection changes, clear the focused image path unless it's a single selection
        if len(selected_file_paths) != 1:
            if self.app_state.focused_image_path:
                self.app_state.focused_image_path = None
                active_view = self._get_active_file_view()
                if active_view:
                    active_view.viewport().update()  # Trigger repaint to remove underline

        if len(selected_file_paths) == 1:
            file_path = selected_file_paths[0]
            # This is a single selection, so it's also the "focused" image.
            self.app_state.focused_image_path = file_path
            active_view = self._get_active_file_view()
            if active_view:
                active_view.viewport().update()

            file_data_from_model = self._get_cached_metadata_for_selection(file_path)
            # This will force the viewer into single-view mode.
            self._display_single_image_preview(file_path, file_data_from_model)

        elif len(selected_file_paths) >= 2:
            # This will force the viewer into side-by-side mode.
            self._display_multi_selection_info(selected_file_paths)

        else:  # No selection
            self._handle_no_selection_or_non_image()
            if self.sidebar_visible and self.metadata_sidebar:
                self.metadata_sidebar.show_placeholder()

    def _apply_filter(self):
        # Guard: Don't apply filters if no images are loaded yet
        if not self.app_state.image_files_data:
            logger.debug("Filter skipped: No images loaded.")
            return

        search_text = self.left_panel.search_input.text()
        logger.info(f"Applying filters. Search term: '{search_text}'")
        search_text = search_text.lower()
        selected_filter_text = self.filter_combo.currentText()
        selected_cluster_text = self.cluster_filter_combo.currentText()
        target_cluster_id = -1
        if (
            self.cluster_filter_combo.isEnabled()
            and selected_cluster_text != "All Clusters"
        ):
            try:
                target_cluster_id = int(selected_cluster_text.split(" ")[-1])
            except ValueError:
                pass

        self.proxy_model.app_state_ref = self.app_state
        self.proxy_model.current_rating_filter = selected_filter_text
        self.proxy_model.current_cluster_filter_id = target_cluster_id
        self.proxy_model.show_folders_mode_ref = self.show_folders_mode
        self.proxy_model.current_view_mode_ref = self.left_panel.current_view_mode

        # Set the search text filter
        self.proxy_model.setFilterRegularExpression(search_text)
        self.proxy_model.setFilterKeyColumn(-1)  # Search all columns
        self.proxy_model.setFilterRole(
            Qt.ItemDataRole.DisplayRole
        )  # ← Changed from UserRole to DisplayRole

        self.proxy_model.invalidateFilter()

    def _start_preview_preloader(self, image_data_list: List[Dict[str, any]]):
        logger.info(
            f"<<< ENTRY >>> _start_preview_preloader called with {len(image_data_list)} items."
        )
        if not image_data_list:
            logger.info(
                "_start_preview_preloader: image_data_list is empty. Hiding overlay."
            )
            self.hide_loading_overlay()
            return

        paths_for_preloader = [
            fd["path"]
            for fd in image_data_list
            if fd and isinstance(fd, dict) and "path" in fd
        ]
        logger.info(
            f"_start_preview_preloader: Extracted {len(paths_for_preloader)} paths for preloader."
        )

        if not paths_for_preloader:
            logger.info(
                "_start_preview_preloader: No valid paths_for_preloader. Hiding overlay."
            )
            self.hide_loading_overlay()
            return

        self.update_loading_text(
            f"Preloading previews ({len(paths_for_preloader)} images)..."
        )
        logger.info(
            f"_start_preview_preloader: Calling worker_manager.start_preview_preload for {len(paths_for_preloader)} paths."
        )
        try:
            logger.info(
                f"_start_preview_preloader: --- CALLING --- worker_manager.start_preview_preload for {len(paths_for_preloader)} paths."
            )
            self.worker_manager.start_preview_preload(
                paths_for_preloader, self.apply_auto_edits_enabled
            )
            logger.info(
                "_start_preview_preloader: --- RETURNED --- worker_manager.start_preview_preload call successful."
            )
        except Exception as e_preview_preload:
            logger.error(
                f"_start_preview_preloader: Error calling worker_manager.start_preview_preload: {e_preview_preload}",
                exc_info=True,
            )
            self.hide_loading_overlay()  # Ensure overlay is hidden on error
        logger.info("<<< EXIT >>> _start_preview_preloader.")

    # Slot for WorkerManager's file_scan_thumbnail_preload_finished signal
    # This signal is now deprecated in favor of chaining after rating load.
    # Keeping the method signature for now in case it's used elsewhere, but logic is changed.
    def _handle_thumbnail_preload_finished(self, all_file_data: List[Dict[str, any]]):
        # This was previously used to kick off preview preloading.
        # Now, preview preloading is kicked off after rating loading finishes.
        # self.update_loading_text("Thumbnails preloaded. Starting preview preloading...")
        # self._start_preview_preloader(all_file_data)
        logger.debug("Thumbnail preload finished (now a deprecated signal).")
        pass  # Intentionally do nothing here, preview starts after rating load now

    # --- Rating Loader Worker Handlers ---
    def _handle_rating_load_progress(self, current: int, total: int, basename: str):
        percentage = int((current / total) * 100) if total > 0 else 0
        logger.debug(
            f"Rating load progress: {percentage}% ({current}/{total}) - {basename}"
        )
        self.update_loading_text(
            f"Loading ratings: {percentage}% ({current}/{total}) - {basename}"
        )

    def _handle_metadata_batch_loaded(
        self, metadata_batch: List[Tuple[str, Dict[str, Any]]]
    ):
        logger.debug(f"Metadata batch loaded with {len(metadata_batch)} items.")

        currently_selected_paths = self._get_selected_file_paths_from_view()

        needs_active_selection_refresh = False
        for image_path, metadata in metadata_batch:
            if not metadata:
                continue

            logger.debug(
                f"Processing metadata from batch for {os.path.basename(image_path)}: {metadata}"
            )

            # Update any visible viewer showing this image
            for viewer in self.advanced_image_viewer.image_viewers:
                if viewer.isVisible() and viewer._file_path == image_path:
                    logger.debug(f"Updating viewer for {os.path.basename(image_path)}.")
                    viewer.update_rating_display(metadata.get("rating", 0))

            # Check if the processed image is part of the current selection
            if image_path in currently_selected_paths:
                logger.debug(
                    f"Batch contains a selected item: {os.path.basename(image_path)}. Marking for UI refresh."
                )
                needs_active_selection_refresh = True

        if needs_active_selection_refresh:
            logger.debug(
                "Triggering _handle_file_selection_changed after processing batch due to active item update."
            )
            self._handle_file_selection_changed()

        # After a batch, it's good practice to re-apply the filter in case ratings changed
        self._apply_filter()

    def _handle_rating_load_finished(self):
        logger.info(
            "_handle_rating_load_finished: Received RatingLoaderWorker.finished signal."
        )
        self.statusBar().showMessage("Background rating loading finished.", 3000)

        if not self.app_state.image_files_data:
            logger.info(
                "_handle_rating_load_finished: No image files data found in app_state. Hiding loading overlay."
            )
            self.hide_loading_overlay()
            return

        logger.info(
            "_handle_rating_load_finished: image_files_data found. Preparing to start preview preloader."
        )
        self.update_loading_text("Ratings loaded. Preloading previews...")
        try:
            logger.info(
                "_handle_rating_load_finished: --- CALLING --- _start_preview_preloader."
            )
            self._start_preview_preloader(
                self.app_state.image_files_data.copy()
            )  # Pass a copy
            logger.info(
                "_handle_rating_load_finished: --- RETURNED --- _start_preview_preloader call completed."
            )
        except Exception as e_start_preview:
            logger.error(
                f"_handle_rating_load_finished: Error calling _start_preview_preloader: {e_start_preview}",
                exc_info=True,
            )
            self.hide_loading_overlay()  # Ensure overlay is hidden on error
        logger.info("<<< EXIT >>> _handle_rating_load_finished.")

    def _handle_rating_load_error(self, message: str):
        logger.error(f"Rating Load Error: {message}")
        self.statusBar().showMessage(f"Rating Load Error: {message}", 5000)
        # Still proceed to preview preloading even if rating load had errors for some files
        if self.app_state.image_files_data:
            self.update_loading_text("Rating load errors. Preloading previews...")
            self._start_preview_preloader(
                self.app_state.image_files_data.copy()
            )  # Pass a copy
        else:
            self.hide_loading_overlay()

    # Slot for WorkerManager's preview_preload_progress signal
    def _handle_preview_progress(self, percentage: int, message: str):
        logger.debug(
            f"<<< ENTRY >>> _handle_preview_progress: {percentage}% - {message}"
        )
        self.update_loading_text(message)
        logger.debug("<<< EXIT >>> _handle_preview_progress.")

    # Slot for WorkerManager's preview_preload_finished signal
    def _handle_preview_finished(self):
        logger.debug(
            "<<< ENTRY >>> _handle_preview_finished: Received PreviewPreloaderWorker.finished signal."
        )
        auto_edits_status = "enabled" if self.apply_auto_edits_enabled else "disabled"
        self.statusBar().showMessage(
            f"Previews regenerated with Auto RAW edits {auto_edits_status}.", 5000
        )
        self.hide_loading_overlay()
        logger.debug("_handle_preview_finished: Loading overlay hidden.")

        # Log final cache vs image size
        if self.app_state.current_folder_path:
            total_image_size_bytes = self._calculate_folder_image_size(
                self.app_state.current_folder_path
            )
            preview_cache_size_bytes = self.image_pipeline.preview_cache.volume()
            logger.info("--- Cache vs. Image Size Diagnostics (Post-Preload) ---")
            logger.info(
                f"Total Original Image Size: {total_image_size_bytes / (1024 * 1024):.2f} MB"
            )
            logger.info(
                f"Final Preview Cache Size: {preview_cache_size_bytes / (1024 * 1024):.2f} MB"
            )
            if total_image_size_bytes > 0:
                ratio = (preview_cache_size_bytes / total_image_size_bytes) * 100
                logger.info(f"Cache-to-Image Size Ratio: {ratio:.2f}%")
            logger.info("---------------------------------------------------------")

        self._update_image_info_label()  # Update UI with final cache size

        # WorkerManager handles thread cleanup
        logger.info("<<< EXIT >>> _handle_preview_finished.")

    # Slot for WorkerManager's preview_preload_error signal
    def _handle_preview_error(self, message: str):
        logger.info(f"<<< ENTRY >>> _handle_preview_error: {message}")
        logger.error(f"Preview Preload Error: {message}")
        self.statusBar().showMessage(f"Preview Preload Error: {message}", 5000)
        self.hide_loading_overlay()
        # WorkerManager handles thread cleanup
        logger.info("<<< EXIT >>> _handle_preview_error.")

    def _toggle_folder_visibility(self, checked: bool):
        self.show_folders_mode = checked
        self._rebuild_model_view()

        if self.left_panel.current_view_mode == "list":
            self.left_panel.set_view_mode_list()
        elif self.left_panel.current_view_mode == "icons":
            self.left_panel.set_view_mode_icons()
        elif self.left_panel.current_view_mode == "date":
            self.left_panel.set_view_mode_date()

    def _toggle_group_by_similarity(self, checked: bool):
        # Always update the mode first, as it's needed by handle_clustering_complete
        self.group_by_similarity_mode = checked
        self.menu_manager.group_by_similarity_action.setChecked(
            checked
        )  # Keep the UI in sync

        if checked and not self.app_state.cluster_results:
            self.app_controller.start_similarity_analysis()
            # The view will be rebuilt by handle_clustering_complete once analysis is done
            return  # Exit here, as handle_clustering_complete will handle the rest

        # If not checking, or if already clustered, proceed to rebuild view
        if checked:  # Only show sort options if grouping is active
            self.menu_manager.cluster_sort_action.setVisible(True)
            self.cluster_sort_combo.setEnabled(True)
        else:
            self.menu_manager.cluster_sort_action.setVisible(False)
            self.cluster_sort_combo.setEnabled(False)

        # Rebuild view if not waiting for analysis, or if unchecking
        if self.left_panel.current_view_mode == "list":
            self.left_panel.set_view_mode_list()
        elif self.left_panel.current_view_mode == "icons":
            self.left_panel.set_view_mode_icons()
        elif self.left_panel.current_view_mode == "grid":
            self.left_panel.set_view_mode_grid()
        elif self.left_panel.current_view_mode == "date":
            self.left_panel.set_view_mode_date()
        else:
            self._rebuild_model_view()

    def _populate_model_by_date(
        self, parent_item: QStandardItem, image_data_list: List[Dict[str, any]]
    ):
        if not image_data_list:
            return

        images_by_year_month: Dict[any, Dict[int, List[Dict[str, any]]]] = {}
        unknown_date_key = "Unknown Date"

        for file_data in image_data_list:
            file_path = file_data["path"]
            img_date: date_obj | None = self.app_state.date_cache.get(file_path)
            year = img_date.year if img_date else unknown_date_key
            month = (
                img_date.month if img_date else 1
            )  # Default to 1 if unknown, for sorting

            if year not in images_by_year_month:
                images_by_year_month[year] = {}
            if month not in images_by_year_month[year]:
                images_by_year_month[year][month] = []
            images_by_year_month[year][month].append(file_data)

        sorted_years = sorted(
            images_by_year_month.keys(), key=lambda y: (y == unknown_date_key, y)
        )
        for year_val in sorted_years:
            year_item = QStandardItem(str(year_val))
            year_item.setEditable(False)
            year_item.setData(f"date_header_{year_val}", Qt.ItemDataRole.UserRole)
            font = year_item.font()
            font.setBold(True)
            year_item.setFont(font)
            parent_item.appendRow(year_item)

            sorted_months = sorted(images_by_year_month[year_val].keys())
            for month_val in sorted_months:
                parent_for_images = (
                    year_item  # Default to year item if month is unknown
                )
                if (
                    year_val != unknown_date_key
                ):  # Only create month sub-item if year is known
                    month_name = date_obj(1900, month_val, 1).strftime("%B")
                    month_item = QStandardItem(month_name)
                    month_item.setEditable(False)
                    month_item.setData(
                        f"date_header_{year_val}-{month_val}", Qt.ItemDataRole.UserRole
                    )
                    year_item.appendRow(month_item)
                    parent_for_images = month_item

                files_in_group_data = sorted(
                    images_by_year_month[year_val][month_val],
                    key=lambda fd: (
                        self.app_state.date_cache.get(fd["path"]) or date_obj.min,
                        os.path.basename(fd["path"]),
                    ),
                )
                for file_data in files_in_group_data:
                    image_item = self._create_standard_item(file_data)
                    parent_for_images.appendRow(image_item)

    def _create_standard_item(self, file_data: Dict[str, any]):
        file_path = file_data["path"]
        is_blurred = file_data.get("is_blurred")

        item_text = os.path.basename(file_path)
        item = QStandardItem(item_text)
        item.setData(file_data, Qt.ItemDataRole.UserRole)
        item.setEditable(False)

        # Icon logic depends on toggle_thumbnails_action and view mode
        if self.menu_manager.toggle_thumbnails_action.isChecked():
            thumbnail_pixmap = self.image_pipeline.get_thumbnail_qpixmap(
                file_path, apply_auto_edits=self.apply_auto_edits_enabled
            )
            if thumbnail_pixmap:
                item.setIcon(QIcon(thumbnail_pixmap))

        if self._is_marked_for_deletion(file_path):
            item.setForeground(
                QColor("#FFB366")
            )  # Orange/Amber color to indicate marked status
            item.setText(item_text)
        elif is_blurred is True:
            item.setForeground(QColor(Qt.GlobalColor.red))
            item.setText(item_text + " (Blurred)")
        else:  # Default
            item.setForeground(QApplication.palette().text().color())
            item.setText(item_text)

        return item

    def _start_similarity_analysis(self):
        logger.info("_start_similarity_analysis called.")
        if self.worker_manager.is_similarity_worker_running():
            self.statusBar().showMessage(
                "Similarity analysis is already in progress.", 3000
            )
            return

        if not self.app_state.image_files_data:
            self.hide_loading_overlay()
            self.statusBar().showMessage(
                "No images loaded to analyze similarity.", 3000
            )
            return

        paths_for_similarity = [fd["path"] for fd in self.app_state.image_files_data]
        if not paths_for_similarity:
            self.hide_loading_overlay()
            self.statusBar().showMessage(
                "No valid image paths for similarity analysis.", 3000
            )
            return

        self.show_loading_overlay("Starting similarity analysis...")
        self.menu_manager.analyze_similarity_action.setEnabled(False)
        self.worker_manager.start_similarity_analysis(
            paths_for_similarity, self.apply_auto_edits_enabled
        )

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
        self.menu_manager.analyze_similarity_action.setEnabled(
            bool(self.app_state.image_files_data)
        )

        if not self.app_state.cluster_results:
            self.hide_loading_overlay()
            self.statusBar().showMessage("Clustering did not produce results.", 3000)
            return

        self.update_loading_text("Clustering complete. Updating view...")
        cluster_ids = sorted(list(set(self.app_state.cluster_results.values())))
        self.cluster_filter_combo.clear()
        self.cluster_filter_combo.addItems(
            ["All Clusters"] + [f"Cluster {cid}" for cid in cluster_ids]
        )
        self.cluster_filter_combo.setEnabled(True)
        self.menu_manager.group_by_similarity_action.setEnabled(True)
        self.menu_manager.group_by_similarity_action.setChecked(
            True
        )  # Automatically switch to group by similarity view
        if (
            self.menu_manager.group_by_similarity_action.isChecked()
            and self.app_state.cluster_results
        ):
            self.menu_manager.cluster_sort_action.setVisible(True)
            self.cluster_sort_combo.setEnabled(True)
        if self.group_by_similarity_mode:
            self._rebuild_model_view()
        self.hide_loading_overlay()

    # Slot for WorkerManager's similarity_error signal
    def _handle_similarity_error(self, message):
        self.statusBar().showMessage(f"Similarity Error: {message}", 8000)
        self.menu_manager.analyze_similarity_action.setEnabled(
            bool(self.app_state.image_files_data)
        )
        self.hide_loading_overlay()

    def _reload_current_folder(self):
        if self.app_state.image_files_data:
            if (
                self.app_state.image_files_data[0]
                and "path" in self.app_state.image_files_data[0]
            ):
                current_dir = os.path.dirname(
                    self.app_state.image_files_data[0]["path"]
                )
                if os.path.isdir(current_dir):
                    self._load_folder(current_dir)
                    return
        self.statusBar().showMessage("No folder context to reload.", 3000)

    def _handle_item_collapsed(self, proxy_index: QModelIndex):
        if self.group_by_similarity_mode and proxy_index.isValid():
            active_view = self.left_panel.tree_display_view
            source_index = self.proxy_model.mapToSource(proxy_index)
            item = self.file_system_model.itemFromIndex(source_index)
            if item:
                item_data = item.data(Qt.ItemDataRole.UserRole)
                if isinstance(item_data, str) and item_data.startswith(
                    "cluster_header_"
                ):
                    QTimer.singleShot(0, lambda: active_view.expand(proxy_index))

    def _cluster_sort_changed(self):
        if self.group_by_similarity_mode and self.app_state.cluster_results:
            self._rebuild_model_view()

    def _get_cluster_timestamps(
        self,
        images_by_cluster: Dict[int, List[Dict[str, any]]],
        date_cache: Dict[str, Optional[date_obj]],
    ) -> Dict[int, date_obj]:
        cluster_timestamps = {}
        for cluster_id, file_data_list in images_by_cluster.items():
            earliest_date = date_obj.max
            found_date = False
            for file_data in file_data_list:
                img_date = date_cache.get(file_data["path"])
                if img_date and img_date < earliest_date:
                    earliest_date = img_date
                    found_date = True
            cluster_timestamps[cluster_id] = (
                earliest_date if found_date else date_obj.max
            )
        return cluster_timestamps

    def _calculate_cluster_centroids(
        self,
        images_by_cluster: Dict[int, List[Dict[str, any]]],
        embeddings_cache: Dict[str, List[float]],
    ) -> Dict[int, np.ndarray]:
        centroids = {}
        if not embeddings_cache:
            return centroids
        for cluster_id, file_data_list in images_by_cluster.items():
            cluster_embeddings = []
            for file_data in file_data_list:
                embedding = embeddings_cache.get(file_data["path"])
                if embedding is not None:
                    if isinstance(embedding, np.ndarray):
                        cluster_embeddings.append(embedding)
                    elif isinstance(embedding, list):
                        cluster_embeddings.append(np.array(embedding))
            if cluster_embeddings:
                try:
                    # Ensure all embeddings are numpy arrays before stacking for mean calculation
                    if all(isinstance(emb, np.ndarray) for emb in cluster_embeddings):
                        if cluster_embeddings:  # Ensure list is not empty
                            # Explicitly cast to float32 if not already, for consistency
                            centroids[cluster_id] = np.mean(
                                np.array(cluster_embeddings, dtype=np.float32), axis=0
                            )
                except Exception as e:  # Catch potential errors in np.mean, like empty list or dtype issues
                    logger.error(
                        f"Error calculating centroid for cluster {cluster_id}: {e}"
                    )
                    pass
        return centroids

    def _sort_clusters_by_similarity_time(
        self,
        images_by_cluster: Dict[int, List[Dict[str, any]]],
        embeddings_cache: Dict[str, List[float]],
        date_cache: Dict[str, Optional[date_obj]],
    ) -> List[int]:
        cluster_ids = list(images_by_cluster.keys())
        if not cluster_ids:
            return []

        centroids = self._calculate_cluster_centroids(
            images_by_cluster, embeddings_cache
        )
        valid_cluster_ids_for_pca = [
            cid
            for cid in cluster_ids
            if cid in centroids
            and centroids[cid] is not None
            and centroids[cid].size > 0
        ]

        if not valid_cluster_ids_for_pca or len(valid_cluster_ids_for_pca) < 2:
            cluster_timestamps_for_fallback = self._get_cluster_timestamps(
                images_by_cluster, date_cache
            )
            return sorted(
                list(images_by_cluster.keys()),
                key=lambda cid_orig: cluster_timestamps_for_fallback.get(
                    cid_orig, date_obj.max
                ),
            )

        valid_centroid_list = [centroids[cid] for cid in valid_cluster_ids_for_pca]
        if not valid_centroid_list:
            cluster_timestamps_for_fallback = self._get_cluster_timestamps(
                images_by_cluster, date_cache
            )
            return sorted(
                list(images_by_cluster.keys()),
                key=lambda cid_orig: cluster_timestamps_for_fallback.get(
                    cid_orig, date_obj.max
                ),
            )

        centroid_matrix = np.array(valid_centroid_list)

        pca_scores = {}
        # Ensure matrix is 2D and has enough samples/features for PCA
        if (
            centroid_matrix.ndim == 2
            and centroid_matrix.shape[0] > 1
            and centroid_matrix.shape[1] > 0
        ):
            try:
                # n_components for PCA must be less than min(n_samples, n_features)
                n_components_pca = min(
                    1,
                    centroid_matrix.shape[0] - 1 if centroid_matrix.shape[0] > 1 else 1,
                    centroid_matrix.shape[1],
                )
                if n_components_pca > 0:  # Ensure n_components is at least 1
                    pca = PCA(n_components=n_components_pca)
                    transformed_centroids = pca.fit_transform(centroid_matrix)
                    for i, cid in enumerate(valid_cluster_ids_for_pca):
                        pca_scores[cid] = (
                            transformed_centroids[i, 0]
                            if transformed_centroids.ndim > 1
                            else transformed_centroids[i]
                        )
            except Exception as e:
                logger.error(f"Error during PCA for cluster sorting: {e}")

        cluster_timestamps = self._get_cluster_timestamps(images_by_cluster, date_cache)
        sortable_clusters = []
        for cid in cluster_ids:
            pca_val = pca_scores.get(
                cid, float("inf")
            )  # Default to inf if PCA score not found
            ts_val = cluster_timestamps.get(cid, date_obj.max)
            sortable_clusters.append((cid, pca_val, ts_val))
        sortable_clusters.sort(
            key=lambda x: (x[1], x[2])
        )  # Sort by PCA score, then timestamp
        return [item[0] for item in sortable_clusters]

    def _handle_toggle_auto_edits(self, checked: bool):
        self.apply_auto_edits_enabled = checked
        set_auto_edit_photos(checked)  # Save to persistent settings

        # If no images are loaded, just set the preference and exit.
        if not self.app_state.image_files_data:
            self.statusBar().showMessage(
                f"Auto RAW edits has been {'enabled' if checked else 'disabled'}.", 4000
            )
            return

        self.show_loading_overlay("Applying new edit settings...")
        QApplication.processEvents()  # Ensure overlay appears immediately

        self.image_pipeline.clear_all_image_caches()
        self._rebuild_model_view()

        if self.app_state.image_files_data:
            # The loading overlay text will be updated by the preview worker's progress signals
            self.worker_manager.start_preview_preload(
                [fd["path"] for fd in self.app_state.image_files_data],
                self.apply_auto_edits_enabled,
            )

        active_view = self._get_active_file_view()
        if active_view:
            current_proxy_idx = active_view.currentIndex()
            if current_proxy_idx.isValid():
                try:
                    active_view.selectionModel().selectionChanged.disconnect(
                        self._handle_file_selection_changed
                    )
                except TypeError:
                    pass

                self._handle_file_selection_changed()

                try:
                    active_view.selectionModel().selectionChanged.connect(
                        self._handle_file_selection_changed
                    )
                except TypeError:
                    pass
            else:
                first_visible_item = self._find_first_visible_item()
                if first_visible_item.isValid():
                    active_view.setCurrentIndex(first_visible_item)

        # The final status bar message is now handled by _handle_preview_finished
        # We can update the status bar here to show that the process has started.
        self.statusBar().showMessage(
            f"Regenerating previews with Auto RAW edits {'enabled' if checked else 'disabled'}...",
            0,
        )

    def _handle_toggle_mark_for_deletion_mode(self, checked: bool):
        self.mark_for_deletion_mode_enabled = checked
        set_mark_for_deletion_mode(checked)
        status_message = (
            "Delete key will now mark files for deletion."
            if checked
            else "Delete key will now move files to trash directly."
        )
        self.statusBar().showMessage(status_message, 4000)

    def _start_blur_detection_analysis(self):
        logger.info("_start_blur_detection_analysis called.")
        if not self.app_state.image_files_data:
            self.statusBar().showMessage(
                "No images loaded to analyze for blurriness.", 3000
            )
            return

        if self.worker_manager.is_blur_detection_running():
            self.statusBar().showMessage("Blur detection is already in progress.", 3000)
            return

        self.show_loading_overlay("Starting blur detection...")
        self.menu_manager.detect_blur_action.setEnabled(False)

        self.worker_manager.start_blur_detection(
            self.app_state.image_files_data.copy(),
            self.blur_detection_threshold,
            self.apply_auto_edits_enabled,
        )

    # Slot for WorkerManager's blur_detection_progress signal
    def _handle_blur_detection_progress(
        self, current: int, total: int, path_basename: str
    ):
        percentage = int((current / total) * 100) if total > 0 else 0
        self.update_loading_text(
            f"Detecting blur: {percentage}% ({current}/{total}) - {path_basename}"
        )

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
            if not top_item:
                continue

            # Check top-level item
            top_item_data = top_item.data(Qt.ItemDataRole.UserRole)
            if (
                isinstance(top_item_data, dict)
                and top_item_data.get("path") == image_path
            ):
                item_to_update = top_item
                break

            if top_item.hasChildren():  # Check children if it's a folder/group
                for r_child in range(top_item.rowCount()):
                    child_item = top_item.child(r_child)
                    if not child_item:
                        continue

                    child_item_data = child_item.data(Qt.ItemDataRole.UserRole)
                    if (
                        isinstance(child_item_data, dict)
                        and child_item_data.get("path") == image_path
                    ):
                        item_to_update = child_item
                        break

                    # Potentially check grandchildren if structure is deeper (e.g., date view inside cluster view)
                    if child_item.hasChildren():
                        for r_grandchild in range(child_item.rowCount()):
                            grandchild_item = child_item.child(r_grandchild)
                            if not grandchild_item:
                                continue
                            grandchild_item_data = grandchild_item.data(
                                Qt.ItemDataRole.UserRole
                            )
                            if (
                                isinstance(grandchild_item_data, dict)
                                and grandchild_item_data.get("path") == image_path
                            ):
                                item_to_update = grandchild_item
                                break
                        if item_to_update:
                            break
                if item_to_update:
                    break

        if item_to_update:
            original_text = os.path.basename(image_path)
            # Update the UserRole data in the source model item
            item_user_data = item_to_update.data(Qt.ItemDataRole.UserRole)
            if isinstance(item_user_data, dict):
                item_user_data["is_blurred"] = is_blurred  # Update existing dict
                item_to_update.setData(item_user_data, Qt.ItemDataRole.UserRole)
            else:  # Should not happen if item was created correctly
                item_to_update.setData(
                    {"path": image_path, "is_blurred": is_blurred},
                    Qt.ItemDataRole.UserRole,
                )

            # Update display text and color
            if is_blurred is True:
                item_to_update.setForeground(QColor(Qt.GlobalColor.red))
                item_to_update.setText(original_text + " (Blurred)")
            elif is_blurred is False:
                default_text_color = QApplication.palette().text().color()
                item_to_update.setForeground(default_text_color)
                item_to_update.setText(original_text)
            else:  # is_blurred is None
                default_text_color = QApplication.palette().text().color()
                item_to_update.setForeground(default_text_color)
                item_to_update.setText(original_text)

            # If the updated item is currently selected, refresh the main image view and status bar
            if active_view and active_view.currentIndex().isValid():
                current_proxy_idx = active_view.currentIndex()
                current_source_idx = proxy_model.mapToSource(current_proxy_idx)
                selected_item = source_model.itemFromIndex(current_source_idx)
                if selected_item == item_to_update:
                    self._handle_file_selection_changed()  # Re-process selection to update main view
        else:
            logger.warning(
                f"Could not find QStandardItem for {image_path} to update blur status in UI."
            )

    # Slot for WorkerManager's blur_detection_finished signal
    def _perform_group_selection_from_key(
        self, key: int, active_view_from_event: QWidget
    ) -> bool:
        if not self.group_by_similarity_mode:
            return False

        target_index = key - Qt.Key.Key_1
        active_view = self._get_active_file_view()

        if active_view_from_event is not active_view or not active_view:
            return False

        current_proxy_idx = active_view.currentIndex()
        if not current_proxy_idx.isValid():
            return False

        # This function is now only called in single-selection contexts.
        # We determine the group from the currently focused item and select the Nth image from it.
        images_to_consider_paths = []
        determined_cluster_id = None
        search_idx = current_proxy_idx
        while search_idx.isValid():
            s_idx = self.proxy_model.mapToSource(search_idx)
            item = self.file_system_model.itemFromIndex(s_idx)
            if not item:
                break
            item_data = item.data(Qt.ItemDataRole.UserRole)
            if isinstance(item_data, dict) and "path" in item_data:
                image_path = item_data["path"]
                if os.path.exists(image_path):
                    determined_cluster_id = self.app_state.cluster_results.get(
                        image_path
                    )
                    break
            elif isinstance(item_data, str) and item_data.startswith("cluster_header_"):
                try:
                    determined_cluster_id = int(item_data.split("_")[-1])
                except (ValueError, IndexError):
                    pass
                break
            search_idx = search_idx.parent()

        if determined_cluster_id is not None:
            images_by_cluster = self._group_images_by_cluster()
            images_in_group_data = images_by_cluster.get(determined_cluster_id, [])
            images_to_consider_paths = [d["path"] for d in images_in_group_data]

        if not images_to_consider_paths:
            return False

        # Sort the paths to match display order
        def sort_key(path):
            date = self.app_state.date_cache.get(path, date_obj.max)
            basename = os.path.basename(path)
            sort_mode = self.cluster_sort_combo.currentText()
            if sort_mode == "Time" or sort_mode == "Similarity then Time":
                return (date, basename)
            return basename

        sorted_paths = sorted(images_to_consider_paths, key=sort_key)

        if 0 <= target_index < len(sorted_paths):
            target_path = sorted_paths[target_index]
            proxy_idx_to_select = self._find_proxy_index_for_path(target_path)

            if proxy_idx_to_select.isValid():
                # In single-select, change the selection in the tree view
                active_view.setCurrentIndex(proxy_idx_to_select)
                active_view.selectionModel().select(
                    proxy_idx_to_select,
                    QItemSelectionModel.SelectionFlag.ClearAndSelect,
                )
                active_view.scrollTo(
                    proxy_idx_to_select,
                    QAbstractItemView.ScrollHint.EnsureVisible,
                )
                return True

        return False

    def eventFilter(self, obj: QObject, event: QEvent) -> bool:
        if event.type() == QEvent.Type.KeyPress:
            # Ensure the event is for one of our views
            is_left_panel_view = obj in self._left_panel_views
            is_image_viewer = obj in self._image_viewer_views

            if is_left_panel_view or is_image_viewer:
                logger.debug(
                    "EventFilter KeyPress: obj=%s, is_left_panel_view=%s, is_image_viewer=%s",
                    obj.__class__.__name__,
                    is_left_panel_view,
                    is_image_viewer,
                )

            if is_left_panel_view or is_image_viewer:
                key_event: QKeyEvent = event
                key = key_event.key()
                modifiers = key_event.modifiers()

                # --- Enhanced Logging ---
                mod_str = []
                if modifiers & Qt.KeyboardModifier.ShiftModifier:
                    mod_str.append("Shift")
                if modifiers & Qt.KeyboardModifier.ControlModifier:
                    mod_str.append("Ctrl")
                if modifiers & Qt.KeyboardModifier.AltModifier:
                    mod_str.append("Alt")
                if modifiers & Qt.KeyboardModifier.MetaModifier:
                    mod_str.append("Meta/Cmd")
                logger.debug(
                    f"KeyPress in view: Key={key}, Modifiers=[{', '.join(mod_str)}]"
                )

                search_has_focus = self.left_panel.search_input.hasFocus()

                if not search_has_focus:
                    # Rotation view-specific shortcuts
                    if self.left_panel.current_view_mode == "rotation":
                        if key == Qt.Key.Key_Y:
                            if modifiers == Qt.KeyboardModifier.ShiftModifier:
                                self._accept_all_rotations()
                                return True
                            elif modifiers == Qt.KeyboardModifier.NoModifier:
                                self._accept_current_rotation()
                                return True
                        elif key == Qt.Key.Key_N:
                            if modifiers == Qt.KeyboardModifier.ShiftModifier:
                                self._refuse_all_rotations()
                                return True
                            elif modifiers == Qt.KeyboardModifier.NoModifier:
                                self._refuse_current_rotation()
                                return True

                    # --- Modifier-based actions ---
                    is_unmodified = modifiers == Qt.KeyboardModifier.NoModifier
                    # On Mac, arrow keys often have KeypadModifier, so treat that as unmodified too
                    is_unmodified_or_keypad = modifiers in (
                        Qt.KeyboardModifier.NoModifier,
                        Qt.KeyboardModifier.KeypadModifier,
                    )
                    is_control_or_meta = modifiers in (
                        Qt.KeyboardModifier.ControlModifier,
                        Qt.KeyboardModifier.MetaModifier,
                    )

                    # Rating shortcuts (Ctrl/Cmd + 0-5)
                    if is_control_or_meta and Qt.Key.Key_0 <= key <= Qt.Key.Key_5:
                        rating = key - Qt.Key.Key_0
                        self._apply_rating_to_selection(rating)
                        return (
                            True  # --- Custom navigation for UNMODIFIED arrow keys ---
                        )
                    if is_unmodified_or_keypad:
                        logger.debug(
                            "Unmodified/keypad key detected: %s (modifiers: %s)",
                            key,
                            modifiers,
                        )
                        if key == Qt.Key.Key_Left or key == Qt.Key.Key_A:
                            logger.debug(
                                f"Arrow key pressed: LEFT/A - Starting navigation"
                            )
                            self._navigate_left_in_group(skip_deleted=True)
                            return True
                        if key == Qt.Key.Key_Right or key == Qt.Key.Key_D:
                            logger.debug(
                                f"Arrow key pressed: RIGHT/D - Starting navigation"
                            )
                            self._navigate_right_in_group(skip_deleted=True)
                            return True
                        if key == Qt.Key.Key_Up or key == Qt.Key.Key_W:
                            logger.debug(
                                f"Arrow key pressed: UP/W - Starting navigation"
                            )
                            self._navigate_up_sequential(skip_deleted=True)
                            return True
                        if key == Qt.Key.Key_Down or key == Qt.Key.Key_S:
                            logger.debug(
                                f"Arrow key pressed: DOWN/S - Starting navigation"
                            )
                            self._navigate_down_sequential(skip_deleted=True)
                            return True
                        if key == Qt.Key.Key_Delete or key == Qt.Key.Key_Backspace:
                            self._handle_delete_action()
                            return True

                    # --- Navigation with Ctrl modifier (bypasses deleted file skipping) ---
                    elif modifiers == Qt.KeyboardModifier.ControlModifier:
                        logger.debug(
                            f"Ctrl+Arrow key detected: {key} - Navigation with deleted file bypass"
                        )
                        ctrl_arrow_actions = {
                            Qt.Key.Key_Left: ("LEFT/A", self._navigate_left_in_group),
                            Qt.Key.Key_A: ("LEFT/A", self._navigate_left_in_group),
                            Qt.Key.Key_Right: (
                                "RIGHT/D",
                                self._navigate_right_in_group,
                            ),
                            Qt.Key.Key_D: ("RIGHT/D", self._navigate_right_in_group),
                            Qt.Key.Key_Up: ("UP/W", self._navigate_up_sequential),
                            Qt.Key.Key_W: ("UP/W", self._navigate_up_sequential),
                            Qt.Key.Key_Down: ("DOWN/S", self._navigate_down_sequential),
                            Qt.Key.Key_S: ("DOWN/S", self._navigate_down_sequential),
                        }
                        if key in ctrl_arrow_actions:
                            direction, action = ctrl_arrow_actions[key]
                            logger.debug(
                                f"Ctrl+Arrow key pressed: {direction} - Starting navigation (bypass deleted)"
                            )
                            action(skip_deleted=False)
                            return True
                    else:
                        logger.debug(
                            f"Key with modifiers detected: {key}, modifiers: {modifiers}"
                        )
            else:
                logger.debug(
                    f"EventFilter: Key press not from tracked views, obj={obj.__class__.__name__ if obj else 'None'}"
                )

        # For all other key presses (including Shift+Arrows), pass the event on.
        return super().eventFilter(obj, event)

    def _toggle_metadata_sidebar(self, checked: bool):
        """Toggle the metadata sidebar visibility"""
        if checked:
            self._show_metadata_sidebar()
        else:
            self._hide_metadata_sidebar()

    def _show_metadata_sidebar(self):
        """Show the metadata sidebar, ensuring it reflects the current selection state."""
        if not self.metadata_sidebar:
            return

        self.sidebar_visible = True
        self.menu_manager.toggle_metadata_sidebar_action.blockSignals(True)
        self.menu_manager.toggle_metadata_sidebar_action.setChecked(True)
        self.menu_manager.toggle_metadata_sidebar_action.blockSignals(False)

        # Explicitly check selection state before showing
        selected_paths = self._get_selected_file_paths_from_view()
        if len(selected_paths) == 2:
            # If two images are selected, force the comparison view
            self._display_multi_selection_info(selected_paths)
        else:
            # Otherwise, update with the single selection (or placeholder)
            self._update_sidebar_with_current_selection()

        self._set_sidebar_visibility(True)
        self.statusBar().showMessage(
            "Image details sidebar shown. Press I to toggle.", 3000
        )

    def _hide_metadata_sidebar(self):
        """Hide the metadata sidebar"""
        if not self.metadata_sidebar or not self.sidebar_visible:
            return

        self.sidebar_visible = False
        # Block signals to avoid signal loop
        self.menu_manager.toggle_metadata_sidebar_action.blockSignals(True)
        self.menu_manager.toggle_metadata_sidebar_action.setChecked(False)
        self.menu_manager.toggle_metadata_sidebar_action.blockSignals(False)

        # Hide sidebar instantly
        self._set_sidebar_visibility(False)

    def _set_sidebar_visibility(self, show: bool):
        """Show or hide the sidebar instantly"""
        if not hasattr(self, "main_splitter") or not self.metadata_sidebar:
            return

        current_sizes = self.main_splitter.sizes()
        if len(current_sizes) != 3:
            return

        if show:
            # Show sidebar with 320px width
            target_width = 320
            total_width = sum(current_sizes)
            new_sizes = [
                max(300, int((total_width - target_width) * 0.3)),  # Left pane
                total_width
                - max(300, int((total_width - target_width) * 0.3))
                - target_width,  # Center pane
                target_width,  # Sidebar
            ]
            self.main_splitter.setSizes(new_sizes)
        else:
            # Hide sidebar
            new_sizes = [
                current_sizes[0],  # Left pane unchanged
                current_sizes[1] + current_sizes[2],  # Center gets sidebar space
                0,  # Sidebar hidden
            ]
            self.main_splitter.setSizes(new_sizes)

    def _show_advanced_viewer(self):
        """Show the advanced image viewer"""
        selected_paths = self._get_selected_file_paths_from_view()

        if not selected_paths:
            self.statusBar().showMessage(
                "No images selected for advanced viewer.", 3000
            )
            return

        # Create advanced viewer window
        self.advanced_viewer_window = QWidget()
        self.advanced_viewer_window.setWindowTitle("PhotoSort - Advanced Viewer")
        self.advanced_viewer_window.setGeometry(200, 200, 1200, 800)

        layout = QVBoxLayout(self.advanced_viewer_window)

        # Create the synchronized viewer
        self.sync_viewer = SynchronizedImageViewer()
        layout.addWidget(self.sync_viewer)

        # Load images
        if len(selected_paths) == 1:
            # Single image mode
            pixmap = self.image_pipeline.get_preview_qpixmap(
                selected_paths[0],
                display_max_size=(8000, 8000),  # High resolution for zoom
                apply_auto_edits=self.apply_auto_edits_enabled,
            )
            if pixmap:
                self.sync_viewer.set_image(pixmap, 0)
        elif len(selected_paths) >= 2:
            # Side-by-side comparison mode
            pixmaps = []
            for path in selected_paths[:2]:  # Max 2 images
                pixmap = self.image_pipeline.get_preview_qpixmap(
                    path,
                    display_max_size=(8000, 8000),
                    apply_auto_edits=self.apply_auto_edits_enabled,
                )
                if pixmap:
                    pixmaps.append(pixmap)

            if pixmaps:
                self.sync_viewer.set_images(pixmaps)

        self.advanced_viewer_window.show()

    def _update_sidebar_with_current_selection(self):
        """Update sidebar with the currently selected image metadata"""

        if not self.metadata_sidebar or not self.sidebar_visible:
            logger.debug(
                "_update_sidebar_with_current_selection: Sidebar not available or not visible"
            )
            return

        active_view = self._get_active_file_view()
        if not active_view:
            logger.debug("_update_sidebar_with_current_selection: No active view")
            self.metadata_sidebar.show_placeholder()
            return

        current_proxy_idx = active_view.currentIndex()
        if not current_proxy_idx.isValid() or not self._is_valid_image_item(
            current_proxy_idx
        ):
            logger.debug(
                "_update_sidebar_with_current_selection: No valid image item selected"
            )
            self.metadata_sidebar.show_placeholder()
            return

        # Get the selected file path and metadata
        source_idx = self.proxy_model.mapToSource(current_proxy_idx)
        item = self.file_system_model.itemFromIndex(source_idx)
        if not item:
            logger.warning(
                "_update_sidebar_with_current_selection: No item from source index"
            )
            return

        item_data = item.data(Qt.ItemDataRole.UserRole)
        if not isinstance(item_data, dict) or "path" not in item_data:
            logger.warning("_update_sidebar_with_current_selection: Invalid item data")
            return

        file_path = item_data["path"]
        file_ext = os.path.splitext(file_path)[1].lower()

        logger.info(
            f"_update_sidebar_with_current_selection: Processing {os.path.basename(file_path)} (extension: {file_ext})"
        )

        if not os.path.exists(file_path):
            logger.error(
                f"_update_sidebar_with_current_selection: File does not exist: {file_path}"
            )
            return

        # Get cached metadata
        metadata = self._get_cached_metadata_for_selection(file_path)
        if not metadata:
            logger.warning(
                f"_update_sidebar_with_current_selection: No cached metadata for {os.path.basename(file_path)}"
            )
            return

        logger.info(
            f"_update_sidebar_with_current_selection: Got cached metadata for {os.path.basename(file_path)}: {metadata}"
        )

        # Get detailed EXIF data for sidebar - now much cleaner
        logger.info(
            f"_update_sidebar_with_current_selection: Calling get_detailed_metadata for {os.path.basename(file_path)}"
        )
        raw_exif = MetadataProcessor.get_detailed_metadata(
            file_path, self.app_state.exif_disk_cache
        )

        if not raw_exif:
            logger.warning(
                f"_update_sidebar_with_current_selection: No raw EXIF data returned for {os.path.basename(file_path)}"
            )
            raw_exif = {}
        else:
            logger.info(
                f"_update_sidebar_with_current_selection: Got {len(raw_exif)} raw EXIF keys for {os.path.basename(file_path)}"
            )

        # Update sidebar
        logger.info(
            f"_update_sidebar_with_current_selection: Updating sidebar for {os.path.basename(file_path)}"
        )
        self.metadata_sidebar.update_metadata(file_path, metadata, raw_exif)

    def _handle_successful_rotation(
        self, file_path: str, direction: str, message: str, is_lossy: bool
    ):
        """Handle successful rotation - update caches and UI."""
        handle_start_time = time.perf_counter()
        filename = os.path.basename(file_path)
        logger.debug(
            f"Handling successful rotation for '{filename}' (Lossy: {is_lossy})"
        )

        t1 = time.perf_counter()
        self.image_pipeline.preview_cache.delete_all_for_path(file_path)
        self.image_pipeline.thumbnail_cache.delete_all_for_path(file_path)
        t2 = time.perf_counter()
        logger.info(f"HSR: Cache clearing for {filename} took {t2 - t1:.4f}s.")

        t3 = time.perf_counter()
        proxy_idx = self._find_proxy_index_for_path(file_path)
        t4 = time.perf_counter()
        logger.info(
            f"HSR: _find_proxy_index_for_path for {filename} took {t4 - t3:.4f}s."
        )

        if proxy_idx.isValid():
            source_idx = self.proxy_model.mapToSource(proxy_idx)
            item = self.file_system_model.itemFromIndex(source_idx)
            if item:
                t5 = time.perf_counter()
                new_thumbnail = self.image_pipeline.get_thumbnail_qpixmap(
                    file_path, apply_auto_edits=self.apply_auto_edits_enabled
                )
                t6 = time.perf_counter()
                logger.info(
                    f"HSR: get_thumbnail_qpixmap for {filename} took {t6 - t5:.4f}s."
                )
                if new_thumbnail:
                    from PyQt6.QtGui import QIcon

                    item.setIcon(QIcon(new_thumbnail))
                    logger.info(f"HSR: Set new icon for {filename}.")

        selected_paths = self._get_selected_file_paths_from_view()
        if file_path in selected_paths:
            logger.info(
                f"HSR: {filename} is in current selection, triggering selection changed handler."
            )
            t7 = time.perf_counter()

            # Pre-cache the correctly generated preview before the selection handler runs
            self.image_pipeline.get_preview_qpixmap(
                file_path,
                display_max_size=(8000, 8000),
                apply_auto_edits=self.apply_auto_edits_enabled,
                force_regenerate=True,
                force_default_brightness=True,  # This is the key change
            )
            logger.info(
                f"HSR: Pre-cached non-brightened preview for {filename} in {time.perf_counter() - t7:.4f}s"
            )

            t8 = time.perf_counter()
            self._handle_file_selection_changed()
            t9 = time.perf_counter()
            logger.info(f"HSR: _handle_file_selection_changed took {t9 - t8:.4f}s.")

        self.statusBar().showMessage(message, 5000)
        logger.info(message)  # Log the original user-facing message
        handle_end_time = time.perf_counter()
        logger.info(
            f"HSR: End for {filename}. Total time: {handle_end_time - handle_start_time:.4f}s"
        )

    def _rotate_current_image_clockwise(self):
        """Rotate the currently selected image(s) 90° clockwise (for keyboard shortcut)."""
        logger.info("Clockwise rotation triggered via shortcut/menu.")
        self._rotate_selected_images("clockwise")

    def _rotate_current_image_counterclockwise(self):
        """Rotate the currently selected image(s) 90° counterclockwise (for keyboard shortcut)."""
        logger.info("Counter-clockwise rotation triggered via shortcut/menu.")
        self._rotate_selected_images("counterclockwise")

    def _rotate_current_image_180(self):
        """Rotate the currently selected image(s) 180° (for keyboard shortcut)."""
        logger.info("180-degree rotation triggered via shortcut/menu.")
        self._rotate_selected_images("180")

    def _rotate_selected_images(self, direction: str):
        """Rotate all currently selected images in the specified direction."""
        focused_path = self.advanced_image_viewer.get_focused_image_path_if_any()

        if focused_path:
            # If a single image is focused in the viewer (from a multi-select),
            # only rotate that specific image.
            selected_paths = [focused_path]
            logger.debug(
                f"Rotation action targeting focused image: {os.path.basename(focused_path)}"
            )
        else:
            # Otherwise, rotate all images in the current selection from the list/grid view.
            selected_paths = self._get_selected_file_paths_from_view()

        if not selected_paths:
            self.statusBar().showMessage("No images selected for rotation.", 3000)
            return

        # Filter out files that don't support rotation
        rotation_supported_paths = []
        unsupported_count = 0

        for path in selected_paths:
            if MetadataProcessor.is_rotation_supported(path):
                rotation_supported_paths.append(path)
            else:
                unsupported_count += 1

        if not rotation_supported_paths:
            self.statusBar().showMessage(
                "None of the selected images support rotation.", 3000
            )
            return

        if unsupported_count > 0:
            self.statusBar().showMessage(
                f"Rotating {len(rotation_supported_paths)} images (skipping {unsupported_count} unsupported files)...",
                3000,
            )

        # Perform rotation on all supported images
        successful_rotations = 0
        failed_rotations = 0

        for i, file_path in enumerate(rotation_supported_paths):
            try:
                # Show progress for multiple files
                if len(rotation_supported_paths) > 1:
                    progress_text = f"Rotating image {i + 1} of {len(rotation_supported_paths)}: {os.path.basename(file_path)}"
                    self.show_loading_overlay(progress_text)
                    self.statusBar().showMessage(progress_text, 0)
                    QApplication.processEvents()

                # Try metadata-only rotation first
                metadata_success, needs_lossy, message = (
                    MetadataProcessor.try_metadata_rotation_first(
                        file_path, direction, self.app_state.exif_disk_cache
                    )
                )

                if metadata_success:
                    # Metadata rotation succeeded
                    self._handle_successful_rotation(
                        file_path, direction, message, is_lossy=False
                    )
                    successful_rotations += 1
                    continue

                if not needs_lossy:
                    # Metadata rotation failed and no lossy option available
                    logger.warning(
                        f"Rotation failed for {os.path.basename(file_path)}: {message}"
                    )
                    failed_rotations += 1
                    continue

                # Metadata rotation failed but lossy rotation is available
                # For batch operations, we'll apply the user's preference without asking each time
                from src.core.app_settings import get_rotation_confirm_lossy

                if get_rotation_confirm_lossy() and len(rotation_supported_paths) > 1:
                    # For multiple images, ask once for the batch
                    rotation_desc = {
                        "clockwise": "90° clockwise",
                        "counterclockwise": "90° counterclockwise",
                        "180": "180°",
                    }.get(direction, direction)

                    proceed, never_ask_again = (
                        self.dialog_manager.show_lossy_rotation_confirmation_dialog(
                            f"{len(rotation_supported_paths)} images", rotation_desc
                        )
                    )

                    if never_ask_again:
                        from src.core.app_settings import set_rotation_confirm_lossy

                        set_rotation_confirm_lossy(False)

                    if not proceed:
                        self.statusBar().showMessage(
                            "Batch rotation cancelled by user.", 3000
                        )
                        return

                    # Update the preference so we don't ask again for remaining images
                    from src.core.app_settings import set_rotation_confirm_lossy

                    set_rotation_confirm_lossy(False)
                elif (
                    get_rotation_confirm_lossy() and len(rotation_supported_paths) == 1
                ):
                    # Single image, ask as usual
                    rotation_desc = {
                        "clockwise": "90° clockwise",
                        "counterclockwise": "90° counterclockwise",
                        "180": "180°",
                    }.get(direction, direction)

                    proceed, never_ask_again = (
                        self.dialog_manager.show_lossy_rotation_confirmation_dialog(
                            os.path.basename(file_path), rotation_desc
                        )
                    )

                    if never_ask_again:
                        from src.core.app_settings import set_rotation_confirm_lossy

                        set_rotation_confirm_lossy(False)

                    if not proceed:
                        self.statusBar().showMessage(
                            "Rotation cancelled by user.", 3000
                        )
                        return

                # Perform lossy rotation
                success = MetadataProcessor.rotate_image(
                    file_path,
                    direction,
                    update_metadata_only=False,
                    exif_disk_cache=self.app_state.exif_disk_cache,
                )

                if success:
                    rotation_desc = {
                        "clockwise": "90° clockwise",
                        "counterclockwise": "90° counterclockwise",
                        "180": "180°",
                    }.get(direction, direction)
                    lossy_message = (
                        f"Rotated {os.path.basename(file_path)} {rotation_desc} (lossy)"
                    )
                    self._handle_successful_rotation(
                        file_path, direction, lossy_message, is_lossy=True
                    )
                    successful_rotations += 1
                else:
                    logger.error(
                        f"Failed to perform lossy rotation for {os.path.basename(file_path)}"
                    )
                    failed_rotations += 1

            except Exception as e:
                logger.error(
                    f"Error rotating {os.path.basename(file_path)}: {str(e)}",
                    exc_info=True,
                )
                failed_rotations += 1

        # Hide loading overlay and show final status
        self.hide_loading_overlay()

        # Compose final status message
        if successful_rotations > 0 and failed_rotations == 0:
            if successful_rotations == 1:
                pass  # Individual success message already shown
            else:
                direction_desc = {
                    "clockwise": "90° clockwise",
                    "counterclockwise": "90° counterclockwise",
                    "180": "180°",
                }.get(direction, direction)
                self.statusBar().showMessage(
                    f"Successfully rotated {successful_rotations} images {direction_desc}.",
                    5000,
                )
        elif successful_rotations > 0 and failed_rotations > 0:
            self.statusBar().showMessage(
                f"Rotated {successful_rotations} images successfully, {failed_rotations} failed.",
                5000,
            )
        elif failed_rotations > 0:
            self.statusBar().showMessage(
                f"Failed to rotate {failed_rotations} images.", 5000
            )

    def changeEvent(self, event: QEvent):
        """Handle window state changes to auto-fit images on maximize."""
        if event.type() == QEvent.Type.WindowStateChange:
            # if self.isMaximized():
            # Fit images to view when window is maximized
            self.advanced_image_viewer.fit_to_viewport()
        super().changeEvent(event)

    def _is_marked_for_deletion(self, file_path: str) -> bool:
        """Checks if a file is marked for deletion by its name."""
        basename = os.path.basename(file_path)
        is_marked = "(DELETED)" in basename
        logger.debug(f"Checking deletion mark for '{basename}': {is_marked}")
        return is_marked

    def _commit_marked_deletions(self):
        """Finds all marked files and moves them to trash, updating the view in-place."""
        active_view = self._get_active_file_view()
        if not self.app_state.current_folder_path or not active_view:
            self.statusBar().showMessage("No folder loaded.", 3000)
            return

        marked_files = [
            f["path"]
            for f in self.app_state.image_files_data
            if self._is_marked_for_deletion(f["path"])
        ]
        if not marked_files:
            self.statusBar().showMessage("No images are marked for deletion.", 3000)
            return

        if not self.dialog_manager.show_commit_deletions_dialog(marked_files):
            return

        # --- Pre-computation for next selection ---
        visible_paths_before = self._get_all_visible_image_paths()
        first_marked_index = -1
        if visible_paths_before and marked_files:
            try:
                first_marked_index = visible_paths_before.index(marked_files[0])
            except ValueError:
                first_marked_index = 0

        # --- Group indices by parent for safe removal ---
        source_indices_by_parent = {}
        for path in marked_files:
            proxy_idx = self._find_proxy_index_for_path(path)
            if proxy_idx.isValid():
                source_idx = self.proxy_model.mapToSource(proxy_idx)
                parent_idx = source_idx.parent()
                if parent_idx not in source_indices_by_parent:
                    source_indices_by_parent[parent_idx] = []
                source_indices_by_parent[parent_idx].append(source_idx.row())

        # --- Delete files and update model ---
        deleted_count = 0
        for file_path in marked_files:
            try:
                self.app_controller.move_to_trash(file_path)
                self.app_state.remove_data_for_path(file_path)
                deleted_count += 1
            except Exception as e:
                logger.error(f"Error moving marked file '{file_path}' to trash: {e}")

        if deleted_count > 0:
            for parent_idx, rows in source_indices_by_parent.items():
                parent_item = (
                    self.file_system_model.itemFromIndex(parent_idx)
                    if parent_idx.isValid()
                    else self.file_system_model.invisibleRootItem()
                )
                if parent_item:
                    for row in sorted(rows, reverse=True):
                        parent_item.takeRow(row)

            self.proxy_model.invalidate()
            self.statusBar().showMessage(f"Committed {deleted_count} deletions.", 5000)
            QApplication.processEvents()

            # --- Select next item ---
            visible_paths_after = self._get_all_visible_image_paths()
            if not visible_paths_after:
                self.advanced_image_viewer.clear()
                return

            next_idx_pos = (
                min(first_marked_index, len(visible_paths_after) - 1)
                if first_marked_index != -1
                else 0
            )
            next_path_to_select = visible_paths_after[max(0, next_idx_pos)]
            next_proxy_idx = self._find_proxy_index_for_path(next_path_to_select)

            if next_proxy_idx.isValid():
                active_view.setCurrentIndex(next_proxy_idx)
                active_view.selectionModel().select(
                    next_proxy_idx, QItemSelectionModel.SelectionFlag.ClearAndSelect
                )
                active_view.scrollTo(
                    next_proxy_idx, QAbstractItemView.ScrollHint.EnsureVisible
                )

    def _mark_selection_for_deletion(self):
        """Toggles the deletion mark for selected files by renaming them, updating the model in-place."""
        active_view = self._get_active_file_view()
        if not active_view:
            return

        original_selection_paths = self._get_selected_file_paths_from_view()
        focused_path = self.advanced_image_viewer.get_focused_image_path_if_any()

        if focused_path:
            paths_to_act_on = [focused_path]
            logger.debug(
                f"Mark for deletion action targeting focused image: {os.path.basename(focused_path)}"
            )
        else:
            paths_to_act_on = original_selection_paths

        if not paths_to_act_on:
            self.statusBar().showMessage(
                "No images selected to mark for deletion.", 3000
            )
            return

        path_index_map = {
            path: self._find_proxy_index_for_path(path) for path in paths_to_act_on
        }
        rename_map = {}

        for old_path in paths_to_act_on:
            is_marked = self._is_marked_for_deletion(old_path)
            directory, filename = os.path.split(old_path)
            new_filename = (
                filename.replace(" (DELETED)", "")
                if is_marked
                else f"{os.path.splitext(filename)[0]} (DELETED){os.path.splitext(filename)[1]}"
            )
            new_path = os.path.join(directory, new_filename)

            try:
                self.app_controller.rename_image(old_path, new_path)
                self.app_state.update_path(old_path, new_path)
                rename_map[old_path] = new_path

                proxy_idx = path_index_map.get(old_path)
                if proxy_idx and proxy_idx.isValid():
                    source_idx = self.proxy_model.mapToSource(proxy_idx)
                    item = self.file_system_model.itemFromIndex(source_idx)
                    if item:
                        item_data = item.data(Qt.ItemDataRole.UserRole)
                        item_data["path"] = new_path
                        item.setData(item_data, Qt.ItemDataRole.UserRole)
                        item.setText(new_filename)
                        if is_marked:  # Unmarking
                            is_blurred = item_data.get("is_blurred")
                            if is_blurred:
                                item.setForeground(QColor(Qt.GlobalColor.red))
                                item.setText(new_filename + " (Blurred)")
                            else:
                                item.setForeground(
                                    QApplication.palette().text().color()
                                )
                        else:  # Marking
                            item.setForeground(QColor("#FFB366"))
            except OSError as e:
                logger.error(f"Error toggling mark for '{filename}': {e}")
                self.statusBar().showMessage(
                    f"Error toggling mark for '{filename}': {e}", 5000
                )

        if not rename_map:
            return

        self.statusBar().showMessage(
            f"Toggled mark for {len(rename_map)} image(s).", 5000
        )
        self.proxy_model.invalidate()
        QApplication.processEvents()

        final_selection_paths = [rename_map.get(p, p) for p in original_selection_paths]
        self._handle_file_selection_changed(
            override_selected_paths=final_selection_paths
        )

        if focused_path:
            new_focused_path = rename_map.get(focused_path, focused_path)
            self.advanced_image_viewer.set_focused_viewer_by_path(new_focused_path)

        selection = QItemSelection()
        first_idx = QModelIndex()
        for path in final_selection_paths:
            proxy_idx = self._find_proxy_index_for_path(path)
            if proxy_idx.isValid():
                selection.select(proxy_idx, proxy_idx)
                if not first_idx.isValid():
                    first_idx = proxy_idx

        if not selection.isEmpty():
            active_view.selectionModel().blockSignals(True)
            active_view.selectionModel().select(
                selection, QItemSelectionModel.SelectionFlag.ClearAndSelect
            )
            active_view.selectionModel().blockSignals(False)
            if first_idx.isValid():
                active_view.scrollTo(
                    first_idx, QAbstractItemView.ScrollHint.EnsureVisible
                )

    def _clear_all_deletion_marks(self):
        """Unmarks all marked files, updating the view in-place."""
        if not self.app_state.current_folder_path:
            self.statusBar().showMessage("No folder loaded.", 3000)
            return

        marked_files = [
            f["path"]
            for f in self.app_state.image_files_data
            if self._is_marked_for_deletion(f["path"])
        ]
        if not marked_files:
            self.statusBar().showMessage("No images are marked for deletion.", 3000)
            return

        unmarked_new_paths = []
        path_index_map = {
            path: self._find_proxy_index_for_path(path) for path in marked_files
        }

        for old_path in marked_files:
            directory = os.path.dirname(old_path)
            filename = os.path.basename(old_path)
            new_filename = filename.replace(" (DELETED)", "")
            new_path = os.path.join(directory, new_filename)

            try:
                self.app_controller.rename_image(old_path, new_path)
                self.app_state.update_path(old_path, new_path)
                unmarked_new_paths.append(new_path)

                proxy_idx = path_index_map.get(old_path)
                if proxy_idx and proxy_idx.isValid():
                    source_idx = self.proxy_model.mapToSource(proxy_idx)
                    item = self.file_system_model.itemFromIndex(source_idx)
                    if item:
                        item_data = item.data(Qt.ItemDataRole.UserRole)
                        item_data["path"] = new_path
                        item.setData(item_data, Qt.ItemDataRole.UserRole)

                        is_blurred = item_data.get("is_blurred")
                        if is_blurred is True:
                            item.setForeground(QColor(Qt.GlobalColor.red))
                            item.setText(new_filename + " (Blurred)")
                        else:
                            item.setForeground(QApplication.palette().text().color())
                            item.setText(new_filename)
            except OSError as e:
                logger.error(f"Error clearing mark for '{filename}': {e}")

        if not unmarked_new_paths:
            return

        self.proxy_model.invalidate()
        self.statusBar().showMessage(
            f"Cleared deletion marks for {len(unmarked_new_paths)} image(s).", 5000
        )

        QApplication.processEvents()
        active_view = self._get_active_file_view()
        selection = QItemSelection()
        first_idx = QModelIndex()

        for path in unmarked_new_paths:
            proxy_idx = self._find_proxy_index_for_path(path)
            if proxy_idx.isValid():
                selection.select(proxy_idx, proxy_idx)
                if not first_idx.isValid():
                    first_idx = proxy_idx

        if not selection.isEmpty() and active_view:
            active_view.selectionModel().select(
                selection, QItemSelectionModel.SelectionFlag.ClearAndSelect
            )
            if first_idx.isValid():
                active_view.scrollTo(
                    first_idx, QAbstractItemView.ScrollHint.EnsureVisible
                )

    def _get_current_selected_image_path(self) -> Optional[str]:
        """Get the file path of the currently selected image."""
        active_view = self._get_active_file_view()
        if not active_view:
            return None

        current_proxy_idx = active_view.currentIndex()
        if not current_proxy_idx.isValid() or not self._is_valid_image_item(
            current_proxy_idx
        ):
            return None

        source_idx = self.proxy_model.mapToSource(current_proxy_idx)
        item = self.file_system_model.itemFromIndex(source_idx)
        if not item:
            return None

        item_data = item.data(Qt.ItemDataRole.UserRole)
        if not isinstance(item_data, dict) or "path" not in item_data:
            return None

        file_path = item_data["path"]
        if not os.path.exists(file_path):
            return None

        return file_path

    def _handle_focused_image_changed(self, index: int, file_path: str):
        """Slot to handle when the focused image changes in the viewer."""
        if not file_path:
            # If the focused image is cleared, remove the underline
            if self.app_state.focused_image_path:
                self.app_state.focused_image_path = None
                view = self._get_active_file_view()
                if view:
                    view.viewport().update()
                    # Process events to ensure the repaint happens immediately
                    QApplication.processEvents()
            return

        active_view = self._get_active_file_view()
        if not active_view:
            return

        # Update the app state with the new focused path
        self.app_state.focused_image_path = file_path
        # Trigger a repaint of the view to draw the underline
        active_view.viewport().update()

        proxy_index = self._find_proxy_index_for_path(file_path)

        if proxy_index.isValid():
            self._is_syncing_selection = True

            selection_model = active_view.selectionModel()
            original_selection = selection_model.selection()

            # Set the current item indicator, which the delegate uses for underlining.
            # This might temporarily clear the visual selection.
            active_view.setCurrentIndex(proxy_index)

            # Re-apply the original selection to ensure the multi-selection highlight is preserved.
            if not original_selection.isEmpty():
                selection_model.select(
                    original_selection, QItemSelectionModel.SelectionFlag.Select
                )

            active_view.scrollTo(
                proxy_index, QAbstractItemView.ScrollHint.PositionAtCenter
            )
            active_view.setFocus()

            # Reset the flag after the event queue is cleared to prevent loops
            QTimer.singleShot(0, lambda: setattr(self, "_is_syncing_selection", False))

    def _update_item_blur_status(self, image_path: str, is_blurred: bool):
        """
        Finds the QStandardItem for a given path and updates its visual state
        to reflect its blurriness. This is the UI-specific part of the update process.
        """
        source_model = self.file_system_model
        proxy_model = self.proxy_model

        # Get the active view from the LeftPanel, not from the MainWindow itself.
        active_view = self.left_panel.get_active_view()
        if not active_view:
            logger.warning(
                f"Could not get active view to update blur status for {image_path}"
            )
            return

        item_to_update = None
        # The search logic remains the same, as it operates on the source_model which MainWindow owns.
        for r_top in range(source_model.rowCount()):
            top_item = source_model.item(r_top)
            if not top_item:
                continue

            # Check top-level item
            top_item_data = top_item.data(Qt.ItemDataRole.UserRole)
            if (
                isinstance(top_item_data, dict)
                and top_item_data.get("path") == image_path
            ):
                item_to_update = top_item
                break

            if top_item.hasChildren():
                for r_child in range(top_item.rowCount()):
                    child_item = top_item.child(r_child)
                    if not child_item:
                        continue

                    child_item_data = child_item.data(Qt.ItemDataRole.UserRole)
                    if (
                        isinstance(child_item_data, dict)
                        and child_item_data.get("path") == image_path
                    ):
                        item_to_update = child_item
                        break

                    if child_item.hasChildren():
                        for r_grandchild in range(child_item.rowCount()):
                            grandchild_item = child_item.child(r_grandchild)
                            if not grandchild_item:
                                continue
                            grandchild_item_data = grandchild_item.data(
                                Qt.ItemDataRole.UserRole
                            )
                            if (
                                isinstance(grandchild_item_data, dict)
                                and grandchild_item_data.get("path") == image_path
                            ):
                                item_to_update = grandchild_item
                                break
                        if item_to_update:
                            break
                if item_to_update:
                    break

        if item_to_update:
            original_text = os.path.basename(image_path)
            item_user_data = item_to_update.data(Qt.ItemDataRole.UserRole)

            # Update the UserRole data on the item for consistency.
            if isinstance(item_user_data, dict):
                item_user_data["is_blurred"] = is_blurred
                item_to_update.setData(item_user_data, Qt.ItemDataRole.UserRole)

            # Update display text and color, respecting the deletion mark status.
            is_marked_deleted = self._is_marked_for_deletion(image_path)
            blur_suffix = " (Blurred)" if is_blurred else ""

            if is_marked_deleted:
                item_to_update.setForeground(QColor("#FFB366"))
                # The name already contains "(DELETED)", so no need to add suffix
                item_to_update.setText(original_text)
            elif is_blurred:
                item_to_update.setForeground(QColor(Qt.GlobalColor.red))
                item_to_update.setText(original_text + blur_suffix)
            else:
                item_to_update.setForeground(QApplication.palette().text().color())
                item_to_update.setText(original_text)

            # If the updated item is currently selected, refresh the main image viewer and status bar.
            if active_view.currentIndex().isValid():
                current_proxy_idx = active_view.currentIndex()
                current_source_idx = proxy_model.mapToSource(current_proxy_idx)
                selected_item = source_model.itemFromIndex(current_source_idx)
                if selected_item == item_to_update:
                    self._handle_file_selection_changed()
        else:
            logger.warning(
                f"Could not find QStandardItem for {image_path} to update blur status in UI."
            )

    def _display_side_by_side_comparison(self, file_path):
        """Displays the current image and the rotated suggestion side-by-side."""
        sbs_start_time = time.perf_counter()
        logger.info(f"SBS_COMP: Start for {os.path.basename(file_path)}")
        logger.info(
            f"Showing side-by-side comparison for: {os.path.basename(file_path)} (path: {file_path})"
        )

        if file_path not in self.rotation_suggestions:
            logger.warning(
                f"SBS_COMP: File {os.path.basename(file_path)} not in rotation_suggestions."
            )
            return

        rotation = self.rotation_suggestions[file_path]
        logger.info(f"SBS_COMP: Suggestion is {rotation} degrees.")

        t1 = time.perf_counter()
        # Load ONE base pixmap, respecting the user's current auto-edit setting.
        # This represents the "current" view of the image.
        current_pixmap = self.image_pipeline.get_preview_qpixmap(
            file_path, (8000, 8000), apply_auto_edits=self.apply_auto_edits_enabled
        )
        t2 = time.perf_counter()
        logger.info(f"SBS_COMP: get_preview_qpixmap (base) took: {t2 - t1:.4f}s")

        if current_pixmap and not current_pixmap.isNull():
            from PyQt6.QtGui import QTransform

            transform = QTransform()
            transform.rotate(rotation)

            t3 = time.perf_counter()
            suggested_pixmap = current_pixmap.transformed(
                transform, Qt.TransformationMode.SmoothTransformation
            )
            t4 = time.perf_counter()
            logger.info(f"SBS_COMP: QPixmap.transformed took: {t4 - t3:.4f}s")

            t5 = time.perf_counter()
            self.advanced_image_viewer.set_images_data(
                [
                    {"pixmap": current_pixmap, "path": file_path, "rating": 0},
                    {"pixmap": suggested_pixmap, "path": file_path, "rating": 0},
                ]
            )
            t6 = time.perf_counter()
            logger.info(
                f"SBS_COMP: advanced_image_viewer.set_images_data took: {t6 - t5:.4f}s"
            )
        else:
            logger.warning(
                f"SBS_COMP: Failed to load base pixmap for {os.path.basename(file_path)}"
            )
            self.advanced_image_viewer.clear()

        sbs_end_time = time.perf_counter()
        logger.info(f"SBS_COMP: End. Total time: {sbs_end_time - sbs_start_time:.4f}s")

    def _accept_all_rotations(self):
        """Applies all suggested rotations and returns to the list view."""
        if not self.rotation_suggestions:
            self.statusBar().showMessage("No rotation suggestions to accept.", 3000)
            return

        self.app_controller._apply_approved_rotations(self.rotation_suggestions)
        self.rotation_suggestions.clear()
        # hide_rotation_view will switch to list view and rebuild the model
        self._hide_rotation_view()

    def _accept_current_rotation(self):
        selected_paths = self._get_selected_file_paths_from_view()
        if not selected_paths:
            return

        # Handle multi-selection for applying rotations. This method is triggered by the "Accept (N)" button in the UI.
        rotations_to_apply = {
            path: self.rotation_suggestions[path]
            for path in selected_paths
            if path in self.rotation_suggestions
        }

        if not rotations_to_apply:
            return

        self.app_controller._apply_approved_rotations(rotations_to_apply)

        # Remove the accepted rotations from the main suggestion list
        for path in rotations_to_apply:
            if path in self.rotation_suggestions:
                del self.rotation_suggestions[path]

        # If no suggestions are left, hide the rotation view and return to list view
        if not self.rotation_suggestions:
            self._hide_rotation_view()
            return

        # Rebuild the rotation view to show the remaining items
        self._rebuild_rotation_view()

        # After batch-accepting, clear the selection and image preview to provide
        # a clean state for the user to make their next selection.
        active_view = self._get_active_file_view()
        if active_view:
            active_view.selectionModel().clear()
            self.advanced_image_viewer.clear()
            # Hide the button until a new selection is made
            self.accept_button.setVisible(False)

    def _accept_rotation(self, file_path: str):
        """Applies a single rotation suggestion and selects the next/previous item."""
        if file_path in self.rotation_suggestions:
            # Get the list of items before modification to determine the next selection
            current_items = list(self.rotation_suggestions.keys())
            try:
                current_index = current_items.index(file_path)
            except ValueError:
                current_index = -1

            rotation = self.rotation_suggestions.pop(file_path)
            self.app_controller._apply_approved_rotations({file_path: rotation})

            if not self.rotation_suggestions:
                self._hide_rotation_view()
                return

            remaining_items = list(self.rotation_suggestions.keys())
            path_to_select = None
            if current_index != -1 and remaining_items:
                next_index = min(current_index, len(remaining_items) - 1)
                path_to_select = remaining_items[next_index]

            self._rebuild_rotation_view()

            if path_to_select:
                proxy_idx_to_select = self._find_proxy_index_for_path(path_to_select)
                if proxy_idx_to_select.isValid():
                    active_view = self._get_active_file_view()
                    if active_view:
                        active_view.setCurrentIndex(proxy_idx_to_select)
                        active_view.selectionModel().select(
                            proxy_idx_to_select,
                            QItemSelectionModel.SelectionFlag.ClearAndSelect,
                        )
                        active_view.scrollTo(
                            proxy_idx_to_select,
                            QAbstractItemView.ScrollHint.EnsureVisible,
                        )

    def _refuse_all_rotations(self):
        """Refuses all remaining rotation suggestions."""
        if not self.rotation_suggestions:
            self.statusBar().showMessage("No rotation suggestions to refuse.", 3000)
            return

        self.rotation_suggestions.clear()
        self.statusBar().showMessage(
            "All rotation suggestions have been refused.", 5000
        )
        self._hide_rotation_view()

    def _refuse_current_rotation(self):
        """Refuses the currently selected rotation suggestions."""
        selected_paths = self._get_selected_file_paths_from_view()
        if not selected_paths:
            return

        for path in selected_paths:
            if path in self.rotation_suggestions:
                del self.rotation_suggestions[path]

        if not self.rotation_suggestions:
            self._hide_rotation_view()
            return

        self._rebuild_rotation_view()
        self.advanced_image_viewer.clear()
        self.accept_button.setVisible(False)
        self.refuse_button.setVisible(False)

    def _hide_rotation_view(self):
        """Hides the rotation view and switches back to the default list view."""
        logger.info("Hiding rotation view as no more suggestions.")
        self.left_panel.set_view_mode_list()
        self.accept_all_button.setVisible(False)
        self.accept_button.setVisible(False)
        self.refuse_button.setVisible(False)
        self.refuse_all_button.setVisible(False)
        self.left_panel.view_rotation_icon.setVisible(False)
        self.statusBar().showMessage("All rotation suggestions processed.", 5000)
        self._rebuild_model_view()

    def _handle_tree_view_click(self, proxy_index: QModelIndex):
        if not proxy_index.isValid() or not self.group_by_similarity_mode:
            return

        source_index = self.proxy_model.mapToSource(proxy_index)
        item = self.file_system_model.itemFromIndex(source_index)

        if not item:
            return

        item_data = item.data(Qt.ItemDataRole.UserRole)
        if isinstance(item_data, str) and item_data.startswith("cluster_header_"):
            active_view = self._get_active_file_view()
            if not active_view or not isinstance(active_view, QTreeView):
                return

            selection = QItemSelection()
            for row in range(item.rowCount()):
                child_item = item.child(row)
                if child_item:
                    child_source_index = child_item.index()
                    child_proxy_index = self.proxy_model.mapFromSource(
                        child_source_index
                    )
                    if child_proxy_index.isValid():
                        selection.select(child_proxy_index, child_proxy_index)

            if not selection.isEmpty():
                active_view.selectionModel().select(
                    selection, QItemSelectionModel.SelectionFlag.ClearAndSelect
                )

    def _on_side_by_side_availability_changed(self, is_available: bool):
        """Enable/disable the side-by-side view action based on availability."""
        self.menu_manager.side_by_side_view_action.setEnabled(is_available)

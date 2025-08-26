import time
import logging
from src.ui.advanced_image_viewer import SynchronizedImageViewer
from PyQt6.QtWidgets import (
    QMainWindow,
    QWidget,
    QVBoxLayout,
    QHBoxLayout,
    QSplitter,
    QFileDialog,
    QTreeView,
    QPushButton,
    QComboBox,
    QStyle,  # For standard icons
    QAbstractItemView,
    QApplication,  # For selection and edit triggersor dialogs
)
import os
from datetime import date as date_obj
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
from sklearn.metrics.pairwise import cosine_similarity
import sys

from src.core.image_pipeline import ImagePipeline
from src.core.image_file_ops import ImageFileOperations
from src.core.image_processing.raw_image_processor import is_raw_extension

from src.core.metadata_processor import MetadataProcessor  # New metadata processor
from src.core.app_settings import (
    get_preview_cache_size_gb,
    set_preview_cache_size_gb,
    set_exif_cache_size_mb,
    DEFAULT_BLUR_DETECTION_THRESHOLD,
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
from src.ui.selection_utils import select_next_surviving_path
from src.ui.helpers.statusbar_utils import build_status_bar_info
from src.ui.helpers.index_lookup_utils import find_proxy_index_for_path

# build_presentation now used only inside DeletionMarkController
from src.ui.controllers.deletion_mark_controller import DeletionMarkController
from src.ui.controllers.file_deletion_controller import FileDeletionController
from src.ui.controllers.rotation_controller import RotationController
from src.ui.controllers.filter_controller import FilterController
from src.ui.controllers.hotkey_controller import HotkeyController
from src.ui.controllers.navigation_controller import NavigationController
from src.ui.controllers.selection_controller import SelectionController
from src.ui.controllers.similarity_controller import SimilarityController
from src.ui.controllers.preview_controller import PreviewController
from src.ui.controllers.metadata_controller import MetadataController

logger = logging.getLogger(__name__)


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
        self.blur_detection_threshold = DEFAULT_BLUR_DETECTION_THRESHOLD
        self.rotation_suggestions = {}
        # Controllers (always created – treat as invariants for simpler code paths)
        self.deletion_controller = DeletionMarkController(
            app_state=self.app_state,
            is_marked_func=lambda p: self.app_state.is_marked_for_deletion(p),
        )
        self.file_deletion_controller = FileDeletionController(self)
        self.rotation_controller = RotationController(
            rotation_suggestions=self.rotation_suggestions,
            apply_rotations=lambda mapping: self.app_controller._apply_approved_rotations(
                mapping
            ),
        )
        # Navigation & selection controllers use this MainWindow as context
        self.navigation_controller = NavigationController(self)
        self.selection_controller = SelectionController(self)
        self.filter_controller = FilterController(self)
        self.similarity_controller = SimilarityController(self)
        self.preview_controller = PreviewController(self)
        self.metadata_controller = MetadataController(self)

        # Hotkey controller wraps navigation key handling
        self.hotkey_controller = HotkeyController(self)

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

        # At this point _create_widgets() built file_system_model + proxy_model and wired it;
        # it's now safe to apply any deferred FilterController initialization.
        try:
            self.filter_controller.ensure_initialized(
                self.show_folders_mode, self._determine_current_view_mode()
            )
        except Exception as e:
            logger.debug(f"FilterController ensure_initialized skipped: {e}")

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

    def _should_apply_raw_processing(self, file_path: str) -> bool:
        """Determine if RAW processing should be applied to the given file."""
        if not file_path:
            return False
        ext = os.path.splitext(file_path)[1].lower()
        return is_raw_extension(ext)

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
        scan_logically_active = False

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

    # --- Helper for controllers ---
    def _determine_current_view_mode(self) -> str:
        """Return a simple string for current primary view mode.

        Existing code historically used 'grid' vs 'list' naming in filtering logic.
        We infer from which left-panel view is visible. Defaults to 'grid'.
        """
        try:
            if hasattr(self, "left_panel"):
                if self.left_panel.grid_display_view.isVisible():
                    return "grid"
                if self.left_panel.tree_display_view.isVisible():
                    return "list"
        except Exception:
            pass
        return "grid"

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
            self._refresh_current_selection_preview()

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
        # Models
        self.file_system_model = QStandardItemModel()  # Model for file system
        self.proxy_model = CustomFilterProxyModel(self)
        self.proxy_model.setSourceModel(self.file_system_model)
        self.proxy_model.app_state_ref = self.app_state  # Link AppState to proxy model

        # Left panel (views list/tree/rotation)
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
        self.advanced_image_viewer.deleteRequested.connect(self._delete_image)
        self.advanced_image_viewer.deleteMultipleRequested.connect(
            self._delete_multiple_images
        )
        self.advanced_image_viewer.markAsDeletedRequested.connect(
            self._mark_image_for_deletion
        )
        self.advanced_image_viewer.markOthersAsDeletedRequested.connect(
            self._mark_others_for_deletion
        )
        self.advanced_image_viewer.unmarkAsDeletedRequested.connect(
            self._unmark_image_for_deletion
        )
        self.advanced_image_viewer.unmarkOthersAsDeletedRequested.connect(
            self._unmark_others_for_deletion
        )
        # Set the function to check deletion state
        self.advanced_image_viewer.set_is_marked_for_deletion_func(
            self._is_marked_for_deletion
        )
        self.advanced_image_viewer.set_has_any_marked_for_deletion_func(
            self._has_any_marked_for_deletion
        )
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
        self.accept_button.clicked.connect(self._on_accept_button_clicked)
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

            current_sort_method = self.cluster_sort_combo.currentText()
            cluster_info = self.similarity_controller.prepare_clusters(
                current_sort_method
            )
            images_by_cluster = self.similarity_controller.get_images_by_cluster()
            sorted_cluster_ids = cluster_info.get("sorted_cluster_ids", [])
            total_clustered_images = cluster_info.get("total_images", 0)

            if not images_by_cluster:
                no_images_in_clusters = QStandardItem("No images assigned to clusters.")
                no_images_in_clusters.setEditable(False)
                root_item.appendRow(no_images_in_clusters)
                return

            for cluster_id in sorted_cluster_ids:
                cluster_item = QStandardItem(f"Group {cluster_id}")
                cluster_item.setEditable(False)
                cluster_item.setData(
                    f"cluster_header_{cluster_id}", Qt.ItemDataRole.UserRole
                )
                cluster_item.setForeground(QColor(Qt.GlobalColor.gray))
                root_item.appendRow(cluster_item)
                files_in_cluster = images_by_cluster.get(cluster_id, [])
                self._populate_model_standard(cluster_item, files_in_cluster)
            self.statusBar().showMessage(
                f"Grouped {total_clustered_images} images into {len(sorted_cluster_ids)} clusters.",
                3000,
            )
        else:  # Not grouping by similarity
            self._populate_model_standard(root_item, self.app_state.image_files_data)
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
                # selection_controller is always initialized in __init__; simplify fallback logic
                first_index = self.selection_controller.find_first_visible_item()
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

    # --- Controller adapter helpers (SimilarityContext / PreviewContext / MetadataContext) ---
    def status_message(self, msg: str, timeout: int = 3000) -> None:
        self.statusBar().showMessage(msg, timeout)

    def rebuild_model_view(self) -> None:
        self._rebuild_model_view()

    def enable_group_by_similarity(self, enabled: bool) -> None:
        self.menu_manager.group_by_similarity_action.setEnabled(enabled)

    def set_group_by_similarity_checked(self, checked: bool) -> None:
        self.group_by_similarity_mode = checked
        self.menu_manager.group_by_similarity_action.setChecked(checked)

    def set_cluster_sort_visible(self, visible: bool) -> None:
        self.menu_manager.cluster_sort_action.setVisible(visible)

    def enable_cluster_sort_combo(self, enabled: bool) -> None:
        self.cluster_sort_combo.setEnabled(enabled)

    def populate_cluster_filter(self, cluster_ids: List[int]) -> None:
        self.cluster_filter_combo.clear()
        self.cluster_filter_combo.addItems(
            ["All Clusters"] + [f"Cluster {cid}" for cid in cluster_ids]
        )
        self.cluster_filter_combo.setEnabled(bool(cluster_ids))

    def get_selected_file_paths(self) -> List[str]:  # For MetadataController
        # Prefer SelectionController; fall back only if something unexpected occurs
        try:
            return self.selection_controller.get_selected_file_paths()
        except Exception:
            return self._get_selected_file_paths_from_view()

    def ensure_metadata_sidebar(self) -> None:
        if not self.metadata_sidebar:
            try:
                self.metadata_sidebar = MetadataSidebar(self)
            except Exception:
                return

    # --- NavigationContext adapter wrappers ---
    # These expose expected public method names for NavigationController while
    # delegating to existing internal implementations that use leading underscores.
    def get_all_visible_image_paths(
        self,
    ) -> List[str]:  # NavigationController expects this name
        return self._get_all_visible_image_paths()

    def get_active_view(self):  # QAbstractItemView | None
        return self._get_active_file_view()

    def is_valid_image_index(self, proxy_index):  # bool
        return self._is_valid_image_item(proxy_index)

    def map_to_source(self, proxy_index):  # QModelIndex
        active_view = self._get_active_file_view()
        if not active_view:
            from PyQt6.QtCore import QModelIndex

            return QModelIndex()
        model = active_view.model()
        try:
            return model.mapToSource(proxy_index)  # type: ignore[attr-defined]
        except Exception:  # Fallback
            from PyQt6.QtCore import QModelIndex

            return QModelIndex()

    def item_from_source(self, source_index):
        try:
            return self.file_system_model.itemFromIndex(source_index)
        except Exception:
            return None

    def get_group_sibling_images(self, current_proxy_index):
        # Defer to existing internal method if present
        internal = getattr(self, "_get_group_sibling_images", None)
        if callable(internal):
            return internal(current_proxy_index)
        # Fallback shape: (parent_idx, [current_proxy_index], [])
        return None, [current_proxy_index], []

    def find_first_visible_item(self):  # Expected by NavigationController
        # Delegate to SelectionController (let exceptions surface during development)
        return self.selection_controller.find_first_visible_item()

    def find_proxy_index_for_path(self, path: str):  # Expected public name
        try:
            return self._find_proxy_index_for_path(path)
        except Exception:
            from PyQt6.QtCore import QModelIndex

            return QModelIndex()

    def validate_and_select_image_candidate(
        self, proxy_index, direction: str, log_skip: bool
    ):
        validator = getattr(self, "_validate_and_select_image_candidate", None)
        if callable(validator):
            return validator(proxy_index, direction, log_skip)
        # Minimal fallback: set current selection
        active_view = self._get_active_file_view()
        if active_view and proxy_index.isValid():
            sel_model = active_view.selectionModel()
            if sel_model:
                try:
                    flag = getattr(sel_model, "SelectionFlag", None)
                    if flag is not None:
                        sel_model.setCurrentIndex(proxy_index, flag.ClearAndSelect)  # type: ignore[attr-defined]
                    else:
                        sel_model.setCurrentIndex(proxy_index, 0)
                except Exception:
                    pass

    def get_marked_deleted(self):  # Iterable[str] expected by NavigationController
        try:
            return self.app_state.get_marked_files()
        except Exception:
            return []

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

    def _delete_image(self, file_path: str):
        """Delete a single image file."""
        logger.debug(f"Deleting image: {file_path}")
        if not os.path.exists(file_path):
            logger.warning(f"File does not exist: {file_path}")
            return

        # Show confirmation dialog
        if not self.dialog_manager.show_confirm_delete_dialog([file_path]):
            logger.debug("User cancelled deletion")
            return

        # Move to trash
        logger.info(f"Moving file to trash: {file_path}")
        success, message = ImageFileOperations.move_to_trash(file_path)
        if success:
            # Remove from app state
            self.app_state.remove_data_for_path(file_path)
            self.statusBar().showMessage(f"Deleted {os.path.basename(file_path)}", 5000)
            logger.info(f"Successfully deleted: {file_path}")
            # Refresh the view
            self._handle_file_selection_changed()
            # Reapply filters to hide deleted items
            self._apply_filter()
        else:
            self.statusBar().showMessage(
                f"Failed to delete {os.path.basename(file_path)}: {message}", 5000
            )
            logger.error(f"Failed to delete {file_path}: {message}")

    def _delete_multiple_images(self, file_paths: List[str]):
        """Delete multiple image files at once."""
        logger.debug(f"Deleting multiple images: {file_paths}")

        # Filter out non-existent files
        existing_file_paths = [path for path in file_paths if os.path.exists(path)]
        if not existing_file_paths:
            logger.warning("No valid files to delete")
            return

        # Show confirmation dialog for all files at once
        if not self.dialog_manager.show_confirm_delete_dialog(existing_file_paths):
            logger.debug("User cancelled deletion")
            return

        # Delete each file
        deleted_count = 0
        for file_path in existing_file_paths:
            logger.info(f"Moving file to trash: {file_path}")
            success, message = ImageFileOperations.move_to_trash(file_path)
            if success:
                # Remove from app state
                self.app_state.remove_data_for_path(file_path)
                logger.info(f"Successfully deleted: {file_path}")
                deleted_count += 1
            else:
                self.statusBar().showMessage(
                    f"Failed to delete {os.path.basename(file_path)}: {message}", 5000
                )
                logger.error(f"Failed to delete {file_path}: {message}")

        # Show status message
        if deleted_count > 0:
            self.statusBar().showMessage(f"Deleted {deleted_count} image(s)", 5000)
            # Refresh the view
            self._handle_file_selection_changed()
            # Reapply filters to hide deleted items
            self._apply_filter()
        elif len(existing_file_paths) > 0:
            self.statusBar().showMessage("Failed to delete any images", 5000)

    def _log_qmodelindex(self, index: QModelIndex, prefix: str = "") -> str:
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

    # --- SelectionController protocol adapter methods ---
    # These lightweight wrappers let SelectionController interact with the
    # existing MainWindow API without renaming legacy methods yet.

    def is_valid_image_item(self, proxy_index: QModelIndex) -> bool:  # protocol alias
        return self._is_valid_image_item(proxy_index)

    def file_system_model_item_from_index(
        self, source_index: QModelIndex
    ):  # protocol alias
        return (
            self.file_system_model.itemFromIndex(source_index)
            if self.file_system_model
            else None
        )

    def refresh_filter(self):
        # Invalidate proxy model to re-run filtering and sorting.
        try:
            self.proxy_model.invalidate()
        except Exception:
            logger.exception("refresh_filter: failed to invalidate proxy model")

    def resizeEvent(self, event: QResizeEvent):
        super().resizeEvent(event)
        if (
            self.left_panel
            and self.left_panel.current_view_mode == "grid"
            and not self.group_by_similarity_mode
        ):
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
        modifiers = event.modifiers()
        is_macos = sys.platform == "darwin"
        if is_macos:
            has_special = bool(modifiers & Qt.KeyboardModifier.MetaModifier)
        else:
            has_special = bool(modifiers & Qt.KeyboardModifier.ControlModifier)
        skip_deleted = not has_special  # Holding Ctrl/Cmd includes deleted
        if self.hotkey_controller.handle_key(key, skip_deleted=skip_deleted):
            event.accept()
            return

        # Escape key to clear focus from search input (if it has focus)
        if key == Qt.Key.Key_Escape:
            if self.left_panel.search_input.hasFocus():
                self.left_panel.search_input.clearFocus()
                active_view = self._get_active_file_view()
                if active_view:
                    active_view.setFocus(
                        Qt.FocusReason.ShortcutFocusReason
                    )  # Return focus to the view
                event.accept()
                return

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
            if num_images > 1 and index < num_images:
                self.advanced_image_viewer.set_focused_viewer(index)
                return  # Handled

    def _focus_search_input(self):
        self.left_panel.search_input.setFocus()
        self.left_panel.search_input.selectAll()

    def _handle_delete_action(self):
        self._move_current_image_to_trash()

    def _move_current_image_to_trash(self):
        # Delegate to controller
        self.file_deletion_controller.move_current_image_to_trash()

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
        # Check if there are any files marked for deletion
        marked_files = self.app_state.get_marked_files()
        if marked_files:
            logger.info(
                f"Found {len(marked_files)} marked files on close, showing confirmation dialog"
            )
            # Show the close confirmation dialog
            choice = self.dialog_manager.show_close_confirmation_dialog(marked_files)

            if choice == "commit":
                logger.info("User chose to commit deletions on close")
                # Commit the deletions and then close
                self._commit_marked_deletions_without_confirmation()
                # Continue with closing
            elif choice == "ignore":
                logger.info("User chose to ignore deletions on close")
                # Ignore the marked files and close
                # Clear the marked files from app_state
                self.app_state.marked_for_deletion.clear()
                # Update the UI to reflect that files are no longer marked
                self._refresh_visible_items_icons()
                # Continue with closing
            else:
                logger.info("User cancelled close operation")
                # Cancel closing
                event.ignore()
                return

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

        try:
            # Debug information to help verify runtime grouping
            logger.debug(
                "_get_current_group_sibling_images: group_size=%d, cur_local_idx=%s",
                len(sibling_image_items),
                current_item_local_idx,
            )
        except Exception:
            pass
        return parent_proxy_idx, sibling_image_items, current_item_local_idx

    # Backwards-compatible alias used by get_group_sibling_images adapter
    def _get_group_sibling_images(self, current_proxy_index: QModelIndex):
        return self._get_current_group_sibling_images(current_proxy_index)

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

        logger.debug(f"Navigate {direction}: Checking candidate item - Path: {path}")

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

        # Always maintain single-selection on keyboard navigation.
        # Use setCurrentIndex with ClearAndSelect atomically to avoid transient paint/focus issues.
        sel_model = active_view.selectionModel()
        if sel_model is not None:
            sel_model.setCurrentIndex(
                candidate_idx, QItemSelectionModel.SelectionFlag.ClearAndSelect
            )
        active_view.scrollTo(candidate_idx, QAbstractItemView.ScrollHint.EnsureVisible)
        # Ensure the view retains focus so selection remains active (blue) instead of inactive (gray)
        active_view.setFocus(Qt.FocusReason.ShortcutFocusReason)
        # Proactively repaint to reflect the new active selection immediately
        active_view.viewport().update()
        if item:
            logger.debug(f"Navigated {direction} to: {item.text()}")

        return True

    def _navigate_left_in_group(self, skip_deleted: bool = True):
        self.navigation_controller.navigate_group("left", skip_deleted)

    def _navigate_right_in_group(self, skip_deleted: bool = True):
        self.navigation_controller.navigate_group("right", skip_deleted)

    def _navigate_up_sequential(self, skip_deleted: bool = True):
        self.navigation_controller.navigate_linear("up", skip_deleted)

    def _navigate_down_sequential(self, skip_deleted: bool = True):
        self.navigation_controller.navigate_linear("down", skip_deleted)

    # HotkeyController expects context methods without underscores (now accept skip_deleted)
    def navigate_left_in_group(self, skip_deleted: bool = True):
        self._navigate_left_in_group(skip_deleted)

    def navigate_right_in_group(self, skip_deleted: bool = True):
        self._navigate_right_in_group(skip_deleted)

    def navigate_up_sequential(self, skip_deleted: bool = True):
        self._navigate_up_sequential(skip_deleted)

    def navigate_down_sequential(self, skip_deleted: bool = True):
        self._navigate_down_sequential(skip_deleted)

    # Smart down navigation: if currently in a similarity group (more than one image in group)
    # and not at end (or wrap), move within group like horizontal cycle; else fall back to linear down.
    def navigate_down_smart(self, skip_deleted: bool = True):
        self._navigate_group_smart("down", skip_deleted)

    def navigate_up_smart(self, skip_deleted: bool = True):
        """Smart up: cycle backwards within a similarity group, else linear up."""
        self._navigate_group_smart("up", skip_deleted)

    def _navigate_group_smart(self, direction: str, skip_deleted: bool):
        """Shared smart navigation inside a similarity group.

        direction: 'up' or 'down'
        Falls back to sequential navigation if any precondition fails.
        """
        if direction not in ("up", "down"):
            return
        try:
            active_view = self._get_active_file_view()
            if not active_view:
                return (
                    self._navigate_up_sequential(skip_deleted)
                    if direction == "up"
                    else self._navigate_down_sequential(skip_deleted)
                )
            cur_idx = active_view.currentIndex()
            if not cur_idx.isValid():
                return (
                    self._navigate_up_sequential(skip_deleted)
                    if direction == "up"
                    else self._navigate_down_sequential(skip_deleted)
                )
            if not getattr(self, "group_by_similarity_mode", False):
                return (
                    self._navigate_up_sequential(skip_deleted)
                    if direction == "up"
                    else self._navigate_down_sequential(skip_deleted)
                )
            _parent_group_idx, group_indices, _ = self.get_group_sibling_images(cur_idx)
            if not group_indices or len(group_indices) <= 1:
                return (
                    self._navigate_up_sequential(skip_deleted)
                    if direction == "up"
                    else self._navigate_down_sequential(skip_deleted)
                )
            try:
                pos = group_indices.index(cur_idx)
            except ValueError:
                return (
                    self._navigate_up_sequential(skip_deleted)
                    if direction == "up"
                    else self._navigate_down_sequential(skip_deleted)
                )
            step = 1 if direction == "down" else -1
            target_pos = (pos + step) % len(group_indices)
            if target_pos == pos:
                return  # single element guard
            target_proxy_idx = group_indices[target_pos]
            if target_proxy_idx.isValid():
                self.validate_and_select_image_candidate(
                    target_proxy_idx, direction, not skip_deleted
                )
            else:
                (
                    self._navigate_up_sequential
                    if direction == "up"
                    else self._navigate_down_sequential
                )(skip_deleted)
        except Exception:
            (
                self._navigate_up_sequential
                if direction == "up"
                else self._navigate_down_sequential
            )(skip_deleted)

    # Removed obsolete _index_below/_index_above and last-visible item logic (moved to SelectionController or no longer needed)

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
        active_view = self._get_active_file_view()
        if not active_view:
            return QModelIndex()
        proxy_model = active_view.model()
        if not isinstance(proxy_model, QSortFilterProxyModel):
            return QModelIndex()
        return find_proxy_index_for_path(
            target_path=target_path,
            proxy_model=proxy_model,
            source_model=self.file_system_model,
            is_valid_image_item=self._is_valid_image_item,
            is_expanded=(lambda idx: active_view.isExpanded(idx))
            if isinstance(active_view, QTreeView)
            else None,
        )

    def _get_selected_file_paths_from_view(self) -> List[str]:
        """Return list of selected file paths using SelectionController.

        Previous inlined implementation lived here; now delegated for clarity.
        """
        return self.selection_controller.get_selected_file_paths()

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
            logger.warning(f"File not found: {file_path}")
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

        logger.debug(f"Getting preview pixmap for {file_path}")
        pixmap = self.image_pipeline.get_preview_qpixmap(
            file_path,
            display_max_size=(8000, 8000),
            apply_auto_edits=self._should_apply_raw_processing(file_path),
        )

        if not pixmap or pixmap.isNull():
            logger.debug(
                f"Preview pixmap unavailable, trying thumbnail for {file_path}"
            )
            pixmap = self.image_pipeline.get_thumbnail_qpixmap(
                file_path, apply_auto_edits=self._should_apply_raw_processing(file_path)
            )

        if pixmap and not pixmap.isNull():
            logger.debug(f"Setting image data for {file_path}")
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
            logger.debug(f"Failed to load image data for {file_path}")
            self.advanced_image_viewer.setText("Failed to load image")
            self.statusBar().showMessage(
                f"Error: Could not load image data for {os.path.basename(file_path)}",
                7000,
            )

        if self.sidebar_visible:
            logger.debug("Updating sidebar with current selection")
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
            apply_auto_edits=self._should_apply_raw_processing(file_path),
        )

        if not pixmap or pixmap.isNull():
            pixmap = self.image_pipeline.get_thumbnail_qpixmap(
                file_path, apply_auto_edits=self._should_apply_raw_processing(file_path)
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
        info = build_status_bar_info(
            file_path=file_path,
            metadata=metadata,
            width=pixmap.width() if pixmap else 0,
            height=pixmap.height() if pixmap else 0,
            cluster_lookup=self.app_state.cluster_results,
            file_data_from_model=file_data_from_model,
        )
        self.statusBar().showMessage(info.to_message())

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
                apply_auto_edits=self._should_apply_raw_processing(path),
            )
            if not pixmap or pixmap.isNull():
                pixmap = self.image_pipeline.get_thumbnail_qpixmap(
                    path, apply_auto_edits=self._should_apply_raw_processing(path)
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
        logger.debug("_handle_no_selection_or_non_image called")
        if not self.app_state.image_files_data:
            logger.debug("No image files data, returning early")
            return

        # Clear focused image path and repaint view to remove underline
        if self.app_state.focused_image_path:
            logger.debug("Clearing focused image path")
            self.app_state.focused_image_path = None
            self._get_active_file_view().viewport().update()

        logger.debug("Clearing viewer and setting 'Select an image' text")
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
            logger.debug("_handle_file_selection_changed: Skipping due to sync lock")
            return

        if override_selected_paths is not None:
            selected_file_paths = override_selected_paths
            logger.debug(
                f"_handle_file_selection_changed: Using overridden selection of {len(selected_file_paths)} paths"
            )
        else:
            selected_file_paths = self._get_selected_file_paths_from_view()
            logger.debug(
                f"_handle_file_selection_changed: Retrieved {len(selected_file_paths)} paths from view"
            )

        if not self.app_state.image_files_data:
            logger.debug(
                "_handle_file_selection_changed: No image files data available"
            )
            return

        # In rotation view, update the accept/refuse buttons based on selection
        if self.left_panel.current_view_mode == "rotation":
            num_suggestions = len(self.rotation_suggestions)
            logger.debug(f"Rotation view with {num_suggestions} suggestions")
            self.accept_all_button.setVisible(num_suggestions > 1)
            self.refuse_all_button.setVisible(num_suggestions > 1)
            num_selected = len(selected_file_paths)

            self.accept_button.setVisible(num_selected > 0)
            self.refuse_button.setVisible(num_selected > 0)

            if num_selected > 0:
                all_selected_have_suggestion = all(
                    p in self.rotation_suggestions for p in selected_file_paths
                )
                logger.debug(
                    f"Selected items have suggestions: {all_selected_have_suggestion}"
                )
                self.accept_button.setEnabled(all_selected_have_suggestion)
                self.refuse_button.setEnabled(all_selected_have_suggestion)

                if num_selected == 1:
                    logger.debug(
                        f"Displaying side-by-side comparison for: {selected_file_paths[0]}"
                    )
                    self.accept_button.setText("Accept (Y)")
                    self.refuse_button.setText("Refuse (N)")
                    self._display_side_by_side_comparison(selected_file_paths[0])
                else:
                    logger.debug(
                        f"Displaying multi-selection info for {num_selected} items"
                    )
                    self.accept_button.setText(f"Accept ({num_selected})")
                    self.refuse_button.setText(f"Refuse ({num_selected})")
                    self.advanced_image_viewer.clear()
                    self.advanced_image_viewer.setText(
                        f"{num_selected} items selected for rotation approval."
                    )
            else:
                logger.debug("No items selected in rotation view")
                self.advanced_image_viewer.clear()
            return
        else:
            logger.debug("Not in rotation view, hiding rotation buttons")
            self.accept_all_button.setVisible(False)
            self.accept_button.setVisible(False)
            self.refuse_button.setVisible(False)
            self.refuse_all_button.setVisible(False)

        # When selection changes, clear the focused image path unless it's a single selection
        if len(selected_file_paths) != 1:
            logger.debug(f"Selection is not single (count={len(selected_file_paths)})")
            if self.app_state.focused_image_path:
                logger.debug("Clearing focused image path")
                self.app_state.focused_image_path = None
                active_view = self._get_active_file_view()
                if active_view:
                    active_view.viewport().update()  # Trigger repaint to remove underline

        if len(selected_file_paths) == 1:
            file_path = selected_file_paths[0]
            # Avoid logging full path each change; keep concise
            logger.debug("Handling single selection")
            # This is a single selection, so it's also the "focused" image.
            self.app_state.focused_image_path = file_path
            active_view = self._get_active_file_view()
            if active_view:
                active_view.viewport().update()

            file_data_from_model = self._get_cached_metadata_for_selection(file_path)
            logger.debug(f"Displaying single image preview for: {file_path}")
            self._display_single_image_preview(file_path, file_data_from_model)

        elif len(selected_file_paths) >= 2:
            logger.debug(f"Handling multi-selection (count={len(selected_file_paths)})")
            self._display_multi_selection_info(selected_file_paths)

        else:  # No selection
            logger.debug("Handling no selection")
            self._handle_no_selection_or_non_image()
            if self.sidebar_visible and self.metadata_sidebar:
                self.metadata_sidebar.show_placeholder()
        # Always allow MetadataController to update (it internally caches selection)
        try:
            self.metadata_controller.refresh_for_selection()
        except Exception:
            pass

    def _apply_filter(self):
        # Guard: Don't apply filters if no images are loaded yet
        if not self.app_state.image_files_data:
            logger.debug("Filter skipped: No images loaded.")
            return
        search_text = self.left_panel.search_input.text()
        logger.info(f"Applying filters. Search term: '{search_text}'")
        self.filter_controller.set_search_text(search_text)
        self.filter_controller.set_rating_filter(self.filter_combo.currentText())
        cluster_text = self.cluster_filter_combo.currentText()
        cluster_id = -1
        if self.cluster_filter_combo.isEnabled() and cluster_text != "All Clusters":
            try:
                cluster_id = int(cluster_text.split(" ")[-1])
            except ValueError:
                cluster_id = -1
        self.filter_controller.set_cluster_filter(cluster_id)
        self.filter_controller.apply_all(
            show_folders=self.show_folders_mode,
            current_view_mode=self.left_panel.current_view_mode,
        )
        # Ensure proxy uses desired roles/columns
        self.proxy_model.setFilterKeyColumn(-1)
        self.proxy_model.setFilterRole(Qt.ItemDataRole.DisplayRole)
        self.proxy_model.invalidateFilter()

    def _start_preview_preloader(self, image_data_list: List[Dict[str, any]]):
        try:
            if logger.isEnabledFor(logging.DEBUG):
                logger.debug(
                    f"Delegating preview preload: {len(image_data_list)} items"
                )
            self.preview_controller.start_preload(image_data_list)
        except Exception as e:
            logger.error(f"PreviewController error: {e}")

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
        self.statusBar().showMessage(
            "Previews regenerated.", 5000
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

        if self.left_panel.current_view_mode == "list":
            self.left_panel.set_view_mode_list()
        elif self.left_panel.current_view_mode == "icons":
            self.left_panel.set_view_mode_icons()
        elif self.left_panel.current_view_mode == "date":
            self.left_panel.set_view_mode_date()
        else:
            # Fallback to ensure model is rebuilt at least once
            self._rebuild_model_view()

        # After the view mode setter has applied TreeView properties and rebuilt model,
        # expand all groups if folder mode is active.
        if self.show_folders_mode and not self.group_by_similarity_mode:

            def _expand_after_layout():
                try:
                    active_view = self._get_active_file_view()
                    if isinstance(active_view, QTreeView):
                        # Ensure expand/collapse toggles are enabled for groups
                        active_view.setItemsExpandable(True)
                        active_view.setRootIsDecorated(True)
                        active_view.expandAll()
                except Exception as e:
                    logger.warning(f"Failed to auto-expand folders after layout: {e}")

            QTimer.singleShot(0, _expand_after_layout)

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
                file_path, apply_auto_edits=self._should_apply_raw_processing(file_path)
            )
            if thumbnail_pixmap:
                item.setIcon(QIcon(thumbnail_pixmap))

        # Unified presentation (marked / blurred) delegated to deletion controller
        self.deletion_controller.apply_presentation(item, file_path, is_blurred)

        return item

    # _update_item_deletion_blur_presentation removed (inlined via deletion_controller)

    def _start_similarity_analysis(self):
        logger.info("_start_similarity_analysis delegated to SimilarityController")
        paths = [
            fd.get("path")
            for fd in (self.app_state.image_files_data or [])
            if fd.get("path")
        ]
        self.similarity_controller.start(paths)  # Automatic RAW processing based on file detection

    # Slot for WorkerManager's similarity_progress signal
    def _handle_similarity_progress(self, percentage, message):
        self.update_loading_text(f"Similarity: {message} ({percentage}%)")

    # Slot for WorkerManager's similarity_embeddings_generated signal
    def _handle_embeddings_generated(self, embeddings_dict):
        self.similarity_controller.embeddings_generated(embeddings_dict)

    # Slot for WorkerManager's similarity_clustering_complete signal
    def _handle_clustering_complete(self, cluster_results_dict: Dict[str, int]):
        self.similarity_controller.clustering_complete(cluster_results_dict, True)

    # Slot for WorkerManager's similarity_error signal
    def _handle_similarity_error(self, message):
        self.similarity_controller.error(message)

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
            True,  # Always enable processing for RAW files
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
            images_by_cluster = self.similarity_controller.get_images_by_cluster()
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
                active_view.setFocus(Qt.FocusReason.ShortcutFocusReason)
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
                                # Prefer the single-item flow that advances selection; if multi-selected, fall back
                                sel = self._get_selected_file_paths_from_view()
                                if sel and len(sel) == 1:
                                    self._accept_single_rotation_and_move_to_next()
                                else:
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
                    # On Mac, arrow keys often have KeypadModifier, so treat that as unmodified too
                    is_unmodified_or_keypad = modifiers in (
                        Qt.KeyboardModifier.NoModifier,
                        Qt.KeyboardModifier.KeypadModifier,
                    )

                    # Platform-aware modifier mapping:
                    # - macOS: Cmd (Meta) is the special modifier
                    # - Windows/Linux: Ctrl is the special modifier
                    is_macos = sys.platform == "darwin"

                    if is_macos:
                        is_special_exact = modifiers == Qt.KeyboardModifier.MetaModifier
                        has_special = bool(modifiers & Qt.KeyboardModifier.MetaModifier)
                    else:
                        # Explicitly avoid treating Alt as special on Windows/Linux
                        is_special_exact = (
                            modifiers == Qt.KeyboardModifier.ControlModifier
                        )
                        has_special = bool(
                            modifiers & Qt.KeyboardModifier.ControlModifier
                        )

                    # (Removed deprecated is_control_or_meta_exact alias)

                    # Rating shortcuts (Ctrl on Win/Linux, Cmd on macOS) + 0-5
                    # MUST be an exact modifier match.
                    if is_special_exact and Qt.Key.Key_0 <= key <= Qt.Key.Key_5:
                        rating = key - Qt.Key.Key_0
                        self._apply_rating_to_selection(rating)
                        return True

                    # --- Arrow Key Navigation ---
                    # For navigation, the platform's special modifier has precedence for
                    # "modified" navigation (include deleted), even with Shift.
                    if has_special:
                        # When the platform special modifier is held, include deleted (skip_deleted=False)
                        if hasattr(
                            self, "hotkey_controller"
                        ) and self.hotkey_controller.handle_key(
                            key, skip_deleted=False
                        ):
                            return True
                    elif is_unmodified_or_keypad:
                        if hasattr(
                            self, "hotkey_controller"
                        ) and self.hotkey_controller.handle_key(key, skip_deleted=True):
                            return True
                        if key == Qt.Key.Key_Delete or key == Qt.Key.Key_Backspace:
                            self._handle_delete_action()
                            return True
                    else:
                        logger.debug(
                            f"Key with other modifiers detected (passing to default handler): {key}, modifiers: {modifiers}"
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
        if not self.main_splitter or not self.metadata_sidebar:
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
                apply_auto_edits=self._should_apply_raw_processing(selected_paths[0]),
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
                    apply_auto_edits=self._should_apply_raw_processing(path),
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
        logger.info(
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
                    file_path, apply_auto_edits=self._should_apply_raw_processing(file_path)
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
                apply_auto_edits=self._should_apply_raw_processing(file_path),
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
        """Checks if a file is marked for deletion."""
        is_marked = self.app_state.is_marked_for_deletion(file_path)
        logger.debug(f"Checking deletion mark for '{file_path}': {is_marked}")
        return is_marked

    def _has_any_marked_for_deletion(self) -> bool:
        """Checks if there are any files marked for deletion."""
        has_marked = len(self.app_state.get_marked_files()) > 0
        logger.debug(f"Checking if any files are marked for deletion: {has_marked}")
        return has_marked

    def _commit_marked_deletions(self):
        """Finds all marked files and moves them to trash with confirmation, updating the view in-place."""
        logger.info("Starting commit marked deletions with confirmation")

        active_view = self._get_active_file_view()
        if not self.app_state.current_folder_path or not active_view:
            self.statusBar().showMessage("No folder loaded.", 3000)
            logger.info("No folder loaded, aborting commit marked deletions")
            return

        marked_files = self.app_state.get_marked_files()
        if not marked_files:
            self.statusBar().showMessage("No images are marked for deletion.", 3000)
            logger.info(
                "No images marked for deletion, aborting commit marked deletions"
            )
            return

        if not self.dialog_manager.show_commit_deletions_dialog(marked_files):
            logger.info("User cancelled commit deletions dialog")
            return

        logger.info(f"User confirmed deletion of {len(marked_files)} files")
        self._perform_deletion_of_marked_files(marked_files)

    def _perform_deletion_of_marked_files(self, marked_files: List[str]):
        """Performs the actual deletion of marked files, updating the view in-place."""
        active_view = self._get_active_file_view()
        if not active_view:
            return

        # --- Pre-computation for next selection ---
        visible_paths_before = self._get_all_visible_image_paths()
        logger.debug(f"Visible paths before deletion: {visible_paths_before}")
        logger.debug(f"Marked files for deletion: {marked_files}")

        # Find the index of the first marked file in the visible list
        first_marked_index = -1
        if visible_paths_before and marked_files:
            try:
                first_marked_index = visible_paths_before.index(marked_files[0])
                logger.debug(f"First marked file index: {first_marked_index}")
            except ValueError:
                first_marked_index = 0
                logger.debug(
                    "First marked file not found in visible paths, using index 0"
                )

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
                logger.info(f"Moved file to trash: {os.path.basename(file_path)}")
            except Exception as e:
                logger.error(f"Error moving marked file '{file_path}' to trash: {e}")

        # Clear the marked files from app state after successful deletion
        self.app_state.clear_all_deletion_marks()

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

            # --- Select next item using robust advancement to next valid image ---
            visible_paths_after_delete = self._get_all_visible_image_paths()
            logger.debug(
                f"{len(visible_paths_after_delete)} visible paths remaining after deletion."
            )
            logger.debug("Visible paths after deletion list suppressed for brevity")

            # Determine the anchor path for selection after deletion
            # This determines which image position to use as reference for finding the next selection
            current_selected_path_before = (
                self.app_state.focused_image_path
                or self._get_current_selected_image_path()
            )

            # If the current selection is one of the deleted files, use it as anchor
            # This will ensure we select the next image after the deleted one
            if current_selected_path_before in marked_files:
                anchor_path = current_selected_path_before
            # If there are deleted files and current selection is not one of them,
            # use the first deleted file as anchor for better UX
            # This handles cases where user marks files for deletion without having them selected
            elif marked_files:
                anchor_path = marked_files[
                    0
                ]  # Use first deleted file as reference point
            # Fallback to current selection
            else:
                anchor_path = current_selected_path_before

            logger.debug(
                f"Current selected (focused) path before deletion: {current_selected_path_before}"
            )
            logger.debug(f"Anchor path for selection after deletion: {anchor_path}")

            if not visible_paths_after_delete:
                logger.debug("No visible image items left after deletion.")
                self.advanced_image_viewer.clear()
                self.advanced_image_viewer.setText("No images left to display.")
                self.statusBar().showMessage("No images left or visible.")
            else:
                # Always find the best next selection. The function is smart enough
                # to keep the current selection if it's still valid.
                logger.debug("Finding next selection after deletion.")
                next_path = select_next_surviving_path(
                    visible_paths_before,
                    marked_files,
                    anchor_path,
                    visible_paths_after_delete,
                )

                if next_path:
                    next_proxy_idx = self._find_proxy_index_for_path(next_path)
                    if next_proxy_idx.isValid():
                        logger.debug("Selecting next path after deletion")
                        active_view.setCurrentIndex(next_proxy_idx)
                        active_view.selectionModel().select(
                            next_proxy_idx,
                            QItemSelectionModel.SelectionFlag.ClearAndSelect,
                        )
                        active_view.scrollTo(
                            next_proxy_idx,
                            QAbstractItemView.ScrollHint.EnsureVisible,
                        )
                        # The selection change will trigger the preview update.
                        # We might need to manually trigger if the selection doesn't change
                        # but this is safer.
                        QTimer.singleShot(0, self._handle_file_selection_changed)
                    else:
                        logger.warning(
                            f"Could not find a valid proxy index for the next path: {next_path}"
                        )
                        self.advanced_image_viewer.clear()
                        self.advanced_image_viewer.setText(
                            "Could not select next image."
                        )
                else:
                    logger.debug("No next valid path found; clearing UI.")
                    self.advanced_image_viewer.clear()
                    self.advanced_image_viewer.setText("No valid image to select.")

            self._update_image_info_label()

        logger.info(f"Completed committing {deleted_count} deletions")

    def _commit_marked_deletions_without_confirmation(self):
        """Finds all marked files and moves them to trash without confirmation, updating the view in-place."""
        active_view = self._get_active_file_view()
        if not self.app_state.current_folder_path or not active_view:
            self.statusBar().showMessage("No folder loaded.", 3000)
            return

        marked_files = self.app_state.get_marked_files()
        if not marked_files:
            self.statusBar().showMessage("No images are marked for deletion.", 3000)
            return

        logger.info(
            f"Committing {len(marked_files)} marked deletions without confirmation"
        )
        self._perform_deletion_of_marked_files(marked_files)

    def _mark_selection_for_deletion(self):
        """Toggles the deletion mark for selected files, updating the model in-place."""
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

        marked_count = self.deletion_controller.toggle_paths(
            paths_to_act_on,
            self._find_proxy_index_for_path,
            self.file_system_model,
            self.proxy_model,
        )

        self.statusBar().showMessage(f"Toggled mark for {marked_count} image(s).", 5000)
        self.proxy_model.invalidate()
        QApplication.processEvents()

        # Keep the same selection behavior
        self._handle_file_selection_changed(
            override_selected_paths=original_selection_paths
        )

    def _mark_image_for_deletion(self, file_path: str):
        """Marks a single image for deletion, updating the model in-place."""
        if not file_path:
            return

        # Mark the file for deletion in the app state
        is_marked = self._is_marked_for_deletion(file_path)
        if is_marked:
            self.app_state.unmark_for_deletion(file_path)
        else:
            self.app_state.mark_for_deletion(file_path)

        # Update the UI
        self.deletion_controller.toggle_paths(
            [file_path],
            self._find_proxy_index_for_path,
            self.file_system_model,
            self.proxy_model,
        )

        self.statusBar().showMessage("Marked 1 image for deletion.", 5000)
        self.proxy_model.invalidate()
        QApplication.processEvents()

    def _mark_others_for_deletion(self, file_path_to_keep: str):
        """Marks all other images in the split view for deletion, updating the model in-place."""
        if not file_path_to_keep:
            return

        # Get all image file paths from the split view
        all_image_paths = []
        for viewer in self.advanced_image_viewer.image_viewers:
            if viewer._file_path is not None:
                all_image_paths.append(viewer._file_path)

        # Filter out the file path to keep
        paths_to_mark = [path for path in all_image_paths if path != file_path_to_keep]

        if not paths_to_mark:
            self.statusBar().showMessage("No other images to mark for deletion.", 3000)
            return

        marked_count = self.deletion_controller.mark_others_in_collection(
            file_path_to_keep,
            paths_to_mark,
            self._find_proxy_index_for_path,
            self.file_system_model,
            self.proxy_model,
        )

        self.statusBar().showMessage(
            f"Marked {marked_count} other image(s) for deletion.", 5000
        )
        self.proxy_model.invalidate()
        QApplication.processEvents()

    def _unmark_image_for_deletion(self, file_path: str):
        """Unmarks a single image for deletion, updating the model in-place."""
        if not file_path:
            return

        # Check if the file is actually marked for deletion
        if not self._is_marked_for_deletion(file_path):
            self.statusBar().showMessage("Image is not marked for deletion.", 3000)
            return

        # Unmark the file for deletion in the app state
        self.app_state.unmark_for_deletion(file_path)

        self.deletion_controller.toggle_paths(
            [file_path],
            self._find_proxy_index_for_path,
            self.file_system_model,
            self.proxy_model,
        )

        self.statusBar().showMessage("Unmarked 1 image for deletion.", 5000)
        self.proxy_model.invalidate()
        QApplication.processEvents()

    def _unmark_others_for_deletion(self, file_path_to_keep: str):
        """Unmarks all other images in the split view for deletion, updating the model in-place."""
        if not file_path_to_keep:
            return

        # Get all image file paths from the split view
        all_image_paths = []
        for viewer in self.advanced_image_viewer.image_viewers:
            if viewer._file_path is not None:
                all_image_paths.append(viewer._file_path)

        # Filter out the file path to keep
        paths_to_unmark = [
            path for path in all_image_paths if path != file_path_to_keep
        ]

        if not paths_to_unmark:
            self.statusBar().showMessage(
                "No other images to unmark for deletion.", 3000
            )
            return

        unmarked_count = self.deletion_controller.unmark_others_in_collection(
            file_path_to_keep,
            paths_to_unmark,
            self._find_proxy_index_for_path,
            self.file_system_model,
            self.proxy_model,
        )

        self.statusBar().showMessage(
            f"Unmarked {unmarked_count} other image(s) for deletion.", 5000
        )
        self.proxy_model.invalidate()
        QApplication.processEvents()

    def _clear_all_deletion_marks(self):
        """Unmarks all marked files, updating the view in-place."""
        if not self.app_state.current_folder_path:
            self.statusBar().showMessage("No folder loaded.", 3000)
            return

        marked_files = self.app_state.get_marked_files()
        if not marked_files:
            self.statusBar().showMessage("No images are marked for deletion.", 3000)
            return

        self.deletion_controller.clear_all_and_update(
            self._find_proxy_index_for_path,
            self.file_system_model,
            self.proxy_model,
        )

        self.statusBar().showMessage(
            f"Cleared deletion marks for {len(marked_files)} image(s).", 5000
        )
        self.proxy_model.invalidate()
        QApplication.processEvents()

        visible_paths = self._get_all_visible_image_paths()
        if not visible_paths:
            self.advanced_image_viewer.clear()
            return

        first_path = marked_files[0]
        first_proxy_idx = self._find_proxy_index_for_path(first_path)
        if first_proxy_idx.isValid():
            active_view = self._get_active_file_view()
            if active_view:
                active_view.setCurrentIndex(first_proxy_idx)
                active_view.selectionModel().select(
                    first_proxy_idx, QItemSelectionModel.SelectionFlag.ClearAndSelect
                )
                active_view.scrollTo(
                    first_proxy_idx, QAbstractItemView.ScrollHint.EnsureVisible
                )

        final_selection_paths = [path for path in marked_files]
        self._handle_file_selection_changed(
            override_selected_paths=final_selection_paths
        )

        if first_path:
            self.advanced_image_viewer.set_focused_viewer_by_path(first_path)

        selection = QItemSelection()
        first_idx = QModelIndex()
        for path in final_selection_paths:
            proxy_idx = self._find_proxy_index_for_path(path)
            if proxy_idx.isValid():
                selection.select(proxy_idx, proxy_idx)
                if not first_idx.isValid():
                    first_idx = proxy_idx

        if not selection.isEmpty():
            active_view = self._get_active_file_view()
            if active_view:
                active_view.selectionModel().blockSignals(True)
                active_view.selectionModel().select(
                    selection, QItemSelectionModel.SelectionFlag.ClearAndSelect
                )
                active_view.selectionModel().blockSignals(False)
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
            active_view.setFocus(Qt.FocusReason.ShortcutFocusReason)

            # Reset the flag after the event queue is cleared to prevent loops
            QTimer.singleShot(0, lambda: setattr(self, "_is_syncing_selection", False))

    def _update_item_blur_status(self, image_path: str, is_blurred: bool):
        self.deletion_controller.update_blur_status(
            image_path,
            is_blurred,
            self._find_proxy_index_for_path,
            self.file_system_model,
            self.proxy_model,
            self.left_panel.get_active_view,
            lambda: self._handle_file_selection_changed(),
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
            file_path, (8000, 8000), apply_auto_edits=self._should_apply_raw_processing(file_path)
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
        """Apply all suggested rotations and exit rotation view."""
        if not self.rotation_controller.has_suggestions():
            self.statusBar().showMessage("No rotation suggestions to accept.", 3000)
            return
        self.rotation_controller.accept_all()
        self._hide_rotation_view()

    def _accept_current_rotation(self):
        selected_paths = self._get_selected_file_paths_from_view()
        if not selected_paths:
            return
        target_paths = [
            p
            for p in selected_paths
            if p in self.rotation_controller.rotation_suggestions
        ]
        if not target_paths:
            return
        visible_before = self.rotation_controller.get_visible_order()
        accepted = self.rotation_controller.accept_paths(target_paths)
        if not self.rotation_controller.has_suggestions():
            self._hide_rotation_view()
            return
        next_path = self.rotation_controller.compute_next_after_accept(
            visible_before, accepted, accepted[0] if accepted else None
        )
        self._rebuild_rotation_view()
        if next_path:
            proxy_idx = self._find_proxy_index_for_path(next_path)
            if proxy_idx.isValid():
                active_view = self._get_active_file_view()
                if active_view:
                    active_view.setCurrentIndex(proxy_idx)
                    active_view.selectionModel().select(
                        proxy_idx, QItemSelectionModel.SelectionFlag.ClearAndSelect
                    )
                    active_view.scrollTo(
                        proxy_idx, QAbstractItemView.ScrollHint.EnsureVisible
                    )
                    return
        active_view = self._get_active_file_view()
        if active_view:
            active_view.selectionModel().clear()
        self.advanced_image_viewer.clear()
        self.accept_button.setVisible(False)

    # (Legacy nested _accept_rotation removed; logic handled by controller methods above.)

    def _on_accept_button_clicked(self):
        """Handle accept button click with automatic navigation in rotation view."""
        # Check if we're in rotation view mode
        if self.left_panel.current_view_mode == "rotation":
            # Use the new method that automatically moves to the next item
            self._accept_single_rotation_and_move_to_next()
        else:
            # Use the standard method for other views
            self._accept_current_rotation()

    def _accept_single_rotation_and_move_to_next(self):
        """Applies a single rotation suggestion and automatically moves to the next item."""
        # Get the currently selected path
        selected_paths = self._get_selected_file_paths_from_view()
        if not selected_paths or len(selected_paths) != 1:
            # If not exactly one item selected, fall back to the standard accept behavior
            self._accept_current_rotation()
            return
        file_path = selected_paths[0]
        if file_path not in self.rotation_controller.rotation_suggestions:
            return
        # Capture current visible order to compute the best next candidate
        try:
            visible_paths_before = self._get_all_visible_image_paths()
        except Exception:
            visible_paths_before = self.rotation_controller.get_visible_order()
        accepted = self.rotation_controller.accept_paths([file_path])
        if not self.rotation_controller.has_suggestions():
            self._hide_rotation_view()
            return
        # Rebuild the view, then compute the next selection
        self._rebuild_rotation_view()
        path_to_select = self.rotation_controller.compute_next_after_accept(
            visible_paths_before, accepted, file_path
        )
        active_view = self._get_active_file_view()
        if path_to_select and active_view:
            proxy_idx_to_select = self._find_proxy_index_for_path(path_to_select)
            if proxy_idx_to_select.isValid():
                active_view.setCurrentIndex(proxy_idx_to_select)
                active_view.selectionModel().select(
                    proxy_idx_to_select,
                    QItemSelectionModel.SelectionFlag.ClearAndSelect,
                )
                active_view.scrollTo(
                    proxy_idx_to_select,
                    QAbstractItemView.ScrollHint.EnsureVisible,
                )
                return
        # Fallback: clear selection and preview if we couldn't determine the next
        if active_view:
            active_view.selectionModel().clear()
        self.advanced_image_viewer.clear()
        self.accept_button.setVisible(False)
        self.refuse_button.setVisible(False)

    def _refuse_all_rotations(self):
        """Refuses all remaining rotation suggestions."""
        if not self.rotation_controller.has_suggestions():
            self.statusBar().showMessage("No rotation suggestions to refuse.", 3000)
            return
        self.rotation_controller.refuse_all()
        self.statusBar().showMessage(
            "All rotation suggestions have been refused.", 5000
        )
        self._hide_rotation_view()

    def _refuse_current_rotation(self):
        """Refuses the currently selected rotation suggestions."""
        selected_paths = self._get_selected_file_paths_from_view()
        if not selected_paths:
            return
        target_paths = [
            p
            for p in selected_paths
            if p in self.rotation_controller.rotation_suggestions
        ]
        if not target_paths:
            return
        self.rotation_controller.refuse_paths(target_paths)
        if not self.rotation_controller.has_suggestions():
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

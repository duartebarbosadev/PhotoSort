import logging

logger = logging.getLogger(__name__)
import os
import subprocess
from typing import TYPE_CHECKING

from PyQt6.QtCore import QPoint, Qt
from PyQt6.QtGui import QAction, QIcon, QKeySequence
from PyQt6.QtWidgets import QMenu, QStyle, QWidget, QWidgetAction, QHBoxLayout, QLabel

from src.core.metadata_processor import MetadataProcessor
from src.core.app_settings import get_recent_folders

if TYPE_CHECKING:
    from src.ui.main_window import MainWindow


class MenuManager:
    """Manages the creation of menus and actions for the main window."""

    def __init__(self, main_window: "MainWindow"):
        self.main_window = main_window
        self.dialog_manager = main_window.dialog_manager
        self.app_state = main_window.app_state

        # --- Actions ---
        # File Menu
        self.open_folder_action: QAction
        self.open_recent_menu: QMenu
        self.exit_action: QAction

        # View Menu
        self.toggle_folder_view_action: QAction
        self.group_by_similarity_action: QAction
        self.toggle_thumbnails_action: QAction
        self.analyze_similarity_action: QAction
        self.detect_blur_action: QAction
        self.auto_rotate_action: QAction
        self.toggle_metadata_sidebar_action: QAction

        # Filter Menu
        self.cluster_sort_action: QWidgetAction

        # Settings Menu
        self.manage_cache_action: QAction
        self.toggle_auto_edits_action: QAction
        self.toggle_mark_for_deletion_action: QAction

        # Image Menu
        self.rotate_clockwise_action: QAction
        self.rotate_counterclockwise_action: QAction
        self.rotate_180_action: QAction
        self.mark_for_delete_action: QAction
        self.unmark_for_delete_action: QAction
        self.commit_deletions_action: QAction
        self.clear_marked_deletions_action: QAction

        # Viewer Actions
        self.zoom_in_action: QAction
        self.zoom_out_action: QAction
        self.fit_to_view_action: QAction
        self.actual_size_action: QAction
        self.single_view_action: QAction
        self.side_by_side_view_action: QAction
        self.sync_pan_zoom_action: QAction

        # Other global actions
        self.find_action: QAction
        self.about_action: QAction
        self.rating_actions: dict[int, QAction] = {}
        self.image_focus_actions: dict[int, QAction] = {}

    def create_menus(self, menu_bar):
        """Creates all menus and actions."""
        logger.debug("Creating menus...")
        menu_bar.setNativeMenuBar(True)
        self._create_actions()

        # Create menus
        self._create_file_menu(menu_bar)
        self._create_view_menu(menu_bar)
        self._create_filter_menu(menu_bar)
        self._create_image_menu(menu_bar)  # Must be after _create_actions are created
        self._create_settings_menu(menu_bar)
        self._create_help_menu(menu_bar)

        logger.debug("Menus created.")

    def _create_actions(self):
        """Create all QActions for the application."""
        main_win = self.main_window
        logger.debug("Creating actions...")

        self.find_action = QAction("Find", main_win)
        self.find_action.setShortcut(QKeySequence.StandardKey.Find)
        self.find_action.setShortcutContext(Qt.ShortcutContext.ApplicationShortcut)
        main_win.addAction(self.find_action)

        # Rotation Actions
        self.rotate_clockwise_action = QAction("Rotate Clockwise", main_win)
        self.rotate_clockwise_action.setShortcut(QKeySequence("R"))
        self.rotate_clockwise_action.setShortcutContext(
            Qt.ShortcutContext.ApplicationShortcut
        )
        main_win.addAction(self.rotate_clockwise_action)

        self.rotate_counterclockwise_action = QAction(
            "Rotate Counterclockwise", main_win
        )
        self.rotate_counterclockwise_action.setShortcut(QKeySequence("Shift+R"))
        self.rotate_counterclockwise_action.setShortcutContext(
            Qt.ShortcutContext.ApplicationShortcut
        )
        main_win.addAction(self.rotate_counterclockwise_action)

        self.rotate_180_action = QAction("Rotate 180°", main_win)
        self.rotate_180_action.setShortcut(QKeySequence("Alt+R"))
        self.rotate_180_action.setShortcutContext(
            Qt.ShortcutContext.ApplicationShortcut
        )
        main_win.addAction(self.rotate_180_action)

        # Focus Actions
        self.image_focus_actions = {}
        for i in range(1, 10):
            action = QAction(main_win)
            action.setShortcut(QKeySequence(str(i)))
            action.setData(i - 1)
            action.setShortcutContext(Qt.ShortcutContext.ApplicationShortcut)
            main_win.addAction(action)
            self.image_focus_actions[i] = action

        # Zoom actions
        self.zoom_in_action = QAction("Zoom In", main_win)
        self.zoom_in_action.setShortcut(QKeySequence.StandardKey.ZoomIn)
        main_win.addAction(self.zoom_in_action)

        self.zoom_out_action = QAction("Zoom Out", main_win)
        self.zoom_out_action.setShortcut(QKeySequence.StandardKey.ZoomOut)
        main_win.addAction(self.zoom_out_action)

        self.fit_to_view_action = QAction("Fit to View", main_win)
        self.fit_to_view_action.setShortcut(QKeySequence("0"))
        main_win.addAction(self.fit_to_view_action)

        self.actual_size_action = QAction("Actual Size", main_win)
        self.actual_size_action.setShortcut(QKeySequence("A"))
        main_win.addAction(self.actual_size_action)

        # View mode actions
        self.single_view_action = QAction("Single View", main_win)
        main_win.addAction(self.single_view_action)

        self.side_by_side_view_action = QAction("Side by Side View", main_win)
        self.side_by_side_view_action.setShortcut(QKeySequence("F2"))
        main_win.addAction(self.side_by_side_view_action)

        self.sync_pan_zoom_action = QAction("Synchronize Pan & Zoom", main_win)
        self.sync_pan_zoom_action.setShortcut(QKeySequence("F3"))
        main_win.addAction(self.sync_pan_zoom_action)

        # Deletion marking actions
        self.mark_for_delete_action = QAction("Mark for Deletion", main_win)
        self.mark_for_delete_action.setShortcut(QKeySequence("D"))
        main_win.addAction(self.mark_for_delete_action)

        self.unmark_for_delete_action = QAction("Unmark for Deletion", main_win)
        main_win.addAction(self.unmark_for_delete_action)

        self.commit_deletions_action = QAction("Commit All Marked Deletions", main_win)
        self.commit_deletions_action.setShortcut(QKeySequence("Shift+D"))
        main_win.addAction(self.commit_deletions_action)

        self.clear_marked_deletions_action = QAction(
            "Clear All Deletion Marks", main_win
        )
        self.clear_marked_deletions_action.setShortcut(QKeySequence("Alt+D"))
        main_win.addAction(self.clear_marked_deletions_action)

        # About action
        self.about_action = QAction("&About", main_win)
        self.about_action.setShortcut(QKeySequence("F12"))
        main_win.addAction(self.about_action)

        logger.debug("Actions created.")

    def _create_file_menu(self, menu_bar):
        main_win = self.main_window
        file_menu = menu_bar.addMenu("&File")

        self.open_folder_action = QAction("&Open Folder...", main_win)
        self.open_folder_action.setShortcut(QKeySequence.StandardKey.Open)
        file_menu.addAction(self.open_folder_action)

        self.open_recent_menu = QMenu("Open &Recent", main_win)
        file_menu.addMenu(self.open_recent_menu)
        self.update_recent_folders_menu()

        file_menu.addSeparator()
        self.exit_action = QAction("&Exit", main_win)
        self.exit_action.setShortcut(QKeySequence.StandardKey.Quit)
        file_menu.addAction(self.exit_action)

    def _create_view_menu(self, menu_bar):
        main_win = self.main_window
        view_menu = menu_bar.addMenu("&View")

        self.toggle_folder_view_action = QAction("Show Images in Folders", main_win)
        self.toggle_folder_view_action.setCheckable(True)
        self.toggle_folder_view_action.setChecked(main_win.show_folders_mode)
        self.toggle_folder_view_action.setShortcut(QKeySequence("F"))
        view_menu.addAction(self.toggle_folder_view_action)

        self.group_by_similarity_action = QAction("Group by Similarity", main_win)
        self.group_by_similarity_action.setCheckable(True)
        self.group_by_similarity_action.setChecked(False)
        self.group_by_similarity_action.setEnabled(False)
        self.group_by_similarity_action.setShortcut(QKeySequence("S"))
        view_menu.addAction(self.group_by_similarity_action)

        view_menu.addSeparator()

        self.toggle_thumbnails_action = QAction("Show Thumbnails", main_win)
        self.toggle_thumbnails_action.setCheckable(True)
        self.toggle_thumbnails_action.setChecked(True)
        self.toggle_thumbnails_action.setShortcut(QKeySequence("T"))
        view_menu.addAction(self.toggle_thumbnails_action)

        view_menu.addSeparator()

        self.analyze_similarity_action = QAction("Analyze Similarity", main_win)
        self.analyze_similarity_action.setToolTip(
            "Generate image embeddings and find similar groups (can be slow)"
        )
        self.analyze_similarity_action.setEnabled(False)
        self.analyze_similarity_action.setShortcut(QKeySequence("Ctrl+S"))
        view_menu.addAction(self.analyze_similarity_action)

        self.detect_blur_action = QAction("Detect Blurriness", main_win)
        self.detect_blur_action.setToolTip(
            "Analyze images for blurriness (can be slow for many images)"
        )
        self.detect_blur_action.setEnabled(False)
        self.detect_blur_action.setShortcut(QKeySequence("Ctrl+B"))
        view_menu.addAction(self.detect_blur_action)

        self.auto_rotate_action = QAction("Auto Rotate Images", main_win)
        self.auto_rotate_action.setToolTip(
            "Automatically detect and suggest rotations for poorly oriented images"
        )
        self.auto_rotate_action.setEnabled(False)
        self.auto_rotate_action.setShortcut(QKeySequence("Ctrl+R"))
        view_menu.addAction(self.auto_rotate_action)

        view_menu.addSeparator()

        self.toggle_metadata_sidebar_action = QAction(
            "Show Image Details Sidebar", main_win
        )
        self.toggle_metadata_sidebar_action.setCheckable(True)
        self.toggle_metadata_sidebar_action.setChecked(False)
        self.toggle_metadata_sidebar_action.setShortcut("I")
        view_menu.addAction(self.toggle_metadata_sidebar_action)

    def _create_image_menu(self, menu_bar):
        image_menu = menu_bar.addMenu("&Image")

        image_menu.addAction(self.rotate_clockwise_action)
        image_menu.addAction(self.rotate_counterclockwise_action)
        image_menu.addAction(self.rotate_180_action)
        image_menu.addSeparator()

        image_menu.addAction(self.mark_for_delete_action)
        image_menu.addAction(self.commit_deletions_action)
        image_menu.addAction(self.clear_marked_deletions_action)

    def _create_filter_menu(self, menu_bar):
        main_win = self.main_window
        filter_menu = menu_bar.addMenu("&Filter")

        # Rating filter
        rating_widget = QWidget()
        rating_layout = QHBoxLayout(rating_widget)
        rating_layout.setContentsMargins(10, 5, 10, 5)
        rating_layout.addWidget(QLabel("Rating:"))
        rating_layout.addWidget(main_win.filter_combo)
        rating_action = QWidgetAction(main_win)
        rating_action.setDefaultWidget(rating_widget)
        filter_menu.addAction(rating_action)

        # Cluster filter
        cluster_widget = QWidget()
        cluster_layout = QHBoxLayout(cluster_widget)
        cluster_layout.setContentsMargins(10, 5, 10, 5)
        cluster_layout.addWidget(QLabel("Cluster:"))
        cluster_layout.addWidget(main_win.cluster_filter_combo)
        cluster_action = QWidgetAction(main_win)
        cluster_action.setDefaultWidget(cluster_widget)
        filter_menu.addAction(cluster_action)

        # Cluster sort
        sort_widget = QWidget()
        sort_layout = QHBoxLayout(sort_widget)
        sort_layout.setContentsMargins(10, 5, 10, 5)
        sort_layout.addWidget(QLabel("Sort Clusters By:"))
        sort_layout.addWidget(main_win.cluster_sort_combo)
        self.cluster_sort_action = QWidgetAction(main_win)
        self.cluster_sort_action.setDefaultWidget(sort_widget)
        self.cluster_sort_action.setVisible(False)
        filter_menu.addAction(self.cluster_sort_action)

    def _create_settings_menu(self, menu_bar):
        main_win = self.main_window
        settings_menu = menu_bar.addMenu("&Settings")

        self.manage_cache_action = QAction("Manage Cache", main_win)
        self.manage_cache_action.setShortcut(QKeySequence("F9"))
        settings_menu.addAction(self.manage_cache_action)

        settings_menu.addSeparator()

        self.toggle_auto_edits_action = QAction("Enable Auto RAW Edits", main_win)
        self.toggle_auto_edits_action.setCheckable(True)
        self.toggle_auto_edits_action.setChecked(main_win.apply_auto_edits_enabled)
        self.toggle_auto_edits_action.setToolTip(
            "Apply automatic brightness, contrast, and color adjustments to RAW previews and thumbnails."
        )
        self.toggle_auto_edits_action.setShortcut(QKeySequence("F10"))
        settings_menu.addAction(self.toggle_auto_edits_action)

        self.toggle_mark_for_deletion_action = QAction(
            "Mark for Deletion (vs. Direct Delete)", main_win
        )
        self.toggle_mark_for_deletion_action.setCheckable(True)
        self.toggle_mark_for_deletion_action.setChecked(
            main_win.mark_for_deletion_mode_enabled
        )
        self.toggle_mark_for_deletion_action.setToolTip(
            "If checked, the Delete key will mark files for later deletion. If unchecked, it will move them to trash immediately."
        )
        self.toggle_mark_for_deletion_action.setShortcut(QKeySequence("F11"))
        settings_menu.addAction(self.toggle_mark_for_deletion_action)

    def _create_help_menu(self, menu_bar):
        help_menu = menu_bar.addMenu("&Help")
        help_menu.addAction(self.about_action)

    def connect_signals(self):
        """Connect all actions to their corresponding slots in MainWindow."""
        main_win = self.main_window

        # File Menu
        self.open_folder_action.triggered.connect(main_win._open_folder_dialog)
        self.exit_action.triggered.connect(main_win.close)

        # View Menu
        self.toggle_folder_view_action.toggled.connect(
            main_win._toggle_folder_visibility
        )
        self.group_by_similarity_action.toggled.connect(
            main_win._toggle_group_by_similarity
        )
        self.toggle_thumbnails_action.toggled.connect(main_win._toggle_thumbnail_view)
        self.analyze_similarity_action.triggered.connect(
            main_win.app_controller.start_similarity_analysis
        )
        self.detect_blur_action.triggered.connect(
            main_win.app_controller.start_blur_detection_analysis
        )
        self.auto_rotate_action.triggered.connect(
            main_win.app_controller.start_auto_rotation_analysis
        )
        self.toggle_metadata_sidebar_action.toggled.connect(
            main_win._toggle_metadata_sidebar
        )

        # Settings Menu
        self.manage_cache_action.triggered.connect(
            self.dialog_manager.show_cache_management_dialog
        )
        self.toggle_auto_edits_action.toggled.connect(
            main_win._handle_toggle_auto_edits
        )
        self.toggle_mark_for_deletion_action.toggled.connect(
            main_win._handle_toggle_mark_for_deletion_mode
        )

        # Image Menu
        self.rotate_clockwise_action.triggered.connect(
            main_win._rotate_current_image_clockwise
        )
        self.rotate_counterclockwise_action.triggered.connect(
            main_win._rotate_current_image_counterclockwise
        )
        self.rotate_180_action.triggered.connect(main_win._rotate_current_image_180)
        self.mark_for_delete_action.triggered.connect(
            main_win._mark_selection_for_deletion
        )
        self.unmark_for_delete_action.triggered.connect(
            main_win._mark_selection_for_deletion
        )
        self.commit_deletions_action.triggered.connect(
            main_win._commit_marked_deletions
        )
        self.clear_marked_deletions_action.triggered.connect(
            main_win._clear_all_deletion_marks
        )

        # Zoom actions
        self.zoom_in_action.triggered.connect(
            main_win.advanced_image_viewer._zoom_in_all
        )
        self.zoom_out_action.triggered.connect(
            main_win.advanced_image_viewer._zoom_out_all
        )
        self.fit_to_view_action.triggered.connect(
            main_win.advanced_image_viewer._fit_all
        )
        self.actual_size_action.triggered.connect(
            main_win.advanced_image_viewer._actual_size_all
        )

        # View mode actions
        self.single_view_action.triggered.connect(
            lambda: main_win.advanced_image_viewer._set_view_mode("single")
        )
        self.side_by_side_view_action.triggered.connect(
            lambda: main_win.advanced_image_viewer._set_view_mode("side_by_side")
        )
        self.sync_pan_zoom_action.triggered.connect(
            main_win.advanced_image_viewer.sync_button.toggle
        )

        # Other Actions
        self.find_action.triggered.connect(main_win._focus_search_input)
        for action in self.image_focus_actions.values():
            action.triggered.connect(main_win._handle_image_focus_shortcut)
        self.about_action.triggered.connect(self.dialog_manager.show_about_dialog)

    def update_recent_folders_menu(self):
        """Update the 'Open Recent' menu with the latest list of folders."""
        if not hasattr(self, "open_recent_menu"):
            return

        self.open_recent_menu.clear()
        recent_folders = get_recent_folders()

        if not recent_folders:
            action = QAction("No Recent Folders", self.main_window)
            action.setEnabled(False)
            self.open_recent_menu.addAction(action)
        else:
            for folder in recent_folders:
                action = QAction(folder, self.main_window)
                action.triggered.connect(
                    lambda checked=False,
                    f=folder: self.main_window.app_controller.load_folder(f)
                )
                self.open_recent_menu.addAction(action)

    def show_image_context_menu(self, position: QPoint):
        main_win = self.main_window
        active_view = main_win.sender()
        if not hasattr(active_view, "indexAt"):
            return

        proxy_index = active_view.indexAt(position)
        if not main_win._is_valid_image_item(proxy_index):
            return

        source_index = main_win.proxy_model.mapToSource(proxy_index)
        item = main_win.file_system_model.itemFromIndex(source_index)
        if not item:
            return

        item_data = item.data(Qt.ItemDataRole.UserRole)
        file_path = item_data.get("path")
        if not file_path or not os.path.exists(file_path):
            return

        menu = QMenu(main_win)

        # Rotation
        if MetadataProcessor.is_rotation_supported(file_path):
            selected_paths = main_win._get_selected_file_paths_from_view()
            num_selected = len(selected_paths) if len(selected_paths) > 1 else 1
            label_suffix = f" {num_selected} Images" if num_selected > 1 else ""

            rotate_cw = QAction(f"Rotate{label_suffix} 90° Clockwise", main_win)
            rotate_ccw = QAction(f"Rotate{label_suffix} 90° Counterclockwise", main_win)
            rotate_180 = QAction(f"Rotate{label_suffix} 180°", main_win)

            rotate_cw.triggered.connect(
                lambda: main_win._rotate_selected_images("clockwise")
            )
            rotate_ccw.triggered.connect(
                lambda: main_win._rotate_selected_images("counterclockwise")
            )
            rotate_180.triggered.connect(
                lambda: main_win._rotate_selected_images("180")
            )

            menu.addAction(rotate_cw)
            menu.addAction(rotate_ccw)
            menu.addAction(rotate_180)
            menu.addSeparator()

        # Deletion Marking
        is_marked = main_win._is_marked_for_deletion(file_path)
        if is_marked:
            menu.addAction(self.unmark_for_delete_action)
        else:
            menu.addAction(self.mark_for_delete_action)
        menu.addSeparator()

        # Explorer
        show_in_explorer = QAction(
            QIcon.fromTheme(
                "folder-open",
                main_win.style().standardIcon(QStyle.StandardPixmap.SP_DirOpenIcon),
            ),
            "Show in Explorer",
            main_win,
        )
        show_in_explorer.triggered.connect(
            lambda: self._open_image_in_explorer(file_path)
        )
        menu.addAction(show_in_explorer)

        menu.exec(active_view.viewport().mapToGlobal(position))

    def _open_image_in_explorer(self, file_path: str):
        try:
            normalized_path = os.path.normpath(file_path)
            if os.name == "nt":
                subprocess.run(["explorer", "/select,", normalized_path], check=False)
            elif os.name == "posix":
                if os.uname().sysname == "Darwin":
                    subprocess.run(["open", "-R", normalized_path], check=False)
                else:
                    subprocess.run(
                        ["xdg-open", os.path.dirname(normalized_path)], check=False
                    )
        except Exception as e:
            logger.error(
                f"Failed to open '{file_path}' in file explorer: {e}", exc_info=True
            )

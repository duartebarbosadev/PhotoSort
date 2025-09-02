from __future__ import annotations
from typing import TYPE_CHECKING
from PyQt6.QtCore import QSize
from PyQt6.QtWidgets import QListView
from core.app_settings import (
    FIXED_ICON_SIZE,
    FIXED_GRID_WIDTH,
    FIXED_GRID_HEIGHT,
    GRID_SPACING,
)

if TYPE_CHECKING:
    from ui.main_window import MainWindow
    from ui.left_panel import LeftPanel
    from ui.app_state import AppState


class ViewManager:
    """Manages the different view modes of the application."""

    def __init__(
        self, main_window: MainWindow, app_state: AppState, left_panel: LeftPanel
    ):
        self.main_window = main_window
        self.app_state = app_state
        self.left_panel = left_panel
        self.current_view_mode = "list"  # Default view mode
        self.thumbnail_delegate = None

    def connect_signals(self):
        """Connects signals for the view manager."""
        self.left_panel.view_list_icon.clicked.connect(self.set_view_mode_list)
        self.left_panel.view_icons_icon.clicked.connect(self.set_view_mode_icons)
        self.left_panel.view_grid_icon.clicked.connect(self.set_view_mode_grid)

    def set_view_mode_list(self):
        self.current_view_mode = "list"
        self.left_panel.tree_display_view.setVisible(True)
        self.left_panel.grid_display_view.setVisible(False)
        self.left_panel.tree_display_view.setIconSize(QSize(16, 16))
        self.left_panel.tree_display_view.setIndentation(10)
        self.left_panel.tree_display_view.setRootIsDecorated(
            self.main_window.show_folders_mode
            or self.main_window.group_by_similarity_mode
        )
        self.left_panel.tree_display_view.setItemsExpandable(
            self.main_window.show_folders_mode
            or self.main_window.group_by_similarity_mode
        )
        if self.left_panel.tree_display_view.itemDelegate() is self.thumbnail_delegate:
            self.left_panel.tree_display_view.setItemDelegate(None)
        self.update_view_button_states()
        self.main_window._rebuild_model_view()
        self.left_panel.tree_display_view.setFocus()

    def set_view_mode_icons(self):
        self.current_view_mode = "icons"
        self.left_panel.tree_display_view.setVisible(True)
        self.left_panel.grid_display_view.setVisible(False)
        self.left_panel.tree_display_view.setIconSize(QSize(64, 64))
        self.left_panel.tree_display_view.setIndentation(20)
        self.left_panel.tree_display_view.setRootIsDecorated(
            self.main_window.show_folders_mode
            or self.main_window.group_by_similarity_mode
        )
        self.left_panel.tree_display_view.setItemsExpandable(
            self.main_window.show_folders_mode
            or self.main_window.group_by_similarity_mode
        )
        if self.left_panel.tree_display_view.itemDelegate() is self.thumbnail_delegate:
            self.left_panel.tree_display_view.setItemDelegate(None)
        self.update_view_button_states()
        self.main_window._rebuild_model_view()
        self.left_panel.tree_display_view.setFocus()

    def set_view_mode_grid(self):
        self.current_view_mode = "grid"
        if (
            self.main_window.group_by_similarity_mode
        ):  # Grid view not supported when grouping by similarity
            self.left_panel.tree_display_view.setVisible(True)
            self.left_panel.grid_display_view.setVisible(False)
            # Use a suitable icon size for tree when grid would have been active
            self.left_panel.tree_display_view.setIconSize(QSize(96, 96))
            self.left_panel.tree_display_view.setIndentation(20)
            self.left_panel.tree_display_view.setRootIsDecorated(True)
            self.left_panel.tree_display_view.setItemsExpandable(True)
            if (
                self.left_panel.tree_display_view.itemDelegate()
                is self.thumbnail_delegate
            ):
                self.left_panel.tree_display_view.setItemDelegate(None)
            self.update_view_button_states()
            self.main_window._rebuild_model_view()
            self.left_panel.tree_display_view.setFocus()
        else:
            self.left_panel.tree_display_view.setVisible(False)
            self.left_panel.grid_display_view.setVisible(True)
            self.left_panel.grid_display_view.setViewMode(QListView.ViewMode.IconMode)
            self.left_panel.grid_display_view.setFlow(QListView.Flow.LeftToRight)
            self.left_panel.grid_display_view.setWrapping(True)
            self.left_panel.grid_display_view.setResizeMode(QListView.ResizeMode.Adjust)
            self.update_view_button_states()
            self.main_window._rebuild_model_view()  # Populate model first
            self.update_grid_view_layout()  # Then adjust layout
            self.left_panel.grid_display_view.setFocus()

    def update_view_button_states(self):
        """Update the visual state of view mode icon buttons"""
        # Reset all icon buttons
        self.left_panel.view_list_icon.setChecked(False)
        self.left_panel.view_icons_icon.setChecked(False)
        self.left_panel.view_grid_icon.setChecked(False)

        # Set the active icon button
        if self.current_view_mode == "list":
            self.left_panel.view_list_icon.setChecked(True)
        elif self.current_view_mode == "icons":
            self.left_panel.view_icons_icon.setChecked(True)
        elif self.current_view_mode == "grid":
            self.left_panel.view_grid_icon.setChecked(True)

    def update_grid_view_layout(self):
        if not self.left_panel.grid_display_view.isVisible():
            return

        # Fixed grid layout to prevent filename length from affecting layout
        FIXED_GRID_SIZE = QSize(
            FIXED_GRID_WIDTH, FIXED_GRID_HEIGHT
        )  # Fixed grid cell size (width, height)

        # Set fixed icon size and grid properties
        self.left_panel.grid_display_view.setIconSize(
            QSize(FIXED_ICON_SIZE, FIXED_ICON_SIZE)
        )
        self.left_panel.grid_display_view.setGridSize(FIXED_GRID_SIZE)
        self.left_panel.grid_display_view.setSpacing(GRID_SPACING)

        # Ensure uniform grid layout
        self.left_panel.grid_display_view.setUniformItemSizes(True)
        self.left_panel.grid_display_view.setWordWrap(True)

        self.left_panel.grid_display_view.updateGeometries()
        self.left_panel.grid_display_view.viewport().update()

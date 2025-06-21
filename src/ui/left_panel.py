from PyQt6.QtWidgets import (
    QWidget, QVBoxLayout, QHBoxLayout, QLabel, QLineEdit, QPushButton, QStyle,
    QTreeView, QListView, QAbstractItemView
)
from PyQt6.QtCore import Qt, QSize

from src.ui.ui_components import DroppableTreeView, FocusHighlightDelegate
from src.core.app_settings import get_auto_edit_photos, get_mark_for_deletion_mode

class LeftPanel(QWidget):
    """
    A widget that encapsulates the entire left panel of the main window,
    including the search bar, view mode buttons, and the file list/grid views.
    """
    def __init__(self, proxy_model, app_state, main_window, parent=None):
        super().__init__(parent)
        self.setObjectName("left_pane_widget")

        self.proxy_model = proxy_model
        self.app_state = app_state
        self.main_window = main_window
        self.current_view_mode = "list"
        self.thumbnail_delegate = None # This will be set from main_window

        self._create_widgets()
        self._create_layout()
        self._create_delegates()

        self.connect_signals()

    def connect_signals(self):
        """Connects signals for the view manager."""
        self.view_list_icon.clicked.connect(self.set_view_mode_list)
        self.view_icons_icon.clicked.connect(self.set_view_mode_icons)
        self.view_grid_icon.clicked.connect(self.set_view_mode_grid)
        self.view_date_icon.clicked.connect(self.set_view_mode_date)
 
    def _create_widgets(self):
        """Creates the widgets for the left panel."""
        # Search container
        self.search_container = QWidget()
        self.search_container.setObjectName("search_container")
        search_layout = QHBoxLayout(self.search_container)
        search_layout.setContentsMargins(0, 0, 0, 0)
        search_layout.setSpacing(5)

        search_layout.addWidget(QLabel("Search:"))
        self.search_input = QLineEdit()
        self.search_input.setPlaceholderText("Filename...")
        search_layout.addWidget(self.search_input)

        # View type icons
        view_icons_container = QWidget()
        view_icons_layout = QHBoxLayout(view_icons_container)
        view_icons_layout.setContentsMargins(0, 0, 0, 0)
        view_icons_layout.setSpacing(2)

        self.view_list_icon = QPushButton()
        self.view_list_icon.setIcon(self.style().standardIcon(QStyle.StandardPixmap.SP_FileDialogDetailedView))
        self.view_list_icon.setToolTip("List View")
        self.view_list_icon.setCheckable(True)
        self.view_list_icon.setMaximumSize(24, 24)

        self.view_icons_icon = QPushButton()
        self.view_icons_icon.setIcon(self.style().standardIcon(QStyle.StandardPixmap.SP_FileDialogListView))
        self.view_icons_icon.setToolTip("Icons View")
        self.view_icons_icon.setCheckable(True)
        self.view_icons_icon.setMaximumSize(24, 24)

        self.view_grid_icon = QPushButton()
        self.view_grid_icon.setIcon(self.style().standardIcon(QStyle.StandardPixmap.SP_FileDialogListView))
        self.view_grid_icon.setToolTip("Grid View")
        self.view_grid_icon.setCheckable(True)
        self.view_grid_icon.setMaximumSize(24, 24)

        self.view_date_icon = QPushButton()
        self.view_date_icon.setIcon(self.style().standardIcon(QStyle.StandardPixmap.SP_FileDialogDetailedView))
        self.view_date_icon.setToolTip("Date View")
        self.view_date_icon.setCheckable(True)
        self.view_date_icon.setMaximumSize(24, 24)

        view_icons_layout.addWidget(self.view_list_icon)
        view_icons_layout.addWidget(self.view_icons_icon)
        view_icons_layout.addWidget(self.view_grid_icon)
        view_icons_layout.addWidget(self.view_date_icon)
        view_icons_layout.addStretch()
        search_layout.addWidget(view_icons_container)

        # Tree and Grid Views
        self.tree_display_view = DroppableTreeView(self.proxy_model, self.main_window)
        self.tree_display_view.setHeaderHidden(True)
        self.tree_display_view.setIndentation(15)
        self.tree_display_view.setSelectionMode(QAbstractItemView.SelectionMode.ExtendedSelection)
        self.tree_display_view.setEditTriggers(QAbstractItemView.EditTrigger.NoEditTriggers)
        self.tree_display_view.setContextMenuPolicy(Qt.ContextMenuPolicy.CustomContextMenu)
        self.tree_display_view.setMinimumWidth(300)
        self.tree_display_view.setDragEnabled(False)
        self.tree_display_view.setAcceptDrops(False)
        self.tree_display_view.setDropIndicatorShown(False)

        self.grid_display_view = QListView()
        self.grid_display_view.setModel(self.proxy_model)
        self.grid_display_view.setViewMode(QListView.ViewMode.IconMode)
        self.grid_display_view.setFlow(QListView.Flow.LeftToRight)
        self.grid_display_view.setWrapping(True)
        self.grid_display_view.setResizeMode(QListView.ResizeMode.Fixed)
        self.grid_display_view.setSelectionMode(QAbstractItemView.SelectionMode.ExtendedSelection)
        self.grid_display_view.setEditTriggers(QAbstractItemView.EditTrigger.NoEditTriggers)
        self.grid_display_view.setContextMenuPolicy(Qt.ContextMenuPolicy.CustomContextMenu)
        self.grid_display_view.setMinimumWidth(300)
        self.grid_display_view.setVisible(False)
        self.grid_display_view.setDragEnabled(True)
        self.grid_display_view.setGridSize(QSize(128, 148))
        self.grid_display_view.setUniformItemSizes(True)
        self.grid_display_view.setWordWrap(True)

    def _create_layout(self):
        """Creates the layout for the left panel."""
        layout = QVBoxLayout(self)
        layout.setContentsMargins(5, 5, 5, 5)
        layout.setSpacing(5)
        layout.addWidget(self.search_container)
        layout.addWidget(self.tree_display_view)
        layout.addWidget(self.grid_display_view)

    def _create_delegates(self):
        """Creates and sets the item delegates for the views."""
        self.focus_delegate = FocusHighlightDelegate(self.app_state, self.main_window)
        self.tree_display_view.setItemDelegate(self.focus_delegate)
        self.grid_display_view.setItemDelegate(self.focus_delegate)

    def get_active_view(self):
        """Returns the currently active file view."""
        if self.grid_display_view.isVisible():
            return self.grid_display_view
        return self.tree_display_view

    def set_view_mode_list(self):
        self.current_view_mode = "list"
        self.tree_display_view.setVisible(True)
        self.grid_display_view.setVisible(False)
        self.tree_display_view.setIconSize(QSize(16, 16))
        self.tree_display_view.setIndentation(10)
        self.tree_display_view.setRootIsDecorated(self.main_window.show_folders_mode or self.main_window.group_by_similarity_mode)
        self.tree_display_view.setItemsExpandable(self.main_window.show_folders_mode or self.main_window.group_by_similarity_mode)
        if self.tree_display_view.itemDelegate() is self.thumbnail_delegate:
            self.tree_display_view.setItemDelegate(None)
        self.update_view_button_states()
        self.main_window._rebuild_model_view()
        self.tree_display_view.setFocus()

    def set_view_mode_icons(self):
        self.current_view_mode = "icons"
        self.tree_display_view.setVisible(True)
        self.grid_display_view.setVisible(False)
        self.tree_display_view.setIconSize(QSize(64, 64))
        self.tree_display_view.setIndentation(20)
        self.tree_display_view.setRootIsDecorated(self.main_window.show_folders_mode or self.main_window.group_by_similarity_mode)
        self.tree_display_view.setItemsExpandable(self.main_window.show_folders_mode or self.main_window.group_by_similarity_mode)
        if self.tree_display_view.itemDelegate() is self.thumbnail_delegate:
             self.tree_display_view.setItemDelegate(None)
        self.update_view_button_states()
        self.main_window._rebuild_model_view()
        self.tree_display_view.setFocus()
        
    def set_view_mode_grid(self):
        self.current_view_mode = "grid"
        if self.main_window.group_by_similarity_mode: # Grid view not supported when grouping by similarity
            self.tree_display_view.setVisible(True)
            self.grid_display_view.setVisible(False)
            # Use a suitable icon size for tree when grid would have been active
            self.tree_display_view.setIconSize(QSize(96, 96))
            self.tree_display_view.setIndentation(20)
            self.tree_display_view.setRootIsDecorated(True)
            self.tree_display_view.setItemsExpandable(True)
            if self.tree_display_view.itemDelegate() is self.thumbnail_delegate:
                 self.tree_display_view.setItemDelegate(None)
            self.update_view_button_states()
            self.main_window._rebuild_model_view()
            self.tree_display_view.setFocus()
        else:
            self.tree_display_view.setVisible(False)
            self.grid_display_view.setVisible(True)
            self.grid_display_view.setViewMode(QListView.ViewMode.IconMode)
            self.grid_display_view.setFlow(QListView.Flow.LeftToRight)
            self.grid_display_view.setWrapping(True)
            self.grid_display_view.setResizeMode(QListView.ResizeMode.Adjust)
            self.update_view_button_states()
            self.main_window._rebuild_model_view() # Populate model first
            self.update_grid_view_layout() # Then adjust layout
            self.grid_display_view.setFocus()

    def set_view_mode_date(self):
        self.current_view_mode = "date"
        self.tree_display_view.setVisible(True)
        self.grid_display_view.setVisible(False)
        self.tree_display_view.setIconSize(QSize(16, 16))
        self.tree_display_view.setIndentation(20)
        self.tree_display_view.setRootIsDecorated(True)
        self.tree_display_view.setItemsExpandable(True)
        if self.tree_display_view.itemDelegate() is self.thumbnail_delegate:
            self.tree_display_view.setItemDelegate(None)
        self.update_view_button_states()
        self.main_window._rebuild_model_view()
        self.tree_display_view.setFocus()

    def update_view_button_states(self):
        """Update the visual state of view mode icon buttons"""
        # Reset all icon buttons
        self.view_list_icon.setChecked(False)
        self.view_icons_icon.setChecked(False)
        self.view_grid_icon.setChecked(False)
        self.view_date_icon.setChecked(False)
        
        # Set the active icon button
        if self.current_view_mode == "list":
            self.view_list_icon.setChecked(True)
        elif self.current_view_mode == "icons":
            self.view_icons_icon.setChecked(True)
        elif self.current_view_mode == "grid":
            self.view_grid_icon.setChecked(True)
        elif self.current_view_mode == "date":
            self.view_date_icon.setChecked(True)

    def update_grid_view_layout(self):
        if not self.grid_display_view.isVisible():
            return
        
        # Fixed grid layout to prevent filename length from affecting layout
        FIXED_ICON_SIZE = 96  # Fixed icon size
        FIXED_GRID_SIZE = QSize(128, 148)  # Fixed grid cell size (width, height)
        GRID_SPACING = 4
        
        # Set fixed icon size and grid properties
        self.grid_display_view.setIconSize(QSize(FIXED_ICON_SIZE, FIXED_ICON_SIZE))
        self.grid_display_view.setGridSize(FIXED_GRID_SIZE)
        self.grid_display_view.setSpacing(GRID_SPACING)
        
        # Ensure uniform grid layout
        self.grid_display_view.setUniformItemSizes(True)
        self.grid_display_view.setWordWrap(True)
        
        self.grid_display_view.updateGeometries()
        self.grid_display_view.viewport().update()
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

        self._create_widgets()
        self._create_layout()
        self._create_delegates()

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
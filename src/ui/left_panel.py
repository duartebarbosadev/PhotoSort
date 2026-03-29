from PyQt6.QtWidgets import (
    QWidget,
    QVBoxLayout,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QPushButton,
    QFrame,
    QStyle,
    QListView,
    QAbstractItemView,
)
from PyQt6.QtCore import Qt, QSize
from core.app_settings import (
    FIXED_ICON_SIZE,
    FIXED_GRID_WIDTH,
    FIXED_GRID_HEIGHT,
    GRID_SPACING,
)

from ui.ui_components import (
    DroppableTreeView,
    FocusHighlightDelegate,
    NoCtrlListView,
)


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
        self.thumbnail_delegate = None

        self._create_widgets()
        self._create_layout()
        self._create_delegates()

        self.connect_signals()

    def connect_signals(self):
        """Connects signals for the view manager."""
        self.view_list_icon.clicked.connect(self.set_view_mode_list)
        self.view_icons_icon.clicked.connect(self.set_view_mode_icons)
        self.view_grid_icon.clicked.connect(self.set_view_mode_grid)
        self.view_rotation_icon.clicked.connect(self.set_view_mode_rotation)
        self.rotation_suggestions_view.selectionModel().selectionChanged.connect(
            self.main_window._handle_file_selection_changed
        )

    def _create_widgets(self):
        """Creates the widgets for the left panel."""
        self.header_card = QFrame()
        self.header_card.setObjectName("sidebarHeaderSection")

        self.sidebar_eyebrow = QLabel("PhotoSort")
        self.sidebar_eyebrow.setObjectName("sidebarEyebrow")

        self.sidebar_title = QLabel("No folder selected")
        self.sidebar_title.setObjectName("sidebarTitle")

        self.sidebar_subtitle = QLabel("Open a folder to start sorting your library.")
        self.sidebar_subtitle.setObjectName("sidebarSubtitle")
        self.sidebar_subtitle.setWordWrap(True)

        self.items_badge = QLabel("0 items")
        self.items_badge.setObjectName("sidebarBadge")

        self.search_container = QFrame()
        self.search_container.setObjectName("search_container")
        search_layout = QHBoxLayout(self.search_container)
        search_layout.setContentsMargins(0, 0, 0, 0)
        search_layout.setSpacing(8)

        self.search_input = QLineEdit()
        self.search_input.setPlaceholderText("Search by filename or extension")
        self.search_input.setClearButtonEnabled(True)
        search_layout.addWidget(self.search_input, 1)

        self.view_icons_container = QFrame()
        self.view_icons_container.setObjectName("sidebarButtonRail")
        view_icons_layout = QHBoxLayout(self.view_icons_container)
        view_icons_layout.setContentsMargins(4, 4, 4, 4)
        view_icons_layout.setSpacing(4)

        self.view_list_icon = QPushButton()
        self.view_list_icon.setIcon(
            self.style().standardIcon(QStyle.StandardPixmap.SP_FileDialogDetailedView)
        )
        self.view_list_icon.setToolTip("List View")
        self.view_list_icon.setCheckable(True)
        self.view_list_icon.setObjectName("sidebarModeButton")
        self.view_list_icon.setMinimumSize(32, 28)

        self.view_icons_icon = QPushButton()
        self.view_icons_icon.setIcon(
            self.style().standardIcon(QStyle.StandardPixmap.SP_FileDialogListView)
        )
        self.view_icons_icon.setToolTip("Icons View")
        self.view_icons_icon.setCheckable(True)
        self.view_icons_icon.setObjectName("sidebarModeButton")
        self.view_icons_icon.setMinimumSize(32, 28)

        self.view_grid_icon = QPushButton()
        self.view_grid_icon.setIcon(
            self.style().standardIcon(QStyle.StandardPixmap.SP_FileDialogListView)
        )
        self.view_grid_icon.setToolTip("Grid View")
        self.view_grid_icon.setCheckable(True)
        self.view_grid_icon.setObjectName("sidebarModeButton")
        self.view_grid_icon.setMinimumSize(32, 28)

        self.view_rotation_icon = QPushButton()
        self.view_rotation_icon.setIcon(
            self.style().standardIcon(QStyle.StandardPixmap.SP_BrowserReload)
        )
        self.view_rotation_icon.setToolTip("Rotation View")
        self.view_rotation_icon.setCheckable(True)
        self.view_rotation_icon.setObjectName("sidebarModeButton")
        self.view_rotation_icon.setMinimumSize(32, 28)
        self.view_rotation_icon.setVisible(False)  # Hidden by default

        view_icons_layout.addWidget(self.view_list_icon)
        view_icons_layout.addWidget(self.view_icons_icon)
        view_icons_layout.addWidget(self.view_grid_icon)
        view_icons_layout.addWidget(self.view_rotation_icon)
        search_layout.addWidget(self.view_icons_container)

        self.browser_title = QLabel("Browser")
        self.browser_title.setObjectName("sidebarSectionLabel")

        self.browser_mode_label = QLabel("List view")
        self.browser_mode_label.setObjectName("sidebarInlineHint")

        # Tree and Grid Views
        self.tree_display_view = DroppableTreeView(self.proxy_model, self.main_window)
        self.tree_display_view.setHeaderHidden(True)
        self.tree_display_view.setIndentation(15)
        self.tree_display_view.setSelectionMode(
            QAbstractItemView.SelectionMode.ExtendedSelection
        )
        self.tree_display_view.setEditTriggers(
            QAbstractItemView.EditTrigger.NoEditTriggers
        )
        self.tree_display_view.setContextMenuPolicy(
            Qt.ContextMenuPolicy.CustomContextMenu
        )
        self.tree_display_view.setMinimumWidth(300)
        # Drag-drop enabled for cluster management in similarity mode
        self.tree_display_view.setDragEnabled(True)
        self.tree_display_view.setAcceptDrops(True)
        self.tree_display_view.setDropIndicatorShown(True)

        self.grid_display_view = NoCtrlListView()
        self.grid_display_view.setModel(self.proxy_model)
        self.grid_display_view.setViewMode(QListView.ViewMode.IconMode)
        self.grid_display_view.setFlow(QListView.Flow.LeftToRight)
        self.grid_display_view.setWrapping(True)
        self.grid_display_view.setResizeMode(QListView.ResizeMode.Fixed)
        self.grid_display_view.setSelectionMode(
            QAbstractItemView.SelectionMode.ExtendedSelection
        )
        self.grid_display_view.setEditTriggers(
            QAbstractItemView.EditTrigger.NoEditTriggers
        )
        self.grid_display_view.setContextMenuPolicy(
            Qt.ContextMenuPolicy.CustomContextMenu
        )
        self.grid_display_view.setMinimumWidth(300)
        self.grid_display_view.setVisible(False)
        self.grid_display_view.setDragEnabled(True)
        self.grid_display_view.setGridSize(QSize(128, 148))
        self.grid_display_view.setUniformItemSizes(True)
        self.grid_display_view.setWordWrap(True)

        self.rotation_suggestions_view = DroppableTreeView(
            self.proxy_model, self.main_window
        )
        self.rotation_suggestions_view.setModel(self.proxy_model)
        self.rotation_suggestions_view.setHeaderHidden(True)
        self.rotation_suggestions_view.setIndentation(15)
        self.rotation_suggestions_view.setSelectionMode(
            QAbstractItemView.SelectionMode.ExtendedSelection
        )
        self.rotation_suggestions_view.setEditTriggers(
            QAbstractItemView.EditTrigger.NoEditTriggers
        )
        self.rotation_suggestions_view.setContextMenuPolicy(
            Qt.ContextMenuPolicy.CustomContextMenu
        )
        self.rotation_suggestions_view.setMinimumWidth(300)
        self.rotation_suggestions_view.setVisible(False)

    def _create_layout(self):
        """Creates the layout for the left panel."""
        layout = QVBoxLayout(self)
        layout.setContentsMargins(14, 14, 14, 14)
        layout.setSpacing(8)

        header_layout = QVBoxLayout(self.header_card)
        header_layout.setContentsMargins(10, 8, 10, 6)
        header_layout.setSpacing(6)

        meta_row = QHBoxLayout()
        meta_row.setContentsMargins(0, 0, 0, 0)
        meta_row.setSpacing(8)
        meta_row.addWidget(self.sidebar_eyebrow)
        meta_row.addStretch()
        header_layout.addLayout(meta_row)

        title_row = QHBoxLayout()
        title_row.setContentsMargins(0, 0, 0, 0)
        title_row.setSpacing(8)
        title_row.addWidget(self.sidebar_title)
        title_row.addStretch()
        title_row.addWidget(self.items_badge)
        header_layout.addLayout(title_row)

        header_layout.addWidget(self.sidebar_subtitle)
        header_layout.addWidget(self.search_container)

        browser_header = QHBoxLayout()
        browser_header.setContentsMargins(4, 0, 4, 0)
        browser_header.setSpacing(8)
        browser_header.addWidget(self.browser_title)
        browser_header.addStretch()
        browser_header.addWidget(self.browser_mode_label)

        layout.addWidget(self.header_card)
        layout.addLayout(browser_header)
        layout.addWidget(self.tree_display_view, 1)
        layout.addWidget(self.grid_display_view, 1)
        layout.addWidget(self.rotation_suggestions_view, 1)

    def _create_delegates(self):
        """Creates and sets the item delegates for the views."""
        self.focus_delegate = FocusHighlightDelegate(self.app_state, self.main_window)
        self.tree_display_view.setItemDelegate(self.focus_delegate)
        self.grid_display_view.setItemDelegate(self.focus_delegate)
        self.rotation_suggestions_view.setItemDelegate(self.focus_delegate)

    def get_active_view(self):
        """Returns the currently active file view based on the current_view_mode property."""
        if self.current_view_mode == "grid":
            return self.grid_display_view
        elif self.current_view_mode == "rotation":
            return self.rotation_suggestions_view
        # "list", "icons" use tree_display_view
        return self.tree_display_view

    def set_view_mode_list(self):
        selected_paths = self.main_window._get_selected_file_paths_from_view()
        focused_path = self.main_window.app_state.focused_image_path
        self.current_view_mode = "list"
        self.tree_display_view.setVisible(True)
        self.grid_display_view.setVisible(False)
        self.rotation_suggestions_view.setVisible(False)
        self.tree_display_view.setIconSize(QSize(16, 16))
        self.tree_display_view.setIndentation(10)
        self.tree_display_view.setRootIsDecorated(
            self.main_window.show_folders_mode
            or self.main_window.group_by_similarity_mode
        )
        self.tree_display_view.setItemsExpandable(
            self.main_window.show_folders_mode
            or self.main_window.group_by_similarity_mode
        )
        if self.tree_display_view.itemDelegate() is self.thumbnail_delegate:
            self.tree_display_view.setItemDelegate(None)
        self.update_view_button_states()
        self.main_window._rebuild_model_view(
            preserved_selection_paths=selected_paths,
            preserved_focused_path=focused_path,
        )
        self.tree_display_view.setFocus()

    def set_view_mode_icons(self):
        selected_paths = self.main_window._get_selected_file_paths_from_view()
        focused_path = self.main_window.app_state.focused_image_path
        self.current_view_mode = "icons"
        self.tree_display_view.setVisible(True)
        self.grid_display_view.setVisible(False)
        self.rotation_suggestions_view.setVisible(False)
        self.tree_display_view.setIconSize(QSize(64, 64))
        self.tree_display_view.setIndentation(20)
        self.tree_display_view.setRootIsDecorated(
            self.main_window.show_folders_mode
            or self.main_window.group_by_similarity_mode
        )
        self.tree_display_view.setItemsExpandable(
            self.main_window.show_folders_mode
            or self.main_window.group_by_similarity_mode
        )
        if self.tree_display_view.itemDelegate() is self.thumbnail_delegate:
            self.tree_display_view.setItemDelegate(None)
        self.update_view_button_states()
        self.main_window._rebuild_model_view(
            preserved_selection_paths=selected_paths,
            preserved_focused_path=focused_path,
        )
        self.tree_display_view.setFocus()

    def set_view_mode_grid(self):
        selected_paths = self.main_window._get_selected_file_paths_from_view()
        focused_path = self.main_window.app_state.focused_image_path
        self.current_view_mode = "grid"
        if (
            self.main_window.group_by_similarity_mode
        ):  # Grid view not supported when grouping by similarity
            self.tree_display_view.setVisible(True)
            self.grid_display_view.setVisible(False)
            self.rotation_suggestions_view.setVisible(False)
            # Use a suitable icon size for tree when grid would have been active
            self.tree_display_view.setIconSize(QSize(96, 96))
            self.tree_display_view.setIndentation(20)
            self.tree_display_view.setRootIsDecorated(True)
            self.tree_display_view.setItemsExpandable(True)
            if self.tree_display_view.itemDelegate() is self.thumbnail_delegate:
                self.tree_display_view.setItemDelegate(None)
            self.update_view_button_states()
            self.main_window._rebuild_model_view(
                preserved_selection_paths=selected_paths,
                preserved_focused_path=focused_path,
            )
            self.tree_display_view.setFocus()
        else:
            self.tree_display_view.setVisible(False)
            self.grid_display_view.setVisible(True)
            self.rotation_suggestions_view.setVisible(False)
            self.grid_display_view.setViewMode(QListView.ViewMode.IconMode)
            self.grid_display_view.setFlow(QListView.Flow.LeftToRight)
            self.grid_display_view.setWrapping(True)
            self.grid_display_view.setResizeMode(QListView.ResizeMode.Adjust)
            self.update_view_button_states()
            self.main_window._rebuild_model_view(
                preserved_selection_paths=selected_paths,
                preserved_focused_path=focused_path,
            )  # Populate model first
            self.update_grid_view_layout()  # Then adjust layout
            self.grid_display_view.setFocus()

    def set_view_mode_date(self):
        selected_paths = self.main_window._get_selected_file_paths_from_view()
        focused_path = self.main_window.app_state.focused_image_path
        self.current_view_mode = "date"
        self.tree_display_view.setVisible(True)
        self.grid_display_view.setVisible(False)
        self.rotation_suggestions_view.setVisible(False)
        self.tree_display_view.setIconSize(QSize(16, 16))
        self.tree_display_view.setIndentation(20)
        self.tree_display_view.setRootIsDecorated(True)
        self.tree_display_view.setItemsExpandable(True)
        if self.tree_display_view.itemDelegate() is self.thumbnail_delegate:
            self.tree_display_view.setItemDelegate(None)
        self.update_view_button_states()
        self.main_window._rebuild_model_view(
            preserved_selection_paths=selected_paths,
            preserved_focused_path=focused_path,
        )
        self.tree_display_view.setFocus()

    def set_view_mode_rotation(self):
        selected_paths = self.main_window._get_selected_file_paths_from_view()
        focused_path = self.main_window.app_state.focused_image_path
        self.current_view_mode = "rotation"
        self.tree_display_view.setVisible(False)
        self.grid_display_view.setVisible(False)
        self.rotation_suggestions_view.setVisible(True)
        self.update_view_button_states()
        self.main_window._rebuild_model_view(
            preserved_selection_paths=selected_paths,
            preserved_focused_path=focused_path,
        )
        self.rotation_suggestions_view.setFocus()

    def update_view_button_states(self):
        """Update the visual state of view mode icon buttons"""
        # Reset all icon buttons
        self.view_list_icon.setChecked(False)
        self.view_icons_icon.setChecked(False)
        self.view_grid_icon.setChecked(False)
        self.view_rotation_icon.setChecked(False)

        # Set the active icon button
        if self.current_view_mode == "list":
            self.view_list_icon.setChecked(True)
        elif self.current_view_mode == "icons":
            self.view_icons_icon.setChecked(True)
        elif self.current_view_mode == "grid":
            self.view_grid_icon.setChecked(True)
        elif self.current_view_mode == "rotation":
            self.view_rotation_icon.setChecked(True)

        view_label = {
            "list": "List view",
            "icons": "Icons view",
            "grid": "Grid view",
            "rotation": "Rotation view",
            "date": "Date view",
        }.get(self.current_view_mode, "List view")
        self.browser_mode_label.setText(view_label)

    def update_context(
        self,
        folder_name: str | None,
        item_count: int,
        subtitle: str | None = None,
    ):
        title = folder_name or "No folder selected"
        subtitle_text = subtitle or "Open a folder to start sorting your library."

        self.sidebar_title.setText(title)
        self.sidebar_subtitle.setText(subtitle_text)
        noun = "item" if item_count == 1 else "items"
        self.items_badge.setText(f"{item_count} {noun}")

    def update_grid_view_layout(self):
        if not self.grid_display_view.isVisible():
            return

        # Fixed grid layout to prevent filename length from affecting layout
        FIXED_GRID_SIZE = QSize(
            FIXED_GRID_WIDTH, FIXED_GRID_HEIGHT
        )  # Fixed grid cell size (width, height)

        # Set fixed icon size and grid properties
        self.grid_display_view.setIconSize(QSize(FIXED_ICON_SIZE, FIXED_ICON_SIZE))
        self.grid_display_view.setGridSize(FIXED_GRID_SIZE)
        self.grid_display_view.setSpacing(GRID_SPACING)

        # Ensure uniform grid layout
        self.grid_display_view.setUniformItemSizes(True)
        self.grid_display_view.setWordWrap(True)

        self.grid_display_view.updateGeometries()
        self.grid_display_view.viewport().update()

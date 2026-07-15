import logging
import os
import sys
import time
import subprocess
from contextlib import suppress
from collections.abc import Callable, Iterable
from typing import override

from PyQt6.QtCore import (
    QEvent,
    QObject,
    QPoint,
    QRunnable,
    QSize,
    Qt,
    QThreadPool,
    QTimer,
    QUrl,
    pyqtSignal,
)
from PyQt6.QtGui import (
    QAction,
    QBrush,
    QColor,
    QDesktopServices,
    QDragEnterEvent,
    QDragLeaveEvent,
    QDragMoveEvent,
    QDropEvent,
    QIcon,
    QKeyEvent,
    QPixmap,
)
from PyQt6.QtWidgets import (
    QAbstractItemView,
    QApplication,
    QButtonGroup,
    QCheckBox,
    QDialog,
    QFrame,
    QHBoxLayout,
    QInputDialog,
    QLabel,
    QListView,
    QListWidget,
    QListWidgetItem,
    QMenu,
    QMessageBox,
    QPlainTextEdit,
    QProgressBar,
    QPushButton,
    QSplitter,
    QStackedWidget,
    QStyle,
    QTreeWidget,
    QTreeWidgetItem,
    QVBoxLayout,
    QWidget,
)

from core.grouping import (
    GroupingGroup,
    GroupingPlan,
    augment_grouping_plan_with_filesystem_paths,
    find_directory_rename_candidates,
)
from core.media_utils import SUPPORTED_MEDIA_EXTENSIONS
from core.app_settings import (
    get_location_grouping_depth,
    set_location_grouping_depth,
    get_companion_files_preference,
    set_companion_files_preference,
    DISPLAY_MAX_RESOLUTION,
    THUMBNAIL_PRELOAD_BATCH_SIZE,
    THUMBNAIL_PRELOAD_VISIBLE_MARGIN,
)
from ui.advanced_image_viewer import ZoomableImageView
from ui.dialog_components import (
    build_dialog_footer,
    build_dialog_header,
    make_dialog_draggable,
)
from ui.workflow_review_components import (
    ORGANIZE_SHORTCUTS,
    WorkflowShortcutStrip,
    install_workflow_shortcuts,
)


GROUPING_MODE_OPTIONS = [
    ("Current", "current"),
    ("Mixed", "mixed"),
    ("Similarity", "similarity"),
    ("Face", "face"),
    ("Location", "location"),
]

ROLE_KIND = int(Qt.ItemDataRole.UserRole)
ROLE_SOURCE_PATH = ROLE_KIND + 1
ROLE_PROJECTED_PATH = ROLE_KIND + 2
ROLE_RELATIVE_PATH = ROLE_KIND + 3
ROLE_GROUP_ID = ROLE_KIND + 4
ROLE_ACTUAL_PATH = ROLE_KIND + 5
ROLE_BUCKET = ROLE_KIND + 6
ROLE_PARENT_RELATIVE_PATH = ROLE_KIND + 7
ROLE_MATCH_RELATIVE_PATH = ROLE_KIND + 8

ITEM_ROOT = "root"
ITEM_DIRECTORY = "directory"
ITEM_GROUP = "group"
ITEM_FILE = "file"
ITEM_UNASSIGNED = "unassigned"
ITEM_SKIPPED = "skipped"

PREVIEW_PAGE_HINT = 0
PREVIEW_PAGE_IMAGE = 1
PREVIEW_PAGE_FOLDER = 2

ROOT_LEVEL_GROUP_LABEL = "Root files"
SELECTED_PREVIEW_DISPLAY_SIZE = DISPLAY_MAX_RESOLUTION
MAX_FOLDER_PREVIEW_ITEMS = 200

_DROP_TARGET_KINDS = {ITEM_GROUP, ITEM_DIRECTORY, ITEM_ROOT, ITEM_UNASSIGNED}
_DRAGGABLE_KINDS = {ITEM_FILE, ITEM_GROUP}

logger = logging.getLogger(__name__)


def validate_directory_inventory(
    directory_path: str, represented_paths: Iterable[str]
) -> tuple[bool, str]:
    """Reject folder deletion when any descendant entry is not represented."""
    represented = {
        os.path.normcase(os.path.normpath(path)) for path in represented_paths
    }
    discovered: set[str] = set()
    try:
        for current_root, dirnames, filenames in os.walk(
            directory_path, followlinks=False
        ):
            for dirname in dirnames:
                candidate = os.path.join(current_root, dirname)
                if os.path.islink(candidate):
                    discovered.add(os.path.normcase(os.path.normpath(candidate)))
            for filename in filenames:
                candidate = os.path.join(current_root, filename)
                discovered.add(os.path.normcase(os.path.normpath(candidate)))
    except OSError as exc:
        return False, f"Folder safety check failed: {exc}"

    missing_from_preview = discovered - represented
    stale_preview = represented - discovered
    if missing_from_preview or stale_preview:
        details = []
        if missing_from_preview:
            details.append(f"{len(missing_from_preview)} unshown item(s)")
        if stale_preview:
            details.append(f"{len(stale_preview)} stale preview item(s)")
        return (
            False,
            "Folder was not marked or trashed because it contains "
            + " and ".join(details)
            + ".",
        )
    return True, ""


class _DirectoryInventorySignals(QObject):
    completed = pyqtSignal(int, str, str, object, bool, str)


class _DirectoryInventoryTask(QRunnable):
    def __init__(
        self,
        request_id: int,
        operation: str,
        directory_path: str,
        represented_paths: list[str],
    ) -> None:
        super().__init__()
        self.request_id = request_id
        self.operation = operation
        self.directory_path = directory_path
        self.represented_paths = represented_paths
        self.signals = _DirectoryInventorySignals()

    @override
    def run(self) -> None:
        safe, reason = validate_directory_inventory(
            self.directory_path, self.represented_paths
        )
        self.signals.completed.emit(
            self.request_id,
            self.operation,
            self.directory_path,
            self.represented_paths,
            safe,
            reason,
        )


class DroppableGroupingTree(QTreeWidget):
    """QTreeWidget subclass that supports drag-and-drop for the After tree."""

    def __init__(
        self, grouping_widget: GroupingStepWidget, parent: QWidget | None = None
    ):
        super().__init__(parent)
        self._grouping_widget = grouping_widget
        self._highlighted_item: QTreeWidgetItem | None = None
        self._original_background: QBrush | None = None
        self.setDragEnabled(True)
        self.viewport().setAcceptDrops(True)
        self.setDropIndicatorShown(False)

    def _is_drop_noop(
        self,
        dragged_files: list[str],
        dragged_group_ids: list[str],
        target_item: QTreeWidgetItem,
    ) -> bool:
        """Check if all dragged items are already in the target location."""
        if not dragged_files and not dragged_group_ids:
            return False
        gw = self._grouping_widget
        target_kind = target_item.data(0, ROLE_KIND)

        if target_kind == ITEM_GROUP:
            target_group_id = target_item.data(0, ROLE_GROUP_ID)
            target_group = gw._find_group(target_group_id)
            if target_group:
                files_in_target = all(
                    f in target_group.source_paths for f in dragged_files
                )
                groups_already_there = all(
                    gid == target_group_id for gid in dragged_group_ids
                )
                return files_in_target and groups_already_there
        elif target_kind == ITEM_DIRECTORY:
            target_rel = target_item.data(0, ROLE_RELATIVE_PATH) or ""
            for g in gw._editable_groups:
                if g.group_label == target_rel:
                    files_in_target = all(f in g.source_paths for f in dragged_files)
                    groups_already_there = all(
                        (found := gw._find_group(gid)) is not None
                        and found.group_label == target_rel
                        for gid in dragged_group_ids
                    )
                    return files_in_target and groups_already_there

        return False

    # ------------------------------------------------------------------
    # Drag event overrides
    # ------------------------------------------------------------------

    @override
    def dragEnterEvent(self, event: QDragEnterEvent | None) -> None:
        if event and event.source() is self:
            self._grouping_widget._drag_in_progress = True
            event.acceptProposedAction()
        elif event:
            event.ignore()

    def dragMoveEvent(self, event: QDragMoveEvent | None) -> None:
        if not event or event.source() is not self:
            if event:
                event.ignore()
            return

        target_item = self._resolve_drop_target(self.itemAt(event.position().toPoint()))
        if target_item is not None and self._is_valid_drop_target(target_item):
            self._highlight_drop_target(target_item)
            event.acceptProposedAction()
        else:
            self._clear_drop_highlight()
            event.ignore()

    def dragLeaveEvent(self, event: QDragLeaveEvent | None) -> None:
        self._clear_drop_highlight()
        self._grouping_widget._drag_in_progress = False
        if event:
            super().dragLeaveEvent(event)

    @override
    def dropEvent(self, event: QDropEvent | None) -> None:
        if not event or event.source() is not self:
            if event:
                event.ignore()
            return

        target_item = self._resolve_drop_target(self.itemAt(event.position().toPoint()))
        if target_item is None or not self._is_valid_drop_target(target_item):
            self._clear_drop_highlight()
            self._grouping_widget._drag_in_progress = False
            event.ignore()
            return

        self._clear_drop_highlight()
        self._grouping_widget._drag_in_progress = False
        dragged_files, dragged_group_ids = self._get_selected_drag_data()

        if not dragged_files and not dragged_group_ids:
            event.ignore()
            return

        if self._is_drop_noop(dragged_files, dragged_group_ids, target_item):
            event.acceptProposedAction()
            return

        if dragged_files:
            self._grouping_widget._check_and_handle_companion_preference(dragged_files)

        target_kind = target_item.data(0, ROLE_KIND)
        gw = self._grouping_widget

        if target_kind == ITEM_GROUP:
            target_group_id = target_item.data(0, ROLE_GROUP_ID)
            if dragged_files:
                gw._move_paths(dragged_files, target_group_id, keep_empty_groups=False)
            if dragged_group_ids:
                for gid in dragged_group_ids:
                    if gid != target_group_id:
                        gw._nest_group_under_target(gid, target_group_id)

        elif target_kind == ITEM_DIRECTORY:
            target_rel = target_item.data(0, ROLE_RELATIVE_PATH) or ""
            if dragged_files:
                gw._move_files_to_directory(dragged_files, target_rel)
            if dragged_group_ids:
                for gid in dragged_group_ids:
                    gw._nest_group_under_directory(gid, target_rel)

        elif target_kind == ITEM_ROOT:
            if dragged_files:
                gw._move_paths_to_unassigned(dragged_files)
            if dragged_group_ids:
                for gid in dragged_group_ids:
                    gw._unnest_group_to_root(gid)

        elif target_kind == ITEM_UNASSIGNED:
            if dragged_files:
                gw._move_paths_to_unassigned(dragged_files)
            # Groups cannot be dropped onto unassigned

        event.acceptProposedAction()

    # ------------------------------------------------------------------
    # Helpers
    # ------------------------------------------------------------------

    def _resolve_drop_target(
        self, item: QTreeWidgetItem | None
    ) -> QTreeWidgetItem | None:
        """If hovering over a file, resolve to its parent folder/group instead."""
        if item is None:
            return None
        kind = item.data(0, ROLE_KIND)
        if kind == ITEM_FILE:
            parent = item.parent()
            return parent if parent is not None else None
        return item

    def _is_valid_drop_target(self, target_item: QTreeWidgetItem) -> bool:
        target_kind = target_item.data(0, ROLE_KIND)
        if target_kind not in _DROP_TARGET_KINDS:
            return False

        selected = self.selectedItems()
        if not selected:
            return False

        for item in selected:
            if item is target_item:
                return False
            item_kind = item.data(0, ROLE_KIND)
            if item_kind not in _DRAGGABLE_KINDS:
                continue
            if item_kind == ITEM_GROUP:
                if self._is_descendant(target_item, item):
                    return False
                source_rel = item.data(0, ROLE_RELATIVE_PATH) or ""
                target_rel = target_item.data(0, ROLE_RELATIVE_PATH) or ""
                if self._is_relative_path_descendant(target_rel, source_rel):
                    return False

        return True

    def _get_selected_drag_data(self) -> tuple:
        dragged_files: list[str] = []
        dragged_group_ids: list[str] = []
        for item in self.selectedItems():
            kind = item.data(0, ROLE_KIND)
            if kind == ITEM_FILE:
                source_path = item.data(0, ROLE_SOURCE_PATH)
                if source_path:
                    dragged_files.append(source_path)
            elif kind == ITEM_GROUP:
                group_id = item.data(0, ROLE_GROUP_ID)
                if group_id:
                    dragged_group_ids.append(group_id)
        return dragged_files, dragged_group_ids

    def _highlight_drop_target(self, item: QTreeWidgetItem) -> None:
        if item is self._highlighted_item:
            return
        self._clear_drop_highlight()
        self._original_background = item.background(0)
        self._highlighted_item = item
        from core.app_settings import GROUPING_DROP_HIGHLIGHT_COLOR

        r, g, b, a = GROUPING_DROP_HIGHLIGHT_COLOR
        item.setBackground(0, QBrush(QColor(r, g, b, a)))

    def _clear_drop_highlight(self) -> None:
        if self._highlighted_item is not None:
            with suppress(RuntimeError):
                self._highlighted_item.setBackground(
                    0, self._original_background or QBrush()
                )
        self._highlighted_item = None
        self._original_background = None

    @staticmethod
    def _is_descendant(child: QTreeWidgetItem, ancestor: QTreeWidgetItem) -> bool:
        current = child.parent()
        while current is not None:
            if current is ancestor:
                return True
            current = current.parent()
        return False

    @staticmethod
    def _is_relative_path_descendant(candidate: str, ancestor: str) -> bool:
        normalized_candidate = os.path.normpath(candidate or "").strip(os.sep)
        normalized_ancestor = os.path.normpath(ancestor or "").strip(os.sep)
        if not normalized_candidate or not normalized_ancestor:
            return False
        return normalized_candidate.startswith(normalized_ancestor + os.sep)


class GroupingStepWidget(QWidget):
    mode_changed = pyqtSignal(str)
    create_requested = pyqtSignal(str, dict, object)
    back_requested = pyqtSignal()
    skip_requested = pyqtSignal()
    select_folder_requested = pyqtSignal()
    toggle_deletion_marks_requested = pyqtSignal(list)
    commit_deletion_marks_requested = pyqtSignal()
    clear_deletion_marks_requested = pyqtSignal()
    trash_requested = pyqtSignal(str, list)

    def __init__(self, parent=None):
        super().__init__(parent)
        self._parent_window = parent
        self._current_plan: GroupingPlan | None = None
        self._source_root: str | None = None
        self._current_output_root: str = ""
        self._editable_groups: list[GroupingGroup] = []
        self._editable_unassigned: list[str] = []
        self._editable_skipped: list[str] = []
        self._file_name_overrides: dict[str, str] = {}
        self._original_group_labels_by_path: dict[str, str] = {}
        self._original_group_labels_by_group_id: dict[str, str] = {}
        self._sticky_empty_group_ids: set[str] = set()
        self._group_id_counter: int = 0
        self._total_items: int = 0
        self._supported_items: int = 0
        self._ignore_preview_item_change = False
        self._syncing_tree_selection = False
        self._drag_in_progress = False
        self._current_preview_source_path: str | None = None
        self._is_marked_func: Callable[[str], bool] = lambda _path: False
        self._folder_validation_request_id = 0
        self._folder_validation_pool = QThreadPool.globalInstance()

        self._before_root_item: QTreeWidgetItem | None = None
        self._after_root_item: QTreeWidgetItem | None = None
        self._before_file_items_by_path: dict[str, QTreeWidgetItem] = {}
        self._before_dir_items_by_relative_path: dict[str, QTreeWidgetItem] = {}
        self._after_file_items_by_path: dict[str, QTreeWidgetItem] = {}
        self._after_dir_items_by_relative_path: dict[str, QTreeWidgetItem] = {}
        self._after_group_items_by_id: dict[str, QTreeWidgetItem] = {}
        self._after_group_items_by_label: dict[str, QTreeWidgetItem] = {}
        self._after_items_by_match_relative_path: dict[str, QTreeWidgetItem] = {}
        self._folder_icon = self.style().standardIcon(QStyle.StandardPixmap.SP_DirIcon)
        self._file_icon = self.style().standardIcon(QStyle.StandardPixmap.SP_FileIcon)

        self._create_widgets()
        self._create_layout()
        self._connect_signals()
        self._shortcuts = install_workflow_shortcuts(
            self,
            ORGANIZE_SHORTCUTS,
            {
                "modes:Alt+1": lambda: self._shortcut_set_mode("current"),
                "modes:Alt+2": lambda: self._shortcut_set_mode("mixed"),
                "modes:Alt+3": lambda: self._shortcut_set_mode("similarity"),
                "modes:Alt+4": lambda: self._shortcut_set_mode("face"),
                "modes:Alt+5": lambda: self._shortcut_set_mode("location"),
                "rename": self._shortcut_rename,
                "new_folder": self._shortcut_new_folder,
                "toggle_delete": self._shortcut_toggle_delete,
                "trash_now": self._shortcut_trash_now,
                "commit_deletions": self.commit_deletion_marks_requested.emit,
                "clear_deletions": self.clear_deletion_marks_requested.emit,
                "apply": self._shortcut_apply,
            },
        )
        self.set_source_folder(None)

    def _create_widgets(self) -> None:
        self.top_bar = QFrame()
        self.top_bar.setObjectName("groupingTopBar")

        self.back_button = QPushButton("← Back")
        self.back_button.setObjectName("groupingGhostButton")
        self.back_button.setVisible(False)

        self.folder_button = QPushButton("📁  Select Folder")
        self.folder_button.setObjectName("groupingFolderButton")

        self.folder_path_label = QLabel("No folder selected")
        self.folder_path_label.setObjectName("groupingFolderPath")

        self._mode_button_group = QButtonGroup(self)
        self._mode_button_group.setExclusive(True)
        self._mode_buttons: dict[str, QPushButton] = {}
        for label, value in GROUPING_MODE_OPTIONS:
            btn = QPushButton(label)
            btn.setObjectName("groupingModePill")
            btn.setCheckable(True)
            self._mode_button_group.addButton(btn)
            self._mode_buttons[value] = btn
        self._mode_buttons["current"].setChecked(True)

        self._location_depth_widget = QWidget()
        self._location_depth_widget.setObjectName("locationDepthWidget")
        _ldw_layout = QHBoxLayout(self._location_depth_widget)
        _ldw_layout.setContentsMargins(0, 0, 0, 0)
        _ldw_layout.setSpacing(2)
        _ldw_sep = QFrame()
        _ldw_sep.setObjectName("groupingTopBarSep")
        _ldw_sep.setFrameShape(QFrame.Shape.VLine)
        _ldw_layout.addWidget(_ldw_sep)
        _ldw_label = QLabel("Levels")
        _ldw_label.setObjectName("locationDepthLabel")
        _ldw_layout.addWidget(_ldw_label)
        _ldw_layout.addSpacing(2)
        self._location_depth_group = QButtonGroup(self)
        self._location_depth_group.setExclusive(True)
        self._location_depth_buttons: dict[int, QPushButton] = {}
        _depth_tips = {
            1: "Country only (e.g. Japan)",
            2: "Country + city (e.g. Japan/Fuji)",
            3: "Country + state + city (e.g. Japan/Shizuoka/Fuji)",
        }
        for depth_val in range(1, 4):
            btn = QPushButton(str(depth_val))
            btn.setObjectName("groupingModePill")
            btn.setCheckable(True)
            btn.setFixedSize(30, 26)
            btn.setToolTip(_depth_tips.get(depth_val, ""))
            self._location_depth_group.addButton(btn)
            self._location_depth_buttons[depth_val] = btn
            _ldw_layout.addWidget(btn)
        saved_depth = get_location_grouping_depth()
        self._location_depth_buttons.get(
            saved_depth, self._location_depth_buttons[3]
        ).setChecked(True)
        self._location_depth_widget.setVisible(False)

        self._depth_debounce = QTimer(self)
        self._depth_debounce.setSingleShot(True)

        self.stats_label = QLabel()
        self.stats_label.setObjectName("groupingSummaryBadge")
        self.stats_label.setVisible(False)

        self.skip_button = QPushButton("Continue without organizing")
        self.skip_button.setObjectName("groupingGhostButton")
        self.skip_button.setEnabled(False)

        self.primary_button = QPushButton("Review, then Apply")
        self.primary_button.setObjectName("groupingPrimaryButton")
        self.primary_button.setMinimumHeight(34)
        self.primary_button.setEnabled(False)

        self.empty_state_frame = QFrame()
        self.empty_state_frame.setObjectName("groupingEmptyState")
        self._empty_icon = QLabel("🗂")
        self._empty_icon.setObjectName("groupingEmptyIcon")
        self._empty_icon.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self._empty_title = QLabel("Select a source folder to begin")
        self._empty_title.setObjectName("groupingEmptyTitle")
        self._empty_title.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self._empty_subtitle = QLabel(
            "PhotoSort will analyse your photos and generate a structured folder plan.\n"
            "Review the before/after trees, refine the staged changes, then commit."
        )
        self._empty_subtitle.setObjectName("groupingEmptySubtitle")
        self._empty_subtitle.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self._empty_subtitle.setWordWrap(True)
        self._empty_cta = QPushButton("📁  Select Folder")
        self._empty_cta.setObjectName("groupingEmptyCTA")

        self.before_panel = QFrame()
        self.before_panel.setObjectName("groupingBeforePanel")
        self.before_header = QLabel("Before")
        self.before_header.setObjectName("groupingTreeHeader")
        self.before_desc = QLabel("Current structure")
        self.before_desc.setObjectName("groupingTreeDesc")
        self.before_tree = QTreeWidget()
        self.before_tree.setObjectName("groupingTree")
        self.before_tree.setColumnCount(1)
        self.before_tree.setHeaderHidden(True)
        self.before_tree.setRootIsDecorated(True)
        self.before_tree.setAlternatingRowColors(False)
        self.before_tree.setIndentation(14)
        self.before_tree.setUniformRowHeights(True)
        self.before_tree.setSelectionMode(
            QAbstractItemView.SelectionMode.SingleSelection
        )
        self.before_tree.setIconSize(QSize(22, 22))
        self.before_tree.setContextMenuPolicy(Qt.ContextMenuPolicy.CustomContextMenu)

        self.after_panel = QFrame()
        self.after_panel.setObjectName("groupingAfterPanel")
        self.after_header = QLabel("After")
        self.after_header.setObjectName("groupingTreeHeader")
        self.after_desc = QLabel("Drag items to move \u00b7 Double-click to rename")
        self.after_desc.setObjectName("groupingTreeDesc")
        self.preview_tree = DroppableGroupingTree(self)
        self.preview_tree.setObjectName("groupingTree")
        self.preview_tree.setColumnCount(1)
        self.preview_tree.setHeaderHidden(True)
        self.preview_tree.setRootIsDecorated(True)
        self.preview_tree.setAlternatingRowColors(False)
        self.preview_tree.setIndentation(14)
        self.preview_tree.setUniformRowHeights(True)
        self.preview_tree.setSelectionMode(
            QAbstractItemView.SelectionMode.ExtendedSelection
        )
        self.preview_tree.setIconSize(QSize(22, 22))
        self.preview_tree.setContextMenuPolicy(Qt.ContextMenuPolicy.CustomContextMenu)

        self.preview_panel = QFrame()
        self.preview_panel.setObjectName("groupingPreviewPanel")
        self.preview_panel_header = QLabel("Preview")
        self.preview_panel_header.setObjectName("groupingTreeHeader")
        self.preview_pane_stack = QStackedWidget()
        self.preview_hint_label = QLabel("Select a photo or folder to preview")
        self.preview_hint_label.setObjectName("groupingPreviewHint")
        self.preview_hint_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.large_preview_view = ZoomableImageView()
        self.large_preview_view.setObjectName("groupingLargePreview")
        self.large_preview_name = QLabel()
        self.large_preview_name.setObjectName("groupingSelectionName")
        self.large_preview_name.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.large_preview_name.setWordWrap(True)
        self.preview_trash_button = QPushButton("🗑️")
        self.preview_trash_button.setObjectName("groupingPreviewTrashButton")
        self.preview_trash_button.setToolTip(
            "Move the current image to Trash now (Delete / Backspace)"
        )
        self.preview_trash_button.setEnabled(False)
        self.folder_preview_page = QWidget()
        self.folder_preview_title = QLabel()
        self.folder_preview_title.setObjectName("groupingSelectionName")
        self.folder_preview_title.setWordWrap(True)
        self.folder_preview_meta = QLabel()
        self.folder_preview_meta.setObjectName("groupingPathValue")
        self.folder_preview_meta.setWordWrap(True)
        self.folder_preview_grid = QListWidget()
        self.folder_preview_grid.setObjectName("groupingFolderPreviewGrid")
        self.folder_preview_grid.setViewMode(QListView.ViewMode.IconMode)
        self.folder_preview_grid.setResizeMode(QListView.ResizeMode.Adjust)
        self.folder_preview_grid.setMovement(QListView.Movement.Static)
        self.folder_preview_grid.setWrapping(True)
        self.folder_preview_grid.setSpacing(10)
        self.folder_preview_grid.setIconSize(QSize(120, 120))
        self.folder_preview_grid.setSelectionMode(
            QAbstractItemView.SelectionMode.SingleSelection
        )
        self.folder_preview_grid.setUniformItemSizes(False)
        self.folder_preview_grid.setWordWrap(True)

        self.stacked = QStackedWidget()
        self.main_splitter = QSplitter(Qt.Orientation.Horizontal)
        self.main_splitter.setObjectName("groupingMainSplitter")
        self.main_splitter.setHandleWidth(1)

        self.bottom_bar = QFrame()
        self.bottom_bar.setObjectName("groupingBottomBar")

        self.loading_label = QLabel("Select a folder to start.")
        self.loading_label.setObjectName("groupingLoadingLabel")
        self.loading_bar = QProgressBar()
        self.loading_bar.setObjectName("groupingLoadingBar")
        self.loading_bar.setTextVisible(False)
        self.loading_bar.setRange(0, 1)
        self.loading_bar.setValue(0)
        self.loading_bar.setVisible(False)

        self.thumb_label = QLabel()
        self.thumb_label.setObjectName("groupingThumb")
        self.thumb_label.setFixedSize(52, 52)
        self.thumb_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.thumb_label.setVisible(False)

        self.preview_selection_label = QLabel()
        self.preview_selection_label.setObjectName("groupingSelectionName")
        self.preview_selection_label.setVisible(False)

        self.preview_selection_meta = QLabel()
        self.preview_selection_meta.setObjectName("groupingPathValue")
        self.preview_selection_meta.setVisible(False)

        self.open_preview_button = QPushButton("Open Preview")
        self.open_preview_button.setObjectName("groupingSecondaryButton")
        self.open_preview_button.setEnabled(False)
        self.open_preview_button.setVisible(False)

        self.shortcut_strip = WorkflowShortcutStrip(ORGANIZE_SHORTCUTS)

        self.output_root_label = QLabel()
        self.output_root_label.setVisible(False)
        self.preview_label = self.loading_label
        self.preview_stats_label = self.stats_label
        self.preview_image_label = self.large_preview_view

    def _create_layout(self) -> None:
        root = QVBoxLayout(self)
        root.setContentsMargins(0, 0, 0, 0)
        root.setSpacing(0)

        tb = QHBoxLayout(self.top_bar)
        tb.setContentsMargins(16, 10, 16, 10)
        tb.setSpacing(6)
        tb.addWidget(self.back_button)
        tb.addWidget(self.folder_button)
        tb.addWidget(self.folder_path_label, 1)
        sep = QFrame()
        sep.setObjectName("groupingTopBarSep")
        sep.setFrameShape(QFrame.Shape.VLine)
        tb.addWidget(sep)
        for _, value in GROUPING_MODE_OPTIONS:
            tb.addWidget(self._mode_buttons[value])
        tb.addWidget(self._location_depth_widget)
        tb.addSpacing(10)
        tb.addWidget(self.stats_label)
        tb.addStretch(1)
        tb.addWidget(self.skip_button)
        tb.addSpacing(6)
        tb.addWidget(self.primary_button)

        root.addWidget(self.top_bar)
        self._add_hsep(root, "groupingBarSep")

        es = QVBoxLayout(self.empty_state_frame)
        es.setContentsMargins(0, 0, 0, 0)
        es.setSpacing(14)
        es.addStretch(1)
        es.addWidget(self._empty_icon)
        es.addWidget(self._empty_title)
        es.addWidget(self._empty_subtitle)
        cta_row = QHBoxLayout()
        cta_row.addStretch(1)
        cta_row.addWidget(self._empty_cta)
        cta_row.addStretch(1)
        es.addLayout(cta_row)
        es.addStretch(1)

        bl = QVBoxLayout(self.before_panel)
        bl.setContentsMargins(0, 0, 0, 0)
        bl.setSpacing(0)
        bh = QHBoxLayout()
        bh.setContentsMargins(16, 12, 16, 10)
        bh.setSpacing(8)
        bh.addWidget(self.before_header)
        bh.addWidget(self.before_desc)
        bh.addStretch(1)
        bl.addLayout(bh)
        self._add_hsep(bl, "groupingPanelSep")
        bl.addWidget(self.before_tree, 1)

        al = QVBoxLayout(self.after_panel)
        al.setContentsMargins(0, 0, 0, 0)
        al.setSpacing(0)
        ah = QHBoxLayout()
        ah.setContentsMargins(16, 12, 16, 10)
        ah.setSpacing(8)
        ah.addWidget(self.after_header)
        ah.addWidget(self.after_desc)
        ah.addStretch(1)
        al.addLayout(ah)
        self._add_hsep(al, "groupingPanelSep")
        al.addWidget(self.preview_tree, 1)

        pv = QVBoxLayout(self.preview_panel)
        pv.setContentsMargins(0, 0, 0, 0)
        pv.setSpacing(0)
        pvh = QHBoxLayout()
        pvh.setContentsMargins(16, 12, 16, 10)
        pvh.setSpacing(8)
        pvh.addWidget(self.preview_panel_header)
        pvh.addStretch(1)
        pv.addLayout(pvh)
        self._add_hsep(pv, "groupingPanelSep")

        img_page = QWidget()
        img_layout = QVBoxLayout(img_page)
        img_layout.setContentsMargins(12, 12, 12, 10)
        img_layout.setSpacing(8)
        img_layout.addWidget(self.large_preview_view, 1)
        preview_caption = QHBoxLayout()
        preview_caption.addWidget(self.large_preview_name, 1)
        preview_caption.addWidget(self.preview_trash_button)
        img_layout.addLayout(preview_caption)
        folder_layout = QVBoxLayout(self.folder_preview_page)
        folder_layout.setContentsMargins(12, 12, 12, 10)
        folder_layout.setSpacing(8)
        folder_layout.addWidget(self.folder_preview_title)
        folder_layout.addWidget(self.folder_preview_meta)
        folder_layout.addWidget(self.folder_preview_grid, 1)
        self.preview_pane_stack.addWidget(self.preview_hint_label)
        self.preview_pane_stack.addWidget(img_page)
        self.preview_pane_stack.addWidget(self.folder_preview_page)
        pv.addWidget(self.preview_pane_stack, 1)

        self.main_splitter.addWidget(self.before_panel)
        self.main_splitter.addWidget(self.after_panel)
        self.main_splitter.addWidget(self.preview_panel)
        self.main_splitter.setStretchFactor(0, 1)
        self.main_splitter.setStretchFactor(1, 1)
        self.main_splitter.setStretchFactor(2, 1)
        self.main_splitter.setSizes([380, 380, 320])

        self.stacked.addWidget(self.empty_state_frame)
        self.stacked.addWidget(self.main_splitter)

        root.addWidget(self.stacked, 1)
        self._add_hsep(root, "groupingBarSep")

        bb = QHBoxLayout(self.bottom_bar)
        bb.setContentsMargins(16, 8, 16, 8)
        bb.setSpacing(10)
        info_col = QVBoxLayout()
        info_col.setSpacing(2)
        info_col.addWidget(self.preview_selection_label)
        info_col.addWidget(self.preview_selection_meta)
        bb.addLayout(info_col)
        bb.addStretch(1)
        bb.addWidget(self.shortcut_strip)
        loading_col = QVBoxLayout()
        loading_col.setSpacing(4)
        loading_col.addWidget(self.loading_label)
        loading_col.addWidget(self.loading_bar)
        bb.addLayout(loading_col)

        root.addWidget(self.bottom_bar)

    @staticmethod
    def _add_hsep(layout, name: str) -> None:
        sep = QFrame()
        sep.setObjectName(name)
        sep.setFrameShape(QFrame.Shape.HLine)
        layout.addWidget(sep)

    def _connect_signals(self) -> None:
        self._mode_button_group.buttonClicked.connect(self._emit_mode_changed)
        self.primary_button.clicked.connect(self._emit_create_requested)
        self.back_button.clicked.connect(self.back_requested.emit)
        self.skip_button.clicked.connect(self.skip_requested.emit)
        self.folder_button.clicked.connect(self.select_folder_requested.emit)
        self._empty_cta.clicked.connect(self.select_folder_requested.emit)
        self.preview_tree.currentItemChanged.connect(self._handle_after_item_changed)
        self.before_tree.currentItemChanged.connect(self._handle_before_item_changed)
        self.preview_tree.itemDoubleClicked.connect(self._handle_tree_double_click)
        self.preview_tree.itemChanged.connect(self._handle_preview_item_changed)
        self.before_tree.customContextMenuRequested.connect(
            self._show_before_context_menu
        )
        self.preview_tree.customContextMenuRequested.connect(
            self._show_after_context_menu
        )
        self.folder_preview_grid.itemActivated.connect(
            self._handle_folder_preview_item_activated
        )
        self.folder_preview_grid.itemClicked.connect(
            self._handle_folder_preview_item_activated
        )
        self.preview_trash_button.clicked.connect(self._trash_current_preview)
        self._location_depth_group.buttonClicked.connect(
            self._on_location_depth_changed
        )
        self._depth_debounce.timeout.connect(self._fire_depth_change)
        self.before_tree.installEventFilter(self)
        self.preview_tree.installEventFilter(self)

    def _shortcut_item(self) -> QTreeWidgetItem | None:
        focus = QApplication.focusWidget()
        if focus is None or not (focus is self or self.isAncestorOf(focus)):
            return None
        if self.preview_tree.state() == QAbstractItemView.State.EditingState:
            return None
        if focus is self.before_tree or self.before_tree.isAncestorOf(focus):
            return self.before_tree.currentItem()
        return self.preview_tree.currentItem()

    def _shortcut_set_mode(self, mode: str) -> None:
        button = self._mode_buttons.get(mode)
        if button is None or not button.isEnabled():
            return
        button.setChecked(True)
        self._emit_mode_changed()

    def _shortcut_rename(self) -> None:
        item = self._shortcut_item()
        if item is None:
            return
        kind = self._item_kind(item)
        if kind == ITEM_FILE:
            self._rename_file(item)
        elif kind == ITEM_GROUP:
            self._rename_group(item)

    def _shortcut_new_folder(self) -> None:
        item = self._shortcut_item()
        if item is not None:
            self._create_subgroup_from_item(item)

    def _shortcut_toggle_delete(self) -> None:
        item = self._shortcut_item()
        if item is not None:
            self._toggle_item_deletion_marks(item)

    def _shortcut_trash_now(self) -> None:
        item = self._shortcut_item()
        if item is not None:
            self._request_trash_for_item(item)

    def _shortcut_apply(self) -> None:
        if self.primary_button.isEnabled():
            self._emit_create_requested()

    def _on_location_depth_changed(self) -> None:
        self._depth_debounce.start(600)

    def _fire_depth_change(self) -> None:
        depth = self.get_location_depth()
        set_location_grouping_depth(depth)
        self.mode_changed.emit(self.current_mode())

    def _emit_mode_changed(self) -> None:
        mode = self.current_mode()
        self._location_depth_widget.setVisible(mode == "location")
        self.mode_changed.emit(mode)

    def get_location_depth(self) -> int:
        for val, btn in self._location_depth_buttons.items():
            if btn.isChecked():
                return val
        return 3

    def _emit_create_requested(self) -> None:
        effective_plan = self.get_effective_plan()
        if not self._confirm_grouping_actions(effective_plan):
            return
        all_source_paths = [
            p
            for g in (effective_plan.groups if effective_plan else [])
            for p in g.source_paths
        ] + (effective_plan.unassigned_paths if effective_plan else [])
        self._check_and_handle_companion_preference(all_source_paths)
        self.create_requested.emit(
            self.current_mode(),
            self.get_group_name_overrides(),
            effective_plan,
        )

    def _check_and_handle_companion_preference(self, source_paths) -> None:
        """Show dialog if companion files (.xmp / same-stem images) exist and preference not set."""
        if get_companion_files_preference() != "ask":
            return

        from core.grouping import _find_companion_files

        found: list[str] = []
        for path in source_paths:
            for comp in _find_companion_files(path):
                name = os.path.basename(comp)
                if name not in found:
                    found.append(name)
            if len(found) >= 5:
                break

        if not found:
            return

        sample = found[:3]
        more = len(found) - len(sample)
        sample_text = ", ".join(sample) + (f" (+{more} more)" if more else "")

        dlg = QDialog(self)
        dlg.setWindowTitle("Companion files found")
        dlg.setMinimumWidth(440)
        vlay = QVBoxLayout(dlg)
        vlay.setSpacing(12)
        vlay.setContentsMargins(20, 20, 20, 16)

        lbl = QLabel(
            f"Found companion files alongside your photos:\n{sample_text}\n\n"
            "These may be .xmp sidecars or same-name files in another format (e.g. RAW + JPEG).\n"
            "Move them to the same destination folder as their photos?"
        )
        lbl.setWordWrap(True)
        vlay.addWidget(lbl)

        remember_cb = QCheckBox("Remember my choice for all future operations")
        vlay.addWidget(remember_cb)

        btn_row = QHBoxLayout()
        btn_row.addStretch(1)
        no_btn = QPushButton("No, skip companions")
        yes_btn = QPushButton("Yes, move companions")
        yes_btn.setDefault(True)
        btn_row.addWidget(no_btn)
        btn_row.addWidget(yes_btn)
        vlay.addLayout(btn_row)

        choice = [False]

        def _accept():
            choice[0] = True
            dlg.accept()

        def _reject():
            choice[0] = False
            dlg.accept()

        yes_btn.clicked.connect(_accept)
        no_btn.clicked.connect(_reject)

        dlg.exec()

        if remember_cb.isChecked():
            set_companion_files_preference("always" if choice[0] else "never")

    def current_mode(self) -> str:
        for value, btn in self._mode_buttons.items():
            if btn.isChecked():
                return value
        return "current"

    def set_current_mode(self, mode: str) -> None:
        btn = self._mode_buttons.get(mode)
        if btn:
            btn.setChecked(True)

    def set_preview_text(self, text: str) -> None:
        self.loading_label.setText(text or "")

    def set_output_root_text(self, text: str) -> None:
        if text:
            self.after_desc.setText(text)

    def set_busy(self, busy: bool) -> None:
        self.primary_button.setEnabled(not busy and self._current_plan is not None)
        for btn in self._mode_buttons.values():
            btn.setEnabled(not busy and self.has_source_folder())
        for btn in self._location_depth_buttons.values():
            btn.setEnabled(not busy and self.has_source_folder())
        self.skip_button.setEnabled(not busy and self.has_source_folder())
        self.folder_button.setEnabled(not busy)
        self.back_button.setEnabled(not busy)
        self.primary_button.setText("Applying…" if busy else "Review, then Apply")

    def set_back_visible(self, visible: bool) -> None:
        self.back_button.setVisible(visible)

    def set_is_marked_func(self, func: Callable[[str], bool]) -> None:
        """Use the application-wide deletion state as Organize's source of truth."""
        self._is_marked_func = func
        self.refresh_deletion_state()

    def refresh_deletion_state(self) -> None:
        """Refresh mark presentation without rebuilding either tree or its icons."""
        if self._current_plan is not None:
            self._update_stats()
        for path, item in self._before_file_items_by_path.items():
            self._apply_deletion_presentation(item, path, editable=False)
        for path, item in self._after_file_items_by_path.items():
            self._apply_deletion_presentation(item, path, editable=True)
        for index in range(self.folder_preview_grid.count()):
            item = self.folder_preview_grid.item(index)
            if item is None:
                continue
            path = item.data(Qt.ItemDataRole.UserRole)
            if path:
                path = str(path)
                item.setText(self._preview_deletion_display_name(path))
                item.setForeground(
                    QBrush(
                        QColor("#FFB366")
                        if self._is_marked_func(path)
                        else QApplication.palette().text().color()
                    )
                )
        current = self._current_preview_source_path
        if current:
            display_name = self._preview_deletion_display_name(current)
            self.large_preview_name.setText(display_name)
            self.preview_selection_label.setText(display_name)
            self._apply_preview_deletion_style(current)

    def remove_deleted_paths(self, paths: Iterable[str]) -> None:
        """Remove paths only after the application confirms they reached Trash."""
        self._remove_deleted_paths_from_state(paths)

    def _deletion_display_name(self, path: str) -> str:
        name = self._display_name_for_source(path)
        return f"{name} (DELETED)" if self._is_marked_func(path) else name

    def _preview_deletion_display_name(self, path: str) -> str:
        name = os.path.basename(path)
        return f"{name} (DELETED)" if self._is_marked_func(path) else name

    def _apply_preview_deletion_style(self, path: str) -> None:
        color = "#FFB366" if self._is_marked_func(path) else ""
        style = f"color: {color};" if color else ""
        self.large_preview_name.setStyleSheet(style)
        self.preview_selection_label.setStyleSheet(style)

    def _apply_deletion_presentation(
        self, item: QTreeWidgetItem, path: str, *, editable: bool
    ) -> None:
        was_ignoring = self._ignore_preview_item_change
        self._ignore_preview_item_change = True
        try:
            if editable:
                item.setData(
                    0,
                    Qt.ItemDataRole.EditRole,
                    self._display_name_for_source(path),
                )
            item.setText(0, self._deletion_display_name(path))
            item.setForeground(
                0,
                QBrush(
                    QColor("#FFB366")
                    if self._is_marked_func(path)
                    else QApplication.palette().text().color()
                ),
            )
        finally:
            self._ignore_preview_item_change = was_ignoring

    def set_source_folder(self, folder_path: str | None) -> None:
        self._folder_validation_request_id += 1
        self._source_root = folder_path
        has_folder = bool(folder_path)
        if has_folder:
            parts = (folder_path or "").replace("\\", "/").split("/")
            display = ("…/" + "/".join(parts[-3:])) if len(parts) > 4 else folder_path
            self.folder_path_label.setText(display)
            self.folder_path_label.setToolTip(folder_path)
            self.folder_button.setText("📁  Change")
            self.stacked.setCurrentIndex(1)
        else:
            self.folder_path_label.setText("No folder selected")
            self.folder_path_label.setToolTip("")
            self.folder_button.setText("📁  Select Folder")
            self.stacked.setCurrentIndex(0)

        for btn in self._mode_buttons.values():
            btn.setEnabled(has_folder)
        self.skip_button.setEnabled(has_folder)
        self.primary_button.setEnabled(has_folder and self._current_plan is not None)
        if not has_folder:
            self._current_preview_source_path = None
            self.before_tree.clear()
            self.preview_tree.clear()
            self._current_plan = None
            self._current_output_root = ""
            self._editable_groups = []
            self._editable_unassigned = []
            self._editable_skipped = []
            self._file_name_overrides = {}
            self._original_group_labels_by_path = {}
            self._original_group_labels_by_group_id = {}
            self._sticky_empty_group_ids.clear()
            self.stats_label.setVisible(False)
            self.loading_label.setText("Select a folder to start.")
            self.loading_bar.setVisible(False)
            self._clear_selected_preview()

    def has_source_folder(self) -> bool:
        return self.folder_path_label.text() != "No folder selected"

    def set_preview_plan(self, plan, output_root: str | None = None) -> None:
        start_time = time.perf_counter()
        plan_groups = len(getattr(plan, "groups", []) or [])
        logger.info(
            "Organize set_preview_plan start: groups=%d total_items=%s supported_items=%s source_root=%s",
            plan_groups,
            getattr(plan, "total_items", 0),
            getattr(plan, "supported_items", 0),
            self._source_root,
        )
        self._current_plan = plan
        self._total_items = int(getattr(plan, "total_items", 0))
        self._supported_items = int(getattr(plan, "supported_items", 0))
        self._current_output_root = (
            output_root or getattr(plan, "output_root", "") or (self._source_root or "")
        )
        inventory_start = time.perf_counter()
        augment_grouping_plan_with_filesystem_paths(plan, self._source_root)
        logger.info(
            "Organize set_preview_plan: prepared inventory consumed in %.3fs",
            time.perf_counter() - inventory_start,
        )
        self._editable_groups = [
            GroupingGroup(
                group_id=str(group.group_id),
                group_label=str(group.group_label),
                source_paths=list(group.source_paths),
                destination_folder=getattr(group, "destination_folder", ""),
                skipped_paths=list(getattr(group, "skipped_paths", [])),
            )
            for group in getattr(plan, "groups", [])
        ]
        self._editable_unassigned = list(getattr(plan, "unassigned_paths", []))
        self._editable_skipped = list(getattr(plan, "skipped_paths", []))
        self._file_name_overrides = dict(getattr(plan, "file_name_overrides", {}) or {})
        self._sticky_empty_group_ids.clear()
        self._group_id_counter = self._compute_group_id_counter()
        self._original_group_labels_by_path = {}
        self._original_group_labels_by_group_id = {}
        for group in self._editable_groups:
            self._original_group_labels_by_group_id[str(group.group_id)] = str(
                group.group_label
            )
            for source_path in group.source_paths:
                self._original_group_labels_by_path[source_path] = group.group_label
        refresh_start = time.perf_counter()
        self._refresh_preview_trees(preserve_selection=False)
        logger.info(
            "Organize set_preview_plan complete in %.3fs (tree refresh %.3fs, editable_groups=%d)",
            time.perf_counter() - start_time,
            time.perf_counter() - refresh_start,
            len(self._editable_groups),
        )

    def get_group_name_overrides(self) -> dict[str, str]:
        return {
            str(group.group_id): group.group_label
            for group in self._editable_groups
            if group.group_id
        }

    def get_effective_plan(self) -> GroupingPlan:
        effective_groups: list[GroupingGroup] = []
        for group in self._editable_groups:
            if not group.source_paths:
                continue
            effective_groups.append(
                GroupingGroup(
                    group_id=str(group.group_id),
                    group_label=str(group.group_label),
                    source_paths=list(group.source_paths),
                )
            )
        return GroupingPlan(
            mode=self.current_mode(),
            total_items=self._total_items,
            supported_items=self._supported_items,
            groups=effective_groups,
            unassigned_paths=list(self._editable_unassigned),
            skipped_paths=list(self._editable_skipped),
            output_root=self._current_output_root,
            file_name_overrides=dict(self._file_name_overrides),
            deleted_paths=[],
            filesystem_inventory_complete=True,
            source_root=self._source_root or "",
        )

    def has_unsaved_grouping_edits(self) -> bool:
        if self._current_plan is None:
            return False
        effective_plan = self.get_effective_plan()
        return self._plan_signature(effective_plan) != self._plan_signature(
            self._current_plan
        )

    def pending_grouping_action_lines(self) -> list[str]:
        return self._build_action_lines(self.get_effective_plan())

    def set_loading_state(
        self, message: str, busy: bool, progress: int | None = None
    ) -> None:
        self.loading_label.setText(message)
        self.loading_bar.setVisible(busy)
        if not busy:
            self.loading_bar.setRange(0, 1)
            self.loading_bar.setValue(0)
            return
        if progress is None or progress < 0:
            self.loading_bar.setRange(0, 0)
        else:
            self.loading_bar.setRange(0, 100)
            self.loading_bar.setValue(max(0, min(100, progress)))

    def _compute_group_id_counter(self) -> int:
        counter = 0
        for group in self._editable_groups:
            try:
                counter = max(counter, int(group.group_id))
            except Exception:
                counter += 1
        return counter

    def _next_group_id(self) -> str:
        self._group_id_counter += 1
        return str(self._group_id_counter)

    def _plan_signature(self, plan: GroupingPlan) -> tuple:
        groups = tuple(
            sorted(
                (
                    str(group.group_id),
                    str(group.group_label),
                    tuple(sorted(group.source_paths)),
                )
                for group in plan.groups
            )
        )
        return (
            groups,
            tuple(sorted(plan.unassigned_paths)),
            tuple(sorted(plan.skipped_paths)),
            tuple(sorted(getattr(plan, "deleted_paths", []) or [])),
            tuple(
                sorted(
                    (str(path), str(name))
                    for path, name in plan.file_name_overrides.items()
                )
            ),
        )

    def _refresh_preview_trees(self, preserve_selection: bool = True) -> None:
        start_time = time.perf_counter()
        logger.info(
            "Organize tree refresh start: preserve_selection=%s editable_groups=%d all_source_paths=%d",
            preserve_selection,
            len(self._editable_groups),
            len(self._all_source_paths()),
        )
        selection_state = (
            self._capture_selection_state() if preserve_selection else None
        )
        self._update_stats()
        self.before_tree.setUpdatesEnabled(False)
        self.preview_tree.setUpdatesEnabled(False)
        try:
            before_start = time.perf_counter()
            self._render_before_tree()
            before_duration = time.perf_counter() - before_start
            after_start = time.perf_counter()
            self._render_after_tree(selection_state)
            after_duration = time.perf_counter() - after_start
        finally:
            self.before_tree.setUpdatesEnabled(True)
            self.preview_tree.setUpdatesEnabled(True)
            self.before_tree.viewport().update()
            self.preview_tree.viewport().update()
        logger.info(
            "Organize tree refresh complete in %.3fs (before=%.3fs after=%.3fs)",
            time.perf_counter() - start_time,
            before_duration,
            after_duration,
        )

    def _update_stats(self) -> None:
        group_count = len(self._editable_groups)
        group_label = "folder" if group_count == 1 else "folders"
        removal_count = sum(
            1 for path in self._all_source_paths() if self._is_marked_func(path)
        )
        removal_label = "mark" if removal_count == 1 else "marks"
        self.stats_label.setText(
            f"{group_count} {group_label}  ·  {removal_count} Trash {removal_label}"
        )
        self.stats_label.setVisible(True)
        self.loading_label.clear()
        self.loading_bar.setVisible(False)
        self.primary_button.setEnabled(
            self.has_source_folder() and self._current_plan is not None
        )

    def _capture_selection_state(self) -> dict[str, object]:
        selected_items = self.preview_tree.selectedItems()
        selected_paths = [
            self._item_source_path(item)
            for item in selected_items
            if self._item_kind(item) == ITEM_FILE and self._item_source_path(item)
        ]
        selected_group_ids = [
            self._item_group_id(item)
            for item in selected_items
            if self._item_kind(item) == ITEM_GROUP and self._item_group_id(item)
        ]
        selected_match_relative_paths = [
            self._item_match_relative_path(item)
            for item in selected_items
            if self._item_kind(item) in {ITEM_GROUP, ITEM_DIRECTORY}
            and self._item_match_relative_path(item)
        ]
        current_item = self.preview_tree.currentItem()
        return {
            "selected_paths": [path for path in selected_paths if path],
            "selected_group_ids": [
                group_id for group_id in selected_group_ids if group_id
            ],
            "selected_match_relative_paths": [
                path for path in selected_match_relative_paths if path
            ],
            "current_path": self._item_source_path(current_item),
            "current_group_id": self._item_group_id(current_item),
            "current_match_relative_path": self._item_match_relative_path(current_item),
        }

    def _restore_selection_state(self, state: dict[str, object] | None) -> None:
        start_time = time.perf_counter()
        if not state:
            self.preview_tree.setCurrentItem(None)
            self._clear_selected_preview()
            logger.info(
                "Organize selection restore: no prior state, cleared preview in %.3fs",
                time.perf_counter() - start_time,
            )
            return

        self.preview_tree.blockSignals(True)
        self.preview_tree.clearSelection()

        for path in state.get("selected_paths", []):
            item = self._after_file_items_by_path.get(str(path))
            if item:
                item.setSelected(True)

        current_path = state.get("current_path")
        current_group_id = state.get("current_group_id")
        current_match_relative_path = state.get("current_match_relative_path")
        target_item = None
        if current_path:
            target_item = self._after_file_items_by_path.get(str(current_path))
        if target_item is None and current_group_id:
            target_item = self._after_group_items_by_id.get(str(current_group_id))
        if target_item is None and current_match_relative_path:
            target_item = self._after_items_by_match_relative_path.get(
                str(current_match_relative_path)
            )
        if target_item is None:
            selected_paths = state.get("selected_paths", [])
            if selected_paths:
                target_item = self._after_file_items_by_path.get(str(selected_paths[0]))
        if target_item is None:
            selected_group_ids = state.get("selected_group_ids", [])
            if selected_group_ids:
                target_item = self._after_group_items_by_id.get(
                    str(selected_group_ids[0])
                )
        if target_item is None:
            selected_match_relative_paths = state.get(
                "selected_match_relative_paths", []
            )
            if selected_match_relative_paths:
                target_item = self._after_items_by_match_relative_path.get(
                    str(selected_match_relative_paths[0])
                )
        self.preview_tree.setCurrentItem(target_item)
        self.preview_tree.blockSignals(False)
        current_path = self._item_source_path(target_item)
        if current_path:
            self._update_selected_preview(current_path)
        elif target_item is not None:
            self._update_folder_preview(target_item)
        else:
            self._clear_selected_preview()
        logger.info(
            "Organize selection restore complete in %.3fs (target_kind=%s target_path=%s)",
            time.perf_counter() - start_time,
            self._item_kind(target_item),
            current_path,
        )

    def _render_after_tree(self, selection_state: dict[str, object] | None) -> None:
        start_time = time.perf_counter()
        self._ignore_preview_item_change = True
        self.preview_tree.clear()
        self._after_root_item = None
        self._after_file_items_by_path = {}
        self._after_dir_items_by_relative_path = {}
        self._after_group_items_by_id = {}
        self._after_group_items_by_label = {}
        self._after_items_by_match_relative_path = {}

        root_display = (
            self._current_output_root or self._source_root or "Selected folder"
        )
        root_name = os.path.basename(os.path.normpath(root_display)) or root_display
        root_item = QTreeWidgetItem([root_name])
        root_item.setIcon(0, self._folder_icon)
        self._set_item_metadata(
            root_item,
            kind=ITEM_ROOT,
            relative_path="",
            projected_path=self._current_output_root,
            actual_path=self._source_root or self._current_output_root,
            match_relative_path="",
        )
        root_item.setFlags(root_item.flags() & ~Qt.ItemFlag.ItemIsEditable)
        self.preview_tree.addTopLevelItem(root_item)
        self._after_root_item = root_item
        self._after_dir_items_by_relative_path[""] = root_item

        directory_items: dict[str, QTreeWidgetItem] = {"": root_item}

        def ensure_directory(relative_path: str) -> QTreeWidgetItem:
            normalized = self._normalize_relative_path(relative_path)
            if normalized in directory_items:
                return directory_items[normalized]
            parent_rel = os.path.dirname(normalized)
            if parent_rel == ".":
                parent_rel = ""
            parent_item = ensure_directory(parent_rel)
            item = QTreeWidgetItem([os.path.basename(normalized)])
            item.setIcon(0, self._folder_icon)
            match_relative_path = self._match_relative_path_for_after_directory(
                normalized
            )
            actual_path = self._filesystem_path_for_relative(
                match_relative_path or normalized
            )
            self._set_item_metadata(
                item,
                kind=ITEM_DIRECTORY,
                relative_path=normalized,
                projected_path=self._projected_directory_path(normalized),
                actual_path=actual_path,
                parent_relative_path=parent_rel,
                match_relative_path=match_relative_path,
            )
            item.setFlags(item.flags() & ~Qt.ItemFlag.ItemIsEditable)
            parent_item.addChild(item)
            directory_items[normalized] = item
            self._after_dir_items_by_relative_path[normalized] = item
            match_relative_path = self._item_match_relative_path(item)
            if match_relative_path:
                self._after_items_by_match_relative_path.setdefault(
                    match_relative_path, item
                )
            return item

        for group in self._editable_groups:
            normalized_label = self._normalize_relative_path(group.group_label)
            if normalized_label:
                parent_rel = os.path.dirname(normalized_label)
                if parent_rel == ".":
                    parent_rel = ""
                leaf_name = os.path.basename(normalized_label)
            else:
                parent_rel = ""
                leaf_name = ROOT_LEVEL_GROUP_LABEL
            # Reuse an existing directory node if one was already created for this label
            # (e.g. as a parent for a cluster subgroup). Prevents duplicate date folders.
            if normalized_label in directory_items:
                group_item = directory_items[normalized_label]
                _reusing_existing = True
            else:
                parent_item = ensure_directory(parent_rel)
                group_item = QTreeWidgetItem([leaf_name])
                group_item.setIcon(0, self._folder_icon)
                _reusing_existing = False
            self._set_item_metadata(
                group_item,
                kind=ITEM_GROUP,
                group_id=str(group.group_id),
                relative_path=normalized_label,
                projected_path=self._projected_directory_path(normalized_label),
                actual_path=self._filesystem_path_for_relative(
                    self._match_relative_path_for_group(group)
                ),
                parent_relative_path=parent_rel,
                match_relative_path=self._match_relative_path_for_group(group),
            )
            group_item.setFlags(
                group_item.flags()
                | Qt.ItemFlag.ItemIsEditable
                | Qt.ItemFlag.ItemIsDragEnabled
            )
            group_item.setData(0, Qt.ItemDataRole.EditRole, leaf_name)
            group_item.setToolTip(
                0, f"Double-click to rename · {len(group.source_paths)} photo(s)"
            )
            if not _reusing_existing:
                parent_item.addChild(group_item)
                directory_items[normalized_label] = group_item
            self._after_group_items_by_id[str(group.group_id)] = group_item
            self._after_group_items_by_label[normalized_label] = group_item
            match_relative_path = self._item_match_relative_path(group_item)
            if match_relative_path:
                self._after_items_by_match_relative_path[match_relative_path] = (
                    group_item
                )
            for source_path in sorted(group.source_paths):
                file_item = QTreeWidgetItem(
                    [self._display_name_for_source(source_path)]
                )
                self._set_item_metadata(
                    file_item,
                    kind=ITEM_FILE,
                    source_path=source_path,
                    group_id=str(group.group_id),
                    relative_path=self._relative_path_for_source(source_path),
                    projected_path=self._projected_path_for_source(source_path),
                    actual_path=source_path,
                    match_relative_path=self._relative_path_for_source(source_path),
                )
                file_item.setFlags(
                    file_item.flags()
                    | Qt.ItemFlag.ItemIsEditable
                    | Qt.ItemFlag.ItemIsDragEnabled
                )
                file_item.setData(
                    0,
                    Qt.ItemDataRole.EditRole,
                    self._display_name_for_source(source_path),
                )
                self._set_tree_item_icon(file_item, source_path)
                group_item.addChild(file_item)
                self._after_file_items_by_path[source_path] = file_item
                self._apply_deletion_presentation(file_item, source_path, editable=True)

        self._add_bucket_item(
            root_item,
            ITEM_UNASSIGNED,
            "📁  Unassigned",
            self._editable_unassigned,
            os.path.join(self._current_output_root, "Unassigned")
            if self._current_output_root
            else "",
        )
        self._add_bucket_item(
            root_item,
            ITEM_SKIPPED,
            "📁  Skipped",
            self._editable_skipped,
            self._source_root or self._current_output_root,
        )

        self.preview_tree.expandAll()
        self._ignore_preview_item_change = False
        self._restore_selection_state(selection_state)
        logger.info(
            "Organize after tree built in %.3fs (groups=%d files=%d dirs=%d buckets_unassigned=%d buckets_skipped=%d)",
            time.perf_counter() - start_time,
            len(self._after_group_items_by_id),
            len(self._after_file_items_by_path),
            len(self._after_dir_items_by_relative_path),
            len(self._editable_unassigned),
            len(self._editable_skipped),
        )

    def _add_bucket_item(
        self,
        root_item: QTreeWidgetItem,
        bucket_kind: str,
        label: str,
        paths: list[str],
        projected_path: str,
    ) -> None:
        if not paths:
            return
        bucket_item = QTreeWidgetItem(
            [label.replace("📁  ", "").replace("📁 ", "").replace("📁", "").strip()]
        )
        bucket_item.setIcon(0, self._folder_icon)
        self._set_item_metadata(
            bucket_item,
            kind=bucket_kind,
            bucket=bucket_kind,
            projected_path=projected_path,
            actual_path=self._source_root,
            match_relative_path="",
        )
        bucket_item.setFlags(bucket_item.flags() & ~Qt.ItemFlag.ItemIsEditable)
        root_item.addChild(bucket_item)
        for source_path in sorted(paths):
            file_item = QTreeWidgetItem([self._display_name_for_source(source_path)])
            self._set_item_metadata(
                file_item,
                kind=ITEM_FILE,
                source_path=source_path,
                bucket=bucket_kind,
                relative_path=self._relative_path_for_source(source_path),
                projected_path=self._projected_path_for_source(source_path),
                actual_path=source_path,
                match_relative_path=self._relative_path_for_source(source_path),
            )
            item_flags = file_item.flags() | Qt.ItemFlag.ItemIsEditable
            if bucket_kind != ITEM_SKIPPED:
                item_flags |= Qt.ItemFlag.ItemIsDragEnabled
            file_item.setFlags(item_flags)
            file_item.setData(
                0,
                Qt.ItemDataRole.EditRole,
                self._display_name_for_source(source_path),
            )
            self._set_tree_item_icon(file_item, source_path)
            bucket_item.addChild(file_item)
            self._after_file_items_by_path[source_path] = file_item
            self._apply_deletion_presentation(file_item, source_path, editable=True)

    def _render_before_tree(self) -> None:
        start_time = time.perf_counter()
        self.before_tree.clear()
        self._before_root_item = None
        self._before_file_items_by_path = {}
        self._before_dir_items_by_relative_path = {}

        tracked_paths = self._all_source_paths()
        root_path = self._source_root or self._common_root_for_paths(tracked_paths)
        # The preview plan already supplies a complete inventory. Rewalking
        # here used to block the UI a second time for every folder.
        all_paths = tracked_paths
        logger.info(
            "Organize before tree input: tracked_paths=%d merged_paths=%d root=%s",
            len(tracked_paths),
            len(all_paths),
            root_path,
        )
        if not all_paths:
            return

        root_name = (
            os.path.basename(os.path.normpath(root_path)) or root_path or "Source"
        )
        root_item = QTreeWidgetItem([root_name])
        root_item.setIcon(0, self._folder_icon)
        self._set_item_metadata(
            root_item,
            kind=ITEM_ROOT,
            relative_path="",
            projected_path=self._current_output_root,
            actual_path=root_path,
            match_relative_path="",
        )
        root_item.setFlags(root_item.flags() & ~Qt.ItemFlag.ItemIsEditable)
        self.before_tree.addTopLevelItem(root_item)
        self._before_root_item = root_item
        self._before_dir_items_by_relative_path[""] = root_item

        dir_items: dict[str, QTreeWidgetItem] = {"": root_item}

        def ensure_dir(relative_dir: str) -> QTreeWidgetItem:
            normalized = self._normalize_relative_path(relative_dir)
            if normalized in dir_items:
                return dir_items[normalized]
            parent_rel = os.path.dirname(normalized)
            if parent_rel == ".":
                parent_rel = ""
            parent_item = ensure_dir(parent_rel)
            item = QTreeWidgetItem([os.path.basename(normalized)])
            item.setIcon(0, self._folder_icon)
            actual_path = (
                os.path.join(root_path, normalized)
                if root_path and normalized
                else root_path
            )
            self._set_item_metadata(
                item,
                kind=ITEM_DIRECTORY,
                relative_path=normalized,
                projected_path=self._projected_directory_path(normalized),
                actual_path=actual_path,
                parent_relative_path=parent_rel,
                match_relative_path=normalized,
            )
            item.setFlags(item.flags() & ~Qt.ItemFlag.ItemIsEditable)
            parent_item.addChild(item)
            dir_items[normalized] = item
            self._before_dir_items_by_relative_path[normalized] = item
            return item

        for source_path in sorted(all_paths):
            rel_file = self._relative_path_for_source(source_path)
            rel_dir = os.path.dirname(rel_file)
            if rel_dir == ".":
                rel_dir = ""
            parent_item = ensure_dir(rel_dir)
            file_item = QTreeWidgetItem([os.path.basename(source_path)])
            self._set_item_metadata(
                file_item,
                kind=ITEM_FILE,
                source_path=source_path,
                relative_path=rel_file,
                projected_path=self._projected_path_for_source(source_path),
                actual_path=source_path,
                group_id=self._group_id_for_path(source_path),
                bucket=self._bucket_for_path(source_path),
                match_relative_path=rel_file,
            )
            file_item.setFlags(file_item.flags() & ~Qt.ItemFlag.ItemIsEditable)
            self._set_tree_item_icon(file_item, source_path)
            parent_item.addChild(file_item)
            self._before_file_items_by_path[source_path] = file_item
            self._apply_deletion_presentation(file_item, source_path, editable=False)

        self.before_tree.expandAll()
        logger.info(
            "Organize before tree built in %.3fs (files=%d dirs=%d)",
            time.perf_counter() - start_time,
            len(self._before_file_items_by_path),
            len(self._before_dir_items_by_relative_path),
        )

    def _handle_preview_item_changed(self, item: QTreeWidgetItem, _column: int) -> None:
        if self._ignore_preview_item_change or self._drag_in_progress:
            return
        try:
            kind = self._item_kind(item)
        except RuntimeError:
            return  # item was deleted by a prior tree refresh triggered by the same setText
        if kind == ITEM_GROUP:
            group = self._find_group(self._item_group_id(item))
            if group is None:
                return
            parent_rel = self._item_parent_relative_path(item)
            new_leaf_name = self._normalize_leaf_name(item.text(0))
            group.group_label = (
                os.path.join(parent_rel, new_leaf_name) if parent_rel else new_leaf_name
            )
            self._prune_empty_groups()
            self._refresh_preview_trees()
            return
        if kind == ITEM_FILE:
            source_path = self._item_source_path(item)
            if not source_path:
                return
            edited_text = item.text(0)
            if self._is_marked_func(source_path) and edited_text.endswith(" (DELETED)"):
                edited_text = edited_text[: -len(" (DELETED)")]
            new_filename = self._normalize_filename(edited_text, source_path)
            if new_filename == os.path.basename(source_path):
                self._file_name_overrides.pop(source_path, None)
            else:
                self._file_name_overrides[source_path] = new_filename
            self._refresh_preview_trees()

    def _set_item_metadata(
        self,
        item: QTreeWidgetItem,
        *,
        kind: str,
        source_path: str | None = None,
        projected_path: str | None = None,
        relative_path: str | None = None,
        group_id: str | None = None,
        actual_path: str | None = None,
        bucket: str | None = None,
        parent_relative_path: str | None = None,
        match_relative_path: str | None = None,
    ) -> None:
        item.setData(0, ROLE_KIND, kind)
        item.setData(0, ROLE_SOURCE_PATH, source_path)
        item.setData(0, ROLE_PROJECTED_PATH, projected_path)
        item.setData(0, ROLE_RELATIVE_PATH, relative_path)
        item.setData(0, ROLE_GROUP_ID, group_id)
        item.setData(0, ROLE_ACTUAL_PATH, actual_path)
        item.setData(0, ROLE_BUCKET, bucket)
        item.setData(0, ROLE_PARENT_RELATIVE_PATH, parent_relative_path)
        item.setData(0, ROLE_MATCH_RELATIVE_PATH, match_relative_path)

    def _item_kind(self, item: QTreeWidgetItem | None) -> str | None:
        return item.data(0, ROLE_KIND) if item is not None else None

    def _item_source_path(self, item: QTreeWidgetItem | None) -> str | None:
        return item.data(0, ROLE_SOURCE_PATH) if item is not None else None

    def _item_projected_path(self, item: QTreeWidgetItem | None) -> str | None:
        return item.data(0, ROLE_PROJECTED_PATH) if item is not None else None

    def _item_relative_path(self, item: QTreeWidgetItem | None) -> str:
        return item.data(0, ROLE_RELATIVE_PATH) or "" if item is not None else ""

    def _item_group_id(self, item: QTreeWidgetItem | None) -> str | None:
        return item.data(0, ROLE_GROUP_ID) if item is not None else None

    def _item_actual_path(self, item: QTreeWidgetItem | None) -> str | None:
        return item.data(0, ROLE_ACTUAL_PATH) if item is not None else None

    def _item_bucket(self, item: QTreeWidgetItem | None) -> str | None:
        return item.data(0, ROLE_BUCKET) if item is not None else None

    def _item_parent_relative_path(self, item: QTreeWidgetItem | None) -> str:
        return item.data(0, ROLE_PARENT_RELATIVE_PATH) or "" if item is not None else ""

    def _item_match_relative_path(self, item: QTreeWidgetItem | None) -> str:
        return item.data(0, ROLE_MATCH_RELATIVE_PATH) or "" if item is not None else ""

    def _find_group(self, group_id: str | None) -> GroupingGroup | None:
        if group_id is None:
            return None
        for group in self._editable_groups:
            if str(group.group_id) == str(group_id):
                return group
        return None

    def _normalize_relative_path(self, value: str) -> str:
        text = (value or "").strip().replace("\\", os.sep).replace("/", os.sep)
        parts = [part.strip() for part in text.split(os.sep) if part.strip()]
        return os.path.join(*parts) if parts else ""

    def _normalize_leaf_name(self, value: str) -> str:
        text = (value or "").strip()
        for prefix in ("📁  ", "📁 ", "📁"):
            if text.startswith(prefix):
                text = text[len(prefix) :].strip()
                break
        normalized = self._normalize_relative_path(text)
        leaf = os.path.basename(normalized) if normalized else ""
        return leaf or "Unnamed"

    def _normalize_filename(self, value: str, source_path: str) -> str:
        text = (value or "").strip().replace("\\", "_").replace("/", "_")
        text = os.path.basename(text).strip().strip(".")
        return text or os.path.basename(source_path)

    def _display_name_for_source(self, source_path: str) -> str:
        override = self._file_name_overrides.get(source_path, "").strip()
        if override:
            return override
        return os.path.basename(source_path)

    def _match_relative_path_for_group(self, group: GroupingGroup) -> str:
        original = self._original_group_labels_by_group_id.get(
            str(group.group_id), ""
        ).strip()
        if original:
            return self._normalize_relative_path(original)
        return self._common_relative_directory_for_paths(group.source_paths)

    def _match_relative_path_for_after_directory(self, relative_path: str) -> str:
        for group in self._editable_groups:
            normalized_label = self._normalize_relative_path(group.group_label)
            if normalized_label == relative_path:
                return self._match_relative_path_for_group(group)
        return ""

    def _relative_path_for_source(self, source_path: str) -> str:
        source_root = self._source_root or ""
        if source_root:
            try:
                return os.path.relpath(source_path, source_root)
            except Exception:
                pass
        return source_path

    def _current_relative_directory_for_source(self, source_path: str) -> str:
        rel_path = self._relative_path_for_source(source_path)
        rel_dir = os.path.dirname(rel_path)
        if rel_dir in {"", "."}:
            return ""
        return self._normalize_relative_path(rel_dir)

    def _relative_dir_label_for_source(self, source_path: str) -> str:
        rel_path = self._relative_path_for_source(source_path)
        rel_dir = os.path.dirname(rel_path)
        if rel_dir in {"", "."}:
            source_root = self._source_root or ""
            return os.path.basename(os.path.normpath(source_root)) or "Root"
        return self._normalize_relative_path(rel_dir)

    def _all_source_paths(self) -> list[str]:
        all_paths: list[str] = []
        for group in self._editable_groups:
            all_paths.extend(group.source_paths)
        all_paths.extend(self._editable_unassigned)
        all_paths.extend(self._editable_skipped)
        return list(dict.fromkeys(all_paths))

    def _common_relative_directory_for_paths(self, paths: list[str]) -> str:
        relative_dirs: list[str] = []
        for source_path in paths:
            relative_path = self._relative_path_for_source(source_path)
            relative_dir = os.path.dirname(relative_path)
            if relative_dir in {"", "."}:
                relative_dirs.append("")
            else:
                relative_dirs.append(self._normalize_relative_path(relative_dir))
        if not relative_dirs:
            return ""
        try:
            return self._normalize_relative_path(os.path.commonpath(relative_dirs))
        except Exception:
            return relative_dirs[0]

    def _common_root_for_paths(self, paths: list[str]) -> str:
        try:
            if len(paths) > 1:
                return os.path.commonpath(paths)
            return os.path.dirname(paths[0])
        except Exception:
            return os.path.dirname(paths[0]) if paths else ""

    def _projected_directory_path(self, relative_path: str) -> str:
        output_root = self._current_output_root or self._source_root or ""
        normalized = self._normalize_relative_path(relative_path)
        return os.path.join(output_root, normalized) if normalized else output_root

    def _group_id_for_path(self, source_path: str) -> str | None:
        for group in self._editable_groups:
            if source_path in group.source_paths:
                return str(group.group_id)
        return None

    def _bucket_for_path(self, source_path: str) -> str | None:
        if source_path in self._editable_unassigned:
            return ITEM_UNASSIGNED
        if source_path in self._editable_skipped:
            return ITEM_SKIPPED
        return None

    def _projected_path_for_source(self, source_path: str) -> str:
        filename = self._display_name_for_source(source_path)
        for group in self._editable_groups:
            if source_path in group.source_paths:
                return os.path.join(
                    self._projected_directory_path(group.group_label),
                    filename,
                )
        if source_path in self._editable_unassigned:
            return os.path.join(
                self._current_output_root or self._source_root or "",
                "Unassigned",
                filename,
            )
        return source_path

    def _filesystem_path_for_relative(self, relative_path: str) -> str:
        source_root = self._source_root or ""
        normalized = self._normalize_relative_path(relative_path)
        if source_root and normalized:
            return os.path.join(source_root, normalized)
        return source_root

    def _set_tree_item_icon(self, item: QTreeWidgetItem, source_path: str) -> None:
        # This is an O(1) lookup in the shared UI-icon cache; it never reads or
        # decodes image data while the tree is being constructed.
        icon = None
        if self._parent_window is not None:
            get_icon = getattr(self._parent_window, "get_cached_thumbnail_icon", None)
            if callable(get_icon):
                icon = get_icon(source_path)
        item.setIcon(0, icon or self._file_icon)

    def visible_thumbnail_paths(
        self,
        limit: int = THUMBNAIL_PRELOAD_BATCH_SIZE
        + (THUMBNAIL_PRELOAD_VISIBLE_MARGIN * 2),
    ) -> list[str]:
        """Return file paths in and around both visible Organize tree viewports."""

        def paths_for_tree(tree: QTreeWidget) -> list[str]:
            item = tree.itemAt(QPoint(1, 1))
            if item is None and tree.topLevelItemCount():
                item = tree.topLevelItem(0)
            for _ in range(THUMBNAIL_PRELOAD_VISIBLE_MARGIN):
                if item is None:
                    break
                previous = tree.itemAbove(item)
                if previous is None:
                    break
                item = previous

            paths: list[str] = []
            inspected = 0
            # Folder rows do not consume the file-path budget.
            max_inspected = max(limit * 4, limit)
            while item is not None and len(paths) < limit and inspected < max_inspected:
                source_path = self._item_source_path(item)
                if source_path:
                    paths.append(source_path)
                item = tree.itemBelow(item)
                inspected += 1
            return paths

        def paths_for_folder_preview() -> list[str]:
            if self.preview_pane_stack.currentIndex() != PREVIEW_PAGE_FOLDER:
                return []
            grid = self.folder_preview_grid
            viewport_rect = grid.viewport().rect()
            margin = max(0, viewport_rect.height())
            preload_rect = viewport_rect.adjusted(0, -margin, 0, margin)
            visible_paths: list[str] = []
            fallback_paths: list[str] = []
            for index in range(grid.count()):
                item = grid.item(index)
                if item is None:
                    continue
                source_path = item.data(Qt.ItemDataRole.UserRole)
                if not source_path:
                    continue
                source_path = str(source_path)
                if len(fallback_paths) < limit:
                    fallback_paths.append(source_path)
                if grid.visualItemRect(item).intersects(preload_rect):
                    visible_paths.append(source_path)
                    if len(visible_paths) >= limit:
                        break
            # A newly populated/offscreen widget may not have valid visual rects
            # until the next layout pass; its first page is still the right work.
            return visible_paths or fallback_paths

        combined = (
            paths_for_folder_preview()
            + paths_for_tree(self.before_tree)
            + paths_for_tree(self.preview_tree)
        )
        return list(dict.fromkeys(combined))[:limit]

    def _set_preview_icon(self, item: QTreeWidgetItem, source_path: str) -> None:
        if self._parent_window is not None:
            get_icon = getattr(self._parent_window, "get_cached_thumbnail_icon", None)
            if callable(get_icon):
                icon = get_icon(source_path)
                if icon is not None:
                    was_ignoring = self._ignore_preview_item_change
                    self._ignore_preview_item_change = True
                    try:
                        item.setIcon(0, icon)
                    finally:
                        self._ignore_preview_item_change = was_ignoring
                    return
        image_pipeline = getattr(self._parent_window, "image_pipeline", None)
        if image_pipeline is None:
            return
        try:
            pixmap = image_pipeline.get_cached_thumbnail_qpixmap(
                source_path,
                memory_only=True,
            )
            if pixmap is not None:
                # QTreeWidget emits itemChanged for presentation changes too.
                # The same signal handles user renames, so suppress only while
                # applying a cached icon to avoid rebuilding the complete plan.
                was_ignoring = self._ignore_preview_item_change
                self._ignore_preview_item_change = True
                try:
                    item.setIcon(0, QIcon(pixmap))
                finally:
                    self._ignore_preview_item_change = was_ignoring
        except Exception:
            return

    def _handle_after_item_changed(
        self, current: QTreeWidgetItem | None, _prev: QTreeWidgetItem | None
    ) -> None:
        source_path = self._item_source_path(current)
        if source_path:
            self._update_selected_preview(source_path)
            self._sync_selection_to_other_tree(current, from_after=True)
        elif current is not None:
            self._update_folder_preview(current)
            self._sync_selection_to_other_tree(current, from_after=True)
        else:
            self._clear_selected_preview()

    def _handle_before_item_changed(
        self, current: QTreeWidgetItem | None, _prev: QTreeWidgetItem | None
    ) -> None:
        source_path = self._item_source_path(current)
        if source_path:
            self._update_selected_preview(source_path)
            self._sync_selection_to_other_tree(current, from_after=False)
        elif current is not None:
            self._update_folder_preview(current)
            self._sync_selection_to_other_tree(current, from_after=False)
        else:
            self._clear_selected_preview()
            self._sync_selection_to_other_tree(current, from_after=False)

    def _handle_tree_double_click(self, item: QTreeWidgetItem, column: int) -> None:
        if (
            self._item_kind(item) in {ITEM_GROUP, ITEM_FILE}
            and item.flags() & Qt.ItemFlag.ItemIsEditable
        ):
            self.preview_tree.editItem(item, column)

    def _clear_selected_preview(self) -> None:
        self._current_preview_source_path = None
        self.large_preview_view.clear()
        self.large_preview_name.clear()
        self.folder_preview_title.clear()
        self.folder_preview_meta.clear()
        self.folder_preview_grid.clear()
        self.preview_pane_stack.setCurrentIndex(PREVIEW_PAGE_HINT)
        self.preview_trash_button.setEnabled(False)
        self.large_preview_name.setStyleSheet("")
        self.preview_selection_label.setStyleSheet("")
        self.preview_selection_label.setVisible(False)
        self.preview_selection_meta.setVisible(False)
        self.thumb_label.clear()
        self.thumb_label.setVisible(False)

    def _update_selected_preview(self, source_path: str) -> None:
        start_time = time.perf_counter()
        current_preview_path = self._current_preview_source_path
        self._current_preview_source_path = source_path
        existing_pixmap = self.large_preview_view.current_pixmap()
        if (
            current_preview_path == source_path
            and existing_pixmap is not None
            and not existing_pixmap.isNull()
        ):
            display_name = self._preview_deletion_display_name(source_path)
            self.large_preview_name.setText(display_name)
            self.preview_pane_stack.setCurrentIndex(PREVIEW_PAGE_IMAGE)
            self.preview_trash_button.setEnabled(os.path.isfile(source_path))
            self._apply_preview_deletion_style(source_path)
            self.preview_selection_label.setText(display_name)
            self.preview_selection_label.setVisible(True)
            self.preview_selection_meta.setText(source_path)
            self.preview_selection_meta.setVisible(True)
            logger.debug(
                "Organize selected preview reused in %.3fs (path=%s)",
                time.perf_counter() - start_time,
                source_path,
            )
            return
        ext = os.path.splitext(source_path)[1].lower()
        image_pipeline = getattr(self._parent_window, "image_pipeline", None)
        pixmap: QPixmap | None = None
        preview_is_cached = False
        if image_pipeline and ext in SUPPORTED_MEDIA_EXTENSIONS:
            pixmap = image_pipeline.get_cached_preview_qpixmap(
                source_path,
                display_max_size=SELECTED_PREVIEW_DISPLAY_SIZE,
                memory_only=True,
            )
            preview_is_cached = bool(pixmap and not pixmap.isNull())
            if pixmap is None or pixmap.isNull():
                pixmap = image_pipeline.get_cached_thumbnail_qpixmap(
                    source_path,
                    memory_only=True,
                )
        if pixmap and not pixmap.isNull():
            self.large_preview_view.set_image(pixmap)
        else:
            self.large_preview_view.setText("Loading preview…")
        display_name = self._preview_deletion_display_name(source_path)
        self.large_preview_name.setText(display_name)
        self.preview_pane_stack.setCurrentIndex(PREVIEW_PAGE_IMAGE)
        self.preview_trash_button.setEnabled(os.path.isfile(source_path))
        self._apply_preview_deletion_style(source_path)
        self.preview_selection_label.setText(display_name)
        self.preview_selection_label.setVisible(True)
        self.preview_selection_meta.setText(source_path)
        self.preview_selection_meta.setVisible(True)
        if not preview_is_cached and self._parent_window is not None:
            request_preview = getattr(
                self._parent_window,
                "request_interactive_preview",
                None,
            )
            if callable(request_preview):
                request_preview(source_path)
        logger.debug(
            "Organize selected preview updated in %.3fs (path=%s cached_preview=%s pixmap=%s queued=%s)",
            time.perf_counter() - start_time,
            source_path,
            preview_is_cached,
            bool(pixmap and not pixmap.isNull()),
            not preview_is_cached,
        )

    def handle_preview_ready(self, source_path: str) -> None:
        """Upgrade the selected placeholder only if this result is still current."""
        if source_path != self._current_preview_source_path:
            return
        image_pipeline = getattr(self._parent_window, "image_pipeline", None)
        if image_pipeline is None:
            return
        pixmap = image_pipeline.get_cached_preview_qpixmap(
            source_path,
            display_max_size=SELECTED_PREVIEW_DISPLAY_SIZE,
            memory_only=True,
        )
        if pixmap is not None and not pixmap.isNull():
            self.large_preview_view.set_image(pixmap)

    def handle_preview_failed(self, source_path: str) -> None:
        if source_path != self._current_preview_source_path:
            return
        if not self.large_preview_view.has_image():
            self.large_preview_view.setText("Preview unavailable")

    def _update_folder_preview(self, item: QTreeWidgetItem) -> None:
        start_time = time.perf_counter()
        self._current_preview_source_path = None
        preview_paths = self._folder_preview_paths_for_item(item)
        if not preview_paths:
            self._clear_selected_preview()
            logger.info(
                "Organize folder preview cleared in %.3fs (no preview paths)",
                time.perf_counter() - start_time,
            )
            return

        total_preview_paths = len(preview_paths)
        visible_preview_paths = preview_paths[:MAX_FOLDER_PREVIEW_ITEMS]
        self.folder_preview_grid.clear()
        for source_path in visible_preview_paths:
            list_item = QListWidgetItem(
                self._preview_deletion_display_name(source_path)
            )
            list_item.setData(Qt.ItemDataRole.UserRole, source_path)
            list_item.setToolTip(self._relative_path_for_source(source_path))
            if self._is_marked_func(source_path):
                list_item.setForeground(QBrush(QColor("#FFB366")))
            icon = self._cached_thumbnail_icon_for_path(source_path)
            if icon is not None:
                list_item.setIcon(icon)
            else:
                list_item.setIcon(self._file_icon)
            self.folder_preview_grid.addItem(list_item)

        item_label = self._display_label_for_item(item)
        self.folder_preview_title.setText(item_label)
        if total_preview_paths > MAX_FOLDER_PREVIEW_ITEMS:
            meta_count = f"{total_preview_paths} item(s) · showing first {MAX_FOLDER_PREVIEW_ITEMS}"
        else:
            meta_count = f"{total_preview_paths} item(s)"
        self.folder_preview_meta.setText(
            f"{meta_count}\n{self._display_path_for_item(item)}"
        )
        self.preview_pane_stack.setCurrentIndex(PREVIEW_PAGE_FOLDER)
        self.preview_trash_button.setEnabled(False)
        self.preview_selection_label.setText(item_label)
        self.preview_selection_label.setVisible(True)
        self.preview_selection_meta.setText(meta_count)
        self.preview_selection_meta.setVisible(True)
        if self._parent_window is not None:
            schedule_thumbnails = getattr(
                self._parent_window,
                "schedule_visible_thumbnail_load",
                None,
            )
            if callable(schedule_thumbnails):
                schedule_thumbnails()
        logger.debug(
            "Organize folder preview updated in %.3fs (item=%s total_paths=%d visible_paths=%d)",
            time.perf_counter() - start_time,
            item_label,
            total_preview_paths,
            len(visible_preview_paths),
        )

    def _folder_preview_paths_for_item(self, item: QTreeWidgetItem | None) -> list[str]:
        return self._preview_paths_for_item(item)

    def _preview_paths_for_item(self, item: QTreeWidgetItem | None) -> list[str]:
        if item is None:
            return []
        source_path = self._item_source_path(item)
        if source_path:
            return [source_path]

        collected_paths: list[str] = []
        self._collect_descendant_source_paths(item, collected_paths)
        # When a GROUP has a sibling DIRECTORY with the same relative path (created when
        # another group was moved into it), include those files too.
        if self._item_kind(item) == ITEM_GROUP:
            relative_path = self._normalize_relative_path(
                self._item_relative_path(item)
            )
            if relative_path:
                dir_item = self._after_dir_items_by_relative_path.get(relative_path)
                if dir_item is not None and dir_item is not item:
                    self._collect_descendant_source_paths(dir_item, collected_paths)
        return list(dict.fromkeys(collected_paths))

    def _collect_descendant_source_paths(
        self, item: QTreeWidgetItem, collected_paths: list[str]
    ) -> None:
        source_path = self._item_source_path(item)
        if source_path:
            collected_paths.append(source_path)
            return
        for index in range(item.childCount()):
            self._collect_descendant_source_paths(item.child(index), collected_paths)

    def _cached_thumbnail_icon_for_path(self, source_path: str) -> QIcon | None:
        if self._parent_window is not None:
            get_icon = getattr(self._parent_window, "get_cached_thumbnail_icon", None)
            if callable(get_icon):
                icon = get_icon(source_path)
                if icon is not None:
                    return icon
        image_pipeline = getattr(self._parent_window, "image_pipeline", None)
        if image_pipeline is None:
            return None
        try:
            pixmap = image_pipeline.get_cached_thumbnail_qpixmap(
                source_path,
                memory_only=True,
            )
            if pixmap is not None and not pixmap.isNull():
                return QIcon(pixmap)
        except Exception:
            return None
        return None

    def refresh_cached_thumbnails(
        self, image_paths: Iterable[str] | None = None
    ) -> None:
        if image_paths is None:
            logger.debug(
                "Refreshing organize thumbnails for all tree items and folder grid"
            )
            self._refresh_all_tree_icons_from_cache()
            self._refresh_folder_preview_icons_from_cache()
            return

        completed_paths = {str(path) for path in image_paths}
        for source_path in completed_paths:
            before_item = self._before_file_items_by_path.get(source_path)
            if before_item is not None:
                self._set_preview_icon(before_item, source_path)
            after_item = self._after_file_items_by_path.get(source_path)
            if after_item is not None and after_item is not before_item:
                self._set_preview_icon(after_item, source_path)

        self._refresh_folder_preview_icons_from_cache(completed_paths)

    def refresh_cached_previews(self) -> None:
        logger.debug("Refreshing organize preview state from cache")
        self.refresh_cached_thumbnails()
        current_path = self._current_preview_source_path
        if not current_path:
            return
        image_pipeline = getattr(self._parent_window, "image_pipeline", None)
        if image_pipeline is None:
            return
        pixmap = image_pipeline.get_cached_preview_qpixmap(
            current_path,
            display_max_size=SELECTED_PREVIEW_DISPLAY_SIZE,
            memory_only=True,
        )
        if pixmap is None or pixmap.isNull():
            return
        self.large_preview_view.set_image(pixmap)
        self.preview_pane_stack.setCurrentIndex(PREVIEW_PAGE_IMAGE)

    def _refresh_all_tree_icons_from_cache(self) -> None:
        try:
            for tree in (self.before_tree, self.preview_tree):
                for i in range(tree.topLevelItemCount()):
                    top_item = tree.topLevelItem(i)
                    if top_item is not None:
                        self._recursive_refresh_icons(top_item)
        except RuntimeError:
            logger.debug(
                "Organize tree items deleted during thumbnail refresh; "
                "tree rebuild will populate icons from cache"
            )

    def _recursive_refresh_icons(self, item: QTreeWidgetItem) -> None:
        source_path = self._item_source_path(item)
        if self._item_kind(item) == ITEM_FILE and source_path:
            self._set_preview_icon(item, source_path)
        for i in range(item.childCount()):
            child = item.child(i)
            if child is not None:
                self._recursive_refresh_icons(child)

    def _refresh_folder_preview_icons_from_cache(
        self, image_paths: set[str] | None = None
    ) -> None:
        for index in range(self.folder_preview_grid.count()):
            item = self.folder_preview_grid.item(index)
            if item is None:
                continue
            source_path = item.data(Qt.ItemDataRole.UserRole)
            if not source_path:
                continue
            if image_paths is not None and str(source_path) not in image_paths:
                continue
            icon = self._cached_thumbnail_icon_for_path(str(source_path))
            if icon is not None:
                item.setIcon(icon)

    def _display_label_for_item(self, item: QTreeWidgetItem) -> str:
        text = item.text(0).strip()
        for prefix in ("📁  ", "📁 ", "📁"):
            if text.startswith(prefix):
                return text[len(prefix) :].strip() or "Folder"
        return text or "Folder"

    def _display_path_for_item(self, item: QTreeWidgetItem) -> str:
        actual_path = self._item_actual_path(item)
        projected_path = self._item_projected_path(item)
        relative_path = self._item_relative_path(item)
        if actual_path:
            return actual_path
        if projected_path:
            return projected_path
        if relative_path:
            return relative_path
        return self._source_root or ""

    def _handle_folder_preview_item_activated(
        self, item: QListWidgetItem | None
    ) -> None:
        if item is None:
            return
        source_path = item.data(Qt.ItemDataRole.UserRole)
        if source_path:
            self._focus_path_in_trees(str(source_path))

    def _focus_path_in_trees(self, source_path: str) -> None:
        after_item = self._after_file_items_by_path.get(source_path)
        if after_item is not None:
            self.preview_tree.setCurrentItem(after_item)
            after_item.setSelected(True)
            self.preview_tree.scrollToItem(after_item)
            return
        before_item = self._before_file_items_by_path.get(source_path)
        if before_item is not None:
            self.before_tree.setCurrentItem(before_item)
            before_item.setSelected(True)
            self.before_tree.scrollToItem(before_item)

    def _show_before_context_menu(self, position: QPoint) -> None:
        item = self.before_tree.itemAt(position)
        if item is None:
            return
        self.before_tree.setCurrentItem(item)
        menu = QMenu(self)
        self._populate_common_context_actions(
            menu, item, self.before_tree, is_after=False
        )
        menu.exec(self.before_tree.viewport().mapToGlobal(position))

    def _show_after_context_menu(self, position: QPoint) -> None:
        item = self.preview_tree.itemAt(position)
        if item is None:
            return
        if not item.isSelected():
            self.preview_tree.clearSelection()
            item.setSelected(True)
        self.preview_tree.setCurrentItem(item)
        menu = QMenu(self)
        common_added = self._populate_common_context_actions(
            menu, item, self.preview_tree, is_after=True
        )
        edit_added = self._populate_after_edit_actions(menu, item)
        if not common_added and not edit_added:
            return
        menu.exec(self.preview_tree.viewport().mapToGlobal(position))

    def _populate_common_context_actions(
        self,
        menu: QMenu,
        item: QTreeWidgetItem,
        tree: QTreeWidget,
        *,
        is_after: bool,
    ) -> bool:
        actual_path = self._existing_path_for_item(item)
        source_path = self._item_source_path(item)
        projected_path = self._item_projected_path(item)
        relative_path = self._item_relative_path(item)

        sections: list[list[QAction]] = []

        open_actions: list[QAction] = []
        if actual_path:
            open_action = QAction("Open", self)
            open_action.triggered.connect(lambda: self._open_item(actual_path))
            open_actions.append(open_action)

            reveal_action = QAction("Reveal in File Manager", self)
            reveal_action.triggered.connect(
                lambda: self._reveal_in_file_manager(actual_path)
            )
            open_actions.append(reveal_action)
        if open_actions:
            sections.append(open_actions)

        copy_actions: list[QAction] = []
        if actual_path or source_path:
            copy_full_path = QAction("Copy full path", self)
            copy_full_path.triggered.connect(
                lambda: self._copy_to_clipboard(actual_path or source_path or "")
            )
            copy_actions.append(copy_full_path)
        if relative_path:
            copy_relative = QAction("Copy relative path", self)
            copy_relative.triggered.connect(
                lambda: self._copy_to_clipboard(relative_path)
            )
            copy_actions.append(copy_relative)
        if projected_path:
            copy_projected = QAction("Copy projected destination", self)
            copy_projected.triggered.connect(
                lambda: self._copy_to_clipboard(projected_path or "")
            )
            copy_actions.append(copy_projected)
        if actual_path:
            open_terminal = QAction("Open terminal here", self)
            open_terminal.triggered.connect(
                lambda: self._open_terminal_here(actual_path)
            )
            copy_actions.append(open_terminal)
        if copy_actions:
            sections.append(copy_actions)

        delete_actions: list[QAction] = []
        if is_after:
            delete_target_path = self._deletable_path_for_item(item)
            candidate_paths = self._preview_paths_for_item(item)
            if candidate_paths:
                all_marked = all(self._is_marked_func(path) for path in candidate_paths)
                mark_action = QAction(
                    "Unmark from Trash" if all_marked else "Mark for Trash", self
                )
                mark_action.triggered.connect(
                    lambda: self._toggle_item_deletion_marks(item)
                )
                delete_actions.append(mark_action)
            if delete_target_path:
                noun = "folder" if os.path.isdir(delete_target_path) else "file"
                trash_action = QAction(f"Move {noun} to Trash now…", self)
                trash_action.triggered.connect(
                    lambda: self._request_trash_for_item(item)
                )
                delete_actions.append(trash_action)
        if delete_actions:
            sections.append(delete_actions)

        subtree_actions: list[QAction] = []
        if item.childCount() > 0:
            expand_subtree = QAction("Expand subtree", self)
            expand_subtree.setEnabled(not self._is_subtree_fully_expanded(item))
            expand_subtree.triggered.connect(
                lambda: self._set_subtree_expanded(tree, item, True)
            )
            subtree_actions.append(expand_subtree)

            collapse_subtree = QAction("Collapse subtree", self)
            collapse_subtree.setEnabled(not self._is_subtree_fully_collapsed(item))
            collapse_subtree.triggered.connect(
                lambda: self._set_subtree_expanded(tree, item, False)
            )
            subtree_actions.append(collapse_subtree)

            if self._has_expandable_descendants(item):
                expand_children = QAction("Expand all children", self)
                expand_children.setEnabled(
                    not self._are_descendants_fully_expanded(item)
                )
                expand_children.triggered.connect(
                    lambda: self._set_children_expanded(tree, item, True)
                )
                subtree_actions.append(expand_children)

                collapse_children = QAction("Collapse all children", self)
                collapse_children.setEnabled(
                    not self._are_descendants_fully_collapsed(item)
                )
                collapse_children.triggered.connect(
                    lambda: self._set_children_expanded(tree, item, False)
                )
                subtree_actions.append(collapse_children)
        if subtree_actions:
            sections.append(subtree_actions)

        return self._append_menu_sections(menu, sections)

    def _populate_after_edit_actions(self, menu: QMenu, item: QTreeWidgetItem) -> bool:
        kind = self._item_kind(item)
        selected_paths = self._selected_preview_file_paths()
        candidate_paths = self._paths_for_action(item)
        editable_candidate_paths = [
            path for path in candidate_paths if path not in self._editable_skipped
        ]
        group_id = self._item_group_id(item)
        sections: list[list[QAction]] = []

        structure_actions: list[QAction] = []
        if kind == ITEM_GROUP:
            rename_group = QAction("Rename folder", self)
            rename_group.triggered.connect(lambda: self._rename_group(item))
            structure_actions.append(rename_group)
        if kind == ITEM_FILE:
            rename_file = QAction("Rename file", self)
            rename_file.triggered.connect(lambda: self._rename_file(item))
            structure_actions.append(rename_file)
        if (
            kind in {ITEM_ROOT, ITEM_DIRECTORY, ITEM_GROUP, ITEM_FILE}
            and self._item_bucket(item) != ITEM_SKIPPED
        ):
            create_subgroup = QAction("Create folder", self)
            create_subgroup.triggered.connect(
                lambda: self._create_subgroup_from_item(item)
            )
            structure_actions.append(create_subgroup)
        if (
            kind in {ITEM_DIRECTORY, ITEM_GROUP}
            and self._item_bucket(item) != ITEM_SKIPPED
        ):
            create_parent = QAction("Create parent folder", self)
            create_parent.triggered.connect(
                lambda: self._create_parent_directory_for_item(item)
            )
            structure_actions.append(create_parent)
        if (
            kind == ITEM_GROUP
            and selected_paths
            and not self._all_paths_in_group(selected_paths, group_id)
        ):
            move_here = QAction("Move selected preview items here", self)
            move_here.triggered.connect(
                lambda: self._move_selected_preview_items_here(item)
            )
            structure_actions.append(move_here)
        if structure_actions:
            sections.append(structure_actions)

        bucket_actions: list[QAction] = []
        if editable_candidate_paths:
            restore_action = QAction("Put back in original folder", self)
            restore_action.triggered.connect(
                lambda: self._restore_paths_to_original_location(
                    editable_candidate_paths
                )
            )
            bucket_actions.append(restore_action)
        if bucket_actions:
            sections.append(bucket_actions)

        inspection_actions: list[QAction] = []
        if self._conflicts_for_item(item):
            conflicts_action = QAction("Show path conflicts/collisions", self)
            conflicts_action.triggered.connect(
                lambda: self._show_conflicts_for_item(item)
            )
            inspection_actions.append(conflicts_action)
        if inspection_actions:
            sections.append(inspection_actions)

        return self._append_menu_sections(menu, sections)

    def _append_menu_sections(self, menu: QMenu, sections: list[list[QAction]]) -> bool:
        added_any = False
        for section in sections:
            if not section:
                continue
            if added_any:
                menu.addSeparator()
            for action in section:
                menu.addAction(action)
            added_any = True
        return added_any

    def _has_expandable_descendants(self, item: QTreeWidgetItem) -> bool:
        for index in range(item.childCount()):
            child = item.child(index)
            if self._item_kind(child) == ITEM_DIRECTORY:
                return True
        return False

    def _is_subtree_fully_expanded(self, item: QTreeWidgetItem) -> bool:
        if item.childCount() <= 0:
            return True
        if not item.isExpanded():
            return False
        return self._are_descendants_fully_expanded(item)

    def _are_descendants_fully_expanded(self, item: QTreeWidgetItem) -> bool:
        for index in range(item.childCount()):
            child = item.child(index)
            if child.childCount() <= 0:
                continue
            if not child.isExpanded():
                return False
            if not self._are_descendants_fully_expanded(child):
                return False
        return True

    def _is_subtree_fully_collapsed(self, item: QTreeWidgetItem) -> bool:
        if item.childCount() <= 0:
            return True
        if item.isExpanded():
            return False
        return self._are_descendants_fully_collapsed(item)

    def _are_descendants_fully_collapsed(self, item: QTreeWidgetItem) -> bool:
        for index in range(item.childCount()):
            child = item.child(index)
            if child.childCount() <= 0:
                continue
            if child.isExpanded():
                return False
            if not self._are_descendants_fully_collapsed(child):
                return False
        return True

    def _selected_preview_file_paths(self) -> list[str]:
        paths: list[str] = []
        for item in self.preview_tree.selectedItems():
            source_path = self._item_source_path(item)
            if self._item_kind(item) == ITEM_FILE and source_path:
                paths.append(source_path)
        return list(dict.fromkeys(paths))

    def _paths_for_action(self, item: QTreeWidgetItem) -> list[str]:
        selected_paths = self._selected_preview_file_paths()
        if selected_paths:
            return selected_paths

        kind = self._item_kind(item)
        source_path = self._item_source_path(item)
        if kind == ITEM_FILE and source_path:
            return [source_path]
        if kind == ITEM_GROUP:
            group = self._find_group(self._item_group_id(item))
            return list(group.source_paths) if group else []
        if kind == ITEM_UNASSIGNED:
            return list(self._editable_unassigned)
        if kind == ITEM_SKIPPED:
            return list(self._editable_skipped)
        return []

    def _existing_path_for_item(self, item: QTreeWidgetItem) -> str | None:
        actual_path = self._item_actual_path(item)
        source_path = self._item_source_path(item)
        kind = self._item_kind(item)

        if source_path and os.path.exists(source_path):
            return source_path
        if actual_path and os.path.exists(actual_path):
            return actual_path
        if kind in {
            ITEM_GROUP,
            ITEM_UNASSIGNED,
            ITEM_SKIPPED,
            ITEM_ROOT,
            ITEM_DIRECTORY,
        }:
            source_root = self._source_root
            if source_root and os.path.exists(source_root):
                return source_root
        return None

    def _copy_to_clipboard(self, text: str) -> None:
        QApplication.clipboard().setText(text or "")
        self._show_status_message("Copied to clipboard.", 2000)

    def _open_item(self, path: str | None) -> None:
        if not path:
            return
        opened = QDesktopServices.openUrl(QUrl.fromLocalFile(path))
        if not opened:
            self._show_status_message(f"Failed to open {path}.", 3000)

    def _reveal_in_file_manager(self, path: str | None) -> None:
        if not path:
            return
        normalized = os.path.normpath(path)
        try:
            if os.name == "nt":
                subprocess.run(["explorer", "/select,", normalized], check=False)
            elif os.name == "posix":
                if os.uname().sysname == "Darwin":
                    if os.path.isdir(normalized):
                        subprocess.run(["open", normalized], check=False)
                    else:
                        subprocess.run(["open", "-R", normalized], check=False)
                else:
                    target = (
                        normalized
                        if os.path.isdir(normalized)
                        else os.path.dirname(normalized)
                    )
                    subprocess.run(["xdg-open", target], check=False)
        except Exception:
            self._show_status_message(f"Failed to reveal {path}.", 3000)

    def _open_terminal_here(self, path: str | None) -> None:
        if not path:
            return
        target = path if os.path.isdir(path) else os.path.dirname(path)
        try:
            if os.name == "posix" and os.uname().sysname == "Darwin":
                subprocess.run(["open", "-a", "Terminal", target], check=False)
            elif os.name == "posix":
                subprocess.run(["xdg-open", target], check=False)
            elif os.name == "nt":
                subprocess.run(
                    ["cmd", "/c", "start", "cmd.exe", "/K", f"cd /d {target}"],
                    check=False,
                )
        except Exception:
            self._show_status_message(f"Failed to open terminal for {target}.", 3000)

    def _show_status_message(self, message: str, timeout: int = 3000) -> None:
        if self._parent_window and hasattr(self._parent_window, "statusBar"):
            self._parent_window.statusBar().showMessage(message, timeout)

    _AFTER_DESC_DEFAULT = "Drag items to move \u00b7 Double-click to rename"

    def _set_subtree_expanded(
        self, tree: QTreeWidget, item: QTreeWidgetItem, expanded: bool
    ) -> None:
        tree.setUpdatesEnabled(False)
        self._set_children_expanded(tree, item, expanded)
        item.setExpanded(expanded)
        tree.setUpdatesEnabled(True)

    def _set_children_expanded(
        self, tree: QTreeWidget, item: QTreeWidgetItem, expanded: bool
    ) -> None:
        for index in range(item.childCount()):
            child = item.child(index)
            child.setExpanded(expanded)
            self._set_children_expanded(tree, child, expanded)

    def _has_matching_item(self, item: QTreeWidgetItem, *, is_after: bool) -> bool:
        return self._find_matching_item(item, is_after=is_after) is not None

    def _find_matching_item(
        self, item: QTreeWidgetItem, *, is_after: bool
    ) -> QTreeWidgetItem | None:
        try:
            kind = self._item_kind(item)
            source_path = self._item_source_path(item)
            relative_path = self._item_relative_path(item)
        except RuntimeError:
            logger.debug("Skipping matching lookup for deleted tree item.")
            return None
        if source_path:
            return (
                self._before_file_items_by_path.get(source_path)
                if is_after
                else self._after_file_items_by_path.get(source_path)
            )
        if kind == ITEM_ROOT:
            return self._before_root_item if is_after else self._after_root_item
        if is_after and kind in {ITEM_GROUP, ITEM_DIRECTORY}:
            return self._before_dir_items_by_relative_path.get(
                self._item_match_relative_path(item) or relative_path
            )
        if not is_after and kind == ITEM_DIRECTORY:
            return (
                self._after_items_by_match_relative_path.get(relative_path)
                or self._after_group_items_by_label.get(relative_path)
                or self._after_dir_items_by_relative_path.get(relative_path)
            )
        return None

    def _select_matching_item(self, item: QTreeWidgetItem, *, is_after: bool) -> None:
        target_item = self._find_matching_item(item, is_after=is_after)
        if target_item is None:
            return
        target_tree = self.before_tree if is_after else self.preview_tree
        target_tree.setCurrentItem(target_item)
        target_item.setSelected(True)
        target_tree.scrollToItem(target_item)

    def _sync_selection_to_other_tree(
        self, item: QTreeWidgetItem | None, *, from_after: bool
    ) -> None:
        if self._syncing_tree_selection:
            return

        if item is None:
            return

        target_tree = self.before_tree if from_after else self.preview_tree
        try:
            target_item = self._find_matching_item(item, is_after=from_after)
        except RuntimeError:
            logger.debug("Skipping selection sync for deleted tree item.")
            return
        if target_item is None:
            return

        self._syncing_tree_selection = True
        try:
            target_tree.blockSignals(True)
            current_target = target_tree.currentItem()
            if current_target is not target_item:
                if target_tree is self.preview_tree:
                    target_tree.clearSelection()
                target_tree.setCurrentItem(target_item)
            if not target_item.isSelected():
                target_item.setSelected(True)
        finally:
            target_tree.blockSignals(False)
            self._syncing_tree_selection = False

    def _preview_path(self, source_path: str | None) -> None:
        if source_path:
            self._focus_path_in_trees(source_path)

    def _rename_group(self, item: QTreeWidgetItem) -> None:
        if self._item_kind(item) != ITEM_GROUP:
            return
        self.preview_tree.editItem(item, 0)

    def _rename_file(self, item: QTreeWidgetItem) -> None:
        if self._item_kind(item) != ITEM_FILE:
            return
        self.preview_tree.editItem(item, 0)

    def _prompt_for_folder_name(self, *, title: str, label: str) -> str:
        folder_name, accepted = QInputDialog.getText(self, title, label)
        if not accepted:
            return ""
        return self._normalize_leaf_name(folder_name)

    def _create_subgroup_from_item(self, item: QTreeWidgetItem) -> None:
        base_relative = ""
        kind = self._item_kind(item)
        if kind == ITEM_FILE:
            group = self._find_group(self._item_group_id(item))
            base_relative = group.group_label if group else ""
        elif kind in {ITEM_GROUP, ITEM_DIRECTORY}:
            base_relative = self._item_relative_path(item)
        elif kind == ITEM_ROOT:
            base_relative = ""

        leaf_name = self._prompt_for_folder_name(
            title="Create Folder",
            label="Folder name:",
        )
        if not leaf_name:
            return
        new_group_label = (
            os.path.join(base_relative, leaf_name) if base_relative else leaf_name
        )
        new_group = GroupingGroup(
            group_id=self._next_group_id(),
            group_label=new_group_label,
            source_paths=[],
        )
        self._editable_groups.append(new_group)
        candidate_paths = self._paths_for_action(item)
        if candidate_paths:
            self._move_paths(
                candidate_paths, new_group.group_id, keep_empty_groups=False
            )
        else:
            self._sticky_empty_group_ids.add(str(new_group.group_id))
            self._refresh_preview_trees()

    def _create_parent_directory_for_item(self, item: QTreeWidgetItem) -> None:
        subtree_root = self._normalize_relative_path(self._item_relative_path(item))
        if not subtree_root:
            return

        parent_name = self._prompt_for_folder_name(
            title="Create Parent Folder",
            label="Parent folder name:",
        )
        if not parent_name:
            return

        parent_rel = os.path.dirname(subtree_root)
        if parent_rel == ".":
            parent_rel = ""
        subtree_leaf = os.path.basename(subtree_root)
        new_root = os.path.join(parent_rel, parent_name, subtree_leaf)
        subtree_prefix = subtree_root + os.sep

        updated = False
        for group in self._editable_groups:
            normalized_label = self._normalize_relative_path(group.group_label)
            if normalized_label == subtree_root:
                group.group_label = new_root
                updated = True
                continue
            if normalized_label.startswith(subtree_prefix):
                suffix = normalized_label[len(subtree_prefix) :]
                group.group_label = os.path.join(new_root, suffix)
                updated = True
        if updated:
            self._refresh_preview_trees()

    def _move_selected_preview_items_here(self, item: QTreeWidgetItem) -> None:
        target_group_id = self._item_group_id(item)
        if target_group_id is None:
            return
        selected_paths = self._selected_preview_file_paths()
        if not selected_paths:
            return
        self._move_paths(selected_paths, target_group_id, keep_empty_groups=False)

    def _remove_paths_from_all_buckets(self, paths: Iterable[str]) -> None:
        path_set = set(paths)
        for group in self._editable_groups:
            group.source_paths = [
                path for path in group.source_paths if path not in path_set
            ]
        self._editable_unassigned = [
            path for path in self._editable_unassigned if path not in path_set
        ]

    def _move_paths(
        self, paths: Iterable[str], target_group_id: str, *, keep_empty_groups: bool
    ) -> None:
        path_list = list(dict.fromkeys(paths))
        target_group = self._find_group(target_group_id)
        if target_group is None or not path_list:
            return
        self._remove_paths_from_all_buckets(path_list)
        for path in path_list:
            if path in self._editable_skipped:
                continue
            if path not in target_group.source_paths:
                target_group.source_paths.append(path)
        target_group.source_paths.sort()
        self._sticky_empty_group_ids.discard(str(target_group.group_id))
        self._prune_empty_groups(keep_sticky=keep_empty_groups)
        self._refresh_preview_trees()

    def _move_paths_to_unassigned(self, paths: Iterable[str]) -> None:
        path_list = [
            path for path in dict.fromkeys(paths) if path not in self._editable_skipped
        ]
        if not path_list:
            return
        self._remove_paths_from_all_buckets(path_list)
        for path in path_list:
            if path not in self._editable_unassigned:
                self._editable_unassigned.append(path)
        self._editable_unassigned.sort()
        self._prune_empty_groups()
        self._refresh_preview_trees()

    def _nest_group_under_target(
        self, source_group_id: str, target_group_id: str
    ) -> None:
        source = self._find_group(source_group_id)
        target = self._find_group(target_group_id)
        if source is None or target is None or source is target:
            return
        source_leaf = os.path.basename(source.group_label) or source.group_label
        target_label = target.group_label
        if source.group_label == target_label or target_label.startswith(
            source.group_label + os.sep
        ):
            return  # circular nesting
        source.group_label = os.path.join(target_label, source_leaf)
        self._refresh_preview_trees()

    def _nest_group_under_directory(
        self, source_group_id: str, target_relative_path: str
    ) -> None:
        source = self._find_group(source_group_id)
        if source is None:
            return
        current_label = self._normalize_relative_path(source.group_label)
        normalized_target = self._normalize_relative_path(target_relative_path)
        if normalized_target == current_label or (
            current_label and normalized_target.startswith(current_label + os.sep)
        ):
            return
        source_leaf = os.path.basename(source.group_label) or source.group_label
        new_label = (
            os.path.join(normalized_target, source_leaf)
            if normalized_target
            else source_leaf
        )
        if new_label == source.group_label:
            return
        source.group_label = new_label
        self._refresh_preview_trees()

    def _unnest_group_to_root(self, source_group_id: str) -> None:
        source = self._find_group(source_group_id)
        if source is None:
            return
        leaf = os.path.basename(source.group_label) or source.group_label
        if leaf == source.group_label:
            return  # already at root
        source.group_label = leaf
        self._refresh_preview_trees()

    def _move_files_to_directory(
        self, paths: list[str], target_relative_path: str
    ) -> None:
        group = next(
            (g for g in self._editable_groups if g.group_label == target_relative_path),
            None,
        )
        if group is None:
            group = GroupingGroup(
                group_id=self._next_group_id(),
                group_label=target_relative_path or "Unnamed",
                source_paths=[],
            )
            self._editable_groups.append(group)
        self._move_paths(paths, group.group_id, keep_empty_groups=False)

    def _directory_path_for_item(self, item: QTreeWidgetItem | None) -> str | None:
        if item is None:
            return None
        if self._item_kind(item) not in {ITEM_DIRECTORY, ITEM_GROUP}:
            return None
        relative_path = self._item_match_relative_path(
            item
        ) or self._item_relative_path(item)
        normalized = self._normalize_relative_path(relative_path)
        source_root = self._source_root or ""
        if not source_root:
            return None
        if not normalized:
            return source_root
        return os.path.join(source_root, normalized)

    def _deletable_path_for_item(self, item: QTreeWidgetItem | None) -> str | None:
        if item is None:
            return None

        kind = self._item_kind(item)
        if kind == ITEM_FILE:
            source_path = self._item_source_path(item)
            if source_path and os.path.isfile(source_path):
                return source_path
            return None

        if kind in {ITEM_DIRECTORY, ITEM_GROUP}:
            directory_path = self._directory_path_for_item(item)
            if directory_path and os.path.isdir(directory_path):
                return directory_path

        return None

    def _tracked_paths_for_directory(self, directory_path: str) -> list[str]:
        normalized_dir = os.path.normcase(os.path.normpath(directory_path))
        tracked: list[str] = []
        for path in self._all_source_paths():
            normalized_path = os.path.normcase(os.path.normpath(path))
            try:
                if (
                    os.path.commonpath([normalized_path, normalized_dir])
                    == normalized_dir
                ):
                    tracked.append(path)
            except Exception:
                continue
        return list(dict.fromkeys(tracked))

    def _tracked_paths_for_file(self, file_path: str) -> list[str]:
        normalized_target = os.path.normcase(os.path.normpath(file_path))
        tracked = [
            path
            for path in self._all_source_paths()
            if os.path.normcase(os.path.normpath(path)) == normalized_target
        ]
        return list(dict.fromkeys(tracked))

    def _remove_deleted_paths_from_state(self, paths: Iterable[str]) -> None:
        path_set = set(paths)
        if not path_set:
            return

        self._editable_groups = [
            GroupingGroup(
                group_id=str(group.group_id),
                group_label=group.group_label,
                source_paths=[p for p in group.source_paths if p not in path_set],
            )
            for group in self._editable_groups
        ]
        self._editable_unassigned = [
            path for path in self._editable_unassigned if path not in path_set
        ]
        self._editable_skipped = [
            path for path in self._editable_skipped if path not in path_set
        ]
        self._file_name_overrides = {
            path: name
            for path, name in self._file_name_overrides.items()
            if path not in path_set
        }
        self._original_group_labels_by_path = {
            path: label
            for path, label in self._original_group_labels_by_path.items()
            if path not in path_set
        }
        if self._current_plan is not None:
            self._current_plan.groups = [
                GroupingGroup(
                    group_id=str(group.group_id),
                    group_label=group.group_label,
                    source_paths=[p for p in group.source_paths if p not in path_set],
                )
                for group in self._current_plan.groups
            ]
            self._current_plan.unassigned_paths = [
                path
                for path in self._current_plan.unassigned_paths
                if path not in path_set
            ]
            self._current_plan.skipped_paths = [
                path
                for path in self._current_plan.skipped_paths
                if path not in path_set
            ]
            self._current_plan.file_name_overrides = {
                path: name
                for path, name in self._current_plan.file_name_overrides.items()
                if path not in path_set
            }
        self._prune_empty_groups()
        self._refresh_preview_trees()

    def _delete_item(self, item: QTreeWidgetItem | None) -> None:
        """Compatibility wrapper for the former staged-removal action."""
        if item is not None:
            self._toggle_item_deletion_marks(item)

    def _toggle_item_deletion_marks(self, item: QTreeWidgetItem) -> None:
        selected_paths = self._selected_preview_file_paths()
        paths = selected_paths or self._preview_paths_for_item(item)
        paths = [path for path in dict.fromkeys(paths) if os.path.isfile(path)]
        if not paths:
            self._show_status_message("No files are available to mark.", 3000)
            return
        directory_path = self._deletable_path_for_item(item)
        if not selected_paths and directory_path and os.path.isdir(directory_path):
            self._start_directory_validation("mark", directory_path, paths)
            return
        self.toggle_deletion_marks_requested.emit(paths)

    def _trash_current_preview(self) -> None:
        path = self._current_preview_source_path
        if path and os.path.isfile(path):
            self.trash_requested.emit(path, [path])

    def _request_trash_for_item(self, item: QTreeWidgetItem) -> None:
        selected_paths = self._selected_preview_file_paths()
        if selected_paths:
            existing = [path for path in selected_paths if os.path.isfile(path)]
            if existing:
                self.trash_requested.emit("", existing)
            return

        target_path = self._deletable_path_for_item(item)
        if not target_path:
            self._show_status_message(
                "This preview item cannot be moved to Trash.", 3000
            )
            return
        tracked_paths = (
            self._tracked_paths_for_directory(target_path)
            if os.path.isdir(target_path)
            else self._tracked_paths_for_file(target_path)
        )
        if os.path.isdir(target_path):
            self._start_directory_validation("trash", target_path, tracked_paths)
            return
        self.trash_requested.emit(target_path, tracked_paths)

    def _start_directory_validation(
        self, operation: str, directory_path: str, represented_paths: list[str]
    ) -> None:
        self._folder_validation_request_id += 1
        task = _DirectoryInventoryTask(
            self._folder_validation_request_id,
            operation,
            directory_path,
            represented_paths,
        )
        task.signals.completed.connect(self._handle_directory_validation)
        self._show_status_message("Checking folder contents…", 2000)
        self._folder_validation_pool.start(task)

    def _handle_directory_validation(
        self,
        request_id: int,
        operation: str,
        directory_path: str,
        represented_paths: object,
        safe: bool,
        reason: str,
    ) -> None:
        if request_id != self._folder_validation_request_id:
            return
        paths = (
            [str(path) for path in represented_paths]
            if isinstance(represented_paths, list)
            else []
        )
        if not safe:
            self._show_status_message(reason, 5000)
            return
        if operation == "mark":
            self.toggle_deletion_marks_requested.emit(paths)
        elif operation == "trash":
            self.trash_requested.emit(directory_path, paths)

    def _restore_paths_to_original_location(self, paths: Iterable[str]) -> None:
        path_list = [
            path for path in dict.fromkeys(paths) if path not in self._editable_skipped
        ]
        if not path_list:
            return
        self._remove_paths_from_all_buckets(path_list)
        for path in path_list:
            if path in self._original_group_labels_by_path:
                target_label = self._original_group_labels_by_path[path]
            else:
                target_label = self._relative_dir_label_for_source(path)
            group = next(
                (
                    candidate
                    for candidate in self._editable_groups
                    if candidate.group_label == target_label
                ),
                None,
            )
            if group is None:
                group = GroupingGroup(
                    group_id=self._next_group_id(),
                    group_label=target_label,
                    source_paths=[],
                )
                self._editable_groups.append(group)
            if path not in group.source_paths:
                group.source_paths.append(path)
                group.source_paths.sort()
        self._prune_empty_groups()
        self._refresh_preview_trees()

    def _prune_empty_groups(self, keep_sticky: bool = True) -> None:
        retained_groups: list[GroupingGroup] = []
        for group in self._editable_groups:
            if group.source_paths:
                retained_groups.append(group)
                continue
            if keep_sticky and str(group.group_id) in self._sticky_empty_group_ids:
                retained_groups.append(group)
                continue
            self._sticky_empty_group_ids.discard(str(group.group_id))
        self._editable_groups = retained_groups

    def _all_paths_in_group(self, paths: Iterable[str], group_id: str | None) -> bool:
        group = self._find_group(group_id)
        if group is None:
            return False
        group_paths = set(group.source_paths)
        return all(path in group_paths for path in paths)

    def _conflicts_for_item(self, item: QTreeWidgetItem) -> list[str]:
        relevant_paths = self._paths_for_action(item)
        if not relevant_paths:
            relevant_paths = self._all_source_paths()
        destinations: dict[str, list[str]] = {}
        for path in relevant_paths:
            projected = self._projected_path_for_source(path)
            destinations.setdefault(projected, []).append(path)
        conflicts = []
        for projected_path, paths in destinations.items():
            if len(paths) > 1:
                conflicts.append(f"{projected_path}\n  " + "\n  ".join(sorted(paths)))
        return conflicts

    def _show_conflicts_for_item(self, item: QTreeWidgetItem) -> None:
        conflicts = self._conflicts_for_item(item)
        if not conflicts:
            QMessageBox.information(
                self,
                "Path Conflicts",
                "No projected path collisions were found for this selection.",
            )
            return
        preview = "\n\n".join(conflicts[:10])
        if len(conflicts) > 10:
            preview += f"\n\n… {len(conflicts) - 10} more"
        QMessageBox.warning(
            self,
            "Path Conflicts",
            preview,
        )

    def _confirm_grouping_actions(self, plan: GroupingPlan) -> bool:
        action_lines = self._build_action_lines(plan)
        if not action_lines:
            QMessageBox.information(
                self,
                "No Changes To Apply",
                "There are no changes to apply in the current grouping plan.",
            )
            return False

        move_count = sum(1 for ln in action_lines if ln.startswith("Move "))
        rename_count = sum(1 for ln in action_lines if ln.startswith("Rename folder "))
        delete_count = sum(1 for ln in action_lines if ln.startswith("Delete "))
        remove_count = sum(
            1 for ln in action_lines if ln.startswith("Remove empty folder ")
        )

        dialog = QDialog(self)
        dialog.setWindowTitle("Confirm Grouping Actions")
        dialog.setObjectName("groupingConfirmDialog")
        dialog.setModal(True)
        dialog.setMinimumSize(620, 420)
        dialog.setWindowFlags(dialog.windowFlags() | Qt.WindowType.FramelessWindowHint)
        make_dialog_draggable(dialog)

        outer = QVBoxLayout(dialog)
        outer.setContentsMargins(0, 0, 0, 0)
        outer.setSpacing(0)

        build_dialog_header("Confirm Changes", "📁", outer)

        body = QVBoxLayout()
        body.setContentsMargins(22, 16, 22, 10)
        body.setSpacing(12)

        parts: list[str] = []
        if move_count:
            parts.append(f"{move_count} file move(s)")
        if rename_count:
            parts.append(f"{rename_count} folder rename(s)")
        if delete_count:
            parts.append(f"{delete_count} deletion(s)")
        if remove_count:
            parts.append(f"{remove_count} empty folder removal(s)")
        summary_text = "This will apply " + ", ".join(parts) + "."

        summary = QLabel(summary_text)
        summary.setObjectName("groupingConfirmMessage")
        summary.setWordWrap(True)
        body.addWidget(summary)

        action_list = QPlainTextEdit()
        action_list.setObjectName("groupingConfirmActionList")
        action_list.setReadOnly(True)
        action_list.setLineWrapMode(QPlainTextEdit.LineWrapMode.NoWrap)
        action_list.setPlainText("\n".join(action_lines))
        body.addWidget(action_list, 1)

        outer.addLayout(body)

        build_dialog_footer(
            outer,
            [
                ("Cancel", "groupingConfirmCancelButton", dialog.reject, False),
                ("Apply Changes", "groupingConfirmApplyButton", dialog.accept, True),
            ],
        )

        return dialog.exec() == int(QDialog.DialogCode.Accepted)

    def _build_action_lines(self, plan: GroupingPlan) -> list[str]:
        lines: list[str] = []
        moving_paths: list[str] = []
        reserved_paths = self._occupied_paths_for_action_preview()
        directory_renames = find_directory_rename_candidates(
            plan,
            source_root=self._source_root or "",
            output_root=self._current_output_root or self._source_root or "",
        )
        for group in plan.groups:
            directory_rename = directory_renames.get(str(group.group_id))
            if directory_rename is not None:
                lines.append(
                    "Rename folder "
                    f"{self._relative_display_path(directory_rename.source_dir)} -> "
                    f"{self._relative_display_path(directory_rename.target_dir)}"
                )
                continue
            for source_path in group.source_paths:
                destination_path = self._preview_destination_path(
                    source_path,
                    os.path.join(
                        self._current_output_root or self._source_root or "",
                        self._normalize_relative_path(group.group_label),
                    ),
                    plan.filename_for_path(source_path),
                    reserved_paths,
                )
                if os.path.normcase(os.path.normpath(source_path)) == os.path.normcase(
                    os.path.normpath(destination_path)
                ):
                    continue
                moving_paths.append(source_path)
                lines.append(
                    f"Move {self._relative_path_for_source(source_path)} -> "
                    f"{os.path.relpath(destination_path, self._current_output_root or self._source_root or os.path.dirname(destination_path))}"
                )
        for source_path in plan.unassigned_paths:
            destination_path = self._preview_destination_path(
                source_path,
                os.path.join(
                    self._current_output_root or self._source_root or "",
                    "Unassigned",
                ),
                plan.filename_for_path(source_path),
                reserved_paths,
            )
            if os.path.normcase(os.path.normpath(source_path)) == os.path.normcase(
                os.path.normpath(destination_path)
            ):
                continue
            moving_paths.append(source_path)
            lines.append(
                f"Move {self._relative_path_for_source(source_path)} -> "
                f"{os.path.relpath(destination_path, self._current_output_root or self._source_root or os.path.dirname(destination_path))}"
            )
        deleted_file_paths: list[str] = []
        for deleted_path in getattr(plan, "deleted_paths", []) or []:
            if os.path.isdir(deleted_path):
                lines.append(
                    f"Delete folder {self._relative_display_path(deleted_path)}"
                )
                continue
            deleted_file_paths.append(deleted_path)
            lines.append(f"Delete file {self._relative_display_path(deleted_path)}")
        lines.extend(
            f"Remove empty folder {self._relative_display_path(folder_path)}"
            for folder_path in self._empty_directories_after_move(
                list(moving_paths) + deleted_file_paths
            )
        )
        return lines

    def _occupied_paths_for_action_preview(self) -> set[str]:
        root = self._current_output_root or self._source_root or ""
        if not root or not os.path.isdir(root):
            return set()
        occupied: set[str] = set()
        for current_root, _dirnames, filenames in os.walk(root):
            for filename in filenames:
                occupied.add(
                    os.path.normcase(
                        os.path.normpath(os.path.join(current_root, filename))
                    )
                )
        return occupied

    def _preview_destination_path(
        self,
        source_path: str,
        destination_dir: str,
        basename: str,
        reserved_paths: set[str],
    ) -> str:
        desired_destination_path = os.path.join(destination_dir, basename)
        normalized_source = os.path.normcase(os.path.normpath(source_path))
        normalized_desired = os.path.normcase(
            os.path.normpath(desired_destination_path)
        )
        if normalized_source == normalized_desired:
            return desired_destination_path

        reserved_paths.discard(normalized_source)
        candidate = desired_destination_path
        stem, ext = os.path.splitext(basename)
        suffix = 1
        while os.path.normcase(os.path.normpath(candidate)) in reserved_paths:
            candidate = os.path.join(destination_dir, f"{stem}_{suffix}{ext}")
            suffix += 1
        reserved_paths.add(os.path.normcase(os.path.normpath(candidate)))
        return candidate

    def _empty_directories_after_move(self, moving_paths: Iterable[str]) -> list[str]:
        source_root = self._source_root or ""
        if not source_root or not os.path.isdir(source_root):
            return []
        moving_set = set()
        for path in moving_paths:
            if path and os.path.exists(path):
                moving_set.add(os.path.normcase(os.path.normpath(path)))
        removable_dirs: set[str] = set()
        removable_dirs_by_key: dict[str, str] = {}
        normalized_source_root = os.path.normcase(os.path.normpath(source_root))
        for current_root, dirnames, filenames in os.walk(source_root, topdown=False):
            normalized_current = os.path.normcase(os.path.normpath(current_root))
            if normalized_current == normalized_source_root:
                continue
            remaining_files = any(
                os.path.normcase(os.path.normpath(os.path.join(current_root, filename)))
                not in moving_set
                for filename in filenames
            )
            remaining_children = any(
                os.path.normcase(os.path.normpath(os.path.join(current_root, dirname)))
                not in removable_dirs
                for dirname in dirnames
            )
            if not remaining_files and not remaining_children:
                removable_dirs.add(normalized_current)
                removable_dirs_by_key[normalized_current] = current_root
        ordered_keys = sorted(
            removable_dirs_by_key.keys(),
            key=lambda path: removable_dirs_by_key[path].count(os.sep),
            reverse=True,
        )
        return [removable_dirs_by_key[path] for path in ordered_keys]

    def _relative_display_path(self, path: str) -> str:
        source_root = self._source_root or ""
        try:
            if source_root:
                return os.path.relpath(path, source_root)
        except Exception:
            pass
        return path

    @override
    def eventFilter(self, obj: QObject, event: QEvent) -> bool:
        if event.type() == QEvent.Type.KeyPress:
            if obj in (self.before_tree, self.preview_tree):
                key_event: QKeyEvent = event
                key = key_event.key()
                modifiers = key_event.modifiers()
                is_up = key in (Qt.Key.Key_Up, Qt.Key.Key_K)
                is_down = key in (Qt.Key.Key_Down, Qt.Key.Key_J)
                if is_up or is_down:
                    has_shift = bool(modifiers & Qt.KeyboardModifier.ShiftModifier)
                    has_alt = bool(modifiers & Qt.KeyboardModifier.AltModifier)
                    if not has_shift and not has_alt:
                        has_special = (
                            bool(modifiers & Qt.KeyboardModifier.MetaModifier)
                            if sys.platform == "darwin"
                            else bool(modifiers & Qt.KeyboardModifier.ControlModifier)
                        )
                        skip_deleted = not has_special
                        tree: QTreeWidget = obj
                        current_item = tree.currentItem()
                        if current_item is not None:
                            candidate = current_item
                            while True:
                                candidate = (
                                    tree.itemAbove(candidate)
                                    if is_up
                                    else tree.itemBelow(candidate)
                                )
                                if candidate is None:
                                    break
                                # If skipping is enabled, check if the candidate is marked for deletion
                                if skip_deleted:
                                    source_path = self._item_source_path(candidate)
                                    if source_path and self._is_marked_func(source_path):
                                        continue
                                # Found a valid item!
                                tree.setCurrentItem(candidate)
                                return True
                            # If we reached the end of the tree and found no valid item,
                            # consume the event to prevent selecting an invalid item.
                            return True
        return super().eventFilter(obj, event)

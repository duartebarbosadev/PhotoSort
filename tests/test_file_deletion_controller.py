import os
from PyQt6.QtCore import Qt, QModelIndex
from PyQt6.QtGui import QStandardItemModel, QStandardItem
from PyQt6.QtWidgets import QApplication, QTreeView
from PyQt6.QtCore import QSortFilterProxyModel

from src.ui.controllers.file_deletion_controller import FileDeletionController

# Ensure a QApplication exists
app = QApplication.instance() or QApplication([])


class IdentityProxy(QSortFilterProxyModel):
    def mapToSource(self, idx):
        return idx

    def mapFromSource(self, source_idx):
        return source_idx

    def rowCount(self, parent):  # pragma: no cover - trivial
        src = self.sourceModel()
        if src is None:
            return 0
        return src.rowCount(parent)

    def index(self, row, column, parent=QModelIndex()):  # pragma: no cover - trivial
        src = self.sourceModel()
        if src is None:
            return super().index(row, column, parent)
        return src.index(row, column, parent)


class DummyStatusBar:
    def __init__(self):
        self.last_message = None

    def showMessage(self, msg, timeout=0):  # pragma: no cover - trivial
        self.last_message = msg


class DummyAppController:
    def __init__(self):
        self.moved_to_trash = []

    def move_to_trash(self, path):
        # Simulate success
        self.moved_to_trash.append(path)
        return True


class DummyAppState:
    def __init__(self):
        self._data_by_path = {}

    def add_path(self, path):
        self._data_by_path[path] = {}

    def remove_data_for_path(self, path):
        self._data_by_path.pop(path, None)


class DummyDialogManager:
    def __init__(self, confirm=True):
        self.confirm = confirm

    def show_confirm_delete_dialog(self, paths):  # pragma: no cover - trivial
        return self.confirm

    def show_error_dialog(self, *a, **k):  # pragma: no cover - trivial
        pass


class DummyAdvancedViewer:
    def __init__(self):
        self.focused = None

    def get_focused_image_path_if_any(self):
        return self.focused

    def clear(self):  # pragma: no cover - trivial
        self.focused = None

    def setText(self, *_):  # pragma: no cover - trivial
        pass


class DeletionTestContext:
    """Minimal context satisfying FileDeletionContextProtocol for tests."""

    def __init__(self):
        self.file_system_model = QStandardItemModel()
        self.proxy_model = IdentityProxy()
        self.proxy_model.setSourceModel(self.file_system_model)
        self.view = QTreeView()
        self.view.setModel(self.proxy_model)
        self.app_controller = DummyAppController()
        self.app_state = DummyAppState()
        self.dialog_manager = DummyDialogManager(confirm=True)
        self.advanced_image_viewer = DummyAdvancedViewer()
        self._selected_paths = []
        self._status_bar = DummyStatusBar()
        self.show_folders_mode = False
        self.group_by_similarity_mode = False

    # Protocol methods
    def _get_active_file_view(self):
        return self.view

    def _get_selected_file_paths_from_view(self):
        return list(self._selected_paths)

    def _get_all_visible_image_paths(self):
        paths = []
        root = self.file_system_model.invisibleRootItem()
        for r in range(root.rowCount()):
            child = root.child(r)
            if not child:
                continue
            d = child.data(Qt.ItemDataRole.UserRole)
            if isinstance(d, dict) and "path" in d:
                paths.append(d["path"])
            # If it's a header (string) include its children
            elif isinstance(d, str):
                for cr in range(child.rowCount()):
                    img = child.child(cr)
                    if not img:
                        continue
                    dd = img.data(Qt.ItemDataRole.UserRole)
                    if isinstance(dd, dict) and "path" in dd:
                        paths.append(dd["path"])
        return paths

    def _find_proxy_index_for_path(self, target_path: str) -> QModelIndex:
        root = self.file_system_model.invisibleRootItem()
        # search depth 1 and 2 (headers)
        for r in range(root.rowCount()):
            child = root.child(r)
            if not child:
                continue
            d = child.data(Qt.ItemDataRole.UserRole)
            if isinstance(d, dict) and d.get("path") == target_path:
                return self.proxy_model.index(r, 0)
            elif isinstance(d, str):  # header
                for cr in range(child.rowCount()):
                    img = child.child(cr)
                    dd = img.data(Qt.ItemDataRole.UserRole)
                    if isinstance(dd, dict) and dd.get("path") == target_path:
                        return self.proxy_model.index(
                            cr, 0, self.proxy_model.index(r, 0)
                        )
        return QModelIndex()

    def _handle_file_selection_changed(
        self, override_selected_paths=None
    ):  # pragma: no cover - trivial
        if override_selected_paths is not None:
            self._selected_paths = list(override_selected_paths)

    def _update_image_info_label(self):  # pragma: no cover - trivial
        pass

    def statusBar(self):  # pragma: no cover - trivial
        return self._status_bar


# Helper to create image item


def make_image_item(path: str):
    it = QStandardItem(os.path.basename(path))
    it.setData({"path": path}, Qt.ItemDataRole.UserRole)
    return it


def test_focused_single_image_delete_restores_selection(tmp_path):
    ctx = DeletionTestContext()
    controller = FileDeletionController(ctx)

    p1 = tmp_path / "a.jpg"
    p1.write_text("x")
    p2 = tmp_path / "b.jpg"
    p2.write_text("y")
    for p in (p1, p2):
        ctx.app_state.add_path(str(p))
        ctx.file_system_model.appendRow(make_image_item(str(p)))
    ctx._selected_paths = [str(p1), str(p2)]
    ctx.advanced_image_viewer.focused = str(p1)  # Focused delete scenario

    controller.move_current_image_to_trash()

    # p1 should be removed; p2 remains and becomes selected/focused candidate
    remaining_paths = ctx._get_all_visible_image_paths()
    assert str(p1) not in remaining_paths and str(p2) in remaining_paths
    assert controller.was_focused_delete is True
    assert ctx.app_controller.moved_to_trash == [str(p1)]


def test_multi_selection_delete(tmp_path):
    ctx = DeletionTestContext()
    controller = FileDeletionController(ctx)

    p1 = tmp_path / "a.jpg"
    p1.write_text("x")
    p2 = tmp_path / "b.jpg"
    p2.write_text("y")
    p3 = tmp_path / "c.jpg"
    p3.write_text("z")
    for p in (p1, p2, p3):
        ctx.app_state.add_path(str(p))
        ctx.file_system_model.appendRow(make_image_item(str(p)))
    ctx._selected_paths = [str(p1), str(p2)]  # multi-select; no focused image

    controller.move_current_image_to_trash()

    remaining = ctx._get_all_visible_image_paths()
    # p1 & p2 removed, p3 should remain
    assert set(remaining) == {str(p3)}
    assert set(ctx.app_controller.moved_to_trash) == {str(p1), str(p2)}
    assert len(ctx.app_controller.moved_to_trash) == 2


def test_empty_header_pruned(tmp_path):
    ctx = DeletionTestContext()
    controller = FileDeletionController(ctx)

    # Create header
    header = QStandardItem("Cluster 1")
    header.setData("cluster_header_1", Qt.ItemDataRole.UserRole)
    p1 = tmp_path / "a.jpg"
    p1.write_text("x")
    ctx.app_state.add_path(str(p1))
    header.appendRow(make_image_item(str(p1)))
    ctx.file_system_model.appendRow(header)
    ctx._selected_paths = [str(p1)]

    # Sanity: header present
    assert ctx.file_system_model.invisibleRootItem().rowCount() == 1

    controller.move_current_image_to_trash()

    # Header should be pruned (rowCount == 0)
    assert ctx.file_system_model.invisibleRootItem().rowCount() == 0

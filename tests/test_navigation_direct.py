import os
from PyQt6.QtCore import Qt, QModelIndex
from PyQt6.QtGui import QStandardItemModel, QStandardItem
from PyQt6.QtWidgets import QApplication, QTreeView
from src.ui.controllers.navigation_controller import NavigationController

app = QApplication.instance() or QApplication([])


class MockNavigationContext:
    def __init__(self):
        self.file_system_model = QStandardItemModel()
        self._active_file_view = QTreeView()
        self._active_file_view.setModel(self.file_system_model)
        self.current_selection_path = None

    def get_active_view(self):
        return self._active_file_view

    def is_valid_image_index(self, index):
        item = self.file_system_model.itemFromIndex(index)
        if not item:
            return False
        d = item.data(Qt.ItemDataRole.UserRole)
        return isinstance(d, dict) and "path" in d

    def map_to_source(self, index):
        return index

    def item_from_source(self, source_index):
        return self.file_system_model.itemFromIndex(source_index)

    def find_first_visible_item(self):
        return QModelIndex()  # Not used in linear navigation

    def get_marked_deleted(self):
        return []

    def get_all_visible_image_paths(self):
        paths = []
        root = self.file_system_model.invisibleRootItem()
        for i in range(root.rowCount()):
            group = root.child(i)
            for j in range(group.rowCount()):
                item = group.child(j)
                d = item.data(Qt.ItemDataRole.UserRole)
                if isinstance(d, dict) and "path" in d:
                    paths.append(d["path"])
        return paths

    def find_proxy_index_for_path(self, path):
        root = self.file_system_model.invisibleRootItem()
        for i in range(root.rowCount()):
            group = root.child(i)
            for j in range(group.rowCount()):
                item = group.child(j)
                d = item.data(Qt.ItemDataRole.UserRole)
                if isinstance(d, dict) and d.get("path") == path:
                    return item.index()
        return QModelIndex()

    def validate_and_select_image_candidate(self, index, direction, log_skip):
        # Mock the selection by storing the path
        if index.isValid():
            item = self.file_system_model.itemFromIndex(index)
            if item:
                d = item.data(Qt.ItemDataRole.UserRole)
                if isinstance(d, dict):
                    self.current_selection_path = d.get("path")

    def get_group_sibling_images(self, current_index):
        parent = current_index.parent()
        count = self.file_system_model.rowCount(parent)
        group_indices = [
            self.file_system_model.index(r, 0, parent) for r in range(count)
        ]
        return parent, group_indices, None

    def set_current_selection(self, path):
        """Helper method to simulate current selection"""
        self.current_selection_path = path

        # Mock the active view's currentIndex
        class MockView:
            def __init__(self, ctx, path):
                self.ctx = ctx
                self.path = path

            def currentIndex(self):
                return self.ctx.find_proxy_index_for_path(self.path)

        self._active_file_view = MockView(self, path)


def make_image_item(path: str):
    it = QStandardItem(os.path.basename(path))
    it.setData({"path": path}, Qt.ItemDataRole.UserRole)
    return it


def test_navigate_down_is_sequential(tmp_path):
    ctx = MockNavigationContext()
    nav = NavigationController(ctx)

    # Create a group with two image items
    group_header = QStandardItem("Group 1")
    group_header.setData("cluster_header_1", Qt.ItemDataRole.UserRole)

    paths = []
    for name in ["a.jpg", "b.jpg"]:
        p = tmp_path / name
        p.write_text("x")
        paths.append(str(p))
        group_header.appendRow(make_image_item(str(p)))
    ctx.file_system_model.appendRow(group_header)

    # Set initial selection to first item
    ctx.set_current_selection(str(paths[0]))  # a.jpg

    # Verify setup
    all_paths = ctx.get_all_visible_image_paths()
    assert all_paths == [str(paths[0]), str(paths[1])], (
        f"Expected paths, got {all_paths}"
    )

    # Navigate down - should move from a.jpg to b.jpg
    nav.navigate_linear("down", skip_deleted=True)

    # Check that selection moved to b.jpg
    assert ctx.current_selection_path == str(paths[1]), (
        f"Expected {paths[1]}, got {ctx.current_selection_path}"
    )


def test_navigate_up_is_sequential(tmp_path):
    ctx = MockNavigationContext()
    nav = NavigationController(ctx)

    # Create a group with three image items
    group_header = QStandardItem("Group 1")
    group_header.setData("cluster_header_1", Qt.ItemDataRole.UserRole)

    paths = []
    for name in ["a.jpg", "b.jpg", "c.jpg"]:
        p = tmp_path / name
        p.write_text("x")
        paths.append(str(p))
        group_header.appendRow(make_image_item(str(p)))
    ctx.file_system_model.appendRow(group_header)

    # Set initial selection to middle item (b.jpg)
    ctx.set_current_selection(str(paths[1]))  # b.jpg

    # Verify setup
    all_paths = ctx.get_all_visible_image_paths()
    assert all_paths == [str(p) for p in paths], f"Expected paths, got {all_paths}"

    # Navigate up - should move from b.jpg to a.jpg
    nav.navigate_linear("up", skip_deleted=True)

    # Check that selection moved to a.jpg
    assert ctx.current_selection_path == str(paths[0]), (
        f"Expected {paths[0]}, got {ctx.current_selection_path}"
    )

import os
from PyQt6.QtCore import Qt
from PyQt6.QtGui import QStandardItemModel, QStandardItem
from PyQt6.QtWidgets import QApplication, QTreeView
from src.ui.main_window import MainWindow

app = QApplication.instance() or QApplication([])


class SimpleTestWindow(MainWindow):
    def __init__(self):
        # Create minimal setup for testing navigation
        self.file_system_model = QStandardItemModel()
        self._active_file_view = QTreeView()
        self._active_file_view.setModel(self.file_system_model)
        
        # Create stub objects that MainWindow expects
        self.app_state = type("S", (), {})()
        self.app_state.date_cache = {}
        
        # Create stub left_panel
        class LeftPanelStub:
            def __init__(self, view):
                self._view = view
            def get_active_view(self):
                return self._view
        self.left_panel = LeftPanelStub(self._active_file_view)
        
        super().__init__()

    def _get_active_file_view(self):
        return self._active_file_view

    def _is_valid_image_item(self, index):
        item = self.file_system_model.itemFromIndex(index)
        if not item:
            return False
        d = item.data(Qt.ItemDataRole.UserRole)
        return isinstance(d, dict) and "path" in d

    # NavigationContext protocol methods
    def get_active_view(self):
        return self._active_file_view

    def is_valid_image_index(self, index):
        return self._is_valid_image_item(index)

    def map_to_source(self, index):
        # No proxy model, so index is already the source index
        return index

    def item_from_source(self, source_index):
        return self.file_system_model.itemFromIndex(source_index)

    def find_first_visible_item(self):
        root = self.file_system_model.invisibleRootItem()
        for i in range(root.rowCount()):
            group = root.child(i)
            for j in range(group.rowCount()):
                item = group.child(j)
                d = item.data(Qt.ItemDataRole.UserRole)
                if isinstance(d, dict) and "path" in d:
                    return item.index()
        from PyQt6.QtCore import QModelIndex
        return QModelIndex()

    def get_marked_deleted(self):
        return []

    def get_all_visible_image_paths(self):
        paths = []
        root = self.file_system_model.invisibleRootItem()
        for i in range(root.rowCount()):
            group = root.child(i)
            # Expand group so children are visible
            self._active_file_view.expand(group.index())
            
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
        from PyQt6.QtCore import QModelIndex
        return QModelIndex()

    def validate_and_select_image_candidate(self, index, direction, log_skip):
        sel_model = self._active_file_view.selectionModel()
        if sel_model:
            sel_model.select(index, sel_model.SelectionFlag.ClearAndSelect)
        self._active_file_view.setCurrentIndex(index)

    def get_group_sibling_images(self, current_index):
        # Simple implementation for testing
        parent = current_index.parent()
        count = self.file_system_model.rowCount(parent)
        group_indices = [self.file_system_model.index(r, 0, parent) for r in range(count)]
        return parent, group_indices, None


def make_image_item(path: str):
    it = QStandardItem(os.path.basename(path))
    it.setData({"path": path}, Qt.ItemDataRole.UserRole)
    return it


def test_navigate_down_works():
    mw = SimpleTestWindow()
    
    # Create a group with two images
    group_header = QStandardItem("Group 1")
    group_header.setData("cluster_header_1", Qt.ItemDataRole.UserRole)
    
    # Create test files and items
    from pathlib import Path
    import tempfile
    with tempfile.TemporaryDirectory() as tmp_dir:
        tmp_path = Path(tmp_dir)
        paths = []
        for name in ["a.jpg", "b.jpg"]:
            p = tmp_path / name
            p.write_text("x")
            paths.append(str(p))
            group_header.appendRow(make_image_item(str(p)))
        
        mw.file_system_model.appendRow(group_header)
        
        # Expand the group so children are visible
        group_idx = group_header.index()
        mw._active_file_view.expand(group_idx)
        
        # Select first image (a.jpg)
        first_item_idx = group_header.child(0).index()
        mw._active_file_view.setCurrentIndex(first_item_idx)
        
        # Verify initial selection
        cur_idx = mw._active_file_view.currentIndex()
        assert cur_idx.isValid(), "Initial selection should be valid"
        current_item = mw.file_system_model.itemFromIndex(cur_idx)
        assert current_item.text() == "a.jpg", "Should initially select a.jpg"
        
        # Navigate down to next item
        mw.navigate_down_sequential(skip_deleted=True)
        
        # Check that we moved to b.jpg
        new_idx = mw._active_file_view.currentIndex()
        assert new_idx.isValid(), "Selection after navigation should be valid"
        new_item = mw.file_system_model.itemFromIndex(new_idx)
        assert new_item.text() == "b.jpg", "Should navigate to b.jpg"


if __name__ == "__main__":
    test_navigate_down_works()

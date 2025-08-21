import os
from PyQt6.QtCore import Qt
from PyQt6.QtGui import QStandardItemModel, QStandardItem
from PyQt6.QtWidgets import QApplication, QTreeView
from src.ui.main_window import MainWindow

app = QApplication.instance() or QApplication([])


class SmartDownTestWindow(MainWindow):
    def __init__(self):
        # Minimal attributes used by sequential down
        self.file_system_model = QStandardItemModel()
        # Use the source model directly instead of a proxy
        self._active_file_view = QTreeView()
        self._active_file_view.setModel(self.file_system_model)
        # Ensure selection model is present
        if not self._active_file_view.selectionModel():
            self._active_file_view.setSelectionModel(self.file_system_model.selectionModel() or self._active_file_view.selectionModel())
        self.group_by_similarity_mode = True  # grouping not used for sequential down
        # Provide stub methods referenced indirectly
        self.app_state = type("S", (), {})()
        self.app_state.date_cache = {}
        # Patch left_panel to provide get_active_view for MainWindow compatibility
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
        # Since we're not using a proxy, the index is already a source index
        item = self.file_system_model.itemFromIndex(index)
        if not item:
            return False
        d = item.data(Qt.ItemDataRole.UserRole)
        return isinstance(d, dict) and "path" in d

    # NavigationContext protocol methods
    def get_active_view(self):
        return self._active_file_view

    def is_valid_image_index(self, index):
        result = self._is_valid_image_item(index)
        if result:
            # Also print the actual path for debugging
            item = self.file_system_model.itemFromIndex(index)
            if item:
                d = item.data(Qt.ItemDataRole.UserRole)
                if isinstance(d, dict):
                    path = d.get("path")
                    print(f"is_valid_image_index({index}) = {result}, path = {path}")
                    return result
        print(f"is_valid_image_index({index}) = {result}")
        return result

    def map_to_source(self, index):
        # No proxy, so index is already the source index
        print(f"map_to_source({index}) = {index} (no proxy)")
        return index

    def item_from_source(self, source_index):
        result = self.file_system_model.itemFromIndex(source_index)
        print(f"item_from_source({source_index}) = {result.text() if result else 'None'}")
        if result:
            d = result.data(Qt.ItemDataRole.UserRole)
            if isinstance(d, dict):
                current_path = d.get("path")
                print(f"Current path from item: {current_path}")
        return result

    def find_first_visible_item(self):
        # Return first valid image item
        root = self.file_system_model.invisibleRootItem()
        for i in range(root.rowCount()):
            group = root.child(i)
            for j in range(group.rowCount()):
                item = group.child(j)
                d = item.data(Qt.ItemDataRole.UserRole)
                if isinstance(d, dict) and "path" in d:
                    return item.index()  # Return source index directly
        from PyQt6.QtCore import QModelIndex
        return QModelIndex()

    def get_marked_deleted(self):
        return []

    def get_all_visible_image_paths(self):
        """Override to ensure groups are expanded and paths are found."""
        paths = []
        root = self.file_system_model.invisibleRootItem()
        for i in range(root.rowCount()):
            group = root.child(i)
            # Ensure the group is expanded so children are visible
            group_idx = group.index()
            if group_idx.isValid():
                self._active_file_view.expand(group_idx)
            
            for j in range(group.rowCount()):
                item = group.child(j)
                d = item.data(Qt.ItemDataRole.UserRole)
                if isinstance(d, dict) and "path" in d:
                    paths.append(d["path"])
        return paths

    # Adapters (unused for sequential down)
    def get_group_sibling_images(self, current_proxy_index):
        parent = current_proxy_index.parent()
        count = parent.model().rowCount(parent)
        group_indices = [parent.model().index(r, 0, parent) for r in range(count)]
        return parent, group_indices, None

    def validate_and_select_image_candidate(self, index, direction, log_skip):
        print(f"validate_and_select_image_candidate called with index={index}, direction={direction}")
        sel_model = self._active_file_view.selectionModel()
        if sel_model:
            sel_model.select(index, sel_model.SelectionFlag.ClearAndSelect)
        self._active_file_view.setCurrentIndex(index)
        # Debug: Verify the selection was set
        if index.isValid():
            item = self.file_system_model.itemFromIndex(index)
            print(f"Selection set to: {item.text() if item else 'None'}")

    def _navigate_down_sequential(self, skip_deleted: bool = True):
        print(f"_navigate_down_sequential called with skip_deleted={skip_deleted}")
        # Debug the current selection state before calling NavigationController
        cur_idx = self._active_file_view.currentIndex()
        print(f"Current index before navigation: {cur_idx}, valid: {cur_idx.isValid()}")
        if cur_idx.isValid():
            print(f"Current index row: {cur_idx.row()}, parent valid: {cur_idx.parent().isValid()}")
            is_valid = self.is_valid_image_index(cur_idx)
            print(f"is_valid_image_index result: {is_valid}")
        # Call the original method
        return super()._navigate_down_sequential(skip_deleted)
    
    def navigate_down_sequential(self, skip_deleted: bool = True):
        print(f"navigate_down_sequential called with skip_deleted={skip_deleted}")
        return self._navigate_down_sequential(skip_deleted)
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
        print(f"find_proxy_index_for_path called with path: {path}")
        root = self.file_system_model.invisibleRootItem()
        for i in range(root.rowCount()):
            group = root.child(i)
            for j in range(group.rowCount()):
                item = group.child(j)
                d = item.data(Qt.ItemDataRole.UserRole)
                if isinstance(d, dict) and d.get("path") == path:
                    # Return source index directly (no proxy)
                    result = item.index()
                    print(f"Found matching item: {item.text()}, returning index: {result}")
                    return result
        from PyQt6.QtCore import QModelIndex
        print(f"No matching item found for path: {path}")
        return QModelIndex()


# Helper to build a group with n image items


def make_image_item(path: str):
    it = QStandardItem(os.path.basename(path))
    it.setData({"path": path}, Qt.ItemDataRole.UserRole)
    return it


def test_navigate_down_is_sequential(tmp_path):
    mw = SmartDownTestWindow()
    # Create a group header (cluster) and add two image children
    group_header = QStandardItem("Group 1")
    group_header.setData("cluster_header_1", Qt.ItemDataRole.UserRole)
    paths = []
    for name in ["a.jpg", "b.jpg"]:
        p = tmp_path / name
        p.write_text("x")
        paths.append(str(p))
        group_header.appendRow(make_image_item(str(p)))
    mw.file_system_model.appendRow(group_header)

    # Select first image
    first_child_idx = group_header.child(0).index()  # This is a source index
    print(f"Setting current index to source: {first_child_idx}, valid: {first_child_idx.isValid()}")
    mw._active_file_view.setCurrentIndex(first_child_idx)

    # Debug: Check what get_all_visible_image_paths returns
    all_paths = mw.get_all_visible_image_paths()
    print(f"All visible paths: {all_paths}")
    
    # Debug: Test navigate_linear utility directly
    from src.ui.helpers.navigation_utils import navigate_linear
    current_path = str(paths[0])  # Should be a.jpg
    print(f"Current path: {current_path}")
    target = navigate_linear(all_paths, current_path, "down", True, set())
    print(f"navigate_linear result: {target}")

    # Debug: Check current selection before navigation
    cur_idx_before = mw._active_file_view.currentIndex()
    if cur_idx_before.isValid():
        source_idx_before = mw.proxy_model.mapToSource(cur_idx_before)
        if source_idx_before.isValid():
            item_before = mw.file_system_model.itemFromIndex(source_idx_before)
            print(f"Current selection before navigation: {item_before.text() if item_before else 'None'}")

    # Down should be sequential: move to next (b)
    mw.navigate_down_sequential(skip_deleted=True)
    
    # Debug: Check current selection after navigation
    cur_idx_after = mw._active_file_view.currentIndex()
    if cur_idx_after.isValid():
        source_idx_after = mw.proxy_model.mapToSource(cur_idx_after)
        if source_idx_after.isValid():
            item_after = mw.file_system_model.itemFromIndex(source_idx_after)
            print(f"Current selection after navigation: {item_after.text() if item_after else 'None'}")

    # Check selected indexes after navigation
    sel_model = mw._get_active_file_view().selectionModel()
    selected = sel_model.selectedIndexes()
    assert selected, "No index selected after navigation"
    cur_idx = selected[0]
    source_idx = mw.proxy_model.mapToSource(cur_idx)
    assert source_idx.isValid()
    assert mw.file_system_model.itemFromIndex(source_idx).text() == "b.jpg"    # Down again: should not wrap, stays on b or moves to next group parent
    mw.navigate_down_sequential(skip_deleted=True)
    sel_model2 = mw._get_active_file_view().selectionModel()
    selected2 = sel_model2.selectedIndexes()
    assert selected2, "No index selected after second navigation"
    cur_idx2 = selected2[0]
    source_idx2 = mw.proxy_model.mapToSource(cur_idx2)
    assert source_idx2.isValid()
    txt2 = mw.file_system_model.itemFromIndex(source_idx2).text()
    assert txt2 in {"b.jpg", "Group 1"}

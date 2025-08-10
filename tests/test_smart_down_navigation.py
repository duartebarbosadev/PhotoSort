import os
from PyQt6.QtCore import Qt
from PyQt6.QtGui import QStandardItemModel, QStandardItem
from PyQt6.QtWidgets import QApplication, QTreeView
from PyQt6.QtCore import QSortFilterProxyModel
from src.ui.main_window import MainWindow

app = QApplication.instance() or QApplication([])


class IdentityProxy(QSortFilterProxyModel):
    def mapToSource(self, idx):
        return idx

    def mapFromSource(self, source_idx):
        return source_idx


class SmartDownTestWindow(MainWindow):
    def __init__(self):
        # Minimal attributes used by navigate_down_smart
        self.file_system_model = QStandardItemModel()
        self.proxy_model = IdentityProxy()
        self.proxy_model.setSourceModel(self.file_system_model)
        self._active_file_view = QTreeView()
        self._active_file_view.setModel(self.proxy_model)
        self.group_by_similarity_mode = True  # enable group mode logic
        # Provide stub methods referenced indirectly
        self.app_state = type("S", (), {})()
        self.app_state.date_cache = {}

    def _get_active_file_view(self):
        return self._active_file_view

    def _is_valid_image_item(self, proxy_idx):
        item = self.file_system_model.itemFromIndex(proxy_idx)
        if not item:
            return False
        d = item.data(Qt.ItemDataRole.UserRole)
        return isinstance(d, dict) and "path" in d

    # Adapters used in navigate_down_smart
    def get_group_sibling_images(self, current_proxy_index):
        # All siblings under same parent are part of the group
        parent = current_proxy_index.parent()
        count = parent.model().rowCount(parent)
        group_indices = [parent.model().index(r, 0, parent) for r in range(count)]
        return parent, group_indices, None

    def validate_and_select_image_candidate(self, proxy_index, direction, log_skip):
        # Simplified: set current index directly
        self._active_file_view.setCurrentIndex(proxy_index)


# Helper to build a group with n image items


def make_image_item(path: str):
    it = QStandardItem(os.path.basename(path))
    it.setData({"path": path}, Qt.ItemDataRole.UserRole)
    return it


def test_navigate_down_smart_cycles_within_group(tmp_path):
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
    first_child_idx = group_header.child(0).index()
    mw._active_file_view.setCurrentIndex(first_child_idx)

    # Invoke smart down: should move to second
    mw.navigate_down_smart(skip_deleted=True)
    cur_idx = mw._get_active_file_view().currentIndex()
    assert cur_idx.isValid()
    assert mw.file_system_model.itemFromIndex(cur_idx).text() == "b.jpg"

    # Invoke again: should wrap back to first
    mw.navigate_down_smart(skip_deleted=True)
    cur_idx2 = mw._get_active_file_view().currentIndex()
    assert cur_idx2.isValid()
    assert mw.file_system_model.itemFromIndex(cur_idx2).text() == "a.jpg"

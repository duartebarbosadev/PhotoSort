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


class SmartUpTestWindow(MainWindow):
    def __init__(self):
        self.file_system_model = QStandardItemModel()
        self.proxy_model = IdentityProxy()
        self.proxy_model.setSourceModel(self.file_system_model)
        self._active_file_view = QTreeView()
        self._active_file_view.setModel(self.proxy_model)
        self.group_by_similarity_mode = True
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

    def get_group_sibling_images(self, current_proxy_index):
        parent = current_proxy_index.parent()
        count = parent.model().rowCount(parent)
        group_indices = [parent.model().index(r, 0, parent) for r in range(count)]
        return parent, group_indices, None

    def validate_and_select_image_candidate(self, proxy_index, direction, log_skip):
        self._active_file_view.setCurrentIndex(proxy_index)


def make_image_item(path: str):
    it = QStandardItem(os.path.basename(path))
    it.setData({"path": path}, Qt.ItemDataRole.UserRole)
    return it


def test_navigate_up_smart_cycles_reverse(tmp_path):
    mw = SmartUpTestWindow()
    group_header = QStandardItem("Group 1")
    group_header.setData("cluster_header_1", Qt.ItemDataRole.UserRole)
    paths = []
    for name in ["a.jpg", "b.jpg", "c.jpg"]:
        p = tmp_path / name
        p.write_text("x")
        paths.append(str(p))
        group_header.appendRow(make_image_item(str(p)))
    mw.file_system_model.appendRow(group_header)

    # Select middle item b
    mid_idx = group_header.child(1).index()
    mw._active_file_view.setCurrentIndex(mid_idx)

    # Smart up: should move to a
    mw.navigate_up_smart(skip_deleted=True)
    cur_idx = mw._active_file_view.currentIndex()
    assert mw.file_system_model.itemFromIndex(cur_idx).text() == "a.jpg"

    # Smart up again: should wrap to c
    mw.navigate_up_smart(skip_deleted=True)
    cur_idx2 = mw._active_file_view.currentIndex()
    assert mw.file_system_model.itemFromIndex(cur_idx2).text() == "c.jpg"

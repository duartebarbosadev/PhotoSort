import os
from PyQt6.QtCore import Qt
from PyQt6.QtGui import QStandardItemModel, QStandardItem
from PyQt6.QtWidgets import QApplication, QTreeView
from src.ui.main_window import MainWindow
from PyQt6.QtCore import QSortFilterProxyModel
from src.ui.app_state import AppState

# Minimal stubs/mocks may be needed; if QApplication already exists reuse.
app = QApplication.instance() or QApplication([])


class DummyImagePipeline:
    def get_preview_qpixmap(self, *a, **k):
        return None

    def get_thumbnail_qpixmap(self, *a, **k):
        return None


class DummyWorkerManager:
    pass


class DummyAdvancedViewer:
    def set_image_data(self, *a, **k):
        pass

    def clear(self):
        pass


# Because MainWindow expects many attributes, we subclass and override initializer heavy parts.
class IdentityProxy(QSortFilterProxyModel):
    def mapToSource(self, idx):
        return idx

    def mapFromSource(self, source_idx):
        return source_idx

    def rowCount(self, parent):
        src = self.sourceModel()
        if src is None:
            return 0
        return src.rowCount(parent)

    def index(self, row, column, parent):
        src = self.sourceModel()
        if src is None:
            return super().index(row, column, parent)
        return src.index(row, column, parent)


class TestableMainWindow(MainWindow):
    def __init__(self):
        self.app_state = AppState()
        self.file_system_model = QStandardItemModel()
        self.proxy_model = IdentityProxy()
        self.proxy_model.setSourceModel(self.file_system_model)
        tv = QTreeView()
        tv.setModel(self.proxy_model)
        self._active_file_view = tv

    def _get_active_file_view(self):
        return self._active_file_view

    def _is_valid_image_item(self, proxy_idx):
        item = self.file_system_model.itemFromIndex(proxy_idx)
        if not item:
            return False
        d = item.data(Qt.ItemDataRole.UserRole)
        return isinstance(d, dict) and "path" in d


def make_item(path):
    it = QStandardItem(os.path.basename(path))
    it.setData({"path": path}, Qt.ItemDataRole.UserRole)
    return it


def test_find_proxy_index_simple(tmp_path):
    mw = TestableMainWindow()
    # Build simple tree: root items with image data
    p1 = tmp_path / "a.jpg"
    p1.write_text("x")
    p2 = tmp_path / "b.jpg"
    p2.write_text("y")
    mw.file_system_model.appendRow(make_item(str(p1)))
    mw.file_system_model.appendRow(make_item(str(p2)))
    idx = mw._find_proxy_index_for_path(str(p2))
    assert idx.isValid()
    # In real code we map to source; here identity but keep explicit for clarity
    item = mw.file_system_model.itemFromIndex(mw.proxy_model.mapToSource(idx))
    assert item.text() == "b.jpg"


def test_find_proxy_index_not_found(tmp_path):
    mw = TestableMainWindow()
    p1 = tmp_path / "c.jpg"
    p1.write_text("x")
    mw.file_system_model.appendRow(make_item(str(p1)))
    idx = mw._find_proxy_index_for_path(str(tmp_path / "missing.jpg"))
    assert not idx.isValid()
    assert not idx.isValid()

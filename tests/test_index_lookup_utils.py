from PyQt6.QtGui import QStandardItemModel, QStandardItem
from PyQt6.QtCore import Qt, QSortFilterProxyModel
from PyQt6.QtWidgets import QApplication, QTreeView
from src.ui.helpers.index_lookup_utils import (
    find_proxy_index_for_path,
    classify_selection,
)

app = QApplication.instance() or QApplication([])


class IdentityProxy(QSortFilterProxyModel):
    def mapToSource(self, idx):
        return idx

    def mapFromSource(self, sidx):
        return sidx

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


def _is_valid(idx, src_model: QStandardItemModel):
    item = src_model.itemFromIndex(idx)
    if not item:
        return False
    data = item.data(Qt.ItemDataRole.UserRole)
    return isinstance(data, dict) and "path" in data


def make_item(path: str):
    it = QStandardItem(path.split("/")[-1])
    it.setData({"path": path}, Qt.ItemDataRole.UserRole)
    return it


def test_find_proxy_index_basic(tmp_path):
    src = QStandardItemModel()
    proxy = IdentityProxy()
    proxy.setSourceModel(src)
    view = QTreeView()
    view.setModel(proxy)
    p1 = tmp_path / "a.jpg"
    p1.write_text("x")
    p2 = tmp_path / "b.jpg"
    p2.write_text("y")
    src.appendRow(make_item(str(p1)))
    src.appendRow(make_item(str(p2)))
    idx = find_proxy_index_for_path(
        str(p2), proxy, src, lambda i: _is_valid(i, src), view.isExpanded
    )
    assert idx.isValid()


def test_find_proxy_index_missing(tmp_path):
    src = QStandardItemModel()
    proxy = IdentityProxy()
    proxy.setSourceModel(src)
    view = QTreeView()
    view.setModel(proxy)
    p1 = tmp_path / "a.jpg"
    p1.write_text("x")
    src.appendRow(make_item(str(p1)))
    idx = find_proxy_index_for_path(
        str(tmp_path / "missing.jpg"),
        proxy,
        src,
        lambda i: _is_valid(i, src),
        view.isExpanded,
    )
    assert not idx.isValid()


def test_classify_selection():
    assert classify_selection([]) == ("none", 0)
    assert classify_selection(["a"]) == ("single", 1)
    assert classify_selection(["a", "b"]) == ("multi", 2)

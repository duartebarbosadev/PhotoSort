import os
from PyQt6.QtCore import QModelIndex, Qt
from PyQt6.QtGui import QStandardItemModel, QStandardItem

from src.ui.controllers.navigation_controller import NavigationController


class StubView:
    def __init__(self):
        self._current = QModelIndex()

    def currentIndex(self):
        return self._current

    def setCurrentIndex(self, idx: QModelIndex):
        self._current = idx


class Ctx:
    """Minimal NavigationContext for group navigation tests."""

    def __init__(self, model: QStandardItemModel):
        self.model = model
        self.view = StubView()

    # NavigationContext API
    def get_active_view(self):
        return self.view

    def is_valid_image_index(self, proxy_index: QModelIndex) -> bool:
        item = self.model.itemFromIndex(proxy_index)
        if not item:
            return False
        d = item.data(Qt.ItemDataRole.UserRole)
        return isinstance(d, dict) and "path" in d

    def map_to_source(self, proxy_index: QModelIndex) -> QModelIndex:
        return proxy_index

    def item_from_source(self, source_index: QModelIndex):
        return self.model.itemFromIndex(source_index)

    def get_group_sibling_images(self, current_proxy_index: QModelIndex):
        parent = current_proxy_index.parent()
        count = self.model.rowCount(parent)
        group_indices = [self.model.index(r, 0, parent) for r in range(count)]
        return parent, group_indices, None

    def find_first_visible_item(self) -> QModelIndex:
        # Not used in group tests (we always have a group), but implement for completeness
        root = QModelIndex()
        for r in range(self.model.rowCount(root)):
            idx = self.model.index(r, 0, root)
            if self.is_valid_image_index(idx):
                return idx
            # descend into children
            for cr in range(self.model.rowCount(idx)):
                cidx = self.model.index(cr, 0, idx)
                if self.is_valid_image_index(cidx):
                    return cidx
        return QModelIndex()

    def find_proxy_index_for_path(self, path: str) -> QModelIndex:
        # Linear scan through tree (small test models)
        root = QModelIndex()
        for r in range(self.model.rowCount(root)):
            idx = self.model.index(r, 0, root)
            for cr in range(self.model.rowCount(idx)):
                cidx = self.model.index(cr, 0, idx)
                item = self.model.itemFromIndex(cidx)
                if not item:
                    continue
                d = item.data(Qt.ItemDataRole.UserRole)
                if isinstance(d, dict) and d.get("path") == path:
                    return cidx
        return QModelIndex()

    def get_all_visible_image_paths(self):
        # Not used in group navigation tests
        return []

    def get_marked_deleted(self):
        return []

    def validate_and_select_image_candidate(
        self, proxy_index: QModelIndex, direction: str, log_skip: bool
    ):
        self.view.setCurrentIndex(proxy_index)


def make_image_item(path: str):
    it = QStandardItem(os.path.basename(path))
    it.setData({"path": path}, Qt.ItemDataRole.UserRole)
    return it


def test_right_wraps_within_group(tmp_path):
    model = QStandardItemModel()
    header = QStandardItem("Group 1")
    header.setData("cluster_header_1", Qt.ItemDataRole.UserRole)
    a = tmp_path / "a.jpg"
    b = tmp_path / "b.jpg"
    a.write_text("x")
    b.write_text("x")
    header.appendRow(make_image_item(str(a)))
    header.appendRow(make_image_item(str(b)))
    model.appendRow(header)

    ctx = Ctx(model)
    nav = NavigationController(ctx)
    # start at last image in group
    last_idx = header.child(1).index()
    ctx.view.setCurrentIndex(last_idx)

    nav.navigate_group("right", skip_deleted=True)
    cur = ctx.view.currentIndex()
    assert model.itemFromIndex(cur).text() == "a.jpg"


def test_left_wraps_within_group(tmp_path):
    model = QStandardItemModel()
    header = QStandardItem("Group 1")
    header.setData("cluster_header_1", Qt.ItemDataRole.UserRole)
    a = tmp_path / "a.jpg"
    b = tmp_path / "b.jpg"
    c = tmp_path / "c.jpg"
    for p in (a, b, c):
        p.write_text("x")
    header.appendRow(make_image_item(str(a)))
    header.appendRow(make_image_item(str(b)))
    header.appendRow(make_image_item(str(c)))
    model.appendRow(header)

    ctx = Ctx(model)
    nav = NavigationController(ctx)
    # start at first image
    first_idx = header.child(0).index()
    ctx.view.setCurrentIndex(first_idx)

    nav.navigate_group("left", skip_deleted=True)
    cur = ctx.view.currentIndex()
    assert model.itemFromIndex(cur).text() == "c.jpg"


def test_right_does_not_move_to_next_group(tmp_path):
    model = QStandardItemModel()
    g1 = QStandardItem("Group 1")
    g1.setData("cluster_header_1", Qt.ItemDataRole.UserRole)
    a = tmp_path / "a.jpg"
    a.write_text("x")
    b = tmp_path / "b.jpg"
    b.write_text("x")
    g1.appendRow(make_image_item(str(a)))
    g1.appendRow(make_image_item(str(b)))
    g2 = QStandardItem("Group 2")
    g2.setData("cluster_header_2", Qt.ItemDataRole.UserRole)
    c = tmp_path / "c.jpg"
    c.write_text("x")
    d = tmp_path / "d.jpg"
    d.write_text("x")
    g2.appendRow(make_image_item(str(c)))
    g2.appendRow(make_image_item(str(d)))
    model.appendRow(g1)
    model.appendRow(g2)

    ctx = Ctx(model)
    nav = NavigationController(ctx)
    # start at last item of first group
    last_g1 = g1.child(1).index()
    ctx.view.setCurrentIndex(last_g1)

    nav.navigate_group("right", skip_deleted=True)
    cur = ctx.view.currentIndex()
    # Should wrap within group 1, not move to c.jpg in group 2
    assert model.itemFromIndex(cur).text() == "a.jpg"

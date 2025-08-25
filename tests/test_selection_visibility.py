from PyQt6.QtCore import Qt, QModelIndex
from PyQt6.QtGui import QStandardItemModel, QStandardItem
from src.ui.controllers.selection_controller import (
    SelectionController,
    SelectionContext,
)


class StubListView:
    """Lightweight list view stub exposing only model() as used by controller."""

    def __init__(self, model):
        self._model = model

    def model(self):
        return self._model


class StubTreeView:
    """Lightweight tree view stub supporting expansion tracking (duck-typed)."""

    def __init__(self, model):
        self._model = model
        self._expanded = set()

    def model(self):
        return self._model

    # Methods emulating QTreeView API subset
    def expand(self, index: QModelIndex):
        if index.isValid():
            self._expanded.add(
                (index.row(), index.parent().row() if index.parent().isValid() else -1)
            )

    def isExpanded(self, index: QModelIndex) -> bool:  # noqa: N802 (Qt style)
        if not index.isValid():
            return False
        key = (index.row(), index.parent().row() if index.parent().isValid() else -1)
        return key in self._expanded


class ListCtx(SelectionContext):  # type: ignore[misc]
    def __init__(self, model):
        self._model = model
        self._view = StubListView(model)

    def get_active_view(self):
        return self._view

    @property
    def proxy_model(self):  # type: ignore[override]
        return self._model

    def is_valid_image_item(self, proxy_index: QModelIndex) -> bool:
        return proxy_index.isValid()

    def file_system_model_item_from_index(self, source_index: QModelIndex):
        return self._model.itemFromIndex(source_index)

    def map_to_source(self, proxy_index: QModelIndex) -> QModelIndex:
        return proxy_index


class TreeCtx(SelectionContext):  # type: ignore[misc]
    def __init__(self, model, view):
        self._model = model
        self._view = view

    def get_active_view(self):
        return self._view

    @property
    def proxy_model(self):  # type: ignore[override]
        return self._model

    def is_valid_image_item(self, proxy_index: QModelIndex) -> bool:
        return proxy_index.isValid()

    def file_system_model_item_from_index(self, source_index: QModelIndex):
        return self._model.itemFromIndex(source_index)

    def map_to_source(self, proxy_index: QModelIndex) -> QModelIndex:
        return proxy_index


def build_model_with_items(names):
    model = QStandardItemModel()
    for n in names:
        it = QStandardItem(n)
        it.setData({"path": n}, Qt.ItemDataRole.UserRole)
        model.appendRow(it)
    return model


def test_find_first_visible_item_list():
    model = build_model_with_items(["a.jpg", "b.jpg", "c.jpg"])
    ctx = ListCtx(model)
    sc = SelectionController(ctx)
    first = sc.find_first_visible_item()
    assert first.isValid()
    assert model.itemFromIndex(first).text() == "a.jpg"


def test_find_last_visible_item_list():
    model = build_model_with_items(["a.jpg", "b.jpg", "c.jpg"])
    ctx = ListCtx(model)
    sc = SelectionController(ctx)
    last = sc.find_last_visible_item()
    assert last.isValid()
    assert model.itemFromIndex(last).text() == "c.jpg"


def test_find_first_visible_item_tree_expanded():
    # Build simple tree with a top-level that isn't an image (simulate non-image header) and children images
    model = QStandardItemModel()
    header = QStandardItem("Group 1")
    header.setData("cluster_header_1", Qt.ItemDataRole.UserRole)
    child1 = QStandardItem("x.jpg")
    child1.setData({"path": "x.jpg"}, Qt.ItemDataRole.UserRole)
    child2 = QStandardItem("y.jpg")
    child2.setData({"path": "y.jpg"}, Qt.ItemDataRole.UserRole)
    header.appendRow(child1)
    header.appendRow(child2)
    model.appendRow(header)
    view = StubTreeView(model)
    view.expand(model.indexFromItem(header))

    class TreeCtxImages(TreeCtx):  # override validity: only dict with path is image
        def is_valid_image_item(self, proxy_index: QModelIndex) -> bool:
            item = self._model.itemFromIndex(proxy_index)
            if not item:
                return False
            data = item.data(Qt.ItemDataRole.UserRole)
            return isinstance(data, dict) and "path" in data

    ctx = TreeCtxImages(model, view)
    sc = SelectionController(ctx)
    first = sc.find_first_visible_item()
    assert first.isValid()
    assert model.itemFromIndex(first).text() == "x.jpg"


def test_find_last_visible_item_tree_expanded():
    model = QStandardItemModel()
    header = QStandardItem("Group 1")
    header.setData("cluster_header_1", Qt.ItemDataRole.UserRole)
    child1 = QStandardItem("x.jpg")
    child1.setData({"path": "x.jpg"}, Qt.ItemDataRole.UserRole)
    child2 = QStandardItem("y.jpg")
    child2.setData({"path": "y.jpg"}, Qt.ItemDataRole.UserRole)
    header.appendRow(child1)
    header.appendRow(child2)
    model.appendRow(header)
    view = StubTreeView(model)
    view.expand(model.indexFromItem(header))

    class TreeCtxImages(TreeCtx):
        def is_valid_image_item(self, proxy_index: QModelIndex) -> bool:
            item = self._model.itemFromIndex(proxy_index)
            if not item:
                return False
            data = item.data(Qt.ItemDataRole.UserRole)
            return isinstance(data, dict) and "path" in data

    ctx = TreeCtxImages(model, view)
    sc = SelectionController(ctx)
    last = sc.find_last_visible_item()
    assert last.isValid()
    assert model.itemFromIndex(last).text() == "y.jpg"

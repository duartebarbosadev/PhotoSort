from PyQt6.QtCore import QSortFilterProxyModel, Qt
from PyQt6.QtGui import QStandardItemModel, QStandardItem
from PyQt6.QtWidgets import QApplication
import pytest

from src.ui.controllers.filter_controller import FilterController, FilterContext


class DummyCtx(FilterContext):  # type: ignore[misc]
    def __init__(self):
        self.model = QStandardItemModel()
        for name in ["apple.jpg", "banana.png", "cherry.JPG"]:
            item = QStandardItem(name)
            self.model.appendRow(item)
        self.proxy_model = QSortFilterProxyModel()
        self.proxy_model.setSourceModel(self.model)
        self.app_state = object()
        self.refresh_called = 0

    def refresh_filter(self) -> None:
        self.refresh_called += 1


@pytest.fixture(scope="module")
def qapp():
    app = QApplication.instance() or QApplication([])
    return app


def test_filter_controller_search_and_rating_cluster(qapp):
    ctx = DummyCtx()
    fc = FilterController(ctx)
    # initial state
    assert fc.get_rating_filter() == "Show All"
    assert fc.get_cluster_filter_id() == -1
    # apply search
    fc.set_search_text("apple")
    fc.apply_all(show_folders=False, current_view_mode="list")
    # After applying search, proxy should filter to one row
    proxy = ctx.proxy_model
    rows = proxy.rowCount()
    assert rows == 1
    index = proxy.index(0, 0)
    assert proxy.data(index, Qt.ItemDataRole.DisplayRole).lower().startswith("apple")


def test_filter_controller_cluster_set(qapp):
    ctx = DummyCtx()
    fc = FilterController(ctx)
    fc.set_cluster_filter(5)
    assert fc.get_cluster_filter_id() == 5
    # Setting again to same value should not trigger extra refresh
    before = ctx.refresh_called
    fc.set_cluster_filter(5)
    assert ctx.refresh_called == before

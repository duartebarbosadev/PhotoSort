from PyQt6.QtCore import QSortFilterProxyModel
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


@pytest.mark.parametrize(
    "search_text,expected_matches",
    [
        ("apple", 1),
        ("banana", 1),
        ("png", 1),
        ("nonexistent", 0),
        ("", 3),  # Empty search should match all
    ],
)
def test_filter_controller_search_text_filtering(qapp, search_text, expected_matches):
    """Test that search text filtering works correctly for different search terms."""
    ctx = DummyCtx()
    fc = FilterController(ctx)
    # initial state
    assert fc.get_rating_filter() == "Show All"
    assert fc.get_cluster_filter_id() == -1
    # apply search
    fc.set_search_text(search_text)
    fc.apply_all(show_folders=False, current_view_mode="list")
    # Check filtering results
    proxy = ctx.proxy_model
    rows = proxy.rowCount()
    assert rows == expected_matches


def test_filter_controller_initial_state(qapp):
    """Test that filter controller initializes with correct default values."""
    ctx = DummyCtx()
    fc = FilterController(ctx)
    assert fc.get_rating_filter() == "Show All"
    assert fc.get_cluster_filter_id() == -1


def test_filter_controller_cluster_filter_setting(qapp):
    """Test setting cluster filter and verifying it persists."""
    ctx = DummyCtx()
    fc = FilterController(ctx)
    fc.set_cluster_filter(5)
    assert fc.get_cluster_filter_id() == 5


def test_filter_controller_cluster_filter_idempotent_refresh(qapp):
    """Test that setting the same cluster filter multiple times doesn't trigger extra refreshes."""
    ctx = DummyCtx()
    fc = FilterController(ctx)
    fc.set_cluster_filter(5)
    assert fc.get_cluster_filter_id() == 5
    # Setting again to same value should not trigger extra refresh
    before = ctx.refresh_called
    fc.set_cluster_filter(5)
    assert ctx.refresh_called == before

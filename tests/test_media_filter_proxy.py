import pyexiv2  # noqa: F401  # Must be first to avoid Windows crashes

from types import SimpleNamespace

import pytest
from PyQt6.QtCore import Qt
from PyQt6.QtGui import QStandardItem, QStandardItemModel
from PyQt6.QtWidgets import QApplication

from ui.models.media_filter_proxy import (
    MediaFilterProxyModel,
    rating_matches_filter,
)


@pytest.fixture(scope="module")
def qapp():
    return QApplication.instance() or QApplication([])


@pytest.mark.parametrize(
    ("label", "rating", "expected"),
    [
        ("Show All", 0, True),
        ("Exactly 3 Stars", 3, True),
        ("Exactly 3 Stars", 4, False),
        ("3 Stars +", 4, True),
        ("Unknown filter", 1, True),
    ],
)
def test_rating_filter_rules(label, rating, expected):
    assert rating_matches_filter(label, rating) is expected


def test_proxy_combines_search_rating_and_cluster_filters(qapp):
    source = QStandardItemModel()
    for name, path in (("alpha.jpg", "a.jpg"), ("beta.jpg", "b.jpg")):
        item = QStandardItem(name)
        item.setData({"path": path}, Qt.ItemDataRole.UserRole)
        source.appendRow(item)

    proxy = MediaFilterProxyModel()
    proxy.setSourceModel(source)
    proxy.app_state_ref = SimpleNamespace(
        rating_cache={"a.jpg": 5, "b.jpg": 2},
        cluster_results={"a.jpg": 7, "b.jpg": 8},
    )
    proxy.current_rating_filter = "4 Stars +"
    proxy.current_cluster_filter_id = 7
    proxy.setFilterRegularExpression("alpha")

    assert proxy.rowCount() == 1
    assert proxy.index(0, 0).data() == "alpha.jpg"

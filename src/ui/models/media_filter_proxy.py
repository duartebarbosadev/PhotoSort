"""Filtering model for the media list and grid views.

Keeping model behavior out of ``MainWindow`` makes the filtering rules usable and
testable without constructing the complete application window.
"""

from __future__ import annotations

from typing import Protocol

from PyQt6.QtCore import QModelIndex, QSortFilterProxyModel, Qt
from PyQt6.QtGui import QStandardItem


class FilterStateView(Protocol):
    """The small slice of application state needed by the proxy model."""

    rating_cache: dict[str, int]
    cluster_results: dict[str, int]


RatingRule = tuple[str, int]

RATING_FILTER_DEFINITIONS: tuple[tuple[str, RatingRule], ...] = (
    ("Show All", ("all", 0)),
    ("Unrated (0)", ("eq", 0)),
    ("Exactly 1 Star", ("eq", 1)),
    ("1 Star +", ("ge", 1)),
    ("Exactly 2 Stars", ("eq", 2)),
    ("2 Stars +", ("ge", 2)),
    ("Exactly 3 Stars", ("eq", 3)),
    ("3 Stars +", ("ge", 3)),
    ("Exactly 4 Stars", ("eq", 4)),
    ("4 Stars +", ("ge", 4)),
    ("5 Stars", ("eq", 5)),
)
RATING_FILTER_RULES = dict(RATING_FILTER_DEFINITIONS)
RATING_FILTER_OPTIONS = [label for label, _rule in RATING_FILTER_DEFINITIONS]


def rating_matches_filter(filter_label: str, current_rating: int) -> bool:
    """Return whether a rating satisfies a named UI filter."""

    mode, threshold = RATING_FILTER_RULES.get(filter_label, ("all", 0))
    if mode == "eq":
        return current_rating == threshold
    if mode == "ge":
        return current_rating >= threshold
    return True


class MediaFilterProxyModel(QSortFilterProxyModel):
    """Filter media by filename, rating, and similarity cluster."""

    def __init__(self, parent=None):
        super().__init__(parent)
        self.current_rating_filter = "Show All"
        self.current_cluster_filter_id = -1
        self.app_state_ref: FilterStateView | None = None
        self.show_folders_mode_ref = False
        self.current_view_mode_ref = "list"

    def _check_item_passes_filter(self, item: QStandardItem) -> bool:
        item_data = item.data(Qt.ItemDataRole.UserRole)
        if not isinstance(item_data, dict) or "path" not in item_data:
            return False

        search_text = self.filterRegularExpression().pattern().lower()
        if search_text not in item.text().lower():
            return False

        state = self.app_state_ref
        if state is None:
            return True

        file_path = item_data["path"]
        current_rating = state.rating_cache.get(file_path, 0)
        if not rating_matches_filter(self.current_rating_filter, current_rating):
            return False

        cluster_id = self.current_cluster_filter_id
        return cluster_id == -1 or state.cluster_results.get(file_path) == cluster_id

    def filterAcceptsRow(self, source_row: int, source_parent: QModelIndex) -> bool:
        source_model = self.sourceModel()
        if source_model is None:
            return False

        source_index = source_model.index(source_row, 0, source_parent)
        if not source_index.isValid():
            return False

        item = source_model.itemFromIndex(source_index)
        if item is None:
            return False
        if self._check_item_passes_filter(item):
            return True

        return any(
            self.filterAcceptsRow(child_row, source_index)
            for child_row in range(item.rowCount())
        )


# Compatibility name retained for plugins and existing imports.
CustomFilterProxyModel = MediaFilterProxyModel

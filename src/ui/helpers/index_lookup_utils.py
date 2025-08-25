from __future__ import annotations

from typing import Callable, List, Optional
from PyQt6.QtCore import QModelIndex, Qt, QSortFilterProxyModel
from PyQt6.QtGui import QStandardItemModel


def find_proxy_index_for_path(
    target_path: str,
    proxy_model: QSortFilterProxyModel,
    source_model: QStandardItemModel,
    is_valid_image_item: Callable[[QModelIndex], bool],
    is_expanded: Optional[Callable[[QModelIndex], bool]] = None,
) -> QModelIndex:
    """Pure traversal of proxy model to locate index for a path.

    Parameters:
        target_path: file path to find.
        proxy_model: the active proxy model (must map to source_model).
        source_model: underlying source model containing QStandardItems with UserRole data dict including 'path'.
        is_valid_image_item: predicate to determine if an index points to an image item.
        is_expanded: optional function telling if a given proxy index is expanded (for tree traversal). When None, assumes expanded.
    Returns:
        QModelIndex for the proxy model if found else invalid QModelIndex().
    """
    if not isinstance(proxy_model, QSortFilterProxyModel):  # Safety
        return QModelIndex()

    queue: List[QModelIndex] = []
    root_parent = QModelIndex()
    for r in range(proxy_model.rowCount(root_parent)):
        queue.append(proxy_model.index(r, 0, root_parent))

    head = 0
    while head < len(queue):
        current_proxy_idx = queue[head]
        head += 1
        if not current_proxy_idx.isValid():
            continue
        if is_valid_image_item(current_proxy_idx):
            source_idx = proxy_model.mapToSource(current_proxy_idx)
            item = source_model.itemFromIndex(source_idx)
            if item:
                data = item.data(Qt.ItemDataRole.UserRole)
                if isinstance(data, dict) and data.get("path") == target_path:
                    return current_proxy_idx
        # traverse children if tree-like
        if is_expanded is None or is_expanded(current_proxy_idx):
            for child_row in range(proxy_model.rowCount(current_proxy_idx)):
                queue.append(proxy_model.index(child_row, 0, current_proxy_idx))
    return QModelIndex()


def classify_selection(selected_paths: List[str]):
    """Return a simple classification tuple for selection state.

    ('none' | 'single' | 'multi', count)
    """
    if not selected_paths:
        return ("none", 0)
    if len(selected_paths) == 1:
        return ("single", 1)
    return ("multi", len(selected_paths))

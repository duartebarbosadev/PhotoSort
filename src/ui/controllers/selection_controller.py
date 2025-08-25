from __future__ import annotations
from typing import List, Protocol, Any
import os
from PyQt6.QtCore import QModelIndex
from PyQt6.QtGui import QStandardItem
from PyQt6.QtCore import Qt


class SelectionContext(Protocol):
    """Protocol the SelectionController depends on.

    Relaxed for duck-typing: any model/view pair exposing the minimal methods
    used by the controller is acceptable (e.g. test stubs without QWidget).
    """

    def get_active_view(self) -> Any | None: ...  # view must expose model()
    @property
    def proxy_model(self) -> Any: ...  # kept for backward compatibility
    def is_valid_image_item(self, proxy_index: QModelIndex) -> bool: ...
    def file_system_model_item_from_index(
        self, source_index: QModelIndex
    ) -> QStandardItem | None: ...
    def map_to_source(self, proxy_index: QModelIndex) -> QModelIndex: ...


class SelectionController:
    """Encapsulates selection gathering logic from MainWindow."""

    def __init__(self, ctx: SelectionContext):
        self.ctx = ctx

    def get_selected_file_paths(self) -> List[str]:
        view = self.ctx.get_active_view()
        if not view:
            return []
        sel_model = view.selectionModel()
        if not sel_model:
            return []
        selected_indexes = sel_model.selectedIndexes()
        out: List[str] = []
        for proxy_index in selected_indexes:
            if proxy_index.column() != 0:
                continue
            source_index = self.ctx.map_to_source(proxy_index)
            if not source_index.isValid():
                continue
            item = self.ctx.file_system_model_item_from_index(source_index)
            if not item:
                continue
            item_user_data = item.data(Qt.ItemDataRole.UserRole)
            if isinstance(item_user_data, dict) and "path" in item_user_data:
                file_path = item_user_data["path"]
                if os.path.isfile(file_path) and file_path not in out:
                    out.append(file_path)
        return out

    # --- Visibility scanning helpers (migrated from MainWindow) ---
    def find_first_visible_item(self) -> QModelIndex:
        """Return first visible, valid image index or invalid if none.

        Tree vs flat detection is duck-typed via presence of 'isExpanded'.
        """
        from PyQt6.QtCore import QModelIndex

        view = self.ctx.get_active_view()
        if view is None:
            return QModelIndex()
        # view must provide model(); tolerate AttributeError
        try:
            model = view.model()
        except Exception:
            return QModelIndex()
        if model is None:
            return QModelIndex()
        root_idx = QModelIndex()
        row_count = model.rowCount(root_idx)

        is_tree = hasattr(view, "isExpanded")
        if is_tree:
            queue = [model.index(r, 0, root_idx) for r in range(row_count)]
            head = 0
            while head < len(queue):
                idx = queue[head]
                head += 1
                if not idx.isValid():
                    continue
                try:
                    # Optional row hidden support
                    if hasattr(view, "isRowHidden") and view.isRowHidden(
                        idx.row(), idx.parent()
                    ):  # type: ignore[attr-defined]
                        continue
                except Exception:
                    pass
                if self.ctx.is_valid_image_item(idx):
                    return idx
                try:
                    if getattr(view, "isExpanded", lambda _i: False)(
                        idx
                    ) and model.hasChildren(idx):
                        for child_row in range(model.rowCount(idx)):
                            queue.append(model.index(child_row, 0, idx))
                except Exception:
                    pass
            return QModelIndex()
        # Flat scan
        for r in range(row_count):
            idx = model.index(r, 0, root_idx)
            if self.ctx.is_valid_image_item(idx):
                return idx
        return QModelIndex()

    def find_last_visible_item(self) -> QModelIndex:
        """Return last visible, valid image index or invalid if none."""
        from PyQt6.QtCore import QModelIndex

        view = self.ctx.get_active_view()
        if view is None:
            return QModelIndex()
        try:
            model = view.model()
        except Exception:
            return QModelIndex()
        if model is None:
            return QModelIndex()
        root_idx = QModelIndex()
        row_count = model.rowCount(root_idx)

        is_tree = hasattr(view, "isExpanded")
        if is_tree:
            last_valid = QModelIndex()
            queue = [model.index(r, 0, root_idx) for r in range(row_count)]
            head = 0
            while head < len(queue):
                idx = queue[head]
                head += 1
                if not idx.isValid():
                    continue
                try:
                    if hasattr(view, "isRowHidden") and view.isRowHidden(
                        idx.row(), idx.parent()
                    ):  # type: ignore[attr-defined]
                        continue
                except Exception:
                    pass
                if self.ctx.is_valid_image_item(idx):
                    last_valid = idx
                try:
                    if getattr(view, "isExpanded", lambda _i: False)(
                        idx
                    ) and model.hasChildren(idx):
                        for child_row in range(model.rowCount(idx)):
                            queue.append(model.index(child_row, 0, idx))
                except Exception:
                    pass
            return last_valid
        for r in range(row_count - 1, -1, -1):
            idx = model.index(r, 0, root_idx)
            if self.ctx.is_valid_image_item(idx):
                return idx
        return QModelIndex()

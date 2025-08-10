from __future__ import annotations
from typing import List, Protocol
import os
from PyQt6.QtCore import QModelIndex
from PyQt6.QtWidgets import QAbstractItemView
from PyQt6.QtGui import QStandardItem
from PyQt6.QtCore import Qt
from PyQt6.QtCore import QSortFilterProxyModel


class SelectionContext(Protocol):
    def get_active_view(self) -> QAbstractItemView | None: ...
    @property
    def proxy_model(self) -> QSortFilterProxyModel: ...  # type: ignore[name-defined]
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

import os
from typing import List
from PyQt6.QtCore import QModelIndex, Qt
from PyQt6.QtGui import QStandardItemModel, QStandardItem
from PyQt6.QtWidgets import QTreeView, QApplication
from src.ui.controllers.selection_controller import (
    SelectionController,
    SelectionContext,
)


class DummyView(QTreeView):
    def __init__(self, model):
        super().__init__()
        self.setModel(model)


class DummyCtx(SelectionContext):  # type: ignore[misc]
    def __init__(self, model):
        self._model = model
        self._view = DummyView(model)

    def get_active_view(self):
        return self._view

    @property
    def proxy_model(self):  # minimal proxy interface usage in controller
        return self._model  # not actually a proxy; map_to_source is identity

    def is_valid_image_item(self, proxy_index: QModelIndex) -> bool:
        return True

    def file_system_model_item_from_index(self, source_index: QModelIndex):
        return self._model.itemFromIndex(source_index)

    def map_to_source(self, proxy_index: QModelIndex) -> QModelIndex:
        return proxy_index


def build_model_with_files(file_paths: List[str]):
    model = QStandardItemModel()
    for p in file_paths:
        item = QStandardItem(os.path.basename(p))
        item.setData({"path": p}, Qt.ItemDataRole.UserRole)
        model.appendRow(item)
    return model


def test_selection_controller_returns_selected_paths(tmp_path):
    # Ensure QApplication exists
    QApplication.instance() or QApplication([])
    # Arrange
    files = [tmp_path / f"img_{i}.jpg" for i in range(3)]
    for fp in files:
        fp.write_text("x")
    paths = [str(f) for f in files]
    model = build_model_with_files(paths)
    ctx = DummyCtx(model)
    controller = SelectionController(ctx)

    # Simulate selecting first and third
    sel_model = ctx.get_active_view().selectionModel()
    first_idx = model.index(0, 0)
    third_idx = model.index(2, 0)
    sel_model.select(
        first_idx, sel_model.SelectionFlag.Select | sel_model.SelectionFlag.Rows
    )
    sel_model.select(
        third_idx, sel_model.SelectionFlag.Select | sel_model.SelectionFlag.Rows
    )

    # Act
    result = controller.get_selected_file_paths()

    # Assert
    assert set(result) == {paths[0], paths[2]}


def test_selection_controller_empty_when_no_view():
    QApplication.instance() or QApplication([])
    model = build_model_with_files([])
    ctx = DummyCtx(model)
    controller = SelectionController(ctx)
    # Replace active view with None to simulate missing view
    ctx._view = None  # type: ignore
    assert controller.get_selected_file_paths() == []

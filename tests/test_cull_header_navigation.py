from types import SimpleNamespace

from PyQt6.QtCore import Qt
from PyQt6.QtGui import QStandardItem, QStandardItemModel
from PyQt6.QtWidgets import QApplication, QTreeView

from src.ui.main_window import MainWindow


app = QApplication.instance() or QApplication([])


def _image_item(path: str) -> QStandardItem:
    item = QStandardItem(path)
    item.setData({"path": path}, Qt.ItemDataRole.UserRole)
    return item


def test_down_stops_on_header_before_entering_next_group():
    model = QStandardItemModel()
    first_group = QStandardItem("Group 1")
    first_group.setData("cluster_header_1", Qt.ItemDataRole.UserRole)
    first_group.appendRow(_image_item("a.jpg"))
    second_group = QStandardItem("Group 2")
    second_group.setData("cluster_header_2", Qt.ItemDataRole.UserRole)
    second_group.appendRow(_image_item("b.jpg"))
    second_group.appendRow(_image_item("c.jpg"))
    third_group = QStandardItem("Group 3")
    third_group.setData("cluster_header_3", Qt.ItemDataRole.UserRole)
    third_group.appendRow(_image_item("d.jpg"))
    model.appendRow(first_group)
    model.appendRow(second_group)
    model.appendRow(third_group)

    view = QTreeView()
    view.setModel(model)
    view.expandAll()

    def is_image(index):
        item = model.itemFromIndex(index)
        return isinstance(item.data(Qt.ItemDataRole.UserRole), dict) if item else False

    def select_image(index, _direction, _skip_deleted):
        view.setCurrentIndex(index)
        return True

    context = SimpleNamespace(
        _get_active_file_view=lambda: view,
        _is_valid_image_item=is_image,
        _validate_and_select_image_candidate=select_image,
    )
    context._select_group_and_children = lambda index: (
        MainWindow._select_group_and_children(context, index)
    )

    assert context._select_group_and_children(first_group.index())
    assert view.currentIndex() == first_group.child(0).index()
    assert view.selectionModel().selectedIndexes() == [first_group.child(0).index()]

    assert MainWindow._navigate_across_tree_header(context, "down", True)
    assert view.currentIndex() == second_group.index()
    assert view.selectionModel().selectedIndexes() == [
        second_group.index(),
        second_group.child(0).index(),
        second_group.child(1).index(),
    ]

    assert MainWindow._navigate_across_tree_header(context, "down", True)
    assert view.currentIndex() == second_group.child(0).index()

    view.setCurrentIndex(second_group.child(1).index())
    assert MainWindow._navigate_across_tree_header(context, "down", True)
    assert view.currentIndex() == third_group.child(0).index()

    assert MainWindow._navigate_across_tree_header(context, "up", True)
    assert view.currentIndex() == second_group.child(1).index()

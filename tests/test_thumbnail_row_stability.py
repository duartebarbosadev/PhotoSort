from types import SimpleNamespace
from unittest.mock import Mock

import pytest
from PyQt6.QtCore import QSize, Qt
from PyQt6.QtGui import QIcon, QPixmap, QStandardItem, QStandardItemModel
from PyQt6.QtWidgets import QApplication, QTreeView

from src.ui.ui_components import FocusHighlightDelegate
from src.ui.main_window import MainWindow


_app = QApplication.instance() or QApplication([])


@pytest.mark.parametrize("icon_height", [16, 64, 96])
def test_thumbnail_arrival_does_not_change_left_panel_row_height(icon_height):
    model = QStandardItemModel()
    item = QStandardItem("example.ARW")
    model.appendRow(item)

    view = QTreeView()
    view.setModel(model)
    view.setIconSize(QSize(icon_height, icon_height))
    app_state = SimpleNamespace(focused_image_path=None)
    main_window = SimpleNamespace(_get_active_file_view=lambda: view)
    view.setItemDelegate(FocusHighlightDelegate(app_state, main_window, view))
    view.resize(320, 240)
    view.show()
    _app.processEvents()

    index = model.index(0, 0)
    height_before = view.sizeHintForIndex(index).height()
    thumbnail = QPixmap(170, 256)
    thumbnail.fill(Qt.GlobalColor.red)
    item.setIcon(QIcon(thumbnail))
    _app.processEvents()

    assert height_before == icon_height + 4
    assert view.sizeHintForIndex(index).height() == height_before
    view.deleteLater()


def test_cull_row_construction_does_not_read_thumbnail_cache():
    image_pipeline = SimpleNamespace(get_cached_thumbnail_qpixmap=Mock())
    pixmap = QPixmap(120, 80)
    pixmap.fill(Qt.GlobalColor.red)
    cached_icon = QIcon(pixmap)
    window = SimpleNamespace(
        _file_items_by_path={},
        image_pipeline=image_pipeline,
        get_cached_thumbnail_icon=Mock(return_value=cached_icon),
        deletion_controller=SimpleNamespace(apply_presentation=Mock()),
        _decorate_best_shot_item=Mock(),
    )

    item = MainWindow._create_standard_item(
        window,
        {"path": "/tmp/example.ARW", "media_type": "image"},
    )

    assert item.text() == "example.ARW"
    assert not item.icon().isNull()
    window.get_cached_thumbnail_icon.assert_called_once_with("/tmp/example.ARW")
    image_pipeline.get_cached_thumbnail_qpixmap.assert_not_called()

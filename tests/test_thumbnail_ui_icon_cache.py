import os
from types import SimpleNamespace
from unittest.mock import Mock

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from PyQt6.QtGui import QPixmap, QStandardItem, QStandardItemModel
from PyQt6.QtWidgets import QApplication

from ui.main_window import MainWindow


_app = QApplication.instance() or QApplication([])


def test_completed_thumbnail_is_cached_for_models_not_built_yet():
    path = "/tmp/future-model.jpg"
    pixmap = QPixmap(256, 128)
    pixmap.fill()
    pipeline = Mock()
    pipeline.get_cached_thumbnail_qpixmap.return_value = pixmap
    action = Mock()
    action.isChecked.return_value = True
    window = SimpleNamespace(
        menu_manager=SimpleNamespace(toggle_thumbnails_action=action),
        image_pipeline=pipeline,
        _file_items_by_path={},
        _thumbnail_icons_by_path={},
    )

    MainWindow._update_thumbnails_from_cache(window, [path])

    icon = window._thumbnail_icons_by_path[os.path.normpath(path)]
    assert not icon.isNull()
    assert icon.availableSizes()[0].width() <= 120
    assert icon.availableSizes()[0].height() <= 120


def test_thumbnail_callback_tolerates_item_deleted_during_folder_change():
    path = "/tmp/old-folder.jpg"
    pixmap = QPixmap(120, 80)
    pixmap.fill()
    pipeline = Mock()
    pipeline.get_cached_thumbnail_qpixmap.return_value = pixmap
    action = Mock()
    action.isChecked.return_value = True
    model = QStandardItemModel()
    stale_item = QStandardItem("old-folder.jpg")
    model.appendRow(stale_item)
    window = SimpleNamespace(
        menu_manager=SimpleNamespace(toggle_thumbnails_action=action),
        image_pipeline=pipeline,
        _file_items_by_path={os.path.normpath(path): stale_item},
        _thumbnail_icons_by_path={},
    )
    model.clear()

    MainWindow._update_thumbnails_from_cache(window, [path])

    assert os.path.normpath(path) in window._thumbnail_icons_by_path
    assert os.path.normpath(path) not in window._file_items_by_path

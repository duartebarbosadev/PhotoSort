import os
from types import SimpleNamespace

from PyQt6.QtWidgets import QApplication

import ui.metadata_sidebar as metadata_sidebar_module
from ui.metadata_sidebar import MetadataSidebar


app = QApplication.instance() or QApplication([])


def test_sidebar_rendering_uses_cached_file_data_without_stat(monkeypatch):
    def unexpected_stat(_path):
        raise AssertionError("metadata rendering must not stat selected files")

    monkeypatch.setattr(
        metadata_sidebar_module,
        "os",
        SimpleNamespace(path=os.path, stat=unexpected_stat),
    )
    sidebar = MetadataSidebar()
    assert sidebar.update_timer.parent() is sidebar
    sidebar.update_metadata(
        "/slow-volume/photo.jpg",
        {
            "file_size": 2048,
            "mtime_ns": 1_700_000_000_000_000_000,
            "rating": 3,
        },
        {"Exif.Image.Model": "Camera"},
    )

    sidebar._delayed_update()

    assert sidebar.raw_metadata["file_size"] == 2048
    assert sidebar.raw_metadata["Exif.Image.Model"] == "Camera"
    sidebar.update_timer.stop()
    sidebar.deleteLater()

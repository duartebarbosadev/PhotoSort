from PyQt6.QtWidgets import QApplication
from PyQt6.QtGui import QPixmap
from PyQt6.QtCore import Qt

from src.ui.advanced_image_viewer import SynchronizedImageViewer


# Ensure a QApplication exists for widget tests.
_app = QApplication.instance() or QApplication([])


def _make_image(path: str, size: int = 16):
    pixmap = QPixmap(size, size)
    pixmap.fill(Qt.GlobalColor.red)
    return {"pixmap": pixmap, "path": path, "rating": 0}


def test_viewer_pool_shrinks_after_single_selection():
    viewer = SynchronizedImageViewer()

    images = [_make_image(f"img_{idx}.jpg") for idx in range(5)]
    viewer.set_images_data(images)

    assert len(viewer.image_viewers) == len(images)

    viewer.set_image_data(images[0])

    assert len(viewer.image_viewers) == 1
    assert viewer.image_viewers[0].get_file_path() == images[0]["path"]

    viewer.deleteLater()


def test_viewer_pool_matches_multi_selection_size():
    viewer = SynchronizedImageViewer()

    images = [_make_image(f"multi_{idx}.jpg") for idx in range(4)]

    viewer.set_images_data(images)
    assert len(viewer.image_viewers) == 4

    reduced_images = images[:2]
    viewer.set_images_data(reduced_images)
    assert len(viewer.image_viewers) == 2

    viewer.set_image_data(reduced_images[0])
    assert len(viewer.image_viewers) == 1

    viewer.deleteLater()

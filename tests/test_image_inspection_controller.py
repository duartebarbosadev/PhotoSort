from PIL import Image
from PyQt6.QtCore import QObject, pyqtSignal
from PyQt6.QtGui import QPixmap
from PyQt6.QtWidgets import QApplication

from ui.advanced_image_viewer import SynchronizedImageViewer
from ui.controllers.image_inspection_controller import (
    ImageInspectionController,
    InspectionImageSpec,
    InspectionQuality,
)

_app = QApplication.instance() or QApplication([])


class _Loader(QObject):
    preview_ready = pyqtSignal(str)
    preview_failed = pyqtSignal(str)
    detail_ready = pyqtSignal(str, object)
    detail_failed = pyqtSignal(str)
    detail_batch_finished = pyqtSignal()

    def __init__(self):
        super().__init__()
        self.preview_requests = []
        self.detail_requests = []
        self.cancel_details_count = 0

    def request(self, paths, **options):
        self.preview_requests.append((tuple(paths), options))

    def request_details(self, paths):
        self.detail_requests.append(tuple(paths))

    def cancel_details(self):
        self.cancel_details_count += 1

    def reset(self):
        self.cancel_details()


class _Pipeline:
    def __init__(self):
        self.immediate_calls = []
        self.preview = self._pixmap(1920, 1200)

    @staticmethod
    def _pixmap(width, height):
        pixmap = QPixmap(width, height)
        pixmap.fill()
        return pixmap

    def get_immediate_review_qpixmap(self, path):
        self.immediate_calls.append(path)
        return self._pixmap(160, 100), False

    def get_cached_preview_qpixmap(self, _path, **_options):
        return self.preview

    def qpixmap_from_pil(self, image):
        return self._pixmap(image.width, image.height)


def _make_controller():
    pipeline = _Pipeline()
    loader = _Loader()
    controller = ImageInspectionController(pipeline, loader)
    viewer = SynchronizedImageViewer()
    viewer.configure_toolbar(show_view_modes=False)
    return controller, loader, pipeline, viewer


def test_dwell_requests_unique_non_video_paths_once():
    controller, loader, pipeline, viewer = _make_controller()
    controller.activate(
        viewer,
        [
            InspectionImageSpec("same.jpg"),
            InspectionImageSpec("same.jpg", rotation_degrees=90),
            InspectionImageSpec("clip.mp4", media_type="video"),
        ],
    )

    assert pipeline.immediate_calls == ["same.jpg"]
    assert loader.preview_requests[-1][0] == ("same.jpg",)
    controller._timer.timeout.emit()
    controller._timer.timeout.emit()
    assert loader.detail_requests == [("same.jpg",)]


def test_navigation_cancels_dwell_and_stale_results():
    controller, loader, _pipeline, viewer = _make_controller()
    controller.activate(viewer, [InspectionImageSpec("old.jpg")])
    controller.activate(viewer, [InspectionImageSpec("new.jpg")])

    loader.detail_ready.emit("old.jpg", Image.new("RGB", (4000, 3000)))
    assert not viewer.displays_path("old.jpg")
    assert controller.active_paths == ("new.jpg",)
    controller._timer.timeout.emit()
    assert loader.detail_requests == [("new.jpg",)]


def test_zoom_and_actual_size_deduplicate_and_defer_one_to_one():
    controller, loader, _pipeline, viewer = _make_controller()
    viewer.show()
    QApplication.processEvents()
    controller.activate(viewer, [InspectionImageSpec("photo.jpg")])
    primary = viewer.get_primary_viewer()
    assert primary is not None

    viewer.detail_requested.emit("zoom")
    viewer.detail_requested.emit("actual_size")
    assert loader.detail_requests == [("photo.jpg",)]

    loader.detail_ready.emit("photo.jpg", Image.new("RGB", (4000, 3000)))
    loader.detail_batch_finished.emit()
    assert primary.image_view.get_zoom_factor() == 1.0


def test_detail_is_monotonic_and_must_add_pixels():
    controller, loader, pipeline, viewer = _make_controller()
    controller.activate(viewer, [InspectionImageSpec("photo.jpg")])
    loader.preview_ready.emit("photo.jpg")
    assert controller._quality["photo.jpg"] == InspectionQuality.PREVIEW

    loader.detail_ready.emit("photo.jpg", Image.new("RGB", (640, 480)))
    assert controller._quality["photo.jpg"] == InspectionQuality.PREVIEW

    loader.detail_ready.emit("photo.jpg", Image.new("RGB", (4000, 3000)))
    assert controller._quality["photo.jpg"] == InspectionQuality.DETAIL
    detail_size = viewer.current_pixmap().size()

    pipeline.preview = pipeline._pixmap(1920, 1200)
    loader.preview_ready.emit("photo.jpg")
    assert viewer.current_pixmap().size() == detail_size


def test_duplicate_source_slots_keep_independent_rotation_on_upgrade():
    controller, loader, _pipeline, viewer = _make_controller()
    controller.activate(
        viewer,
        [
            InspectionImageSpec("photo.jpg"),
            InspectionImageSpec("photo.jpg", rotation_degrees=90),
        ],
    )
    loader.detail_ready.emit("photo.jpg", Image.new("RGB", (4000, 3000)))

    left = viewer.image_viewers[0].get_current_pixmap()
    right = viewer.image_viewers[1].get_current_pixmap()
    assert (left.width(), left.height()) == (4000, 3000)
    assert (right.width(), right.height()) == (3000, 4000)

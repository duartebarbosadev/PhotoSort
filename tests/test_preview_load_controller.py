from core.app_settings import NAVIGATION_PREVIEW_LOOKAHEAD
from ui.controllers.preview_load_controller import PreviewLoadController
from PIL import Image


class _Pipeline:
    def __init__(self):
        self.generated = []
        self.options = []

    def ensure_preview_cached(self, path, **options):
        self.generated.append(path)
        self.options.append(options)
        return True

    def get_source_dimensions(self, _path):
        return (100, 100)

    def load_detail_image(self, path, _target):
        self.generated.append(path)
        return Image.new("RGBA", (100, 100))


class _Pool:
    def __init__(self):
        self.started = []
        self.clear_count = 0

    def start(self, worker):
        self.started.append(worker)

    def clear(self):
        self.clear_count += 1

    def waitForDone(self, _timeout):
        return True


def test_same_primary_request_keeps_existing_lookahead_work():
    pipeline = _Pipeline()
    controller = PreviewLoadController(pipeline)
    pool = _Pool()
    controller._pool = pool

    paths = [f"image-{index}.jpg" for index in range(NAVIGATION_PREVIEW_LOOKAHEAD + 1)]
    controller.request(paths)
    controller.request([paths[0]])

    assert len(pool.started) == 1
    pool.started[0].run()
    assert pipeline.generated == paths


def test_secondary_pair_focus_does_not_cancel_existing_pair_request():
    pipeline = _Pipeline()
    controller = PreviewLoadController(pipeline)
    pool = _Pool()
    controller._pool = pool

    controller.request(["left.jpg", "right.jpg"])
    controller.request(["right.jpg"])

    assert len(pool.started) == 1
    pool.started[0].run()
    assert pipeline.generated == ["left.jpg", "right.jpg"]


def test_new_selection_cancels_stale_work_and_only_emits_latest_result():
    pipeline = _Pipeline()
    controller = PreviewLoadController(pipeline)
    pool = _Pool()
    controller._pool = pool
    ready = []
    controller.preview_ready.connect(ready.append)

    controller.request(["old.jpg", "old-next.jpg"])
    old_worker = pool.started[-1]
    controller.request(["current.jpg"])
    current_worker = pool.started[-1]

    old_worker.run()
    current_worker.run()

    assert pipeline.generated == ["current.jpg"]
    assert ready == ["current.jpg"]
    assert pool.clear_count >= 2


def test_force_default_brightness_is_part_of_request_identity_and_reaches_worker():
    pipeline = _Pipeline()
    controller = PreviewLoadController(pipeline)
    pool = _Pool()
    controller._pool = pool

    controller.request(["rotated.arw"])
    controller.request(["rotated.arw"], force_default_brightness=True)

    assert len(pool.started) == 2
    pool.started[-1].run()
    assert pipeline.generated == ["rotated.arw"]
    assert pipeline.options == [{"force_default_brightness": True}]


def test_new_detail_set_cancels_stale_results():
    pipeline = _Pipeline()
    controller = PreviewLoadController(pipeline)
    pool = _Pool()
    controller._detail_pool = pool
    ready = []
    controller.detail_ready.connect(lambda path, _image: ready.append(path))

    controller.request_details(["old-left.jpg", "old-right.jpg"])
    old_worker = pool.started[-1]
    controller.request_details(["current-left.jpg", "current-right.jpg"])
    current_worker = pool.started[-1]

    old_worker.run()
    current_worker.run()

    assert set(pipeline.generated) == {"current-left.jpg", "current-right.jpg"}
    assert set(ready) == {"current-left.jpg", "current-right.jpg"}

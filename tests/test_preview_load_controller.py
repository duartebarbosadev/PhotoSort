from core.app_settings import NAVIGATION_PREVIEW_LOOKAHEAD
from ui.controllers.preview_load_controller import PreviewLoadController
from PIL import Image


class _Pipeline:
    def __init__(self):
        self.generated = []

    def ensure_preview_cached(self, path):
        self.generated.append(path)
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

    def activeThreadCount(self):
        return 0


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


def test_explicit_cancel_restarts_same_path_request_after_source_change():
    pipeline = _Pipeline()
    controller = PreviewLoadController(pipeline)
    pool = _Pool()
    controller._pool = pool

    controller.request(["rotated.arw"])
    old_worker = pool.started[-1]
    controller.cancel()
    controller.request(["rotated.arw"])
    current_worker = pool.started[-1]

    assert len(pool.started) == 2
    old_worker.run()
    current_worker.run()
    assert pipeline.generated == ["rotated.arw"]


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


def test_reset_discards_callbacks_already_queued_by_old_preview():
    controller = PreviewLoadController(_Pipeline())
    controller._pool = pool = _Pool()
    ready, failed = [], []
    controller.preview_ready.connect(ready.append)
    controller.preview_failed.connect(failed.append)
    controller.request(["old.jpg"])
    old_id = pool.started[-1].request_id

    controller.reset()
    controller._handle_preview_ready("old.jpg", old_id)
    controller._handle_preview_failed("old.jpg", old_id)

    assert ready == failed == []


def test_shutdown_never_waits_and_refuses_new_preview_work():
    from unittest.mock import Mock

    controller = PreviewLoadController(_Pipeline())
    controller._pool = pool = _Pool()
    controller._detail_pool = detail_pool = _Pool()
    pool.waitForDone = Mock()
    detail_pool.waitForDone = Mock()
    pool.activeThreadCount = lambda: 1
    controller.request(["old.jpg"])

    controller.shutdown()
    controller.request(["new.jpg"])
    controller.request_details(["new.jpg"])

    assert controller.is_active()
    pool.activeThreadCount = lambda: 0
    assert not controller.is_active()
    pool.waitForDone.assert_not_called()
    detail_pool.waitForDone.assert_not_called()
    assert len(pool.started) == 1
    assert detail_pool.started == []


def test_shutdown_returns_while_a_real_preview_pool_is_decoding():
    import threading
    import time
    from PyQt6.QtWidgets import QApplication

    app = QApplication.instance() or QApplication([])
    started, release = threading.Event(), threading.Event()

    class SlowPipeline:
        def ensure_preview_cached(self, path):
            started.set()
            release.wait(5)
            return True

    controller = PreviewLoadController(SlowPipeline())
    ready = []
    controller.preview_ready.connect(ready.append)
    controller.request(["slow.arw"])
    try:
        assert started.wait(1)
        before = time.monotonic()
        controller.shutdown()
        assert time.monotonic() - before < 0.5
        assert controller.is_active()
    finally:
        release.set()
        assert controller._pool.waitForDone(2000)
        app.processEvents()
    assert not controller.is_active()
    assert ready == []

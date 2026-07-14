from core.app_settings import NAVIGATION_PREVIEW_LOOKAHEAD
from ui.controllers.preview_load_controller import PreviewLoadController


class _Pipeline:
    def __init__(self):
        self.generated = []

    def ensure_preview_cached(self, path):
        self.generated.append(path)
        return True


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

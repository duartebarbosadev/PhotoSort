import pyexiv2  # noqa: F401  # Must be first to avoid Windows crash with pyexiv2
from src.ui.controllers.preview_controller import PreviewController


class DummyCtx:
    def __init__(self):
        self.worker_manager = type(
            "WM", (), {"calls": [], "start_preview_preload": self._start}
        )()
        self.loading = []
        self.statuses = []

    def _start(self, paths):
        self.worker_manager.calls.append(paths)

    def show_loading_overlay(self, text):
        self.loading.append(("show", text))

    def hide_loading_overlay(self):
        self.loading.append(("hide",))

    def status_message(self, msg, timeout=3000):
        self.statuses.append(msg)


def test_preview_preload_filters_and_starts():
    ctx = DummyCtx()
    pc = PreviewController(ctx)
    pc.start_preload([{"path": "a.jpg"}, {"path": None}, {"no": "key"}])
    assert ctx.worker_manager.calls == [["a.jpg"]]


def test_preview_preload_no_items():
    ctx = DummyCtx()
    pc = PreviewController(ctx)
    pc.start_preload([])
    assert ctx.worker_manager.calls == []
    assert any("No previews" in s for s in ctx.statuses)

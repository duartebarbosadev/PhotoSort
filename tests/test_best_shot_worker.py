import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import pytest
from PIL import Image
from PyQt6.QtWidgets import QApplication

from workers.best_shot_worker import BestShotWorker


@pytest.fixture(scope="module", autouse=True)
def qt_app():
    app = QApplication.instance()
    if app is None:
        app = QApplication([])
    return app


class DummyPipeline:
    def __init__(self):
        self.calls = []

    def get_preview_image(self, image_path, display_max_size=None, force_regenerate=False, force_default_brightness=False):
        self.calls.append((image_path, display_max_size))
        img = Image.new("RGB", (64, 64), color="white")
        img.info["source_path"] = image_path
        img.info["region"] = "full"
        return img


class _DummyResult:
    def __init__(self, path: str):
        self.image_path = path
        self.composite_score = 1.0
        self.metrics = {}
        self.raw_metrics = {}

    def to_dict(self):
        return {
            "image_path": self.image_path,
            "composite_score": self.composite_score,
            "metrics": self.metrics,
            "raw_metrics": self.raw_metrics,
        }


class DummySelector:
    def __init__(self, **kwargs):
        self.loader = kwargs["image_loader"]
        self.rank_calls = []

    def rank_images(self, image_paths):
        results = []
        for path in image_paths:
            img = self.loader(path)
            assert isinstance(img, Image.Image)
            results.append(_DummyResult(path))
        return results


def test_best_shot_worker_uses_preview_pipeline(monkeypatch):
    pipeline = DummyPipeline()
    worker = BestShotWorker({0: ["/tmp/sel1.jpg", "/tmp/sel2.jpg"]}, image_pipeline=pipeline)

    monkeypatch.setattr(
        "core.ai.best_photo_selector.BestPhotoSelector",
        DummySelector,
    )
    monkeypatch.setattr(
        "core.ai.model_checker.check_best_shot_models",
        lambda models_root: [],
    )

    completed_payload = []
    worker.completed.connect(lambda payload: completed_payload.append(payload))

    worker.run()

    assert [call[0] for call in pipeline.calls] == ["/tmp/sel1.jpg", "/tmp/sel2.jpg"]
    assert completed_payload
    assert 0 in completed_payload[0]
    assert [entry["image_path"] for entry in completed_payload[0][0]] == ["/tmp/sel1.jpg", "/tmp/sel2.jpg"]

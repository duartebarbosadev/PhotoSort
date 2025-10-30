import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import pytest
from PIL import Image
from PyQt6.QtWidgets import QApplication

from core.ai.best_shot_pipeline import BaseBestShotStrategy
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


class DummyStrategy(BaseBestShotStrategy):
    def __init__(self, pipeline):
        super().__init__(models_root=None, image_pipeline=pipeline)
        self.pipeline = pipeline

    @property
    def max_workers(self) -> int:  # pragma: no cover - simple override
        return 4

    def rank_cluster(self, cluster_id, image_paths):
        payload = []
        for path in image_paths:
            img = self.pipeline.get_preview_image(path)
            assert isinstance(img, Image.Image)
            payload.append({
                "image_path": path,
                "composite_score": 1.0,
                "metrics": {},
            })
        return payload

    def rate_image(self, image_path):  # pragma: no cover - unused in test
        return None


def test_best_shot_worker_uses_preview_pipeline(monkeypatch):
    pipeline = DummyPipeline()
    strategy = DummyStrategy(pipeline)
    worker = BestShotWorker(
        {0: ["/tmp/sel1.jpg", "/tmp/sel2.jpg"]},
        image_pipeline=pipeline,
        strategy=strategy,
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

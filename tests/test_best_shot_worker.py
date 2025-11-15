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

    def get_preview_image(
        self,
        image_path,
        display_max_size=None,
        force_regenerate=False,
        force_default_brightness=False,
    ):
        self.calls.append((image_path, display_max_size))
        img = Image.new("RGB", (64, 64), color="white")
        img.info["source_path"] = image_path
        img.info["region"] = "full"
        return img


class DummyStrategy(BaseBestShotStrategy):
    def __init__(self, pipeline):
        super().__init__(image_pipeline=pipeline)
        self.pipeline = pipeline
        self.rank_calls: list[tuple[str, ...]] = []

    @property
    def max_workers(self) -> int:  # pragma: no cover - simple override
        return 4

    def rank_cluster(self, cluster_id, image_paths):
        self.rank_calls.append(tuple(image_paths))
        payload = []
        for path in image_paths:
            img = self.pipeline.get_preview_image(path)
            assert isinstance(img, Image.Image)
            payload.append(
                {
                    "image_path": path,
                    "composite_score": 1.0,
                    "metrics": {},
                }
            )
        return payload

    def rate_image(self, image_path):  # pragma: no cover - unused in test
        return None


def test_best_shot_worker_uses_preview_pipeline():
    pipeline = DummyPipeline()
    strategy = DummyStrategy(pipeline)
    worker = BestShotWorker(
        {0: ["/tmp/sel1.jpg", "/tmp/sel2.jpg"]},
        image_pipeline=pipeline,
        strategy=strategy,
    )

    completed_payload = []
    worker.completed.connect(lambda payload: completed_payload.append(payload))

    worker.run()

    assert [call[0] for call in pipeline.calls] == ["/tmp/sel1.jpg", "/tmp/sel2.jpg"]
    assert completed_payload
    assert 0 in completed_payload[0]
    assert [entry["image_path"] for entry in completed_payload[0][0]] == [
        "/tmp/sel1.jpg",
        "/tmp/sel2.jpg",
    ]


def test_best_shot_worker_batches_large_clusters():
    pipeline = DummyPipeline()

    class RecordingStrategy(DummyStrategy):
        pass

    strategy = RecordingStrategy(pipeline)
    image_paths = [f"/tmp/batch_{idx}.jpg" for idx in range(1, 7)]
    worker = BestShotWorker(
        {0: image_paths},
        image_pipeline=pipeline,
        strategy=strategy,
        best_shot_batch_size=3,
    )

    completed_payload = []
    worker.completed.connect(lambda payload: completed_payload.append(payload))

    worker.run()

    assert completed_payload and 0 in completed_payload[0]
    result_paths = {entry["image_path"] for entry in completed_payload[0][0]}
    assert set(image_paths) == result_paths
    # Ensure every rank_cluster call respects the configured batch size
    assert all(len(call) <= 3 for call in strategy.rank_calls)

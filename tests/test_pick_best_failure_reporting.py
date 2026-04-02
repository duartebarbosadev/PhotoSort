import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from pathlib import Path

import pytest
from PyQt6.QtWidgets import QApplication

from core.best_photo_finder.config import SelectorConfig
from core.best_photo_finder.errors import NoScorableImagesError, SelectionError
from core.best_photo_finder.pipeline import PhotoSelector
from ui.pick_best_step_widget import PickBestStepWidget
from workers.pick_best_worker import PickBestWorker


_app = QApplication.instance() or QApplication([])


class _FailingTechnicalScorer:
    def score(self, path: Path, config: SelectorConfig):
        raise SelectionError(f"Could not read image: {path.name}")

    def score_image(self, path: Path, image, config: SelectorConfig):
        raise SelectionError(f"Could not read image: {path.name}")


class _UnusedAestheticScorer:
    model_name = "test"

    @property
    def device_used(self) -> str:
        return "cpu"

    def score_batch(self, paths, config):
        return {}

    def score_batch_from_images(self, images_by_path, config):
        return {}


def test_selector_includes_per_image_failures_in_no_scorable_error():
    selector = PhotoSelector(
        technical_scorer=_FailingTechnicalScorer(),
        aesthetic_scorer=_UnusedAestheticScorer(),
    )

    with pytest.raises(NoScorableImagesError) as exc_info:
        selector.select(["/tmp/a.jpg", "/tmp/b.jpg"])

    error = exc_info.value
    assert error.failures == [
        (str(Path("/tmp/a.jpg").resolve()), "Could not read image: a.jpg"),
        (str(Path("/tmp/b.jpg").resolve()), "Could not read image: b.jpg"),
    ]
    assert "a.jpg: Could not read image: a.jpg" in str(error)
    assert "b.jpg: Could not read image: b.jpg" in str(error)


def test_pick_best_worker_keeps_failure_reasons_when_cluster_is_skipped(monkeypatch):
    class _FakeSelector:
        def __init__(self, preview_loader=None):
            self.preview_loader = preview_loader

        def select(self, paths):
            raise NoScorableImagesError(
                "No images could be scored successfully.",
                failures=[
                    (paths[0], "Could not read image preview."),
                    (paths[1], "Could not read image preview."),
                ],
            )

    monkeypatch.setattr("workers.pick_best_worker.PhotoSelector", _FakeSelector)

    worker = PickBestWorker({7: ["/tmp/a.jpg", "/tmp/b.jpg"]})
    completed_payload = []
    worker.completed.connect(lambda payload: completed_payload.append(payload))

    worker.run()

    cluster = completed_payload[0][7]
    assert cluster["winner_path"] is None
    assert cluster["failed"] == [
        {
            "path": "/tmp/a.jpg",
            "status": "failed",
            "failure_reason": "Could not read image preview.",
        },
        {
            "path": "/tmp/b.jpg",
            "status": "failed",
            "failure_reason": "Could not read image preview.",
        },
    ]


def test_pick_best_widget_shows_failure_reason_for_unscored_image(monkeypatch):
    monkeypatch.setattr(
        "ui.pick_best_step_widget.MetadataProcessor.get_detailed_metadata",
        lambda path, cache: {},
    )

    widget = PickBestStepWidget()
    widget.show_results(
        {
            1: {
                "winner_path": "/tmp/winner.jpg",
                "ranked": [
                    {"path": "/tmp/winner.jpg", "final_score": 0.91},
                ],
                "failed": [
                    {
                        "path": "/tmp/failed.jpg",
                        "failure_reason": "Aesthetic model did not return a score for this image.",
                    }
                ],
                "all_paths": ["/tmp/failed.jpg", "/tmp/winner.jpg"],
                "unsupported_paths": [],
            }
        }
    )

    failed_card = widget._compare_cards[0]
    assert failed_card.path == "/tmp/failed.jpg"
    assert failed_card._score_label.text() == "Score unavailable"
    assert (
        failed_card._score_label.toolTip()
        == "Aesthetic model did not return a score for this image."
    )
    assert failed_card._meta_rows[0][0].text() == "Scoring"
    assert (
        failed_card._meta_rows[0][1].text()
        == "Aesthetic model did not return a score for this image."
    )

import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from pathlib import Path

import pytest
from PyQt6.QtWidgets import QApplication

from core.best_photo_finder.config import SelectorConfig
from core.best_photo_finder.errors import (
    FaceLandmarkerError,
    IncompleteSelectionError,
    NoScorableImagesError,
    SelectionError,
)
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


def test_selector_rejects_partial_technical_results():
    class _PartiallyFailingTechnicalScorer(_FailingTechnicalScorer):
        def score(self, path: Path, config: SelectorConfig):
            if path.name == "b.jpg":
                return super().score(path, config)
            from core.best_photo_finder.models import TechnicalMetrics

            return TechnicalMetrics(
                blur_variance=100.0,
                blur_penalty=0.0,
                face_count=0,
                closed_face_count=0,
                eye_penalty=0.0,
                max_face_area_ratio=0.0,
                image_width=100,
                image_height=100,
            )

    selector = PhotoSelector(
        technical_scorer=_PartiallyFailingTechnicalScorer(),
        aesthetic_scorer=_UnusedAestheticScorer(),
    )

    with pytest.raises(IncompleteSelectionError, match="requires every image"):
        selector.select(["/tmp/a.jpg", "/tmp/b.jpg"])


def test_pick_best_worker_stops_when_cluster_cannot_be_scored(monkeypatch):
    class _FakeSelector:
        def __init__(self, preview_loader=None, **_kwargs):
            self.preview_loader = preview_loader

        def select(self, paths):
            raise NoScorableImagesError(
                "No images could be scored successfully.",
                failures=[
                    (paths[0], "Could not read image preview."),
                    (paths[1], "Could not read image preview."),
                ],
            )

        def close(self):
            pass

    monkeypatch.setattr("workers.pick_best_worker.PhotoSelector", _FakeSelector)

    worker = PickBestWorker({7: ["/tmp/a.jpg", "/tmp/b.jpg"]})
    errors = []
    completed_payload = []
    worker.error.connect(errors.append)
    worker.completed.connect(lambda payload: completed_payload.append(payload))

    worker.run()

    assert completed_payload == []
    assert len(errors) == 1
    assert "cluster 7 could not be scored" in errors[0]


def test_pick_best_worker_closes_selector_when_cancelled(monkeypatch):
    closed = []

    class _FakeSelector:
        def __init__(self, **_kwargs):
            pass

        def select(self, paths):  # pragma: no cover - cancelled before selection
            raise AssertionError(paths)

        def close(self):
            closed.append(True)

    monkeypatch.setattr("workers.pick_best_worker.PhotoSelector", _FakeSelector)

    worker = PickBestWorker({1: ["/tmp/a.jpg", "/tmp/b.jpg"]})
    worker.stop()
    worker.run()

    assert closed == [True]


def test_pick_best_worker_stops_on_face_landmarker_failure(monkeypatch):
    class _FakeTechnicalScorer:
        pass

    class _FakeSelector:
        def __init__(self, **_kwargs):
            pass

        def select(self, _paths):
            raise FaceLandmarkerError("model could not load")

        def close(self):
            closed.append(True)

    monkeypatch.setattr(
        "workers.pick_best_worker.OpenCvMediapipeTechnicalScorer",
        _FakeTechnicalScorer,
    )
    monkeypatch.setattr("workers.pick_best_worker.PhotoSelector", _FakeSelector)

    worker = PickBestWorker({1: ["/tmp/a.jpg", "/tmp/b.jpg"]})
    errors = []
    completed = []
    closed = []
    worker.error.connect(errors.append)
    worker.completed.connect(completed.append)

    worker.run()

    assert completed == []
    assert closed == [True]
    assert len(errors) == 1
    assert "Pick Best stopped" in errors[0]
    assert "model could not load" in errors[0]


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


def test_pick_best_widget_shows_capture_date_in_metadata(monkeypatch):
    monkeypatch.setattr(
        "ui.pick_best_step_widget.MetadataProcessor.get_cached_detailed_metadata",
        lambda path, cache: {
            "Exif.Photo.DateTimeOriginal": "2024:01:02 03:04:05",
            "Exif.Image.Make": "SONY",
            "Exif.Image.Model": "A7 III",
        },
    )

    widget = PickBestStepWidget()
    widget.show_results(
        {
            1: {
                "winner_path": "/tmp/winner.jpg",
                "ranked": [
                    {"path": "/tmp/winner.jpg", "final_score": 0.91},
                    {"path": "/tmp/challenger.jpg", "final_score": 0.72},
                ],
                "failed": [],
                "all_paths": ["/tmp/challenger.jpg", "/tmp/winner.jpg"],
                "unsupported_paths": [],
            }
        }
    )

    challenger_card = widget._compare_cards[0]
    assert challenger_card.path == "/tmp/challenger.jpg"
    assert challenger_card._meta_rows[0][0].text() == "Date"
    assert challenger_card._meta_rows[0][1].text() == "2024-01-02 03:04"

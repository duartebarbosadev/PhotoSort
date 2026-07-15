import pyexiv2  # noqa: F401 - initialize native metadata libraries before Qt

from pathlib import Path
from types import SimpleNamespace

import numpy as np
import pytest

from core.best_photo_finder.errors import FaceLandmarkerError
from core.best_photo_finder.config import SelectorConfig
from core.best_photo_finder.scorers import (
    LEFT_EYE_INDICES,
    RIGHT_EYE_INDICES,
    MediaPipeTasksFaceLandmarker,
    OpenCvMediapipeTechnicalScorer,
    _eye_aspect_ratio,
)
from core.runtime_paths import resolve_face_landmarker_model_path


class _Backend:
    def __init__(self, landmarks=()):
        self.landmarks = landmarks
        self.detect_calls = 0
        self.close_calls = 0

    def detect_landmarks(self, rgb_image):
        self.detect_calls += 1
        return self.landmarks

    def close(self):
        self.close_calls += 1


def test_vendored_face_landmarker_model_is_resolvable():
    model_path = resolve_face_landmarker_model_path()

    assert model_path.name == "face_landmarker.task"
    assert model_path.stat().st_size == 3_758_596


def test_tasks_adapter_converts_rgb_array_and_returns_landmarks(monkeypatch, tmp_path):
    created = {}
    expected_landmarks = [[SimpleNamespace(x=0.2, y=0.3)]]

    class FakeDetector:
        def detect(self, image):
            created["image"] = image
            return SimpleNamespace(face_landmarks=expected_landmarks)

        def close(self):
            created["closed"] = True

    class FakeFaceLandmarker:
        @staticmethod
        def create_from_options(options):
            created["options"] = options
            return FakeDetector()

    class FakeImage:
        def __init__(self, *, image_format, data):
            self.image_format = image_format
            self.data = data

    fake_vision = SimpleNamespace(
        FaceLandmarkerOptions=lambda **kwargs: SimpleNamespace(**kwargs),
        FaceLandmarker=FakeFaceLandmarker,
        RunningMode=SimpleNamespace(IMAGE="image"),
    )
    fake_mediapipe = SimpleNamespace(
        __version__="test",
        tasks=SimpleNamespace(
            BaseOptions=lambda **kwargs: SimpleNamespace(**kwargs),
            vision=fake_vision,
        ),
        Image=FakeImage,
        ImageFormat=SimpleNamespace(SRGB="srgb"),
    )
    monkeypatch.setattr(
        "core.best_photo_finder.scorers._require_module",
        lambda name: fake_mediapipe,
    )

    model_path = tmp_path / "face_landmarker.task"
    adapter = MediaPipeTasksFaceLandmarker(model_path)
    rgb = np.zeros((4, 5, 3), dtype=np.uint8)[:, ::-1]

    assert adapter.detect_landmarks(rgb) == expected_landmarks
    assert created["options"].base_options.model_asset_path == str(model_path)
    assert created["image"].data.flags.c_contiguous

    adapter.close()
    assert created["closed"] is True


def test_technical_scorer_initializes_landmarker_once_and_closes_it():
    backend = _Backend()
    calls: list[Path] = []

    def factory(model_path):
        calls.append(model_path)
        return backend

    scorer = OpenCvMediapipeTechnicalScorer(face_landmarker_factory=factory)

    assert scorer._get_face_landmarker() is backend
    assert scorer._get_face_landmarker() is backend
    assert len(calls) == 1

    scorer.close()
    assert backend.close_calls == 1


def test_invalid_landmarker_is_a_fatal_scoring_error():
    calls = 0

    def factory(_model_path):
        nonlocal calls
        calls += 1
        raise RuntimeError("invalid model")

    scorer = OpenCvMediapipeTechnicalScorer(face_landmarker_factory=factory)

    with pytest.raises(FaceLandmarkerError, match="could not be initialized"):
        scorer._get_face_landmarker()

    assert calls == 1


def test_technical_scorer_uses_landmarker_for_the_full_image():
    landmarks = [SimpleNamespace(x=0.5, y=0.5) for _ in range(478)]
    for indices in (LEFT_EYE_INDICES, RIGHT_EYE_INDICES):
        p1, p2, p3, p4, p5, p6 = indices
        landmarks[p1] = SimpleNamespace(x=0.2, y=0.4)
        landmarks[p4] = SimpleNamespace(x=0.8, y=0.4)
        landmarks[p2] = SimpleNamespace(x=0.3, y=0.3)
        landmarks[p6] = SimpleNamespace(x=0.3, y=0.5)
        landmarks[p3] = SimpleNamespace(x=0.7, y=0.3)
        landmarks[p5] = SimpleNamespace(x=0.7, y=0.5)

    backend = _Backend([landmarks])
    scorer = OpenCvMediapipeTechnicalScorer(
        face_landmarker_factory=lambda _path: backend
    )
    image = np.zeros((120, 200, 3), dtype=np.uint8)

    metrics = scorer.score_image(Path("photo.jpg"), image, SelectorConfig())

    assert backend.detect_calls == 1
    assert metrics.face_count == 1
    assert metrics.closed_face_count == 0
    assert metrics.max_face_area_ratio > 0


def test_landmarker_detection_failure_closes_native_resource_and_fails():
    class FailingBackend(_Backend):
        def detect_landmarks(self, rgb_image):
            raise RuntimeError("native detection error")

    backend = FailingBackend()
    scorer = OpenCvMediapipeTechnicalScorer(
        face_landmarker_factory=lambda _path: backend
    )

    with pytest.raises(FaceLandmarkerError, match="native detection error"):
        scorer.score_image(
            Path("photo.jpg"),
            np.zeros((40, 40, 3), dtype=np.uint8),
            SelectorConfig(),
        )

    assert backend.close_calls == 1


def test_eye_aspect_ratio_preserves_open_and_closed_eye_threshold_behavior():
    points = [SimpleNamespace(x=0.5, y=0.5) for _ in range(478)]
    p1, p2, p3, p4, p5, p6 = LEFT_EYE_INDICES
    points[p1] = SimpleNamespace(x=0.0, y=0.0)
    points[p4] = SimpleNamespace(x=1.0, y=0.0)
    points[p2] = SimpleNamespace(x=0.25, y=0.20)
    points[p6] = SimpleNamespace(x=0.25, y=-0.20)
    points[p3] = SimpleNamespace(x=0.75, y=0.20)
    points[p5] = SimpleNamespace(x=0.75, y=-0.20)

    open_ratio = _eye_aspect_ratio(points, LEFT_EYE_INDICES)
    points[p2].y = points[p3].y = 0.02
    points[p5].y = points[p6].y = -0.02
    closed_ratio = _eye_aspect_ratio(points, LEFT_EYE_INDICES)

    assert open_ratio == pytest.approx(0.4)
    assert closed_ratio == pytest.approx(0.04)
    assert closed_ratio < open_ratio

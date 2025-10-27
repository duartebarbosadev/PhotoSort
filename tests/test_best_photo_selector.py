from __future__ import annotations

import numpy as np
from PIL import Image

from core.ai.best_photo_selector import (
    BestPhotoSelector,
    FaceDetectionResult,
    QualityScore,
    _default_focus_score,
)


def _make_detection() -> FaceDetectionResult:
    return FaceDetectionResult(
        score=0.92,
        bbox=(10, 10, 90, 90),
        bbox_normalized=(0.1, 0.1, 0.9, 0.9),
        keypoints=[(0.3, 0.4), (0.7, 0.4), (0.5, 0.55), (0.5, 0.65), (0.25, 0.5), (0.75, 0.5)],
        image_size=(100, 100),
    )


class DummyFaceDetector:
    def __init__(self, mapping):
        self.mapping = mapping

    def detect_faces(self, image, image_path=None, max_faces=None):
        return list(self.mapping.get(image_path, []))


class DummyEyeClassifier:
    def __init__(self, mapping):
        self.mapping = mapping

    def predict_open_probability(self, eye_image, image_path=None):
        path = image_path or eye_image.info.get("source_path")
        return float(self.mapping.get(path, 0.5))


class DummyQualityModel:
    def __init__(self, full_scores, face_scores):
        self.full_scores = full_scores
        self.face_scores = face_scores

    def score(self, image, return_embedding=False):
        path = image.info.get("source_path")
        region = image.info.get("region", "full")
        table = self.face_scores if region == "face" else self.full_scores
        result = table[path]
        return result


def _loader_factory():
    def _loader(image_path: str) -> Image.Image:
        img = Image.new("RGB", (100, 100), color="white")
        img.info["source_path"] = image_path
        img.info["region"] = "full"
        return img

    return _loader


def test_selector_prefers_open_eyes_and_subject_focus(tmp_path):
    img_a = str(tmp_path / "a.jpg")
    img_b = str(tmp_path / "b.jpg")
    tmp_path.joinpath("a.jpg").write_text("a")
    tmp_path.joinpath("b.jpg").write_text("b")

    face_detector = DummyFaceDetector({img_a: [_make_detection()]})
    eye_classifier = DummyEyeClassifier({img_a: 0.9})

    full_scores = {
        img_a: QualityScore(raw=8.5, normalized=0.83, embedding=np.array([1.0, 0.0])),
        img_b: QualityScore(raw=7.0, normalized=0.66, embedding=np.array([0.0, 1.0])),
    }
    face_scores = {
        img_a: QualityScore(raw=7.2, normalized=0.7, embedding=np.array([0.9, 0.1])),
    }

    def focus_metric(image: Image.Image) -> float:
        region = image.info.get("region", "full")
        path = image.info.get("source_path")
        if region == "face" and path == img_a:
            return 0.8
        if region == "full" and path == img_a:
            return 0.75
        return 0.55

    selector = BestPhotoSelector(
        face_detector=face_detector,
        eye_classifier=eye_classifier,
        quality_model=DummyQualityModel(full_scores, face_scores),
        image_loader=_loader_factory(),
        focus_metric_fn=focus_metric,
    )

    results = selector.rank_images([img_b, img_a])
    assert [r.image_path for r in results] == [img_a, img_b]
    assert results[0].metrics["eyes_open"] == 0.9
    assert "framing" in results[0].metrics
    assert "eyes_open" not in results[1].metrics


def test_selector_handles_images_without_faces(tmp_path):
    img_a = str(tmp_path / "a.jpg")
    img_b = str(tmp_path / "b.jpg")
    tmp_path.joinpath("a.jpg").write_text("a")
    tmp_path.joinpath("b.jpg").write_text("b")

    full_scores = {
        img_a: QualityScore(raw=7.5, normalized=0.7, embedding=None),
        img_b: QualityScore(raw=6.5, normalized=0.6, embedding=None),
    }

    def focus_metric(image: Image.Image) -> float:
        return 0.8 if image.info.get("source_path") == img_a else 0.5

    selector = BestPhotoSelector(
        face_detector=DummyFaceDetector({}),
        eye_classifier=DummyEyeClassifier({}),
        quality_model=DummyQualityModel(full_scores, {}),
        image_loader=_loader_factory(),
        focus_metric_fn=focus_metric,
    )

    results = selector.rank_images([img_b, img_a])
    assert [r.image_path for r in results] == [img_a, img_b]

    result = results[0]
    assert result.metrics["technical"] == 0.8
    assert "eyes_open" not in result.metrics
    assert "framing" not in result.metrics

    # Only aesthetic + technical contribute (equal weight in normalization)
    expected = (0.7 + 0.8) / 2.0
    assert abs(result.composite_score - expected) < 1e-6


def test_default_focus_score_handles_uint16_image():
    data = np.random.randint(0, 65535, (12, 12), dtype=np.uint16)
    img = Image.fromarray(data, mode="I;16")
    score = _default_focus_score(img)
    assert 0.0 <= score <= 1.0

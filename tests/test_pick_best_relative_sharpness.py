import pyexiv2  # noqa: F401  # Must be first to avoid Windows crash

from pathlib import Path

import pytest

from core.best_photo_finder.config import SelectorConfig
from core.best_photo_finder.models import TechnicalMetrics
from core.best_photo_finder.pipeline import PhotoSelector


class _TechnicalScorer:
    def __init__(self, variances: dict[str, float], penalties: dict[str, float]):
        self.variances = variances
        self.penalties = penalties

    def score(self, path: Path, _config: SelectorConfig) -> TechnicalMetrics:
        return TechnicalMetrics(
            blur_variance=self.variances[path.name],
            blur_penalty=self.penalties.get(path.name, 0.0),
            face_count=0,
            closed_face_count=0,
            eye_penalty=0.0,
            max_face_area_ratio=0.0,
            image_width=1024,
            image_height=768,
        )

    def close(self) -> None:
        pass


class _AestheticScorer:
    model_name = "test-aesthetic"

    def __init__(self, scores: dict[str, float]):
        self.scores = scores

    @property
    def device_used(self) -> str:
        return "cpu"

    def score_batch(self, paths, _config):
        return {path: self.scores[path.name] for path in paths}

    def score_batch_from_images(self, images_by_path, config):
        return self.score_batch(images_by_path, config)


def _selector(
    variances: dict[str, float],
    aesthetic_scores: dict[str, float],
    penalties: dict[str, float] | None = None,
) -> PhotoSelector:
    return PhotoSelector(
        technical_scorer=_TechnicalScorer(variances, penalties or {}),
        aesthetic_scorer=_AestheticScorer(aesthetic_scores),
    )


def test_substantially_softer_frame_cannot_win_on_aesthetics():
    selector = _selector(
        variances={"soft.jpg": 21.47, "sharp.jpg": 47.07},
        aesthetic_scores={"soft.jpg": 0.664, "sharp.jpg": 0.304},
        penalties={"soft.jpg": 0.282, "sharp.jpg": 0.200},
    )

    result = selector.select(["/tmp/soft.jpg", "/tmp/sharp.jpg"])
    by_name = {Path(image.path).name: image for image in result.ranked_images}

    assert Path(result.winner.path).name == "sharp.jpg"
    assert by_name["soft.jpg"].cluster_sharpness_ratio == pytest.approx(21.47 / 47.07)
    assert by_name["soft.jpg"].sharpness_eligible is False
    assert by_name["sharp.jpg"].sharpness_eligible is True
    assert by_name["soft.jpg"].base_score > by_name["sharp.jpg"].base_score
    assert by_name["soft.jpg"].final_score < by_name["sharp.jpg"].final_score
    assert "Substantially softer" in by_name["soft.jpg"].issues[-1]


def test_aesthetics_still_selects_between_comparably_sharp_frames():
    selector = _selector(
        variances={"a.jpg": 90.0, "b.jpg": 100.0},
        aesthetic_scores={"a.jpg": 0.8, "b.jpg": 0.5},
    )

    result = selector.select(["/tmp/a.jpg", "/tmp/b.jpg"])

    assert Path(result.winner.path).name == "a.jpg"
    assert all(image.sharpness_eligible for image in result.ranked_images)


def test_featureless_cluster_does_not_arbitrarily_eliminate_frames():
    selector = _selector(
        variances={"a.jpg": 0.0, "b.jpg": 0.0},
        aesthetic_scores={"a.jpg": 0.4, "b.jpg": 0.7},
    )

    result = selector.select(["/tmp/a.jpg", "/tmp/b.jpg"])

    assert Path(result.winner.path).name == "b.jpg"
    assert all(image.sharpness_eligible for image in result.ranked_images)
    assert all(image.cluster_sharpness_ratio == 1.0 for image in result.ranked_images)

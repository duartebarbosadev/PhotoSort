from core.ai.best_photo_selector import BestShotResult
from core.ai.best_shot_pipeline import _compute_quality_rating


def _make_result(
    musiq: float, maniqa: float, liqe: float, composite: float
) -> BestShotResult:
    return BestShotResult(
        image_path="dummy.jpg",
        composite_score=composite,
        metrics={},
        raw_metrics={
            "musiq_raw": musiq,
            "maniqa_raw": maniqa,
            "liqe_raw": liqe,
        },
    )


def test_quality_rating_spreads_scores():
    poor = _make_result(20.0, 0.2, 25.0, 0.2)
    rich = _make_result(85.0, 0.9, 90.0, 0.9)

    poor_rating, poor_score = _compute_quality_rating(poor)
    rich_rating, rich_score = _compute_quality_rating(rich)

    assert poor_rating <= 2
    assert rich_rating == 5
    assert poor_score < 0.3 < rich_score


def test_mid_quality_maps_to_four():
    mid = _make_result(55.0, 0.45, 60.0, 0.5)
    rating, _ = _compute_quality_rating(mid)
    assert rating == 4

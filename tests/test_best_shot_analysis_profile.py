from core.ai.best_photo_selector import DEFAULT_METRIC_SPECS
from core.ai.best_shot_pipeline import (
    MAX_LOCAL_ANALYSIS_EDGE,
    RESPONSIVE_LOCAL_ANALYSIS_EDGE,
    select_local_analysis_profile,
)
from core.app_settings import PerformanceMode


def _metric_names(profile) -> tuple[str, ...]:
    return tuple(spec.name for spec in profile.metric_specs)


def test_balanced_mode_keeps_full_quality_stack():
    profile = select_local_analysis_profile(PerformanceMode.BALANCED)
    assert profile.max_edge == MAX_LOCAL_ANALYSIS_EDGE
    expected = tuple(spec.name for spec in DEFAULT_METRIC_SPECS)
    assert _metric_names(profile) == expected


def test_performance_mode_keeps_full_quality_stack():
    profile = select_local_analysis_profile(PerformanceMode.PERFORMANCE)
    assert profile.max_edge == MAX_LOCAL_ANALYSIS_EDGE
    expected = tuple(spec.name for spec in DEFAULT_METRIC_SPECS)
    assert _metric_names(profile) == expected


def test_custom_mode_uses_ratio_threshold():
    high_ratio_profile = select_local_analysis_profile(
        PerformanceMode.CUSTOM, custom_thread_ratio=0.99
    )
    assert high_ratio_profile.max_edge == MAX_LOCAL_ANALYSIS_EDGE

    low_ratio_profile = select_local_analysis_profile(
        PerformanceMode.CUSTOM, custom_thread_ratio=0.5
    )
    assert low_ratio_profile.max_edge == RESPONSIVE_LOCAL_ANALYSIS_EDGE

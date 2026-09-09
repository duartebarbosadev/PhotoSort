import pyexiv2  # noqa: F401  # Must be first to avoid Windows crash

from unittest.mock import Mock

from core.caching.analysis_cache import AnalysisCache
from core.grouping import GroupingMode, build_grouping_plan
from core.similarity_clustering import cluster_paths_with_cache


def _engine(clusters):
    engine = Mock()
    engine.model.cache_key = "dinov2-small"
    engine.model.region_cache_key = "regions-v1"
    engine.run_analysis_sync.return_value = ({}, clusters)
    return engine


def test_second_workflow_reuses_clusters_without_re_encoding(tmp_path):
    """Organize and the similarity view must share one warm cluster cache."""

    cache = AnalysisCache(str(tmp_path / "analysis_cache"))
    folder = str(tmp_path)
    paths = [str(tmp_path / "a.jpg"), str(tmp_path / "b.jpg")]
    for path in paths:
        with open(path, "wb") as handle:
            handle.write(b"jpeg")

    first = _engine({paths[0]: 1, paths[1]: 1})
    result = cluster_paths_with_cache(
        first, paths, analysis_cache=cache, folder_path=folder
    )
    assert result.reused is False
    assert first.run_analysis_sync.call_count == 1

    second = _engine({})
    reused = cluster_paths_with_cache(
        second, paths, analysis_cache=cache, folder_path=folder
    )

    assert reused.reused is True
    assert reused.clusters == {paths[0]: 1, paths[1]: 1}
    # The expensive encode must not happen a second time.
    assert second.run_analysis_sync.call_count == 0


def test_manual_overrides_survive_a_fresh_analysis(tmp_path):
    cache = AnalysisCache(str(tmp_path / "analysis_cache"))
    folder = str(tmp_path)
    path = str(tmp_path / "a.jpg")
    with open(path, "wb") as handle:
        handle.write(b"jpeg")
    cache.save_manual_cluster_override(folder, path, 42)

    result = cluster_paths_with_cache(
        _engine({path: 1}), [path], analysis_cache=cache, folder_path=folder
    )

    assert result.clusters == {path: 42}


def test_changed_photos_force_a_new_analysis(tmp_path):
    cache = AnalysisCache(str(tmp_path / "analysis_cache"))
    folder = str(tmp_path)
    path = str(tmp_path / "a.jpg")
    with open(path, "wb") as handle:
        handle.write(b"jpeg")
    cluster_paths_with_cache(
        _engine({path: 1}), [path], analysis_cache=cache, folder_path=folder
    )

    with open(path, "wb") as handle:
        handle.write(b"jpeg-rotated-and-larger")
    engine = _engine({path: 3})
    result = cluster_paths_with_cache(
        engine, [path], analysis_cache=cache, folder_path=folder
    )

    assert result.reused is False
    assert engine.run_analysis_sync.call_count == 1


def test_organize_similarity_plan_reuses_the_shared_cluster_cache(
    tmp_path, monkeypatch
):
    cache = AnalysisCache(str(tmp_path / "analysis_cache"))
    folder = str(tmp_path)
    paths = [str(tmp_path / "a.jpg"), str(tmp_path / "b.jpg")]
    for path in paths:
        with open(path, "wb") as handle:
            handle.write(b"jpeg")

    cluster_paths_with_cache(
        _engine({paths[0]: 1, paths[1]: 1}),
        paths,
        analysis_cache=cache,
        folder_path=folder,
    )

    engine = _engine({})
    plan = build_grouping_plan(
        [{"path": path, "media_type": "image"} for path in paths],
        GroupingMode.SIMILARITY,
        similarity_engine=engine,
        analysis_cache=cache,
        folder_path=folder,
    )

    assert engine.run_analysis_sync.call_count == 0
    assert len(plan.groups) == 1
    assert sorted(plan.groups[0].source_paths) == sorted(paths)

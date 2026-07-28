import os
from unittest.mock import MagicMock, Mock

from src.core.caching.analysis_cache import AnalysisCache


def test_analysis_cache_persists_clusters_and_best_shots(tmp_path):
    cache_dir = tmp_path / "analysis_cache"
    cache = AnalysisCache(str(cache_dir))

    folder = os.path.join("/tmp", "photosort", "session")
    clusters = {
        "/tmp/photosort/session/img1.jpg": 1,
        "/tmp/photosort/session/img2.jpg": 1,
        "/tmp/photosort/session/img3.jpg": 2,
    }

    cache.save_cluster_results(folder, clusters, signature="signature")
    restored = cache.load(folder)
    assert restored["cluster_results"] == clusters
    assert (
        cache.load_valid_cluster_results(
            folder,
            signature="signature",
            expected_paths=set(clusters),
        )
        == clusters
    )

    rankings_cluster_1 = [
        {"image_path": "/tmp/photosort/session/img1.jpg", "composite_score": 0.9},
        {"image_path": "/tmp/photosort/session/img2.jpg", "composite_score": 0.8},
    ]

    cache.update_best_shot_results(folder, 1, rankings_cluster_1)
    restored_after = cache.load(folder)
    assert "best_shot_rankings" in restored_after
    assert "1" in restored_after["best_shot_rankings"]
    assert (
        restored_after["best_shot_rankings"]["1"][0]["image_path"]
        == "/tmp/photosort/session/img1.jpg"
    )

    completed = cache.get_completed_best_shot_clusters(folder)
    assert completed == {1}

    cache.close()


def test_analysis_cache_rejects_unsigned_partial_or_changed_clusters(tmp_path):
    cache = AnalysisCache(str(tmp_path / "analysis_cache"))
    folder = "/tmp/photos"
    clusters = {"a.jpg": 1, "b.jpg": 1}
    cache.save_cluster_results(folder, clusters, signature="current")

    assert (
        cache.load_valid_cluster_results(
            folder,
            signature="changed",
            expected_paths=set(clusters),
        )
        is None
    )
    assert (
        cache.load_valid_cluster_results(
            folder,
            signature="current",
            expected_paths={"a.jpg", "b.jpg", "c.jpg"},
        )
        is None
    )


def test_similarity_invalidation_preserves_manual_overrides(tmp_path):
    cache = AnalysisCache(str(tmp_path / "analysis_cache"))
    folder = "/tmp/photos"
    cache.save_cluster_results(
        folder,
        {"a.jpg": 1},
        signature="current",
    )
    cache.save_manual_cluster_override(folder, "a.jpg", 8)

    cache.invalidate_similarity(folder)

    entry = cache.load(folder)
    assert entry["manual_cluster_overrides"] == {"a.jpg": 8}
    assert "cluster_results" not in entry
    assert "similarity_signature" not in entry


def test_best_shot_batch_uses_one_cache_read_and_write():
    cache = AnalysisCache.__new__(AnalysisCache)
    cache._cache = MagicMock()
    cache.load = Mock(return_value={})

    cache.update_best_shot_results_batch(
        "/photos",
        {
            1: [{"image_path": "a.jpg", "composite_score": 0.9}],
            2: [{"image_path": "b.jpg", "composite_score": 0.8}],
        },
    )

    cache.load.assert_called_once_with("/photos")
    cache._cache.set.assert_called_once()
    saved = cache._cache.set.call_args.args[1]
    assert set(saved["best_shot_rankings"]) == {"1", "2"}


def test_analysis_cache_migrates_folder_and_all_path_references():
    old = "/photos/source/a.jpg"
    new = "/photos/output/a.jpg"
    cache = AnalysisCache.__new__(AnalysisCache)
    cache._cache = MagicMock()
    cache._cache.__contains__ = Mock(return_value=True)
    cache.load = Mock(
        return_value={
            "cluster_results": {old: 1},
            "manual_cluster_overrides": {old: 2},
            "best_shot_rankings": {"1": [{"image_path": old}]},
            "best_shot_scores_by_path": {old: {"image_path": old}},
            "best_shot_winners": {"1": {"image_path": old}},
        }
    )

    cache.migrate_folder_paths(
        "/photos/source",
        "/photos/output",
        {old: new},
    )

    cache.load.assert_called_once_with("/photos/source")
    cache._cache.set.assert_called_once()
    saved = cache._cache.set.call_args.args[1]
    assert saved["cluster_results"] == {new: 1}
    assert saved["manual_cluster_overrides"] == {new: 2}
    assert saved["best_shot_rankings"]["1"][0]["image_path"] == new
    assert saved["best_shot_scores_by_path"][new]["image_path"] == new
    assert saved["best_shot_winners"]["1"]["image_path"] == new
    cache._cache.__delitem__.assert_called_once()


def test_analysis_cache_removes_deleted_paths_with_one_read_and_write():
    removed = "/photos/deleted.jpg"
    kept = "/photos/kept.jpg"
    cache = AnalysisCache.__new__(AnalysisCache)
    cache._cache = MagicMock()
    cache.load = Mock(
        return_value={
            "cluster_results": {removed: 1, kept: 1},
            "manual_cluster_overrides": {removed: 1},
            "best_shot_rankings": {
                "1": [{"image_path": removed}, {"image_path": kept}]
            },
            "best_shot_scores_by_path": {
                removed: {"image_path": removed},
                kept: {"image_path": kept},
            },
            "best_shot_winners": {"1": {"image_path": removed}},
        }
    )

    cache.remove_paths("/photos", {removed})

    cache.load.assert_called_once_with("/photos")
    cache._cache.set.assert_called_once()
    saved = cache._cache.set.call_args.args[1]
    assert saved["cluster_results"] == {kept: 1}
    assert saved["manual_cluster_overrides"] == {}
    assert saved["best_shot_rankings"] == {"1": [{"image_path": kept}]}
    assert saved["best_shot_scores_by_path"] == {kept: {"image_path": kept}}
    assert saved["best_shot_winners"] == {"1": {"image_path": kept}}

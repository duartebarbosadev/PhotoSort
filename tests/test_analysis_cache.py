import os

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

    cache.save_cluster_results(folder, clusters)
    restored = cache.load(folder)
    assert restored["cluster_results"] == clusters

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

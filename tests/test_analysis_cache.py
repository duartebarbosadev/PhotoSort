import os
import threading
from unittest.mock import MagicMock, Mock

from src.core.caching.analysis_cache import (
    MANUAL_OVERRIDE_NAMESPACE_CULL,
    MANUAL_OVERRIDE_NAMESPACE_SIMILARITY,
    AnalysisCache,
)


def _mock_backed_cache(entry):
    """Build a cache whose single folder entry is served by a mock backend."""

    cache = AnalysisCache.__new__(AnalysisCache)
    cache._cache = MagicMock()
    cache._lock = threading.RLock()
    cache.load = Mock(return_value=entry)
    return cache


def test_analysis_cache_persists_clusters(tmp_path):
    cache = AnalysisCache(str(tmp_path / "analysis_cache"))
    folder = os.path.join("/tmp", "photosort", "session")
    clusters = {
        "/tmp/photosort/session/img1.jpg": 1,
        "/tmp/photosort/session/img2.jpg": 1,
        "/tmp/photosort/session/img3.jpg": 2,
    }

    cache.save_cluster_results(folder, clusters, signature="signature")

    assert cache.load(folder)["cluster_results"] == clusters
    assert (
        cache.load_valid_cluster_results(
            folder,
            signature="signature",
            expected_paths=set(clusters),
        )
        == clusters
    )
    cache.close()


def test_analysis_cache_ignores_obsolete_best_shot_keys():
    cache = AnalysisCache.__new__(AnalysisCache)
    cache._cache = MagicMock()
    cache._cache.get.return_value = {
        "cluster_results": {"a.jpg": 1},
        "best_shot_rankings": {"1": [{"image_path": "a.jpg"}]},
        "best_shot_scores_by_path": {"a.jpg": {"score": 1}},
        "best_shot_winners": {"1": {"image_path": "a.jpg"}},
        "pick_best_results": {"a.jpg": 0.9},
    }

    restored = cache.load("/photos")

    assert restored == {
        "cluster_results": {"a.jpg": 1},
        "pick_best_results": {"a.jpg": 0.9},
    }


def test_analysis_cache_rejects_unsigned_partial_or_changed_clusters(tmp_path):
    cache = AnalysisCache(str(tmp_path / "analysis_cache"))
    folder = "/tmp/photos"
    clusters = {"a.jpg": 1, "b.jpg": 1}
    cache.save_cluster_results(folder, clusters, signature="current")

    assert (
        cache.load_valid_cluster_results(
            folder, signature="changed", expected_paths=set(clusters)
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
    cache.save_cluster_results(folder, {"a.jpg": 1}, signature="current")
    cache.save_manual_cluster_override(folder, "a.jpg", 8)

    cache.invalidate_similarity(folder)

    entry = cache.load(folder)
    assert entry["manual_cluster_overrides"] == {"a.jpg": 8}
    assert "cluster_results" not in entry
    assert "similarity_signature" not in entry


def test_cull_grouping_state_round_trips_and_is_invalidated_centrally(tmp_path):
    cache = AnalysisCache(str(tmp_path / "analysis_cache"))
    folder = "/tmp/photos"
    cache.save_cull_grouping_state(
        folder,
        artifacts={"a.jpg": {"fingerprint": [1, 2]}},
        pair_verifications={"a.jpg\0b.jpg": {"accepted": True}},
        clusters={"a.jpg": 1, "b.jpg": 1},
        grouping_signature="group-v1",
        model_signature="models-v1",
        pair_pipeline_version="pairs-v1",
        pair_context_signature="times-v1",
    )

    restored = cache.load_cull_grouping_state(folder)
    assert restored["cull_cluster_results"] == {"a.jpg": 1, "b.jpg": 1}
    assert restored["cull_grouping_signature"] == "group-v1"
    assert restored["cull_pair_context_signature"] == "times-v1"

    cache.save_manual_cluster_override(folder, "a.jpg", 9)
    cache.invalidate_similarity(folder)
    invalidated = cache.load(folder)
    assert invalidated["manual_cluster_overrides"] == {"a.jpg": 9}
    assert "cull_subject_artifacts" not in invalidated
    assert "cull_pair_verifications" not in invalidated
    assert "cull_cluster_results" not in invalidated


def test_cull_artifact_checkpoint_never_publishes_partial_clusters(tmp_path):
    cache = AnalysisCache(str(tmp_path / "analysis_cache"))
    folder = "/tmp/photos"
    cache.save_cull_grouping_state(
        folder,
        artifacts={"old.jpg": {}},
        pair_verifications={"old.jpg\0other.jpg": {}},
        clusters={"old.jpg": 1},
        grouping_signature="old-group",
        model_signature="old-model",
        pair_pipeline_version="old-pairs",
        pair_context_signature="old-times",
    )

    cache.merge_cull_artifacts_checkpoint(
        folder,
        artifacts={"new.jpg": {"fingerprint": [1, 2]}},
        model_signature="new-model",
    )

    restored = cache.load_cull_grouping_state(folder)
    assert restored["cull_subject_artifacts"] == {"new.jpg": {"fingerprint": [1, 2]}}
    assert restored["cull_model_signature"] == "new-model"
    assert "cull_cluster_results" not in restored
    assert "cull_pair_verifications" not in restored
    assert "cull_pair_context_signature" not in restored
    cache.close()


def test_cull_artifact_checkpoint_merges_only_new_artifacts(tmp_path):
    """Checkpoints must stay O(new artifacts), not rewrite the whole set."""

    cache = AnalysisCache(str(tmp_path / "analysis_cache"))
    folder = "/tmp/photos"

    cache.merge_cull_artifacts_checkpoint(
        folder, artifacts={"a.jpg": {"n": 1}}, model_signature="model"
    )
    cache.merge_cull_artifacts_checkpoint(
        folder, artifacts={"b.jpg": {"n": 2}}, model_signature="model"
    )

    restored = cache.load_cull_grouping_state(folder)
    assert restored["cull_subject_artifacts"] == {"a.jpg": {"n": 1}, "b.jpg": {"n": 2}}
    cache.close()


def test_manual_overrides_are_isolated_per_cluster_namespace(tmp_path):
    """Cull and similarity cluster ids are unrelated and must not leak across."""

    cache = AnalysisCache(str(tmp_path / "analysis_cache"))
    folder = "/tmp/photos"

    cache.save_manual_cluster_overrides(
        folder, {"a.jpg": 3}, namespace=MANUAL_OVERRIDE_NAMESPACE_SIMILARITY
    )
    cache.save_manual_cluster_overrides(
        folder, {"a.jpg": 90}, namespace=MANUAL_OVERRIDE_NAMESPACE_CULL
    )

    assert cache.get_manual_overrides(
        folder, namespace=MANUAL_OVERRIDE_NAMESPACE_SIMILARITY
    ) == {"a.jpg": 3}
    assert cache.get_manual_overrides(
        folder, namespace=MANUAL_OVERRIDE_NAMESPACE_CULL
    ) == {"a.jpg": 90}

    entry = cache.load(folder)
    assert entry["cluster_results"] == {"a.jpg": 3}
    assert entry["cull_cluster_results"] == {"a.jpg": 90}

    cache.clear_all_manual_overrides(folder, namespace=MANUAL_OVERRIDE_NAMESPACE_CULL)
    assert cache.get_manual_overrides(
        folder, namespace=MANUAL_OVERRIDE_NAMESPACE_SIMILARITY
    ) == {"a.jpg": 3}
    assert not cache.get_manual_overrides(
        folder, namespace=MANUAL_OVERRIDE_NAMESPACE_CULL
    )
    cache.close()


def test_concurrent_mutations_do_not_lose_updates(tmp_path):
    """A long checkpoint run and a deletion must not overwrite each other."""

    cache = AnalysisCache(str(tmp_path / "analysis_cache"))
    folder = "/tmp/photos"
    cache.merge_cull_artifacts_checkpoint(
        folder,
        artifacts={f"seed{index}.jpg": {"n": index} for index in range(20)},
        model_signature="model",
    )

    def checkpoint_writer() -> None:
        for index in range(40):
            cache.merge_cull_artifacts_checkpoint(
                folder,
                artifacts={f"new{index}.jpg": {"n": index}},
                model_signature="model",
            )

    def deleter() -> None:
        for index in range(20):
            cache.remove_paths(folder, {f"seed{index}.jpg"})

    threads = [
        threading.Thread(target=checkpoint_writer),
        threading.Thread(target=deleter),
    ]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()

    artifacts = cache.load(folder)["cull_subject_artifacts"]
    assert not any(path.startswith("seed") for path in artifacts)
    assert len(artifacts) == 40
    cache.close()


def test_analysis_cache_migrates_owned_path_mappings():
    old = "/photos/source/a.jpg"
    new = "/photos/output/a.jpg"
    cache = _mock_backed_cache(
        {
            "cluster_results": {old: 1},
            "cull_cluster_results": {old: 3},
            "cull_subject_artifacts": {old: {"path": old}},
            "cull_pair_verifications": {
                f"{old}\0/zz-other.jpg": {
                    "path_a": old,
                    "path_b": "/zz-other.jpg",
                    "accepted": True,
                }
            },
            "cull_grouping_signature": "stale",
            "manual_cluster_overrides": {old: 2},
            "cull_manual_cluster_overrides": {old: 5},
            "subject_descriptors": {old: {"descriptor": {}}},
        }
    )
    cache._cache.__contains__ = Mock(return_value=True)

    cache.migrate_folder_paths("/photos/source", "/photos/output", {old: new})

    saved = cache._cache.set.call_args.args[1]
    assert saved["cluster_results"] == {new: 1}
    assert saved["cull_cluster_results"] == {new: 3}
    assert saved["cull_subject_artifacts"] == {new: {"path": new}}
    assert saved["cull_pair_verifications"] == {
        f"{new}\0/zz-other.jpg": {
            "path_a": new,
            "path_b": "/zz-other.jpg",
            "accepted": True,
        }
    }
    assert "cull_grouping_signature" not in saved
    assert saved["manual_cluster_overrides"] == {new: 2}
    assert saved["cull_manual_cluster_overrides"] == {new: 5}
    assert saved["subject_descriptors"] == {new: {"descriptor": {}}}
    cache._cache.__delitem__.assert_called_once()


def test_analysis_cache_removes_deleted_paths_with_one_read_and_write():
    removed = "/photos/deleted.jpg"
    kept = "/photos/kept.jpg"
    cache = _mock_backed_cache(
        {
            "cluster_results": {removed: 1, kept: 1},
            "cull_cluster_results": {removed: 4, kept: 4},
            "cull_subject_artifacts": {removed: {}, kept: {}},
            "cull_pair_verifications": {f"{removed}\0{kept}": {"accepted": True}},
            "cull_grouping_signature": "stale",
            "manual_cluster_overrides": {removed: 1},
            "cull_manual_cluster_overrides": {removed: 4},
            "subject_descriptors": {removed: {}, kept: {}},
        }
    )

    cache.remove_paths("/photos", {removed})

    cache.load.assert_called_once_with("/photos")
    cache._cache.set.assert_called_once()
    saved = cache._cache.set.call_args.args[1]
    assert saved["cluster_results"] == {kept: 1}
    assert saved["cull_cluster_results"] == {kept: 4}
    assert saved["cull_subject_artifacts"] == {kept: {}}
    assert saved["cull_pair_verifications"] == {}
    assert "cull_grouping_signature" not in saved
    assert saved["manual_cluster_overrides"] == {}
    assert saved["cull_manual_cluster_overrides"] == {}
    assert saved["subject_descriptors"] == {kept: {}}


def test_similarity_invalidation_keeps_untouched_photos_expensive_artifacts(tmp_path):
    """Rotating one photo must not force re-encoding of every other photo."""

    cache = AnalysisCache(str(tmp_path / "analysis_cache"))
    folder = "/tmp/photos"
    cache.save_cull_grouping_state(
        folder,
        artifacts={"a.jpg": {"fingerprint": [1, 2]}, "b.jpg": {"fingerprint": [3, 4]}},
        pair_verifications={
            "a.jpg\0b.jpg": {"accepted": True},
            "b.jpg\0c.jpg": {"accepted": False},
        },
        clusters={"a.jpg": 1, "b.jpg": 1},
        grouping_signature="group-v1",
        model_signature="models-v1",
        pair_pipeline_version="pairs-v1",
        pair_context_signature="times-v1",
    )
    cache.save_cluster_results(folder, {"a.jpg": 1, "b.jpg": 1}, signature="sig-v1")

    cache.invalidate_similarity(folder, changed_paths=["a.jpg"])

    entry = cache.load(folder)
    assert entry["cull_subject_artifacts"] == {"b.jpg": {"fingerprint": [3, 4]}}
    assert set(entry["cull_pair_verifications"]) == {"b.jpg\0c.jpg"}
    # Reusable work survives, so the model signature must stay valid.
    assert entry["cull_model_signature"] == "models-v1"
    assert entry["cull_pair_pipeline_version"] == "pairs-v1"
    assert entry["cull_pair_context_signature"] == "times-v1"
    # Group assignments depend on the whole set and are always discarded.
    assert "cull_cluster_results" not in entry
    assert "cluster_results" not in entry
    assert "similarity_signature" not in entry


def test_similarity_invalidation_without_changed_paths_still_clears_everything(
    tmp_path,
):
    cache = AnalysisCache(str(tmp_path / "analysis_cache"))
    folder = "/tmp/photos"
    cache.save_cull_grouping_state(
        folder,
        artifacts={"a.jpg": {}},
        pair_verifications={"a.jpg\0b.jpg": {}},
        clusters={"a.jpg": 1},
        grouping_signature="group-v1",
        model_signature="models-v1",
        pair_pipeline_version="pairs-v1",
        pair_context_signature="times-v1",
    )

    cache.invalidate_similarity(folder)

    entry = cache.load(folder)
    for field in (
        "cull_subject_artifacts",
        "cull_pair_verifications",
        "cull_model_signature",
        "cull_pair_pipeline_version",
        "cull_pair_context_signature",
    ):
        assert field not in entry

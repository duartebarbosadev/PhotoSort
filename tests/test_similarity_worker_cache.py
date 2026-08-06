from types import SimpleNamespace
from unittest.mock import Mock

from core.similarity_cache import (
    SimilarityClusteringResult,
    normalize_cluster_results,
)
from ui.ui_components import SimilarityWorker


class _Signal:
    def __init__(self):
        self.callbacks = []

    def connect(self, callback):
        self.callbacks.append(callback)

    def emit(self, *args):
        for callback in list(self.callbacks):
            if isinstance(callback, _Signal):
                callback.emit(*args)
            elif hasattr(callback, "emit"):
                callback.emit(*args)
            else:
                callback(*args)


class _FakeEngine:
    last_instance = None

    def __init__(self, **_kwargs):
        type(self).last_instance = self
        self.model = SimpleNamespace(
            cache_key="global-v1",
            region_cache_key="regional-v1",
        )
        self.progress_update = _Signal()
        self.embeddings_generated = _Signal()
        self.regional_embeddings_generated = _Signal()
        self.clustering_complete = _Signal()
        self.error = _Signal()
        self.calls = []

    def stop(self):
        pass

    def generate_embeddings_for_files(self, paths, **kwargs):
        self.calls.append((paths, kwargs))
        self.embeddings_generated.emit({path: [1.0, 0.0] for path in paths})
        self.regional_embeddings_generated.emit({path: [[1.0, 0.0]] for path in paths})
        if kwargs["perform_clustering"]:
            self.clustering_complete.emit({path: 1 for path in paths})


def test_cluster_results_normalize_legacy_and_manual_assignment_values():
    assert normalize_cluster_results(
        {
            "legacy.jpg": "1 - 87.34%",
            "manual.jpg": 2,
            "invalid.jpg": "unknown",
        }
    ) == {"legacy.jpg": 1, "manual.jpg": 2}


def test_worker_hydrates_artifacts_and_reuses_valid_clusters(monkeypatch):
    monkeypatch.setattr("core.similarity_engine.SimilarityEngine", _FakeEngine)
    analysis_cache = Mock()
    analysis_cache.load_valid_cluster_results.return_value = {"photo.jpg": 7}
    worker = SimilarityWorker(
        ["photo.jpg"],
        folder_path="/photos",
        analysis_cache=analysis_cache,
        fingerprints={"photo.jpg": (10, 20)},
    )
    completed = []
    worker.clustering_complete.connect(completed.append)

    worker.run()

    engine = _FakeEngine.last_instance
    assert engine.calls[0][1]["perform_clustering"] is False
    assert completed == [
        SimilarityClusteringResult(
            clusters={"photo.jpg": 7},
            signature=completed[0].signature,
            reused=True,
        )
    ]
    analysis_cache.save_cluster_results.assert_not_called()


def test_worker_persists_fresh_clusters_with_signature(monkeypatch):
    monkeypatch.setattr("core.similarity_engine.SimilarityEngine", _FakeEngine)
    analysis_cache = Mock()
    analysis_cache.load_valid_cluster_results.return_value = None
    analysis_cache.get_manual_overrides.return_value = {"photo.jpg": 9}
    worker = SimilarityWorker(
        ["photo.jpg"],
        folder_path="/photos",
        analysis_cache=analysis_cache,
        fingerprints={"photo.jpg": (10, 20)},
    )
    completed = []
    worker.clustering_complete.connect(completed.append)

    worker.run()

    result = completed[0]
    assert result.clusters == {"photo.jpg": 9}
    assert result.reused is False
    analysis_cache.save_cluster_results.assert_called_once_with(
        "/photos",
        {"photo.jpg": 9},
        signature=result.signature,
    )

from unittest.mock import Mock

import numpy as np
from PIL import Image
import pytest

from core.similarity_cache import (
    SimilarityArtifactCacheFormatError,
    build_similarity_signature,
    load_similarity_artifact_cache,
    save_similarity_artifact_cache,
)
from core.similarity_engine import SimilarityEngine


def _artifact(fingerprint=(10, 20), orientation="landscape"):
    return {
        "fingerprint": fingerprint,
        "embedding": [1.0, 0.0],
        "regional_embeddings": [[1.0, 0.0]] * 6,
        "orientation": orientation,
    }


def test_similarity_artifact_cache_round_trip(tmp_path):
    cache_path = tmp_path / "artifacts.pkl.zst"
    artifacts = {"photo.jpg": _artifact()}

    save_similarity_artifact_cache(cache_path, artifacts)

    assert load_similarity_artifact_cache(cache_path) == artifacts


def test_similarity_artifact_cache_rejects_legacy_payload(tmp_path):
    from compression import zstd
    import pickle

    cache_path = tmp_path / "legacy.pkl.zst"
    with zstd.open(cache_path, "wb") as cache_file:
        pickle.dump({"format_version": 0, "data": {}}, cache_file)

    with pytest.raises(
        SimilarityArtifactCacheFormatError,
        match="unsupported similarity artifact cache version",
    ):
        load_similarity_artifact_cache(cache_path)


def test_similarity_signature_covers_inputs_and_settings():
    paths = ["b.jpg", "a.jpg"]
    fingerprints = {"a.jpg": (1, 2), "b.jpg": (3, 4)}
    base = build_similarity_signature(
        paths,
        fingerprints,
        model_cache_key="model-v1",
        regional_cache_key="regions-v1",
        clustering_eps=0.05,
        min_samples=2,
    )
    reordered = build_similarity_signature(
        list(reversed(paths)),
        fingerprints,
        model_cache_key="model-v1",
        regional_cache_key="regions-v1",
        clustering_eps=0.05,
        min_samples=2,
    )
    changed = build_similarity_signature(
        paths,
        {**fingerprints, "a.jpg": (1, 3)},
        model_cache_key="model-v1",
        regional_cache_key="regions-v1",
        clustering_eps=0.05,
        min_samples=2,
    )

    assert reordered == base
    assert changed != base


def test_warm_artifact_run_skips_model_decode_and_clustering(tmp_path):
    cache_path = tmp_path / "artifacts.pkl.zst"
    save_similarity_artifact_cache(cache_path, {"photo.jpg": _artifact()})
    pipeline = Mock()
    engine = SimilarityEngine(image_pipeline=pipeline)
    engine._cache_path = cache_path
    engine._load_model = Mock(return_value=True)
    engine.cluster_embeddings = Mock()
    emitted_embeddings = []
    emitted_regions = []
    engine.embeddings_generated.connect(emitted_embeddings.append)
    engine.regional_embeddings_generated.connect(emitted_regions.append)

    engine.generate_embeddings_for_files(
        ["photo.jpg"],
        fingerprints={"photo.jpg": (10, 20)},
        perform_clustering=False,
    )

    engine._load_model.assert_not_called()
    pipeline.get_analysis_image.assert_not_called()
    engine.cluster_embeddings.assert_not_called()
    assert emitted_embeddings == [{"photo.jpg": [1.0, 0.0]}]
    assert emitted_regions == [{"photo.jpg": [[1.0, 0.0]] * 6}]


def test_changed_fingerprint_reencodes_only_changed_artifact(tmp_path):
    cache_path = tmp_path / "artifacts.pkl.zst"
    save_similarity_artifact_cache(
        cache_path,
        {
            "unchanged.jpg": _artifact((10, 20)),
            "changed.jpg": _artifact((30, 40)),
        },
    )
    pipeline = Mock()
    pipeline.get_analysis_image.return_value = Image.new("RGB", (80, 120), "blue")
    engine = SimilarityEngine(image_pipeline=pipeline)
    engine._cache_path = cache_path
    engine._load_model = Mock(return_value=True)
    engine.model.encode_with_regions = Mock(
        return_value=(
            np.asarray([[1.0, 0.0]], dtype=np.float32),
            [np.asarray([[1.0, 0.0]] * 6, dtype=np.float32)],
        )
    )

    engine.generate_embeddings_for_files(
        ["unchanged.jpg", "changed.jpg"],
        fingerprints={
            "unchanged.jpg": (10, 20),
            "changed.jpg": (30, 41),
        },
        perform_clustering=False,
    )

    engine._load_model.assert_called_once_with()
    pipeline.get_analysis_image.assert_called_once()
    assert pipeline.get_analysis_image.call_args.args[0] == "changed.jpg"
    saved = load_similarity_artifact_cache(cache_path)
    assert tuple(saved["changed.jpg"]["fingerprint"]) == (30, 41)
    assert saved["changed.jpg"]["orientation"] == "portrait"

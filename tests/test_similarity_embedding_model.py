import pyexiv2  # noqa: F401  # Must be first to avoid Windows crash

import json
import types

import numpy as np
import pytest

from core.model_provisioning import EMBEDDING_MODEL
from core.similarity_embedding_model import (
    SimilarityEmbeddingModel,
    SimilarityModelNotInstalledError,
    build_similarity_image_regions,
    normalize_similarity_model_name,
    resolve_similarity_model_snapshot,
)


pytestmark = pytest.mark.usefixtures("inline_model_download")


def _install_downloader(monkeypatch, downloader) -> None:
    monkeypatch.setattr(
        "core.model_provisioning._snapshot_download", lambda: downloader
    )


def _valid_snapshot(root) -> str:
    """Build a snapshot that satisfies the shared model validator."""

    path = root / "snapshot"
    path.mkdir()
    (path / "config.json").write_text(json.dumps({"model_type": "dinov2"}), "utf-8")
    (path / "preprocessor_config.json").write_text("{}", encoding="utf-8")
    (path / "model.safetensors").write_bytes(b"weights-placeholder")
    return str(path)


def test_snapshot_resolution_delegates_to_the_shared_pinned_model(
    monkeypatch, tmp_path
):
    calls = []
    snapshot_path = _valid_snapshot(tmp_path)

    def downloader(repo_id, **kwargs):
        calls.append((repo_id, kwargs))
        return snapshot_path

    _install_downloader(monkeypatch, downloader)

    assert resolve_similarity_model_snapshot() == snapshot_path
    repo_id, kwargs = calls[0]
    assert repo_id == EMBEDDING_MODEL.repo_id
    assert kwargs["revision"] == EMBEDDING_MODEL.revision
    assert kwargs["local_files_only"] is True


def test_missing_snapshot_without_download_raises_clear_error(monkeypatch):
    def downloader(*_args, **_kwargs):
        raise FileNotFoundError("missing")

    _install_downloader(monkeypatch, downloader)

    with pytest.raises(
        SimilarityModelNotInstalledError, match="has not been downloaded yet"
    ):
        resolve_similarity_model_snapshot()


def test_approved_download_retries_online(monkeypatch, tmp_path):
    calls = []
    snapshot_path = _valid_snapshot(tmp_path)
    progress_events = []

    def downloader(repo_id, **kwargs):
        calls.append(kwargs)
        if kwargs["local_files_only"]:
            raise FileNotFoundError("missing")
        progress = kwargs["tqdm_class"](total=10, unit="B", desc="Downloading")
        progress.update(5)
        progress.close()
        return snapshot_path

    _install_downloader(monkeypatch, downloader)

    resolved = resolve_similarity_model_snapshot(
        allow_download=True,
        progress_callback=lambda percent, message: progress_events.append(
            (percent, message)
        ),
    )

    assert resolved == snapshot_path
    assert [call["local_files_only"] for call in calls] == [True, False]
    assert "tqdm_class" in calls[1]
    assert any(percent == 50 for percent, _message in progress_events)


def test_legacy_model_names_collapse_onto_the_single_model():
    """The app ships one embedding model, so stale settings must not fork it."""

    assert normalize_similarity_model_name("facebook/dinov2-base") == (
        EMBEDDING_MODEL.repo_id
    )
    assert normalize_similarity_model_name(None) == EMBEDDING_MODEL.repo_id

    legacy = SimilarityEmbeddingModel("facebook/dinov2-base")
    current = SimilarityEmbeddingModel()

    assert legacy.cache_key == current.cache_key
    assert legacy.region_cache_key == current.region_cache_key
    assert "dinov2-cls-v1" in current.cache_key
    assert "dinov2-regions-v1" in current.region_cache_key


def test_encode_returns_normalized_cls_embeddings():
    torch = pytest.importorskip("torch")

    class FakeProcessor:
        def __call__(self, images, return_tensors):
            assert return_tensors == "pt"
            return {"pixel_values": torch.zeros((len(images), 3, 224, 224))}

    class FakeOutputs:
        last_hidden_state = torch.tensor(
            [
                [[3.0, 4.0, 0.0], [0.0, 0.0, 0.0]],
                [[0.0, 5.0, 0.0], [0.0, 0.0, 0.0]],
            ]
        )

    class FakeModel:
        def __call__(self, **_inputs):
            return FakeOutputs()

    model = SimilarityEmbeddingModel()
    model.processor = FakeProcessor()
    model.model = FakeModel()
    model.device = "cpu"

    embeddings = model.encode([object(), object()])

    assert embeddings.shape == (2, 3)
    assert np.allclose(np.linalg.norm(embeddings, axis=1), [1.0, 1.0])


def test_encode_with_regions_returns_global_and_regional_embeddings():
    torch = pytest.importorskip("torch")
    Image = pytest.importorskip("PIL.Image")

    class FakeProcessor:
        def __call__(self, images, return_tensors):
            assert return_tensors == "pt"
            return {"pixel_values": torch.zeros((len(images), 3, 224, 224))}

    class FakeModel:
        def __call__(self, **inputs):
            count = inputs["pixel_values"].shape[0]
            vectors = torch.zeros((count, 1, 3), dtype=torch.float32)
            vectors[:, 0, 0] = torch.arange(1, count + 1, dtype=torch.float32)
            vectors[:, 0, 1] = 1.0
            return types.SimpleNamespace(last_hidden_state=vectors)

    images = [Image.new("RGB", (100, 80)), Image.new("RGB", (120, 90))]
    model = SimilarityEmbeddingModel()
    model.processor = FakeProcessor()
    model.model = FakeModel()
    model.device = "cpu"

    global_embeddings, regional_embeddings = model.encode_with_regions(images)

    assert len(build_similarity_image_regions(images[0])) == 6
    assert global_embeddings.shape == (2, 3)
    assert len(regional_embeddings) == 2
    assert regional_embeddings[0].shape == (6, 3)
    assert regional_embeddings[1].shape == (6, 3)
    assert np.allclose(global_embeddings[0], regional_embeddings[0][0])
    assert np.allclose(global_embeddings[1], regional_embeddings[1][0])
    assert np.allclose(np.linalg.norm(global_embeddings, axis=1), [1.0, 1.0])

import sys
import types

import numpy as np
import pytest

from core.similarity_embedding_model import (
    SimilarityEmbeddingModel,
    SimilarityModelNotInstalledError,
    build_similarity_image_regions,
    resolve_similarity_model_snapshot,
)


def test_local_snapshot_resolution_uses_local_files_only(monkeypatch, tmp_path):
    calls = []
    snapshot_path = tmp_path / "snapshot"

    def fake_snapshot_download(model_name, **kwargs):
        calls.append((model_name, kwargs))
        return str(snapshot_path)

    monkeypatch.setitem(
        sys.modules,
        "huggingface_hub",
        types.SimpleNamespace(snapshot_download=fake_snapshot_download),
    )
    monkeypatch.setattr(
        "core.similarity_embedding_model.get_huggingface_cache_dir",
        lambda: str(tmp_path / "hf"),
    )

    resolved = resolve_similarity_model_snapshot("facebook/dinov2-small")

    assert resolved == str(snapshot_path)
    assert calls == [
        (
            "facebook/dinov2-small",
            {"cache_dir": str(tmp_path / "hf"), "local_files_only": True},
        )
    ]


def test_missing_snapshot_without_download_raises_clear_error(monkeypatch, tmp_path):
    def fake_snapshot_download(*_args, **_kwargs):
        raise FileNotFoundError("missing")

    monkeypatch.setitem(
        sys.modules,
        "huggingface_hub",
        types.SimpleNamespace(snapshot_download=fake_snapshot_download),
    )
    monkeypatch.setattr(
        "core.similarity_embedding_model.get_huggingface_cache_dir",
        lambda: str(tmp_path / "hf"),
    )

    with pytest.raises(SimilarityModelNotInstalledError, match="not installed locally"):
        resolve_similarity_model_snapshot("facebook/dinov2-small")


def test_approved_download_retries_online(monkeypatch, tmp_path):
    calls = []
    snapshot_path = tmp_path / "snapshot"
    progress_events = []

    def fake_snapshot_download(model_name, **kwargs):
        calls.append((model_name, kwargs))
        if kwargs["local_files_only"]:
            raise FileNotFoundError("missing")
        progress = kwargs["tqdm_class"](total=10, unit="B", desc="Downloading")
        progress.update(5)
        progress.close()
        return str(snapshot_path)

    monkeypatch.setitem(
        sys.modules,
        "huggingface_hub",
        types.SimpleNamespace(snapshot_download=fake_snapshot_download),
    )
    monkeypatch.setattr(
        "core.similarity_embedding_model.get_huggingface_cache_dir",
        lambda: str(tmp_path / "hf"),
    )

    resolved = resolve_similarity_model_snapshot(
        "facebook/dinov2-small",
        allow_download=True,
        progress_callback=lambda percent, message: progress_events.append(
            (percent, message)
        ),
    )

    assert resolved == str(snapshot_path)
    assert [call[1]["local_files_only"] for call in calls] == [True, False]
    assert "tqdm_class" in calls[1][1]
    assert any(percent == 50 for percent, _message in progress_events)


def test_embedding_cache_key_changes_with_model_name():
    small = SimilarityEmbeddingModel("facebook/dinov2-small")
    base = SimilarityEmbeddingModel("facebook/dinov2-base")

    assert small.cache_key != base.cache_key
    assert small.region_cache_key != base.region_cache_key
    assert "dinov2-cls-v1" in small.cache_key
    assert "dinov2-regions-v1" in small.region_cache_key


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

    model = SimilarityEmbeddingModel("facebook/dinov2-small")
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
    model = SimilarityEmbeddingModel("facebook/dinov2-small")
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

from pathlib import Path

import pytest

from core.embedding_cache import (
    EmbeddingCacheFormatError,
    load_embedding_cache,
    save_embedding_cache,
)


def test_zstd_embedding_cache_round_trip_and_atomic_cleanup(tmp_path):
    cache_path = tmp_path / "embeddings.pkl.zst"
    data = {"photo.jpg": [0.1, 0.2, 0.3]}

    save_embedding_cache(cache_path, data, kind="global")

    assert load_embedding_cache(cache_path, kind="global") == data
    assert not list(tmp_path.glob("*.tmp"))


def test_embedding_cache_rejects_wrong_kind(tmp_path):
    cache_path = tmp_path / "embeddings.pkl.zst"
    save_embedding_cache(cache_path, {}, kind="regional")

    with pytest.raises(EmbeddingCacheFormatError, match="expected 'global'"):
        load_embedding_cache(cache_path, kind="global")


def test_failed_atomic_write_preserves_existing_cache(monkeypatch, tmp_path):
    cache_path = tmp_path / "embeddings.pkl.zst"
    original = {"original.jpg": [1.0]}
    save_embedding_cache(cache_path, original, kind="global")

    def broken_dump(*_args, **_kwargs):
        raise OSError("disk full")

    monkeypatch.setattr("core.embedding_cache.pickle.dump", broken_dump)

    with pytest.raises(OSError, match="disk full"):
        save_embedding_cache(cache_path, {"new.jpg": [2.0]}, kind="global")

    assert load_embedding_cache(cache_path, kind="global") == original
    assert not list(Path(tmp_path).glob("*.tmp"))

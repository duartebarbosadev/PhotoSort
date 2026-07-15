from compression import zstd
from pathlib import Path
import pickle
import tempfile
from typing import Any


EMBEDDING_CACHE_FORMAT_VERSION = 1
EMBEDDING_CACHE_COMPRESSION_LEVEL = 3


class EmbeddingCacheFormatError(ValueError):
    """Raised when an embedding cache has an unknown or invalid schema."""


def load_embedding_cache(path: Path, *, kind: str) -> Any:
    """Load and validate one versioned Zstandard embedding cache."""

    with zstd.open(path, "rb") as cache_file:
        payload = pickle.load(cache_file)
    if not isinstance(payload, dict):
        raise EmbeddingCacheFormatError("cache payload is not a dictionary")
    if payload.get("format_version") != EMBEDDING_CACHE_FORMAT_VERSION:
        raise EmbeddingCacheFormatError("unsupported embedding cache version")
    if payload.get("kind") != kind:
        raise EmbeddingCacheFormatError(
            f"expected {kind!r} cache, found {payload.get('kind')!r}"
        )
    if "data" not in payload:
        raise EmbeddingCacheFormatError("embedding cache has no data field")
    return payload["data"]


def save_embedding_cache(path: Path, data: Any, *, kind: str) -> None:
    """Atomically write one versioned Zstandard embedding cache."""

    path.parent.mkdir(parents=True, exist_ok=True)
    payload = {
        "format_version": EMBEDDING_CACHE_FORMAT_VERSION,
        "kind": kind,
        "data": data,
    }
    temporary_path: Path | None = None
    try:
        with tempfile.NamedTemporaryFile(
            dir=path.parent,
            prefix=f".{path.name}.",
            suffix=".tmp",
            delete=False,
        ) as temporary_file:
            temporary_path = Path(temporary_file.name)
        with zstd.open(
            temporary_path,
            "wb",
            level=EMBEDDING_CACHE_COMPRESSION_LEVEL,
        ) as cache_file:
            pickle.dump(payload, cache_file, protocol=pickle.HIGHEST_PROTOCOL)
        temporary_path.replace(path)
    finally:
        if temporary_path is not None:
            temporary_path.unlink(missing_ok=True)

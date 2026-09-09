"""Versioned cache records and signatures for the shared similarity pipeline."""

from __future__ import annotations

from dataclasses import dataclass
import hashlib
import json
from pathlib import Path
import pickle
import tempfile
from collections.abc import Mapping
from typing import Literal, TypedDict

from compression import zstd


SIMILARITY_ARTIFACT_CACHE_VERSION = 1
SIMILARITY_CLUSTERING_PIPELINE_VERSION = "regional-dbscan-v2"
SIMILARITY_ORIENTATION_PIPELINE_VERSION = "visual-orientation-v1"
SIMILARITY_ARTIFACT_CACHE_COMPRESSION_LEVEL = 3

FileFingerprint = tuple[int, int]
CachedOrientation = Literal["portrait", "landscape", "square"]


class SimilarityArtifact(TypedDict):
    fingerprint: FileFingerprint
    embedding: list[float]
    regional_embeddings: list[list[float]]
    orientation: CachedOrientation


class SimilarityArtifactCacheFormatError(ValueError):
    """Raised when a similarity artifact cache has an unsupported schema."""


@dataclass(frozen=True)
class SimilarityClusteringResult:
    clusters: dict[str, int]
    signature: str
    reused: bool = False


def parse_cluster_id(value: object) -> int | None:
    """Return the numeric ID from current or legacy cluster assignments."""

    if isinstance(value, bool):
        return None
    if isinstance(value, int):
        return value
    if isinstance(value, str):
        try:
            return int(value.split(" - ", 1)[0].strip())
        except ValueError:
            return None
    return None


def normalize_cluster_results(clusters: object) -> dict[str, int]:
    """Normalize a cluster mapping to the application-wide integer contract."""

    if not isinstance(clusters, Mapping):
        return {}
    normalized: dict[str, int] = {}
    for path, value in clusters.items():
        cluster_id = parse_cluster_id(value)
        if path and cluster_id is not None:
            normalized[str(path)] = cluster_id
    return normalized


def fingerprint_path(path: str) -> FileFingerprint | None:
    try:
        stat_result = Path(path).stat()
    except OSError:
        return None
    return int(stat_result.st_size), int(stat_result.st_mtime_ns)


def normalize_fingerprints(
    file_paths: list[str],
    fingerprints: dict[str, FileFingerprint] | None = None,
) -> dict[str, FileFingerprint]:
    """Return valid fingerprints for requested paths, statting as a fallback."""

    supplied = fingerprints or {}
    normalized: dict[str, FileFingerprint] = {}
    for path in file_paths:
        value = supplied.get(path)
        if (
            isinstance(value, (tuple, list))
            and len(value) == 2
            and all(isinstance(item, int) for item in value)
        ):
            normalized[path] = int(value[0]), int(value[1])
            continue
        discovered = fingerprint_path(path)
        if discovered is not None:
            normalized[path] = discovered
    return normalized


def build_similarity_signature(
    file_paths: list[str],
    fingerprints: dict[str, FileFingerprint],
    *,
    model_cache_key: str,
    regional_cache_key: str,
    clustering_eps: float,
    min_samples: int,
) -> str:
    payload = {
        "clustering_pipeline": SIMILARITY_CLUSTERING_PIPELINE_VERSION,
        "orientation_pipeline": SIMILARITY_ORIENTATION_PIPELINE_VERSION,
        "model_cache_key": model_cache_key,
        "regional_cache_key": regional_cache_key,
        "clustering_eps": float(clustering_eps),
        "min_samples": int(min_samples),
        "files": [
            [path, *fingerprints.get(path, (-1, -1))] for path in sorted(file_paths)
        ],
    }
    encoded = json.dumps(
        payload, ensure_ascii=False, separators=(",", ":"), sort_keys=True
    ).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def _validate_artifact(path: str, artifact: object) -> SimilarityArtifact:
    if not isinstance(artifact, dict):
        raise SimilarityArtifactCacheFormatError(
            f"artifact for {path!r} is not a dictionary"
        )
    fingerprint = artifact.get("fingerprint")
    if not (
        isinstance(fingerprint, (tuple, list))
        and len(fingerprint) == 2
        and all(isinstance(item, int) for item in fingerprint)
    ):
        raise SimilarityArtifactCacheFormatError(
            f"artifact for {path!r} has no valid fingerprint"
        )
    if not isinstance(artifact.get("embedding"), list):
        raise SimilarityArtifactCacheFormatError(
            f"artifact for {path!r} has no global embedding"
        )
    if not isinstance(artifact.get("regional_embeddings"), list):
        raise SimilarityArtifactCacheFormatError(
            f"artifact for {path!r} has no regional embeddings"
        )
    if artifact.get("orientation") not in {"portrait", "landscape", "square"}:
        raise SimilarityArtifactCacheFormatError(
            f"artifact for {path!r} has no valid orientation"
        )
    return {
        "fingerprint": (int(fingerprint[0]), int(fingerprint[1])),
        "embedding": list(artifact["embedding"]),
        "regional_embeddings": [
            list(region) for region in artifact["regional_embeddings"]
        ],
        "orientation": artifact["orientation"],
    }


def load_similarity_artifact_cache(path: Path) -> dict[str, SimilarityArtifact]:
    with zstd.open(path, "rb") as cache_file:
        payload = pickle.load(cache_file)
    if not isinstance(payload, dict):
        raise SimilarityArtifactCacheFormatError("cache payload is not a dictionary")
    if payload.get("format_version") != SIMILARITY_ARTIFACT_CACHE_VERSION:
        raise SimilarityArtifactCacheFormatError(
            "unsupported similarity artifact cache version"
        )
    raw_artifacts = payload.get("artifacts")
    if not isinstance(raw_artifacts, dict):
        raise SimilarityArtifactCacheFormatError("cache has no artifacts mapping")
    return {
        str(item_path): _validate_artifact(str(item_path), artifact)
        for item_path, artifact in raw_artifacts.items()
    }


def save_similarity_artifact_cache(
    path: Path, artifacts: dict[str, SimilarityArtifact]
) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    payload = {
        "format_version": SIMILARITY_ARTIFACT_CACHE_VERSION,
        "artifacts": artifacts,
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
            level=SIMILARITY_ARTIFACT_CACHE_COMPRESSION_LEVEL,
        ) as cache_file:
            pickle.dump(payload, cache_file, protocol=pickle.HIGHEST_PROTOCOL)
        temporary_path.replace(path)
    finally:
        if temporary_path is not None:
            temporary_path.unlink(missing_ok=True)

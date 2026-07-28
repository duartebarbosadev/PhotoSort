import pickle
import random
import sys
import tempfile
import time
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(PROJECT_ROOT / "src"))

from core.similarity_cache import (  # noqa: E402
    SimilarityArtifact,
    load_similarity_artifact_cache,
    save_similarity_artifact_cache,
)


def main() -> None:
    random_source = random.Random(42)
    artifacts: dict[str, SimilarityArtifact] = {
        f"photo-{index:05d}.jpg": {
            "fingerprint": (20_000_000 + index, 1_700_000_000_000_000_000 + index),
            "embedding": [random_source.uniform(-1.0, 1.0) for _ in range(384)],
            "regional_embeddings": [
                [random_source.uniform(-1.0, 1.0) for _ in range(384)] for _ in range(6)
            ],
            "orientation": "landscape",
        }
        for index in range(1_000)
    }
    raw = pickle.dumps(artifacts, protocol=pickle.HIGHEST_PROTOCOL)

    with tempfile.TemporaryDirectory() as directory:
        cache_path = Path(directory) / "similarity-artifacts.pkl.zst"
        save_started = time.perf_counter()
        save_similarity_artifact_cache(cache_path, artifacts)
        save_seconds = time.perf_counter() - save_started

        load_started = time.perf_counter()
        restored = load_similarity_artifact_cache(cache_path)
        load_seconds = time.perf_counter() - load_started

        assert restored == artifacts
        compressed_size = cache_path.stat().st_size
        print(f"artifacts: {len(artifacts)}")
        print(f"pickle bytes: {len(raw)}")
        print(f"zstd bytes: {compressed_size}")
        print(f"size reduction: {(1 - compressed_size / len(raw)) * 100:.1f}%")
        print(f"save seconds: {save_seconds:.4f}")
        print(f"load seconds: {load_seconds:.4f}")


if __name__ == "__main__":
    main()

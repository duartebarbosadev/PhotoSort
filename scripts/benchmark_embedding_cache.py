import pickle
import random
import sys
import tempfile
import time
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(PROJECT_ROOT / "src"))

from core.embedding_cache import (  # noqa: E402
    load_embedding_cache,
    save_embedding_cache,
)


def main() -> None:
    random_source = random.Random(42)
    embeddings = {
        f"photo-{index:05d}.jpg": [random_source.uniform(-1.0, 1.0) for _ in range(384)]
        for index in range(1_000)
    }
    raw = pickle.dumps(embeddings, protocol=pickle.HIGHEST_PROTOCOL)

    with tempfile.TemporaryDirectory() as directory:
        cache_path = Path(directory) / "embeddings.pkl.zst"
        save_started = time.perf_counter()
        save_embedding_cache(cache_path, embeddings, kind="global")
        save_seconds = time.perf_counter() - save_started

        load_started = time.perf_counter()
        restored = load_embedding_cache(cache_path, kind="global")
        load_seconds = time.perf_counter() - load_started

        assert restored == embeddings
        compressed_size = cache_path.stat().st_size
        print(f"entries: {len(embeddings)}")
        print(f"pickle bytes: {len(raw)}")
        print(f"zstd bytes: {compressed_size}")
        print(f"size reduction: {(1 - compressed_size / len(raw)) * 100:.1f}%")
        print(f"save seconds: {save_seconds:.4f}")
        print(f"load seconds: {load_seconds:.4f}")


if __name__ == "__main__":
    main()

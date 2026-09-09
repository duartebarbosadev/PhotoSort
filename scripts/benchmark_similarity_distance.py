"""Deterministic benchmark for the regional similarity distance implementation."""

from __future__ import annotations

import argparse
import time

import numpy as np

from core.similarity_utils import (
    build_regional_distance_matrix,
    regional_embedding_distance,
)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--images", type=int, default=400)
    parser.add_argument("--regions", type=int, default=6)
    parser.add_argument("--dimensions", type=int, default=384)
    args = parser.parse_args()

    rng = np.random.default_rng(7)
    arrays = rng.normal(size=(args.images, args.regions, args.dimensions)).astype(
        np.float32
    )
    paths = [str(index) for index in range(args.images)]
    embeddings = {path: arrays[index, 0].tolist() for index, path in enumerate(paths)}
    regional = {path: arrays[index].tolist() for index, path in enumerate(paths)}

    reference = np.zeros((args.images, args.images), dtype=np.float32)
    started = time.perf_counter()
    for first in range(args.images):
        for second in range(first + 1, args.images):
            distance = regional_embedding_distance(arrays[first], arrays[second])
            reference[first, second] = distance
            reference[second, first] = distance
    reference_seconds = time.perf_counter() - started

    started = time.perf_counter()
    optimized = build_regional_distance_matrix(embeddings, regional, paths)
    optimized_seconds = time.perf_counter() - started
    speedup = reference_seconds / max(optimized_seconds, 1e-9)
    max_error = float(np.max(np.abs(reference - optimized)))

    print(
        f"images={args.images} reference={reference_seconds:.4f}s "
        f"optimized={optimized_seconds:.4f}s speedup={speedup:.1f}x "
        f"max_abs_error={max_error:.3g}"
    )
    if not np.allclose(reference, optimized, atol=5e-7):
        return 1
    return 0 if speedup >= 20.0 else 2


if __name__ == "__main__":
    raise SystemExit(main())

"""Compare shared inference adapters with a baseline checkout and cached weights.

Run with PYTHONPATH=src and the same PHOTOSORT_CACHE_ROOT used to install models.
This script never downloads models. Both adapters share the same loaded weights;
timings exclude model loading, file decoding, and cache hits.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
from pathlib import Path
import platform
import statistics
import sys
import time

import numpy as np
from PIL import Image, ImageEnhance
import torch
from transformers import AutoImageProcessor, AutoModel, AutoModelForImageClassification

from core.best_photo_finder.config import SelectorConfig
from core.best_photo_finder.devices import ResolvedDevice
from core.best_photo_finder.scorers import HuggingFaceAestheticScorer
from core.model_provisioning import AESTHETIC_MODEL, EMBEDDING_MODEL, resolve_snapshot
from core.similarity_embedding_model import SimilarityEmbeddingModel


def load_baseline(root: Path, relative_path: str, name: str):
    spec = importlib.util.spec_from_file_location(name, root / relative_path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


def flatten(value):
    if isinstance(value, dict):
        return np.asarray(list(value.values()))
    if isinstance(value, (tuple, list)):
        return np.concatenate([flatten(item) for item in value])
    return np.asarray(value).ravel()


def compare(before, after, *, device: str, repeats: int):
    def synchronize():
        if device == "mps":
            torch.mps.synchronize()
        elif device == "cuda":
            torch.cuda.synchronize()

    # Warm both paths, then alternate order to reduce thermal/order bias.
    for _ in range(3):
        before()
        after()
    synchronize()
    reference, optimized = flatten(before()), flatten(after())
    assert np.isfinite(reference).all() and np.isfinite(optimized).all()
    np.testing.assert_allclose(optimized, reference, rtol=0, atol=1e-6)
    timings = {"before": [], "after": []}
    for index in range(repeats):
        cases = [("before", before), ("after", after)]
        if index % 2:
            cases.reverse()
        for name, function in cases:
            synchronize()
            started = time.perf_counter()
            function()
            synchronize()
            timings[name].append((time.perf_counter() - started) * 1000)
    medians = {key: statistics.median(values) for key, values in timings.items()}
    return {
        "before_ms": medians["before"],
        "after_ms": medians["after"],
        "speedup": medians["before"] / medians["after"],
        "max_absolute_error": float(np.max(np.abs(reference - optimized))),
        "samples_ms": timings,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline-root", type=Path, required=True)
    parser.add_argument("--device", choices=("cpu", "mps", "cuda"), default="cpu")
    parser.add_argument("--batch-size", type=int, default=8)
    parser.add_argument("--repeats", type=int, default=7)
    parser.add_argument("--threads", type=int, default=torch.get_num_threads())
    parser.add_argument("--image", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if min(args.batch_size, args.repeats, args.threads) < 1:
        parser.error("batch-size, repeats, and threads must be positive")
    torch.set_num_threads(args.threads)
    old_scorers = load_baseline(
        args.baseline_root, "src/core/best_photo_finder/scorers.py", "baseline_scorers"
    )
    old_embeddings = load_baseline(
        args.baseline_root,
        "src/core/similarity_embedding_model.py",
        "baseline_embeddings",
    )
    if args.image:
        with Image.open(args.image) as source:
            source = source.convert("RGB").resize((640, 480))
    else:
        source = Image.fromarray(
            np.random.default_rng(7).integers(0, 256, (480, 640, 3), dtype=np.uint8)
        )
    images = [
        ImageEnhance.Brightness(source).enhance(0.6 + index / args.batch_size)
        for index in range(args.batch_size)
    ]
    device = args.device
    dtype_name = "float32" if device == "cpu" else "float16"
    aesthetic_model = (
        AutoModelForImageClassification.from_pretrained(
            resolve_snapshot(AESTHETIC_MODEL),
            local_files_only=True,
            dtype=getattr(torch, dtype_name),
        )
        .to(device)
        .eval()
    )
    scorers = [old_scorers.HuggingFaceAestheticScorer(), HuggingFaceAestheticScorer()]
    for scorer in scorers:
        scorer._model = aesthetic_model
        scorer._resolved_device = ResolvedDevice(device, device, device, dtype_name)
        scorer._aesthetic_label_index = scorer._resolve_aesthetic_label_index(
            aesthetic_model
        )
    inputs = {Path(f"photo-{index}.jpg"): image for index, image in enumerate(images)}
    config = SelectorConfig(device=device, aesthetic_batch_size=args.batch_size)
    calls = [
        lambda scorer=scorer: scorer.score_batch_from_images(inputs, config)
        for scorer in scorers
    ]
    results = {"aesthetic": compare(*calls, device=device, repeats=args.repeats)}
    old_scores, new_scores = calls[0](), calls[1]()
    assert sorted(old_scores, key=old_scores.get) == sorted(
        new_scores, key=new_scores.get
    ), "Aesthetic ranking changed"
    results["aesthetic"]["ranking_unchanged"] = True
    print("Aesthetic:", results["aesthetic"], flush=True)

    snapshot = resolve_snapshot(EMBEDDING_MODEL)
    model = AutoModel.from_pretrained(snapshot, local_files_only=True).to(device).eval()
    processor = AutoImageProcessor.from_pretrained(snapshot, local_files_only=True)
    encoders = [old_embeddings.SimilarityEmbeddingModel(), SimilarityEmbeddingModel()]
    for encoder in encoders:
        encoder.model, encoder.processor, encoder.device = model, processor, device
    for method in ("encode", "encode_with_patches"):
        calls = [
            lambda encoder=encoder, method=method: getattr(encoder, method)(images)
            for encoder in encoders
        ]
        results[method] = compare(*calls, device=device, repeats=args.repeats)
        print(method, results[method], flush=True)
    report = {
        "platform": platform.platform(),
        "torch": torch.__version__,
        "device": device,
        "aesthetic_dtype": dtype_name,
        "batch_size": args.batch_size,
        "threads": args.threads,
        "repeats": args.repeats,
        "model_revisions": {
            model.repo_id: model.revision
            for model in (AESTHETIC_MODEL, EMBEDDING_MODEL)
        },
        "results": results,
    }
    args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

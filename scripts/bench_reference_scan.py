"""Measure the actual reference FileScanner without changing its project.

Run with the Python reference virtual environment and PYTHONDONTWRITEBYTECODE=1:
  python -B scripts/bench_reference_scan.py /path/to/PhotoSort /path/to/fixture 7
Imports are excluded from scan timing. No GUI, decoder, cache or model is created.
"""
import json
import logging
import sys
import time
from pathlib import Path

reference, fixture = map(Path, sys.argv[1:3])
repeats = int(sys.argv[3]) if len(sys.argv) > 3 else 7
if not 1 <= repeats <= 100:
    raise SystemExit("use 1..100 repetitions")
sys.dont_write_bytecode = True
sys.path.insert(0, str(reference / "src"))
from core.file_scanner import FileScanner

logging.disable(logging.CRITICAL)
runs = []
for _ in range(repeats):
    # FileScanner uses the pipeline only as injected ownership, not for discovery.
    scanner = FileScanner(None)
    found, errors, batches = [], [], []
    started = time.perf_counter()

    def batch(items):
        batches.append((len(items), (time.perf_counter() - started) * 1000))

    scanner.files_found.connect(batch)
    scanner.error.connect(errors.append)
    scanner.thumbnail_preload_finished.connect(found.extend)
    scanner.scan_directory(str(fixture))
    elapsed = (time.perf_counter() - started) * 1000
    if errors or sum(size for size, _ in batches) != len(found):
        raise RuntimeError(errors or "batch count disagrees with final results")
    runs.append({"count": len(found), "first_batch_ms": batches[0][1] if batches else None,
                 "scan_ms": elapsed, "largest_batch": max((n for n, _ in batches), default=0)})
print(json.dumps({"implementation": "python", "runs": runs}))

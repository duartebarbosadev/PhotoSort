# PhotoSort performance guide

PhotoSort treats a folder as an indexed media library, not as a batch job that must finish every possible computation before the UI becomes usable.

## Runtime invariants

- A folder is walked once. Scanner batches carry path, type, byte size, and nanosecond modification time.
- Scan batches update `AppState` path lookups and media totals incrementally. Per-file lookup and status-bar summaries must remain O(1) after insertion.
- The file list becomes usable when discovery finishes. Ratings and thumbnails remain background enhancements.
- Full previews are generated on demand. Opening a folder never generates a preview for every image.
- Thumbnail work is limited to the current viewport plus a small margin, in bounded batches.
- Image decoding uses at most four concurrent workers; memory-heavy RAW/HEIC decodes are serialized.
- Organize, Cull, similarity grouping, and Pick Best reuse the same `ImagePipeline` instance.
- Disk cache keys include a schema version and source fingerprint. Cache payloads are compressed; hot decoded images use a bounded 256 MB LRU.
- ML libraries, non-default workflows, and the metadata sidebar load only on first use.

## Release budgets

The automated suite enforces structural budgets that are stable in CI. Timing and memory budgets are measured with the smoke runner because shared CI timing is noisy.

| Measurement | Target |
|---|---:|
| Main-window module import, warm local run | <= 0.75 s |
| Empty window construction, warm local run | <= 1.5 s |
| Main-thread decode concurrency | <= 4 |
| Display preview longest edge | <= 2560 px |
| Thumbnail request batch | <= 32 items |
| Automatic full preview generation on folder open | 0 images |
| 80-file sample peak RSS after initial load | <= 750 MB |

The pre-optimization 80-file sample used roughly 2.1 GiB RSS because every preview was decoded and cached at folder open. The main-window import also loaded scikit-learn eagerly. Keep these as regression reference points.

On the same development machine after this work, a cold-cache run of the included 80-file sample became usable in 0.97 seconds and peaked at 495 MB after eight seconds; a warm-cache run peaked at 251 MB. The empty-window run imported the main module in 0.30 seconds, constructed the window in 0.26 seconds, and peaked at 143 MB. These figures are reference measurements, not machine-independent test assertions.

## Measuring

Run an empty-window measurement:

```bash
QT_QPA_PLATFORM=offscreen .venv/bin/python scripts/performance_smoke.py
```

Run the included 80-file sample long enough to complete discovery and initial visible thumbnails:

```bash
QT_QPA_PLATFORM=offscreen .venv/bin/python scripts/performance_smoke.py \
  --folder sample_images_2 --duration 8 --cold-cache
```

The command prints JSON suitable for comparing branches. Record `import_seconds`, `window_construct_seconds`, `folder_usable_seconds`, `media_count`, and `max_rss_mb`. A release should be investigated when it exceeds a budget or regresses more than 20% against the same machine and dataset.

## Verification

```bash
QT_QPA_PLATFORM=offscreen .venv/bin/pytest -q
.venv/bin/ruff check .
```

Performance-related tests cover startup import boundaries, scanner batching and indexed state, the absence of global preview preloading, cache invalidation/compression, single-flight thumbnail generation, viewport request deduplication, and bounded constants.

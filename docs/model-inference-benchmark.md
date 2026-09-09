# Aesthetic scoring benchmark

Measured on 2026-09-09 against dependency-update commit
`c7ef86e942b8d9f4410611fb21ba89a0bd704b1c`.

## Method

- Apple M4 Max, macOS 26.5.2, Python 3.14.5, PyTorch 2.14.0.
- Default 10 PyTorch CPU threads, eight images per batch (PhotoSort's default).
- Actual pinned `cafeai/cafe_aesthetic` weights at
  `48a343764f786abc1ca8aaddfcd60a688a70da9b`.
- Eight deterministic brightness variations of MediaPipe's public
  [test portrait](https://storage.googleapis.com/mediapipe-assets/portrait.jpg),
  resized to 640 × 480 before timing.
- Both adapters share identical loaded weights and inputs. Three warmup pairs,
  followed by 15 measurement pairs with alternating execution order. GPU timing
  synchronizes before and after each call.
- Timings include preprocessing, inference, and score extraction. They exclude
  model loading, photo decoding, cache hits, face analysis, and UI work.

## Results

| Backend | Baseline median | Updated median | Reduction | Maximum score difference |
| --- | ---: | ---: | ---: | ---: |
| Apple GPU, float16 | 151.07 ms | 144.19 ms | 4.6% | 0 |
| CPU, float32 | 500.53 ms | 491.97 ms | 1.7% | 2.98e-7 |

Rank order was unchanged on both backends. A separate 21-pair GPU run with four
CPU threads measured 149.34 → 142.14 ms (4.8% lower), also with identical scores.
The CPU difference is small; the repeatable GPU reduction is the main benefit.
These are measurements of the scoring stage on this machine, not a claim that
an entire workflow or every device is 5% faster. NVIDIA GPU performance was not
measured.

The retained changes batch normalization and tensor conversion, reuse one image
processor, transfer scores to the CPU once per batch, and use inference mode
inside aesthetic scoring. Pillow's original bicubic resize is preserved.
Switching resize to Torchvision changed normalized pixels by up to 0.0157 in an
exploratory test, so it was not retained. Inference mode did not materially
improve DINOv2; its implementation remains unchanged.

## Reproduce

Use the updated requirements and a separate baseline checkout at the commit
above. Install both pinned models through PhotoSort first. The benchmark only
reads locally cached models and never downloads them.

```sh
PYTHONPATH=src python scripts/benchmark_model_inference.py \
  --baseline-root /path/to/baseline \
  --device mps --batch-size 8 --repeats 15 \
  --image /path/to/portrait.jpg --output /tmp/inference-mps.json
```

Repeat with `--device cpu`. The JSON includes individual timing samples, model
revisions, maximum output differences, and ranking parity. DINOv2 timings are
an unchanged control. Omitting `--image` uses seeded synthetic pixels instead;
those results are not directly comparable to the portrait measurements above.

## Regression checks

Tests compare the prior normalization/scoring math with batches of 1, 3, and 8,
including an incomplete final batch. They verify pixel/score tolerances, ordering,
one transfer per batch, model and processor reuse, and inference mode inside a
background thread. A real Qt-worker smoke run kept the main event loop active
while scoring and honored cancellation before the next cluster, without
publishing results. Cancellation still takes effect at the existing cluster
boundary; it does not interrupt an in-flight model call.

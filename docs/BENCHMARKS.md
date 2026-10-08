Latest real-photo comparison: [preview and thumbnail preparation, 9 October](diagnostics/review-comparison-2026-10-09/README.md).
On the authorized samples, median initial Rust preparation was 2.35–2.76× faster
on mixed folders and 27.26× faster on 100 RAW photos. Rendering recipes and proxy
sizes differ; this does not establish whole-application speed. The report includes
cache reuse, RSS ranges and the larger Rust disk-cache footprint.

# File discovery comparison — 2026-09-28

The Rust catalog scanner completed this synthetic 100,000-path workload in
**1.39 seconds median**, versus **5.15 seconds** for the actual
Python FileScanner: **3.70× faster for this scan workload**.
This is not an overall application speed, UI startup, decoder or AI benchmark.

| Measurement | Rust release | Python reference |
| --- | ---: | ---: |
| Warm scan median (18 measurements) | 1390.4 ms | 5151.2 ms |
| Warm scan range | 1112.1–1850.3 ms | 4187.1–6805.0 ms |
| First batch median | 1.71 ms | 173.22 ms |
| Native discovery batch size | 128 | 64 |
| Median peak scanner-process RSS | 48.8 MiB | 1166.0 MiB |

## Method and limits

* macOS 26.5.2 arm64; Rust 1.94.1 release build; the reference project's Python
  3.14.5 virtual environment and clean commit
  `7c62143f8ba80247b10dac8ebf38a37beacdd170`.
* Both implementations scanned exactly the same temporary directory: 100,000
  distinct PNG paths hardlinked to **one disposable PNG copy**, with an inode
  distinct from the original sample. This isolates path discovery; it does not
  represent reading 100,000 distinct image contents or a diverse nested library.
* Three fresh processes per implementation, alternating order Rust/Python,
  Python/Rust, Rust/Python. Seven scans each. Each process's first scan is excluded
  from warm timing summaries, leaving 18 measured scans per implementation.
  The OS cache was not flushed: the excluded scans are not claimed to be cold.
* Python calls the existing FileScanner and receives its actual Qt signals. It
  injects no image pipeline because the scanner does not use it during discovery.
  Rust calls the existing Catalog::scan and counts its actual batches. Neither
  harness decodes images, extracts embedded metadata or starts a model/UI.
  Import time is excluded from scan timings. Python bytecode writes are disabled.
* Each implementation retains its native behavior. Rust builds inventory and
  sorts catalog paths; Python builds dictionaries with additional bookmark and
  birth-time fields. Their batch sizes differ, so first-batch timing describes
  each implementation's behavior rather than an equal-sized batch comparison.
* RSS comes from macOS `/usr/bin/time -l` for each complete scanner process,
  including dependency imports and all seven repetitions. It is **not** UI memory,
  steady-state heap, GPU memory or the footprint of a fully loaded photo library.
  The Python scanner imports the real reference dependency chain.
* No GUI application or build ran during the timed comparison. The machine was
  not otherwise isolated; the ranges show run-to-run variation. Use the raw data
  and repeat on representative independent files before treating this as a target.

## Reproduce

From the Rust project, with a caller-owned read-only fixture directory:

```sh
cargo build --locked --release --example bench_folder
/usr/bin/time -l target/release/examples/bench_folder /path/to/fixture 7
PYTHONDONTWRITEBYTECODE=1 /usr/bin/time -l ../PhotoSort/.venv/bin/python -B scripts/bench_reference_scan.py ../PhotoSort /path/to/fixture 7
```

Repeat the two commands in alternating order for three process pairs. Both
harnesses emit JSON containing every scan's count, duration and first batch.
The scripts perform no file mutation. Do not infer file-operation, image-quality
or AI-result parity from their timing.

[Raw results and source hashes](benchmarks/scan-2026-09-28.json) preserve all 42
scans and six process RSS measurements. The original seven sample hashes and
clean Python working tree were verified after the run; all 100,000 fixture paths
retained the same unchanged disposable inode and no sidecars were created.


## Regional scene inference — 2026-10-01

The shared Rust DINO model now processes one photo's five regional crops in a
single bounded call. On the seven original sample photos, the optimized crop
preparation/inference stage took **0.650 seconds median**, versus **1.123 seconds**
for five individual calls: **1.73× faster for this stage**. All regional vectors
were exactly equal in all three runs. This compares two Rust execution strategies;
it does not measure overall application speed or speed relative to Python.

| Seven-photo process | Individual crops | Batched crops |
| --- | ---: | ---: |
| 1 | 1.092 s | 0.650 s |
| 2 | 1.123 s | 0.613 s |
| 3 | 1.281 s | 0.681 s |

Three fresh release-test processes used the shared neutral analysis pipeline and
one model session. Timings include crop preparation and inference, but exclude
photo decoding, initial whole-image embedding/model setup, grouping and UI.
Individual calls ran first for each photo: these are indicative measurements,
not a counterbalanced or cold-cache benchmark. No other validation build/test from
this task overlapped the final runs; the host was otherwise not isolated.

A separate native CPU probe compared widths 1, 2 and 5 on five pre-normalized
synthetic tensors. Six fresh processes ran widths 1, 2, 5, 5, 2, 1, with one
warm-up pass excluded and four timed passes each. The test harness was unoptimized
Rust; inference used the native runtime with 11 intra-op threads, sequential
execution and spinning disabled. The timings include packing and output copying.
Every output token was identical to individual inference.

| Batch width | Five-input median | Maximum process RSS across two runs |
| --- | ---: | ---: |
| 1 | 147.62 ms | 194.1 MiB |
| 2 | 129.05 ms | 233.4 MiB |
| 5 | 65.72 ms | 287.4 MiB |

Width 5 was 2.25× faster in that probe and used **93.3 MiB more peak process RSS**
than width 1. This is a measured speed/memory tradeoff, not a memory reduction.
The production batch is capped at one photo's five crops; it does not grow with
library size. The seven-photo test process exercised both methods and reached
829.3–838.6 MiB RSS including decoding/cache/model work, so its RSS cannot isolate
the methods' memory costs. These are CPU process measurements, not UI/GPU memory.

The seven-photo correctness/performance check can be reproduced without retaining
project build output (requires the pinned native models/runtime and sample files):

```sh
(
  export CARGO_TARGET_DIR="$(mktemp -d "${TMPDIR:-/tmp}/photosort-benchmark.XXXXXX")"
  trap 'cargo clean --target-dir "$CARGO_TARGET_DIR"' EXIT
  export PHOTOSORT_SAMPLE_DIR="$(cd ../PhotoSort/tests/samples && pwd)"
  cargo test --locked --release --lib core::models::regional_batch_tests::seven_samples_match_individual_crops_with_one_batched_forward -- --exact --ignored --nocapture
)
```

For independent process RSS, compile once with `--no-run` and run the resulting
`release/deps/photosort-*` test executable under `/usr/bin/time -l`; exclude
compilation from timing. The `core::models::batch_probe::scene_batch_timing_and_equivalence`
ignored test selects widths using `PHOTOSORT_BATCH_WIDTH=1`, `2` or `5`.
[Raw runs, method details and source/model/sample hashes](benchmarks/scene-batching-2026-10-01.json)
retain the evidence. All build targets were cleaned afterward.


## Shared evidence cache — 2026-10-01

A controlled 512-record synthetic grouping workload took **0.211 s median**
with the production binary Store versus **0.352 s** with the previous JSON
load/parse/validate path: **1.67× faster for this workload**. Cached regional
artifacts occupied **4,746,240 bytes** versus **14,331,145 bytes**,
a **66.9% reduction**. Both paths use the same production clustering code,
produce the same 128 four-record groups, and retain at most 128 descriptors.

| Path | Median grouping time | Range | Maximum whole-process RSS |
| --- | ---: | ---: | ---: |
| JSON | 0.352 s | 0.345–0.359 s | 11.0 MiB |
| Binary | 0.211 s | 0.207–0.218 s | 10.7 MiB |

Six fresh release processes ran JSON/binary, binary/JSON, JSON/binary. Each ran
four grouping passes; the first was excluded, leaving nine timed passes per
format. Every record contains six 384-float vectors. Seeded pseudorandom vectors
with small per-photo perturbations make 512 distinct descriptors, avoiding the
identical-record collapse optimization. This is a storage/computation fixture,
not evidence of real photographic grouping accuracy or a diverse real library.

Timing includes repeated reads, format/semantic validation, normalization and the
unchanged pair comparisons. Fixture generation, cache writes, warmup, photo
loading, model work and UI are excluded. Binary reads include checksums and use
the actual shared Store. JSON reads mirror the previous pinned-cache path.
Artifact bytes count one encoded copy per record; active pinning and optional
persistent copies can occupy additional space. RSS is from /usr/bin/time -l for
each complete process, including setup/warmup; it is not UI/model memory or an
isolated heap measurement. No build or test from this task overlapped timed runs;
the host was otherwise not isolated and OS caches were not flushed.

Reproduce with `cargo build --locked --release --no-default-features --example
bench_evidence`, then run the resulting executable with `json 512` or `binary 512`
in the order above under `/usr/bin/time -l`. Use a temporary CARGO_TARGET_DIR and
clean it afterward as shown in the regional benchmark instructions. The harness
creates and automatically removes its own disposable cache fixtures.
[Raw process measurements, counters and source hashes](benchmarks/evidence-cache-2026-10-01.json)
retain every timed pass. Worst-case clustering remains quadratic; this result
neither establishes whole-application speed nor compares against Python.


## Metadata service baseline before persistence — 2026-10-02

Before persistence, the real metadata services showed a mixed result. On a synthetic nested library of
2,100 paths, Rust is **1.36× faster for initial reads** and **13.37× faster for
same-owner repeated reads**, but **21.73× slower after recreating the metadata
owner**. Rust rereads all source files after reopening; Python reuses its disk
EXIF cache. Persistent metadata-only caching is a concrete remaining Rust gap.

| 2,100-path metadata phase | Rust median | Python median |
| --- | ---: | ---: |
| Initial, empty application cache | 2,722.16 ms | 3,705.66 ms |
| Repeated, same owner | 9.89 ms | 132.13 ms |
| New owner, same cache directory | 2,804.85 ms | 129.10 ms |
| Repeated after reopening | 10.22 ms | 131.11 ms |
| Median whole-process peak RSS | 23.63 MiB | 332.41 MiB |

All three reopened Rust runs perform 2,100 metadata source reads; all reopened
Python runs perform zero. Both perform zero source reads on warm and rewarmed
passes. Initial timing ranges are 2,695.83–2,770.96 ms Rust and
3,564.51–3,778.88 ms Python. Reopened ranges are 2,675.63–2,893.43 ms Rust and
127.28–129.43 ms Python.

On the seven independent samples, initial medians are 10.17 ms Rust / 15.66 ms
Python; repeated medians are 0.032 / 0.516 ms; reopened medians are 9.17 / 0.639 ms.
These small fixtures are not large-library startup or overall app speed results.

### Method and correctness boundaries

- Apple M4 Max, 36 GiB memory, macOS 26.5.2 arm64. Rust 1.94.1 release/native Exiv2
  0.28.9 versus the unchanged Python 3.14.5 project with pyexiv2 2.16.0/Exiv2 0.28.8.
- Seven independent disposable copies of the original JPEG, PNG and ARW samples.
  The nested fixture has 300 directories, each hardlinked to those seven copies:
  2,100 paths but only seven independent contents and about 116 MB of seed bytes.
  Originals are never hardlinked or modified. This measures repeated-content
  metadata-service scaling, not a diverse 2,100-photo collection.
- Three fresh process pairs per fixture in Rust/Python, Python/Rust, Rust/Python
  order. Each process performs initial, warm, reopened-owner and rewarmed passes.
  Reopened creates a new Pipeline or ExifCache over the same temporary cache
  directory; it does not restart the complete process or GUI. Imports, discovery,
  cache construction, hashing and teardown are excluded from phase timings.
- Rust calls the shared `Pipeline::metadata`, including its normal identity and
  sidecar checks. Python calls `MetadataProcessor.get_detailed_metadata` with a
  temporary real ExifCache. Only a transparent wrapper counts actual Python
  source-reader calls. Both retain their native caching behavior. Rust returns a
  typed subset with normalized fields; Python returns comprehensive dictionaries
  and writes its disk cache. The Python configured EXIF limit was 2 GiB, with no
  cache-capacity pressure. This is not a parser-only or language-only comparison.
- All phases verify common path/dimension/orientation/camera/lens values. The
  initial comparison intentionally stopped when ARW dimensions differed: Python
  reports stored 6048×4024 bounds, while Rust uses declared 6000×4000 photo/crop
  dimensions. Validation now uses the declared photo dimensions on both sides,
  outside the timer; raw Python dimension differences remain in the JSON. Neither
  application was changed to force equality. This does not prove every metadata
  field is identical. See `metadata-dimension-audit-2026-10-02.json` for the audit.
- `/usr/bin/time -l` supplies whole-process peak RSS, including imports, discovery,
  retained results and caches. It is not GUI, GPU, steady-state heap, or full-app
  library memory. No task build or test overlapped timed runs; other host activity
  was not isolated and the OS cache was not flushed. “Initial” is not disk-cold.

### Reproduce the service passes

Build `cargo build --locked --release --example bench_metadata` in a temporary
CARGO_TARGET_DIR, then copy out the example executable and clean that target.
Use a read-only JPEG/PNG/ARW fixture and alternate these commands three times:

```sh
/usr/bin/time -l /path/to/bench_metadata /path/to/fixture
PYTHONDONTWRITEBYTECODE=1 /usr/bin/time -l ../PhotoSort/.venv/bin/python -B scripts/bench_reference_metadata.py ../PhotoSort /path/to/fixture
```

Each executable owns and removes its temporary cache. The source fixture must
remain unchanged during the comparison. The harnesses create no GUI, models or
image pixels. Twelve completed processes and all 48 phase records, source hashes,
platform details, dimension differences, ranges and RSS are preserved in
[the raw benchmark](benchmarks/metadata-2026-10-02.json). All original/seed hashes
and all 2,100 hardlink identities were verified; the synthetic fixture was removed.

**Follow-up (implemented below):** persist typed metadata through the existing shared cache,
with source/sidecar/recipe invalidation, corruption recovery, cancellation, cache
limits and clear/disabled-cache behavior. Re-run the same benchmark after that
change rather than inferring improvement from the language or a hot memory hit.


## Persistent metadata and cache-write cost — 2026-10-02

The shared Rust metadata cache now survives owner/runtime recreation. On the same
synthetic 2,100-path fixture, median reopening falls from **2,804.85 ms to 55.00 ms**
(51.0× faster than the prior Rust run). The repeated Python comparison measures
130.14 ms, making Rust 2.37× faster for this phase. Every reopened run performs
zero source reads; Rust records 2,100 metadata disk hits. This is an owner/service
benchmark, not complete application startup.

| 2,100-path phase | Rust before persistence | Rust with persistence | Python rerun |
| --- | ---: | ---: | ---: |
| Initial, empty application cache | 2,722.16 ms | 3,022.78 ms | 3,586.47 ms |
| Repeated, same owner | 9.89 ms | 9.71 ms | 131.20 ms |
| New owner, same cache directory | 2,804.85 ms | 55.00 ms | 130.14 ms |
| Repeated after reopening | 10.22 ms | 9.11 ms | 136.96 ms |

Persistence adds about 300.62 ms (11%) to the previous initial Rust measurement,
while the final initial pass is still 1.19× faster than Python in the repeated
comparison. Final Rust initial/reopen ranges are 2,996.60–3,024.81 ms and
54.41–56.07 ms. Seven-sample initial/reopen medians are 11.07/0.203 ms Rust versus
15.87/0.456 ms Python. Whole benchmark-process median peak RSS is 22.63 MiB Rust
and 311.02 MiB Python, including validation and excluding the GUI/GPU. These
figures must not be used as full-app memory or startup claims.

A completed first attempt reused the synced writer used for user data. It reduced
reopening to 75.63 ms but increased initial Rust ingestion to **13,941.40 ms**.
Regenerable binary artifacts now use one shared exclusive-temp/atomic-replace
writer without per-record physical flushing. Interrupted or lost cache records
recompute; photo edits, decisions, settings and recovery journals keep their
synced write path. The first attempt is retained in
[metadata-persistent-synced-2026-10-02.json](benchmarks/metadata-persistent-synced-2026-10-02.json).
The old non-persistent baseline remains unchanged.

The final comparison repeats all 12 processes/48 phases with the same seven
original sample contents and 300-directory disposable hardlink fixture, three
alternating Rust/Python pairs per fixture, no overlapping task builds/tests, and
unflushed OS caches. All common-field checksums agree with Python under the
previously documented declared-photo-dimension normalization. The Rust harness
now additionally hashes every typed metadata field outside phase timing and
verifies equality across all four phases. This adds validation work to whole-
process RSS; it does not change timed service calls. Original and seed hashes,
all 2,100 links and the untouched Python checkout were verified before removing
the synthetic fixture. Raw final evidence is in
[metadata-persistent-2026-10-02.json](benchmarks/metadata-persistent-2026-10-02.json).
Seven repeated contents remain a scaling fixture, not a diverse-photo workload.


## Exact regional early rejection — 2026-10-04

The shared grouping service is **1.90× faster at 512 records** and **2.18× faster
at 2,048 records** on a controlled synthetic cached-evidence workload. A pair
comparison now stops once even maximally similar remaining regions cannot
meet the current threshold. This is an exact rejection bound, not approximate
neighbor selection. Every timed pass produced identical stable label hashes.

| Records | Full regional calculation, median | Early rejection, median | Full / early regional dots | Evidence loads, both |
| --- | ---: | ---: | ---: | ---: |
| 512 | 227.20 ms | 119.86 ms | 782,592 / 132,352 | 2,816 |
| 2,048 | 3,279.92 ms | 1,501.96 ms | 12,567,552 / 2,102,272 | 35,840 |

At 512 records the full/early timing ranges were 224.81–231.14 /
117.04–172.19 ms; at 2,048 they were 3,236.86–3,493.42 /
1,467.35–1,553.92 ms. Peak whole-test-process RSS was 13.92 / 13.95 MiB
and 14.61 / 14.78 MiB respectively. Both paths retain at most 128 descriptors.
Each resident descriptor adds one f32 bound plus struct padding; no larger
image cache or per-photo search index was introduced.

Method: Apple M4 Max, 36 GiB, macOS 26.5.2 arm64, Rust 1.94.1 default-feature
release test executable (opt-level 3, thin LTO, one codegen unit). For each
size, six fresh processes ran full/early, early/full, full/early. Each process
ran four passes, excluded its first, and retained nine timed passes per path.
The same production binary Store, normalization, deduplication, block loader,
union state and stable label publication were used; only the test-selected
neighbor predicate differed. The full predicate calls the unchanged public
`Features::distance`. Timing includes evidence reads, validation, normalization
and complete grouping; fixture creation/cache writes, warmup, image/model work
and UI are excluded. RSS includes the whole test process and fixture setup.
No app or other build/test from this task overlapped the measurements; the host
was otherwise not isolated and OS caches were not flushed.

Each photo has six 384-float rows. Seeded random scene vectors with small
per-photo perturbations create distinct records in four-photo groups. The
scenes are well separated, so they are favorable to early rejection; this is
not a diverse photographic dataset, an accuracy result, a whole-app speedup
or a Python comparison. Worst-case pair enumeration and repeated block
loading remain quadratic. A separate read-only seven-sample native-model
regression preserves groups at 0.02/0.055/0.20 and reduces regional dots from
54 to 9/10/21 respectively, without additional image/model work.

Reproduce by building the ignored
`core::regional::tests::regional_pruning_benchmark` release test, then running
its executable under `/usr/bin/time -l` with `--exact --ignored --nocapture`,
`PHOTOSORT_REGIONAL_MODE=exhaustive` or `pruned`, and
`PHOTOSORT_REGIONAL_COUNT=512` or `2048`. Use a disposable CARGO_TARGET_DIR
and clean it afterward. Compiling the entire optimized unit-test executable
is expensive; exclude compilation from timings.
[Raw measurements and source hashes](benchmarks/regional-pruning-2026-10-04.json)
retain all measured passes, counters, label hashes and process RSS.


## Metadata capacity and real photo folders — 6 October 2026

The shared metadata cache now evicts least-recently-used entries using `lru`
0.18.5 instead of clearing every entry at 100,000. The red/green shared-boundary
regression proves that a recently used photo avoids a disk reread after another
entry crosses the capacity. One before/after 100,000-path release observation
found broadly similar service times and whole-process peak RSS of 350.45 versus
302.19 MiB; this is not evidence of a universal throughput improvement.
[Method, raw measurements and limitations](diagnostics/metadata-lru-2026-10-06/README.md).

The newly authorized sample folders provided 263 photo paths (186 unique file
contents), including 102 ARW and 90 HEIC. Every folder passed read-only metadata,
proxy, technical analysis, original inspection and persistent-cache checks.
Quality and visual grouping completed on all three larger folders, with identical
reopened results and zero new decoding/inference. All originals and sidecars
remained unchanged. This is format/consistency evidence, not labeled accuracy.

This larger corpus exposed remaining performance costs: 135.59 s to cold-render
100 RAW browsing proxies versus 21.15 s for Quality analysis using embedded
previews; image-pipeline maximum RSS reached roughly 2 GiB on mixed HEIC folders,
and Quality-process maximum RSS reached roughly 4.7 GiB including model setup.
The next performance work should measure fast RAW browsing previews, bounded
Apple image decoding and native/model allocation lifetimes. These observed peaks
must not be described as steady-state memory or explained as leaks without
further profiling. [Full corpus results](diagnostics/real-library-2026-10-06/README.md).

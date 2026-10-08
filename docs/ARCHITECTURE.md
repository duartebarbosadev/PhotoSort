# Native architecture

## GUI decision (before implementation)

Use egui/eframe 0.33 with wgpu: native Rust, GPU-backed textures, custom comparison
canvas, direct control over visible-row virtualization and repaint scheduling.
The application requests repaints for input and worker results, not a permanent
60 Hz loop. The UI uses point-based layouts for high-DPI displays and switches
from sidebar navigation to a compact horizontal stepper below 1000 points.

Iced's Elm-style message architecture is attractive, but egui offers a simpler
custom photo canvas and visible-row grid for this workload. Slint is viable but
adds its declarative language and license selection. A browser/webview would add
a second frontend runtime without helping decoding or the review canvas.

Sources: https://github.com/emilk/egui,
https://docs.rs/egui/0.33.3/egui/containers/scroll_area/struct.ScrollArea.html,
https://github.com/iced-rs/iced, https://slint.dev/faqs.
These are design reasons, not measured claims that egui outperforms every toolkit.

The shared theme configures fonts once when the native application is created.
Proportional text keeps egui's normal family order and appends the existing
monospace family as a fallback for missing control symbols. This reuses fonts
already bundled by egui, without platform font discovery or per-workflow font
setup. A renderer-level regression checks navigation, rotation, disclosure,
rating and measurement glyphs in every proportional text style used by the UI.

## Ownership

* `color`: a pure-Rust `moxcms` owner inside the existing image pipeline; no
  additional thread or native dependency. API reference: https://docs.rs/moxcms/0.8.1/moxcms/. Standard-image and embedded RAW JPEG
  decoding read the ICC profile from the same decoder, before pixel allocation.
  ICC-tagged CMYK JPEGs use `jpeg-decoder` without Rayon to retain original ink
  coverage for both Adobe CMYK and YCCK; regular JPEGs keep the existing decoder.
  This focused fallback avoids custom YCCK math and another native dependency.
  The library is in maintenance mode, so its scope stays limited to this missing
  decoder capability (https://github.com/image-rs/jpeg-decoder).
  CMYK transforms have a distinct cache layout identity and chunk into RGB.
  RGB/gray profiles normalize to sRGB at their decoded 8/16-bit or float precision;
  RGB(A) conversion uses a bounded scratch chunk rather than another full image.
  Four recently used transforms are retained by profile bytes and sample layout.
  Parsing limits profile/CLUT bytes and tone-curve entries. The color owner also
  owns the shared 32768-pixel dimension and 512 MiB decoded-buffer policy for
  ordinary images, embedded JPEGs and the CMYK fallback. Cancellation is checked
  between 8,192-pixel chunks, and unsuccessful conversion publishes no pixels.
  The shared source recipe version invalidates old display, analysis and derived
  evidence caches together. Canonical pixels serve thumbnails/detail/analysis;
  corrected disk proxies require no repeated transform after restart. Metadata
  loading alone does not decode or create a color transform.

* `desktop`: one application-owned, lazy, single-inflight OS handoff using
  `opener`. Inspector and library context menus capture the clicked catalog path;
  video handoff shares this service and pauses internal playback. No image decode,
  metadata parse, catalog scan or model initialization occurs. Filesystem checks
  and native calls run off the UI thread. Folder generations reject stale results.
  Requests are disabled during scanning/file work but remain available during AI.
  An OS request already handed off cannot be recalled. Shutdown detaches at most
  one blocked native call, owning no photo handles or application queues, so a
  stalled file manager cannot prevent quit. Windows/Linux native behavior still
  needs platform verification.

* `model`: typed media identity, fingerprints, metadata, decisions and steps.
* `catalog`: one recursive inventory; companions and all workflow views reuse it.
* `pipeline`: one image owner with display/detail and neutral analysis recipes,
  byte-limited LRU, fingerprinted disk proxies, explicit invalidation.
* `analysis`: deterministic technical quality and candidate grouping; no UI.
* `metadata`: EXIF reading and non-destructive XMP interoperability.
* `operations`: reviewable plans, collision protection, companion groups,
  fingerprint checks and per-operation results.
* `session`: application-private persistence, never stored inside photo folders.
* `jobs`: bounded messages, generation checks and cancellation; expensive services
  live on workers. UI only uploads ready pixels for visible cells/previews.
* `browsing`: pure rating/filter/order rules plus one application-owned derived
  view index. The runtime owns its worker, independently of image decoding.
  A catalog seeds cached name/path/time keys and an immutable folder index once.
  Folder choices reuse scan inventory, including hidden and empty directories;
  component-aware direct/recursive scopes compose with every other filter.
  Results share the folder list by Arc rather than cloning it per query. Metadata and individual decision
  changes are incremental; bulk review changes replace only decision state.
  One coalescing mailbox retains the latest query/result and at most one pending
  update per media ID. Chunk sorting and merging check cancellation; folder resets
  cancel index construction. No rescans, image decodes or model initialization.
* `selection`: browser membership and range anchors in the accepted display order.
  Modifier selection, select-all and reconciliation never decode images or read
  metadata. Selection is independent of the bounded comparison presentation.
* `decisions`: one manual staging rule for individual and bulk ratings, rotation
  and Trash marks. It validates catalog membership and media capability, deduplicates
  input IDs and leaves filesystem work to the existing reviewed operation planner.
  The UI publishes changed decisions in one coalesced batch and reconciles guided
  review state once, without cloning unrelated decisions for every selected row.
* `ui`: shared navigation, virtualized library grid/list/icon layouts, review canvas,
  inspector and plan review. Library layouts share thumbnail requests and one
  selection/comparison/action handler. Focus changes reveal the corresponding row;
  redraws do not scan the library or request offscreen thumbnails. Cull selection
  is application-owned across list/icon/grid layouts. Organize's editor retains
  its separate draft selection because it selects proposed files and directory
  nodes for atomic draft moves; it does not stage Cull ratings or Trash marks.
  Guided review choices retain their confirmation/history owners and reuse the
  application decision store when published.

## Resource rules

No folder-wide decoded image collection. No metadata or image IO in frame code.
No per-page services. Bound CPU images, GPU textures and event queues separately.
Preview and thumbnail come from one oriented canonical proxy. Reject oversized
images through decoder allocation limits. Cancel between work units and discard
old-generation results after a folder switch. OS codec calls may finish their
current decode; they cannot be forcibly interrupted safely.

## Verification policy

Test core boundaries: scan reuse, companion plans, collisions, source changes,
XMP preservation, cache reuse/invalidation/eviction and worker cancellation/stale
results. Use generated test images, never mutate reference sample libraries.
Run cargo fmt, clippy, full tests and a release build. Document runtime checks and
remaining codec/model/platform validation separately from compilation.

## Native analysis and resource boundaries

`models` owns a single lazy ONNX Runtime registry for orientation, DINOv2, BeIT,
BlazeFace and Face Mesh. `faces` performs bounded detector crops, suppression,
landmark geometry and eye metrics through that registry. `geography` owns a lazy
spherical k-d tree for offline nearest-place lookup. Neither service is created by
individual workflow pages. Generated thumbnails and previews share the exact same
oriented source pixels.

Descriptors are cached across workflow runs, invalidated on folder/model/file
changes, and capped at 20,000 entries. The group index keeps bounded candidate
search and up to eight anchors per group. File discovery never initializes models.
Model calls and native decodes finish their current unit before cancellation; the
UI immediately stops accepting results for the cancelled run.

Small native adapters cover LibRaw, Exiv2 and macOS ImageIO. They do not own UI,
application state or workers. On macOS, a minimal NSApplication subclass also
turns native terminate requests into a Rust Quit request. It leaves winit's
application delegate and event dispatch intact. A main-thread-owned Rust handler
keeps callback state alive, coalesces requests and wakes egui; dropping it detaches
the callback before freeing that state. Native Quit and window close both use the
application's existing departure guards. Only an authorized close reaches normal
worker shutdown and session saving. No framework delegate replacement or method
swizzling is introduced by PhotoSort.

Python scripts only export model assets or assemble a macOS bundle at build time;
they are not launched by the Rust application.


## Presentation bookmarks

`core::views` owns versioned per-folder presentation documents, separate from
`Session` file decisions and disposable image/model caches. The existing IO
worker reads views after its single catalog scan, reconciles referenced IDs and
fingerprints against that snapshot, and drains accepted saves across navigation
and shutdown. Corrupt view data emits a nonfatal warning and cannot prevent a
library from opening. No bookmark read decodes images or initializes a model.

`ui::bookmarks` checkpoints the active workflow at most once per second and at
workflow/folder/close transitions. It stores selection and range anchor, browser
filters/layout/sort, focus/comparison, inspection geometry, library scroll and
workflow configuration. It waits for required metadata/analysis before final
restoration; explicit user navigation or changed controls takes precedence.
Organize stores editor selection/folder/scroll separately from its mutable draft.
Pick Best restores an existing pair or starts that pair as a fresh review using
shared analysis, without serializing Keep/Trash choices or confirmations.

Fingerprint identity comes from the scanner's existing stat snapshot. Unix uses
device/inode and available birth time; Windows currently uses creation time in
addition to size/mtime and still needs native validation. Stable birth time avoids
false conflicts when the move executor creates/removes hardlinks. Older decision
sessions without identity migrate using their previous size/mtime check; new
presentation bookmarks require complete fingerprint equality.

A worker failure during deferred restoration leaves the saved bookmark intact.
Restoration remains suspended until a fresh workflow preparation or explicit
user navigation; it cannot checkpoint an incomplete view over the saved one.

Explicit cancellation also suspends deferred restoration; retrying analysis or
metadata reading resumes it through the existing worker owner. Discard checkpoints
the visible pair before resetting review history, then protects that checkpoint
through the subsequent departure and shutdown. Thus a fresh review can reopen the
same pair without restoring confirmed rounds or mutation decisions.


Application preferences live in a versioned `preferences.json`, independently of
library bookmarks and decisions. One visibility rule drives both navigation sizes,
Previous/Next, direct step shortcuts, and hidden-step resume. Hiding the current
step uses the normal departure guard; cancelling leaves it reachable until the
user departs. A successful catalog open updates bounded recent-folder history
without another scan. Accepted settings writes use the existing IO worker and
drain at shutdown; write errors do not cancel image or analysis jobs. Tests inject
an isolated settings destination. Startup argument parsing precedes graphics and
worker/model initialization; --last-folder chooses the first existing recent root.

Easy Delete sensitivity is an immutable value captured by each accepted analysis
request. The shared technical descriptor includes brightness, peak sharpness and
black-pixel fraction. Issue classification is a separate pure operation over that
cached descriptor, so changing thresholds does not invalidate measurements or
model outputs. Other workflow profiles retain their default thresholds. Changing
preferences never resets existing reviews or decisions; a future explicit or
restored Easy Delete analysis uses the new values. Analysis statistics now count
technical measurements to detect redundant work in regression tests.

### Shared cache maintenance

`core::cache` owns normalized cache limits, usage snapshots, generated-file
ownership and coalesced maintenance requests. The existing image actor applies
limits, evicts retained pixels and clears regenerable metadata/analysis entries.
Maintenance requests are independent of folder epochs, have scoped cancellation,
and never enqueue large results into the UI event channel. All workflows keep
current review results and presentation state during maintenance. The Settings
panel only submits requests and renders snapshots; no page traverses cache files.
Disk cleanup and --clear-cache share the same non-recursive fingerprint-artifact
filter, excluding symlinks, foreign files and application-data stores. RAM limits
cover retained pipeline entries, not externally held display textures, transient
decodes, metadata maps or model allocations. Disk trimming is periodic, so writes
can temporarily exceed the configured budget.

### Cached movement rules

The existing browse worker builds `navigation::Index` from its final visible
order and cached group/rating/mark records. It contains compact position lists,
not image or metadata copies, and is published with the matching view generation.
Navigation uses this snapshot for cyclic horizontal movement, rating jumps and
multi-photo group traversal. Staged ratings override file metadata. Stale/pending
view snapshots cannot drive movement. Settings persist movement preferences;
keyboard and Previous/Next buttons share one application navigation path. Other
workflow review controllers retain their own review progression. Building the
index is cancellable; selecting a target does no filesystem or model work.


Group ordering belongs to the existing browse worker. Incremental records share
immutable embedding `Arc`s with the model-result owner. Earliest-time and PCA
orders are cached independently of visibility filters and photo sorting; only
group membership, capture timestamps or embedding replacement invalidates them.
PCA uses group centroids and the smaller feature-covariance/group-Gram matrix,
reusing centroid storage for centering. It retains group IDs only after sorting,
not additional image/model buffers. The finite-iteration pure-Rust nalgebra
symmetric solver runs off the UI thread; cancellation is checked during record,
centroid and matrix work and before/after decomposition. An in-progress solver
call itself is not preempted. See [nalgebra's solver API](https://docs.rs/nalgebra/0.35.0/nalgebra/linalg/struct.SymmetricEigen.html).
No inference or filesystem work is started by changing group order. A saved
grouped view reuses the existing deferred-analysis resume path when its groups
must be reconstructed after restart.


### Shared same-subject pipeline

`core::subjects` owns immutable evidence and pure policy, candidate, geometry and
complete-link operations. Pure decisions have no image pipeline or model owner; persistence is isolated in `subjects::store`.
Subject and face assignment uses one maximum-total Hungarian match and the weakest
chosen score; all cross-pairs must be accepted before two clusters merge. Strictness
can be reapplied to raw pair evidence without decoding, model inference or geometry.
Semantic candidate search retains 32 nearest neighbors per image and temporal
search adds eight within ten minutes. Indices must be supplied in stable path order.
The shared `subjects::candidates` module uses one bounded heap per photo and
evaluates each finite semantic pair once for both endpoints. A full heap changes
only when the new candidate improves its worst member. Temporal retrieval merges
ordered equal-time groups to preserve stable ties without scanning entire bursts.
No N-by-N similarity matrix is retained. Semantic search remains quadratic; the
temporal pass is O(n log n + n*k), k=8. Synthetic descriptor-stage measurements and
the small additional heap-memory cost are documented in
`diagnostics/subject-candidates-2026-10-08/README.md`; they do not establish an
end-to-end application speedup.
Candidate and cluster operations, plus RANSAC loops, cooperatively check cancellation.

`Models::dense_embedding` uses the existing lazy DINO session and the same input
preparation as `Models::embedding`. A single forward pass supplies normalized CLS
and patch tokens; spatial pooling retains at most 16 region descriptors. Raw patch
tensors are discarded. Subject values are rounded to the reference's float16
precision but currently held in float32 vectors; this is not a half-memory artifact
cache. `half` was already a transitive dependency and is now declared directly.
No workflow constructs an additional model or image service.

The pure Rust geometry implementation uses mutual region matches, deterministic
RANSAC, a homography solve, explicit degeneracy checks and convex-hull coverage.
It matches clean-transform reference fixtures and strong/weak outcomes in the
included noisy/outlier fixtures. Sampling and refinement differ from OpenCV, so
exact noisy masks and real-photo equivalence are not established.

Cull `Subjects` and Pick Best `Quality` collect the same complete subject evidence.
Organize now has an explicit `Visual` profile, preserving its distinct algorithm.
The image actor owns all extraction, including face count/eye scoring, every face's
landmark-derived box and two eye positions, aligned face crops and DINO identity
embeddings. Face geometry is retained once and reused by scoring and grouping;
no independent face detector or model registry was introduced. Crop interpolation
follows [Pillow's transform cubic](https://raw.githubusercontent.com/python-pillow/Pillow/main/src/libImaging/Geometry.c),
with seven Python-derived pixel fixtures and a one-byte channel tolerance.

A separate policy worker performs candidate search, cached pair verification and
complete-link clustering. It cannot access a decoder or model. It publishes a
complete group map before AnalysisDone; provisional per-image results never drive
Cull/Pick Best groups. Epoch/run checks discard cancelled or superseded results,
including while waiting for space in the event channel. The UI shows the grouping
phase separately and keeps previews on the existing image actor.

`subjects::store` keys extraction by the pipeline's source/sidecar fingerprint and
model signature, and keys raw pairs by both artifacts and capture timestamps.
Strictness is deliberately excluded from pair keys: it reclassifies raw evidence,
running geometry only when a newly plausible pair needs it. Persisted files use
salted 64-hex names and the existing generated-cache ownership/budget rules. A
private temporary working set pins active evidence against persistent trimming;
it is released when its store's last owner leaves. The policy worker's regional
LRU retains at most 256 artifacts, rather than every image's dense regions. Global
vectors, candidate pairs and final pair records still scale with library size.
Temporary working files are outside the persistent disk limit and may be large;
zero disk cache disables persistence but still permits this transient working set.
JSON storage is currently used and binary packing remains a performance task.

Clearing caches retains active-job snapshots but forces a fresh store for the next
analysis. Folder epochs and model changes replace the store. Post-policy writes
schedule trimming through the same image/cache owner. Subject strictness is one
persistent application preference captured per run; changing it leaves current
review decisions untouched. Warm model evidence can be reused across workflows
and app restarts, while unrelated quality measurements retain their existing cache
lifecycle.


### Near-duplicate comparator

`core::near_duplicates` owns decode-independent exposure fitting, coherent-change
components, masked local/global SSIM, affine motion limits and face-motion evidence.
It borrows already decoded grayscale arrays and checks cancellation through the
measurement phases. It cannot scan, decode, initialize models, alter decisions or
mutate files. `Gray` rejects malformed/non-finite input; missing measurements remain
unknown. `Comparator` combines camera registration, normal-view and aligned face-crop
preparation with the complete acceptance policy. Its lazy face provider is invoked
only after cheap rejection stages, and cancellation is checked between providers.
The shared Runtime integration below replaces the former review heuristic.

Face-motion evidence consumes `core::faces::Face`, extended with the full landmarks
already produced by the shared verified Face Mesh pass. Existing eye/crop geometry
and all workflow consumers retain one model owner. Application analysis rows and
the in-memory analysis result cache receive only `FaceSummary` (counts and primary
bounds); full points remain in shared artifacts and bounded policy working sets. Legacy cache records deserialize
with empty landmarks; motion comparison treats these as unknown. Near-duplicate
integration must upgrade such missing landmark evidence through the shared owner,
without invalidating usable dense/identity evidence or treating unknown as no faces.

Connected components use a single eight-connected flood over the morphological
mask, collecting pixel differences during traversal. This avoids the reference's
full-image label scan for every component. Gaussian SSIM uses separable 11-tap
filtering and the reference reflected border, support-mask erosion and lower-quartile
tile rule. Numerical tolerances and reference cases are recorded in VALIDATION.md; no
whole-application speed or broad real-library accuracy claim is made yet.


Registration uses RustFFT for phase correlation and a native affine ECC solver at
half/full resolution up to a 512-pixel long edge. The normal system accumulates a
6-by-6 matrix without storing six image-sized Jacobian planes. FFT plans are reused
for consecutive pairs with the same padded dimensions and released when dimensions
change, so a mixed library cannot accumulate plans for every encountered shape.
The image warp preserves constant-color regions, which is necessary for stable
strict percentile trimming during exposure fitting. Tests use the actual reference
environment's OpenCV 5.0.0 interpolation, not the older 1/32-pixel approximation.

The independent Rust implementation follows the reference mathematical sequence;
implementation references are the upstream [ECC solver](https://github.com/opencv/opencv/blob/5.x/modules/video/src/ecc.cpp),
[phase correlation](https://github.com/opencv/opencv/blob/5.x/modules/imgproc/src/phasecorr.cpp)
and [warp kernels](https://github.com/opencv/opencv/blob/5.x/modules/imgproc/src/warp_kernels.simd.hpp).
OpenCV is used to capture verification fixtures only; it is not a Rust application
runtime dependency. RustFFT is locked to 6.4.1 in Cargo.lock.


### Duplicate review integration

Easy Delete requests the Duplicates analysis profile. The image actor owns dense
embedding extraction, technical measurements, hashes and metadata for all workflows.
A separate cancellable policy thread performs registration/comparison; it cannot
construct a decoder, model registry or pipeline. Bounded request/reply messages
return canonical pixels or lazy face geometry from the existing image actor.
Inspection and previews remain serviceable between requests. Every request checks
library/run generations and source-plus-sidecar fingerprints. Shutdown cancels
both sides of the request before joining workers.

Exact cohorts produce disjoint pairs first, in linear space, and need no comparison
pixels or faces. Remaining candidates come from the same general regional DBSCAN service
used by Visual Organize, with normalized global cosine >= .97. Accepted near pairs
are ordered by visual distance and stable source indices; each photo belongs to at
most one pair. The general regional service is described below.
The old perceptual-hash/recent-256 acceptance shortcut has been removed.

Full assessments are cached through the existing pinned Store and cache owner.
Keys include ordered source/sidecar fingerprints, model recipe and comparator/pixel
recipe version. Cancellation and missing face evidence are never persisted as valid
assessments. File identities are rechecked before final publication. Old face records
without landmarks are upgraded through the same model owner; unchanged face crops
retain their existing DINO identities. The UI retains old reviews until the final
pair set and completion arrive; it updates only affected duplicate-filter rows.

Keeper ranking is lexicographic: Python-compatible ties-to-even rounded sharpness
when both measurements are known, populated metadata count capped at 999, file size
capped at 999,999,999 bytes, then source order. The native Exiv2 reader includes
maker notes, IPTC, XMP and file information; all seven reference counts match.
Comparison now uses the shared neutral 1024-edge analysis input. Byte-identical cohorts intentionally bypass
Python's embedding-distance gate; byte equality is conclusive and avoids fragile
model-dependent exclusion. Technical issues take precedence over duplicate pairs.
No review result writes files or marks Trash until the existing confirmation flow.


Native Quit and window-close requests are evaluated after the current frame's
keyboard/widget actions and before the departure dialogs. This prevents a rapid
Confirm-then-Quit sequence from authorizing exit against the previous frame's
empty decision state. The same frame method is exercised headlessly, so ordering
regressions are covered through the actual shared workspace update path.


### Shared general regional clustering

`regional` owns the general similarity policy used by Visual Organize, Mixed
Organize and Easy Delete. It is distinct from `subjects`, which owns Cull/Pick Best.
`Models::similarity_regions` reuses the existing whole-image DINO descriptor and
encodes five reference crops (center, left, right, top, bottom) through the same
session. The image actor alone calls this method. Regional records are persisted
through the existing Store, pinning and disk-cache controls; keys include the
source/sidecar/model identity and a separate crop-recipe version. Changing the
clustering threshold does not invalidate reusable model output.

A single general policy job is queued after image extraction. The policy thread
loads normalized records in blocks of 64, retaining at most two blocks, and uses
union-find rather than allocating a dense distance matrix or neighbor graph.
DBSCAN with the reference fixed `min_samples=2` has exactly these connected
components; isolated points receive unique noise labels. Aligned regions use their
mean cosine similarity; unequal legacy region counts use the reference best-pair
fallback. Double-precision accumulation avoids drift with 384-dimensional vectors,
while stored descriptors remain f32. Portraits form a separate partition, actual
clusters precede noise labels, and input order resolves deterministic numbering.
Admissible identical normalized descriptors collapse before pair comparison; zero
or incomplete records are not assumed to have zero mutual distance.

Mixed mode uses the same service within each date, numbering multi-photo folders
and leaving singleton/unscored photos directly under the date. General results
publish atomically without replacing Cull/Pick Best groups. Easy Delete then checks
near pairs inside those completed general groups, through its existing comparator
and lazy image/face requests. Inspection and previews retain their shared actor.
Source identities and run generations guard each phase and final publication.

The similarity-distance setting has reference default .055 and range .02–.20,
normalizes invalid input, and is copied into the accepted analysis job. The UI
explains that changes apply to future analysis. There is no second decoder, model
registry, regional service or workflow-specific cache. The obsolete incremental
MixedGroups implementation was removed; date/planner tests now exercise this owner.
Worst-case comparison time remains quadratic for distinct descriptors, although
memory has no quadratic matrix/graph component and already-connected components
skip redundant comparisons. The 100,000-identical-record test is not a benchmark
of a 100,000-photo diverse library. Exact model preprocessing and broad real-photo
accuracy remain separately tracked parity requirements.


### Native runtime shutdown

The application entry point disables ONNX Runtime telemetry before native UI,
workers or dynamically loaded model code initialize. Cargo supplies the same
process environment to tests/examples, and the shared Models registry also sets
`with_telemetry(false)`. The early environment setting matters: the API alone can
run after the POSIX uploader has already initialized. A captured ONNX Runtime
1.29.0 crash showed its uploader callback accessing a destroyed recursive mutex
during C++ static teardown, after all Rust workers had joined. Disabling this
unused uploader avoids that lifecycle entirely; it does not bypass worker joins
or force process exit. Standalone library consumers should likewise set
`ORT_DISABLE_TELEMETRY=1` in their process environment before runtime loading.
See the [upstream telemetry initialization guidance](https://github.com/microsoft/onnxruntime/blob/main/docs/Privacy.md).

The opt-in native shutdown regression starts three independent sample-test
processes and requires successful process termination, catching failures that
occur after the test harness prints its passing summary.


### Shared technical pixel measurements

`pixels` owns RGB-to-gray quantization, reflect-101 border addressing, isolated
Laplacian variance and RGB8 area downsampling. Easy Delete, Pick Best and the
near-duplicate comparator use these primitives rather than maintaining different
luminance coefficients or edge rules. The comparator's existing fractional
alignment/blur resampling remains float-valued; quantizing that evidence to byte
images would change its purpose. Its border mapping now uses the shared helper.

The image actor computes `Analysis` through a cancellable combined measurement
pass. The prepared RGB frame is at most 1024 pixels on its longest side. Easy
Delete further downsamples, without enlargement, into 640×480 using ties-to-even
dimensions and area coverage. RGB is quantized before gray conversion. Local
sharpness takes the maximum isolated reflected-border variance over numpy-style
uneven four-by-four tiles; tiny frames use whole-frame variance. Pick Best's
whole-frame score is computed from the same prepared RGB data and stored on the
same `Analysis` value. The former second `CachedAnalysis.quality_sharpness` field
and separate runtime measurement call are removed. Future threshold changes
reclassify existing measurements; workflow changes do not repeat them.

Area resampling holds two scratch rows rather than a full floating-point image.
Sharpness accumulates stencil responses directly instead of allocating a
Laplacian image. Horizontal area accumulation uses explicit fused multiply-add
to preserve native OpenCV rounding. Cancellation is checked by row and tile,
and incomplete measurements never enter the cache. The initial bounded Lanczos
resize remains one non-interruptible image-crate operation.

The measurement contract is verified against Python/OpenCV on identical input
pixels. This does not establish byte-exact decoder or resampling equivalence.
The shared neutral source-to-1024 path described below removes the previous
2048-edge display-proxy reduction and enhanced RAW input. Rust's image-crate
Lanczos and JPEG decoder still differ from Pillow; palette images intentionally
filter decoded colors instead of reproducing Pillow's nearest-neighbor fallback.
These are shared pipeline decisions, not workflow-local compensations. The upstream area-filter implementation is
[OpenCV resize.cpp](https://github.com/opencv/opencv/blob/4.x/modules/imgproc/src/resize.cpp).


### Shared AI CPU allocation

`processing::Control` owns one atomic requested thread count. Persistent settings
resolve Balanced to 85% of available logical processors (rounded down, minimum
one), Maximum to all available processors, and Custom to a clamped count. Each
accepted analysis or rotation job captures a value; saving settings neither
restarts work nor changes its budget halfway through. Startup applies saved
settings before opening the folder. The settings dialog preserves current reviews,
ratings and history.

The single image actor applies that snapshot to its one `Models` registry. A
changed budget releases old ONNX sessions; the next uncached inference constructs
a session with the requested intra-operation thread count. Graph execution stays
sequential and idle thread spinning is disabled. No competing model or decoder
instances are created. Decoded pixels, technical measurements, regional vectors
and other persistent model evidence remain valid across budget changes. Repeated
cached analyses therefore need no model reload.

This intentionally differs from Python's batch-executor worker setting: Rust
configures AI inference pools, while shared image/file/policy actors retain their
existing ownership and concurrency. Native decoder internals and all other app
work are outside this setting; it is not a process-wide CPU usage limit. More
threads are not guaranteed to be faster. `bench_model_threads` probes DINO input
preparation plus inference on a sample and checks descriptors across budgets.


### Complete metadata through one shared snapshot

The default `metadata-native` feature uses a small, read-only Exiv2 C++ bridge.
One open/read provides selected display fields, raw date/rating/GPS values, image
dimensions and a populated-field count. Camera maker notes, IPTC and structured
XMP are counted as the Python cache does; scalar empty/`None` values are excluded,
collection values count once, and the five basic file-information entries remain.
Only selected UI fields cross the FFI boundary, not thousands of maker-note
strings. Duplicate keeper ranking still caps this component at 999 and uses
sharpness first, then metadata, then file size and stable source order.

`Pipeline` remains the single metadata/cache owner. Successful native reads skip
the fallback EXIF parser and embedded-XMP decoder. Minimal builds and unsupported
inputs retain the basic Rust reader; they do not provide full richness parity.
Sidecar rating/orientation/date precedence remains application-owned. macOS HEIF
container orientation still comes from ImageIO even when Exiv2 succeeds.

The bridge serializes Exiv2/XMP access, opens only local FileIo (never a URL
backend), catches all C++ exceptions, and retains no Rust buffers. Native reads
finish one file before cancellation, as do native pixel decodes; stale results
are rejected by the existing worker generation checks. No page creates its own
metadata reader. The image/cache recipe is now v3 and includes native-reader
selection, preventing stale proxies/evidence when orientation interpretation
changes between builds.

Exiv2 0.28+ development headers/library are required by full builds. Native macOS
CI installs it; the packager recursively includes its native dependencies.
Exiv2 is GPL-2.0-or-later; its notices are included under `resources/licenses`,
and its [source and licensing terms](https://exiv2.org/download.html) apply to
redistribution. PhotoSort's source license does not replace dependency licenses.

### Why there is no near-duplicate-distance slider

Inspection of Python `EasyDeleteWorker._detect_duplicates` shows that the
configurable distance only gates `_files_are_identical`. Subject-safe visual
comparison independently uses cosine similarity >=0.97 plus alignment, face and
visible-change tests. It does not use that setting as a visual acceptance cutoff.
Rust intentionally performs exact hash cohorts before similarity grouping; a
slider on the old hash gate would be redundant or hide provably identical copies.
The exact-cohort regression uses invalid embeddings and separate group IDs to
prove this behavior. Visual-group distance and subject strictness remain distinct
settings with their actual effects explained in the UI.


### Reviewed embedded metadata updates

`WritePolicy` is persistent application state; `WriteTarget` is serialized into
each reviewed action. The default preference embeds where the native reader
reports both EXIF and XMP writing support. Existing sidecars and unsupported
formats use XMP sidecars. Legacy action JSON defaults to Sidecar, and changing
preferences never changes a pending/retry action. Review rows identify the target.

Decision planning moved from the IO actor to the shared image actor, which injects
`Pipeline::metadata` into the common planner. Indexing, previews and planning reuse
one metadata entry; no decoder/model is created for planning. The standalone
sidecar-only planner remains available to independent core callers. Planning checks
cancellation between files. All actual mutations remain with the single IO owner.

The planner resolves shared-sidecar stems before creating actions. Two independent
embedded edits may have different values; edits sharing a sidecar must agree.
Only sidecar actions invalidate companion decisions; embedded actions affect their
own source. Source and sidecar fingerprints are captured in the reviewed plan.

An embedded update copies bytes to an exclusively created file in the same
directory, using an independent copy rather than the move helper's hard link.
The Exiv2 bridge writes EXIF and XMP values in one metadata transaction, rereads
and verifies both values, and catches native exceptions. Reads and writes share
one process-wide Exiv2/XMP mutex. Rust syncs the rewritten staging path, rechecks
the source identity and sidecar absence, then atomically replaces the source.
Failures clean the temporary copy and retain the action for review/retry. The
operation journal records accepted and completed actions as before. Cancellation
stops between mutation units; it does not interrupt a native container rewrite.

Verified embedded replacements produce same-path identity transitions in the
executor report. Session state and bookmarks accept those verified fingerprints;
rotation-only application retains unrelated staged ratings. Same-ID transitions
do not reset folder scope. Cache invalidation and catalog reload use the existing
application completion path. General plan application still clears the decisions
that it actually applied.

The active Python `RotationApplicationWorker` explicitly rejects lossy fallback;
the legacy lossy-confirmation dialog has no current UI/worker call sites. Rust
continues to avoid pixel re-encoding and offers a reviewable sidecar fallback.
HEIF container transforms and broader native-format interoperability remain
separate work; capability to write metadata is not a claim of validation on every
camera format.


### Shared neutral analysis inputs

`Pipeline::analysis` is the input boundary for accepted analysis measurement/model
runs and lazy duplicate pixel/face requests. Rotation uses the shared neutral
bounded working image through `Pipeline::load_cancellable`, then its existing
416-square resize and 384-square model crop. Avoiding the intermediate 1024-edge
resize preserves fine detail and reuses the browsing decode/cache. Its v2 cache
recipe invalidates earlier rotation predictions without changing user decisions.
Analysis prepares an oriented input no larger than 1024 pixels. Display previews and original detail
requests retain their own rendering recipe and never use analysis pixels.
Standard images share the existing bounded canonical decode cache, so requesting
analysis and display in either order reuses resident originals. After eviction,
a later display request may decode again; analysis does not eagerly generate full
display proxies for the entire library. RAW browsing and analysis share
LibRaw's embedded JPEG; missing, corrupt or non-JPEG previews fall back to camera-white-balanced,
half-size, 8-bit sRGB processing without automatic brightness or cosmetic edits.
The shared metadata orientation, including reviewed embedded/sidecar edits, applies
once to both RAW paths. Embedded camera-preview orientation variants beyond the
sample remain a format-coverage concern.

Both purposes share one metadata owner, one bounded image LRU and one disk budget.
Analysis pixels use a distinct fingerprint-derived cache key and lossless PNG
artifact, so restart does not decode again. No display thumbnail is generated for
analysis-only work. Source/sidecar keys include the neutral-input recipe revision,
invalidating earlier model evidence and pair assessments. RAW preview artifacts preserve full-source geometry from LibRaw, rather than
substituting embedded JPEG dimensions. Cache clearing, epoch invalidation
and maintenance apply to both purposes through the existing ownership boundary.

Cancellation is checked before preparation, after decode/resize and before
publication. Source identity is rechecked before caching. LibRaw progress callbacks
also observe cancellation during native work; Rust panics are contained at that
FFI boundary. Image-crate decode and resize may finish their current operation.
The shared-cache regressions cover both request orders, disk restart, source and
sidecar changes, cancellation and no enlargement. A generated palette checkerboard
explicitly documents the intentional filtered-color behavior: alternating black
and white averages near gray, while Pillow's palette thumbnail becomes white.


## People grouping: native recognition instead of appearance heuristics

Face Organize uses SFace/MobileFaceNet through the existing lazy `Models`
registry, image actor and ONNX runtime. It does not load DINO, run aesthetic or
technical measurements, or hash full source contents. Existing neutral analysis
pixels and verified BlazeFace/Face Mesh results supply all input. Detection is
stored independently of DINO artifacts and can be seeded from complete older
subject records without re-inference. Quality, Subjects and duplicate comparisons
consume the same verified detection artifact.

`face_groups` derives eye centers, nose and mouth corners from the shared mesh,
fits an orientation-preserving five-point similarity transform, and directly
samples 112×112 RGB NCHW float input. RGB remains in 0..255 because the ONNX
model contains its normalization. Bilinear sampling uses black borders. Invalid
geometry, eye separation below 12 source pixels, alignment residual RMS above
8 target pixels, no detected face, or a second face at least 60% of the largest
face's area yield no descriptor. These are conservative initial quality gates,
not calibrated confidence scores. Group portraits remain reviewable as
Unassigned rather than arbitrarily selecting someone.

One normalized 128-float descriptor is cached per admissible primary face.
The cache recipe includes source/sidecar identity, detector/mesh fingerprints,
SFace fingerprint and alignment/selection version. Missing or corrupt evidence
fails visibly; cancellation cannot publish partial groups. Warm extraction and
worker restart recover artifacts before decoding or initializing models.

The policy worker sorts records by stable media identity and admits a new face
to a group only when every unique member has cosine similarity at least 0.5.
Among admissible groups it chooses the best weakest match. Exact descriptors
collapse immediately. This prevents transitive A–B–C identity chains. It is a
deterministic conservative partition, not a globally optimal clustering or an
accuracy guarantee. Retained vectors cost 512 bytes per unique face plus
indices/maps; there is no pair matrix. Worst-case CPU on diverse collections
remains quadratic and requires further profiling.

Subject grouping deliberately retains its existing geometric pair policy: it
matches full scenes/multiple subjects and reuses dense DINO evidence, whereas
People grouping matches one identity across scenes. Reusing that pair graph
would add unnecessary quadratic edge storage. Both share extraction, sessions,
cache ownership, generation checks and worker lifecycle.

The UI owns a separate atomic people-group snapshot. Face, scene and culling
results cannot overwrite one another. Preview moves waits for completed people
analysis, then uses the existing editable folder draft and mutation planner.
No physical move occurs during recognition. Comparison uses the people snapshot
and does not treat all Unassigned images as the same person.


## Cached library navigator

Cull's folder/calendar/group navigator is a view of the existing catalog and
browse index. `core::browsing::tree` builds flat preorder nodes with parent,
subtree-end and sibling indices. Folder nodes reuse the scanned inventory,
including hidden/empty folders. Calendar nodes share capture-time resolution,
but preserve the written camera date across explicit timezone offsets rather
than shifting an event to a different day in the computer's timezone. Group
nodes consume Cull's existing subject groups. Node counts are whole-library
counts, clearly labeled; photo filters continue to combine folder scope, date,
similarity group, rating, search and decision state.

Construction, collapse projection and navigation membership run on the existing
browse worker with cancellation checks. Each mode has a cached immutable tree;
metadata-day changes invalidate calendar nodes, group changes invalidate group
nodes, and a new catalog replaces all trees. Ratings, marks, search and expansion
do not rebuild the tree or sort keys. Projection jumps over closed subtrees.
The UI holds the tree and projected rows through Arcs and draws only visible
rows; it never clones the complete projection per frame. No second scan,
metadata pipeline, image pipeline or model is reachable from tree construction.

The navigator is enabled in Cull's browse controls. Wide layouts use a resizable
side pane; compact layouts use a collapsible panel with bounded height. Empty
photo filters keep the navigator available. Selecting a node modifies shared
browse options, so all photo layouts and multiselection use the same result.
Folders and calendar nodes expand in the navigator; group members and photos
remain in the existing photo grid/list instead of duplicating thumbnail widgets
inside a second tree. This is an intentional UI redesign.

Folder/date mode supplies direct-parent membership to the shared movement index.
Left/Right cycles within that visible folder or calendar day, still honoring
marks and modifier behavior. Explicit similarity cycling retains precedence.
Previous/Next node controls select sibling folders, periods or groups. Existing
bookmark persistence owns enabled/mode/collapse state and the calendar filter;
older saved views default these fields without losing existing decisions.


## Durable manual similarity groups

`core::manual_groups` owns explicit photo membership independently of generated
AI group IDs. Cull's new/existing-group, restore and undo controls dispatch to the
existing IO actor; no extra scanner, decoder, cache, model or worker is created.
Moving to an existing group pins its current members plus the selected photos.
New AI members cannot silently join that cohort. Videos are excluded. The policy
is cancellable and returns immutable snapshots; folder epochs and an application
request ID reject stale results. Previous groups remain visible during analysis.

The saved session owns manual memberships and reconciles exact fingerprints.
Executor-verified relocations migrate memberships; output-library merges remap
local tokens instead of joining unrelated destination groups. Old session JSON
loads with empty corrections. Clearing regenerable caches cannot remove user
membership. Automatic groups remain a separate snapshot; reset overlays the
latest analysis without rescanning or decoding. After a restart without fresh
automatic analysis, restoring automatic membership returns those photos to
Ungrouped until Analyze runs. Undo retains one prior snapshot for this session.

Cull layouts, comparison, navigation and new Pick Best reviews consume one
resolved map. Existing Pick Best tournaments retain their captured comparisons.
The browse worker accepts an atomic group-only snapshot, retaining metadata,
embeddings and normalized sort keys; it invalidates group ordering, the group
navigator and cached picker counts/examples. The UI only constructs visible
picker rows. General visual/people Organize grouping remains separate; its
existing folder-draft editor owns manual destination adjustments.


## Shared model setup and recovery

`core::model_assets` declares the six native models once: workflow roles, expected
size, SHA-256 from the compiled asset manifest, and optional pinned download URL.
The image actor uses this dependency map for cheap prerequisite checks before
starting analysis or rotation. Structured `ModelsNeeded` events retire only the
matching analysis request; the app preserves review/decision state and opens the
shared setup panel. Retry is explicit and bound to the original folder/workflow.
A different folder, workflow or Organize grouping mode cannot resume an old request.

`Runtime` owns one lazy model-setup worker. It owns file verification and model
transfer, while `Models` remains the sole inference/session owner. The worker is
separate from imaging and file mutation so network stalls cannot block previews or
accepted photo operations. No thread, hashing, download, model session or extra
photo scan starts merely because the setup control exists. Its queue accepts one
operation; progress is coalesced and snapshots cross the UI through Arcs.

Imports/downloads write a private temporary file in the managed model directory,
streaming SHA-256 and enforcing an exact byte limit. Source/destination identities
are checked before commit. Only a fully verified file is atomically installed.
Failed or canceled transfers remove temporary data and keep prior files. A
completed atomic installation is retained if cancellation arrives afterward.
Verification is cached by full file identity and expected checksum; a successful
streamed install seeds that cache rather than rereading hundreds of MB immediately.
A fresh process verifies again. Session/photo caches are separate from installed
models. Replaced model identities invalidate the existing inference/evidence keys
at the next accepted analysis; installation controls wait for analysis to stop.

Native HTTP uses [reqwest](https://docs.rs/reqwest/0.13.5/reqwest/struct.Response.html#method.chunk)
with HTTPS-only production requests and certificate validation. A current-thread
[Tokio runtime](https://docs.rs/tokio/latest/tokio/runtime/struct.Builder.html)
runs only during a download. Cancellation selects against the entire HTTP future,
including header/body waits, and shutdown bounds resolver cleanup. There are no
archive extractors, dynamic download URLs, executable-runtime downloads or Python
conversion steps. Direct downloads currently cover DINOv2 and SFace; the remaining
prepared exports are restored/imported until native distribution artifacts exist.

The setup modal uses one scrollable body with persistent recovery/close controls,
including narrow windows and long errors. Unsupported AI builds explain their
limits. Runtime presence and an explicit orientation override are displayed
separately from verified managed weights; presence is not an ABI/load guarantee.


## Persistent measurements and lazy analysis inputs

The image actor is still the sole owner of measurements, source hashing, decoding
and model execution. `analysis::Measurements` serializes threshold-independent
technical evidence, analysis geometry and the full-content BLAKE3 hash into the
existing `subjects::store`. Its recipe key includes the pipeline source/sidecar
identity and neutral-input/measurement version, but no sensitivity or model.
Aesthetic probabilities have a separate source/recipe/aesthetic-model key. Review
choices, issue labels and combined AI scores are not persisted as raw evidence.
Validated records use the same generated filenames, atomic writes, temporary
active working set, periodic disk budget and explicit cache clear as other evidence.
Zero disk budget disables durable writes while permitting the active working set.

Each photo's analysis input is requested lazily and retained only for that unit.
Technical, dense, face, regional and aesthetic evidence consult their existing
owners first. `faces::cached_lazy` centralizes face lookup and only calls its pixel
supplier when neither shared nor legacy evidence is complete. It is also the
implementation behind existing eager callers. Fully cached Technical, Quality,
Subjects, Visual and Mixed runs need metadata but no image preparation; missing
entries compute through the same pipeline and lazy model registry. Rotation
requests its working image only when persistent rotation evidence misses; near-duplicate verification can request pixels for previously
unverified pairs. GPS-to-location enrichment moved to the shared metadata owner,
so metadata-only and image-backed analysis use the same grouping location.

New measurements and aesthetic scores are accepted only after cancellation and
source-key checks. Every analyzed photo checks the live source, sidecar and catalog
identity before publishing. Cache corruption is a miss; source replacements and
sidecar changes invalidate evidence. The existing fingerprint contract applies:
this is not a content scan on every restart, and external edits that preserve all
observable identity/size/timestamp fields cannot be detected without rereading.
The explicit `content_hashes` statistic measures actual full-file hash passes.

The descriptor map retains its existing 20,000-entry bound; cache snapshots and
models are not a total-process memory cap. Cache hits still read small evidence
records and metadata, and collection-wide grouping still runs. This improves
repeat work rather than establishing a speed ratio against the Python application.

JSON uses serde_json's `float_roundtrip` parser so reloaded measurements retain
their exact floating-point values, including at classification boundaries. A
real-sample restart exposed a one-ULP difference with the default parser; the
shared-store regression and complete native restart comparison cover the fix.


## Shared native video review

`Runtime` owns one lazy `playback::Player`. Selecting the active review video
starts a dedicated actor; every workflow uses the same canvas and controls.
Commands coalesce under a mutex/condition variable, and only the latest frame is
retained. Relative skips accumulate against the latest pending destination. One
seek runs at a time; intermediate destinations are replaced while the backend
finishes the active request. AVPlayer completion and GStreamer preroll/endpoint
state settle requests. Old seek frames and EOF cannot overwrite a newer request.
Transport and volume remain independently controllable. The timeline displays the
pending target immediately, and time counters show tenths of a second. A seek that
never completes fails after 15 seconds. Folder generations and source fingerprints
reject stale output. Selection changes, modal dialogs and focus loss pause playback;
shutdown cancels pending asset opening and joins the owner. Decoder errors release
the backend and expose Retry or external-player recovery. Photos never start it.

On macOS, an isolated ARC Objective-C bridge uses AVFoundation for local-file
demuxing, decoding, playback timing and audio. Asset reference restrictions forbid
external references. AVPlayerItemVideoOutput supplies BGRA frames through a
rotation-aware composition capped at 1280 pixels on the long edge and 30 fps.
Accelerate converts channels into a bounded Rust buffer; egui reuses one texture.
The cap bounds application-visible buffers, not AVFoundation's internal memory.
The composition APIs retain macOS 13 support despite newer SDK deprecations.
AVPlayer readiness also needs the application main event loop; the windowless
`video_probe` example supplies that loop explicitly and stays muted.

Video posters remain work for the existing image actor and shared cache. On macOS
AVAssetImageGenerator extracts a transformed, bounded poster with asynchronous
completion and cancellation. Windows/Linux default builds use the GStreamer
adapter for both playback and posters; builds without portable-video retain
external playback and a cancellable FFmpeg poster fallback. GStreamer owns
demuxing, decoding, audio, orientation and timing, with a one-frame RGBA appsink
bounded to 1280 pixels. Paused exact-end seeks can preroll a final frame without
posting bus EOF; the adapter recognizes a completed preroll at the reported
endpoint, as well as sink EOF without a frame. Playback has no growing frame queue
or per-workflow decoder/cache. No Python runtime or separately installed macOS
video runtime is required. Linux headless fixtures exercise the real GStreamer
engine; Windows execution and Linux desktop interaction still need validation.
Native fixture checks do not establish audible sync or broad HDR/color fidelity.
See `diagnostics/video-seeking-2026-10-06/README.md` for seek verification.


## Explicit folder spelling changes

`organize::Draft` retains original-to-current directory intent alongside labels
and empty-folder intent. Pure in-place case changes become `RenameDirectory`
actions at the start of the shared file plan, ordered from parents to children.
The planner rebases subsequent sources and cleanup paths without rescanning the
library or decoding images. It checks existing filenames at both present and
future directory spellings, including case-sensitive volumes. External output
libraries keep the existing companion-aware file move path.

Each directory action records the catalog inventory's regular files and directory
stamps. A cancellable mutation preflight walks the affected subtree to reject
new/missing entries, links, replaced directories and changed files; this walk
validates a reviewed mutation rather than rebuilding metadata or image state.
The executor then uses one no-overwrite rename, with no copy or temporary
intermediate name. The platform implementations use macOS `renamex_np(RENAME_EXCL)`,
Linux `renameat2(RENAME_NOREPLACE)` and Windows `MoveFileExW` without replacement.
Unsupported filesystem semantics fail explicitly; no replacing rename is used as
a fallback. Source validation remains a preflight snapshot, not a filesystem lock
against concurrent writers.

Primary API references: [Apple's case-variant exclusive-rename tests](https://github.com/apple-oss-distributions/xnu/blob/main/tests/rename_excl.c),
[Linux rename API](https://man7.org/linux/man-pages/man2/rename.2.html), and
[Windows MoveFileExW](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexw).
Only macOS execution has been validated locally. Linux case-folding filesystems
may reject same-entry exclusive renames; Windows behavior remains unverified.

A failed directory prerequisite stops dependent work and retains the remaining
plan. Cancellation during preflight returns a cancelled report without an error
or partial rename; cancellation between actions retains already completed
renames. Every completed path transition is collapsed to the original-to-final
identity before publication. This preserves decisions, manual groups and saved
views through nested renames followed by ordinary file moves. The shared
application worker saves these transitions and reloads the catalog. Journal
pending/completed entries cover directory operations. New accepted plans also
use the durable recovery protocol described below; uncertain actions are never
automatically classified as completed or replayed.


## Startup maintenance ownership

Startup arguments are parsed before any graphics, worker or model initialization.
Help/version return first. Standalone diagnostics cannot be combined with mutation
or folder-opening options and uses `model_assets::Inspector` for the same pinned
inventory as the setup UI. Its versioned JSON separates build capabilities,
verified model files, runtime-file presence and custom override presence from
runtime/model compatibility, which it does not test. The minimal build reports
neural inference disabled and does not require AI assets for file readiness.

`model_assets::clear_managed` owns model removal. It uses the existing six-asset
catalog and only the managed data root; it does not traverse resource search roots
or recursively remove arbitrary directories. The complete known-file set is
preflighted before deletion, and each fingerprint is rechecked immediately before
removal. Cancellation and partial errors return the exact completed files/bytes.
CLI startup prints this outcome and does not open the UI after an incomplete clear.
Inspector/model sessions have not yet been created, so there is no competing live
cache to invalidate in this startup path. Other application instances and external
writers are not locked out; this is a checked snapshot, not a cross-process lock.


## Native inference installation check

`diagnostics::check_runtime` runs only as an exclusive startup command, before
UI/runtime workers are constructed. It reuses the existing Inspector for pinned
file checks, then creates one `Models` owner with one CPU inference thread and one
decoded embedded fixture. Scene descriptors, aesthetic scores, detection/landmarks,
SFace recognition and orientation all call their production services. There is
no second inference implementation, alternate model catalog or workflow cache.
The Apache-2.0 OpenCV Zoo fixture and its provenance are embedded from the existing
recognition test fixture; no user photo or application cache is opened or written.

Two passes verify output contracts and one-time initialization of six sessions.
Counters and per-stage durations are reported; they are installation evidence,
not a startup/performance or photographic accuracy benchmark. A failed first pass
retains stage results and skips the second pass. Cancellation is checked during
file hashing, between stages, inside the existing face pipeline and after the
last inference. A native inference call can finish its current unit before
cancellation is observed. An invalid model/runtime that crashes native code or
hangs still requires the caller's process timeout/crash handling.

Runtime presence now shares the analysis prerequisite rule: a nonempty regular
file. Presence alone never establishes ABI support. Diagnostics preserve original
PathBuf values internally and serialize display text so non-UTF-8 names do not
break JSON output. An explicit orientation override selects its own runtime
contract instead of requiring an unused bundled orientation file; the inventory
retains that default row with `selected: false`. Successful runtime checks mark
loaded/exercised status and the override's interface compatibility, without
claiming semantic accuracy, graphics support, codec compatibility or GPU use.


### Movie recording times

The shared metadata reader owns movie capture timestamps, so all workflows use
the same snapshot and source/sidecar invalidation. A bounded native Rust reader
seeks past media payloads to `moov/mvhd`, reads version 0/1 creation seconds,
validates extended/zero-size atom ranges and shares a 4096-atom total budget.
It introduces no decoder, playback actor, subprocess or additional dependency.
The epoch and UTC interpretation follow [Apple's movie-header documentation](https://developer.apple.com/documentation/quicktime-file-format/movie_header_atom).
Version 1's 64-bit creation field is also used by [FFmpeg's MOV demuxer](https://github.com/FFmpeg/FFmpeg/blob/master/libavformat/mov.c).

Explicit EXIF/XMP dates keep precedence; usable movie times precede filename and
filesystem fallbacks. Chronology stores absolute milliseconds; calendar/grouping
use the current local date with an explicit offset in displayed capture text.
Old cameras that wrote local time into this nominally UTC field cannot have
their original timezone inferred. Unsupported containers, zero/pre-Unix values,
malformed or out-of-range headers retain existing fallbacks. This does not parse
Apple keyed creation-date metadata, compressed moov atoms or all container tags.

Metadata requests now carry cancellation through the existing image actor and
pipeline. Traversal checks between atom headers; cancelled snapshots are neither
cached nor published by metadata indexing. Native EXIF calls and a currently
blocked filesystem read must still return before cancellation can take effect.
Repeated browse/analysis/image requests share one cached metadata snapshot;
video source keys carry the new recipe while photo keys remain unchanged.


## Accepted operation recovery

All five workflow UIs submit accepted plans through `ApplyRecoverable` on the
existing IO worker. `operations::recovery::Store` writes a versioned complete plan,
source-library context, staged decisions/manual membership and view bookmarks to
one unique audit journal before any file action. A small per-library index points
to the active journal. Each executor action records a synced pending index, then a
synced completion with its identity transitions, or failure/cancellation. Cleanup
prerequisites and chained relocations share the same `Report::complete` rule in
normal execution and replay. Action records grow linearly; no whole-plan rewrite
is performed after each move. The header streams to disk without cloning the
entire plan/state first. Recovery media snapshots include only staged-decision
companions needed to retire shared XMP edits. Session fingerprints include only
staged decisions and manual-group members; bookmark identities stay in the view
store. Context capture queries the existing index for those IDs instead of
walking or copying every untouched photo. A small accepted edit therefore does
not append a whole-library fingerprint inventory to each retained journal.

A nonblocking [standard-library file lock](https://doc.rust-lang.org/std/fs/struct.File.html#method.try_lock)
serializes local instances using the same library index. A retry/discard ticket
includes the unique journal name and fingerprint. Stale windows cannot replace a
newer plan or apply different destinations; the UI reloads current recovery state.
Each explicit retry gets a new journal, preserving older attempts byte-for-byte,
including torn tails. Discard removes the active index after reconciliation while
retaining audit journals and all media. Locks cover this application's recovery
protocol, not external writers, different machines, or all session editing.

On library open, the worker streams JSON through a bounded input buffer, checking
cancellation between records and file-buffer refills. Headers are limited to
512 MiB, with the same limit enforced before acceptance or media mutation;
unaccepted temporary output is removed on error. Action receipts allow up to
2 GiB to accommodate file-identity arrays while still parsing incrementally.
There is no second whole-record JSON byte buffer, and action journal writes also
stream borrowed structures instead of constructing duplicate JSON value trees.
The restored plan/state themselves still require memory proportional to the
accepted work. The reader validates action indices and record transitions. A final record lacking its newline cannot prove completion. Recorded
completions restore session identities and view focus before the normal catalog
scan/reconciliation; unconfirmed actions stay in the reviewed plan, and pending
ones are explicitly uncertain. No media inference, metadata reads, secondary
image pipeline or automatic file operation is involved in this recovery pass.
Only the normal shared catalog scan then indexes the current filesystem. Old
unindexed journals cannot reveal never-started actions and remain audit-only.

The accepted context owns persistence until its receipt is reconciled. UI
session autosave cannot enqueue a pre-move snapshot behind the executor, and
completion handling does not checkpoint old bookmark identities. Session and view
writes are completed before the active index is marked settled; interruption in
that sequence replays from the saved context. Settled partial plans do not replay
state again on every scan. UI reconciliation may later save a newer live view via
the same session/view services; this is a later checkpoint, not another file
mutation or catalog/model owner.

Recovery supports abrupt process termination. It does not promise power-loss
atomicity across multiple filesystem files, infer success from a destination's
mere existence, automatically repair partially moved companion sets, or let users
edit individual retry actions. Current original-source and no-overwrite checks
still decide whether an explicitly reviewed retry can execute. Native desktop
interaction and platform behavior outside macOS require separate validation.


## Workflow resume after moving to a different library

`Report::relocations_into` intersects verified final identity receipts with completed
move destinations under the output root, using component-aware paths. Failed,
uncertain and metadata-only operations cannot import presentation state. This
selection uses receipts and reviewed paths; it does not scan or decode files.

`views::Saved::import_relocated` is the sole merge rule for all five workflows.
Only source bookmarks referring to verified arrivals are imported. Their focus,
selection, range anchor, complete comparison sets, workflow-specific photo
references, layout and inspection preferences follow the mapped identities.
Incomplete comparisons are discarded. Destination bookmarks for unrelated steps
and their fingerprints are preserved. The source's current step is adopted only
when its bookmark was imported. Imported folder scopes, generated group numbers,
collapsed navigation entries and scroll positions are reset because they belong
to a different inventory; photo anchors remain available for normal restoration.
Organize editor photo selections move with their identities, while source-only
folder selections are cleared. Normal catalog reconciliation still rejects any
arrival that was subsequently replaced.

The existing recoverable operation context saves destination views before marking
the journal settled. Both normal execution and interrupted-operation recovery use
this path. Later retries with no referenced arrivals leave existing destination
views untouched, and re-opening a settled partial plan cannot overwrite newer
view changes. No workflow constructs a separate pipeline, cache, model or worker.
Concurrent independent windows editing the same destination view are not given a
new cross-window merge protocol by this change.


## Supporting-file identity and Organize resume

The shared catalog owns supporting-file identity snapshots. Its existing walk
still discovers files once; after media directories are known, a cancellable pass
over that inventory stats only files eligible for Organize. Eligibility is one
shared predicate used by both the editor inventory and fingerprint collection:
non-media files within media directories or their descendants, excluding XMP
companions. Symlinks remain excluded. Fingerprints contain size, modification
time and platform file identity, using the same identity reader as photos. This
adds one metadata stat per eligible supporting file to initial scanning; it adds
no content reads, image decodes or secondary filesystem walk.

Presentation checkpoints use the catalog's identity maps, and worker reloads
reconcile against the same scan snapshot. Supporting identities validate only
Organize editor file selections; they cannot become active review media, rating
selections, comparison pairs or generated-group anchors. Old view files remain
readable and naturally lack these additional identities until checkpointed.
Verified move receipts already relocate editor selections/fingerprints, including
moves into another library through the shared operation recovery context.

The editor intersects saved file and folder selections with hash sets built from
its current rows. Membership checks now scale with rows plus selected entries, replacing
a scan of all rows for every selected file/folder; folder selections retain
their existing ordered-set storage. Staged organization edits still
belong to drafts and are not restored by presentation bookmarks. No separate
workflow worker, cache or image pipeline is introduced.


## Shared application shortcuts and input ownership

Application shortcut dispatch uses the existing folder opener/departure flow and
the same layout/navigation setters as pointer controls. Global Open, Find and
About actions precede ordinary text handling, but remain blocked by active modal
UI, popups and file-worker busy state. Global actions ignore held-key repeats and
accept only their defined modifier combinations. Find requests focus on the
search widget's stable ID; it does not clear the query or bypass review guards.
Pick Best has no library search field and does not acquire invisible focus.

Cull-only layout and navigator keys are handled in ordered photo-key dispatch,
before comparison slots or ratings. They reuse cached navigation; toggling group
navigation does not launch analysis. Unbound Ctrl/Cmd combinations cannot fall
through to unmodified photo edits; explicit ratings, movement, Select All and
zoom remain available. Ordinary typing and selection stay with focused text
fields. If an earlier action opens a departure prompt in the same event batch,
later photo keys stop at that modal boundary. The keyboard guide uses a bounded
scrolling window rather than growing beyond short screens.


## Bounded regional scene inference

The shared model registry accepts at most five image tensors in one native call.
Regional similarity prepares its five crops from the current neutral image and
submits one batch to dynamic-batch DINO exports. The already cached whole-image
embedding remains region zero. Crop geometry, RGB conversion, resizing,
normalization and descriptor ordering are shared with individual embedding calls;
no additional decode, prefetch queue, model instance or workflow worker is added.
Only one crop image is retained during preparation, plus at most five 224-square
NCHW inputs (about 2.9 MiB). Native intermediate memory is additional and measured
separately; fewer calls do not imply lower peak RAM.

The registry inspects the existing session input shape. Fixed-one exports use
individual calls through that same session, checking cancellation before each;
it does not swallow a failed native batch and blindly retry arbitrary errors.
Tensor count/length/overflow guards precede model IO. Native batch outputs must
have the expected leading dimension and finite data. Cancellation is checked
during crop preparation, before native work and after it returns; a running ONNX
call itself remains non-preemptible. Cancelled work cannot publish a regional
artifact. The cache recipe changes only for regional vectors, preserving reusable
whole-image, face and technical evidence.

This optimization is limited to five crops from one photo. Cross-photo batching,
face/aesthetic/orientation batch scheduling and GPU providers remain separate
work. The opt-in batch probe is intentionally a standalone measurement of the
same native session options, rather than another application inference service.


## Timed startup smoke checks

Startup parses --smoke-test and --smoke-delay-ms before graphics or workers. The
application stores one optional timer, initialized by its first UI frame using
egui's monotonic input time. Repaint scheduling wakes an idle welcome screen at
the deadline without a timer thread or polling worker. Expiry consumes the timer
and feeds the same close request as native Quit, after that frame's keyboard and
widget actions. All five workflows therefore retain their review, staged-change,
organization and file-operation guards. Cancellation does not rearm the timer.
Normal on_exit still checkpoints bookmarks and drains accepted worker writes.
Smoke mode does not assert scan/analysis completion or force termination; use a
clean disposable library and an external CI timeout.


## Compact shared analysis evidence

The existing subjects::store::Store owns serialization for subject/dense features,
regional crops, verified faces, recognition, measurements, aesthetic scores and
pair assessments. All workflows continue to receive typed values from this owner;
there are no page-specific serializers or additional resident descriptor caches.
New snapshots use Postcard 1.1 with only its alloc feature, wrapped in a PhotoSort
header containing format/schema version 1, an explicit artifact type and a BLAKE3
payload checksum. This removes decimal-float parsing and text expansion while
preserving f32/f64 bits. The version covers our complete serialized Rust schemas;
adding/reordering fields or enum variants requires a version bump or an explicit
migration. Postcard's stable wire format does not make application schema changes
automatically compatible. Its [format documentation](https://docs.rs/postcard/1.1.3/postcard/)
explains the underlying Serde representation.

Reads are capped at 8 MiB, reject non-files/links, enforce type/version/checksum,
reject trailing payload bytes and retain each artifact's existing semantic
validation. A malformed current binary record is a cache miss, never a fallback
to potentially older JSON. Valid legacy Rust JSON cache entries migrate lazily
through the same validator and writer; model/source recipe keys are unchanged,
so format migration does not request another decode or inference. Legacy JSON
files remain subject to normal cache maintenance. Session decisions, recovery
journals, settings and bookmarks keep their existing formats.

Binary .bin and legacy .json records share the existing cache owner/budget.
Atomic writes still publish complete files. A Store's private working directory
pins snapshots for active analysis/grouping and survives persistent cache clearing;
disabling disk persistence retains only that active working set. Migration writes
use the same pinning path, and parallel readers may safely publish identical
validated snapshots. No unbounded in-memory cache is added. General similarity
clustering still bounds descriptors to two 64-record blocks; this representation
change does not remove its worst-case quadratic pair comparisons.

## Persistent shared metadata — 2026-10-02

`Pipeline` now persists metadata-only snapshots before display decoding or
geographic lookup. Browsing, grouping, image preparation, inspection and operation
planning keep using this single owner. Metadata disk hits never request pixels
or initialize models. The former analysis-only envelope is now owned by
`core::cache_record`; both pipeline metadata and `subjects::Store` use its bounded,
checksummed typed format. Existing analysis record bytes and identities remain
compatible. Metadata uses a new artifact kind and a versioned recipe including
the source/sidecar identity, native metadata capability, HEIF feature and OS.

Records contain the raw typed snapshot and the local timezone offset at its
captured instant. A changed offset rejects the record, so local EXIF/movie dates
are reread. Geographic names are resolved through the current geography owner;
decoded sensor/display dimensions remain in the separate existing geometry
record. Metadata records are not backfilled from old preview JSON, which can
contain those derived dimensions. Source identity is rechecked before persistence
and publication; cancellation rejects incomplete/stale results. Corrupt records
and write failures fall back to normal source reads. Cache clear, disabled disk
persistence and the shared disk budget cover the new records.

Binary cache writes use exclusive temporary files and atomic replacement without
per-record physical disk flushing. This applies only to regenerable metadata and
analysis artifacts, including active-job pins. A crash may lose a cache entry;
bounded reads and checksum validation then cause recomputation. Photo edits,
settings, decisions, bookmarks and operation journals retain their existing synced
write path. The first benchmark attempt demonstrated why this distinction matters:
forcing a flush for each metadata record made initial ingestion substantially
slower despite fast reopening. Detailed before/after measurements and limitations
are retained in BENCHMARKS.md.

## Shared Organize naming intent — 2026-10-03

The Organize editor now stages base-name edits alongside its folder draft,
including supporting documents and extensionless files from the same catalog
inventory. The inspector opens that editor instead of maintaining a second edit
control. A private name-map snapshot isolates Cancel; Keep/Preview publishes names
to the existing application-owned file_names map. Keep/Preview also accepts the
currently typed name, while invalid input keeps the editor open. Folder layout
and naming still enter the existing reviewed operation planner and mutation actor.
Extensions remain unchanged intentionally: a rename must not suggest converting
an image or disconnect the actual file format from its suffix.

core::organize resolves naming requests by photo cohort before the planner visits
files. A request on the later RAW/JPEG member therefore cannot disappear because
the earlier member already claimed its companions. Enabled photo companions use
one base name; videos and supporting files retain independent identities, even
when an extensionless document shares a photo stem. Conflicting explicit cohort
names fail before mutation. Editor changes replace the cohort's prior intent
atomically, and restoration removes its overrides. The resolver uses only the
existing inventory, has cancellation boundaries and skips empty naming maps.

The original planner still owns extension-preserving destinations, one collision
suffix for a cohort, source snapshots, exclusive moves, operation journals,
retry/recovery and central identity/bookmark migration. No page-specific file
operation, rescan, metadata reader or image decoder was introduced. Name edits
are staged intent; accepted operation plans retain their existing restart recovery.

## Shared operation-plan review — 2026-10-03

The application installs each immutable review plan through `set_review_plan`.
Its presentation index retains one cumulative row offset per operation group;
it does not copy file paths, scan the filesystem or request metadata/images.
The same setter handles new plans, restored retries, failed dispatch restoration
and clearing on folder/workflow transitions. Replacements refresh scroll identity.

All workflow dialogs use `ui/plan_review.rs`. Each move/rename or Trash member is
an independently scrollable visible row while its original companion operation
group remains intact. Binary row lookup and egui virtualization format only the
visible entries. Directory and metadata actions retain their own semantics.
Before/after paths use labeled source/destination roots and support horizontal
scrolling. The existing planner, worker, mutation guards and recovery journal
remain authoritative; the review index is disposable presentation state only.

## Workflow-aware status publication — 2026-10-03

`ui/status.rs` owns status publication and footer visibility. Each message is
shared activity/recovery or carries its originating workflow; Organize results
also carry their grouping mode. Asynchronous review completions name their
owner explicitly, rather than borrowing whichever step is visible when the
result arrives. The footer substitutes current-library readiness for unrelated
workflow instructions. Navigation notices and model-recovery feedback retain
their existing priority.

All production status writes use the shared setters. New shared progress,
filesystem receipts, recovery and library-open messages clear previous scope.
Status filtering does not alter accepted jobs, review decisions, cached results
or worker cancellation. It performs no metadata, image or model requests.

## Video metadata geometry and controls — 2026-10-03

Video panels explicitly own a top-down controls layout inside the shared
horizontal review slots. Buttons, timeline and time occupy separate rows; slots
under 280 points reuse the existing compact menu. Playback, decoder and texture
ownership are unchanged.

The existing bounded `metadata::movie` reader now reads both creation time and
first-video-track presentation dimensions in one open. It finds the vide handler
before reading the v0/v1 tkhd, applies its affine matrix to the fixed-point track
size and rejects unsupported perspective. One atom budget and checked parent
ranges cover all lookups; no file-sized allocation or media-payload scan is used.
The shared metadata cache persists this result. Video pipeline keys include the
new geometry recipe; photo keys remain unchanged.

Video posters and playback surfaces stay bounded independently of source size.
Neither canonical poster decoding nor cached poster JSON is allowed to overwrite
video metadata geometry. This rule is shared across all five workflows; the
inspector only presents known dimensions. Existing photo canonical-geometry
behavior is retained. Current coverage is track presentation size in MP4/MOV/
M4V/3GP/3G2, not a general demuxer or a claim about movie-level transforms,
clean aperture or pixel aspect ratio. Broader container support remains open.

## File spelling changes — 2026-10-03

The shared Organize planner emits `RenameFile` for same-parent case-only name
changes. It resolves companion naming before traversal, distinguishes existing
source aliases from distinct entries, and rebases through previously planned
folder spelling changes. Other moves retain the existing Move implementation.

The filesystem worker reuses the same no-overwrite platform primitive as folder
spelling changes. File preflight checks source fingerprints, exact directory
entries and symlink components. A per-pass directory-name snapshot avoids one
read_dir per file; successful case renames update the snapshot and other actions
invalidate it. No per-workflow mutation path or duplicate pipeline is introduced.

Each companion receives its own rename action and durable completion record.
Shared reports publish verified old/new identities, preserving staged decisions,
manual memberships and view migration through the existing owners. Cancellation,
restart and retry operate on exact unfinished actions. There is no multi-file
atomicity promise and no temporary-name/copy/unlink fallback. The common review
renderer exposes every source/target spelling before application.


### Managed model storage controls (2026-10-03)

`model_assets::Inspector` supplies each catalog row with an optional opaque
`ManagedCopy`: the catalog identity, exact managed path, file fingerprint, actual
byte size and canonical model directory. Links, directories and absent files do
not become removable copies. The same shared service reviews CLI cleanup and
executes both CLI and UI removal, rechecking root ownership, directory resolution
and the reviewed file identity before each unlink. The CLI retains its all-files
preflight and partial-result/cancellation report.

The existing lazy setup worker owns `Work::Remove`; it invalidates its verification
snapshot before mutation and publishes a newly inspected fallback or missing-model
state afterward. Failure/cancellation leaves readiness unavailable until refresh,
rather than presenting the old copy as installed. Existing Inspector hash caching
also applies to bundled fallback, so already-verified unchanged models are not
hashed again. Inference remains in the image owner's model registry: its existing
path/fingerprint checks and per-run preflight handle subsequent model selection.
No workflow adds an independent model cache, filesystem worker or decoder.

The common Local AI models dialog displays aggregate managed size, per-copy size,
exact reviewed path and fallback consequences. A per-copy review supports Keep
copy and Remove reviewed copy; active analysis/file work disables mutation. This
intentionally offers selective removal instead of copying the Python dialog's
single Delete All Models action. Closing or refreshing clears an unaccepted removal
review. Completed photo reviews and staged decisions are unaffected. Resident ONNX
sessions are not a new memory-management target of this disk-storage control.


### Multi-person Organize results (2026-10-04)

`face_groups::Recognition` now contains a bounded set of normalized 128-float
SFace descriptors. The existing 60%-of-largest-face area rule selects the main
people; small background faces do not change a foreground portrait's destination.
Every selected face must align successfully before any recognition inference is
started. A missing/unclear main face makes the set empty instead of quietly
assigning a partial group. The detector's existing ten-face limit bounds tensor
memory; recognition calls share the same model registry and one image decode.
The recognition recipe key is versioned independently of shared face geometry,
so existing landmark evidence remains reusable while old single-person recognition
records cannot satisfy the expanded schema.

Grouping retains conservative all-member admission and exact-descriptor reuse.
Unique descriptors are sorted by their contents before admission, independently
of photo IDs, paths and input order. Verified moves therefore cannot renumber
people or change admission when the descriptor set is unchanged. Combination
numbering uses ordered person sets. This does not promise permanent person numbers
when photos, model outputs or recognition policy change. A photo whose main faces
resolve to the same identity is ambiguous and remains Unassigned. Otherwise its
sorted person set becomes one destination: Person 001 for a single identity or
People 001 + 003 for that exact combination. Combination groups are distinct from
individual person groups. No photo is copied into several people folders.

`Event::FaceGroups` publishes the shared grouping Output (memberships and folder
labels) atomically. The application retains this as one state value; the inspector,
folder editor, comparison and existing companion-aware move planner consume it.
Old/cancelled runs cannot replace the previous output. All moves still require the
standard per-file review. This replaces the Python primary-face-only choice with
an explicit one-destination multi-person policy, without adding workflow-specific
model or filesystem ownership.

The exact-descriptor lookup is released after gathering. Clustering member lists
and descriptor payloads are released before output construction; per-photo index
buffers are reused for identity sets. Memory remains linear in faces/photos;
all-member comparison still has quadratic
worst-case CPU cost for diverse descriptors. A group portrait requires more SFace
calls than the old immediate-Unassigned shortcut. Broad identity calibration,
small/distant-face detection, crowded scenes and large-library timing remain open;
passing the composited close-up fixture does not establish those results.


### Stable visual destination numbering (2026-10-04)

The shared regional cluster service numbers components using the minimum normalized
evidence digest in each component. It reuses digests already computed for exact
descriptor deduplication. Union roots, source paths, cache keys and scan order no
longer select numbers for distinct evidence components. Portraits still precede
landscapes; real clusters still precede singleton noise within each partition.
DBSCAN membership, distance thresholds and orientation/date boundaries are unchanged.

Visual Organize and Date + similarity consume these numbers through the existing
grouping worker and companion-aware planner; Easy Delete uses the same regional
membership service. No workflow-local numbering or cache is introduced. The exact
deduplication lookup is released before pair comparisons. Canonical numbering keeps
only linear-sized digests, component keys and labels, with cancellation checked
throughout. It requires no extra evidence loads, decoding or inference and retains
the 128-descriptor working-set bound. Diverse inputs retain quadratic worst-case
comparison cost; this change is a destination-stability improvement, not a faster
clustering algorithm.

Numbers may change once from the former scan-order scheme. They remain stable when
the same valid evidence is reordered or source identities change. Adding/removing
evidence, changing models, dates, orientation or thresholds can change numbering.
Identical non-mergeable evidence (for example zero vectors) cannot distinguish
individual photos by content; only those ties fall back to cache key/node order.
No persistent person/scene identity is claimed for that case.


### Shared automatic Organize preview prerequisite (2026-10-04)

Visual, Faces and Date + similarity now use one captured `(run, mode)` owner for
automatic Organize analysis. Preview moves starts the matching profile, joins an
already accepted matching run, or supersedes an incompatible run through the same
runtime generation mechanism. A pending preview dispatches to the existing IO
planner only after completion, in the original workflow and mode. Cancellation,
errors, folder reset, workflow departure and mode changes clear that intent.
Completed snapshots survive cancelled reanalysis. An explicit manual folder draft
can still be reviewed without restarting AI.

Completed regional visual membership is an Option snapshot: None means not yet
available; Some(empty) is a completed pass with no usable memberships. It is
separate from provisional technical grouping, which remains available for its
existing comparison fallback. These are different algorithm results, not duplicate
pipelines or model owners. Only verified regional results supply Visual scene
destinations; failed/ungrouped photos and videos remain in place. The inspector,
editor, departure guard and planner share the same readiness/label ownership.
Faces and Mixed retain their established destination policies.

Model recovery captures the original request with named root, workflow, mode and
preview-intent fields. Explicit Retry analysis starts a fresh shared run and
re-registers the preview only when that was the original action. A plain Analyze
request does not acquire a move preview, and changing context invalidates recovery.
No photo move is applied automatically: the standard final review remains required.


### Transactional background-job admission (2026-10-04)

Analysis, rotation and metadata indexing share RunGeneration at the runtime
boundary. A short admission gate serializes nonblocking enqueue, publication of
the accepted run ID and explicit cancellation. A full or disconnected queue leaves
the previously accepted run valid. The receiver synchronizes at receipt so it
cannot reject a newly enqueued message before the sender publishes its ID. It
rejects obsolete analysis/rotation/metadata messages before replacing pending work
or preparing models and evidence stores.

Running image, metadata, duplicate and subject work still reads an atomic ID for
cancellation. The gate is never held during decoding, filesystem work, model setup,
inference or result publication. Analysis and metadata have independent generations;
rotation shares analysis ownership. No workflow-specific queues or pipelines were
introduced. Model preflight/setup publication checks cancellation after synchronous
setup stages, which may themselves finish before cancellation is observed.

Shared-boundary regressions cover rejected full/disconnected requests, actual
accepted-worker completion, receipt before admission commit, concurrent explicit
cancellation, independent generations and obsolete requests interleaved with new
work. Obsolete work must produce no decode, metadata-read or model-inference cost.


### Canonical default destination after discovery (2026-10-04)

The IO scan owner already resolves relative paths and symlink aliases before
loading the catalog, decisions, saved views and interrupted-operation records.
Opened adopts that catalog root for shared application state and recent history.
It now also updates the default destination when the field still equals the
original requested folder spelling. A destination edited while discovery runs is
retained, including an empty or invalid draft; normal planner validation applies.

Canonicalization remains off the UI thread. No new scan, path resolver or per-page
opening implementation is introduced. CLI arguments, folder pickers, drag/drop,
recent folders and post-operation reloads all receive the same Opened handling.
The source catalog remains the authority for photo IDs, cache keys and saved-state
identity; existing canonical saved state requires no migration.


### Independent durable scene membership (2026-10-04, storage phase)

The existing ManualGroups policy owns both subject and scene membership. Session
keeps the established manual_groups field for Cull/Pick Best and a separate
optional scene_groups field for general scene corrections. Missing scene_groups
loads as empty, preserving older Rust sessions. Exact photo membership remains
independent of regenerated AI numbers; destination merges remap library-local
tokens separately in each namespace. Verified identity changes and fingerprint
reconciliation apply the same policy to both.

Application session_intent/session_snapshot builders supply all persistence paths:
normal saves, shutdown, reviewed operation context and post-apply reconciliation.
The shared scan restores both namespaces and folder reset clears both. Sparse
recovery context captures fingerprints for the union of decisions and membership
in either namespace, so a scene-only edit survives interrupted file operations
without journaling every unedited photo in a large library.

This change prepares durable ownership only. Scene editing controls, general
analysis publication and Easy Delete candidate integration are not connected yet.
They must reuse the existing cancellable EditGroups worker and ManualGroups
policy, preserve the subject namespace, and respect date partitions in Mixed mode.
No second image pipeline, model registry or analysis cache is introduced.


Verified identity remapping now uses one shared map helper for staged decisions
and both membership namespaces. It detaches all old values before inserting new
keys, preserving swaps and chains without cloning payloads. Fingerprints remove
all old keys first and then install the executor's verified identities. The former
sequential decision/fingerprint loop could overwrite a later source during a swap;
a failing regression established lost ratings before this correction. Unchanged
items, in-place identity updates and per-namespace cohorts remain intact.


Recovery's library lock now has a scope guard that explicitly calls File::unlock
before its descriptor closes, on normal return and unwind. Closing just one File
can leave a lock held by a duplicated/inherited descriptor; this was reproduced on
macOS after a parallel recovery test failed immediate reacquisition. Explicit
unlock permits reacquisition while an unrelated duplicate still exists. The
regression also checks that an active owner remains exclusive and closing the old
duplicate cannot release a subsequently acquired lock. The exact fork timing of
the original parallel failure was not captured. See the standard-library
[File locking and unlocking contract](https://doc.rust-lang.org/std/fs/struct.File.html#method.unlock).


### Scene correction publication and editing (2026-10-04)

Visual Organize captures the session's immutable scene membership with each accepted
analysis request. The existing image/model owner forwards it to the similarity
policy worker. That worker resolves membership after automatic grouping and source
verification, before assigning Easy Delete candidate groups or publishing visual
results. The automatic map is retained separately so restore/undo does not treat
previous manual output as new AI evidence. Missing analysis inputs are excluded
from effective groups, even when saved membership exists. Exact content equality
continues to bypass the near-duplicate candidate-group gate.

Mixed date grouping and people/subject grouping retain their independent policies.
The scene UI explicitly explains this scope. General corrections can cross capture
dates in Visual mode, but cannot merge Mixed date buckets.

The application routes Scenes and Cull Groups through the same cancellable EditGroups
IO command, group-edit state machine, session snapshot and member policy. Organize
Visual uses shared library selection for Cmd/Ctrl-click, Shift ranges, select-all,
keyboard navigation and comparison. Scene edits support a new group, an existing
group (pinning its exact current cohort), restore selected/all and undo. Current
folder drafts block scene editing to protect their reviewed destinations. No edit
moves files or launches image analysis; Preview/Apply remains the file boundary.

Scene target choices are indexed on the worker and the popup virtualizes rows.
Group edits retain previous results on cancellation and ignore late completions.
They affect the next Easy Delete analysis without rewriting an existing review.


### Verified interrupted moves (2026-10-04)

On first recovery of an unsettled accepted journal, the shared filesystem worker
examines only actions with a complete pending receipt and no terminal receipt.
A Move is recognized as completed only when every reviewed source is absent and
every destination has the same saved Unix device/inode/birth identity, byte length
and modification timestamp. Paths must be absolute and free of symlink components.
Empty cohorts, ambiguous paths, changed files, copies with a different identity and
partial companion removals remain unresolved. No image decode, metadata extraction,
photo hashing, recursive inventory scan or photo mutation is added to restoration.

When evidence is sufficient, recovery streams the journal's complete-record prefix
into a new temporary journal, rechecks the old journal and move identities, appends
explicit recovered receipts, syncs, and atomically replaces the library's active
pointer. The old journal, including a torn tail, is retained unchanged. Receipt
replay migrates ratings, manual group membership and view bookmarks through the
existing report/session owner. Only uncompleted actions remain in the retry plan;
conditional empty-directory cleanup retains its completed prerequisites.

This inference is intentionally limited to first restoration before the journal
has been settled. Subsequent opens must not replay its historical context over new
saved decisions. Cross-device copies, partially removed companion cohorts, Trash,
metadata writes and case-only renames still require explicit resolution; this is
not a generic filesystem repair or content-equality heuristic. Cancellation while
verifying or copying discards the unpublished checkpoint and retains the old index.


## Exact regional early rejection (2026-10-04)

Visual Organize, Mixed date/scene grouping and Easy Delete all reach
`regional::cluster` through the existing similarity worker. For matching
multi-region descriptors, its neighbor predicate now accumulates the same
regional cosine values in the same order as `Features::distance`, but bounds
each unevaluated region by its maximum possible normalized dot product. If
that optimistic final score still exceeds the distance threshold, comparison
stops. Incomplete/unequal-region evidence retains its existing best-pair path.

The bound includes normalization, f64 accumulation and f32 rounding allowances;
optimistic additions/division/subtraction use the same monotone f32 operations
as the final score. No approximate-neighbor search, changed threshold or
changed evidence recipe is involved. Digests, stable group numbers, orientation
partitions, transitive membership, manual scene corrections, evidence loading
and cancellation ownership are unchanged. Diagnostics separately count
photo-pair tests and evaluated regional dots. Each resident descriptor adds
one f32 bound (plus struct padding); there is no new image/model/cache owner.
The descriptor working set stays at two blocks of 64. Diverse-library pair
enumeration and repeated block loading remain quadratic in the worst case.


## Durable destination amendments (2026-10-04)

All workflow retry dialogs share destination editing through the existing
filesystem worker and recovery Store. A reviewer selects a pending Move cohort
and supplies a folder relative to its existing output library. Filenames and
all companions are preserved. The dialog indexes eligible operation groups once
when the plan changes and renders only visible choices. It performs no scanning,
source reads, decoding or model work.

The worker takes the same library lock as execute/discard, verifies the current
journal ticket, requires settled recovery with no uncertain receipts, verifies
each cohort source fingerprint, and checks destination collisions, symlinks and
other pending targets. An already-copied same-identity destination or missing/
changed source is not silently reinterpreted. A revised complete header is
streamed to a new journal and the active pointer is replaced atomically only
after final source/journal checks. The old journal stays unchanged. Execution
and amendment share the same cancellable, size-bounded header writer.

Saving does not apply files or replay the old captured session: the new pointer
is already settled, so newer ratings, manual groups and bookmarks remain
authoritative. Retry captures current intent and requires the new ticket and
exact revised plan. Completed actions stay absent; conditional cleanup retains
its completed prerequisites. Cancellation observed before commit drops the new
temporary journal. Once publication succeeds, the UI reports the saved plan,
even if cancellation arrives afterward. Original source and overwrite checks
still run when the user explicitly retries.

This editor changes one pending Move cohort's destination folder per save. It
does not rename individual files, move to another output library, edit directory
rename/metadata/Trash actions, or resolve uncertain/partly applied moves. Those
capabilities remain separate recovery work, rather than weakening saved source
identity checks or silently rebuilding a plan from a fresh photo scan.


### Quitting with a durable unfinished plan

Native Quit and window close share the application departure owner. A retained
plan with a recovery ticket offers Save plan and quit, including when its review
was already open. The application captures the current session and shared view
bookmark, then queues one explicit-root Checkpoint on the existing filesystem
worker. Session and view files each use their existing atomic write; the UI waits
for both writes to succeed before authorizing close. A failure retains the review,
ticket and choices, with retry available. This is an acknowledged two-file save,
not a transaction spanning both files: a failed view save may leave the newer
session safely persisted, but never authorizes quitting.

Saving does not execute or replace the operation journal. On acknowledged exit,
shutdown drains the shared workers without redundantly rewriting the session.
Separate organization drafts, confirmed rotation choices, provisional reviews,
destination edits and active file operations retain their departure guards.
The new action does not imply general persistence for those unsaved drafts.


### Interrupted companion staging

Move stages every destination before unlinking any source. The shared recovery
owner classifies each member of an unreceipted Move on its first restoration:
completed (source absent, destination has the exact reviewed Unix identity),
staged (both paths still identify the reviewed file), or untouched (reviewed
source unchanged and destination absent). Unknown identities, replacements,
symlinks and cross-device copies prevent inference for the whole cohort.

A new checkpoint retains the old audit bytes and records member states. Completed
members contribute identity migrations and cleanup dependencies; untouched
members remain ordinary moves. Staged members become individual FinishMove
actions, each with one normal pending/completed receipt. Restoring the library
never unlinks a source. Explicit Retry rechecks both paths and the complete
fingerprint before removing the remaining source link, keeping the destination
and preserving file bytes and identity. Another interrupted retry reuses the
same recovery path. Ambiguous states and settled failed operations retain their
existing explicit review behavior.

FinishMove shares report/session/view migration, cache invalidation, the
filesystem worker, and the virtualized review renderer with other file actions.
Its already-staged destination cannot be edited. The multi-member representation
shares move inspection, but execution requires exactly one member so each source
removal has a separate durable receipt.

## Shared inference providers

`processing::Control` snapshots thread allocation and backend together at job
admission. The image owner passes that immutable configuration and the accepted
run's cancellation/generation check to `Models::begin_run`. All shared inference
calls, including calls without a local cancellation argument, observe it before
loading a model and before a provider fallback. `models::execution` constructs
platform providers and owns explicit CPU fallback. The registry retains one
session per model, reuses input tensors for execution retry, and releases failed
or obsolete sessions before allocating replacements. Backend changes preserve
photo/evidence caches. Preparation and per-session provider/fallback status are
published through short control locks; native work never holds a UI-shared lock.

The Core ML DINO session specializes symbolic axes to one image. The existing
fixed-one crop fallback reuses it; other providers keep dynamic batches. The
standalone provider profiling source under `docs/diagnostics` duplicates only
session construction for controlled measurement. Application workflows never
construct a separate pipeline or model registry. See `ACCELERATION.md` for cold
compilation, cancellation and platform-validation boundaries.

## Image-engine reassessment — 2026-10-05

An isolated libvips/Little CMS comparison supports prototyping a mature native
image engine behind the current shared pipeline. The current production backend
is unchanged; the exploratory CMYK TIFF adapter was set aside. The measured
preview benefits, single-thread PNG regression, color differences, cancellation
result and Rust binding error-handling concern are recorded in
[the evaluation](diagnostics/imaging-evaluation-2026-10-05/README.md).
Integration must preserve shared ownership, neutral analysis, visible color
fallbacks, cancellation and packaging guarantees before replacing the backend.


### Native image engine integration (opt-in)

`native-imaging` connects `crates/imaging-vips` to the existing `core::pipeline`
owner. JPEG, PNG, TIFF and GIF use libvips/Little CMS; RAW, HEIF, video and other
raster formats keep their established implementations. There are no additional
workflow pipelines, scanners, metadata stores or application workers.

Browsing and analysis reuse one bounded 2048-edge working image; original pixels
are decoded on demand. Working and original recipes have separate entries in the
same bounded LRU, so analysis does not depend on whether original detail was viewed
first. Small full-precision images may share an entry. Oversized originals do not
evict useful working images merely because they cannot fit the cache. Analysis
sizes derive from source geometry rather than the rounded intermediate dimensions.

Native callbacks use thread-safe cancellation. Original inspection now checks the
current epoch and requested view during preparation as well as before publishing.
Render warnings and source geometry travel with the pixels and their disk record;
a proxy without its record is rebuilt. Warnings reach the shared inspector and
review canvas in all five workflows. Cache recipes include native backend/library
versions and invalidate dependent analysis evidence. The previous Rust color-manager
counters remain distinct from the new native-render counter.

`scripts/build_vips.py` builds a minimal pinned dependency in `.tools/vips` with
retained licenses/provenance and disposable build tools. The feature remains
opt-in; the default packaged application is unchanged. A separate macOS package
now passes native launch, original detail, visible warnings across restart, active
analysis cancellation and subsequent navigation. Packaging resolves transitive
loader paths, audits bundled dependencies and retains upstream notices. Combined
workload, supported-platform distribution and default-selection gates remain open. The
application does not yet use the native region API: original views still use the
full-source LRU. See the crate README and the recorded color-engine comparison for
validation boundaries and the explicit CMYK rendering tradeoff.

Opaque RGBA8 proxies are compacted to RGB before cache encoding, preserving exact
reopened RGBA pixels. Alpha-bearing and 16-bit images retain their representation.
Cancellation checks bound the opacity scan; all five steps share this boundary.

The private native builder enables Highway SIMD by default and records the exact
codec versions/options in a SHA-256 build recipe exposed through pkg-config.
The Rust image cache identity includes this recipe, target architecture and
library versions. A same-version native rebuild can therefore invalidate pixels
and dependent analysis without affecting filesystem review state. External
packages without the private recipe retain version/target fallback identity.

The [controlled SIMD workload](diagnostics/proxy-workload-2026-10-05/README.md)
resolves the earlier scalar browsing regression: first browsing is 15% faster,
combined pipeline time 29% shorter, and whole-process peak RSS 3% lower than the
default in that run. Original inspection still uses full-source decoding; do not
claim region-based memory savings or broad photographic accuracy from this result.

The final SIMD macOS package passes native transparent/warning rendering, JPEG
original detail, active analysis cancellation, post-cancellation navigation and
restart checks. Evidence: `docs/diagnostics/simd-native-validation-2026-10-05.json`
(repository-relative path). The default app bundle remains separate.


## Shared desktop path actions — 6 October 2026

Library Grid/List/Icons and Organize file/folder context menus share the
`ui::desktop::locations_menu` component and existing `core::desktop::Worker`.
The `opener` crate owns default-app and file-manager integration. Clipboard
actions use egui's platform output. Captured absolute original paths are verified
on the bounded desktop worker; neither menu creation nor dispatch scans, decodes
images or initializes models. App generations reject obsolete completions.

Organize resolves draft folders back to their unique catalog source path; new or
ambiguous folders cannot dispatch an original-path action. Proposed destinations
reuse the move planner's folder construction and shared companion filename rule.
Collision suffixes remain the responsibility of Preview moves. Source paths and
proposed paths are distinct values; right-click actions preserve selection and
staged decisions. Desktop failures are also shown inside the open editor.


## Terminal ownership — 6 October 2026

`core::desktop::terminal` is a small adapter under the existing desktop worker.
It uses native launch facilities, not a terminal emulator or shell-script engine.
The shared request verifier resolves files to parent directories only on the
worker. Original requests remain attached to completions and generation guards.
Library and Organize menu composition is unchanged beyond adding the action.

macOS waits only for the LaunchServices helper to return. On Linux/Windows, the
same service retains native child handles; `try_wait` removes finished children
without waiting or allocating a reaper thread per window. Lock contention defers
cleanup to the next maintenance frame. App shutdown releases handles without
killing the user's terminal. Linux delegates preferred-terminal selection to
`xdg-terminal-exec`, including its desktop-entry validation. No new crate or
workflow-local worker/cache was introduced.


## Bounded metadata recency — 6 October 2026

The single imaging actor owns a `lru::LruCache<String, Metadata>` with a
100,000-entry capacity. Metadata indexing, image preparation and both inspection
paths insert into this same cache; metadata reads refresh recency. A new entry
evicts the least recently used entry instead of clearing the whole map. The
`sparse` constructor starts without reserving the full capacity, and central
mutation invalidation still clears all affected in-memory metadata. Persistent
metadata records, source fingerprints and stale/cancellation gates are unchanged.

The dependency uses the standard-library hash map with its optional hashbrown
feature disabled; no additional transitive dependency was introduced. This
bounds the number of metadata entries, not their bytes or the entire app's memory.
A sequential traversal larger than the capacity can still thrash any LRU cache.
The improvement is retaining recent metadata when browsing across the capacity
boundary, not eliminating all rereads of arbitrarily large libraries.

The shared-boundary regression fills the actual production capacity, refreshes
one real photo, loads another, and verifies no disk reread or decode for the
recent photo. The former implementation fails that same test.


### Demand-based RAW detail and shared camera previews

RAW display and analysis derive their respective 2048/1024-edge proxies from one
prepared camera preview in the existing bounded source cache. Original inspection
has a separate entry and always uses the full RAW rendering recipe. Camera JPEG
color/crop can differ from that rendering; explicit 100% viewing remains available.
The source key includes `raw-shared-preview-v1`, invalidating dependent RAW
artifacts while leaving other formats' keys unchanged.

LibRaw provides both the embedded JPEG and full-source geometry. Its
`libraw_adjust_sizes_info_only` advances decoder state: embedded extraction happens
first, and a missing/corrupt preview uses a separate short-lived header-only handle
for geometry before the primary handle performs a neutral half-size sensor decode.
This intentional second header parse avoids an invalid LibRaw call sequence; it
does not create another application pipeline or repeat model work. A failure to
obtain geometry falls back to full-size neutral decoding.

The shared review canvas compares available preview pixels with the physical
pixels required by the current viewport, zoom, rotation and display scale. It
requests original detail only for insufficient preview resolution or explicit
100% inspection. The existing 300 ms idle delay still applies to fit views that
need more detail. Adequate previews no longer wake the renderer every 300 ms.
Cancellation, source/sidecar invalidation and bounded cache ownership remain in
the shared pipeline and worker. Regression coverage exercises all five workflow
canvases and independent RAW copies with missing/corrupt previews, orientation
changes, both request orders, reopened caches and cancellation.

RAW metadata-only requests also recover source geometry from the existing display
or analysis render descriptor. This matters when persisted model evidence avoids
pixel preparation after restart: the UI must retain the same source dimensions
as the first analysis run. The EXIF snapshot stays unchanged, and descriptor
lookup follows the same source/sidecar identity and cache-budget policy. An
analysis-only/reopened-metadata regression requires exact metadata equality, zero
pixel decodes, zero source metadata reads and empty pixel caches.

### Reduced macOS HEIF working images

The shared pipeline requests at most a 2048-pixel long edge from Apple's ImageIO
thumbnail API for HEIC/HEIF browsing and analysis. It asks ImageIO to derive that
image from the source, rather than selecting an embedded thumbnail. Orientation
is disabled at that API boundary and remains owned by the existing shared
EXIF/sidecar transform. Full source geometry travels separately from reduced
pixel dimensions; metadata-only cache reopening restores it from the existing
render descriptor without decoding pixels or overwriting the raw EXIF snapshot.

A reduced working image and full-resolution original use separate entries in the
same bounded prepared-source cache. Analysis therefore does not depend on whether
original detail was requested first. Small, already-full-resolution RGBA images
can reuse one entry. Display and neutral analysis share the working decode;
explicit original inspection retains the full ImageIO decode. This reduces
application-visible working buffers, not a guaranteed bound on codec-internal or
whole-process memory. The portable libheif path retains its existing pixel policy.

`core::apple_image` is the single safe Rust boundary for ImageIO decoding and
orientation reads. One C header is shared by the Objective-C implementation and
fixture tools. Calls have local autorelease pools, explicit CF/CG ownership,
checked geometry, and malloc-owned straight-alpha sRGB buffers copied into Rust
ownership. No Cocoa objects escape into the photo worker. Cancellation is checked
around native decoding and shared preparation; a running ImageIO call itself is
not interrupted. HEIF-only recipe versioning rejects old previews and derived
analysis while preserving unrelated format caches.

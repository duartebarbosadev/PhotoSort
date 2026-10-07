# Functional reference audit

Reference: ../PhotoSort, inspected 27 September 2026. The Python checkout is read-only.

## Execution paths

`main.py` builds shared services, `MainWindow` and `AppController`. Folder changes
flow through `AppController.load_folder`, cancellable `FileScanner`, batched
`AppState` records and the shared thumbnail coordinator. Workflow widgets select
configuration and launch workers through the controller/worker manager. Workers
consume `ImagePipeline`, metadata/analysis caches and shared similarity services.
File moves, deletion and rating/rotation writes invalidate caches and update the
central path index, selections and workflow results.

| Step | Reference behavior and owners |
| --- | --- |
| 1 Organize | `grouping_step_widget.py` → `AppController` → `GroupingPreviewWorker` / `GroupingWorkflowWorker` → `grouping.py`. Current folders, similarity, faces, offline GPS location and mixed grouping. Editable group labels and file names; preview before applying. Collision-safe moves, optional RAW/JPEG/XMP companions, directory cleanup and partial-failure reports. |
| 2 Easy Delete | `easy_delete_step_widget.py` → `EasyDeleteWorker`. Local-tile Laplacian sharpness, near-black/white exposure, exact-byte and visually verified near-duplicates. Suggestions remain reversible deletion marks until review. |
| 3 Fix Rotation | `fix_rotation_step_widget.py` → detection/application workers → shared orientation model / `ImageRotator`. Original/corrected comparison, manual quarter turns, local ONNX orientation inference. RAW/HEIF metadata rotation; some raster formats rewrite pixels. |
| 4 Pick Best | `pick_best_step_widget.py` → `PickBestWorker` → similarity and `best_photo_finder`. DINOv2 clustering, relative-sharpness gate, BeIT aesthetic score, MediaPipe face/open-eye penalties, manual overrides and keep-all. |
| 5 Cull | main window/review components and shared controllers. Browsing, multi-photo comparison, synchronized zoom/pan, ratings, deletion marks, folder/search/rating filters, metadata inspector, similarity groups, video playback. |

Steps are numbered but optional and revisitable. Marks, metadata, active photo,
selection and cached work must survive navigation. Ratings are 0–5; videos are
browse/play only. Scan inventory includes many RAW extensions plus JPEG/PNG/TIFF,
HEIF and common videos; the README also promises WebP. Scan must not trigger AI.

## Data and expensive work

Python uses mutable path-keyed records in AppState, filesystem fingerprints,
workflow result maps and persistent diskcache caches. Ratings/EXIF use serialized
metadata access. The canonical RAW rendering path is `RawImageProcessor.render_display`
→ `ImagePipeline.ensure_review_assets_cached`; thumbnails derive from the same
render as previews. There are generation locks and separate high-memory decode
limits. Preview appearance must not change when resolution improves.

Analysis is local. DINOv2 small/base uses pinned Hugging Face weights; aesthetic
scoring uses cafeai/cafe_aesthetic; orientation uses a four-class ONNX model
(0, 90, 180, -90), RGB resize to 416, center crop to 384, ImageNet normalization.
No cloud photo APIs. Heavy model initialization is lazy and shared.

## Dependencies being replaced

PyQt6, Pillow/pillow-heif/rawpy, NumPy/OpenCV, scikit-learn, diskcache,
pyexiv2/piexif, send2trash, reverse-geocode, torch/torchvision/transformers,
Hugging Face and ONNX Runtime. Qt object/thread ownership, Python dictionaries,
unbounded per-workflow state and Python packaging are not migration targets.

## Compatibility decisions

* New application data/cache namespace; never read or mutate Python cache files.
* Typed catalog and immutable worker results replace QObject/controller graphs.
* One image worker owns canonical decoding and a byte-bounded cache. All five
  steps use it; requesting another step cannot create another decoder/cache.
* Non-destructive XMP sidecars are preferable to lossy pixel rewriting. Existing
  sidecars must retain unrelated metadata, and compatibility differences must be
  explicit. Original pixels are preserved.
* File writes are staged in a review plan and executed by a single mutation owner.
  OS Trash is required; permanent deletion is not an alternative.
* Do not present heuristic grouping/ranking as DINOv2/BeIT/face model equivalence.
  Native model parity requires separate real-model and real-camera validation.


## Organize context-action audit — 6 October 2026

The reference `grouping_step_widget.py` context menu also provides Open, Reveal,
Copy full path, Copy relative path, Copy projected destination and Open terminal
here, alongside its existing organization/trash actions. File-relative paths
refer to the original source; folder-relative paths describe the displayed draft
folder. Its existing-path resolver falls back to the library root for proposed
folders that do not exist. The Rust editor instead disables Open/Reveal when a
draft folder has no unique original directory, keeping proposed paths available
for copying. The Rust terminal shortcut now shares the desktop worker; its platform behavior is recorded in COMPATIBILITY.md.


## HEIF rotation claim correction — 6 October 2026

The initial overview describes the reference's intended HEIF metadata rotation.
A later complete source trace and dependency probe show that its installed
`piexif.insert` accepts JPEG/WebP and rejects HEIF. The shared rotation worker
therefore reports a failed HEIF rotation when it reaches this path. Do not treat
embedded HEIF rotation as a demonstrated working reference capability. Rust's
sidecar rotation is an intentional working alternative; embedded container edits
would improve interoperability beyond this broken reference path. Evidence:
`diagnostics/heif-write-feasibility-2026-10-06/README.md`.

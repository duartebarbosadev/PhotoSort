# Parity with the Python PhotoSort

Reference: the sibling `PhotoSort` checkout at commit `7c62143` (read-only).
The target is the same workflows with native design and better performance, not a
pixel- or number-identical copy. Per-row history, with the evidence behind each
claim, is in [history/PARITY-DETAIL.md](history/PARITY-DETAIL.md) and
[history/COMPLETION_AUDIT.md](history/COMPLETION_AUDIT.md). Menu-by-menu mapping is
in [INTERACTION_AUDIT.md](INTERACTION_AUDIT.md); deliberate design differences are
in [COMPATIBILITY.md](COMPATIBILITY.md).

Status: **Done** = implemented and covered by automated tests; **Redesigned** = same
goal, different mechanism; **Open** = not done.

## Workflows

| Python feature | Status | Notes |
| --- | --- | --- |
| Organize: Current, Mixed, Visual, Faces, Location grouping | Done | Plus Date and Camera. Alt+1–5 select modes. |
| Organize: editable folders and names, preview, review before apply | Done | Create/rename/nest/drag; RAW/JPEG/XMP companions move together. |
| Organize: compare several selected photos | Done | Up to 9 (Python: 12). |
| Easy Delete: blur, dark, bright, exact and near duplicates | Done | Thresholds in Settings; suggestions stay reversible until review. |
| Fix Rotation: detect, compare, manual turns, apply | Done | Orientation goes into embedded metadata or an XMP sidecar. An opt-in setting ("Embed, or rotate pixels") rotates the pixels instead when a file cannot store orientation itself (BMP in the full build; PNG/TIFF/BMP in the minimal build). |
| Pick Best: clusters, sharpness, aesthetic score, eyes, keep-all | Done | Progressive comparisons, previous-round revision. |
| Cull: grid/list/icons, search, rating/folder/date/group filters, ordering | Done | Virtualized; filters intersect. |
| Cull: side-by-side comparison, synchronized zoom/pan | Done | Fit, 1:1, numbered focus. |
| Ratings, rotations, Trash marks | Done | One staged decision path; embedded EXIF/XMP writes verified, or sidecar. |
| Details sidebar (single photo) | Done | Source, lens, focal length, aperture, shutter, ISO, flash, dimensions, megapixels, orientation, bit depth, colour space, white balance, metering, exposure mode/compensation, scene, software. |
| Details sidebar (compare mode) | Done | One column per photo; differing rows highlighted. |
| Video browse and playback | Done | macOS verified; Windows/Linux unverified. |
| Capture-date ordering shared by all lists | Done | EXIF/XMP, then movie header, then filename, then file time. |
| Per-folder, per-workflow resume | Done | Filters, selection, focus, scroll, comparison. Python caches are not imported. |
| Recent folders, Settings, cache management, shortcuts guide, About | Done | Panel layout differs. |
| Update check | Redesigned | Client implemented; **no Rust release feed exists yet**. |
| Intro video | Redesigned | Replaced by welcome screen, step descriptions and keyboard guide. |
| Ctrl+Shift+L workflow-panel toggle | Redesigned | Panels are replaced by the filmstrip/choosers, so there is nothing to hide. |

## Beyond the Python app

Staged, reviewable file plans with durable crash recovery; no-overwrite moves;
OS Trash only; ICC colour normalization shared by thumbnail, preview and analysis;
persistent measurements so reopening an unchanged library skips decoding and model
loading; metadata indexing that never decodes images; Core ML/CUDA/DirectML
execution with per-model CPU fallback; session logging and `--diagnostics` /
`--check-runtime` commands.

## Open items

1. **Distribution:** three prepared model exports (aesthetic, detector, landmarks)
   have no public download source; no release feed, Developer ID signing or
   notarization. Publishing is a separate decision from building locally.
2. **Platforms:** Windows and Linux desktop behaviour, CUDA and DirectML on real
   hardware, and audible video sync are unverified. CPU fallback is the only
   evidence for the accelerators.
3. **Accuracy:** grouping, face, keeper-ranking and rotation checks use 186 distinct
   photos. Broader labelled sets are needed before claiming accuracy.
4. **First-run memory:** the first Quality analysis on a machine peaks around
   3–4.6 GiB while ONNX Runtime optimizes the 378 MB aesthetic model once; later runs
   reuse the prepared graph. Avoiding it entirely means shipping a pre-optimized
   model (an asset/provenance decision). Measurements on a real 517-photo library are
   in [BENCHMARKS.md](BENCHMARKS.md).

## Reference corrections

The Python app forces DINOv2-small and ignores legacy model overrides despite its
README, omits WebP from its scanner, and its HEIF embedded-rotation path fails.
These are not gaps in the Rust version.

# Architecture

PhotoSort Rust is one process with a thin immediate-mode UI over a set of
long-lived worker threads. `src/core` has no UI dependency; `src/ui` never
decodes images, reads metadata or touches the filesystem on the frame thread.

This file is the current map of the application.

## Why egui

egui/eframe on wgpu gives native GPU textures, a custom comparison canvas and
direct control over row virtualization and repaint scheduling. The app requests
repaints for input and worker results rather than running a permanent 60 Hz loop.
Layouts are point-based for high-DPI screens and switch from a sidebar to a compact
stepper below 1000 points. A webview would add a second runtime without helping
decoding or the review canvas; Iced and Slint were considered and are viable but
offered no advantage for a custom photo canvas.

## Layers

```
src/main.rs, lib.rs   CLI flags, diagnostics, session logging
src/ui/               egui screens; owns only presentation state
src/core/             everything else; no egui types
crates/imaging-vips/  optional libvips/Little CMS adapter (feature native-imaging)
native/               exiv2 metadata bridge (C++)
```

### Core modules

| Area | Modules | Responsibility |
| --- | --- | --- |
| Identity | `model`, `catalog`, `dates` | `Media`, fingerprints, one recursive inventory shared by all workflows |
| Pixels | `pipeline`, `pixels`, `color`, `inspection`, `heif`, `apple_image` | One image owner: thumbnails, previews, original-detail tiles, neutral analysis inputs, ICC normalization, bounded caches |
| Metadata | `metadata`, `details`, `cache_record` | EXIF/XMP/movie headers read once; Details panel formatting; versioned cache envelope |
| Analysis | `analysis`, `models`, `processing`, `subjects`, `regional`, `near_duplicates`, `faces`, `face_groups`, `rotation` | Technical measurements, ONNX model sessions with CPU/Core ML/CUDA/DirectML, grouping, duplicates, orientation |
| Review | `delete_review`, `pick_review`, `decisions`, `selection`, `manual_groups`, `views` | Staged user choices; nothing touches files until a plan is applied |
| Files | `operations`, `organize`, `desktop` | Reviewable plans, no-overwrite moves, companion files, OS Trash, crash recovery |
| Browsing | `browsing`, `navigation`, `geography` | Filter/sort/group index on its own worker; folder, date and location trees |
| Runtime | `jobs`, `cache`, `session`, `preferences`, `model_assets`, `updates`, `playback`, `startup` | Worker threads, persistence, model downloads, update check, video |

### Threads

`core::jobs::Runtime` owns four workers plus a few single-purpose services.
Messages are bounded and carry a folder **epoch**; results from an older epoch are
dropped, and long jobs poll a cancellation closure between units of work.

| Thread | Owns |
| --- | --- |
| `photosort-files` | scan, session/view/preference saves, file-plan building and application |
| `photosort-images` | the `Pipeline`, model sessions, metadata indexing, thumbnails, previews, analysis |
| `photosort-subjects`, `photosort-similarity` | grouping policy over stored evidence, so slow clustering never blocks decoding |
| browsing worker | filter/sort/group index for Cull |
| desktop, updates, model-assets, playback | OS handoff, release check, downloads, video decode |

The UI drains at most 12 events per frame (`ui/events.rs`), uploads ready pixels
as textures, and schedules repaints only while work is outstanding.

## Resource rules

- No folder-wide decoded image collection; CPU images, GPU textures and queues are
  bounded separately.
- Thumbnail and preview come from one oriented proxy; oversized images are rejected
  through decoder allocation limits.
- Work is cancelled between units and old-generation results discarded after a
  folder switch. A native codec or inference call already in progress may finish.
- Persisted records are versioned (`cache_record`); changing a stored struct means
  changing its recipe key so old records are recomputed, never misread.

## Safety rules for files

Every change is staged, then shown as a plan before anything is touched. Moves never
overwrite, check fingerprints immediately before acting, keep RAW/JPEG/XMP companions
together and write a recovery journal first. Deletion goes to the OS Trash; there is
no permanent-delete path. Metadata edits are written to a private copy, verified and
atomically swapped in.

## Verification policy

Tests use generated images and temporary directories and never mutate reference sample
libraries (`--ignored` tests that read real samples are opt-in). The expected checks are
`cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test
--locked`, each also with `--no-default-features`. Runtime and platform checks were
run and recorded during development.

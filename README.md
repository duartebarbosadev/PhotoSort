# PhotoSort

A local-first desktop app for culling and organizing photo libraries.

[![Rust checks](https://github.com/duartebarbosadev/PhotoSort/actions/workflows/check.yml/badge.svg)](https://github.com/duartebarbosadev/PhotoSort/actions/workflows/check.yml)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

> This is the native rewrite of PhotoSort. The previous Python app is available at tag
> [`python-final`](https://github.com/duartebarbosadev/PhotoSort/tree/python-final) and in
> [releases](https://github.com/duartebarbosadev/PhotoSort/releases) up to v2.1.0.

## What it is

PhotoSort takes a folder of new photos to a sorted, culled library. It is a native app
written in Rust with egui/eframe and wgpu. All analysis runs on your computer with local
AI models, and no photo is uploaded. Files change on disk only after you review a plan of
the exact moves, renames and edits; try it on a copy of a folder first.

## Features

### Five steps

The steps sit in a bar at the bottom center of the window. Each one is optional and can be
revisited, Settings can hide the three middle steps, and decisions are saved per folder.

| Step | What it does |
| --- | --- |
| **1. Organize** | Proposes folders from current folders, capture date, camera, offline location, visual similarity, faces, or date then similarity. On the folder board, drag photos onto folders and create or rename folders in place. Moves are previewed and collision-checked, and RAW/JPEG/XMP companions move together. |
| **2. Easy Delete** | Reviews exact duplicates, near-duplicates, and blurry, dark or bright photos as paired Keep/Trash suggestions. Confirm, revoke or reset each one, or accept the defaults in bulk. |
| **3. Fix Rotation** | A local model suggests quarter turns, cross-checked against rotated copies of each photo; uncertain ones need individual confirmation. Compare the original and corrected views, rotate manually, and apply only confirmed corrections. |
| **4. Pick Best** | Two-photo comparisons within groups of shots of the same subject. Suggestions combine an aesthetic score, sharpness and a closed-eye penalty, and choices never change star ratings. |
| **5. Cull** | List, icon and grid views, and up to nine photos side by side with synchronized or independent zoom down to native pixels. Rate, mark for Trash, filter and sort. The library navigator browses folders, dates or similarity groups, and the Groups menu corrects groups with Undo. The filmstrip sits at the bottom or on the left, with optional file names. **Analyze library** (More menu or Ctrl/Cmd+S) builds similarity groups. Here and in Organize, right-click marks every photo in a comparison at once. |

### Across every step

- **Reviewed file plans.** Moves, renames, Trash and metadata edits are listed before they
  run. If the app stops mid-plan, reopen the library and use **Review changes** to retry
  unfinished operations; completed ones are not repeated.
- **Local AI.** Models run through ONNX Runtime on your computer, and models are downloaded
  only when you ask.
- **Metadata write modes.** Ratings and rotation are embedded where the format supports it,
  or written to XMP sidecars only. Embedded edits are verified on a temporary copy and
  never re-encode pixels. An optional mode losslessly rotates the pixels of files that
  cannot store orientation, such as BMP.
- **Color management.** Embedded ICC profiles (RGB, grayscale and CMYK JPEG) are converted
  to sRGB once, so thumbnails, previews, detail views and analysis agree.
- **Reveal in file manager.** Any file can be shown in Finder, Explorer or the system file
  manager, and videos can open in the default player.

### Supported formats

| Kind | Formats |
| --- | --- |
| Photos | JPEG, PNG, TIFF, GIF, WebP, BMP |
| RAW | ARW, CR2, CR3, NEF, NRW, DNG, ORF, RAF, RW2, PEF, SRW and other LibRaw formats |
| HEIF | HEIC, HEIF |
| Video | MOV, MP4, M4V; also AVI, MKV, MPEG, WMV and WebM |

RAW and HEIF need the default build; RAW browsing uses the camera's embedded preview until
you zoom in. Video plays through AVFoundation on macOS and GStreamer on Linux and Windows,
so codec support follows the system.

## Platform status

| Platform | Status |
| --- | --- |
| macOS 13+ | Full build, CI-tested with both image backends |
| Linux | Full build and tests CI-tested in a Debian container; desktop use not yet verified |
| Windows | Compiles and passes tests in CI; the full native build and desktop use are not yet verified |

Releases of the native app are not yet published; build from source or package a macOS app.

## Building from source

You need Rust 1.95 or later, a C/C++ compiler and pkg-config. Default builds also need
LibRaw 0.21+ and Exiv2 0.28+, and on Linux and Windows libheif 1.23+ with an HEVC decoder
plugin and GStreamer 1.20+. On macOS, HEIF and video use system frameworks.

**macOS** (with the Xcode command line tools):

```sh
brew install libraw exiv2 pkg-config
```

**Linux** (Debian 13 "trixie", as in CI; other distributions may ship older libheif or
Exiv2 versions):

```sh
sudo apt-get install build-essential pkg-config \
  libraw-dev libexiv2-dev libheif-dev libheif-plugin-libde265 \
  libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
  gstreamer1.0-plugins-base gstreamer1.0-plugins-good gstreamer1.0-libav \
  libxkbcommon-dev libwayland-dev libx11-dev libxi-dev libxcursor-dev \
  libxrandr-dev libxinerama-dev libegl1-mesa-dev
```

**Windows** (untested): install Exiv2 0.28+, LibRaw, libheif 1.23+ with an HEVC decoder
plugin, and the official GStreamer MSVC runtime and development packages, all
discoverable through pkg-config.

Build and open a folder:

```sh
cargo run --release -- --folder /path/to/photos
```

## AI models and ONNX Runtime

| Model | Purpose | Used by | In-app download |
| --- | --- | --- | --- |
| `dinov2-small.onnx` (DINOv2 small) | Scene similarity | Organize (visual, date + similarity), Easy Delete, Pick Best, Cull | Yes |
| `aesthetic.onnx` (cafe_aesthetic, BEiT) | Aesthetic score | Pick Best | No |
| `face_detector.onnx` (MediaPipe) | Face detection | Organize (faces), Easy Delete, Pick Best, Cull | No |
| `face_landmarks_detector.onnx` (MediaPipe Face Mesh V2) | Eye positions and alignment | Organize (faces), Easy Delete, Pick Best, Cull | No |
| `face_blendshapes.onnx` (MediaPipe) | Eye openness | Easy Delete, Pick Best, Cull | No |
| `face_recognition_sface.onnx` (OpenCV Zoo SFace) | Same person across photos | Organize (faces) | Yes |
| `orientation.onnx` (deep-image-orientation-detection v2) | Sideways and upside-down photos | Fix Rotation | Yes |

Model weights are not in git; `resources/SHA256SUMS` pins their exact content. **Menu → AI
models** checks installed models and restores missing or damaged ones from the app bundle
or a prepared ONNX file. DINOv2, SFace and the orientation model can also be downloaded
there from pinned upstream files, verified by size and SHA-256. The other four are
converted exports with no public download: they ship in an app bundle packaged with them in
`resources/models`, or can be prepared with the recipes in
[resources/ASSETS.md](resources/ASSETS.md). Manual review works without any model.

ONNX Runtime is loaded at run time. It is not in git and the model manager does not
download it. PhotoSort looks for it in this order:

1. The path in `ORT_DYLIB_PATH`.
2. `runtime/libonnxruntime.dylib` (`onnxruntime.dll` on Windows, `libonnxruntime.so` on
   Linux) in the app's data folder, in `./resources`, or in the app bundle's resources.
3. `/opt/homebrew/lib/libonnxruntime.dylib` and `/usr/local/lib/libonnxruntime.so`.

The bundled runtime is ONNX Runtime 1.29.0. `photosort --diagnostics` shows which files
were found, and `photosort --check-runtime` loads the runtime and runs every model.

Settings selects the inference backend: CPU (the default), Automatic, Apple Core ML, NVIDIA
CUDA or Windows DirectML. A backend that fails falls back to CPU with an explanation. Core
ML can take minutes to prepare on first use; CUDA and DirectML are not verified on real
hardware.

## Command-line usage

Usage: `photosort [--folder PATH | --last-folder] [options]` or `photosort [PATH]`.

| Option | Description |
| --- | --- |
| `--folder PATH`, `PATH` | Open a photo folder. |
| `--last-folder` | Open the most recent folder that still exists. |
| `--clear-cache` | Clear regenerable previews and analysis evidence before startup; reviews and decisions are kept. |
| `--clear-models` | Remove managed model copies before startup; bundled models are kept. |
| `--diagnostics` | Print JSON file and readiness diagnostics and exit without a window; cannot be combined with folder or clearing options. |
| `--check-runtime` | Run native inference checks, print JSON and exit without a window. |
| `--runtime-backend NAME` | `cpu` (default), `auto`, `coreml`, `cuda` or `directml`; requires `--check-runtime`. A pass includes CPU fallback, so check `model_providers` in the JSON. |
| `--smoke-test` | Request a normal, guarded quit after the first UI frame and a delay; unapplied choices can keep the app open. |
| `--smoke-delay-ms N` | Delay of 200–60000 ms (default 2000); requires `--smoke-test`. |
| `--log-file PATH` | Write optional session diagnostics to a new file (16 MiB cap); logs may contain local paths. |
| `--log-level LEVEL` | `error`, `warn`, `info` (default), `debug` or `trace`; requires `--log-file`. |
| `-h`, `--help` | Show help. |
| `-V`, `--version` | Show the version. |

From source, pass options after `--`, for example `cargo run --release -- --diagnostics`.

## Keyboard shortcuts

| Keys | Action |
| --- | --- |
| Ctrl/Cmd+O | Open a library |
| Ctrl/Cmd+F | Focus library search (except in Pick Best) |
| Ctrl/Cmd+S | Analyze this step; detect rotations in Fix Rotation |
| Ctrl/Cmd+Alt+1–5 | Switch to a workflow step |
| Shift+Enter | Review staged changes |
| Alt+1–5 (Organize) | Group by current folders, date + similarity, visual, faces or location |
| Alt+1/2/3 (Cull) | List, icons or grid |
| F / S (Cull) | Folder or similarity-group navigator |
| Left / Right (Cull) | Cycle through visible photos or the current similarity group |
| Ctrl/Cmd+0–5 | Rate the selection |
| D (Cull) | Toggle the Trash mark |
| R / Shift+R / Alt+R (Cull) | Rotate right, left or 180 degrees |

Each step shows its own keys in the status bar, and **Menu → Keyboard guide** has the full
list.

## macOS app bundle

```sh
python3 scripts/package_macos.py
```

```sh
open target/PhotoSort.app
```

The packager builds a release binary and copies it, `resources/` (including any models and
ONNX Runtime there) and the linked native libraries with their license notices into a
relocatable bundle, then verifies its signature and resource manifest.
`--output /path/to/PhotoSort.app` chooses another destination, and `CARGO_TARGET_DIR` is
honored (see [CONTRIBUTING.md](CONTRIBUTING.md#build-storage) for a temporary build
directory).

Bundles are ad-hoc signed and not notarized by default, so macOS may ask again for folder
access after a rebuild. `--sign-identity` takes an existing codesign identity name or SHA-1
for a consistent signature.

## Optional libvips backend

The `native-imaging` feature decodes JPEG, PNG, TIFF and GIF through libvips and Little CMS
instead of the default Rust decoders. It is opt-in, faster for JPEG, PNG and TIFF, and
required for ICC-profiled CMYK TIFF. `scripts/build_vips.py` builds a pinned private copy
into `.tools/vips`, and the packager's `--native-imaging` option bundles it
(`--native-prefix` selects another location). To run from source, set
`PKG_CONFIG_PATH="$PWD/.tools/vips/lib/pkgconfig"` and pass `--features native-imaging` to
cargo. See [crates/imaging-vips/README.md](crates/imaging-vips/README.md) for details.

```sh
brew install glib jpeg-turbo libpng libtiff little-cms2 highway
```

```sh
python3 scripts/build_vips.py
```

```sh
python3 scripts/package_macos.py --native-imaging
```

## Updates (for packagers)

Menu → Check for updates reads the GitHub releases of the repository set with
`PHOTOSORT_RELEASE_REPOSITORY=owner/repository` at build time. Builds with it also check
once a day at startup unless that is turned off in Settings; builds without it make no
update requests. Release tags must be semantic versions such as `3.0.1` or `v3.0.1`, and
drafts, prereleases and downgrades are ignored. The app links to the release page and
never downloads or installs an update.

## Project layout

```text
src/core/             catalog, image pipeline, metadata, models, file operations
src/ui/               egui interface and the five steps
crates/imaging-vips/  optional libvips adapter (native-imaging feature)
native/               ImageIO, AVFoundation, LibRaw, Exiv2 and libheif bindings
resources/            fonts, location data, license notices, model checksums
scripts/              macOS packager, libvips build, model export, test oracles
examples/             developer audits and benchmarks
tests/                integration tests
tests/fixtures/       small generated fixtures
tests/samples/        seven camera samples for opt-in reference tests
```

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for setup, checks and developer tools. Report bugs
and request features in [GitHub issues](https://github.com/duartebarbosadev/PhotoSort/issues).

## License

PhotoSort's source code is licensed under the Apache License 2.0 ([LICENSE](LICENSE)).
Third-party components keep their own terms:

- Default builds link Exiv2 (GPL-2.0-or-later) and LibRaw (LGPL-2.1 or CDDL-1.0).
- The aesthetic model (`aesthetic.onnx`) is AGPL-3.0 per its model card; the other models
  are Apache-2.0 or MIT.
- ONNX Runtime is MIT-licensed.
- Offline location data is derived from GeoNames (CC BY 4.0) through the reverse_geocode
  package (LGPL).
- The Inter font is licensed under the SIL Open Font License 1.1.

Because PhotoSort links Exiv2, released app binaries are distributed under the terms of the
GNU General Public License, version 3. The source code itself remains Apache-2.0.

See [resources/ASSETS.md](resources/ASSETS.md) for provenance and
[resources/licenses/](resources/licenses/) for license texts. The macOS packager also
copies the notices of bundled native libraries into the app.

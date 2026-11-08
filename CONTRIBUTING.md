# Contributing to PhotoSort

Bug reports and pull requests are welcome. Report bugs and suggest features in
[GitHub issues](https://github.com/duartebarbosadev/PhotoSort/issues). Maintainers who
package and publish releases should read [docs/RELEASING.md](docs/RELEASING.md).

## Building

You need Rust 1.95 or later, a C/C++ compiler, pkg-config and Python 3.11+.

**macOS** (with the Xcode command line tools):

```sh
brew install libraw exiv2 pkg-config glib jpeg-turbo libpng libtiff little-cms2 highway
```

**Linux** (Debian 13 "trixie", as in CI):

```sh
sudo apt-get install build-essential pkg-config python3 python3-venv curl \
  libglib2.0-dev libexpat1-dev libjpeg-dev libpng-dev libtiff-dev liblcms2-dev libhwy-dev zlib1g-dev \
  libraw-dev libexiv2-dev libheif-dev libheif-plugin-libde265 \
  libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
  gstreamer1.0-plugins-base gstreamer1.0-plugins-good gstreamer1.0-libav \
  libxkbcommon-dev libwayland-dev libx11-dev libxi-dev libxcursor-dev \
  libxrandr-dev libxinerama-dev libegl1-mesa-dev
```

**Windows** (10 version 1903 or later, or 11; x64):

```powershell
winget install Rustlang.Rustup
winget install Microsoft.VisualStudio.2022.BuildTools --override "--wait --quiet --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
winget install gstreamerproject.gstreamer
cargo install cargo-vcpkg
cargo vcpkg build
```

`cargo vcpkg build` builds Exiv2, LibRaw, libheif and the libraries libvips uses, at the
versions pinned in `Cargo.toml`. It takes about 20 minutes once. vcpkg fails in a path with
non-ASCII characters, such as `C:\Users\João`. If yours has them, set a plain build folder
before both commands and keep using it:

```powershell
$env:CARGO_TARGET_DIR = "C:\photosort-target"
```

If GStreamer is not in its default folder, set `PKG_CONFIG` and `PKG_CONFIG_PATH` to its
`bin\pkg-config.exe` and `lib\pkgconfig`.

### libvips

The first `cargo build` builds libvips into `.tools/vips` with
[scripts/build_vips.py](scripts/build_vips.py). Delete that folder to rebuild it, or set
`PHOTOSORT_VIPS_PREFIX` to use another build. On Linux, run tests and the app with:

```sh
export LD_LIBRARY_PATH="$PWD/.tools/vips/lib"
```

## Development setup

Applying changes edits the folder you open, so try the app on a copy of the sample photos:

```sh
cp -R tests/samples ~/photosort-scratch
cargo run --release -- --folder ~/photosort-scratch
```

On Windows, in PowerShell:

```powershell
Copy-Item -Recurse tests\samples $HOME\photosort-scratch
cargo run --release -- --folder $HOME\photosort-scratch
```

Tests and examples use a temporary data and cache folder, so they never touch your settings.
They find models in `resources/models/` unless `PHOTOSORT_DATA_ROOT` points elsewhere.

| Variable | Purpose |
| --- | --- |
| `ORT_DYLIB_PATH` | ONNX Runtime library to load |
| `PHOTOSORT_DATA_ROOT` | Absolute path for app data (preferences, sessions, managed models) |
| `PHOTOSORT_CACHE_ROOT` | Absolute path for regenerable caches |
| `PHOTOSORT_ROTATION_MODEL` | Orientation model to use instead of `orientation.onnx` |
| `PHOTOSORT_SAMPLE_DIR` | Sample folder for the reference tests |
| `PHOTOSORT_RELEASE_REPOSITORY` | Build time only: GitHub `owner/repository` for update checks |

## AI models and ONNX Runtime

The models and ONNX Runtime are not in git. **Menu → AI models** downloads ONNX Runtime, the
seven models and, on Windows with an NVIDIA driver, NVIDIA GPU support. Every file is checked
against a pinned size and SHA-256. The runtime pins are in
[src/core/model_assets/runtime.rs](src/core/model_assets/runtime.rs), the model pins in
[resources/SHA256SUMS](resources/SHA256SUMS), and their sources in
[resources/ASSETS.md](resources/ASSETS.md).

`photosort --diagnostics` shows which runtime and models were found. `photosort
--check-runtime` loads them and runs every model; add `--runtime-backend cuda` to test the
GPU. On Linux, point `ORT_DYLIB_PATH` at ONNX Runtime's `gpu_cuda12` build to use CUDA.

## Command-line options

Run `photosort --help`, or `cargo run -- --help` from source.

## Checks

CI runs these on macOS, Windows and Linux. Run them before opening a pull request:

```sh
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
```

CI uses [cargo-nextest](https://nexte.st); `cargo nextest run` works the same locally.
Integration tests build into one binary, so list each new file in `tests/` in
[tests/integration.rs](tests/integration.rs).

The Python scripts have their own tests:

```sh
python3 -B -m unittest discover -s scripts -p 'test_*.py'
```

## Reference tests

Tests marked `#[ignore]` read the camera files in `tests/samples/` and edit only copies.
Several need the AI models. A few need extra variables named in their `#[ignore]` reason.
One moves a small text file to the system Trash.

```sh
PHOTOSORT_SAMPLE_DIR=tests/samples cargo test --release -- --ignored
PHOTOSORT_SAMPLE_DIR="$PWD/tests/samples" cargo test --locked -p photosort-imaging-vips -- --ignored
```

## Developer tools

Run each with `cargo run --release --example NAME -- ARGS`. They use temporary caches and
leave the photos they read unchanged.

| Example | Purpose |
| --- | --- |
| `audit_faces` | Lists every face-detector hit with its score, Face Mesh result, eye ratio and whether it was kept, as JSON. |
| `audit_library` | Runs the application workers on a real folder and reports repeatability and cache reuse. |
| `audit_orientation` | Prints orientation predictions, scores and uncertainty for the given photos, as JSON. |
| `bench_alignment` | Benchmarks near-duplicate alignment, excluding decoding. |
| `bench_catalog` | Benchmarks scanning a synthetic folder of empty files. |
| `bench_evidence` | Benchmarks cached-evidence loading and grouping on a synthetic library. |
| `bench_pipeline` | Runs a combined image-pipeline workload on a real folder. |
| `fold_model` | Pre-applies ONNX Runtime's basic graph optimizations to a model and checks the output is bit-identical. |
| `generate_heif_fixtures` | Writes tiny HEIF test fixtures to a new directory (Windows and Linux; needs an HEVC encoder). |
| `perf_budget` | Measures time to first thumbnail on a real folder and frame time while scrolling a 10,000-photo grid. |
| `ui_gallery` | Renders headless screenshots of the real UI from a temporary copy of a folder. |
| `verify_media` | Decodes the given files, checks detail-view crops, and prints dimensions and load times. |
| `video_probe` | Exercises AVFoundation playback on `tests/fixtures/video` (macOS only). |

## Project layout

```text
src/core/             catalog, image pipeline, metadata, models, file operations
src/ui/               egui interface and the five steps
crates/imaging-vips/  libvips adapter: JPEG, PNG, TIFF and GIF decoding
native/               ImageIO, AVFoundation, LibRaw, Exiv2 and libheif bindings
resources/            fonts, location data, license notices, model checksums
scripts/              packagers, libvips build, model export
examples/             developer audits and benchmarks
tests/                integration tests
tests/fixtures/       small generated fixtures
tests/samples/        seven camera samples for the reference tests
assets/screenshots/   README screenshots, rendered with the ui_gallery example
```

## Commit style

Write commit subjects in the imperative mood and sentence case, without a trailing period,
for example "Keep the missing-file check on Windows". Keep each commit to one change, and
run `cargo fmt --all` before committing.

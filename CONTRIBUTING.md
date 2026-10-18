# Contributing to PhotoSort

Bug reports and pull requests are welcome. Report bugs and suggest features in
[GitHub issues](https://github.com/duartebarbosadev/PhotoSort/issues). This guide covers
the development setup, the checks CI runs, the opt-in reference tests and the developer
tools in `examples/`.

## Development setup

Install Rust 1.95 or later and the native libraries for your platform, as described in
[Building from source](README.md#building-from-source). Then try the app on a disposable
copy of the sample photos, since applied plans change the files in the folder you open:

```sh
cp -R tests/samples ~/photosort-scratch
```

```sh
cargo run --release -- --folder ~/photosort-scratch
```

The AI models and ONNX Runtime are not in git. For the AI steps, put the runtime library
in `resources/runtime/` (or set `ORT_DYLIB_PATH`) and the models in `resources/models/`.
You can copy them from an existing `PhotoSort.app/Contents/Resources`, download DINOv2,
SFace and the orientation model from Menu → AI models, or prepare the others with the
recipes in [resources/ASSETS.md](resources/ASSETS.md); the face recipes start from
`assets/models/face_landmarker.task` in the `python-final` tag. `cargo run -- --diagnostics`
reports what was found.

Tests and examples are built with the `test-isolation` feature, so each process uses a
temporary data and cache folder and never touches your real preferences or caches. They
find models in `resources/models/`, not in the app's data folder, unless
`PHOTOSORT_DATA_ROOT` points there.

| Variable | Purpose |
| --- | --- |
| `ORT_DYLIB_PATH` | ONNX Runtime library to load |
| `PHOTOSORT_DATA_ROOT` | Absolute path for app data (preferences, sessions, managed models) |
| `PHOTOSORT_CACHE_ROOT` | Absolute path for regenerable caches |
| `PHOTOSORT_ROTATION_MODEL` | Orientation model to use instead of `orientation.onnx` |
| `PHOTOSORT_SAMPLE_DIR` | Sample folder for the opt-in reference tests |
| `PHOTOSORT_RELEASE_REPOSITORY` | Build time only: GitHub `owner/repository` for update checks |

## Checks

CI ([`.github/workflows/check.yml`](.github/workflows/check.yml)) runs the minimal build on
Ubuntu, macOS and Windows, the default build on macOS (with and without libvips), and the
libvips build in a Debian container. Run the same checks before opening a pull request:

```sh
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo clippy --locked --no-default-features --all-targets -- -D warnings
cargo test --locked
cargo test --locked --no-default-features
cargo check --locked --no-default-features --features onnx --all-targets
```

The packager and libvips build script have their own tests, which need no Rust build:

```sh
python3 -B -m unittest discover -s scripts -p 'test_*.py'
```

### libvips backend

Changes to the image pipeline should also pass with `native-imaging`. Build the private
libvips copy once (delete `.tools/vips` to rebuild it), then point pkg-config at it:

```sh
brew install glib jpeg-turbo libpng libtiff little-cms2 highway
python3 -B scripts/build_vips.py
export PKG_CONFIG_PATH="$PWD/.tools/vips/lib/pkgconfig"
cargo test --locked --features native-imaging
cargo clippy --locked --features native-imaging --all-targets -- -D warnings
cargo test --locked --manifest-path crates/imaging-vips/Cargo.toml
```

On Debian, install the libvips build dependencies instead of the Homebrew packages, and
also set `LD_LIBRARY_PATH` before running tests:

```sh
sudo apt-get install python3 python3-venv cmake ninja-build libglib2.0-dev \
  libexpat1-dev libjpeg-dev libpng-dev libtiff-dev liblcms2-dev libhwy-dev zlib1g-dev
export LD_LIBRARY_PATH="$PWD/.tools/vips/lib"
```

## Reference tests

Tests marked `#[ignore]` use real camera files from `tests/samples/` and the installed
models. They read the originals without modifying them and make edits on copies.

```sh
PHOTOSORT_SAMPLE_DIR=tests/samples cargo test --release -- --ignored
```

This runs every ignored test. Several need ONNX Runtime and the models, and a few need
extra variables named in their `#[ignore]` reason (`PHOTOSORT_ANALYSIS_REFERENCE`,
`PHOTOSORT_VIDEO_TIME_SAMPLE`, `PHOTOSORT_ORIENTATION_AUDIT_OUTPUT`) and fail without them.
`native_trash_moves_only_the_disposable_test_file` moves a small text file to the system
Trash. To run one file:

```sh
PHOTOSORT_SAMPLE_DIR=tests/samples cargo test --release --test reference_samples -- --ignored --nocapture
```

Cargo runs the root crate's tests from the repository root, so the relative path works
there. The libvips crate runs from its own directory and needs an absolute path:

```sh
PHOTOSORT_SAMPLE_DIR="$PWD/tests/samples" cargo test --locked --manifest-path crates/imaging-vips/Cargo.toml -- --ignored
```

## Regenerating test oracles

The reference files in `tests/` and `tests/fixtures/` were captured from the Python app by
the `scripts/capture_*_reference.py` scripts. They import the Python source, so check out
the last Python version next to this repository and install its dependencies with the
Python version named in its `.python-version`:

```sh
git worktree add ../photosort-python python-final
```

```sh
python3 -m venv ../photosort-python/.venv
```

```sh
../photosort-python/.venv/bin/pip install -r ../photosort-python/requirements.txt
```

Each script takes the Python checkout as its first argument and an output file or
directory as its second (see the script's docstring). For example:

```sh
../photosort-python/.venv/bin/python scripts/capture_technical_reference.py ../photosort-python tests/technical_reference.json
```

`capture_analysis_reference.py` writes a directory of images; pass it to the analysis test
as `PHOTOSORT_ANALYSIS_REFERENCE`. Remove the worktree with
`git worktree remove ../photosort-python` when you are done.

## Developer tools

Run each with `cargo run --release --example NAME -- ARGS`. All of them use temporary
caches and leave the photos they read unchanged.

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
| `generate_heif_fixtures` | Writes tiny HEIF test fixtures to a new directory (libheif builds; needs an HEVC encoder). |
| `perf_budget` | Measures time to first thumbnail on a real folder and frame time while scrolling a 10,000-photo grid. |
| `ui_gallery` | Renders headless screenshots of the real UI from a temporary copy of a folder. |
| `verify_media` | Decodes the given files, checks detail-view crops, and prints dimensions and load times. |
| `video_probe` | Exercises AVFoundation playback on `tests/fixtures/video` (macOS only). |

To check orientation suggestions without changing photos:

```sh
cargo run --release --locked --example audit_orientation -- /path/to/photo.jpg
```

Model scores in its output are softmax values, not calibrated probabilities of correctness.

## Build storage

The `dev` and `test` profiles use line-table debug information and disable incremental
compilation. Backtraces keep file and line numbers, and `target/` stays much smaller, at
the cost of slower rebuilds. For a full debug session:

```sh
CARGO_PROFILE_DEV_DEBUG=2 CARGO_INCREMENTAL=1 cargo run
```

To package the macOS app from a temporary build directory that is removed afterwards:

```sh
(
  build_dir="$(mktemp -d)"
  trap 'cargo clean --target-dir "$build_dir"' EXIT
  CARGO_TARGET_DIR="$build_dir" python3 scripts/package_macos.py
)
```

`cargo clean` removes all build output, including `target/PhotoSort.app`.

## Commit style

Write commit subjects in the imperative mood and sentence case, without a trailing period,
for example "Keep the missing-file check on Windows". Keep each commit to one change, and
run `cargo fmt --all` before committing.

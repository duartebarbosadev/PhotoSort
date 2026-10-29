# Contributing to PhotoSort

Bug reports and pull requests are welcome. Report bugs and suggest features in
[GitHub issues](https://github.com/duartebarbosadev/PhotoSort/issues). This guide covers
building on each platform, the AI runtime, command-line options, packaging, the checks CI
runs, the opt-in reference tests and the developer tools in `examples/`.

## Building

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

## Development setup

Try the app on a disposable copy of the sample photos, since applied plans change the files
in the folder you open:

```sh
cp -R tests/samples ~/photosort-scratch
```

```sh
cargo run --release -- --folder ~/photosort-scratch
```

The AI models and ONNX Runtime are not in git. For the AI steps, put the runtime library
in `resources/runtime/` (or set `ORT_DYLIB_PATH`). Menu → AI models downloads all seven
models into the app's data folder. Alternatively, put them in `resources/models/`: copy
them from an existing `PhotoSort.app/Contents/Resources`, or recreate the converted ones
with the recipes in [resources/ASSETS.md](resources/ASSETS.md); the face recipes start from
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

## AI models and ONNX Runtime

| Model | Purpose | Used by |
| --- | --- | --- |
| `dinov2-small.onnx` (DINOv2 small) | Scene similarity | Organize (visual, date + similarity), Easy Delete, Pick Best, Cull |
| `aesthetic.onnx` (cafe_aesthetic, BEiT) | Aesthetic score | Pick Best |
| `face_detector.onnx` (MediaPipe) | Face detection | Organize (faces), Easy Delete, Pick Best, Cull |
| `face_landmarks_detector.onnx` (MediaPipe Face Mesh V2) | Eye positions and alignment | Organize (faces), Easy Delete, Pick Best, Cull |
| `face_blendshapes.onnx` (MediaPipe) | Eye openness | Easy Delete, Pick Best, Cull |
| `face_recognition_sface.onnx` (OpenCV Zoo SFace) | Same person across photos | Organize (faces) |
| `orientation.onnx` (deep-image-orientation-detection v2) | Sideways and upside-down photos | Fix Rotation |

`resources/SHA256SUMS` pins the exact content of every model. **Menu → AI models** checks
the installed copies and downloads any of the seven: DINOv2, SFace and the orientation model
from pinned upstream files, and the four converted exports from this repository's
[`models-1` prerelease](https://github.com/duartebarbosadev/PhotoSort/releases/tag/models-1).
Every download is verified by size and SHA-256 before it is installed, and nothing downloads
unless you ask. The same panel restores a bundled copy or imports a prepared ONNX file;
[resources/ASSETS.md](resources/ASSETS.md) has the provenance and export recipes.

ONNX Runtime is loaded at run time and is never downloaded by the app. PhotoSort looks for
it in this order:

1. The path in `ORT_DYLIB_PATH`.
2. `runtime/libonnxruntime.dylib` (`onnxruntime.dll` on Windows, `libonnxruntime.so` on
   Linux) in the app's data folder, in `./resources`, or in the app bundle's resources.
3. `/opt/homebrew/lib/libonnxruntime.dylib` and `/usr/local/lib/libonnxruntime.so`.

The bundled runtime is ONNX Runtime 1.29.0. `photosort --diagnostics` shows which files
were found, and `photosort --check-runtime` loads the runtime and runs every model.

Settings selects the inference backend: CPU, Automatic, Apple Core ML, NVIDIA CUDA or
Windows DirectML. New installs default to Automatic on macOS (Core ML) and to CPU on Windows
and Linux, where CUDA and DirectML are not yet verified on real hardware; saved preferences
keep their choice. A backend that fails falls back to CPU with an explanation, per model.
Core ML can take minutes to prepare on first use; prepared models are cached afterwards.

## Command-line options

Usage: `photosort [--folder PATH | --last-folder] [options]` or `photosort [PATH]`. From
source, pass options after `--`, for example `cargo run --release -- --diagnostics`.

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

## macOS app bundle

`python3 scripts/package_macos.py` builds a release binary and copies it, `resources/`
(including any models and ONNX Runtime there) and the linked native libraries with their
license notices into a relocatable `target/PhotoSort.app`, renders `AppIcon.icns` from
`resources/icons/app_icon.png` with `sips` and `iconutil`, then verifies its signature and
resource manifest. `--output /path/to/PhotoSort.app` chooses another destination,
`--native-imaging` bundles the libvips backend (see [libvips backend](#libvips-backend)), and
`CARGO_TARGET_DIR` is honored (see [Build storage](#build-storage)).

Bundles are ad-hoc signed and not notarized by default, so macOS may ask again for folder
access after a rebuild. `--sign-identity` takes an existing codesign identity name or SHA-1
for a consistent signature; named identities also get a secure timestamp and the hardened
runtime, which notarization requires. The bundle's minimum macOS is the highest required by
any binary in it, and Homebrew libraries require the macOS version they were built for.

Update checks read the GitHub releases of the repository set with
`PHOTOSORT_RELEASE_REPOSITORY=owner/repository` at build time (the release workflow sets
`duartebarbosadev/PhotoSort`); builds without it make no update requests. Release tags must be semantic versions such as `3.0.1` or `v3.0.1`, and
drafts, prereleases and downgrades are ignored. The app links to the release page and never
downloads or installs an update.

## Releases

[`.github/workflows/release.yml`](.github/workflows/release.yml) builds the macOS app for
Apple Silicon and drafts a GitHub release. To release:

1. Make sure `main` has what you want to ship and its checks pass.
2. Open **Actions → Release → Run workflow** and enter the version, for example `3.0.1`.
   The workflow sets it in `Cargo.toml` and `Cargo.lock`, commits "Release 3.0.1" to
   `main`, tags it `v3.0.1`, builds the app and creates a draft release. It refuses an
   existing tag or a version older than the current one.
3. Review the draft release, replace the summary placeholder in its notes, and publish it.

Leave the version empty for a test build: it builds and checks the same app and keeps it
as a workflow artifact, without a commit, tag or release. Pushing a `v*` tag by hand also
starts a release; the tag must then match the version already in `Cargo.toml`.

The workflow runs on a pinned `macos-15` runner. It installs the Homebrew libraries, builds
the private libvips copy, downloads ONNX Runtime 1.29.0 from the official release archive,
fetches DINOv2, SFace and the orientation model from their pinned upstream URLs and the
four converted models from the `models-1` prerelease (see
[Converted models](#converted-models)), checks every file against `resources/SHA256SUMS`,
and packages the app with `--native-imaging` and
`PHOTOSORT_RELEASE_REPOSITORY=duartebarbosadev/PhotoSort`. It then runs `photosort --check-runtime` from the finished bundle and uploads
`PhotoSort-<version>-macos-arm64.zip`, its SHA-256 checksum file and notes filled in from
[`.github/release-notes.md`](.github/release-notes.md). The app's update dialog shows release
notes as plain text, so keep them readable without Markdown rendering. Bundled Homebrew
libraries require the macOS version they were built for, so the runner image sets the
app's minimum macOS; the packager writes it to `Info.plist` and the notes repeat it.

Drafts are not "latest", so neither PhotoSort 3 nor PhotoSort 2.x sees a release until it
is published. PhotoSort 2.x announces any newer published release on every platform and
opens its page, even without a build for that platform; the notes template says so.

**Test build.** Run the workflow with an empty version (Release → Run workflow, then pick a
branch) or with `gh workflow run release.yml --ref <branch>`. It builds and checks
everything, including signing and notarization when the secrets are set, and keeps the zip,
checksums and notes as the `photosort-macos-arm64` workflow artifact for 14 days. It never
creates a commit, tag or release. To release from the command line instead, use
`gh workflow run release.yml -f version=3.0.1`.

### Converted models

`aesthetic.onnx`, `face_detector.onnx`, `face_landmarks_detector.onnx` and
`face_blendshapes.onnx` are converted exports with no upstream download (see
[resources/ASSETS.md](resources/ASSETS.md)). Release builds download them from the assets
of this repository's `models-1` prerelease and then check them against
`resources/SHA256SUMS`, so a changed asset fails the build instead of shipping. Create
the prerelease once, from a folder of files that match those pins:

```sh
gh release create models-1 --repo duartebarbosadev/PhotoSort --target main --prerelease \
  --title "Converted models 1" \
  --notes "Converted ONNX models for release builds; provenance and licenses in resources/ASSETS.md. aesthetic.onnx is AGPL-3.0." \
  aesthetic.onnx face_detector.onnx face_landmarks_detector.onnx face_blendshapes.onnx
```

Keep it a prerelease: PhotoSort 3 and 2.x both read the latest release, and its tag is not
a version. Its tag must not start with `v`, or it would start this workflow. To change a
converted model, publish a new prerelease (`models-2`), update its pin in
`resources/SHA256SUMS` and `MODELS_RELEASE` in the workflow, and leave older ones in place so
earlier tags still build.

### Signing and notarization

Without secrets, releases keep the packager's ad-hoc signature and are not notarized, so
macOS asks users to approve the first launch; the notes explain how. Set these repository
secrets (Settings → Secrets and variables → Actions) to sign and notarize instead:

| Secret | Value |
| --- | --- |
| `MACOS_CERTIFICATE` | Base64 of a `.p12` export of the Developer ID Application certificate with its private key (`base64 -i certificate.p12`) |
| `MACOS_CERTIFICATE_PASSWORD` | The password chosen for that export |
| `APPLE_ID` | Apple Account email of a member of the developer team |
| `APPLE_TEAM_ID` | The ten-character team ID |
| `APPLE_APP_PASSWORD` | An app-specific password for that Apple Account |

The two certificate secrets alone give a Developer ID signature with a secure timestamp
and the hardened runtime, but no notarization. All five also notarize the zip with
`notarytool`, staple the ticket to the app and zip it again. A partial set of either group
stops the workflow instead of silently producing a weaker build. The certificate goes into
a temporary keychain that is deleted at the end of the job.

The hardened runtime enables library validation, so a Developer ID build loads only code
signed by the same team: its bundled ONNX Runtime works, but `ORT_DYLIB_PATH` or a Homebrew
copy of the library is refused. Ad-hoc builds are unaffected.

## Minimal build

`--no-default-features` builds PhotoSort without its optional native dependencies: LibRaw,
libheif, ONNX Runtime, Exiv2, GStreamer and the network features. It opens JPEG, PNG, TIFF,
GIF, WebP and BMP, keeps every step's manual review, and writes ratings and rotation to XMP
sidecars only. It is not a release configuration. It exists so that:

- CI can build and test on Windows, where the full native stack is not set up yet;
- you can work on the UI and core logic with only Rust installed, and compile faster;
- a binary without GPL code (Exiv2) can be built if one is ever needed.

```sh
cargo run --release --no-default-features
```

On Linux and Windows, `portable-video` adds in-app video playback to it; without it, video
posters need `ffmpeg` on the PATH. macOS always uses AVFoundation.

```sh
cargo run --release --no-default-features --features portable-video
```

## Checks

CI ([`.github/workflows/check.yml`](.github/workflows/check.yml)) runs the minimal build on
Ubuntu and Windows, the full build on macOS (with and without libvips), and the full libvips
build in a Debian container. Formatting is checked once. Changes that only touch Markdown
files skip CI, and Rust build caches are saved from `main`, so pull requests reuse them.
CI runs tests with [cargo-nextest](https://nexte.st), which runs every test in one parallel
pool; `cargo nextest run` works the same locally, and plain `cargo test` still runs
everything. Integration tests build into one binary, so a new file in `tests/` must be listed
in `tests/integration.rs` (a test fails if it is not). The private libvips build is cached
until its script or libraries change.
Release builds are separate (see [Releases](#releases)). Run the same checks before opening
a pull request:

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

The optional `native-imaging` feature decodes JPEG, PNG, TIFF and GIF through libvips and
Little CMS instead of the default Rust decoders. It is faster for large JPEG, PNG and TIFF
files and supports ICC-profiled CMYK TIFF. See
[crates/imaging-vips/README.md](crates/imaging-vips/README.md) for details.

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
PHOTOSORT_SAMPLE_DIR=tests/samples cargo test --release --test integration reference_samples:: -- --ignored --nocapture
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
assets/screenshots/   README screenshots, rendered with the ui_gallery example
```

## Commit style

Write commit subjects in the imperative mood and sentence case, without a trailing period,
for example "Keep the missing-file check on Windows". Keep each commit to one change, and
run `cargo fmt --all` before committing.

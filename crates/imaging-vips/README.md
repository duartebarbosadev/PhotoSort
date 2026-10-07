# PhotoSort native image engine

Implementation stage: connected to `core::pipeline` through the optional
`native-imaging` feature. Default builds retain the existing Rust image/color
path while supported-platform validation continues. No workflow constructs
or directly calls this crate.

The adapter delegates codecs, color conversion, thumbnail shrinking and regions
to libvips 8.18+ and Little CMS 2.16+. It owns only the native interface:

- Process initialization once, four native evaluation threads, no global operation
  cache and no per-owner global shutdown. Native calls are serialized to protect
  libvips' process-global error buffer; waiting admission remains cancellable.
- Literal filesystem paths (including libvips option-like brackets).
- Native errors, null pointers, buffer lengths and cleanup checked before Rust
  constructs slices; native allocations always released by their allocator.
- Borrowed, synchronous cancellation callbacks requiring `Sync`, with panics
  contained at the Rust boundary. Failed/cancelled renders never return pixels.
- 32768 maximum dimension, 512 MiB source/output byte limits, 4 MiB ICC limit,
  and preview edges from 1 to 4096. These are allocation limits, not a promise
  about total process RSS. Rust currently copies the native output once.
- Exposed invalid, mismatched or relative-intent-incompatible ICC profiles return a visible
  error rather than allowing an unreported profile fallback. The shared UI uses its
  existing image-error and Retry handling. If a loader discards
  metadata first (for example libpng rejecting iCCP), its setup warning is returned
  separately with the pixels and must be surfaced by the shared UI. Warning
  capture is scoped to the calling thread/job, not another concurrent image.
- Straight-alpha sRGB RGBA; 16-bit integer precision retained for full resolution.
  EXIF and sidecar orientation remain the shared pipeline's responsibility.

Build the small private native dependency with `python3 scripts/build_vips.py`
from the repository root. The script verifies the pinned upstream source checksum,
uses temporary build tools/source/output, retains license notices and provenance,
and installs only to `.tools/vips` by default. Highway SIMD is enabled by default;
`--no-simd` is available for scalar comparisons. Existing prefixes are never overwritten.
Python is not a runtime dependency. Native development libraries must already be
installed; the script does not change the system package manager.

For the currently verified macOS setup, run from the repository root:

```sh
export PKG_CONFIG_PATH="$PWD/.tools/vips/lib/pkgconfig"
export CARGO_TARGET_DIR=/private/tmp/photosort-vips-rust-build
cargo test --locked --manifest-path crates/imaging-vips/Cargo.toml
PHOTOSORT_SAMPLE_DIR="$PWD/../PhotoSort/tests/samples" cargo test --locked --manifest-path crates/imaging-vips/Cargo.toml original_jpeg_png_samples -- --ignored
cargo clippy --locked --manifest-path crates/imaging-vips/Cargo.toml --all-targets -- -D warnings
cargo clean --manifest-path crates/imaging-vips/Cargo.toml
```

Tests cover transparent pixels, ICC correction, 16-bit low bits, CMYK/YCCK JPEG,
packed/planar/LZW CMYK TIFF, malformed inputs/profiles, crop bounds, orientation,
concurrent initialization, cancellation/panic recovery and the six original
raster samples. TIFF generators reuse the archived evaluation fixture code.
The seventh sample is RAW and remains owned by LibRaw.

The shared pipeline prepares one 2048-edge working image for display and neutral
analysis, loads originals on demand, and keeps both recipes in its bounded LRU.
Analysis geometry comes from source dimensions, avoiding rounding drift from an
intermediate image. Analysis pixels are independent of original-view order.
Warnings and geometry are persisted with both proxy types, then reach the shared
inspector and review canvas. Opaque 8-bit proxies omit redundant alpha channels on disk, while retaining exact
RGBA pixels after reopening; transparent and 16-bit behavior remains unchanged.
Cache identities include library versions, target architecture and the private
build recipe (codec versions and options). External packages without that recipe
retain version/target identity and require cache clearing after custom same-version
rebuilds.
RAW, HEIF, video and unsupported native formats retain their established services.
Original inspection now requests oriented source regions through the same image
actor and detail LRU. Source-resolution requests expand to 512-pixel grid bounds
so nearby pans and workflow changes can reuse one prepared neighborhood. Scaled
requests retain only their output-size pixels. Cropping occurs before rotation;
rotation materializes the crop after any display-size reduction so sequential
codecs do not need to reread earlier rows. Existing full-resolution working images
are reused when available. The adapter returns canonical source dimensions and
the pipeline owns sidecar precedence, bounds checks, source invalidation and
cancellation. There is no independent tile cache or workflow decoder.

The seven focused region regressions cover all eight orientations, 16-bit pixels,
ICC/alpha, clipped bounds, budget eviction, nearby-pan reuse, latest-worker epochs
and rotated fit views. Complete native/default macOS suites, release measurements and native
Organize/Cull detail checks now pass; see
`docs/diagnostics/native-regions-2026-10-06/README.md`. On the controlled
24-megapixel sample JPEG/PNG nearby-pan workload, peak process RSS is 61–63% lower
and total time 17–38% shorter than the previous full-original allocation, with
identical native crop pixels. These results do not establish savings for every
decoder or whole-application performance. Subsequent macOS checks verified native
panning in Organize and Cull on disposable JPEG/PNG and sidecar-rotated samples;
see [shared canvas gesture evidence](../../docs/diagnostics/canvas-gestures-2026-10-06/README.md).
All-five-workflow tests cover pointer ownership, completed gestures, nearby-pan
reuse, bounds and cancellation. The native checks cover the recorded workflows
and inputs; they do not establish every workflow or platform's interaction.

To run the optional application path after building the private dependency, set
`PKG_CONFIG_PATH` and `CARGO_TARGET_DIR` as above, then run
`cargo run --locked --features native-imaging -- --folder /path/to/library`.
On macOS, `python3 scripts/package_macos.py --native-imaging --native-prefix
.tools/vips-simd --output ".tools/native-vips/PhotoSort Rust.app"` bundles the
verified SIMD prefix. Omit `--native-prefix` to use `.tools/vips`. The packager
resolves transitive native libraries from their original loader paths, retains
license/provenance notices, audits the resulting dependency graph and verifies
the local signature. The default packaging command keeps the previous backend.

The optional macOS package now passes launch, shared warning display, original
detail, active 80-image analysis cancellation, post-cancellation navigation and
warning persistence after restart. See
`docs/diagnostics/vips-native-validation-2026-10-05.json`.

The adapter and shared application pipeline also pass headless Linux aarch64 tests
in an isolated Debian container; see `docs/diagnostics/linux-native-2026-10-05/README.md`.
Private builds use a fixed `lib` directory on both platforms. On Linux, set
`LD_LIBRARY_PATH` to the private prefix’s `lib` directory as well as setting
`PKG_CONFIG_PATH` before running the application or tests.

Before making this the default application backend: complete supported-platform
validation. Linux desktop interaction/distribution and Windows native execution
remain unverified. A successful crate test does not establish application parity. See
`docs/diagnostics/vips-color-comparison-2026-10-05` for the explicitly accepted,
measured CMYK difference between color engines; it is not an accuracy improvement
claim or a promise of pixel-identical output.

The combined seven-sample comparison is recorded in
`docs/diagnostics/proxy-workload-2026-10-05/README.md`. SIMD resolves the earlier
scalar first-browsing regression: 15% faster first browsing and 29% shorter combined
pipeline time than the default path in the same run. Peak whole-process RSS is
only 3% lower once originals are included. This is not whole-app, Python or
large-library/model-accuracy evidence.

The final SIMD macOS package passes native transparent/warning rendering, JPEG
original detail, active analysis cancellation, post-cancellation navigation and
restart checks. Evidence: `docs/diagnostics/simd-native-validation-2026-10-05.json`
(repository-relative path). The default app bundle remains separate.

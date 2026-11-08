# PhotoSort native image engine

`photosort-imaging-vips` decodes JPEG, PNG, TIFF and GIF for PhotoSort. It hands decoding,
color conversion, preview shrinking and region reads to libvips 8.18+ and Little CMS 2.16+,
behind the shared image pipeline (`core::pipeline`). RAW, HEIF and video have their own
services, and WebP and BMP use the Rust `image` crate. No workflow calls this crate directly.

JPEG previews shrink while decoding: the adapter opens a JPEG with libjpeg-turbo's scaled
IDCT at the largest 1/2, 1/4 or 1/8 that still covers the requested edge, then reduces once.
`vips_thumbnail` alone keeps at least 2x for its final reduce, so a 24 MP JPEG would decode
at full size for a 2048 px preview. The adapter also reads ICC-profiled CMYK TIFF.

## What the adapter owns

- One process-wide initialization, four native evaluation threads, no global operation
  cache and no per-owner global shutdown. Native calls are serialized to protect libvips'
  process-global error buffer; waiting for admission stays cancellable.
- Literal filesystem paths (including libvips option-like brackets).
- Native errors, null pointers, buffer lengths and cleanup are checked before Rust builds
  slices; native allocations are always released by their own allocator.
- Borrowed, synchronous cancellation callbacks requiring `Sync`, with panics contained at
  the Rust boundary. Failed or cancelled renders never return pixels.
- Limits: 32768 maximum dimension and 512 MiB source/output bytes for full-size and
  region renders, 4 MiB ICC profiles, and preview edges from 1 to 4096. A downsized
  preview is bounded by its output instead, since loaders shrink on load or stream the
  source; its source may be up to 65535 px on an edge and 2^30 pixels. These bound
  allocations, not total process memory.
- Invalid, mismatched or relative-intent-incompatible ICC profiles return a visible error
  instead of an unreported fallback. If a loader discards metadata first (for example
  libpng rejecting iCCP), its warning is returned separately with the pixels. Warning
  capture is scoped to the calling thread.
- Output is straight-alpha sRGB RGBA, keeping 16-bit precision at full resolution. EXIF
  and sidecar orientation stay the shared pipeline's responsibility.

Original inspection requests oriented source regions through the shared image actor and
detail cache. Source-resolution requests expand to 512-pixel grid bounds so nearby pans
can reuse one prepared neighborhood; cropping happens before rotation. Cache identities
include the libvips version, target architecture and private build recipe, so a custom
rebuild of the same version needs a cache clear.

## Building the private dependency

[scripts/build_vips.py](../../scripts/build_vips.py) builds a pinned libvips into
`.tools/vips` (or `--prefix`). It verifies the source checksum, builds in temporary
directories, keeps license notices and provenance, and never overwrites an existing prefix.
Highway SIMD is on by default; `--no-simd` builds the scalar variant. The script does not
install system packages (see [CONTRIBUTING.md](../../CONTRIBUTING.md#building)).

`build.rs` uses `PHOTOSORT_VIPS_PREFIX` if set, else a libvips pkg-config finds (macOS and
Linux), else `.tools/vips`, which it builds on first use. A lock file beside the prefix
keeps concurrent builds from building it twice, and a failed attempt removes its partial
prefix.

On Windows (MSVC) the script builds a static libvips after `cargo vcpkg build`. GLib comes
from the installed GStreamer, and libjpeg-turbo, libpng, libtiff, Little CMS, expat, zlib
and Highway are vcpkg's static libraries. `build.rs` links libvips and the codecs into the
application statically, under names of their own so GStreamer's `jpeg.lib` or `tiff.lib`
cannot stand in for them. GLib links through GStreamer's import libraries, so the process
has one GLib, shared with video, and no extra DLL.

PhotoSort delay-loads GStreamer's DLLs so that it starts without them, and points Windows
at them (`core::playback::locate_runtime`) before the first image is decoded. A delay-loaded
DLL cannot export variables to the program, and libvips reads two GLib variables
(`g_ascii_table`, `g_param_spec_types`) through macros, so `native/glib_data_win.c` defines
their import slots, linked whole, and fills them from GLib before `vips_init`.

## Testing

From the repository root:

```sh
cargo test --locked --manifest-path crates/imaging-vips/Cargo.toml
cargo clippy --locked --manifest-path crates/imaging-vips/Cargo.toml --all-targets -- -D warnings
```

The tests cover transparency, ICC correction, 16-bit low bits, CMYK/YCCK JPEG, packed,
planar and LZW CMYK TIFF, malformed inputs and profiles, crop bounds, orientation,
concurrent initialization and cancellation/panic recovery. `src/tiff_fixture.rs` generates
the TIFF fixtures. An opt-in test decodes the repository's sample photos:

```sh
PHOTOSORT_SAMPLE_DIR="$PWD/tests/samples" cargo test --locked \
  --manifest-path crates/imaging-vips/Cargo.toml original_jpeg_png_samples -- --ignored
```

On Linux, set `LD_LIBRARY_PATH` to the prefix's `lib` directory first. On Windows, the
tests build outside the main `target` folder and link GLib directly, so point them at the
vcpkg tree and put GStreamer on `PATH`:

```powershell
$env:VCPKG_ROOT = "$PWD\target\vcpkg"
$env:PATH = "C:\Program Files\gstreamer\1.0\msvc_x86_64\bin;$env:PATH"
cargo test --locked --manifest-path crates/imaging-vips/Cargo.toml
```

## Packaging

On macOS, `python3 scripts/package_macos.py` bundles the private build and its license
notices, audits the dependency graph and verifies the code signature. On Windows, libvips
is part of `photosort.exe`, and `python scripts/package_windows.py` bundles its license
notices and build provenance.

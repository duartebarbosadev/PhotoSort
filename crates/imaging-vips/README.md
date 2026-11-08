# PhotoSort native image engine

`photosort-imaging-vips` decodes JPEG, PNG, TIFF and GIF for PhotoSort. It hands decoding,
color conversion, preview shrinking and region reads to libvips 8.18+ and Little CMS 2.16+,
behind the shared image pipeline (`core::pipeline`). RAW, HEIF and video have their own
services, and WebP and BMP use the Rust `image` crate. No workflow calls this crate directly.

Embedded RAW previews and the preview cache's JPEGs also decode with `image`: they are
small, and libvips' fixed per-call cost makes it slower on them. Original photos are the
case libvips wins. The comment on `native_raster` in `src/core/pipeline.rs` has the
benchmark.

JPEG previews shrink while decoding: the adapter opens a JPEG with libjpeg-turbo's scaled
IDCT at the largest 1/2, 1/4 or 1/8 that still covers the requested edge, then reduces once.
`vips_thumbnail` alone keeps at least 2x for its final reduce, so a 24 MP JPEG would decode
at full size for a 2048 px preview. The adapter also reads ICC-profiled CMYK TIFF.

## What the adapter guarantees

- libvips initializes once per process, with four evaluation threads and no operation
  cache. Native calls are serialized; waiting for one stays cancellable.
- Native errors, null pointers and buffer lengths are checked before Rust builds slices,
  and a failed or cancelled render never returns pixels.
- Full-size and region renders are limited to 32768 px on an edge and 512 MiB; previews
  are bounded by their output size.
- A bad ICC profile is an error, not a silent fallback.
- Output is straight-alpha sRGB RGBA, 16-bit at full resolution. Orientation is the shared
  pipeline's job.
- Cache keys include the libvips version and build recipe, so a custom rebuild of the same
  version needs a cache clear.

## Building the private dependency

[scripts/build_vips.py](../../scripts/build_vips.py) builds a pinned libvips into
`.tools/vips` (or `--prefix`). It verifies the source checksum, builds in temporary
directories, keeps license notices and provenance, and never overwrites an existing prefix.
It always builds with Highway SIMD. The script does not install system packages (see [CONTRIBUTING.md](../../CONTRIBUTING.md#building)).

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

The crate is part of the workspace, so `cargo clippy --workspace` checks it and this runs
its tests:

```sh
cargo test --locked -p photosort-imaging-vips
```

The tests cover transparency, ICC correction, 16-bit low bits, CMYK/YCCK JPEG, packed,
planar and LZW CMYK TIFF, malformed inputs and profiles, crop bounds, orientation,
concurrent initialization and cancellation/panic recovery. `src/tiff_fixture.rs` generates
the TIFF fixtures. An opt-in test decodes the repository's sample photos:

```sh
PHOTOSORT_SAMPLE_DIR="$PWD/tests/samples" cargo test --locked -p photosort-imaging-vips \
  original_jpeg_png_samples -- --ignored
```

On Linux, set `LD_LIBRARY_PATH` to the prefix's `lib` directory first. On Windows, the
tests link GStreamer's GLib directly, so put it on `PATH`:

```powershell
$env:PATH = "C:\Program Files\gstreamer\1.0\msvc_x86_64\bin;$env:PATH"
cargo test --locked -p photosort-imaging-vips
```

## Packaging

On macOS, `python3 scripts/package_macos.py` bundles the private build and its license
notices, audits the dependency graph and verifies the code signature. On Windows, libvips
is part of `photosort.exe`, and `python scripts/package_windows.py` bundles its license
notices and build provenance.

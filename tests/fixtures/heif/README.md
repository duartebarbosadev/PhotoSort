# Generated HEIC alpha fixture

`alpha.heic` is a self-generated 96 × 64 sRGB HEIC image containing four vertical
bands with the same straight RGB source color `(220, 100, 40)` and alpha values
255, 128, 64 and 0. It contains no personal photographs. The checked-in bytes were
encoded by macOS ImageIO on 5 October 2026; SHA-256:
`849ad8bad4f812bcc56615c673dfa9b1eb9c75cf6dbfd024f955440448cddf5e`.

The fixture establishes the alpha convention at the shared pipeline boundary.
Lossy HEVC color encoding and 8-bit unpremultiplication can round colors slightly;
the regression allows five levels per visible color channel, preserves alpha
exactly and checks preview, analysis, original detail, shared workflow reuse and
persistent-cache reopening. Fully transparent pixels are zeroed by vImage.
This does not establish HDR, wide-gamut or arbitrary HEIF container coverage.

To regenerate a disposable candidate from the repository root:

```sh
clang tests/fixtures/heif/generate_alpha.c native/apple_image.m \
  -framework Foundation -framework CoreFoundation -framework CoreGraphics -framework ImageIO \
  -framework Accelerate -o /private/tmp/photosort-heif-alpha-probe
/private/tmp/photosort-heif-alpha-probe /private/tmp/photosort-alpha.heic
```

The helper also prints four pixels from the production decoder. Re-encoding on
another OS/codec version may change bytes; do not replace the pinned fixture just
because a regenerated file has a different hash. ImageIO encoding may require
normal native-app permissions outside a restrictive shell sandbox.

## Portable fixtures

`portable/` contains twelve self-generated 64 × 48 HEVC fixtures, encoded with
libheif 1.23.4 and its x265 plugin on Linux aarch64. They contain only synthetic
gray patterns. `portable/manifest.json` pins their SHA-256 hashes.

- `orientation-1.heic` through `orientation-8.heic`: all eight EXIF-equivalent
  container transforms. The coded image is 64 × 64; a clean-aperture property
  removes 16 padded rows before rotation/mirroring. The regression also decodes
  with transformations disabled to establish that this crop is actually present.
- `linear-icc.heic` / `linear-nclx.heic`: gray 128 in linear RGB, alpha 128. The
  shared pipeline should produce approximately sRGB 188, preserving alpha.
- `premultiplied.heic`: stored RGB 64 with alpha 128 and the premultiplied flag;
  the display contract requires approximately straight RGB 128.
- `invalid-icc.heic`: malformed embedded profile which must fail visibly.

Generate candidates in a new directory on a non-macOS HEIF build:

```sh
cargo run --locked --example generate_heif_fixtures -- /tmp/photosort-heif-candidates
```

Generation requires an HEVC encoder plugin; normal tests need only a decoder
(such as Debian's `libheif-plugin-libde265`). The generator uses libheif's encoder
and orientation APIs. For the linear NCLX fixture only, it changes the encoder's
single NCLX tag to BT.709 primaries / linear transfer, keeping encoded gray values
unchanged. This small fixture-authoring patch is not an application parser.

These fixtures cover SDR 8-bit behavior. They do not establish HDR, every camera
profile, Windows runtime behavior, or cancellation inside a running native codec
call. Cancellation is checked around decoding and during row/color processing.

## Bounded preview fixture

`large-alpha.heic` is a self-generated 3073 x 2051 sRGB HEIC with asymmetric
gradients, a checker pattern and four alpha bands. It contains no personal photos.
ImageIO encoded these bytes on 6 October 2026; SHA-256 `51fbf09f202d9517e078eaaa0f4f3bbb9b609e066bcae4990cf8ad51c554b1c2`.
It tests reduced previews, source geometry, alpha, all eight sidecar orientations,
original-first versus preview-first behavior and metadata-only cache reopening.
It complements the small container-orientation and profile fixtures.

Generate a disposable candidate with:

```sh
clang -O2 scripts/generate_heif_preview_fixture.c \
  -framework CoreFoundation -framework CoreGraphics -framework ImageIO \
  -o /private/tmp/photosort-heif-preview-fixture
/private/tmp/photosort-heif-preview-fixture /private/tmp/photosort-heif-preview-candidate.heic
```

Encoding is lossy; exact file bytes may vary with OS/codec versions. The regression
requires exact original pixels and request-order independence, and compares the
reduced view to a full-rendered reference with a mean absolute channel error below
six levels. This allows different resampling kernels without permitting flipped,
rotated, darkened or opaque output.

# Asset provenance

This file records where every bundled model, runtime and data file comes from, how it was
prepared, and the terms it is distributed under. `SHA256SUMS` pins the exact content of
each file; the app and the macOS packager verify against it.

Large ONNX weights and native runtime libraries are not stored in Git (see `.gitignore`).
A fresh clone gets them in one of three ways:

- **In the app.** Menu → AI models downloads the pinned DINOv2, SFace and orientation files.
  Downloads stream into private temporary files and are installed only after their exact
  size and SHA-256 match. Nothing is downloaded automatically.
- **Import.** The same panel imports a prepared ONNX file and verifies it against
  `SHA256SUMS` (for example, from another copy of a full app bundle).
- **Recreate.** The aesthetic and MediaPipe exports can be rebuilt with the recipes below.

ONNX Runtime itself is never downloaded by the app; see [ONNX Runtime](#onnx-runtime).

| File | Source / conversion | Terms |
| --- | --- | --- |
| `models/orientation.onnx` | Unmodified upstream `v2` release asset `orientation_model_v2_0.9882.onnx`; RGB 416 resize, center crop 384 | MIT; `licenses/ORIENTATION-MIT.txt` |
| `models/dinov2-small.onnx`, `models/dinov2-preprocessor.json` | `onnx-community/dinov2-small`, revision `8b1f705a3a7f6f062f6bdd21986c1583d3ef105d`, `onnx/model.onnx` | Apache-2.0 |
| `models/aesthetic.onnx` | `cafeai/cafe_aesthetic` (BEiT), revision `48a343764f786abc1ca8aaddfcd60a688a70da9b`; exported with `scripts/export_aesthetic.py` (opset 17), then folded with `examples/fold_model.rs` | AGPL-3.0; `licenses/CAFE-AESTHETIC-AGPL-3.0.txt`, model card in `licenses/cafe-aesthetic-model-card.md` |
| `models/face_detector.onnx`, `models/face_landmarks_detector.onnx` | MediaPipe face detector and Face Mesh V2 from `face_landmarker.task` (see below); `scripts/normalize_faces.py` folds float16 constants, then tflite2onnx 0.4.1 exports opset 11 | Apache-2.0 (MediaPipe) |
| `models/face_blendshapes.onnx` | MediaPipe `face_blendshapes.tflite` from the same `face_landmarker.task` (52 expression scores from 146 Face Mesh points); `scripts/convert_blendshapes.py` with TensorFlow 2.16.2 and tf2onnx 1.16.1, opset 17, explicit convolution pads; verified against TFLite within 3e-6 | Apache-2.0 (MediaPipe) |
| `models/face_recognition_sface.onnx` | OpenCV Zoo `face_recognition_sface_2021dec.onnx`, unmodified (see below) | Apache-2.0; `licenses/APACHE-2.0.txt` |
| `runtime/libonnxruntime.dylib` | ONNX Runtime 1.29.0 shared library (see below) | MIT; `licenses/ONNXRUNTIME-MIT.txt` |
| `locations.tsv` | 155,210 latitude/longitude/country/region/city records converted from reverse_geocode 1.6.6's GeoNames dataset | GeoNames CC BY 4.0, `licenses/GEONAMES-CC-BY-4.0.txt`; reverse_geocode package LGPL |
| `fonts/Inter-*.ttf` | Inter 4.001 (Regular, Medium, SemiBold) | SIL OFL 1.1; `licenses/INTER-OFL-1.1.txt` |
| `icons/app_icon.png` | PhotoSort's app icon, 1024×1024 RGBA; unchanged from the Python app's `assets/app_icon.png` (tag `python-final`). The app sets it as the window and Dock icon, and the macOS packager derives `AppIcon.icns` from it with `sips` and `iconutil` | Project artwork, Apache-2.0 with the source |

Links: [DINOv2 export](https://huggingface.co/onnx-community/dinov2-small/tree/8b1f705a3a7f6f062f6bdd21986c1583d3ef105d),
[BEiT aesthetic model](https://huggingface.co/cafeai/cafe_aesthetic/tree/48a343764f786abc1ca8aaddfcd60a688a70da9b),
[MediaPipe](https://github.com/google-ai-edge/mediapipe),
[Face Mesh model card](https://storage.googleapis.com/mediapipe-assets/Model%20Card%20MediaPipe%20Face%20Mesh%20V2.pdf),
[ONNX Runtime](https://github.com/microsoft/onnxruntime),
[GeoNames](https://www.geonames.org/),
[reverse_geocode](https://github.com/richardpenman/reverse_geocode),
[Inter](https://github.com/rsms/inter).

The application source license does not replace these assets' terms.

## Licensing notes for distributors

`scripts/package_macos.py` copies all of `resources/` into the app, so a full bundle ships
the aesthetic model. AGPL-3.0 then applies to that model file: ship its licence and model
card (both in `licenses/`) and offer the corresponding source of the shipped file, which
is the pinned upstream revision plus `scripts/export_aesthetic.py` and
`examples/fold_model.rs`. A release that cannot meet those terms should leave the model
out; Pick Best then ranks without aesthetic scores.

Default builds link Exiv2, which is GPL-2.0-or-later (see
[Native metadata library](#native-metadata-library)). Distributors should review the
obligations of every bundled native library, not only the application's Apache-2.0 source
license.

## Recreating model exports

Use a separate export environment with torch, transformers, onnx, numpy, flatbuffers,
tflite 2.18.0 and tflite2onnx 0.4.1. These are build tools, not application dependencies.

The MediaPipe models come from Google's Face Landmarker float16 bundle:

- Source: https://storage.googleapis.com/mediapipe-models/face_landmarker/face_landmarker/float16/latest/face_landmarker.task
- SHA-256: `64184e229b263107bc2b804c6625db1341ff2bb731874b0bcc2fe6544e0bc9ff` (3,758,596 bytes)

```sh
python scripts/export_aesthetic.py /path/to/pinned/beit/snapshot aesthetic-export.onnx
cargo run --release --example fold_model -- aesthetic-export.onnx resources/models/aesthetic.onnx
python scripts/normalize_faces.py /path/to/face_landmarker.task resources/models
python scripts/explicit_pads.py resources/models/face_landmarks_detector.onnx landmarks.onnx
mv landmarks.onnx resources/models/face_landmarks_detector.onnx
python scripts/convert_blendshapes.py /path/to/face_landmarker.task resources/models/face_blendshapes.onnx
```

`explicit_pads.py` gives pooling and convolution layers without padding explicit zero
pads, which Core ML requires, and only writes its output after ONNX Runtime returns
bit-identical results (needs `onnx`, `onnxruntime`). `convert_blendshapes.py` needs a
throwaway Python 3.12 environment with TensorFlow 2.16.2 and tf2onnx 1.16.1; it checks the
ONNX file against TFLite before writing it. tf2onnx names and orders generated nodes
differently on each run, so a rebuilt blendshape file behaves the same but has a different
checksum, which must then be re-pinned in `SHA256SUMS`.

Folding pre-computes the aesthetic export's constant subgraphs once. Without it, ONNX
Runtime does this at every first load and briefly needs about 3.5 GiB; with it the first
load peaks under 1 GiB and is about 7× faster. `fold_model` refuses to finish unless the
folded graph gives bit-identical outputs. The pinned checksum was produced with ONNX
Runtime 1.29.0 (export `6e6aa39e…`, 378,039,625 bytes → folded `862b59e3…`,
534,515,358 bytes); another runtime version may serialize different bytes, which then
needs a new checksum. Import only the folded file.

The face conversion converts constant half-precision buffers to exact float32 values and
removes their redundant DEQUANTIZE operators. It does not train or quantize the models.
The resulting ONNX graphs are NCHW. Detector inputs use [-1, 1]; Face Mesh inputs use
[0, 1].

## ONNX Runtime

Inference uses ONNX Runtime 1.29.0, loaded at runtime. The app looks for the library in
this order: `ORT_DYLIB_PATH`, the bundled `resources/runtime/` copy, then the system
locations `/opt/homebrew/lib/libonnxruntime.dylib` (macOS) and
`/usr/local/lib/libonnxruntime.so` (Linux). On Windows, place `onnxruntime.dll` in
`resources/runtime/` or point `ORT_DYLIB_PATH` at it. Official builds are published at
https://github.com/microsoft/onnxruntime/releases/tag/v1.29.0. Builds without the `onnx`
feature need no runtime and keep manual review only.

## Native metadata library

Default builds link Exiv2 0.28+ for embedded metadata. The macOS packager copies the
linked library and its dependencies into the app; no Exiv2 command-line tool runs at
runtime. Exiv2 is GPL-2.0-or-later; its COPYING and notice are kept as
`licenses/EXIV2-GPL-2.0.txt` and `licenses/EXIV2-NOTICE.txt`. Source and upstream terms:
https://exiv2.org/download.html. LibRaw and the other Homebrew libraries that the packager
bundles have their licence files collected into the app at packaging time.

## Person recognition model

`models/face_recognition_sface.onnx` is OpenCV Zoo's unmodified full-precision
`face_recognition_sface_2021dec.onnx` (MobileFaceNet/SFace), revision
`47534e27c9851bb1128ccc0102f1145e27f23f98`, 38,696,353 bytes,
SHA-256 `0ba9fbfa01b5270c96627c4ef784da859931e02f04419c829e83484087c34e79`.
[Upstream model and Apache-2.0 license](https://github.com/opencv/opencv_zoo/tree/47534e27c9851bb1128ccc0102f1145e27f23f98/models/face_recognition_sface).
Five-point alignment is implemented in Rust; the model consumes RGB 0..255 at 112×112.
The pinned file can be restored from
https://media.githubusercontent.com/media/opencv/opencv_zoo/47534e27c9851bb1128ccc0102f1145e27f23f98/models/face_recognition_sface/face_recognition_sface_2021dec.onnx.

## Orientation model

Fix Rotation downloads the unmodified
[v2 ONNX release](https://github.com/duartebarbosadev/deep-image-orientation-detection/releases/tag/v2)
through the same installer as DINOv2 and SFace. Release asset `272536092` is 80,578,424
bytes, SHA-256 `cffe911c1dff47fbfbbd90110aaab9c07134645c460d35b3ae8832079bea91ba`. A changed
asset at that URL fails the pinned content checks instead of silently changing inference.
The [upstream MIT notice](https://github.com/duartebarbosadev/deep-image-orientation-detection/blob/675bb69662bd65d58da8fa4b55eef87d8c3f859b/LICENSE)
is kept verbatim in `licenses/ORIENTATION-MIT.txt`.

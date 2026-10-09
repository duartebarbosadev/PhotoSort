# Asset provenance

These assets were prepared for this local working copy on 27 September 2026.
`SHA256SUMS` records exact content. Downloads are user-initiated through Menu → AI models. The app downloads only
the pinned native DINOv2, SFace and orientation ONNX files, streaming into private temporary
files and verifying exact size and SHA-256 before atomic installation. The other
prepared exports support bundled-copy restore and verified import. No Python
export/conversion tools run in the app, and runtime libraries are not downloaded.
Large ONNX weights and native runtime binaries are ignored by Git. Preserve or
restore these files when making a fresh checkout or distributing a local bundle.

| File | Source / conversion | Upstream terms |
| --- | --- | --- |
| `models/orientation.onnx` | Unmodified upstream `v2` release asset `orientation_model_v2_0.9882.onnx`; identical to the reference copy; RGB 416 resize, center crop 384 | MIT upstream repository; notice retained in `licenses/ORIENTATION-MIT.txt` |
| `models/dinov2-small.onnx` | `onnx-community/dinov2-small`, revision `8b1f705a3a7f6f062f6bdd21986c1583d3ef105d`, `onnx/model.onnx` | Apache-2.0 model repository |
| `models/aesthetic.onnx` | `cafeai/cafe_aesthetic`, revision `48a343764f786abc1ca8aaddfcd60a688a70da9b`; exported using `scripts/export_aesthetic.py`, opset 17, then folded with `examples/fold_model.rs` (ONNX Runtime basic, hardware-independent optimizations; outputs verified bit-identical) | AGPL-3.0 model card; original model card retained below |
| `models/face_detector.onnx`, `models/face_landmarks_detector.onnx` | Reference `assets/models/face_landmarker.task`, Google MediaPipe face detector and Face Mesh V2; `scripts/normalize_faces.py` folds float16 constants, then tflite2onnx 0.4.1 exports opset 11 | Google MediaPipe assets, Apache-2.0 project; model cards upstream |
| `runtime/libonnxruntime.dylib` | Native ONNX Runtime 1.29.0 library, copied from the reference environment; no Python runtime included | MIT |
| `locations.tsv` | 155,210 latitude/longitude/country/region/city records converted from reverse_geocode 1.6.6's GeoNames dataset | GeoNames CC BY 4.0; reverse_geocode package LGPL |

Links: [DINO export](https://huggingface.co/onnx-community/dinov2-small/tree/8b1f705a3a7f6f062f6bdd21986c1583d3ef105d),
[BeIT model](https://huggingface.co/cafeai/cafe_aesthetic/tree/48a343764f786abc1ca8aaddfcd60a688a70da9b),
[MediaPipe](https://github.com/google-ai-edge/mediapipe),
[Face Mesh model card](https://storage.googleapis.com/mediapipe-assets/Model%20Card%20MediaPipe%20Face%20Mesh%20V2.pdf),
[ONNX Runtime](https://github.com/microsoft/onnxruntime),
[GeoNames](https://www.geonames.org/),
[reverse_geocode](https://github.com/richardpenman/reverse_geocode).

The application source license does not replace these assets' terms. Packaging
here is a local development build; no public release or redistribution was done.

## Recreating model exports

Use a separate export environment with torch, transformers, onnx, numpy, flatbuffers,
tflite 2.18.0 and tflite2onnx 0.4.1. These are build tools, not application dependencies.

```
python scripts/export_aesthetic.py /path/to/pinned/beit/snapshot aesthetic-export.onnx
cargo run --release --example fold_model -- aesthetic-export.onnx resources/models/aesthetic.onnx
python scripts/normalize_faces.py /path/to/face_landmarker.task resources/models
```

Folding pre-computes the export's constant subgraphs once. Without it, ONNX
Runtime does this at every first load and briefly needs about 3.5 GiB; with it the
first load peaks under 1 GiB and loads about 7× faster. `fold_model` refuses to
finish unless the folded graph gives bit-identical outputs. The pinned checksum was
produced with the bundled ONNX Runtime 1.29.0 (export `6e6aa39e…`, 378,039,625 bytes
→ folded `862b59e3…`, 534,515,358 bytes); another runtime version may serialize
different bytes, which then needs a new checksum. Import only the folded file.

The face conversion first converts constant half-precision buffers to exact float32
values and removes their redundant DEQUANTIZE operators. It does not train or
quantize the models. The resulting ONNX graph is NCHW. Detector inputs use [-1, 1];
Face Mesh inputs use [0, 1]. Native runtime tests exercise both graph outputs.

## Native metadata library

Full builds added Exiv2 0.28.9 on 30 September 2026 through Homebrew's native
development package. The macOS packager copies the linked library and its
dependencies into the app, without invoking an Exiv2 CLI or Python at runtime.
Exiv2 is GPL-2.0-or-later; its COPYING and notice are retained as
`licenses/EXIV2-GPL-2.0.txt` and `licenses/EXIV2-NOTICE.txt`. Source and upstream
terms: https://exiv2.org/download.html . Existing source licensing does not
replace native-library redistribution obligations. This remains a local build.

## Native person recognition

`models/face_recognition_sface.onnx` is OpenCV Zoo's unmodified full-precision
`face_recognition_sface_2021dec.onnx` (MobileFaceNet/SFace), revision
`47534e27c9851bb1128ccc0102f1145e27f23f98`, 38,696,353 bytes.
SHA-256: `0ba9fbfa01b5270c96627c4ef784da859931e02f04419c829e83484087c34e79`.
[Upstream model and Apache-2.0 license](https://github.com/opencv/opencv_zoo/tree/47534e27c9851bb1128ccc0102f1145e27f23f98/models/face_recognition_sface).
The Apache license is retained in `licenses/APACHE-2.0.txt`. The app uses the
existing native ONNX owner; OpenCV and Python runtimes are not added. Five-point
alignment is implemented in Rust; the model consumes RGB 0..255 at 112 square.
Restore the exact pinned asset from
https://media.githubusercontent.com/media/opencv/opencv_zoo/47534e27c9851bb1128ccc0102f1145e27f23f98/models/face_recognition_sface/face_recognition_sface_2021dec.onnx
and verify SHA256SUMS before packaging.

## Native orientation model

Fix Rotation can download the unmodified [v2 ONNX release](https://github.com/duartebarbosadev/deep-image-orientation-detection/releases/tag/v2)
through the same shared installer as DINOv2 and SFace. Release asset `272536092`
is 80,578,424 bytes, SHA-256
`cffe911c1dff47fbfbbd90110aaab9c07134645c460d35b3ae8832079bea91ba`.
The published GitHub asset size and digest were verified on 8 October 2026;
these match the existing bundled model. A changed asset at that URL fails the
pinned content checks rather than silently changing inference behavior.

The [upstream MIT notice](https://github.com/duartebarbosadev/deep-image-orientation-detection/blob/675bb69662bd65d58da8fa4b55eef87d8c3f859b/LICENSE)
is retained verbatim. The release does not require a Python runtime or local
conversion. This provenance record does not establish PhotoSort accuracy across
arbitrary libraries; preprocessing and inference behavior remain unchanged.

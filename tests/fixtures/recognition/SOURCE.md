# Native recognition test fixture

Unmodified `models/face_recognition_sface/example_outputs/demo.jpg` from
OpenCV Zoo revision `47534e27c9851bb1128ccc0102f1145e27f23f98`.
https://github.com/opencv/opencv_zoo/tree/47534e27c9851bb1128ccc0102f1145e27f23f98/models/face_recognition_sface

Upstream licenses every file in this model directory under Apache-2.0.
See `resources/licenses/APACHE-2.0.txt` for the retained license.
SHA-256: `0f879881a598fea6fec74e047e6a1d00e36d81de63bf0ed392b628e6ab6c2fc4`.

The native opt-in test derives a portrait and its brightness variant, the distant
group scene, a repeated-person pair, a close crop of another person, and a
close-up two-person pair plus its brightness variant. It checks verified detection,
SFace inference, single-person and person-combination grouping, ambiguous repeated
identity handling, and restart reuse. The original distant scene produces no
verified faces with the current detector and remains unassigned. The second
close-up is enlarged from the source image; these derived/composited fixtures are
pipeline smoke tests, not representative photographic accuracy measurements.

"""Convert MediaPipe's face blendshape model (inside face_landmarker.task) to ONNX.

Build-time only. Use a throwaway Python 3.12 environment with the pinned tools:
    pip install tensorflow==2.16.2 tf2onnx==1.16.1 onnx onnxruntime "numpy<2"
Convolution pads are made explicit for Core ML (see explicit_pads.py). The ONNX
file is only written after it matches the TFLite model on random inputs.

Usage: python convert_blendshapes.py face_landmarker.task resources/models/face_blendshapes.onnx
"""

import os
import subprocess
import sys
import tempfile
import zipfile

import numpy as np
import onnx
import onnxruntime as ort
import tensorflow as tf

from explicit_pads import rewrite as explicit_pads


def main(task: str, target: str) -> None:
    # Stage beside the target so the final os.replace stays on one filesystem.
    folder = os.path.dirname(os.path.abspath(target))
    with tempfile.TemporaryDirectory(dir=folder, prefix=".face_blendshapes-") as work:
        tflite = os.path.join(work, "face_blendshapes.tflite")
        with zipfile.ZipFile(task) as archive, open(tflite, "wb") as out:
            out.write(archive.read("face_blendshapes.tflite"))
        staged = os.path.join(work, "face_blendshapes.onnx")
        # tf2onnx numbers and orders generated nodes differently on each run, so
        # equality with TFLite (below), not a byte checksum, proves a rebuild.
        subprocess.run(
            [sys.executable, "-m", "tf2onnx.convert", "--tflite", tflite,
             "--output", staged, "--opset", "17"],
            check=True,
        )
        model = onnx.load(staged)
        model.graph.doc_string = "converted from face_blendshapes.tflite"  # not the temp path
        explicit_pads(model)  # Core ML rejects convolutions without explicit pads
        onnx.save(model, staged)
        interpreter = tf.lite.Interpreter(tflite)
        interpreter.allocate_tensors()
        source_in = interpreter.get_input_details()[0]
        source_out = interpreter.get_output_details()[0]
        session = ort.InferenceSession(staged, providers=["CPUExecutionProvider"])
        rng = np.random.default_rng(0)
        worst = 0.0
        for _ in range(50):
            points = (rng.random(source_in["shape"]) * 1024).astype(np.float32)
            interpreter.set_tensor(source_in["index"], points)
            interpreter.invoke()
            expected = interpreter.get_tensor(source_out["index"]).reshape(-1)
            actual = session.run(None, {session.get_inputs()[0].name: points})[0].reshape(-1)
            worst = max(worst, float(np.abs(expected - actual).max()))
        assert worst < 1e-4, f"ONNX differs from TFLite by {worst}"
        os.replace(staged, target)
        print(f"wrote {target}; max |ONNX - TFLite| = {worst:.2e}")


if __name__ == "__main__":
    main(*sys.argv[1:3])

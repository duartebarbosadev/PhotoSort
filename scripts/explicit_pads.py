"""Rewrite pooling nodes that use auto_pad=VALID to the equivalent explicit zero pads.

VALID means "no padding", so this changes no arithmetic. Core ML's ONNX converter
requires explicit pads and otherwise sends the whole model to CPU. The output file
is only written after ONNX Runtime gives bit-identical results on fixed inputs.

Usage: python explicit_pool_pads.py input.onnx output.onnx
"""

import sys

import numpy as np
import onnx
import onnxruntime as ort
from onnx import helper


def rewrite(model: onnx.ModelProto) -> int:
    changed = 0
    for node in model.graph.node:
        if node.op_type not in ("MaxPool", "AveragePool"):
            continue
        auto_pad = next((a for a in node.attribute if a.name == "auto_pad"), None)
        if auto_pad is None or auto_pad.s != b"VALID":
            continue
        kernel = next(a for a in node.attribute if a.name == "kernel_shape").ints
        node.attribute.remove(auto_pad)
        node.attribute.append(helper.make_attribute("pads", [0] * (2 * len(kernel))))
        changed += 1
    return changed


def outputs(path: str, inputs: dict) -> list:
    session = ort.InferenceSession(path, providers=["CPUExecutionProvider"])
    return session.run(None, inputs)


def main(source: str, target: str) -> None:
    model = onnx.load(source)
    changed = rewrite(model)
    onnx.checker.check_model(model)
    shape = [d.dim_value for d in model.graph.input[0].type.tensor_type.shape.dim]
    rng = np.random.default_rng(0)
    staged = target + ".tmp"
    onnx.save(model, staged)
    for _ in range(5):
        feed = {model.graph.input[0].name: rng.random(shape, dtype=np.float32)}
        for before, after in zip(outputs(source, feed), outputs(staged, feed)):
            assert np.array_equal(before, after), "rewritten model output differs"
    import os

    os.replace(staged, target)
    print(f"{changed} pooling nodes rewritten; outputs bit-identical")


if __name__ == "__main__":
    main(*sys.argv[1:3])

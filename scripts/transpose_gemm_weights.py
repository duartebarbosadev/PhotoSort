"""Store Gemm weights pre-transposed (transB=1), so Core ML reads them from its weight file.

ONNX Runtime's Core ML converter turns Gemm into a `linear` op, which wants the weight
as [N, K]. With transB=0 it transposes B itself and writes the result inline into the
Core ML program. For the folded aesthetic model that is 60 matrices, about 340 MB, which
Core ML's compiler then expands into a 1.2 GB text file: slow to compile, slow to load
and large on disk. Storing B as [N, K] with transB=1 changes no arithmetic. The output
file is only written after ONNX Runtime gives bit-identical results on fixed inputs.

Usage: python transpose_gemm_weights.py input.onnx output.onnx [input-edge, default 384]
"""

import os
import sys

import numpy as np
import onnx
import onnxruntime as ort
from onnx import helper, numpy_helper


def rewrite(model: onnx.ModelProto) -> int:
    """Give every Gemm with a constant, untransposed B a transposed copy and transB=1."""
    initializers = {i.name: i for i in model.graph.initializer}
    uses: dict[str, int] = {}
    for node in model.graph.node:
        for name in node.input:
            uses[name] = uses.get(name, 0) + 1
    changed = 0
    for node in model.graph.node:
        if node.op_type != "Gemm" or node.input[1] not in initializers:
            continue
        attributes = {a.name: a for a in node.attribute}
        if attributes.get("transB") is not None and attributes["transB"].i != 0:
            continue
        source = initializers[node.input[1]]
        weight = numpy_helper.to_array(source)
        if weight.ndim != 2:
            continue
        transposed = numpy_helper.from_array(np.ascontiguousarray(weight.T), source.name + "_nk")
        if uses[source.name] == 1:
            model.graph.initializer.remove(source)
        model.graph.initializer.append(transposed)
        node.input[1] = transposed.name
        if "transB" in attributes:
            node.attribute.remove(attributes["transB"])
        node.attribute.append(helper.make_attribute("transB", 1))
        changed += 1
    return changed


def outputs(path: str, inputs: dict) -> list:
    session = ort.InferenceSession(path, providers=["CPUExecutionProvider"])
    return session.run(None, inputs)


def main(source: str, target: str, edge: str = "384") -> None:
    assert not os.path.exists(target), "Output must be a new file"
    model = onnx.load(source)
    changed = rewrite(model)
    onnx.checker.check_model(model, full_check=False)
    staged = target + ".tmp"
    onnx.save(model, staged)
    size = int(edge)
    rng = np.random.default_rng(0)
    for _ in range(5):
        feed = {model.graph.input[0].name: rng.random((1, 3, size, size), dtype=np.float32) * 2 - 1}
        for before, after in zip(outputs(source, feed), outputs(staged, feed)):
            assert np.array_equal(before, after), "rewritten model output differs"
    os.replace(staged, target)
    print(f"{changed} Gemm weights transposed; outputs bit-identical")


if __name__ == "__main__":
    main(*sys.argv[1:4])

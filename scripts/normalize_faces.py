"""Normalize constant float16 TFLite weights to float32 for tract.
Build-time only. Requires tflite, flatbuffers, numpy. Graph math is unchanged;
constant DEQUANTIZE nodes are folded and every consumer is rewired.
Usage: python scripts/normalize_faces.py face_landmarker.task resources/models
"""
import struct
import sys
import zipfile
from pathlib import Path
import numpy as np
import tflite

def normalize(source):
    data = bytearray(source)
    model = tflite.Model.GetRootAsModel(data, 0)
    converted = set()
    for gi in range(model.SubgraphsLength()):
        graph = model.Subgraphs(gi)
        constants = set()
        for ti in range(graph.TensorsLength()):
            tensor = graph.Tensors(ti)
            if tensor.Type() != tflite.TensorType.FLOAT16:
                continue
            buffer = model.Buffers(tensor.Buffer())
            assert buffer.DataLength(), 'Only constant float16 weights are supported'
            constants.add(ti)
            if tensor.Buffer() not in converted:
                values = np.frombuffer(bytes(buffer.DataAsNumpy()), dtype='<f2').astype('<f4').tobytes()
                data.extend(b'\0' * (-len(data) % 4))
                vector = len(data)
                data.extend(struct.pack('<I', len(values)) + values)
                pointer = buffer._tab.Pos + buffer._tab.Offset(4)
                struct.pack_into('<I', data, pointer, vector-pointer)
                converted.add(tensor.Buffer())
            data[tensor._tab.Pos + tensor._tab.Offset(6)] = tflite.TensorType.FLOAT32
        aliases = {}
        kept = []
        for oi in range(graph.OperatorsLength()):
            op = graph.Operators(oi)
            code = model.OperatorCodes(op.OpcodeIndex())
            opcode = max(code.DeprecatedBuiltinCode(), code.BuiltinCode())
            if opcode == tflite.BuiltinOperator.DEQUANTIZE:
                assert op.InputsLength() == 1 and op.Inputs(0) in constants
                aliases[op.Outputs(0)] = op.Inputs(0)
            else:
                kept.append(op)
        for op in kept:
            start = op._tab.Vector(op._tab.Offset(6))
            for i in range(op.InputsLength()):
                original = op.Inputs(i)
                if original in aliases:
                    struct.pack_into('<i', data, start+i*4, aliases[original])
        vector = graph._tab.Vector(graph._tab.Offset(10))
        struct.pack_into('<I', data, vector-4, len(kept))
        for i, op in enumerate(kept):
            struct.pack_into('<I', data, vector+i*4, op._tab.Pos-(vector+i*4))
    return data

if __name__ == '__main__':
    source, output = Path(sys.argv[1]), Path(sys.argv[2])
    output.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(source) as archive:
        for name in ('face_detector.tflite', 'face_landmarks_detector.tflite'):
            result = normalize(archive.read(name))
            (output/name).write_bytes(result)
            import tflite2onnx
            tflite2onnx.convert(str(output/name), str(output/Path(name).with_suffix('.onnx')))
            print(name, len(result))

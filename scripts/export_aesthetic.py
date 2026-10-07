"""One-time build tool: convert cached BeIT weights to ONNX; never used by the app.

Usage: python export_aesthetic.py /path/to/pinned/hf/snapshot output.onnx
Requires torch, transformers and onnx in the export environment only.
"""
from pathlib import Path
import sys
import torch
from transformers import AutoModelForImageClassification

torch.set_num_threads(2)
source, destination = Path(sys.argv[1]), Path(sys.argv[2])
model = AutoModelForImageClassification.from_pretrained(
    source, local_files_only=True, trust_remote_code=False, attn_implementation="eager"
).eval()

class Logits(torch.nn.Module):
    def __init__(self, model):
        super().__init__()
        self.model = model

    def forward(self, pixel_values):
        return self.model(pixel_values=pixel_values).logits

with torch.inference_mode():
    torch.onnx.export(
        Logits(model), (torch.zeros(1, 3, 384, 384),), str(destination),
        input_names=["pixel_values"], output_names=["logits"],
        opset_version=17, dynamo=False,
    )
print(f"Exported {destination} ({destination.stat().st_size:,} bytes)")

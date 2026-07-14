"""Frozen-application dependency verification used by release CI."""

from __future__ import annotations

import importlib
import json
import os
import platform
import sys
from pathlib import Path
from typing import Any


REQUIRED_PACKAGED_MODULES = (
    "PIL.Image",
    "cv2",
    "mediapipe.python.solutions.face_mesh",
    "onnxruntime",
    "openai",
    "pillow_heif",
    "pyexiv2",
    "rawpy",
    "reverse_geocode",
    "sklearn.cluster",
    "torch",
    "torchvision.transforms",
    "transformers.models.auto.image_processing_auto",
    "transformers.models.auto.modeling_auto",
    "transformers.models.beit.image_processing_beit",
    "transformers.models.beit.modeling_beit",
    "transformers.models.bit.image_processing_bit",
    "transformers.models.dinov2.modeling_dinov2",
    "ui.easy_delete_step_widget",
    "ui.fix_rotation_step_widget",
    "ui.metadata_sidebar",
    "ui.pick_best_step_widget",
    "workers.best_shot_worker",
    "workers.easy_delete_worker",
    "workers.grouping_worker",
    "workers.pick_best_worker",
    "workers.rotation_detection_step_worker",
)


def _write_report(report: dict[str, Any]) -> None:
    report_path = os.environ.get("PHOTOSORT_PACKAGING_SMOKE_REPORT", "").strip()
    serialized = json.dumps(report, indent=2, sort_keys=True)
    if report_path:
        destination = Path(report_path).expanduser().resolve()
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(serialized + "\n", encoding="utf-8")
    if sys.stdout is not None:
        print(serialized)


def run_packaging_smoke() -> int:
    """Import every deferred workflow/backend and return a process-safe status."""
    modules: dict[str, dict[str, str | bool]] = {}
    for module_name in REQUIRED_PACKAGED_MODULES:
        try:
            importlib.import_module(module_name)
            modules[module_name] = {"ok": True}
        except Exception as exc:  # noqa: BLE001 - report every frozen import failure
            modules[module_name] = {
                "ok": False,
                "error": f"{type(exc).__name__}: {exc}",
            }

    failures = [name for name, result in modules.items() if not result["ok"]]
    report: dict[str, Any] = {
        "ok": not failures,
        "failures": failures,
        "frozen": bool(getattr(sys, "frozen", False)),
        "platform": platform.platform(),
        "python": platform.python_version(),
        "modules": modules,
    }
    _write_report(report)
    return 0 if not failures else 1

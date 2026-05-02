from __future__ import annotations

from dataclasses import dataclass

from core.best_photo_finder.config import DevicePreference


@dataclass(slots=True)
class ResolvedDevice:
    requested: DevicePreference
    backend: str
    pipeline_device: str | int
    torch_dtype_name: str | None


def resolve_device(preference: DevicePreference) -> ResolvedDevice:
    try:
        import torch
    except ImportError:
        return ResolvedDevice(
            requested=preference,
            backend="cpu",
            pipeline_device="cpu",
            torch_dtype_name=None,
        )

    backend = "cpu"
    if preference == "cuda":
        backend = "cuda" if torch.cuda.is_available() else "cpu"
    elif preference == "mps":
        backend = (
            "mps"
            if getattr(torch.backends, "mps", None)
            and torch.backends.mps.is_available()
            else "cpu"
        )
    elif preference == "cpu":
        backend = "cpu"
    else:
        if torch.cuda.is_available():
            backend = "cuda"
        elif getattr(torch.backends, "mps", None) and torch.backends.mps.is_available():
            backend = "mps"

    pipeline_device: str | int
    torch_dtype_name: str | None
    if backend == "cuda":
        pipeline_device = 0
        torch_dtype_name = "float16"
    elif backend == "mps":
        pipeline_device = "mps"
        torch_dtype_name = "float16"
    else:
        pipeline_device = "cpu"
        torch_dtype_name = "float32"

    return ResolvedDevice(
        requested=preference,
        backend=backend,
        pipeline_device=pipeline_device,
        torch_dtype_name=torch_dtype_name,
    )

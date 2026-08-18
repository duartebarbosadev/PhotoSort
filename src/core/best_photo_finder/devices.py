from dataclasses import dataclass

from core.app_settings import get_preferred_torch_device
from core.best_photo_finder.config import DevicePreference


@dataclass(slots=True)
class ResolvedDevice:
    requested: DevicePreference
    backend: str
    pipeline_device: str | int
    torch_dtype_name: str | None


def resolve_device(preference: DevicePreference) -> ResolvedDevice:
    """Resolve one torch backend, honouring the app-wide device policy.

    ``get_preferred_torch_device`` is the single chooser for the whole app, so
    the ``PHOTOSORT_TORCH_DEVICE`` and ``PHOTOSORT_FORCE_CPU`` overrides apply
    here exactly as they do to similarity and Cull. An explicit non-auto
    preference is still honoured, but only when the hardware supports it.
    """

    available = get_preferred_torch_device()
    if preference in ("cuda", "mps"):
        backend = preference if preference == available else "cpu"
    elif preference == "cpu":
        backend = "cpu"
    else:
        backend = available

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

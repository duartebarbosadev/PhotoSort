"""Compatibility helpers for third-party packages expecting NumPy < 2.0.

Some dependencies (e.g. imgaug via pyiqa) still rely on ``np.sctypes`` which
was removed in NumPy 2.0.  Import this module early to reintroduce the
attribute so those packages do not crash at import-time.
"""

from __future__ import annotations

import numpy as np

__all__ = ["ensure_numpy_sctypes"]


def _collect_scalar_types(*type_names: str):
    """Return unique NumPy scalar types for the provided attribute names."""
    seen = set()
    result = []
    for name in type_names:
        attr = getattr(np, name, None)
        if attr is None or attr in seen:
            continue
        seen.add(attr)
        result.append(attr)
    return result


def ensure_numpy_sctypes() -> None:
    """Recreate ``np.sctypes`` when running on NumPy 2.0+."""
    if hasattr(np, "sctypes"):
        return

    np.sctypes = {  # type: ignore[attr-defined]
        "int": _collect_scalar_types(
            "byte",
            "short",
            "intc",
            "intp",
            "int_",
            "longlong",
        ),
        "uint": _collect_scalar_types(
            "ubyte",
            "ushort",
            "uintc",
            "uintp",
            "uint",
            "ulonglong",
        ),
        "float": _collect_scalar_types(
            "half",
            "single",
            "double",
            "longdouble",
            "float16",
            "float32",
            "float64",
        ),
        "complex": _collect_scalar_types(
            "csingle",
            "cdouble",
            "clongdouble",
            "complex64",
            "complex128",
        ),
        "others": _collect_scalar_types(
            "bool_",
            "bytes_",
            "str_",
            "void",
        ),
        "character": _collect_scalar_types(
            "bytes_",
            "str_",
        ),
    }

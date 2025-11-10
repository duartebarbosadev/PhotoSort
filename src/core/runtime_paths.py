"""Helpers for working with PyInstaller/runtime resource locations."""

from __future__ import annotations

import os
import sys
from typing import List, Optional


def is_frozen_runtime() -> bool:
    """Return True when running inside a PyInstaller bundle."""
    return bool(getattr(sys, "frozen", False) or getattr(sys, "_MEIPASS", None))


def resolve_runtime_root(fallback: Optional[str] = None) -> str:
    """Resolve the base directory for resource lookups.

    When frozen, prefer PyInstaller's extraction directory, otherwise the
    directory containing the executable. During source runs, fall back to the
    provided path (typically the project root) or the current working directory.
    """
    meipass = getattr(sys, "_MEIPASS", None)
    if meipass:
        return meipass
    if getattr(sys, "frozen", False):
        return os.path.dirname(sys.executable)
    if fallback:
        return fallback
    return os.getcwd()


def iter_bundle_roots(include_executable_dir: bool = False) -> List[str]:
    """Return candidate directories that may contain bundled resources."""
    locations: List[str] = []
    meipass = getattr(sys, "_MEIPASS", None)
    if meipass:
        locations.append(meipass)
    if include_executable_dir and getattr(sys, "frozen", False):
        locations.append(os.path.dirname(sys.executable))
    return locations

"""Helpers for working with PyInstaller/runtime resource locations."""

from __future__ import annotations

import os
import sys
import tempfile
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


_APP_NAME = "PhotoSort"


def _platform_cache_base() -> str:
    """Return the OS-appropriate user cache base directory."""
    if sys.platform == "darwin":
        return os.path.join(os.path.expanduser("~"), "Library", "Caches")
    if sys.platform == "win32":
        local_app_data = os.environ.get("LOCALAPPDATA", "")
        if local_app_data:
            return local_app_data
    # Linux / other: honour XDG_CACHE_HOME, fall back to ~/.cache
    xdg = os.environ.get("XDG_CACHE_HOME", "")
    if xdg:
        return xdg
    return os.path.join(os.path.expanduser("~"), ".cache")


def resolve_user_cache_dir(app_subdir: str) -> str:
    """Return a writable cache directory nested under the app's platform cache dir.

    All cache paths live under ``<platform_cache_base>/PhotoSort/<app_subdir>``
    so all PhotoSort data is grouped in one visible location rather than spread
    across multiple sibling directories.
    """
    candidates: List[str] = [
        os.path.join(_platform_cache_base(), _APP_NAME, app_subdir),
        os.path.join(tempfile.gettempdir(), _APP_NAME, app_subdir),
    ]

    for candidate in candidates:
        try:
            os.makedirs(candidate, exist_ok=True)
            with tempfile.NamedTemporaryFile(dir=candidate):
                pass
            return candidate
        except OSError:
            continue

    fallback = os.path.join(os.getcwd(), _APP_NAME, app_subdir)
    os.makedirs(fallback, exist_ok=True)
    return fallback


def get_app_cache_root() -> str:
    """Return the root PhotoSort cache directory (parent of all cache subdirs)."""
    return os.path.join(_platform_cache_base(), _APP_NAME)


def get_app_log_dir() -> str:
    """Return the OS-appropriate directory for PhotoSort log files.

    - macOS:   ~/Library/Logs/PhotoSort/
    - Windows: %LOCALAPPDATA%\\PhotoSort\\Logs\\
    - Linux:   $XDG_STATE_HOME/PhotoSort/  (default: ~/.local/state/PhotoSort/)
    """
    if sys.platform == "darwin":
        return os.path.join(os.path.expanduser("~"), "Library", "Logs", _APP_NAME)
    if sys.platform == "win32":
        local_app_data = os.environ.get("LOCALAPPDATA", "")
        if local_app_data:
            return os.path.join(local_app_data, _APP_NAME, "Logs")
    # Linux / other: XDG_STATE_HOME is the spec-correct location for logs
    xdg_state = os.environ.get("XDG_STATE_HOME", "")
    if xdg_state:
        return os.path.join(xdg_state, _APP_NAME)
    return os.path.join(os.path.expanduser("~"), ".local", "state", _APP_NAME)

"""Chronological ordering shared by every workflow that lists media.

Photos and videos are shown in the order they were taken. The capture date is
resolved from, in order of preference:

1. The metadata date (EXIF/XMP capture date, QuickTime creation date for
   videos, or a date in the filename) once background metadata has loaded.
2. The file's creation time recorded by the scanner.
3. The file's modification time recorded by the scanner.

Media without any date sorts after dated media. Ties are broken by filename so
the order is stable and predictable.
"""

from __future__ import annotations

import os
from collections.abc import Mapping
from datetime import datetime
from typing import Any

CaptureOrderKey = tuple[int, float, str, str]

_UNDATED_TIMESTAMP = float("inf")


def _timestamp_from_ns(value: Any) -> float | None:
    try:
        if value:
            return float(value) / 1_000_000_000
    except TypeError, ValueError:
        return None
    return None


def file_record_timestamp(file_data: Mapping[str, Any] | None) -> float | None:
    """Return the scanner-recorded creation or modification time, if any."""

    if not isinstance(file_data, Mapping):
        return None
    timestamp = _timestamp_from_ns(file_data.get("birthtime_ns"))
    if timestamp is not None:
        return timestamp
    timestamp = _timestamp_from_ns(file_data.get("mtime_ns"))
    if timestamp is not None:
        return timestamp
    try:
        mtime = file_data.get("mtime")
        return float(mtime) if mtime else None
    except TypeError, ValueError:
        return None


def capture_datetime(
    path: str,
    date_cache: Mapping[str, datetime | None],
    file_data: Mapping[str, Any] | None = None,
) -> datetime | None:
    """Return the best known capture date without touching the filesystem."""

    cached = date_cache.get(path)
    if cached is not None:
        return cached
    timestamp = file_record_timestamp(file_data)
    if timestamp is None:
        return None
    try:
        return datetime.fromtimestamp(timestamp)
    except OverflowError, OSError, ValueError:
        return None


def capture_order_key(
    path: str,
    date_cache: Mapping[str, datetime | None],
    file_data: Mapping[str, Any] | None = None,
) -> CaptureOrderKey:
    """Return a sort key that orders media by capture date, oldest first."""

    name = os.path.basename(path).lower()
    resolved = capture_datetime(path, date_cache, file_data)
    if resolved is None:
        return (1, _UNDATED_TIMESTAMP, name, path)
    try:
        timestamp = resolved.timestamp()
    except OverflowError, OSError, ValueError:
        return (1, _UNDATED_TIMESTAMP, name, path)
    return (0, timestamp, name, path)


def filename_order_key(path: str) -> CaptureOrderKey:
    """Fallback key for components used before capture dates are available."""

    return capture_order_key(path, {})

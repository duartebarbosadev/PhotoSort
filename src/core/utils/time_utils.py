"""Time-related helper utilities shared across workers."""

from __future__ import annotations

import math


def format_duration(seconds: float) -> str:
    """
    Return a compact human-readable duration string like '1h 05m 12s'.
    Values that are NaN/inf or negative yield an empty string.
    """
    if not math.isfinite(seconds):
        return ""
    seconds = max(0, int(round(seconds)))
    hours, remainder = divmod(seconds, 3600)
    minutes, secs = divmod(remainder, 60)
    parts: list[str] = []
    if hours:
        parts.append(f"{hours}h")
    if minutes or hours:
        parts.append(f"{minutes}m")
    if secs or not parts:
        parts.append(f"{secs}s")
    return " ".join(parts)


__all__ = ["format_duration"]

from __future__ import annotations

import os
from dataclasses import dataclass
from typing import Any, Optional


@dataclass
class StatusBarInfo:
    filename: str
    rating: int
    date_text: str
    cluster_id: Optional[int]
    size_kb: Optional[int]
    width: int
    height: int
    is_blurred: Optional[bool]

    def to_message(self) -> str:
        cluster_part = f" | C: {self.cluster_id}" if self.cluster_id is not None else ""
        size_part = f" | Size: {self.size_kb} KB" if self.size_kb is not None else " | Size: N/A"
        blur_part = (
            " | Blurred: Yes"
            if self.is_blurred is True
            else (" | Blurred: No" if self.is_blurred is False else "")
        )
        return (
            f"{self.filename} | R: {self.rating} | {self.date_text}" f"{cluster_part}{size_part} | {self.width}x{self.height}{blur_part}"
        )


def build_status_bar_info(
    file_path: str,
    metadata: dict[str, Any],
    width: int,
    height: int,
    cluster_lookup: dict[str, int] | None = None,
    file_data_from_model: Optional[dict[str, Any]] = None,
) -> StatusBarInfo:
    filename = os.path.basename(file_path)
    rating = int(metadata.get("rating", 0) or 0)
    date_obj = metadata.get("date")
    date_text = f"D: {date_obj.strftime('%Y-%m-%d')}" if date_obj else "D: Unknown"
    cluster_id = None
    if cluster_lookup and file_path in cluster_lookup:
        cluster_id = cluster_lookup[file_path]
    try:
        size_kb = os.path.getsize(file_path) // 1024
    except OSError:
        size_kb = None
    is_blurred = file_data_from_model.get("is_blurred") if file_data_from_model else None
    return StatusBarInfo(
        filename=filename,
        rating=rating,
        date_text=date_text,
        cluster_id=cluster_id,
        size_kb=size_kb,
        width=width,
        height=height,
        is_blurred=is_blurred,
    )

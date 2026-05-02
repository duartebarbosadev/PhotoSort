from __future__ import annotations

from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Literal

DevicePreference = Literal["auto", "cpu", "mps", "cuda"]


@dataclass(slots=True)
class SelectorConfig:
    blur_threshold: float = 110.0
    blur_penalty_weight: float = 0.35
    eye_closed_threshold: float = 0.21
    eye_penalty_weight: float = 0.30
    thumbnail_size: int = 384
    aesthetic_batch_size: int = 8
    tie_threshold: float = 0.002
    device: DevicePreference = "auto"
    supported_extensions: tuple[str, ...] = (
        ".jpg",
        ".jpeg",
        ".png",
        ".webp",
        ".bmp",
        ".tif",
        ".tiff",
    )
    output_path: Path | None = None
    verbose: bool = False

    def to_dict(self) -> dict[str, object]:
        payload = asdict(self)
        if self.output_path is not None:
            payload["output_path"] = str(self.output_path)
        return payload

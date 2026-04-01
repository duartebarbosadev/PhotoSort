from __future__ import annotations

from dataclasses import asdict, dataclass, field
import json
from typing import Any, Literal


@dataclass(slots=True)
class TechnicalMetrics:
    blur_variance: float
    blur_penalty: float
    face_count: int
    closed_face_count: int
    eye_penalty: float
    max_face_area_ratio: float
    image_width: int
    image_height: int
    issues: tuple[str, ...] = ()

    @property
    def pixel_count(self) -> int:
        return self.image_width * self.image_height


@dataclass(slots=True)
class ImageScore:
    path: str
    status: Literal["scored", "failed"] = "scored"
    blur_variance: float | None = None
    blur_penalty: float = 0.0
    face_count: int = 0
    closed_face_count: int = 0
    eye_penalty: float = 0.0
    technical_penalty: float = 0.0
    aesthetic_score: float | None = None
    final_score: float | None = None
    max_face_area_ratio: float = 0.0
    image_width: int | None = None
    image_height: int | None = None
    issues: tuple[str, ...] = ()
    failure_reason: str | None = None

    @property
    def pixel_count(self) -> int:
        if self.image_width is None or self.image_height is None:
            return 0
        return self.image_width * self.image_height

    def to_dict(self) -> dict[str, Any]:
        return asdict(self)


@dataclass(slots=True)
class SelectionResult:
    winner: ImageScore
    ranked_images: list[ImageScore]
    failed_images: list[ImageScore] = field(default_factory=list)
    config: dict[str, object] = field(default_factory=dict)
    device_used: str = "unknown"
    model_name: str | None = None

    def to_dict(self) -> dict[str, Any]:
        payload = {
            "winner": self.winner.to_dict(),
            "ranked_images": [image.to_dict() for image in self.ranked_images],
            "failed_images": [image.to_dict() for image in self.failed_images],
            "config": self.config,
            "device_used": self.device_used,
            "model_name": self.model_name,
        }
        return payload

    def to_json(self, *, indent: int = 2) -> str:
        return json.dumps(self.to_dict(), indent=indent)

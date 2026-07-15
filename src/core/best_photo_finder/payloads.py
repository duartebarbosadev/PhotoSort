from typing import Literal, NotRequired, TypedDict


class ImageScorePayload(TypedDict):
    """Serialized image score shared by the scorer worker and Pick Best UI."""

    path: str
    status: NotRequired[Literal["scored", "failed"]]
    blur_variance: NotRequired[float | None]
    blur_penalty: NotRequired[float]
    face_count: NotRequired[int]
    closed_face_count: NotRequired[int]
    eye_penalty: NotRequired[float]
    technical_penalty: NotRequired[float]
    aesthetic_score: NotRequired[float | None]
    final_score: NotRequired[float | None]
    max_face_area_ratio: NotRequired[float]
    image_width: NotRequired[int | None]
    image_height: NotRequired[int | None]
    issues: NotRequired[tuple[str, ...]]
    failure_reason: NotRequired[str | None]


class PickBestClusterResult(TypedDict):
    """One cluster's complete Pick Best result payload."""

    winner_path: str | None
    ranked: list[ImageScorePayload]
    failed: list[ImageScorePayload]
    all_paths: list[str]
    unsupported_paths: list[str]
    _mark_state: NotRequired[dict[str, bool]]


type PickBestResults = dict[int, PickBestClusterResult]

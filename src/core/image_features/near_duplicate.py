"""Conservative, subject-aware assessment of near-duplicate photographs."""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass, field
from enum import Enum

import cv2
import numpy as np

from core.image_features.face_analysis import FaceDescriptor, SubjectDescriptor
from core.image_features.structural_similarity import (
    aligned_structural_similarity,
    prepare_same_frame_preview,
)


NEAR_DUPLICATE_ALGORITHM_VERSION = "subject-safe-v1"

CHANGE_ABSOLUTE_FLOOR = 10.0 / 255.0
CHANGE_MAD_MULTIPLIER = 6.0
CHANGE_COMPONENT_MIN_FRACTION = 0.0005
CHANGE_COMPONENT_MIN_PIXELS = 48
CHANGE_COMPONENT_MIN_P90 = 16.0 / 255.0
CHANGE_COMBINED_MIN_FRACTION = 0.0015
MAX_ALIGNMENT_FRACTION = 0.04
FACE_MIN_IOU = 0.60
FACE_LANDMARK_MEDIAN_LIMIT = 0.018
FACE_LANDMARK_P90_LIMIT = 0.040
FACE_CROP_MIN_SSIM = 0.985
SAME_FRAME_MIN_SSIM = 0.98


class NearDuplicateDecision(Enum):
    EXACT_DUPLICATE = "exact_duplicate"
    SAFE_NEAR_DUPLICATE = "safe_near_duplicate"
    SUBJECT_CHANGED = "subject_changed"
    UNCERTAIN = "uncertain"


@dataclass(frozen=True, slots=True)
class CoherentChangeMetrics:
    threshold: float
    largest_component_fraction: float
    coherent_fraction: float
    largest_component_p90: float
    meaningful_change: bool


@dataclass(frozen=True, slots=True)
class NearDuplicateAssessment:
    decision: NearDuplicateDecision
    algorithm_version: str = NEAR_DUPLICATE_ALGORITHM_VERSION
    structural_similarity: float | None = None
    change: CoherentChangeMetrics | None = None
    face_count_a: int | None = None
    face_count_b: int | None = None
    face_min_iou: float | None = None
    face_max_median_displacement: float | None = None
    face_max_p90_displacement: float | None = None
    face_min_crop_similarity: float | None = None
    detail: str = ""
    metrics: dict[str, float | int | bool | None] = field(default_factory=dict)

    @property
    def accepted(self) -> bool:
        return self.decision in {
            NearDuplicateDecision.EXACT_DUPLICATE,
            NearDuplicateDecision.SAFE_NEAR_DUPLICATE,
        }

    def result_metrics(self) -> dict[str, object]:
        values: dict[str, object] = {
            "assessment_decision": self.decision.value,
            "assessment_version": self.algorithm_version,
            "assessment_detail": self.detail,
            "structural_similarity": self.structural_similarity,
            "face_count_a": self.face_count_a,
            "face_count_b": self.face_count_b,
            "face_min_iou": self.face_min_iou,
            "face_max_median_displacement": self.face_max_median_displacement,
            "face_max_p90_displacement": self.face_max_p90_displacement,
            "face_min_crop_similarity": self.face_min_crop_similarity,
        }
        if self.change is not None:
            values.update(
                {
                    "change_threshold": self.change.threshold,
                    "change_largest_component_fraction": (
                        self.change.largest_component_fraction
                    ),
                    "change_coherent_fraction": self.change.coherent_fraction,
                    "change_largest_component_p90": (
                        self.change.largest_component_p90
                    ),
                }
            )
        values.update(self.metrics)
        return values


class SubjectSafeNearDuplicateComparator:
    """Require same framing, no coherent subject change, and stable faces."""

    def __init__(self, should_stop: Callable[[], bool] | None = None) -> None:
        self._should_stop = should_stop or (lambda: False)

    def assess(
        self,
        path_a: str,
        path_b: str,
        fingerprint_a: tuple[int, int] | None,
        fingerprint_b: tuple[int, int] | None,
        image_a: np.ndarray | None,
        image_b: np.ndarray | None,
        *,
        descriptor_a: SubjectDescriptor | None = None,
        descriptor_b: SubjectDescriptor | None = None,
        identical: bool = False,
    ) -> NearDuplicateAssessment:
        del path_a, path_b, fingerprint_a, fingerprint_b
        if identical:
            return NearDuplicateAssessment(
                NearDuplicateDecision.EXACT_DUPLICATE,
                detail="byte-for-byte identical",
            )
        if self._should_stop():
            return NearDuplicateAssessment(
                NearDuplicateDecision.UNCERTAIN, detail="cancelled"
            )
        prepared = _prepare_aligned_pair(image_a, image_b)
        if prepared is None:
            return NearDuplicateAssessment(
                NearDuplicateDecision.UNCERTAIN,
                detail="images could not be aligned safely",
            )
        first_gray, aligned_second, shift_x, shift_y = prepared
        structural = aligned_structural_similarity(
            prepare_same_frame_preview(first_gray * 255.0),
            prepare_same_frame_preview(aligned_second * 255.0),
        )
        if structural is None:
            return NearDuplicateAssessment(
                NearDuplicateDecision.UNCERTAIN,
                detail="structural similarity was unavailable",
            )
        if structural < SAME_FRAME_MIN_SSIM:
            return NearDuplicateAssessment(
                NearDuplicateDecision.SUBJECT_CHANGED,
                structural_similarity=structural,
                detail="framing or subject structure changed",
            )
        if self._should_stop():
            return NearDuplicateAssessment(
                NearDuplicateDecision.UNCERTAIN,
                structural_similarity=structural,
                detail="cancelled",
            )
        change = coherent_change_metrics(first_gray, aligned_second)
        if change is None:
            return NearDuplicateAssessment(
                NearDuplicateDecision.UNCERTAIN,
                structural_similarity=structural,
                detail="localized change measurement was unstable",
            )
        if change.meaningful_change:
            return NearDuplicateAssessment(
                NearDuplicateDecision.SUBJECT_CHANGED,
                structural_similarity=structural,
                change=change,
                detail="coherent foreground or subject change detected",
            )
        if self._should_stop():
            return NearDuplicateAssessment(
                NearDuplicateDecision.UNCERTAIN,
                structural_similarity=structural,
                change=change,
                detail="cancelled",
            )
        if descriptor_a is None or descriptor_b is None:
            return NearDuplicateAssessment(
                NearDuplicateDecision.UNCERTAIN,
                structural_similarity=structural,
                change=change,
                detail="face analysis was unavailable",
            )
        face_result = _compare_faces(
            descriptor_a,
            descriptor_b,
            first_gray,
            aligned_second,
            shift_x,
            shift_y,
        )
        return NearDuplicateAssessment(
            face_result["decision"],
            structural_similarity=structural,
            change=change,
            face_count_a=len(descriptor_a.faces),
            face_count_b=len(descriptor_b.faces),
            face_min_iou=face_result["min_iou"],
            face_max_median_displacement=face_result["max_median"],
            face_max_p90_displacement=face_result["max_p90"],
            face_min_crop_similarity=face_result["min_crop_ssim"],
            detail=face_result["detail"],
        )


def coherent_change_metrics(
    first: np.ndarray, aligned_second: np.ndarray
) -> CoherentChangeMetrics | None:
    if first.shape != aligned_second.shape or first.size == 0:
        return None
    first = np.asarray(first, dtype=np.float32)
    second = np.asarray(aligned_second, dtype=np.float32)
    if not np.isfinite(first).all() or not np.isfinite(second).all():
        return None

    corrected = _exposure_correct(first, second)
    if corrected is None:
        return None
    difference = np.abs(first - corrected)
    median = float(np.median(difference))
    normalized_mad = float(1.4826 * np.median(np.abs(difference - median)))
    threshold = max(
        CHANGE_ABSOLUTE_FLOOR,
        median + CHANGE_MAD_MULTIPLIER * normalized_mad,
    )
    mask = np.asarray(difference >= threshold, dtype=np.uint8) * 255
    mask = cv2.morphologyEx(
        mask, cv2.MORPH_OPEN, np.ones((3, 3), dtype=np.uint8)
    )
    mask = cv2.morphologyEx(
        mask, cv2.MORPH_CLOSE, np.ones((5, 5), dtype=np.uint8)
    )

    count, labels, stats, _centroids = cv2.connectedComponentsWithStats(mask, 8)
    image_area = int(difference.size)
    minimum_area = max(
        CHANGE_COMPONENT_MIN_PIXELS,
        int(np.ceil(image_area * CHANGE_COMPONENT_MIN_FRACTION)),
    )
    coherent_area = 0
    largest_area = 0
    largest_p90 = 0.0
    significant_component = False
    for label in range(1, count):
        area = int(stats[label, cv2.CC_STAT_AREA])
        if area <= 0:
            continue
        values = difference[labels == label]
        p90 = float(np.quantile(values, 0.90)) if values.size else 0.0
        if area >= minimum_area:
            coherent_area += area
            if area > largest_area:
                largest_area = area
                largest_p90 = p90
            if p90 >= CHANGE_COMPONENT_MIN_P90:
                significant_component = True
    coherent_fraction = coherent_area / max(image_area, 1)
    meaningful = (
        significant_component
        or coherent_fraction >= CHANGE_COMBINED_MIN_FRACTION
    )
    return CoherentChangeMetrics(
        threshold=threshold,
        largest_component_fraction=largest_area / max(image_area, 1),
        coherent_fraction=coherent_fraction,
        largest_component_p90=largest_p90,
        meaningful_change=meaningful,
    )


def _prepare_aligned_pair(
    image_a: np.ndarray | None, image_b: np.ndarray | None
) -> tuple[np.ndarray, np.ndarray, float, float] | None:
    if image_a is None or image_b is None:
        return None
    first = _to_gray_unit(image_a)
    second = _to_gray_unit(image_b)
    if first is None or second is None or first.shape != second.shape:
        return None
    try:
        (shift_x, shift_y), response = cv2.phaseCorrelate(first, second)
    except cv2.error:
        return None
    if not np.isfinite((shift_x, shift_y, response)).all() or response <= 0.0:
        return None
    height, width = first.shape
    if (
        abs(shift_x) > width * MAX_ALIGNMENT_FRACTION
        or abs(shift_y) > height * MAX_ALIGNMENT_FRACTION
    ):
        return None
    transform = np.float32([[1.0, 0.0, shift_x], [0.0, 1.0, shift_y]])
    aligned = cv2.warpAffine(
        second,
        transform,
        (width, height),
        flags=cv2.INTER_LINEAR | cv2.WARP_INVERSE_MAP,
        borderMode=cv2.BORDER_REFLECT,
    )
    return first, aligned, float(shift_x), float(shift_y)


def _to_gray_unit(image: np.ndarray) -> np.ndarray | None:
    array = np.asarray(image)
    if array.size == 0:
        return None
    if array.ndim == 3:
        if array.shape[2] < 3:
            return None
        array = cv2.cvtColor(array[:, :, :3], cv2.COLOR_RGB2GRAY)
    if array.ndim != 2:
        return None
    result = np.asarray(array, dtype=np.float32)
    if result.max(initial=0.0) > 1.5:
        result /= 255.0
    return np.clip(result, 0.0, 1.0)


def _exposure_correct(
    target: np.ndarray, source: np.ndarray
) -> np.ndarray | None:
    low, high = np.percentile(source, (2.0, 98.0))
    mask = (source > low) & (source < high)
    values = source[mask]
    targets = target[mask]
    if values.size < 64:
        return None
    source_mean = float(values.mean())
    target_mean = float(targets.mean())
    centered = values - source_mean
    variance = float(np.dot(centered, centered))
    if variance <= np.finfo(np.float32).eps:
        # Flat identical frames remain comparable through a simple offset.
        return np.clip(source + (target_mean - source_mean), 0.0, 1.0)
    gain = float(np.dot(centered, targets - target_mean) / variance)
    if not np.isfinite(gain) or gain < 0.5 or gain > 2.0:
        return None
    offset = target_mean - gain * source_mean
    if not np.isfinite(offset) or abs(offset) > 0.25:
        return None
    return np.clip(source * gain + offset, 0.0, 1.0)


def _compare_faces(
    first: SubjectDescriptor,
    second: SubjectDescriptor,
    first_gray: np.ndarray,
    aligned_second: np.ndarray,
    shift_x: float,
    shift_y: float,
) -> dict[str, object]:
    if len(first.faces) != len(second.faces):
        return _face_result(
            NearDuplicateDecision.SUBJECT_CHANGED,
            detail="face count changed",
        )
    if not first.faces:
        return _face_result(
            NearDuplicateDecision.SAFE_NEAR_DUPLICATE,
            detail="same framing with no coherent subject change",
        )

    height, width = first_gray.shape
    adjusted_second = [
        _shift_face(face, -shift_x / width, -shift_y / height)
        for face in second.faces
    ]
    remaining = set(range(len(adjusted_second)))
    matches: list[tuple[FaceDescriptor, FaceDescriptor, float]] = []
    for face_a in sorted(first.faces, key=lambda face: face.bbox[0]):
        scored = [
            (_bbox_iou(face_a.bbox, adjusted_second[index].bbox), index)
            for index in remaining
        ]
        if not scored:
            return _face_result(
                NearDuplicateDecision.SUBJECT_CHANGED,
                detail="a face could not be matched",
            )
        iou, index = max(scored)
        remaining.remove(index)
        matches.append((face_a, adjusted_second[index], iou))

    min_iou = min(match[2] for match in matches)
    if min_iou < FACE_MIN_IOU:
        return _face_result(
            NearDuplicateDecision.SUBJECT_CHANGED,
            min_iou=min_iou,
            detail="face position or size changed",
        )

    max_median = 0.0
    max_p90 = 0.0
    min_crop_ssim = 1.0
    for face_a, face_b, _iou in matches:
        displacement = _landmark_displacement(face_a, face_b)
        if displacement is None:
            return _face_result(
                NearDuplicateDecision.UNCERTAIN,
                min_iou=min_iou,
                detail="face landmarks were incompatible",
            )
        median, p90 = displacement
        max_median = max(max_median, median)
        max_p90 = max(max_p90, p90)
        crop_similarity = _face_crop_similarity(
            first_gray, aligned_second, face_a.bbox
        )
        if crop_similarity is None:
            return _face_result(
                NearDuplicateDecision.UNCERTAIN,
                min_iou=min_iou,
                max_median=max_median,
                max_p90=max_p90,
                detail="face crop was too small to compare",
            )
        min_crop_ssim = min(min_crop_ssim, crop_similarity)

    changed = (
        max_median > FACE_LANDMARK_MEDIAN_LIMIT
        or max_p90 > FACE_LANDMARK_P90_LIMIT
        or min_crop_ssim < FACE_CROP_MIN_SSIM
    )
    return _face_result(
        (
            NearDuplicateDecision.SUBJECT_CHANGED
            if changed
            else NearDuplicateDecision.SAFE_NEAR_DUPLICATE
        ),
        min_iou=min_iou,
        max_median=max_median,
        max_p90=max_p90,
        min_crop_ssim=min_crop_ssim,
        detail=(
            "face, expression, or head pose changed"
            if changed
            else "same framing with stable subjects and faces"
        ),
    )


def _face_result(
    decision: NearDuplicateDecision,
    *,
    min_iou: float | None = None,
    max_median: float | None = None,
    max_p90: float | None = None,
    min_crop_ssim: float | None = None,
    detail: str,
) -> dict[str, object]:
    return {
        "decision": decision,
        "min_iou": min_iou,
        "max_median": max_median,
        "max_p90": max_p90,
        "min_crop_ssim": min_crop_ssim,
        "detail": detail,
    }


def _shift_face(face: FaceDescriptor, dx: float, dy: float) -> FaceDescriptor:
    x0, y0, x1, y1 = face.bbox
    return FaceDescriptor(
        bbox=(x0 + dx, y0 + dy, x1 + dx, y1 + dy),
        landmarks=tuple((x + dx, y + dy) for x, y in face.landmarks),
    )


def _bbox_iou(
    first: tuple[float, float, float, float],
    second: tuple[float, float, float, float],
) -> float:
    x0 = max(first[0], second[0])
    y0 = max(first[1], second[1])
    x1 = min(first[2], second[2])
    y1 = min(first[3], second[3])
    intersection = max(0.0, x1 - x0) * max(0.0, y1 - y0)
    first_area = max(0.0, first[2] - first[0]) * max(0.0, first[3] - first[1])
    second_area = max(0.0, second[2] - second[0]) * max(
        0.0, second[3] - second[1]
    )
    return intersection / max(first_area + second_area - intersection, 1e-12)


def _landmark_displacement(
    first: FaceDescriptor, second: FaceDescriptor
) -> tuple[float, float] | None:
    if len(first.landmarks) != len(second.landmarks) or not first.landmarks:
        return None
    first_points = np.asarray(first.landmarks, dtype=np.float64)
    second_points = np.asarray(second.landmarks, dtype=np.float64)
    first_scale = np.hypot(
        first.bbox[2] - first.bbox[0], first.bbox[3] - first.bbox[1]
    )
    second_scale = np.hypot(
        second.bbox[2] - second.bbox[0], second.bbox[3] - second.bbox[1]
    )
    if min(first_scale, second_scale) <= 1e-8:
        return None
    first_normalized = (
        first_points - np.asarray(first.bbox[:2], dtype=np.float64)
    ) / first_scale
    second_normalized = (
        second_points - np.asarray(second.bbox[:2], dtype=np.float64)
    ) / second_scale
    first_centered = first_normalized - first_normalized.mean(axis=0)
    second_centered = second_normalized - second_normalized.mean(axis=0)
    try:
        u, _singular, vt = np.linalg.svd(second_centered.T @ first_centered)
    except np.linalg.LinAlgError:
        return None
    rotation = u @ vt
    aligned_second = second_centered @ rotation
    distances = np.linalg.norm(first_centered - aligned_second, axis=1)
    return float(np.median(distances)), float(np.quantile(distances, 0.90))


def _face_crop_similarity(
    first: np.ndarray,
    second: np.ndarray,
    bbox: tuple[float, float, float, float],
) -> float | None:
    height, width = first.shape
    x0, y0, x1, y1 = bbox
    padding_x = (x1 - x0) * 0.25
    padding_y = (y1 - y0) * 0.25
    left = max(0, int(np.floor((x0 - padding_x) * width)))
    top = max(0, int(np.floor((y0 - padding_y) * height)))
    right = min(width, int(np.ceil((x1 + padding_x) * width)))
    bottom = min(height, int(np.ceil((y1 + padding_y) * height)))
    if right - left < 16 or bottom - top < 16:
        return None
    first_crop = first[top:bottom, left:right] * 255.0
    second_crop = second[top:bottom, left:right] * 255.0
    return aligned_structural_similarity(
        prepare_same_frame_preview(first_crop),
        prepare_same_frame_preview(second_crop),
    )

"""Conservative, subject-aware assessment of near-duplicate photographs."""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass, field
from enum import Enum
import math
import time

import cv2
import numpy as np

from core.image_features.face_analysis import FaceDescriptor, SubjectDescriptor
from core.image_features.structural_similarity import (
    prepare_same_frame_preview,
    structural_similarity_for_aligned,
)


NEAR_DUPLICATE_ALGORITHM_VERSION = "normal-view-subject-safe-v2"

CHANGE_ABSOLUTE_FLOOR = 10.0 / 255.0
CHANGE_MAD_MULTIPLIER = 6.0
CHANGE_COMPONENT_MIN_FRACTION = 0.0005
CHANGE_COMPONENT_MIN_PIXELS = 48
CHANGE_COMPONENT_MIN_P90 = 16.0 / 255.0
CHANGE_COMBINED_MIN_FRACTION = 0.0015
NORMAL_VIEW_LONG_EDGE = 384
NORMAL_VIEW_BLUR_SIGMA = 0.6
NORMAL_VIEW_COMPONENT_MIN_FRACTION = 0.0008
NORMAL_VIEW_COMPONENT_MIN_PIXELS = 24
NORMAL_VIEW_COMBINED_MIN_FRACTION = 0.0025
NORMAL_VIEW_COMPONENT_MIN_THICKNESS = 6
NORMAL_VIEW_COMPONENT_MAX_ELONGATION = 4.0
ALIGNMENT_MAX_LONG_EDGE = 512
ALIGNMENT_PYRAMID_SCALES = (0.5, 1.0)
MAX_ALIGNMENT_FRACTION = 0.04
MAX_ALIGNMENT_ROTATION_DEGREES = 1.5
MAX_ALIGNMENT_SCALE_CHANGE = 0.02
MAX_ALIGNMENT_SHEAR = 0.01
MIN_ALIGNMENT_OVERLAP = 0.85
MIN_ALIGNMENT_CORRELATION = 0.90
HIGH_CONFIDENCE_ALIGNMENT = 0.995
FACE_MIN_IOU = 0.60
FACE_LANDMARK_MEDIAN_LIMIT = 0.018
FACE_LANDMARK_P90_LIMIT = 0.040
FACE_CROP_MIN_SSIM = 0.985
SAME_FRAME_MIN_SSIM = 0.98
SAFE_NEAR_DUPLICATE_MIN_SSIM = 0.995


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
class AlignmentMetrics:
    mode: str
    correlation: float
    translation_x: float
    translation_y: float
    rotation_degrees: float
    scale_x: float
    scale_y: float
    shear: float
    overlap_fraction: float
    transform: tuple[tuple[float, float, float], tuple[float, float, float]]


@dataclass(frozen=True, slots=True)
class _AlignedPair:
    first_gray: np.ndarray
    aligned_second: np.ndarray
    valid_mask: np.ndarray
    transform: np.ndarray
    metrics: AlignmentMetrics


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
    reason_code: str = ""
    detail: str = ""
    metrics: dict[str, object] = field(default_factory=dict)

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
            "assessment_reason_code": self.reason_code,
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
        descriptor_loader_a: Callable[[], SubjectDescriptor | None] | None = None,
        descriptor_loader_b: Callable[[], SubjectDescriptor | None] | None = None,
        identical: bool = False,
    ) -> NearDuplicateAssessment:
        del path_a, path_b, fingerprint_a, fingerprint_b
        if identical:
            return NearDuplicateAssessment(
                NearDuplicateDecision.EXACT_DUPLICATE,
                reason_code="exact_duplicate",
                detail="byte-for-byte identical",
            )
        if self._should_stop():
            return self._uncertain("cancelled", "cancelled")

        alignment_started = time.perf_counter()
        prepared = _prepare_aligned_pair(image_a, image_b, self._should_stop)
        if prepared is None:
            return self._uncertain(
                "alignment_uncertain", "images could not be aligned safely"
            )
        alignment_seconds = time.perf_counter() - alignment_started
        if self._should_stop():
            return self._uncertain("cancelled", "cancelled", prepared=prepared)

        perceptual_started = time.perf_counter()
        corrected = _exposure_correct(
            prepared.first_gray,
            prepared.aligned_second,
            prepared.valid_mask,
        )
        if corrected is None:
            return self._uncertain(
                "alignment_uncertain",
                "exposure compensation was unstable",
                prepared=prepared,
            )
        normal_first, normal_second, normal_mask = _normal_view_pair(
            prepared.first_gray,
            corrected,
            prepared.valid_mask,
        )
        perceptual_first = prepare_same_frame_preview(normal_first * 255.0) / 255.0
        perceptual_second = prepare_same_frame_preview(normal_second * 255.0) / 255.0
        perceptual_mask = cv2.resize(
            normal_mask.astype(np.uint8),
            (perceptual_first.shape[1], perceptual_first.shape[0]),
            interpolation=cv2.INTER_NEAREST,
        ).astype(bool)
        structural = structural_similarity_for_aligned(
            perceptual_first * 255.0,
            perceptual_second * 255.0,
            perceptual_mask,
        )
        if structural is None:
            return self._uncertain(
                "alignment_uncertain",
                "structural similarity was unavailable",
                prepared=prepared,
            )
        full_change = coherent_change_metrics(
            prepared.first_gray,
            prepared.aligned_second,
            valid_mask=prepared.valid_mask,
            should_stop=self._should_stop,
        )
        visible_change = coherent_change_metrics(
            normal_first,
            normal_second,
            valid_mask=normal_mask,
            component_min_fraction=NORMAL_VIEW_COMPONENT_MIN_FRACTION,
            component_min_pixels=NORMAL_VIEW_COMPONENT_MIN_PIXELS,
            combined_min_fraction=NORMAL_VIEW_COMBINED_MIN_FRACTION,
            component_min_thickness=NORMAL_VIEW_COMPONENT_MIN_THICKNESS,
            component_max_elongation=NORMAL_VIEW_COMPONENT_MAX_ELONGATION,
            exposure_correct=False,
            should_stop=self._should_stop,
        )
        perceptual_seconds = time.perf_counter() - perceptual_started
        shared_metrics = _assessment_metrics(
            prepared,
            full_change=full_change,
            visible_change=visible_change,
            alignment_seconds=alignment_seconds,
            perceptual_seconds=perceptual_seconds,
        )
        shared_metrics["normal_view_structural_similarity"] = structural
        if structural < SAME_FRAME_MIN_SSIM:
            return NearDuplicateAssessment(
                NearDuplicateDecision.SUBJECT_CHANGED,
                structural_similarity=structural,
                change=visible_change,
                reason_code="visible_subject_change",
                detail="framing or subject structure changed",
                metrics=shared_metrics,
            )
        if self._should_stop():
            return self._uncertain(
                "cancelled",
                "cancelled",
                prepared=prepared,
                structural=structural,
                change=visible_change,
                metrics=shared_metrics,
            )

        if visible_change is None:
            if self._should_stop():
                return self._uncertain(
                    "cancelled",
                    "cancelled",
                    prepared=prepared,
                    structural=structural,
                    metrics=shared_metrics,
                )
            return self._uncertain(
                "alignment_uncertain",
                "normal-view change measurement was unstable",
                prepared=prepared,
                structural=structural,
                metrics=shared_metrics,
            )

        face_started = time.perf_counter()
        if descriptor_a is None and descriptor_loader_a is not None:
            descriptor_a = descriptor_loader_a()
        if self._should_stop():
            return self._uncertain(
                "cancelled",
                "cancelled",
                prepared=prepared,
                structural=structural,
                change=visible_change,
                metrics=shared_metrics,
            )
        if descriptor_b is None and descriptor_loader_b is not None:
            descriptor_b = descriptor_loader_b()
        face_seconds = time.perf_counter() - face_started
        shared_metrics["face_seconds"] = face_seconds
        if descriptor_a is None or descriptor_b is None:
            return self._uncertain(
                "face_analysis_unavailable",
                "face analysis was unavailable",
                prepared=prepared,
                structural=structural,
                change=visible_change,
                metrics=shared_metrics,
            )

        face_result = _compare_faces(
            descriptor_a,
            descriptor_b,
            prepared.first_gray,
            prepared.aligned_second,
            prepared.valid_mask,
            prepared.transform,
        )
        face_decision = face_result["decision"]
        if face_decision is NearDuplicateDecision.SUBJECT_CHANGED:
            decision = NearDuplicateDecision.SUBJECT_CHANGED
            reason_code = "face_changed"
            detail = str(face_result["detail"])
        elif face_decision is NearDuplicateDecision.UNCERTAIN:
            decision = NearDuplicateDecision.UNCERTAIN
            reason_code = "face_analysis_unavailable"
            detail = str(face_result["detail"])
        elif visible_change.meaningful_change:
            decision = NearDuplicateDecision.SUBJECT_CHANGED
            reason_code = "visible_subject_change"
            detail = "coherent change remains visible at normal viewing size"
        elif (
            prepared.metrics.correlation >= HIGH_CONFIDENCE_ALIGNMENT
            and structural >= SAFE_NEAR_DUPLICATE_MIN_SSIM
        ):
            decision = NearDuplicateDecision.SAFE_NEAR_DUPLICATE
            reason_code = "normal_view_equivalent"
            detail = "indistinguishable at normal viewing size"
        else:
            decision = NearDuplicateDecision.UNCERTAIN
            reason_code = "borderline_similarity"
            detail = "similar, but below the high-confidence Easy Delete proof"

        return NearDuplicateAssessment(
            decision,
            structural_similarity=structural,
            change=visible_change,
            face_count_a=len(descriptor_a.faces),
            face_count_b=len(descriptor_b.faces),
            face_min_iou=face_result["min_iou"],
            face_max_median_displacement=face_result["max_median"],
            face_max_p90_displacement=face_result["max_p90"],
            face_min_crop_similarity=face_result["min_crop_ssim"],
            reason_code=reason_code,
            detail=detail,
            metrics=shared_metrics,
        )

    @staticmethod
    def _uncertain(
        reason_code: str,
        detail: str,
        *,
        prepared: _AlignedPair | None = None,
        structural: float | None = None,
        change: CoherentChangeMetrics | None = None,
        metrics: dict[str, object] | None = None,
    ) -> NearDuplicateAssessment:
        combined = dict(metrics or {})
        if prepared is not None:
            combined.update(_alignment_result_metrics(prepared.metrics))
        return NearDuplicateAssessment(
            NearDuplicateDecision.UNCERTAIN,
            structural_similarity=structural,
            change=change,
            reason_code=reason_code,
            detail=detail,
            metrics=combined,
        )


def coherent_change_metrics(
    first: np.ndarray,
    aligned_second: np.ndarray,
    *,
    valid_mask: np.ndarray | None = None,
    component_min_fraction: float = CHANGE_COMPONENT_MIN_FRACTION,
    component_min_pixels: int = CHANGE_COMPONENT_MIN_PIXELS,
    combined_min_fraction: float = CHANGE_COMBINED_MIN_FRACTION,
    component_min_thickness: int = 1,
    component_max_elongation: float = float("inf"),
    exposure_correct: bool = True,
    should_stop: Callable[[], bool] | None = None,
) -> CoherentChangeMetrics | None:
    should_stop = should_stop or (lambda: False)
    if first.shape != aligned_second.shape or first.size == 0:
        return None
    first = np.asarray(first, dtype=np.float32)
    second = np.asarray(aligned_second, dtype=np.float32)
    if not np.isfinite(first).all() or not np.isfinite(second).all():
        return None

    if valid_mask is None:
        valid = np.ones(first.shape, dtype=bool)
    else:
        valid = np.asarray(valid_mask, dtype=bool)
        if valid.shape != first.shape or not np.any(valid):
            return None

    if exposure_correct:
        corrected = _exposure_correct(first, second, valid)
        if corrected is None:
            return None
    else:
        corrected = second
    difference = np.abs(first - corrected)
    valid_difference = difference[valid]
    median = float(np.median(valid_difference))
    normalized_mad = float(
        1.4826 * np.median(np.abs(valid_difference - median))
    )
    threshold = max(
        CHANGE_ABSOLUTE_FLOOR,
        median + CHANGE_MAD_MULTIPLIER * normalized_mad,
    )
    mask = np.asarray((difference >= threshold) & valid, dtype=np.uint8) * 255
    mask = cv2.morphologyEx(
        mask, cv2.MORPH_OPEN, np.ones((3, 3), dtype=np.uint8)
    )
    mask = cv2.morphologyEx(
        mask, cv2.MORPH_CLOSE, np.ones((5, 5), dtype=np.uint8)
    )

    count, labels, stats, _centroids = cv2.connectedComponentsWithStats(mask, 8)
    image_area = int(np.count_nonzero(valid))
    minimum_area = max(
        component_min_pixels,
        int(np.ceil(image_area * component_min_fraction)),
    )
    coherent_area = 0
    largest_area = 0
    largest_p90 = 0.0
    significant_component = False
    for label in range(1, count):
        if should_stop():
            return None
        area = int(stats[label, cv2.CC_STAT_AREA])
        if area <= 0:
            continue
        component_width = int(stats[label, cv2.CC_STAT_WIDTH])
        component_height = int(stats[label, cv2.CC_STAT_HEIGHT])
        minimum_dimension = min(component_width, component_height)
        elongation = max(component_width, component_height) / max(
            minimum_dimension, 1
        )
        if (
            minimum_dimension < component_min_thickness
            or elongation > component_max_elongation
        ):
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
        or coherent_fraction >= combined_min_fraction
    )
    return CoherentChangeMetrics(
        threshold=threshold,
        largest_component_fraction=largest_area / max(image_area, 1),
        coherent_fraction=coherent_fraction,
        largest_component_p90=largest_p90,
        meaningful_change=meaningful,
    )


def _prepare_aligned_pair(
    image_a: np.ndarray | None,
    image_b: np.ndarray | None,
    should_stop: Callable[[], bool] | None = None,
) -> _AlignedPair | None:
    should_stop = should_stop or (lambda: False)
    if image_a is None or image_b is None:
        return None
    first = _to_gray_unit(image_a)
    second = _to_gray_unit(image_b)
    if first is None or second is None or first.shape != second.shape:
        return None
    if should_stop():
        return None

    height, width = first.shape
    alignment_scale = min(
        1.0,
        ALIGNMENT_MAX_LONG_EDGE / max(height, width),
    )
    alignment_size = (
        max(32, int(round(width * alignment_scale))),
        max(32, int(round(height * alignment_scale))),
    )
    if alignment_size != (width, height):
        alignment_first = cv2.resize(
            first, alignment_size, interpolation=cv2.INTER_AREA
        )
        alignment_second = cv2.resize(
            second, alignment_size, interpolation=cv2.INTER_AREA
        )
    else:
        alignment_first = first
        alignment_second = second

    try:
        (shift_x, shift_y), response = cv2.phaseCorrelate(
            alignment_first,
            alignment_second,
        )
    except cv2.error:
        return None
    if not np.isfinite((shift_x, shift_y, response)).all() or response <= 0.0:
        return None

    base_transform = np.asarray(
        [[1.0, 0.0, shift_x], [0.0, 1.0, shift_y]],
        dtype=np.float32,
    )
    correlation = float(response)
    for pyramid_scale in ALIGNMENT_PYRAMID_SCALES:
        if should_stop():
            return None
        level_size = (
            max(32, int(round(alignment_size[0] * pyramid_scale))),
            max(32, int(round(alignment_size[1] * pyramid_scale))),
        )
        if level_size == alignment_size:
            level_first = alignment_first
            level_second = alignment_second
        else:
            level_first = cv2.resize(
                alignment_first, level_size, interpolation=cv2.INTER_AREA
            )
            level_second = cv2.resize(
                alignment_second, level_size, interpolation=cv2.INTER_AREA
            )
        level_transform = base_transform.copy()
        level_transform[:, 2] *= pyramid_scale
        try:
            correlation, level_transform = cv2.findTransformECC(
                level_first,
                level_second,
                level_transform,
                cv2.MOTION_AFFINE,
                (
                    cv2.TERM_CRITERIA_EPS | cv2.TERM_CRITERIA_COUNT,
                    100,
                    1e-6,
                ),
                None,
                5,
            )
        except cv2.error:
            return None
        if not np.isfinite(correlation) or not np.isfinite(level_transform).all():
            return None
        base_transform = np.asarray(level_transform, dtype=np.float32)
        base_transform[:, 2] /= pyramid_scale

    transform = base_transform.copy()
    transform[:, 2] /= alignment_scale
    alignment_metrics = _validate_alignment(
        transform,
        correlation=float(correlation),
        width=width,
        height=height,
    )
    if alignment_metrics is None:
        return None

    aligned = cv2.warpAffine(
        second,
        transform,
        (width, height),
        flags=cv2.INTER_LINEAR | cv2.WARP_INVERSE_MAP,
        borderMode=cv2.BORDER_CONSTANT,
        borderValue=0.0,
    )
    validity = cv2.warpAffine(
        np.ones(first.shape, dtype=np.uint8),
        transform,
        (width, height),
        flags=cv2.INTER_NEAREST | cv2.WARP_INVERSE_MAP,
        borderMode=cv2.BORDER_CONSTANT,
        borderValue=0,
    ).astype(bool)
    overlap_fraction = float(np.count_nonzero(validity) / validity.size)
    if overlap_fraction < MIN_ALIGNMENT_OVERLAP:
        return None
    alignment_metrics = AlignmentMetrics(
        mode=alignment_metrics.mode,
        correlation=alignment_metrics.correlation,
        translation_x=alignment_metrics.translation_x,
        translation_y=alignment_metrics.translation_y,
        rotation_degrees=alignment_metrics.rotation_degrees,
        scale_x=alignment_metrics.scale_x,
        scale_y=alignment_metrics.scale_y,
        shear=alignment_metrics.shear,
        overlap_fraction=overlap_fraction,
        transform=alignment_metrics.transform,
    )
    return _AlignedPair(
        first_gray=first,
        aligned_second=aligned,
        valid_mask=validity,
        transform=transform,
        metrics=alignment_metrics,
    )


def _validate_alignment(
    transform: np.ndarray,
    *,
    correlation: float,
    width: int,
    height: int,
) -> AlignmentMetrics | None:
    if transform.shape != (2, 3) or not np.isfinite(transform).all():
        return None
    if not np.isfinite(correlation) or correlation < MIN_ALIGNMENT_CORRELATION:
        return None
    linear = np.asarray(transform[:, :2], dtype=np.float64)
    translation_x = float(transform[0, 2])
    translation_y = float(transform[1, 2])
    if (
        abs(translation_x) > width * MAX_ALIGNMENT_FRACTION
        or abs(translation_y) > height * MAX_ALIGNMENT_FRACTION
    ):
        return None

    column_x = linear[:, 0]
    column_y = linear[:, 1]
    scale_x = float(np.linalg.norm(column_x))
    scale_y = float(np.linalg.norm(column_y))
    if min(scale_x, scale_y) <= np.finfo(np.float64).eps:
        return None
    rotation_degrees = math.degrees(math.atan2(linear[1, 0], linear[0, 0]))
    shear = float(abs(np.dot(column_x, column_y) / (scale_x * scale_y)))
    if (
        abs(rotation_degrees) > MAX_ALIGNMENT_ROTATION_DEGREES
        or abs(scale_x - 1.0) > MAX_ALIGNMENT_SCALE_CHANGE
        or abs(scale_y - 1.0) > MAX_ALIGNMENT_SCALE_CHANGE
        or shear > MAX_ALIGNMENT_SHEAR
    ):
        return None

    return AlignmentMetrics(
        mode="pyramidal_affine_ecc",
        correlation=float(correlation),
        translation_x=translation_x,
        translation_y=translation_y,
        rotation_degrees=float(rotation_degrees),
        scale_x=scale_x,
        scale_y=scale_y,
        shear=shear,
        overlap_fraction=0.0,
        transform=(
            tuple(float(value) for value in transform[0]),
            tuple(float(value) for value in transform[1]),
        ),
    )


def _normal_view_pair(
    first: np.ndarray,
    second: np.ndarray,
    valid_mask: np.ndarray,
) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    height, width = first.shape
    scale = min(1.0, NORMAL_VIEW_LONG_EDGE / max(height, width))
    target_size = (
        max(1, int(round(width * scale))),
        max(1, int(round(height * scale))),
    )
    if target_size == (width, height):
        normal_first = first.copy()
        normal_second = second.copy()
        normal_mask = valid_mask.copy()
    else:
        normal_first = cv2.resize(
            first, target_size, interpolation=cv2.INTER_AREA
        )
        normal_second = cv2.resize(
            second, target_size, interpolation=cv2.INTER_AREA
        )
        normal_mask = cv2.resize(
            valid_mask.astype(np.uint8),
            target_size,
            interpolation=cv2.INTER_NEAREST,
        ).astype(bool)
    normal_first = cv2.GaussianBlur(
        normal_first,
        (0, 0),
        sigmaX=NORMAL_VIEW_BLUR_SIGMA,
        sigmaY=NORMAL_VIEW_BLUR_SIGMA,
    )
    normal_second = cv2.GaussianBlur(
        normal_second,
        (0, 0),
        sigmaX=NORMAL_VIEW_BLUR_SIGMA,
        sigmaY=NORMAL_VIEW_BLUR_SIGMA,
    )
    return normal_first, normal_second, normal_mask


def _alignment_result_metrics(metrics: AlignmentMetrics) -> dict[str, object]:
    return {
        "alignment_mode": metrics.mode,
        "alignment_correlation": metrics.correlation,
        "alignment_translation_x": metrics.translation_x,
        "alignment_translation_y": metrics.translation_y,
        "alignment_rotation_degrees": metrics.rotation_degrees,
        "alignment_scale_x": metrics.scale_x,
        "alignment_scale_y": metrics.scale_y,
        "alignment_shear": metrics.shear,
        "alignment_overlap_fraction": metrics.overlap_fraction,
        "alignment_transform": [
            list(metrics.transform[0]),
            list(metrics.transform[1]),
        ],
        "normal_view_long_edge": NORMAL_VIEW_LONG_EDGE,
        "normal_view_blur_sigma": NORMAL_VIEW_BLUR_SIGMA,
    }


def _assessment_metrics(
    prepared: _AlignedPair,
    *,
    full_change: CoherentChangeMetrics | None,
    visible_change: CoherentChangeMetrics | None,
    alignment_seconds: float,
    perceptual_seconds: float,
) -> dict[str, object]:
    metrics = _alignment_result_metrics(prepared.metrics)
    metrics.update(
        {
            "alignment_seconds": alignment_seconds,
            "perceptual_seconds": perceptual_seconds,
            "full_resolution_change_fraction": (
                full_change.coherent_fraction
                if full_change is not None
                else None
            ),
            "full_resolution_largest_component_fraction": (
                full_change.largest_component_fraction
                if full_change is not None
                else None
            ),
            "normal_view_change_fraction": (
                visible_change.coherent_fraction
                if visible_change is not None
                else None
            ),
        }
    )
    return metrics


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
    target: np.ndarray,
    source: np.ndarray,
    valid_mask: np.ndarray | None = None,
) -> np.ndarray | None:
    valid_values = (
        source[np.asarray(valid_mask, dtype=bool)]
        if valid_mask is not None
        else source.ravel()
    )
    if valid_values.size < 64:
        return None
    low, high = np.percentile(valid_values, (2.0, 98.0))
    mask = (source > low) & (source < high)
    if valid_mask is not None:
        mask &= np.asarray(valid_mask, dtype=bool)
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
    valid_mask: np.ndarray,
    transform: np.ndarray,
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
    try:
        second_to_first = cv2.invertAffineTransform(
            np.asarray(transform, dtype=np.float64)
        )
    except cv2.error:
        return _face_result(
            NearDuplicateDecision.UNCERTAIN,
            detail="face alignment transform was unavailable",
        )
    adjusted_second = [
        _transform_face(face, second_to_first, width, height)
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
            first_gray,
            aligned_second,
            face_a.bbox,
            valid_mask,
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


def _transform_face(
    face: FaceDescriptor,
    transform: np.ndarray,
    width: int,
    height: int,
) -> FaceDescriptor:
    points = np.asarray(face.landmarks, dtype=np.float64)
    pixel_points = np.column_stack((points[:, 0] * width, points[:, 1] * height))
    transformed = cv2.transform(
        pixel_points.reshape(1, -1, 2),
        np.asarray(transform, dtype=np.float64),
    ).reshape(-1, 2)
    normalized = np.column_stack(
        (transformed[:, 0] / width, transformed[:, 1] / height)
    )
    xs = normalized[:, 0]
    ys = normalized[:, 1]
    return FaceDescriptor(
        bbox=(float(xs.min()), float(ys.min()), float(xs.max()), float(ys.max())),
        landmarks=tuple((float(x), float(y)) for x, y in normalized),
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
    valid_mask: np.ndarray | None = None,
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
    crop_mask = (
        valid_mask[top:bottom, left:right]
        if valid_mask is not None
        else np.ones(first_crop.shape, dtype=bool)
    )
    preview_mask = cv2.resize(
        crop_mask.astype(np.uint8),
        (128, 96),
        interpolation=cv2.INTER_NEAREST,
    ).astype(bool)
    return structural_similarity_for_aligned(
        prepare_same_frame_preview(first_crop),
        prepare_same_frame_preview(second_crop),
        preview_mask,
    )

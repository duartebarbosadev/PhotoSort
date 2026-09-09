"""Small, decode-independent helpers for recognizing unchanged photo framing."""

from __future__ import annotations

from dataclasses import dataclass

import cv2
import numpy as np


SAME_FRAME_PREVIEW_SIZE = (128, 96)
_SSIM_WINDOW_SIZE = (11, 11)
_SSIM_SIGMA = 1.5


@dataclass(frozen=True, slots=True)
class LocalizedChangeMetrics:
    """Strength and concentration of aligned pixel differences."""

    p99_difference: float
    concentration_ratio: float


def prepare_same_frame_preview(gray: np.ndarray) -> np.ndarray:
    """Return a compact, denoised grayscale preview for structural comparison."""
    if gray.ndim == 3:
        gray = cv2.cvtColor(gray, cv2.COLOR_BGR2GRAY)
    resized = cv2.resize(
        gray,
        SAME_FRAME_PREVIEW_SIZE,
        interpolation=cv2.INTER_AREA,
    ).astype(np.float32)
    # RAW burst frames often differ mostly in sensor noise. Suppress that noise while
    # retaining subject edges and positions, which are the signal this check needs.
    return cv2.GaussianBlur(resized, (0, 0), sigmaX=2.0, sigmaY=2.0)


def aligned_structural_similarity(
    first: np.ndarray,
    second: np.ndarray,
) -> float | None:
    """Return a strict 0..1 same-framing score after compensating for tiny camera shifts."""
    if first.size == 0 or second.size == 0 or first.shape != second.shape:
        return None

    first_float = np.asarray(first, dtype=np.float32)
    second_float = np.asarray(second, dtype=np.float32)
    aligned = _align_second_preview(first_float, second_float)
    if aligned is None:
        return None
    aligned_second, within_shift_limit = aligned
    if not within_shift_limit:
        return 0.0

    return structural_similarity_for_aligned(first_float, aligned_second)


def structural_similarity_for_aligned(
    first: np.ndarray,
    second: np.ndarray,
    valid_mask: np.ndarray | None = None,
) -> float | None:
    """Return global/local SSIM for images that already share one coordinate frame."""

    if first.size == 0 or second.size == 0 or first.shape != second.shape:
        return None
    first_float = np.asarray(first, dtype=np.float32)
    second_float = np.asarray(second, dtype=np.float32)
    if not np.isfinite(first_float).all() or not np.isfinite(second_float).all():
        return None

    score_map = _ssim_map(first_float, second_float)
    if valid_mask is None:
        mask = np.ones(score_map.shape, dtype=bool)
    else:
        mask = np.asarray(valid_mask, dtype=bool)
        if mask.shape != score_map.shape:
            return None
        # SSIM uses an 11x11 support window. Exclude pixels whose support touches
        # an alignment border so synthetic border values never affect the score.
        mask = cv2.erode(
            mask.astype(np.uint8),
            np.ones(_SSIM_WINDOW_SIZE, dtype=np.uint8),
        ).astype(bool)
    if not np.any(mask):
        return None

    global_score = float(score_map[mask].mean())
    tile_scores = [
        float(score_tile[mask_tile].mean())
        for score_rows, mask_rows in zip(
            np.array_split(score_map, 4, axis=0),
            np.array_split(mask, 4, axis=0),
            strict=True,
        )
        for score_tile, mask_tile in zip(
            np.array_split(score_rows, 4, axis=1),
            np.array_split(mask_rows, 4, axis=1),
            strict=True,
        )
        if np.any(mask_tile)
    ]
    if not tile_scores or not np.isfinite(global_score):
        return None

    # The lower-quartile tile score prevents a moved subject from being hidden by a
    # large unchanged background, while tolerating small uniform exposure/noise changes.
    local_score = float(np.quantile(tile_scores, 0.25))
    return max(0.0, min(1.0, global_score, local_score))


def aligned_localized_change_metrics(
    first: np.ndarray,
    second: np.ndarray,
) -> LocalizedChangeMetrics | None:
    """Measure whether otherwise small differences cluster in one image region."""
    if first.size == 0 or second.size == 0 or first.shape != second.shape:
        return None

    first_float = np.asarray(first, dtype=np.float32)
    second_float = np.asarray(second, dtype=np.float32)
    aligned = _align_second_preview(first_float, second_float)
    if aligned is None:
        return None
    aligned_second, within_shift_limit = aligned
    if not within_shift_limit:
        return LocalizedChangeMetrics(255.0, float("inf"))

    # Remove a global exposure offset/gain before inspecting the difference tail.
    low, high = np.percentile(aligned_second, (2.0, 98.0))
    fit_mask = (aligned_second > low) & (aligned_second < high)
    source = aligned_second[fit_mask]
    target = first_float[fit_mask]
    if source.size >= 16:
        source_mean = float(source.mean())
        target_mean = float(target.mean())
        centered_source = source - source_mean
        variance = float(np.dot(centered_source, centered_source))
        if variance > np.finfo(np.float32).eps:
            gain = float(np.dot(centered_source, target - target_mean) / variance)
            gain = max(0.5, min(2.0, gain))
            offset = target_mean - gain * source_mean
            aligned_second = np.clip(
                aligned_second * gain + offset,
                0.0,
                255.0,
            )

    difference = np.abs(first_float - aligned_second)
    mean_difference = float(difference.mean())
    p99_difference = float(np.quantile(difference, 0.99))
    return LocalizedChangeMetrics(
        p99_difference=p99_difference,
        concentration_ratio=p99_difference / max(mean_difference, 0.25),
    )


def _align_second_preview(
    first: np.ndarray,
    second: np.ndarray,
) -> tuple[np.ndarray, bool] | None:
    try:
        (shift_x, shift_y), _response = cv2.phaseCorrelate(first, second)
    except cv2.error:
        return None
    if not np.isfinite((shift_x, shift_y)).all():
        return None

    height, width = first.shape[:2]
    within_shift_limit = not (
        abs(shift_x) > width * 0.04 or abs(shift_y) > height * 0.04
    )
    transform = np.float32([[1.0, 0.0, shift_x], [0.0, 1.0, shift_y]])
    aligned_second = cv2.warpAffine(
        second,
        transform,
        (width, height),
        flags=cv2.INTER_LINEAR | cv2.WARP_INVERSE_MAP,
        borderMode=cv2.BORDER_REFLECT,
    )
    return aligned_second, within_shift_limit


def _ssim_map(first: np.ndarray, second: np.ndarray) -> np.ndarray:
    c1 = (0.01 * 255.0) ** 2
    c2 = (0.03 * 255.0) ** 2
    mean_first = cv2.GaussianBlur(first, _SSIM_WINDOW_SIZE, _SSIM_SIGMA)
    mean_second = cv2.GaussianBlur(second, _SSIM_WINDOW_SIZE, _SSIM_SIGMA)
    mean_first_sq = mean_first * mean_first
    mean_second_sq = mean_second * mean_second
    mean_both = mean_first * mean_second
    variance_first = (
        cv2.GaussianBlur(first * first, _SSIM_WINDOW_SIZE, _SSIM_SIGMA) - mean_first_sq
    )
    variance_second = (
        cv2.GaussianBlur(second * second, _SSIM_WINDOW_SIZE, _SSIM_SIGMA)
        - mean_second_sq
    )
    covariance = (
        cv2.GaussianBlur(first * second, _SSIM_WINDOW_SIZE, _SSIM_SIGMA) - mean_both
    )
    numerator = (2.0 * mean_both + c1) * (2.0 * covariance + c2)
    denominator = (mean_first_sq + mean_second_sq + c1) * (
        variance_first + variance_second + c2
    )
    return numerator / np.maximum(denominator, np.finfo(np.float32).eps)

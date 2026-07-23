"""Small, decode-independent helpers for recognizing unchanged photo framing."""

from __future__ import annotations

import cv2
import numpy as np


SAME_FRAME_PREVIEW_SIZE = (128, 96)
_SSIM_WINDOW_SIZE = (11, 11)
_SSIM_SIGMA = 1.5


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
    try:
        (shift_x, shift_y), _response = cv2.phaseCorrelate(
            first_float, second_float
        )
    except cv2.error:
        return None
    if not np.isfinite((shift_x, shift_y)).all():
        return None

    height, width = first_float.shape[:2]
    # A large alignment would hide a real composition change. The allowance only
    # compensates for sub-pixel/tiny handheld movement between burst frames.
    if abs(shift_x) > width * 0.04 or abs(shift_y) > height * 0.04:
        return 0.0

    transform = np.float32([[1.0, 0.0, shift_x], [0.0, 1.0, shift_y]])
    aligned_second = cv2.warpAffine(
        second_float,
        transform,
        (width, height),
        flags=cv2.INTER_LINEAR | cv2.WARP_INVERSE_MAP,
        borderMode=cv2.BORDER_REFLECT,
    )

    score_map = _ssim_map(first_float, aligned_second)
    global_score = float(score_map.mean())
    tile_scores = [
        float(tile.mean())
        for tile_row in np.array_split(score_map, 4, axis=0)
        for tile in np.array_split(tile_row, 4, axis=1)
        if tile.size
    ]
    if not tile_scores or not np.isfinite(global_score):
        return None

    # The lower-quartile tile score prevents a moved subject from being hidden by a
    # large unchanged background, while tolerating small uniform exposure/noise changes.
    local_score = float(np.quantile(tile_scores, 0.25))
    return max(0.0, min(1.0, global_score, local_score))


def _ssim_map(first: np.ndarray, second: np.ndarray) -> np.ndarray:
    c1 = (0.01 * 255.0) ** 2
    c2 = (0.03 * 255.0) ** 2
    mean_first = cv2.GaussianBlur(
        first, _SSIM_WINDOW_SIZE, _SSIM_SIGMA
    )
    mean_second = cv2.GaussianBlur(
        second, _SSIM_WINDOW_SIZE, _SSIM_SIGMA
    )
    mean_first_sq = mean_first * mean_first
    mean_second_sq = mean_second * mean_second
    mean_both = mean_first * mean_second
    variance_first = (
        cv2.GaussianBlur(first * first, _SSIM_WINDOW_SIZE, _SSIM_SIGMA)
        - mean_first_sq
    )
    variance_second = (
        cv2.GaussianBlur(second * second, _SSIM_WINDOW_SIZE, _SSIM_SIGMA)
        - mean_second_sq
    )
    covariance = (
        cv2.GaussianBlur(first * second, _SSIM_WINDOW_SIZE, _SSIM_SIGMA)
        - mean_both
    )
    numerator = (2.0 * mean_both + c1) * (2.0 * covariance + c2)
    denominator = (mean_first_sq + mean_second_sq + c1) * (
        variance_first + variance_second + c2
    )
    return numerator / np.maximum(denominator, np.finfo(np.float32).eps)

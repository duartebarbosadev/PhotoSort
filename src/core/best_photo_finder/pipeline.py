from dataclasses import replace
from functools import cmp_to_key
import os
from pathlib import Path
from collections.abc import Callable, Iterable, Sequence

from core.best_photo_finder.config import SelectorConfig
from core.best_photo_finder.errors import (
    FaceLandmarkerError,
    IncompleteSelectionError,
    NoScorableImagesError,
    NoSupportedImagesError,
    SelectionError,
)
from core.best_photo_finder.models import ImageScore, SelectionResult, TechnicalMetrics
from core.best_photo_finder.scorers import (
    AestheticScorer,
    HuggingFaceAestheticScorer,
    OpenCvMediapipeTechnicalScorer,
    TechnicalScorer,
)
from core.image_processing.raw_image_processor import is_raw_extension
from core.image_processing.standard_image_processor import SUPPORTED_STANDARD_EXTENSIONS


def _coerce_paths(paths: Iterable[str | Path], config: SelectorConfig) -> list[Path]:
    normalized = [Path(path).expanduser().resolve() for path in paths]
    filtered = [
        path
        for path in normalized
        if path.suffix.lower() in config.supported_extensions
        or path.suffix.lower() in SUPPORTED_STANDARD_EXTENSIONS
        or is_raw_extension(path.suffix.lower())
    ]
    if not filtered:
        raise NoSupportedImagesError("No supported image files were provided.")
    return sorted(filtered)


def _image_score_from_metrics(path: Path, metrics: TechnicalMetrics) -> ImageScore:
    return ImageScore(
        path=str(path),
        blur_variance=metrics.blur_variance,
        blur_penalty=metrics.blur_penalty,
        face_count=metrics.face_count,
        closed_face_count=metrics.closed_face_count,
        eye_penalty=metrics.eye_penalty,
        technical_penalty=metrics.blur_penalty + metrics.eye_penalty,
        max_face_area_ratio=metrics.max_face_area_ratio,
        image_width=metrics.image_width,
        image_height=metrics.image_height,
        issues=metrics.issues,
    )


def _failure_details(images: Sequence[ImageScore]) -> list[tuple[str, str]]:
    details: list[tuple[str, str]] = []
    for image in images:
        reason = (image.failure_reason or "").strip()
        if not reason:
            continue
        details.append((image.path, reason))
    return details


def _format_failure_summary(
    failures: Sequence[tuple[str, str]], *, limit: int = 3
) -> str:
    if not failures:
        return ""

    preview = [
        f"{os.path.basename(path)}: {reason}" for path, reason in failures[:limit]
    ]
    remaining = len(failures) - len(preview)
    if remaining > 0:
        preview.append(f"+{remaining} more")
    return " Failures: " + "; ".join(preview)


def _sort_comparator(tie_threshold: float):
    def compare(left: ImageScore, right: ImageScore) -> int:
        if left.sharpness_eligible != right.sharpness_eligible:
            return -1 if left.sharpness_eligible else 1

        left_final = left.final_score if left.final_score is not None else float("-inf")
        right_final = (
            right.final_score if right.final_score is not None else float("-inf")
        )
        if abs(left_final - right_final) > tie_threshold:
            return -1 if left_final > right_final else 1

        left_aesthetic = (
            left.aesthetic_score if left.aesthetic_score is not None else float("-inf")
        )
        right_aesthetic = (
            right.aesthetic_score
            if right.aesthetic_score is not None
            else float("-inf")
        )
        if left_aesthetic != right_aesthetic:
            return -1 if left_aesthetic > right_aesthetic else 1

        if left.pixel_count != right.pixel_count:
            return -1 if left.pixel_count > right.pixel_count else 1

        if left.max_face_area_ratio != right.max_face_area_ratio:
            return -1 if left.max_face_area_ratio > right.max_face_area_ratio else 1

        return -1 if left.path < right.path else 1 if left.path > right.path else 0

    return compare


def _apply_cluster_relative_sharpness(
    images: Sequence[ImageScore], floor: float
) -> None:
    """Exclude substantially softer frames using within-cluster measurements."""

    variances = [
        max(0.0, float(image.blur_variance))
        for image in images
        if image.blur_variance is not None
    ]
    reference = max(variances, default=0.0)
    normalized_floor = max(0.0, min(1.0, float(floor)))
    for image in images:
        variance = max(0.0, float(image.blur_variance or 0.0))
        ratio = variance / reference if reference > 0.0 else 1.0
        image.cluster_sharpness_ratio = min(1.0, ratio)
        image.sharpness_eligible = ratio >= normalized_floor
        if not image.sharpness_eligible:
            image.issues = (
                *image.issues,
                f"Substantially softer than this cluster's sharpest frame "
                f"({ratio:.0%} relative sharpness)",
            )


def _enforce_sharpness_gate(images: Sequence[ImageScore], tie_threshold: float) -> None:
    """Keep every ineligible display score below every eligible display score."""

    eligible_scores = [
        image.final_score
        for image in images
        if image.sharpness_eligible and image.final_score is not None
    ]
    ineligible_scores = [
        image.final_score
        for image in images
        if not image.sharpness_eligible and image.final_score is not None
    ]
    if not eligible_scores or not ineligible_scores:
        return
    offset = max(
        0.0,
        max(ineligible_scores) - min(eligible_scores) + max(1e-6, tie_threshold),
    )
    if offset <= 0.0:
        return
    for image in images:
        if not image.sharpness_eligible and image.final_score is not None:
            image.final_score -= offset


class PhotoSelector:
    def __init__(
        self,
        *,
        technical_scorer: TechnicalScorer | None = None,
        aesthetic_scorer: AestheticScorer | None = None,
        preview_loader: Callable[[Path], object] | None = None,
    ) -> None:
        self.technical_scorer = technical_scorer or OpenCvMediapipeTechnicalScorer()
        self.aesthetic_scorer = aesthetic_scorer or HuggingFaceAestheticScorer()
        self.preview_loader = preview_loader

    def close(self) -> None:
        """Release native scorer resources owned by this selector."""
        close = getattr(self.technical_scorer, "close", None)
        if callable(close):
            close()

    def select(
        self, paths: Sequence[str | Path], config: SelectorConfig | None = None
    ) -> SelectionResult:
        config = config or SelectorConfig()
        normalized_paths = _coerce_paths(paths, config)

        scored: list[ImageScore] = []
        failed: list[ImageScore] = []
        path_lookup: dict[Path, ImageScore] = {}
        preview_images: dict[Path, object] = {}

        for path in normalized_paths:
            try:
                preview = self.preview_loader(path) if self.preview_loader else None
                if preview is not None:
                    preview_images[path] = preview
                    metrics = self.technical_scorer.score_image(path, preview, config)
                else:
                    metrics = self.technical_scorer.score(path, config)
            except FaceLandmarkerError:
                raise
            except SelectionError as exc:
                failed.append(
                    ImageScore(
                        path=str(path), status="failed", failure_reason=str(exc)
                    ),
                )
                continue
            image_score = _image_score_from_metrics(path, metrics)
            scored.append(image_score)
            path_lookup[path] = image_score

        if not scored:
            failures = _failure_details(failed)
            raise NoScorableImagesError(
                "No images could be scored successfully."
                + _format_failure_summary(failures),
                failures=failures,
            )
        if failed:
            failures = _failure_details(failed)
            raise IncompleteSelectionError(
                "Pick Best requires every image in a cluster to be scored."
                + _format_failure_summary(failures),
                failures=failures,
            )

        _apply_cluster_relative_sharpness(scored, config.relative_sharpness_floor)

        if preview_images:
            preview_batch = {
                path: preview_images[path]
                for path in path_lookup
                if path in preview_images
            }
            aesthetic_scores = self.aesthetic_scorer.score_batch_from_images(
                preview_batch, config
            )
        else:
            aesthetic_scores = self.aesthetic_scorer.score_batch(
                list(path_lookup.keys()), config
            )

        rankable: list[ImageScore] = []
        for path, image_score in path_lookup.items():
            score = aesthetic_scores.get(path)
            if score is None:
                failed.append(
                    replace(
                        image_score,
                        status="failed",
                        final_score=None,
                        failure_reason="Aesthetic model did not return a score for this image.",
                    )
                )
                continue
            image_score.aesthetic_score = score
            image_score.base_score = score - image_score.technical_penalty
            image_score.final_score = image_score.base_score
            rankable.append(image_score)

        if not rankable:
            failures = _failure_details(failed)
            raise NoScorableImagesError(
                "Aesthetic scoring failed for every image."
                + _format_failure_summary(failures),
                failures=failures,
            )
        if failed:
            failures = _failure_details(failed)
            raise IncompleteSelectionError(
                "Pick Best requires an aesthetic score for every image."
                + _format_failure_summary(failures),
                failures=failures,
            )

        _enforce_sharpness_gate(rankable, config.tie_threshold)

        ranked = sorted(
            rankable, key=cmp_to_key(_sort_comparator(config.tie_threshold))
        )
        winner = ranked[0]
        return SelectionResult(
            winner=winner,
            ranked_images=ranked,
            failed_images=failed,
            config=config.to_dict(),
            device_used=self.aesthetic_scorer.device_used,
            model_name=self.aesthetic_scorer.model_name,
        )


def select_best_image(
    paths: Sequence[str | Path], config: SelectorConfig | None = None
) -> SelectionResult:
    selector = PhotoSelector()
    return selector.select(paths, config=config)

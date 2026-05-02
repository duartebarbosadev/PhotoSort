from __future__ import annotations

from dataclasses import replace
from functools import cmp_to_key
import os
from pathlib import Path
from typing import Callable, Iterable, Sequence

from core.best_photo_finder.config import SelectorConfig
from core.best_photo_finder.errors import (
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


def _format_failure_summary(failures: Sequence[tuple[str, str]], *, limit: int = 3) -> str:
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

        if preview_images:
            preview_batch = {
                path: preview_images[path]
                for path in path_lookup.keys()
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
            image_score.final_score = score - image_score.technical_penalty
            rankable.append(image_score)

        if not rankable:
            failures = _failure_details(failed)
            raise NoScorableImagesError(
                "Aesthetic scoring failed for every image."
                + _format_failure_summary(failures),
                failures=failures,
            )

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

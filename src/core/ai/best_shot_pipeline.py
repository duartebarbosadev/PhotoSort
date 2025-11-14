from __future__ import annotations

import base64
import io
import json
import logging
import math
import os
import re
import threading
from concurrent.futures import ThreadPoolExecutor, as_completed
from dataclasses import dataclass
from enum import Enum
from typing import Any, Callable, Dict, List, Optional, Sequence, Tuple, Set

import numpy as np
from PIL import Image, ImageDraw, ImageFont, ImageOps, ImageStat

try:  # pragma: no cover - OpenCV optional during tests
    import cv2
except Exception:  # pragma: no cover - gracefully degrade if OpenCV missing
    cv2 = None

from core.ai.best_photo_selector import (
    BestPhotoSelector,
    DEFAULT_METRIC_SPECS,
    EyeStateAnalyzer,
    MetricSpec,
)
from core.app_settings import (
    PerformanceMode,
    get_custom_thread_count,
    get_best_shot_engine,
    get_openai_config,
    get_performance_mode,
    get_preferred_torch_device,
    calculate_max_workers,
    DEFAULT_OPENAI_API_KEY,
    DEFAULT_OPENAI_MODEL,
    DEFAULT_OPENAI_BASE_URL,
    DEFAULT_OPENAI_MAX_TOKENS,
    DEFAULT_OPENAI_TIMEOUT,
    DEFAULT_OPENAI_MAX_WORKERS,
)

logger = logging.getLogger(__name__)


class BestShotEngine(str, Enum):
    LOCAL = "local"
    LLM = "llm"


DEFAULT_BEST_SHOT_PROMPT = (
    "You are an expert photography critic tasked with selecting the best image from a similar set of {image_count} images.\n\n"
    "Analyze each image based on the following criteria:\n"
    "- Sharpness and Focus\n- Color/Lighting\n- Composition\n- Subject Expression\n- Technical Quality\n- Overall Appeal\n- Editing Potential\n- Subject Sharpness\n\n"
    "If any person’s eyes are closed, the photo automatically receives a low rating (1–2).\n\n"
    "Please analyze each image and then provide your response in the following format:\n\n"
    "Best Image: [Image number, 1-{image_count}]\n"
    "Confidence: [High/Medium/Low]\n"
    "Reasoning: [Brief explanation]\n\n"
    "Be decisive and pick ONE image as the best, even if the differences are subtle."
)

DEFAULT_RATING_PROMPT = (
    "Quantitatively evaluate the photograph by inspecting the high-frequency detail (micro-contrast), subject facial cues, noise distribution, tonal balance, color fidelity, compositional geometry, and lighting directionality.\n"
    "Assign each of the following metrics a score from 0–100 (integers) where 50 represents acceptable quality for professional sharing:\n"
    "- sharpness: edge acuity and micro-contrast on the subject's eyes and key textures\n"
    "- noise_control: luminance/chroma noise in mid-tones and shadows (higher = cleaner)\n"
    "- exposure_balance: dynamic range handling, highlight retention, and shadow lift\n"
    "- color_accuracy: white balance correctness and skin tone realism\n"
    "- composition_balance: adherence to composition rules (framing, leading lines, clutter control)\n"
    "- subject_expression: clarity of subject intent (eyes open, engaging expression, lack of motion blur)\n\n"
    "Compute an overall_quality score as the weighted average of the metrics with weights:\n"
    "sharpness 0.25, noise_control 0.15, exposure_balance 0.15, color_accuracy 0.15, composition_balance 0.15, subject_expression 0.15.\n"
    "Map overall_quality to a 1–5 star rating using these deterministic thresholds (include the boundary in the higher rating):\n"
    "1 star <= 40 < 2 star, 2 star <= 55 < 3 star, 3 star <= 70 < 4 star, 4 star <= 85 < 5 star, 5 star >= 85.\n"
    "The same image must always produce the same rating when scored with this rubric.\n"
    "Provide one concise sentence noting the dominant strengths and the limiting flaw(s)."
)

MAX_LOCAL_ANALYSIS_EDGE = 1024
RESPONSIVE_LOCAL_ANALYSIS_EDGE = 640
PERFORMANCE_RATIO_THRESHOLD = 0.95
PREFILTER_PREVIEW_MAX_EDGE = 512
PREFILTER_MAX_CANDIDATES = 3
PREFILTER_MIN_CLUSTER_SIZE = 4
HEURISTIC_SHARPNESS_NORMALIZER = 250.0
HEURISTIC_CONTRAST_NORMALIZER = 75.0

if hasattr(Image, "Resampling"):
    _RESAMPLE_BEST = Image.Resampling.LANCZOS
else:  # pragma: no cover - Pillow < 10
    _RESAMPLE_BEST = Image.LANCZOS


@dataclass
class LLMConfig:
    api_key: Optional[str]
    model: str = DEFAULT_OPENAI_MODEL
    base_url: Optional[str] = DEFAULT_OPENAI_BASE_URL
    max_tokens: int = DEFAULT_OPENAI_MAX_TOKENS
    timeout: int = DEFAULT_OPENAI_TIMEOUT
    best_shot_prompt: Optional[str] = None
    rating_prompt: Optional[str] = None
    max_workers: int = DEFAULT_OPENAI_MAX_WORKERS

    def __post_init__(self) -> None:
        if not self.best_shot_prompt:
            self.best_shot_prompt = DEFAULT_BEST_SHOT_PROMPT
        if not self.rating_prompt:
            self.rating_prompt = DEFAULT_RATING_PROMPT


@dataclass(frozen=True)
class LocalAnalysisProfile:
    name: str
    max_edge: int
    metric_specs: Sequence[MetricSpec]


def _metric_specs_for(names: Sequence[str]) -> Tuple[MetricSpec, ...]:
    enabled = {name.lower() for name in names}
    filtered = tuple(
        spec for spec in DEFAULT_METRIC_SPECS if spec.name.lower() in enabled
    )
    return filtered or tuple(DEFAULT_METRIC_SPECS)


_PERFORMANCE_ANALYSIS_PROFILE = LocalAnalysisProfile(
    name="performance",
    max_edge=MAX_LOCAL_ANALYSIS_EDGE,
    metric_specs=tuple(DEFAULT_METRIC_SPECS),
)

_RESPONSIVE_ANALYSIS_PROFILE = LocalAnalysisProfile(
    name="responsive",
    max_edge=RESPONSIVE_LOCAL_ANALYSIS_EDGE,
    metric_specs=_metric_specs_for(("musiq", "maniqa")),
)


def _calculate_custom_thread_ratio() -> Optional[float]:
    cpu_count = os.cpu_count() or 0
    if cpu_count <= 0:
        return None
    try:
        custom_threads = get_custom_thread_count()
    except Exception:
        return None
    clamped = max(1, min(cpu_count, int(custom_threads)))
    return clamped / float(cpu_count)


def select_local_analysis_profile(
    mode: PerformanceMode,
    *,
    custom_thread_ratio: Optional[float] = None,
) -> LocalAnalysisProfile:
    if mode in (PerformanceMode.PERFORMANCE, PerformanceMode.BALANCED):
        return _PERFORMANCE_ANALYSIS_PROFILE
    if mode == PerformanceMode.CUSTOM and custom_thread_ratio is not None:
        if custom_thread_ratio >= PERFORMANCE_RATIO_THRESHOLD:
            return _PERFORMANCE_ANALYSIS_PROFILE
    return _RESPONSIVE_ANALYSIS_PROFILE


def _determine_local_analysis_profile() -> LocalAnalysisProfile:
    mode = get_performance_mode()
    ratio = _calculate_custom_thread_ratio() if mode == PerformanceMode.CUSTOM else None
    profile = select_local_analysis_profile(mode, custom_thread_ratio=ratio)
    logger.info(
        "Using '%s' local AI profile (max edge %d px, metrics: %s)",
        profile.name,
        profile.max_edge,
        ", ".join(spec.name.upper() for spec in profile.metric_specs),
    )
    return profile


@dataclass(frozen=True)
class HeuristicCandidate:
    image_path: str
    score: float
    sharpness: float
    exposure_balance: float
    histogram_balance: float
    eye_openness: float

    def as_dict(self) -> Dict[str, float]:
        return {
            "score": self.score,
            "sharpness": self.sharpness,
            "exposure_balance": self.exposure_balance,
            "histogram_balance": self.histogram_balance,
            "eye_openness": self.eye_openness,
        }


class FastHeuristicStage:
    """
    Lightweight heuristics that quickly reject obviously bad frames before heavy IQA.

    Signals reuse the same Laplacian variance idea as the blur detector plus
    coarse histogram/contrast checks and (optionally) the eye-state classifier.
    """

    def __init__(
        self, image_pipeline, preview_max_edge: int = PREFILTER_PREVIEW_MAX_EDGE
    ):
        self._image_pipeline = image_pipeline
        self._preview_max_edge = preview_max_edge
        self._eye_detection_disabled = False
        self._eye_analyzer_local = threading.local()

    def _load_preview(self, image_path: str) -> Optional[Image.Image]:
        preview = None
        if self._image_pipeline is not None:
            try:
                preview = self._image_pipeline.get_preview_image(image_path)
                if preview is not None:
                    preview = preview.copy()
            except Exception:
                logger.debug(
                    "Heuristic preview load failed via pipeline for %s",
                    image_path,
                    exc_info=True,
                )
        if preview is None:
            try:
                with Image.open(image_path) as raw:
                    prepared = ImageOps.exif_transpose(raw)
                    preview = prepared.convert("RGB").copy()
            except Exception:
                logger.debug(
                    "Heuristic preview load failed from disk for %s",
                    image_path,
                    exc_info=True,
                )
                return None
        try:
            prepared = _downscale_image(preview, self._preview_max_edge)
            if prepared is preview:
                # Ensure caller gets a live image even if no resize was needed.
                prepared = prepared.copy()
            return prepared
        finally:
            try:
                preview.close()
            except Exception:
                pass

    def _estimate_sharpness(self, image: Image.Image) -> float:
        if cv2 is None:
            return 0.5
        try:
            gray = np.array(image.convert("L"))
            variance = float(cv2.Laplacian(gray, cv2.CV_64F).var())
            normalized = variance / HEURISTIC_SHARPNESS_NORMALIZER
            return max(0.0, min(1.0, normalized))
        except Exception:
            logger.debug("Sharpness heuristic failed", exc_info=True)
            return 0.5

    @staticmethod
    def _estimate_exposure_balance(image: Image.Image) -> float:
        gray = image.convert("L")
        stats = ImageStat.Stat(gray)
        mean_luma = stats.mean[0] / 255.0 if stats.mean else 0.5
        stddev = (
            stats.stddev[0] if stats.stddev else 0.0
        ) / HEURISTIC_CONTRAST_NORMALIZER
        brightness_penalty = min(1.0, abs(mean_luma - 0.5) * 2.0)
        score = max(0.0, 1.0 - brightness_penalty)
        contrast_bonus = max(0.0, min(1.0, stddev))
        return 0.6 * score + 0.4 * contrast_bonus

    @staticmethod
    def _estimate_histogram_balance(image: Image.Image) -> float:
        gray = image.convert("L")
        hist = gray.histogram()
        total = sum(hist)
        if total <= 0:
            return 0.5
        tail_bins = 6
        shadow_ratio = sum(hist[:tail_bins]) / total
        highlight_ratio = sum(hist[-tail_bins:]) / total
        clipping = shadow_ratio + highlight_ratio
        return max(0.0, 1.0 - clipping * 3.0)

    def _get_eye_analyzer(self) -> Optional["EyeStateAnalyzer"]:
        if self._eye_detection_disabled:
            return None
        analyzer = getattr(self._eye_analyzer_local, "instance", None)
        if analyzer is not None:
            return analyzer
        try:
            analyzer = EyeStateAnalyzer(max_faces=1)
        except Exception:
            logger.warning(
                "EyeStateAnalyzer unavailable; heuristic stage will skip eye checks."
            )
            self._eye_detection_disabled = True
            return None
        self._eye_analyzer_local.instance = analyzer
        return analyzer

    def _estimate_eye_openness(self, image: Image.Image) -> float:
        analyzer = self._get_eye_analyzer()
        if analyzer is None:
            return 0.5
        try:
            probability = analyzer.predict_open_probability(image)
            if probability is None:
                return 0.5
            return max(0.0, min(1.0, float(probability)))
        except Exception:
            logger.debug("Eye-state heuristic failed", exc_info=True)
            return 0.5

    def evaluate(self, image_path: str) -> Optional[HeuristicCandidate]:
        preview = self._load_preview(image_path)
        if preview is None:
            return None
        try:
            sharpness = self._estimate_sharpness(preview)
            exposure = self._estimate_exposure_balance(preview)
            histogram_balance = self._estimate_histogram_balance(preview)
            eye_openness = self._estimate_eye_openness(preview)
            score = (
                0.5 * sharpness
                + 0.2 * exposure
                + 0.15 * histogram_balance
                + 0.15 * eye_openness
            )
            return HeuristicCandidate(
                image_path=image_path,
                score=score,
                sharpness=sharpness,
                exposure_balance=exposure,
                histogram_balance=histogram_balance,
                eye_openness=eye_openness,
            )
        finally:
            try:
                preview.close()
            except Exception:
                pass


def _load_font(image_size: Tuple[int, int]) -> ImageFont.ImageFont:
    longer_side = max(image_size)
    font_size = max(24, int(longer_side * 0.08))
    try:
        return ImageFont.truetype("DejaVuSans-Bold.ttf", font_size)
    except Exception:
        return ImageFont.load_default()


def _annotate_image(image: Image.Image, label: str) -> Image.Image:
    annotated = image.copy()
    if annotated.mode != "RGBA":
        annotated = annotated.convert("RGBA")
    draw = ImageDraw.Draw(annotated)
    font = _load_font(annotated.size)
    text_bbox = draw.textbbox((0, 0), label, font=font)
    text_width = text_bbox[2] - text_bbox[0]
    text_height = text_bbox[3] - text_bbox[1]
    padding = max(10, text_height // 2)
    position = (padding, padding)
    background_box = (
        position[0] - padding // 2,
        position[1] - padding // 2,
        position[0] + text_width + padding // 2,
        position[1] + text_height + padding // 2,
    )
    draw.rectangle(background_box, fill=(0, 0, 0, 180))
    draw.text(position, label, font=font, fill=(255, 255, 255, 255))
    return annotated


def _image_to_base64(image: Image.Image) -> str:
    buf = io.BytesIO()
    image.save(buf, format="PNG")
    return base64.b64encode(buf.getvalue()).decode("utf-8")


def _downscale_image(
    image: Image.Image, max_edge: int = MAX_LOCAL_ANALYSIS_EDGE
) -> Image.Image:
    width, height = image.size
    longest = max(width, height)
    if longest <= max_edge:
        return image
    scale = max_edge / float(longest)
    new_size = (
        max(1, int(round(width * scale))),
        max(1, int(round(height * scale))),
    )
    return image.resize(new_size, _RESAMPLE_BEST)


class BaseBestShotStrategy:
    def __init__(
        self,
        models_root: Optional[str],
        image_pipeline,
        llm_config: Optional[LLMConfig] = None,
        status_callback: Optional[Callable[[str], None]] = None,
    ) -> None:
        self.models_root = models_root
        self.image_pipeline = image_pipeline
        self.llm_config = llm_config
        self._status_callback = status_callback

    @property
    def max_workers(self) -> int:
        return 4

    def rank_cluster(
        self, cluster_id: int, image_paths: Sequence[str]
    ) -> List[Dict[str, object]]:
        raise NotImplementedError

    def rate_image(self, image_path: str) -> Optional[Dict[str, object]]:
        raise NotImplementedError

    def shutdown(self) -> None:
        """Clean up resources once processing is done."""

    def validate_connection(self) -> None:
        """Optional connectivity check before work begins."""


def _normalize_for_rating(value: float, *, lower: float, upper: float) -> float:
    if upper <= lower:
        return 0.0
    normalized = (value - lower) / (upper - lower)
    return max(0.0, min(1.0, normalized))


def _map_score_to_rating(normalized_score: float) -> int:
    thresholds = [0.22, 0.42, 0.62, 0.8]
    for idx, threshold in enumerate(thresholds, start=1):
        if normalized_score < threshold:
            return idx
    return 5


def _compute_quality_rating(result) -> Tuple[int, float]:
    def _is_number(value: object) -> bool:
        return isinstance(value, (int, float)) and not isinstance(value, bool)

    quality_score: Optional[float] = None

    composite = getattr(result, "composite_score", None)
    if _is_number(composite):
        composite_value = float(composite)
        if math.isfinite(composite_value):
            quality_score = composite_value

    if quality_score is None:
        metrics = getattr(result, "metrics", {}) or {}
        metric_values = [
            float(value) for value in metrics.values() if _is_number(value)
        ]
        if metric_values:
            quality_score = sum(metric_values) / len(metric_values)

    if quality_score is None:
        samples: List[float] = []
        raw = getattr(result, "raw_metrics", {}) or {}
        musiq_raw = raw.get("musiq_raw")
        if _is_number(musiq_raw):
            samples.append(_normalize_for_rating(musiq_raw, lower=25.0, upper=85.0))
        liqe_raw = raw.get("liqe_raw")
        if _is_number(liqe_raw):
            samples.append(_normalize_for_rating(liqe_raw, lower=30.0, upper=90.0))
        maniqa_raw = raw.get("maniqa_raw")
        if _is_number(maniqa_raw):
            samples.append(_normalize_for_rating(maniqa_raw, lower=0.25, upper=0.85))
        if samples:
            quality_score = sum(samples) / len(samples)

    if quality_score is None:
        quality_score = 0.0

    quality_score = max(0.0, min(1.0, float(quality_score)))
    rating = _map_score_to_rating(quality_score)
    return rating, quality_score


class LocalBestShotStrategy(BaseBestShotStrategy):
    def __init__(
        self,
        models_root,
        image_pipeline,
        llm_config=None,
        status_callback: Optional[Callable[[str], None]] = None,
    ) -> None:
        super().__init__(
            models_root, image_pipeline, llm_config, status_callback=status_callback
        )
        self._thread_local = threading.local()
        self._device_hint = get_preferred_torch_device()
        self._analysis_profile = _determine_local_analysis_profile()
        self._max_local_analysis_edge = self._analysis_profile.max_edge
        self._metric_specs = self._analysis_profile.metric_specs
        self._prefilter_stage = FastHeuristicStage(image_pipeline)
        responsive_profile = self._analysis_profile is _RESPONSIVE_ANALYSIS_PROFILE
        min_prefilter_workers = 1 if responsive_profile else 2
        max_prefilter_workers = 2 if responsive_profile else 4
        self._prefilter_workers = max(
            1,
            calculate_max_workers(
                min_workers=min_prefilter_workers, max_workers=max_prefilter_workers
            ),
        )
        logger.info(
            "Local best-shot strategy targeting torch device '%s'", self._device_hint
        )

    @property
    def max_workers(self) -> int:
        return calculate_max_workers(min_workers=1, max_workers=8)

    def _get_selector(self) -> BestPhotoSelector:
        selector = getattr(self._thread_local, "selector", None)
        if selector is None:
            # Use image pipeline for better RAW and format support
            image_loader = self._create_image_loader() if self.image_pipeline else None
            selector = BestPhotoSelector(
                models_root=self.models_root,
                image_loader=image_loader,
                status_callback=self._status_callback,
                device=self._device_hint,
                metric_specs=self._metric_specs,
            )
            self._thread_local.selector = selector
        return selector

    def _create_image_loader(self):
        """Create an image loader using the app pipeline + downscaling for efficiency."""

        def pipeline_image_loader(image_path: str) -> Image.Image:
            try:
                # Use image pipeline to get preview (handles RAW files properly)
                preview = self.image_pipeline.get_preview_image(image_path)
                if preview is not None:
                    return self._prepare_image(preview, image_path)
            except Exception as exc:
                logger.warning("Image pipeline failed for %s: %s", image_path, exc)

            # Fallback to direct loading for standard formats only
            ext = os.path.splitext(image_path)[1].lower()
            if ext in {
                ".jpg",
                ".jpeg",
                ".png",
                ".bmp",
                ".gif",
                ".tiff",
                ".tif",
                ".webp",
            }:
                try:
                    with Image.open(image_path) as img:
                        prepared = ImageOps.exif_transpose(img)
                        return self._prepare_image(prepared, image_path)
                except Exception as exc:
                    logger.error(
                        "Failed to load standard format image %s: %s", image_path, exc
                    )
            else:
                logger.error(
                    "Unsupported format for local AI analysis: %s (%s)", ext, image_path
                )

            raise RuntimeError(f"Cannot load image for local AI analysis: {image_path}")

        return pipeline_image_loader

    def _prepare_image(self, image: Image.Image, source_path: str) -> Image.Image:
        prepared = image.copy()
        if prepared.mode != "RGB":
            prepared = prepared.convert("RGB")
        prepared = _downscale_image(prepared, self._max_local_analysis_edge)
        prepared.info.setdefault("source_path", source_path)
        prepared.info.setdefault("region", "full")
        return prepared

    def _evaluate_prefilter_candidates(
        self, stage: FastHeuristicStage, image_paths: Sequence[str]
    ) -> Dict[str, Optional[HeuristicCandidate]]:
        results: Dict[str, Optional[HeuristicCandidate]] = {}
        if not image_paths:
            return results
        worker_count = min(self._prefilter_workers, len(image_paths))
        if worker_count <= 1:
            for path in image_paths:
                try:
                    results[path] = stage.evaluate(path)
                except Exception:
                    logger.debug(
                        "Heuristic evaluation failed for %s", path, exc_info=True
                    )
                    results[path] = None
        else:
            with ThreadPoolExecutor(max_workers=worker_count) as executor:
                future_map = {
                    executor.submit(stage.evaluate, path): path for path in image_paths
                }
                for future in as_completed(future_map):
                    path = future_map[future]
                    try:
                        results[path] = future.result()
                    except Exception:
                        logger.debug(
                            "Heuristic evaluation raised unexpectedly for %s",
                            path,
                            exc_info=True,
                        )
                        results[path] = None
        for path in image_paths:
            results.setdefault(path, None)
        return results

    def _prefilter_cluster(
        self, cluster_id: int, image_paths: Sequence[str]
    ) -> Tuple[List[str], Dict[str, HeuristicCandidate]]:
        if len(image_paths) < PREFILTER_MIN_CLUSTER_SIZE:
            return list(image_paths), {}

        limit = min(PREFILTER_MAX_CANDIDATES, len(image_paths))
        if limit >= len(image_paths):
            return list(image_paths), {}

        stage = self._prefilter_stage
        if stage is None:
            return list(image_paths), {}

        evaluations = self._evaluate_prefilter_candidates(stage, image_paths)
        scored: List[HeuristicCandidate] = []
        fallbacks: List[str] = []
        for path in image_paths:
            candidate = evaluations.get(path)
            if candidate is None:
                fallbacks.append(path)
                continue
            scored.append(candidate)

        if not scored:
            return list(image_paths), {}

        scored.sort(key=lambda c: c.score, reverse=True)
        selected = [candidate.image_path for candidate in scored[:limit]]
        if len(selected) < limit:
            for path in fallbacks:
                if path not in selected:
                    selected.append(path)
                if len(selected) >= limit:
                    break

        if len(selected) < len(image_paths):
            logger.info(
                "Heuristic prefilter reduced cluster %s from %d to %d candidates",
                cluster_id,
                len(image_paths),
                len(selected),
            )

        info_map = {candidate.image_path: candidate for candidate in scored}
        return selected, info_map

    def rank_cluster(
        self, cluster_id: int, image_paths: Sequence[str]
    ) -> List[Dict[str, object]]:
        logger.info(
            f"Local AI ranking cluster {cluster_id} with {len(image_paths)} images using local models"
        )
        candidate_paths, prefilter_map = self._prefilter_cluster(
            cluster_id, image_paths
        )
        if len(candidate_paths) != len(image_paths):
            logger.info(
                "Cluster %s trimmed to %d candidate(s) prior to IQA",
                cluster_id,
                len(candidate_paths),
            )
        worker_count = min(self.max_workers, len(candidate_paths))
        if worker_count > 1:
            logger.debug(
                "Parallel IQA scoring enabled for cluster %s with %d worker(s)",
                cluster_id,
                worker_count,
            )
            result_objects = self._rank_images_parallel(candidate_paths, worker_count)
        else:
            selector = self._get_selector()
            result_objects = selector.rank_images(candidate_paths)
        ranked_results: List[Dict[str, object]] = []
        for result in result_objects:
            payload = result.to_dict()
            info = prefilter_map.get(payload.get("image_path"))
            if info:
                payload["prefilter"] = info.as_dict()
            if logger.isEnabledFor(logging.DEBUG):
                image_name = os.path.basename(payload.get("image_path", ""))
                composite = payload.get("composite_score", 0.0)
                metrics = payload.get("metrics") or {}
                metric_summary = ", ".join(
                    f"{name.upper()} {value:.3f}"
                    for name, value in sorted(metrics.items())
                    if isinstance(value, (int, float))
                )
                eye_value = metrics.get("eyes_open")
                if isinstance(eye_value, (int, float)) and "EYES_OPEN" not in metric_summary:
                    metric_summary = (
                        f"{metric_summary}, EYES_OPEN {eye_value:.3f}"
                        if metric_summary
                        else f"EYES_OPEN {eye_value:.3f}"
                    )
                prefilter = payload.get("prefilter") or {}
                if prefilter:
                    prefilter_summary = ", ".join(
                        f"{key}={value:.3f}" if isinstance(value, (int, float)) else f"{key}={value}"
                        for key, value in sorted(prefilter.items())
                    )
                    metric_summary = (
                        f"{metric_summary} | prefilter: {prefilter_summary}"
                        if metric_summary
                        else f"prefilter: {prefilter_summary}"
                    )
                logger.debug(
                    "Cluster %s candidate %s -> composite %.4f%s",
                    cluster_id,
                    image_name or payload.get("image_path"),
                    composite,
                    f" ({metric_summary})" if metric_summary else "",
                )
            ranked_results.append(payload)
        if ranked_results:
            logger.info(
                f"Completed local AI ranking for cluster {cluster_id}. Best image: {os.path.basename(ranked_results[0]['image_path'])}"
            )
        return ranked_results

    def _rank_images_parallel(
        self, image_paths: Sequence[str], worker_count: int
    ) -> List[BestShotResult]:
        results: List[BestShotResult] = []

        def _evaluate(path: str) -> Optional[BestShotResult]:
            selector = self._get_selector()
            return selector.analyze_image(path)

        with ThreadPoolExecutor(max_workers=worker_count) as executor:
            futures = {executor.submit(_evaluate, path): path for path in image_paths}
            for future in as_completed(futures):
                path = futures[future]
                try:
                    result = future.result()
                except Exception as exc:
                    logger.warning(
                        "Parallel IQA scoring failed for %s: %s", path, exc, exc_info=True
                    )
                    continue
                if result:
                    results.append(result)
        results.sort(key=lambda r: r.composite_score, reverse=True)
        return results

    def rate_image(self, image_path: str) -> Optional[Dict[str, object]]:
        logger.info(f"Local AI rating image: {os.path.basename(image_path)}")
        selector = self._get_selector()
        results = selector.rank_images([image_path])
        if not results:
            return None
        result = results[0]
        rating, quality_score = _compute_quality_rating(result)
        logger.info(
            "Local AI rated %s as %d/5 (quality score %.3f)",
            os.path.basename(image_path),
            rating,
            quality_score,
        )
        return {
            "image_path": image_path,
            "rating": rating,
            "score": quality_score,
            "metrics": result.metrics,
        }


class LLMBestShotStrategy(BaseBestShotStrategy):
    def __init__(
        self,
        models_root,
        image_pipeline,
        llm_config: LLMConfig,
        status_callback: Optional[Callable[[str], None]] = None,
    ) -> None:
        super().__init__(
            models_root, image_pipeline, llm_config, status_callback=status_callback
        )
        try:
            from openai import OpenAI  # type: ignore
        except ImportError as exc:  # pragma: no cover
            raise RuntimeError(
                "openai package not installed. Install it to use LLM best-shot engine."
            ) from exc

        self._timeout = llm_config.timeout
        self._base_url = llm_config.base_url or DEFAULT_OPENAI_BASE_URL
        client_kwargs: Dict[str, object] = {
            "base_url": self._base_url,
            "timeout": self._timeout,
        }
        if llm_config.api_key and llm_config.api_key != DEFAULT_OPENAI_API_KEY:
            client_kwargs["api_key"] = llm_config.api_key
        self._client = OpenAI(**client_kwargs)
        self._model = llm_config.model
        self._max_tokens = llm_config.max_tokens
        self._prompt_template = llm_config.best_shot_prompt
        self._rating_prompt = llm_config.rating_prompt
        self._lock = threading.Lock()
        self._worker_count = llm_config.max_workers

    @property
    def max_workers(self) -> int:
        return max(1, self._worker_count)

    def _with_timeout(self, timeout_seconds: int):
        client = self._client
        if hasattr(client, "with_options"):
            try:
                return client.with_options(timeout=timeout_seconds)
            except Exception:
                return client
        return client

    def _load_preview(self, image_path: str) -> Image.Image:
        """Load image as RGB preview, ensuring compatibility with AI services.

        Always uses the image pipeline to handle RAW files and other formats properly,
        as AI services typically don't support RAW formats natively.
        """
        preview = None
        if self.image_pipeline is not None:
            try:
                preview = self.image_pipeline.get_preview_image(image_path)
                if preview is not None and preview.mode != "RGB":
                    preview = preview.convert("RGB")
            except Exception:
                logger.exception("Preview generation failed for %s", image_path)

        if preview is None:
            try:
                # Fallback for standard formats only - avoid RAW files
                ext = os.path.splitext(image_path)[1].lower()
                if ext in {
                    ".jpg",
                    ".jpeg",
                    ".png",
                    ".bmp",
                    ".gif",
                    ".tiff",
                    ".tif",
                    ".webp",
                }:
                    preview = Image.open(image_path).convert("RGB")
                else:
                    raise RuntimeError(
                        f"Unsupported format for AI analysis: {ext}. Preview generation required."
                    )
            except Exception as exc:
                logger.error("Failed to load image %s: %s", image_path, exc)
                raise RuntimeError(f"Cannot load image for AI analysis: {exc}") from exc

        return preview

    def _build_messages(
        self,
        prompt: str,
        labelled_images: List[Tuple[int, str]],
        *,
        system_prompt: Optional[str] = None,
    ) -> List[Dict[str, object]]:
        content: List[Dict[str, object]] = [{"type": "text", "text": prompt}]
        for index, b64 in labelled_images:
            content.append(
                {
                    "type": "image_url",
                    "image_url": {
                        "url": f"data:image/png;base64,{b64}",
                        "detail": "high",
                    },
                }
            )
        messages: List[Dict[str, object]] = []
        if system_prompt:
            messages.append({"role": "system", "content": system_prompt})
        messages.append({"role": "user", "content": content})
        return messages

    def _call_llm(
        self,
        messages: List[Dict[str, object]],
        *,
        tools: Optional[List[Dict[str, object]]] = None,
        tool_choice: Optional[str] = None,
        max_tokens: Optional[int] = None,
    ):
        with self._lock:
            try:
                kwargs: Dict[str, object] = {
                    "model": self._model,
                    "messages": messages,
                    "max_tokens": max(max_tokens or self._max_tokens, 256),
                    "temperature": 0.3,
                }
                if tools is not None:
                    kwargs["tools"] = tools
                if tool_choice is not None:
                    kwargs["tool_choice"] = tool_choice
                response = self._client.chat.completions.create(**kwargs)
            except Exception as exc:
                raise RuntimeError(
                    f"LLM request failed for model '{self._model}' at {self._base_url}: {exc}"
                ) from exc
        message = response.choices[0].message
        content = getattr(message, "content", None) or ""
        return message, content

    def validate_connection(self) -> None:
        probe_timeout = min(max(5, int(self._timeout * 0.25)), max(self._timeout, 5))
        client = self._with_timeout(probe_timeout)
        try:
            response = client.models.list()
        except Exception as exc:
            raise RuntimeError(
                f"Unable to reach LLM endpoint at {self._base_url}: {exc}"
            ) from exc
        data = getattr(response, "data", None)
        model_ids: Set[str] = set()
        if data:
            for entry in data:
                if isinstance(entry, dict):
                    identifier = entry.get("id") or entry.get("name")
                else:
                    identifier = getattr(entry, "id", None) or getattr(
                        entry, "name", None
                    )
                if identifier:
                    model_ids.add(str(identifier))

        if not model_ids:
            raise RuntimeError(
                "LLM endpoint responded but returned zero models; ensure your server exposes an active model."
            )
        if self._model not in model_ids:
            raise RuntimeError(
                f"LLM endpoint reachable, but model '{self._model}' not found. Available models: {', '.join(sorted(model_ids))}."
            )

    def rank_cluster(
        self, cluster_id: int, image_paths: Sequence[str]
    ) -> List[Dict[str, object]]:
        logger.info(
            f"AI ranking cluster {cluster_id} with {len(image_paths)} images using LLM strategy"
        )
        if len(image_paths) <= 1:
            normalized_results: List[Dict[str, object]] = []
            for path in image_paths:
                normalized_results.append(
                    {
                        "image_path": path,
                        "composite_score": 1.0,
                        "metrics": {"llm_selected": True},
                        "analysis": "",
                    }
                )
            if normalized_results:
                logger.info(
                    "Cluster %s has a single image; skipping LLM call.", cluster_id
                )
            return normalized_results

        images = []
        labelled_payloads: List[Tuple[int, str]] = []
        for idx, path in enumerate(image_paths, start=1):
            preview = self._load_preview(path)
            annotated = _annotate_image(preview, str(idx))
            labelled_payloads.append((idx, _image_to_base64(annotated)))
            images.append((idx, path))

        prompt = self._prompt_template.format(image_count=len(image_paths))
        messages = self._build_messages(prompt, labelled_payloads)

        logger.info(f"Sending {len(image_paths)} images to LLM for analysis")
        _, analysis = self._call_llm(messages)
        logger.info(f"Received LLM analysis response (length: {len(analysis)} chars)")

        best_match = re.search(r"Best Image\s*:\s*\[?\s*(\d+)", analysis, re.IGNORECASE)
        best_index = None
        if best_match:
            try:
                candidate = int(best_match.group(1))
                if 1 <= candidate <= len(image_paths):
                    best_index = candidate
                    logger.info(
                        f"LLM selected image {best_index} as best from {len(image_paths)} options"
                    )
            except ValueError:
                best_index = None

        if best_index is None:
            logger.warning(
                f"Could not parse best image selection from LLM response: {analysis[:200]}..."
            )

        ranked: List[Dict[str, object]] = []
        for idx, path in images:
            score = 1.0 if idx == best_index else 0.5
            ranked.append(
                {
                    "image_path": path,
                    "composite_score": score,
                    "metrics": {"llm_selected": idx == best_index},
                    "analysis": analysis,
                }
            )

        if best_index is not None:
            ranked.sort(key=lambda item: item["metrics"]["llm_selected"], reverse=True)
            logger.info(
                f"Completed AI ranking for cluster {cluster_id}. Best image: {os.path.basename(ranked[0]['image_path'])}"
            )
        else:
            logger.warning(
                f"Completed AI ranking for cluster {cluster_id} but no clear winner identified"
            )
        return ranked

    def rate_image(self, image_path: str) -> Optional[Dict[str, object]]:
        logger.info(f"AI rating image: {os.path.basename(image_path)}")

        preview = self._load_preview(image_path)
        annotated = _annotate_image(preview, "1")
        b64 = _image_to_base64(annotated)
        prompt = self._rating_prompt
        system_prompt = (
            "You are a photography scientist performing repeatable image quality audits. "
            "Use the provided evaluation rubric and respond only by calling the provided tool."
        )
        messages = self._build_messages(
            prompt,
            [(1, b64)],
            system_prompt=system_prompt,
        )
        tools = [
            {
                "type": "function",
                "function": {
                    "name": "record_photo_quality",
                    "description": "Store deterministic quality scores for a single photograph.",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "overall_rating": {
                                "type": "integer",
                                "minimum": 1,
                                "maximum": 5,
                                "description": "Overall star rating derived from the weighted quality score (1-5).",
                            },
                            "overall_quality": {
                                "type": "number",
                                "minimum": 0,
                                "maximum": 100,
                                "description": "Weighted quantitative quality score (0-100).",
                            },
                            "confidence": {
                                "type": "string",
                                "enum": ["low", "medium", "high"],
                                "description": "Confidence in the rating after evaluating visual evidence.",
                            },
                            "score_breakdown": {
                                "type": "object",
                                "properties": {
                                    "sharpness": {
                                        "type": "integer",
                                        "minimum": 0,
                                        "maximum": 100,
                                    },
                                    "noise_control": {
                                        "type": "integer",
                                        "minimum": 0,
                                        "maximum": 100,
                                    },
                                    "exposure_balance": {
                                        "type": "integer",
                                        "minimum": 0,
                                        "maximum": 100,
                                    },
                                    "color_accuracy": {
                                        "type": "integer",
                                        "minimum": 0,
                                        "maximum": 100,
                                    },
                                    "composition_balance": {
                                        "type": "integer",
                                        "minimum": 0,
                                        "maximum": 100,
                                    },
                                    "subject_expression": {
                                        "type": "integer",
                                        "minimum": 0,
                                        "maximum": 100,
                                    },
                                },
                                "required": [
                                    "sharpness",
                                    "noise_control",
                                    "exposure_balance",
                                    "color_accuracy",
                                    "composition_balance",
                                    "subject_expression",
                                ],
                            },
                            "notes": {
                                "type": "string",
                                "description": "One concise sentence summarising the key strengths and weaknesses.",
                            },
                        },
                        "required": [
                            "overall_rating",
                            "overall_quality",
                            "confidence",
                            "score_breakdown",
                            "notes",
                        ],
                    },
                },
            }
        ]
        tool_choice = "required"

        logger.debug("Sending image to LLM for rating analysis")
        message, freeform_analysis = self._call_llm(
            messages,
            tools=tools,
            tool_choice=tool_choice,
        )
        analysis = freeform_analysis
        structured_payload: Dict[str, Any] = {}
        tool_calls = getattr(message, "tool_calls", None) or []
        if tool_calls:
            try:
                raw_args = tool_calls[0].function.arguments  # type: ignore[attr-defined]
                structured_payload = json.loads(raw_args) if raw_args else {}
            except Exception:
                logger.exception("Failed to parse AI rating tool output")
        else:
            raise RuntimeError(
                "AI rating response did not include the required tool call."
            )

        rating = structured_payload.get("overall_rating")
        if rating is not None:
            rating = max(1, min(5, rating))
            logger.info(f"AI rated {os.path.basename(image_path)} as {rating}/5")
        else:
            snippet = (analysis or "").strip()[:200]
            logger.warning(
                "AI rating missing or invalid for %s; response sample: %s",
                os.path.basename(image_path),
                snippet or "<empty response>",
            )
        if structured_payload and not analysis:
            breakdown = structured_payload.get("score_breakdown", {})
            breakdown_parts = [
                f"{name.replace('_', ' ')} {value}" for name, value in breakdown.items()
            ]
            notes = structured_payload.get("notes")
            confidence = structured_payload.get("confidence")
            summary_bits = []
            if breakdown_parts:
                summary_bits.append(" | ".join(breakdown_parts))
            if notes:
                summary_bits.append(notes)
            if confidence:
                summary_bits.append(f"confidence: {confidence}")
            analysis = " ".join(summary_bits)

        payload = {
            "image_path": image_path,
            "rating": rating,
            "analysis": analysis,
        }
        if structured_payload:
            payload["quality_scores"] = structured_payload
        return payload


def create_best_shot_strategy(
    engine: Optional[str] = None,
    *,
    models_root: Optional[str] = None,
    image_pipeline=None,
    llm_config: Optional[LLMConfig] = None,
    status_callback: Optional[Callable[[str], None]] = None,
) -> BaseBestShotStrategy:
    """Create AI strategy for image analysis.

    Both LLM and Local strategies now properly support RAW images by using
    the image_pipeline to generate RGB previews suitable for AI analysis.
    """
    engine_name = (engine or get_best_shot_engine() or "local").lower()
    logger.info(f"Creating AI strategy with engine: {engine_name}")
    if engine_name == BestShotEngine.LLM.value:
        config = llm_config or LLMConfig(**get_openai_config())
        logger.info(f"Using LLM strategy with endpoint: {config.base_url}")
        return LLMBestShotStrategy(
            models_root,
            image_pipeline,
            config,
            status_callback=status_callback,
        )
    logger.info("Using local model strategy")
    return LocalBestShotStrategy(
        models_root,
        image_pipeline,
        llm_config,
        status_callback=status_callback,
    )


__all__ = [
    "BestShotEngine",
    "LLMBestShotStrategy",
    "LocalBestShotStrategy",
    "LocalAnalysisProfile",
    "create_best_shot_strategy",
    "LLMConfig",
    "select_local_analysis_profile",
]

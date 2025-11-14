"""Best-shot ranking powered by MUSIQ, MANIQA, and LIQE.

This leans on modern no-reference IQA models provided by `pyiqa`.
Each metric produces an independent
quality estimate which we normalise and blend to obtain a composite score for
every image in a similarity cluster.
"""

from __future__ import annotations

import logging
import os
import threading
from dataclasses import dataclass, field, replace
from typing import Any, Callable, Dict, List, Optional, Sequence, Tuple
from urllib.parse import urlparse

import numpy as np
from PIL import Image, ImageOps

from core.app_settings import get_local_best_shot_constants

from core.numpy_compat import ensure_numpy_sctypes

logger = logging.getLogger(__name__)

PROJECT_ROOT = os.path.abspath(
    os.path.join(os.path.dirname(__file__), "..", "..", "..")
)
DEFAULT_MODELS_ROOT = os.environ.get(
    "PHOTOSORT_MODELS_DIR", os.path.join(PROJECT_ROOT, "models")
)

_PYIQA_DOWNLOAD_LOCK = threading.Lock()
_METRIC_CACHE_LOCK = threading.Lock()
_METRIC_CACHE: Dict[Tuple[str, str], Tuple[Any, threading.Lock]] = {}
DEFAULT_EYE_OPEN_WEIGHT = 0.35

if hasattr(Image, "Resampling"):
    _RESAMPLE_LANCZOS = Image.Resampling.LANCZOS
else:  # pragma: no cover - Pillow < 10 fallback
    _RESAMPLE_LANCZOS = Image.LANCZOS

MANIQA_SAFE_INPUT = 224

ensure_numpy_sctypes()


class EyeStateAnalyzer:
    """Estimates eye openness using MediaPipe Face Mesh landmarks."""

    _LEFT_LANDMARKS = {
        "upper": 159,
        "lower": 145,
        "outer": 33,
        "inner": 133,
    }
    _RIGHT_LANDMARKS = {
        "upper": 386,
        "lower": 374,
        "outer": 263,
        "inner": 362,
    }

    def __init__(self, max_faces: int = 1) -> None:
        import mediapipe as mp  # type: ignore

        self._mp_face_mesh = mp.solutions.face_mesh.FaceMesh(
            static_image_mode=True,
            max_num_faces=max_faces,
            refine_landmarks=True,
            min_detection_confidence=0.5,
            min_tracking_confidence=0.5,
        )

    def predict_open_probability(self, image: Image.Image) -> Optional[float]:
        arr = np.array(image.convert("RGB"))
        if arr.ndim != 3 or arr.shape[2] != 3:
            return None
        # MediaPipe expects writeable flag False for performance
        arr.flags.writeable = False
        results = self._mp_face_mesh.process(arr)
        if not results.multi_face_landmarks:
            return None
        height, width, _ = arr.shape
        scores: List[float] = []
        for face_landmarks in results.multi_face_landmarks[:1]:
            ratio_left = self._compute_ratio(
                face_landmarks.landmark, width, height, self._LEFT_LANDMARKS
            )
            ratio_right = self._compute_ratio(
                face_landmarks.landmark, width, height, self._RIGHT_LANDMARKS
            )
            for ratio in (ratio_left, ratio_right):
                if ratio is not None:
                    scores.append(self._ratio_to_probability(ratio))
        if not scores:
            return None
        return float(sum(scores) / len(scores))

    @staticmethod
    def _compute_ratio(
        landmarks, width: int, height: int, indices: Dict[str, int]
    ) -> Optional[float]:
        max_index = max(indices.values(), default=-1)
        if max_index >= 0 and hasattr(landmarks, "__len__"):
            try:
                if len(landmarks) <= max_index:
                    # Not enough landmarks to satisfy the requested indices.
                    return None
            except TypeError:
                return None

        try:
            upper = landmarks[indices["upper"]]
            lower = landmarks[indices["lower"]]
            outer = landmarks[indices["outer"]]
            inner = landmarks[indices["inner"]]
        except (IndexError, KeyError):  # pragma: no cover - defensive guard
            return None
        vertical = abs(upper.y - lower.y)
        horizontal = abs(outer.x - inner.x)
        if horizontal <= 0:
            return None
        return vertical / horizontal

    @staticmethod
    def _ratio_to_probability(ratio: float) -> float:
        closed_threshold = 0.18
        open_threshold = 0.28
        if ratio <= closed_threshold:
            return 0.0
        if ratio >= open_threshold:
            return 1.0
        span = open_threshold - closed_threshold
        return (ratio - closed_threshold) / span if span > 0 else 0.0


def _clamp(value: float, *, minimum: float = 0.0, maximum: float = 1.0) -> float:
    return float(max(minimum, min(maximum, value)))


def _normalize(value: float, lower: float, upper: float) -> float:
    if upper <= lower:
        return 0.0
    return _clamp((value - lower) / (upper - lower))


_LOCAL_BEST_SHOT_SETTINGS = get_local_best_shot_constants()


def _pil_to_tensor(image: Image.Image):
    try:
        import torch  # type: ignore
    except ImportError as exc:  # pragma: no cover - torch is a hard dependency
        raise RuntimeError(
            "torch is required for IQA scoring. Install it via `pip install torch`."
        ) from exc

    cache_key = _LOCAL_BEST_SHOT_SETTINGS.tensor_cache_key
    cache_dict = image.info if isinstance(getattr(image, "info", None), dict) else None
    if cache_dict is not None:
        cached = cache_dict.get(cache_key)
        if cached is not None:
            return cached

    if image.mode != "RGB":
        image = image.convert("RGB")
    arr = np.asarray(image, dtype=np.float32)
    if arr.ndim == 2:  # grayscale image
        arr = np.stack([arr, arr, arr], axis=-1)
    if arr.shape[2] == 4:  # RGBA → RGB
        arr = arr[:, :, :3]
    arr = _pad_to_model_stride(arr, _LOCAL_BEST_SHOT_SETTINGS.model_stride)
    arr /= 255.0
    tensor = torch.from_numpy(arr).permute(2, 0, 1).unsqueeze(0).contiguous()
    if cache_dict is not None:
        cache_dict[cache_key] = tensor
    return tensor


def _pad_to_model_stride(arr: np.ndarray, stride: int) -> np.ndarray:
    if stride <= 1 or arr.ndim != 3:
        return arr
    height, width, _ = arr.shape
    pad_h = (-height) % stride
    pad_w = (-width) % stride
    if pad_h == 0 and pad_w == 0:
        return arr
    return np.pad(arr, ((0, pad_h), (0, pad_w), (0, 0)), mode="edge")


@dataclass(frozen=True)
class MetricSpec:
    name: str
    weight: float = 1.0
    min_score: float = 0.0
    max_score: float = 100.0


DEFAULT_METRIC_SPECS: Sequence[MetricSpec] = (
    MetricSpec(name="musiq", weight=0.45, min_score=0.0, max_score=100.0),
    MetricSpec(name="maniqa", weight=0.3, min_score=0.0, max_score=1.0),
    MetricSpec(name="liqe", weight=0.25, min_score=0.0, max_score=100.0),
)


MetricScoreFn = Callable[[Image.Image], float]


@dataclass
class IQAMetricRunner:
    spec: MetricSpec
    scorer: Optional[MetricScoreFn] = None
    device_hint: Optional[str] = None
    status_callback: Optional[Callable[[str], None]] = None

    def evaluate(self, image: Image.Image) -> Optional[Dict[str, float]]:
        scorer = self._ensure_scorer()
        if scorer is None:
            return None
        raw = float(scorer(image))
        normalized = _normalize(raw, self.spec.min_score, self.spec.max_score)
        return {
            "raw": raw,
            "normalized": normalized,
        }

    def _ensure_scorer(self) -> Optional[MetricScoreFn]:
        if self.scorer is None:
            self.scorer = self._build_pyiqa_scorer()
        return self.scorer

    def _build_pyiqa_scorer(self) -> MetricScoreFn:
        try:
            import torch  # type: ignore
            import pyiqa  # type: ignore
            import pyiqa.utils.download_util as download_util  # type: ignore
        except ImportError as exc:  # pragma: no cover - import guarded for tests
            raise RuntimeError(
                "pyiqa is required for the MUSIQ/MANIQA/LIQE pipeline."
                " Install it with `pip install pyiqa`."
            ) from exc

        if self.device_hint is not None:
            device = torch.device(self.device_hint)
        else:
            device = torch.device("cuda" if torch.cuda.is_available() else "cpu")

        cache_key = (self.spec.name, str(device))

        with _METRIC_CACHE_LOCK:
            cached = _METRIC_CACHE.get(cache_key)
            if cached is None:

                def _factory():
                    return pyiqa.create_metric(
                        self.spec.name,
                        device=device,
                        as_loss=False,
                    )

                if self.status_callback is None:
                    metric = _factory()
                else:
                    metric = self._with_download_notifications(
                        download_util, _factory
                    )
                metric.eval()
                cached = (metric, threading.Lock())
                _METRIC_CACHE[cache_key] = cached

        metric, metric_lock = cached
        torch_module = torch

        def _run_metric(input_tensor):
            with torch_module.no_grad():
                output = metric(input_tensor)
            return float(output.item()) if hasattr(output, "item") else float(output)

        def _tensor_on_device(source: Image.Image):
            return _pil_to_tensor(source).to(device)

        def _score(image: Image.Image) -> float:
            tensor = _tensor_on_device(image)
            with metric_lock:
                try:
                    value = _run_metric(tensor)
                except Exception as exc:
                    # MANIQA is known to raise a "list index out of range" error on some inputs; fallback by recropping.
                    if (
                        self.spec.name != "maniqa"
                        or "list index out of range" not in str(exc).lower()
                    ):
                        raise
                    logger.debug(
                        "MANIQA failed on %s; retrying with %dx%d center crop",
                        image.info.get("source_path", "<unknown>"),
                        MANIQA_SAFE_INPUT,
                        MANIQA_SAFE_INPUT,
                    )
                    safe_image = ImageOps.fit(
                        image,
                        (MANIQA_SAFE_INPUT, MANIQA_SAFE_INPUT),
                        method=_RESAMPLE_LANCZOS,
                        centering=(0.5, 0.5),
                    )
                    tensor = _tensor_on_device(safe_image)
                    value = _run_metric(tensor)
            return value

        return _score

    def _with_download_notifications(self, download_util, factory):
        original_loader = download_util.load_file_from_url

        def wrapped_loader(url, model_dir=None, progress=True, file_name=None):
            target_dir = model_dir or download_util.DEFAULT_CACHE_DIR
            filename = file_name or os.path.basename(urlparse(url).path)
            destination = os.path.abspath(os.path.join(target_dir, filename))
            should_notify = not os.path.exists(destination)
            if should_notify:
                self._report_download_status("start", destination)
            try:
                return original_loader(
                    url,
                    model_dir=model_dir,
                    progress=progress,
                    file_name=file_name,
                )
            finally:
                if should_notify:
                    self._report_download_status("done", destination)

        with _PYIQA_DOWNLOAD_LOCK:
            download_util.load_file_from_url = wrapped_loader
            try:
                return factory()
            finally:
                download_util.load_file_from_url = original_loader

    def _report_download_status(self, stage: str, destination: str) -> None:
        if not self.status_callback:
            return
        friendly_metric = self.spec.name.upper()
        target = os.path.expanduser(destination)
        if stage == "start":
            message = (
                f"Downloading {friendly_metric} weights to {target}. "
                "Progress also appears in the log window."
            )
        else:
            message = f"{friendly_metric} weights cached at {target}."
        self.status_callback(message)


@dataclass
class BestShotResult:
    image_path: str
    composite_score: float
    metrics: Dict[str, float] = field(default_factory=dict)
    raw_metrics: Dict[str, float] = field(default_factory=dict)

    def to_dict(self) -> Dict[str, object]:
        return {
            "image_path": self.image_path,
            "composite_score": self.composite_score,
            "metrics": self.metrics,
            "raw_metrics": self.raw_metrics,
        }


class BestPhotoSelector:
    """Ranks images by blending multiple no-reference IQA scores."""

    def __init__(
        self,
        face_detector=None,  # Legacy arguments kept for backwards compatibility
        eye_classifier=None,
        quality_model=None,
        models_root: Optional[str] = None,
        weights: Optional[Dict[str, float]] = None,
        image_loader: Optional[Callable[[str], Image.Image]] = None,
        focus_metric_fn=None,
        metric_specs: Optional[Sequence[MetricSpec]] = None,
        metric_factories: Optional[Dict[str, MetricScoreFn]] = None,
        device: Optional[str] = None,
        status_callback: Optional[Callable[[str], None]] = None,
        eye_state_analyzer: Optional[EyeStateAnalyzer] = None,
        enable_eye_detection: bool = True,
    ):
        if any(
            arg is not None
            for arg in (face_detector, eye_classifier, quality_model, focus_metric_fn)
        ):
            logger.debug(
                "Legacy detectors/classifiers are no longer used by the IQA pipeline."
            )

        self.models_root = models_root
        self._image_loader = image_loader or self._default_loader
        self._status_callback = status_callback

        base_specs = metric_specs or DEFAULT_METRIC_SPECS
        if not base_specs:
            raise ValueError("At least one metric specification is required")

        self._metric_runners: List[IQAMetricRunner] = []
        self._metric_weights: Dict[str, float] = {}
        factories = metric_factories or {}
        for spec in base_specs:
            adjusted_spec = (
                replace(spec, weight=weights.get(spec.name, spec.weight))
                if weights and spec.name in weights
                else spec
            )
            runner = IQAMetricRunner(
                spec=adjusted_spec,
                scorer=factories.get(spec.name),
                device_hint=device,
                status_callback=self._status_callback,
            )
            self._metric_runners.append(runner)
            self._metric_weights[adjusted_spec.name] = adjusted_spec.weight

        self._eye_analyzer: Optional[EyeStateAnalyzer] = None
        desired_eye_weight = (
            weights.get("eyes_open", DEFAULT_EYE_OPEN_WEIGHT)
            if weights and "eyes_open" in weights
            else DEFAULT_EYE_OPEN_WEIGHT
        )
        if enable_eye_detection and desired_eye_weight > 0:
            self._eye_analyzer = eye_state_analyzer or EyeStateAnalyzer()
            self._metric_weights["eyes_open"] = desired_eye_weight

    def rank_directory(
        self, directory: str, recursive: bool = False
    ) -> List[BestShotResult]:
        image_paths: List[str] = []
        if recursive:
            for root, _, files in os.walk(directory):  # pragma: no cover - convenience
                for filename in files:
                    if self._is_supported_file(filename):
                        image_paths.append(os.path.join(root, filename))
        else:
            for filename in os.listdir(directory):
                if self._is_supported_file(filename):
                    image_paths.append(os.path.join(directory, filename))
        return self.rank_images(sorted(image_paths))

    def rank_images(self, image_paths: Sequence[str]) -> List[BestShotResult]:
        results: List[BestShotResult] = []
        for path in image_paths:
            result = self.analyze_image(path)
            if result:
                results.append(result)
        return sorted(results, key=lambda r: r.composite_score, reverse=True)

    def analyze_image(self, image_path: str) -> Optional[BestShotResult]:
        return self._analyze_image(image_path)

    def _is_supported_file(self, filename: str) -> bool:
        _, ext = os.path.splitext(filename)
        return ext.lower() in {
            ".jpg",
            ".jpeg",
            ".png",
            ".bmp",
            ".tif",
            ".tiff",
            ".webp",
            ".heif",
            ".heic",
        }

    def _default_loader(self, image_path: str) -> Image.Image:
        with Image.open(image_path) as img:
            prepared = ImageOps.exif_transpose(img).convert("RGB")
            prepared.info["source_path"] = image_path
            return prepared.copy()

    def _analyze_image(self, image_path: str) -> Optional[BestShotResult]:
        try:
            image = self._image_loader(image_path)
            image.info.setdefault("source_path", image_path)
            logger.info("Analyzing image through BestPhotoSelector: %s", image_path)
        except Exception as exc:
            logger.error("Failed to load %s: %s", image_path, exc)
            return None

        metrics: Dict[str, float] = {}
        raw_metrics: Dict[str, float] = {}
        for runner in self._metric_runners:
            try:
                payload = runner.evaluate(image)
            except Exception as exc:  # pragma: no cover - defensive logging
                logger.warning(
                    "Metric %s failed for %s: %s",
                    runner.spec.name,
                    image_path,
                    exc,
                )
                continue
            if not payload:
                continue
            metrics[runner.spec.name] = payload["normalized"]
            raw_metrics[f"{runner.spec.name}_raw"] = payload["raw"]

        eye_prob = self._compute_eye_openness(image)
        if eye_prob is not None:
            metrics["eyes_open"] = eye_prob
            raw_metrics["eyes_open_probability"] = eye_prob
            logger.info(
                "Eye openness for %s: %.3f",
                os.path.basename(image_path),
                eye_prob,
            )
        elif self._eye_analyzer is not None:
            logger.debug(
                "Eye analyzer could not determine probability for %s",
                os.path.basename(image_path),
            )

        image.close()

        if not metrics:
            logger.error("All IQA metrics failed for %s", image_path)
            return None

        composite = self._combine_scores(metrics)
        return BestShotResult(
            image_path=image_path,
            composite_score=composite,
            metrics=metrics,
            raw_metrics=raw_metrics,
        )

    def _compute_eye_openness(self, image: Image.Image) -> Optional[float]:
        if self._eye_analyzer is None:
            return None

        def _predict(candidate: Image.Image) -> Optional[float]:
            try:
                value = self._eye_analyzer.predict_open_probability(candidate)
                if value is None:
                    return None
                return max(0.0, min(1.0, float(value)))
            except Exception as exc:  # pragma: no cover - defensive logging
                logger.warning(
                    "Eye-state analysis failed for %s: %s",
                    candidate.info.get("source_path", "<unknown>"),
                    exc,
                )
                return None

        source_path = image.info.get("source_path")
        disposable: List[Image.Image] = []
        candidates: List[Image.Image] = [image]

        def _append_candidate(candidate: Optional[Image.Image]) -> None:
            if candidate is None:
                return
            candidates.append(candidate)
            disposable.append(candidate)

        # Center crops of the working preview sometimes help MediaPipe to focus on faces
        for crop in self._build_eye_crops(image):
            _append_candidate(crop)

        if source_path:
            fallback = self._load_eye_image(source_path)
            _append_candidate(fallback)
            if fallback is not None:
                for crop in self._build_eye_crops(fallback):
                    _append_candidate(crop)

        try:
            for candidate in candidates:
                result = _predict(candidate)
                if result is not None:
                    return result
        finally:
            for extra in disposable:
                try:
                    extra.close()
                except Exception as exc:
                    logger.debug(
                        "Failed to close disposable eye candidate: %s", exc, exc_info=True
                    )

        return None

    def _load_eye_image(self, source_path: str) -> Optional[Image.Image]:
        try:
            with Image.open(source_path) as raw:
                prepared = ImageOps.exif_transpose(raw).convert("RGB")
                prepared.thumbnail(
                    (
                        _LOCAL_BEST_SHOT_SETTINGS.eye_fallback_max_edge,
                        _LOCAL_BEST_SHOT_SETTINGS.eye_fallback_max_edge,
                    ),
                    _RESAMPLE_LANCZOS,
                )
                buffered = prepared.copy()
                buffered.info.setdefault("source_path", source_path)
                return buffered
        except Exception:
            logger.debug(
                "Fallback eye preview load failed for %s", source_path, exc_info=True
            )
            return None

    def _build_eye_crops(self, image: Image.Image) -> List[Image.Image]:
        width, height = image.size
        if width < 80 or height < 80:
            return []
        scales = (0.85, 0.7)
        vertical_bias = (0.35, 0.25)
        crops: List[Image.Image] = []
        source_path = image.info.get("source_path")
        for scale, bias in zip(scales, vertical_bias):
            crop_w = max(64, int(width * scale))
            crop_h = max(64, int(height * scale))
            left = max(0, (width - crop_w) // 2)
            top = max(0, int((height - crop_h) * bias))
            right = min(width, left + crop_w)
            bottom = min(height, top + crop_h)
            if right - left < 64 or bottom - top < 64:
                continue
            crop = image.crop((left, top, right, bottom)).copy()
            max_edge = _LOCAL_BEST_SHOT_SETTINGS.eye_fallback_max_edge
            if max(crop.size) > max_edge:
                crop.thumbnail((max_edge, max_edge), _RESAMPLE_LANCZOS)
            crop.info.setdefault("source_path", source_path)
            crops.append(crop)
        return crops

    def _combine_scores(self, normalized_metrics: Dict[str, float]) -> float:
        numerator = 0.0
        denom = 0.0
        for name, value in normalized_metrics.items():
            weight = self._metric_weights.get(name, 1.0)
            numerator += value * weight
            denom += weight
        return numerator / denom if denom else 0.0


__all__ = [
    "BestPhotoSelector",
    "BestShotResult",
    "MetricSpec",
    "DEFAULT_MODELS_ROOT",
]

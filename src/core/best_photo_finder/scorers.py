from dataclasses import dataclass, field
from math import dist
from pathlib import Path
from typing import Protocol
from collections.abc import Callable, Iterable, Mapping, Sequence

from core.best_photo_finder.config import SelectorConfig
from core.best_photo_finder.devices import ResolvedDevice, resolve_device
from core.best_photo_finder.errors import (
    FaceLandmarkerError,
    MissingDependencyError,
    SelectionError,
)
from core.best_photo_finder.models import TechnicalMetrics
from core.app_settings import get_huggingface_cache_dir
from core.huggingface_progress import build_hf_tqdm_class
from core.runtime_paths import resolve_face_landmarker_model_path

LEFT_EYE_INDICES = (33, 160, 158, 133, 153, 144)
RIGHT_EYE_INDICES = (362, 385, 387, 263, 373, 380)


class TechnicalScorer(Protocol):
    def score(self, path: Path, config: SelectorConfig) -> TechnicalMetrics:
        """Compute blur and face-aware metrics for a single image."""

    def score_image(
        self, path: Path, image, config: SelectorConfig
    ) -> TechnicalMetrics:
        """Compute blur and face-aware metrics for a preloaded image."""

    def close(self) -> None:
        """Release native resources held by the scorer."""


class AestheticScorer(Protocol):
    model_name: str

    def score_batch(
        self, paths: Sequence[Path], config: SelectorConfig
    ) -> Mapping[Path, float]:
        """Compute aesthetic scores for a batch of images."""

    @property
    def device_used(self) -> str:
        """Return the resolved backend used for inference."""

    def score_batch_from_images(
        self, images_by_path: Mapping[Path, object], config: SelectorConfig
    ) -> Mapping[Path, float]:
        """Compute aesthetic scores for a batch of preloaded images."""


def _clamp(value: float, low: float, high: float) -> float:
    return max(low, min(high, value))


def _require_module(name: str):
    try:
        return __import__(name)
    except ImportError as exc:
        raise MissingDependencyError(
            f"Missing optional dependency '{name}'. Install the required extras before running the selector."
        ) from exc


class FaceLandmarkerBackend(Protocol):
    def detect_landmarks(self, rgb_image) -> Sequence[Sequence[object]]:
        """Return normalized landmarks for every face in an RGB image."""

    def close(self) -> None:
        """Release native MediaPipe resources."""


class MediaPipeTasksFaceLandmarker:
    """Small adapter around MediaPipe Tasks' model-backed Face Landmarker."""

    def __init__(self, model_path: Path) -> None:
        mediapipe = _require_module("mediapipe")
        try:
            tasks = mediapipe.tasks
            vision = tasks.vision
            options = vision.FaceLandmarkerOptions(
                base_options=tasks.BaseOptions(model_asset_path=str(model_path)),
                running_mode=vision.RunningMode.IMAGE,
                num_faces=10,
            )
            self._landmarker = vision.FaceLandmarker.create_from_options(options)
            self._image_type = mediapipe.Image
            self._image_format = mediapipe.ImageFormat.SRGB
        except (AttributeError, ValueError, RuntimeError) as exc:
            version = getattr(mediapipe, "__version__", "unknown")
            raise MissingDependencyError(
                "MediaPipe Tasks Face Landmarker could not be initialized "
                f"(detected version: {version}, model: {model_path})."
            ) from exc

    def detect_landmarks(self, rgb_image) -> Sequence[Sequence[object]]:
        try:
            import numpy as np
        except ImportError as exc:
            raise MissingDependencyError(
                "Missing optional dependency 'numpy'. Install the required extras before running the selector."
            ) from exc
        image = self._image_type(
            image_format=self._image_format,
            data=np.ascontiguousarray(rgb_image),
        )
        return self._landmarker.detect(image).face_landmarks

    def close(self) -> None:
        self._landmarker.close()


def _create_face_landmarker(model_path: Path) -> FaceLandmarkerBackend:
    return MediaPipeTasksFaceLandmarker(model_path)


def _normalized_blur_penalty(variance: float, config: SelectorConfig) -> float:
    if config.blur_threshold <= 0:
        return 0.0
    deficit = max(0.0, config.blur_threshold - variance)
    return _clamp(
        (deficit / config.blur_threshold) * config.blur_penalty_weight,
        0.0,
        config.blur_penalty_weight,
    )


def _landmark_xy(landmark) -> tuple[float, float]:
    return landmark.x, landmark.y


def _eye_aspect_ratio(landmarks, indices: tuple[int, int, int, int, int, int]) -> float:
    p1 = _landmark_xy(landmarks[indices[0]])
    p2 = _landmark_xy(landmarks[indices[1]])
    p3 = _landmark_xy(landmarks[indices[2]])
    p4 = _landmark_xy(landmarks[indices[3]])
    p5 = _landmark_xy(landmarks[indices[4]])
    p6 = _landmark_xy(landmarks[indices[5]])

    horizontal = max(dist(p1, p4), 1e-6)
    vertical = dist(p2, p6) + dist(p3, p5)
    return vertical / (2.0 * horizontal)


def _face_area_ratio(landmarks) -> float:
    xs = [point.x for point in landmarks]
    ys = [point.y for point in landmarks]
    return _clamp((max(xs) - min(xs)) * (max(ys) - min(ys)), 0.0, 1.0)


@dataclass(slots=True)
class OpenCvMediapipeTechnicalScorer:
    face_landmarker_factory: Callable[[Path], FaceLandmarkerBackend] = field(
        default=_create_face_landmarker, repr=False
    )
    _face_landmarker: FaceLandmarkerBackend | None = field(
        default=None, init=False, repr=False
    )

    def _get_face_landmarker(self) -> FaceLandmarkerBackend:
        if self._face_landmarker is not None:
            return self._face_landmarker
        try:
            self._face_landmarker = self.face_landmarker_factory(
                resolve_face_landmarker_model_path()
            )
        except (
            FileNotFoundError,
            MissingDependencyError,
            OSError,
            ValueError,
            RuntimeError,
        ) as exc:
            raise FaceLandmarkerError(
                f"Face Landmarker could not be initialized: {exc}"
            ) from exc
        return self._face_landmarker

    def close(self) -> None:
        landmarker = self._face_landmarker
        self._face_landmarker = None
        if landmarker is not None:
            try:
                landmarker.close()
            except RuntimeError:
                pass

    def score(self, path: Path, config: SelectorConfig) -> TechnicalMetrics:
        cv2 = _require_module("cv2")

        image = cv2.imread(str(path))
        if image is None:
            raise SelectionError(f"Could not read image: {path}")
        return self._score_loaded_image(path, image, config, cv2)

    def score_image(
        self, path: Path, image, config: SelectorConfig
    ) -> TechnicalMetrics:
        cv2 = _require_module("cv2")
        try:
            import numpy as np
        except ImportError as exc:
            raise MissingDependencyError(
                "Missing optional dependency 'numpy'. Install the required extras before running the selector."
            ) from exc

        if hasattr(image, "convert"):
            rgb_image = image.convert("RGB")
            loaded_image = cv2.cvtColor(np.asarray(rgb_image), cv2.COLOR_RGB2BGR)
        else:
            loaded_image = image
        if loaded_image is None:
            raise SelectionError(f"Could not read image: {path}")
        return self._score_loaded_image(path, loaded_image, config, cv2)

    def _score_loaded_image(
        self, path: Path, image, config: SelectorConfig, cv2
    ) -> TechnicalMetrics:
        if image is None:
            raise SelectionError(f"Could not read image: {path}")

        height, width = image.shape[:2]
        gray = cv2.cvtColor(image, cv2.COLOR_BGR2GRAY)
        blur_variance = float(cv2.Laplacian(gray, cv2.CV_64F).var())
        blur_penalty = _normalized_blur_penalty(blur_variance, config)

        rgb = cv2.cvtColor(image, cv2.COLOR_BGR2RGB)
        face_landmarker = self._get_face_landmarker()
        try:
            faces = face_landmarker.detect_landmarks(rgb)
        except (MissingDependencyError, OSError, ValueError, RuntimeError) as exc:
            self.close()
            raise FaceLandmarkerError(
                f"Face Landmarker failed while analysing {path.name}: {exc}"
            ) from exc

        face_count = len(faces)
        closed_face_count = 0
        max_face_area_ratio = 0.0
        issues: list[str] = []

        for landmarks in faces:
            left_ear = _eye_aspect_ratio(landmarks, LEFT_EYE_INDICES)
            right_ear = _eye_aspect_ratio(landmarks, RIGHT_EYE_INDICES)
            if min(left_ear, right_ear) < config.eye_closed_threshold:
                closed_face_count += 1
            max_face_area_ratio = max(max_face_area_ratio, _face_area_ratio(landmarks))

        eye_penalty = 0.0
        if face_count:
            eye_penalty = config.eye_penalty_weight * (closed_face_count / face_count)
            if closed_face_count:
                issues.append(
                    f"{closed_face_count}/{face_count} faces look like they have closed eyes"
                )

        return TechnicalMetrics(
            blur_variance=blur_variance,
            blur_penalty=blur_penalty,
            face_count=face_count,
            closed_face_count=closed_face_count,
            eye_penalty=eye_penalty,
            max_face_area_ratio=max_face_area_ratio,
            image_width=width,
            image_height=height,
            issues=tuple(issues),
        )


@dataclass(slots=True)
class HuggingFaceAestheticScorer:
    model_name: str = "cafeai/cafe_aesthetic"
    progress_callback: Callable[[int, str], None] | None = None
    _model: object | None = field(default=None, init=False, repr=False)
    _aesthetic_label_index: int | None = field(default=None, init=False, repr=False)
    _resolved_device: ResolvedDevice | None = field(
        default=None, init=False, repr=False
    )

    @property
    def device_used(self) -> str:
        if self._resolved_device is None:
            return "uninitialized"
        return self._resolved_device.backend

    def _load_thumbnail(self, path: Path, size: int):
        try:
            from PIL import Image, ImageOps
        except ImportError as exc:
            raise MissingDependencyError(
                "Missing optional dependency 'Pillow'. Install the aesthetic or vision extras before scoring."
            ) from exc

        image = Image.open(path)
        image = ImageOps.exif_transpose(image)
        image = image.convert("RGB")
        image.thumbnail((size, size), Image.Resampling.LANCZOS)
        return image

    def _build_model(self, config: SelectorConfig):
        self._resolved_device = resolve_device(config.device)
        try:
            import torch
            from huggingface_hub import snapshot_download
            from transformers import AutoModelForImageClassification
        except ImportError as exc:
            raise MissingDependencyError(
                "Missing optional dependency required for aesthetic scoring. "
                "Install the aesthetic extras before running the selector."
            ) from exc

        model_kwargs: dict[str, object] = {}
        if self._resolved_device.torch_dtype_name is not None:
            model_kwargs["dtype"] = getattr(
                torch, self._resolved_device.torch_dtype_name
            )

        model_path = snapshot_download(
            self.model_name,
            cache_dir=get_huggingface_cache_dir(),
            tqdm_class=build_hf_tqdm_class(
                self.progress_callback,
                label=f"Downloading {self.model_name}",
            ),
        )
        if self.progress_callback:
            self.progress_callback(-1, f"Loading {self.model_name}")
        model = AutoModelForImageClassification.from_pretrained(
            model_path, local_files_only=True, **model_kwargs
        )
        if self._resolved_device.backend == "cuda":
            model = model.to("cuda")
        elif self._resolved_device.backend == "mps":
            model = model.to("mps")
        else:
            model = model.to("cpu")

        model.eval()
        self._aesthetic_label_index = self._resolve_aesthetic_label_index(model)
        return model

    def _ensure_model(self, config: SelectorConfig):
        if self._model is None:
            self._model = self._build_model(config)
        return self._model

    def _resolve_aesthetic_label_index(self, model) -> int:
        id2label = getattr(model.config, "id2label", {}) or {}
        for raw_index, label in id2label.items():
            text = str(label).lower()
            if "not" not in text and "aesthetic" in text:
                return int(raw_index)

        if len(id2label) == 2:
            for raw_index, label in id2label.items():
                if "not" in str(label).lower():
                    other_index = [
                        int(index) for index in id2label if int(index) != int(raw_index)
                    ]
                    if other_index:
                        return other_index[0]

        return int(max(id2label, key=lambda key: int(key))) if id2label else 0

    def _model_input_dtype(self, model):
        try:
            import torch
        except ImportError as exc:
            raise MissingDependencyError(
                "Missing optional dependency required for aesthetic scoring. "
                "Install the aesthetic extras before running the selector."
            ) from exc

        parameter = next(model.parameters(), None)
        if parameter is not None:
            return parameter.dtype
        buffer = next(model.buffers(), None)
        if buffer is not None:
            return buffer.dtype
        return torch.float32

    def _preprocess_for_model(self, image):
        try:
            import numpy as np
            import torch
            from PIL import Image
        except ImportError as exc:
            raise MissingDependencyError(
                "Missing optional dependency required for aesthetic scoring. "
                "Install the aesthetic extras before running the selector."
            ) from exc

        image_size = config_size = 384
        if self._model is not None:
            image_size = int(
                getattr(self._model.config, "image_size", config_size) or config_size
            )

        if image.size != (image_size, image_size):
            image = image.resize((image_size, image_size), Image.Resampling.BICUBIC)

        pixels = np.asarray(image, dtype="float32") / 255.0
        mean = np.array((0.5, 0.5, 0.5), dtype="float32")
        std = np.array((0.5, 0.5, 0.5), dtype="float32")
        pixels = (pixels - mean) / std
        tensor = torch.from_numpy(pixels).permute(2, 0, 1)
        return tensor

    def _extract_aesthetic_score(
        self, predictions: Iterable[dict[str, float]]
    ) -> float:
        labels = list(predictions)
        if not labels:
            raise SelectionError("Aesthetic model returned no predictions.")

        for item in labels:
            label = str(item.get("label", "")).lower()
            if "not" not in label and "aesthetic" in label:
                return float(item["score"])

        if len(labels) == 2:
            not_aesthetic = next(
                (
                    item
                    for item in labels
                    if "not" in str(item.get("label", "")).lower()
                ),
                None,
            )
            if not_aesthetic is not None:
                return 1.0 - float(not_aesthetic["score"])

        best = max(labels, key=lambda item: float(item["score"]))
        return float(best["score"])

    def score_batch_from_images(
        self, images_by_path: Mapping[Path, object], config: SelectorConfig
    ) -> Mapping[Path, float]:
        try:
            import torch
        except ImportError as exc:
            raise MissingDependencyError(
                "Missing optional dependency 'torch'. Install the aesthetic extras before scoring."
            ) from exc

        model = self._ensure_model(config)
        device = self.device_used
        scores: dict[Path, float] = {}
        aesthetic_label_index = self._aesthetic_label_index or 0
        batch_size = max(1, config.aesthetic_batch_size)
        items = list(images_by_path.items())

        for start in range(0, len(items), batch_size):
            batch_items = items[start : start + batch_size]
            input_dtype = self._model_input_dtype(model)
            pixel_values = torch.stack(
                [self._preprocess_for_model(image) for _, image in batch_items]
            )
            if device == "cuda":
                pixel_values = pixel_values.to(device="cuda", dtype=input_dtype)
            elif device == "mps":
                pixel_values = pixel_values.to(device="mps", dtype=input_dtype)
            else:
                pixel_values = pixel_values.to(dtype=input_dtype)

            with torch.no_grad():
                logits = model(pixel_values=pixel_values).logits
                probabilities = torch.softmax(logits, dim=-1)[:, aesthetic_label_index]

            for (path, _image), probability in zip(
                batch_items, probabilities, strict=True
            ):
                scores[path] = _clamp(
                    float(probability.detach().cpu().item()), 0.0, 1.0
                )
        return scores

    def score_batch(
        self, paths: Sequence[Path], config: SelectorConfig
    ) -> Mapping[Path, float]:
        images_by_path = {
            path: self._load_thumbnail(path, config.thumbnail_size) for path in paths
        }
        return self.score_batch_from_images(images_by_path, config)

from __future__ import annotations

from dataclasses import dataclass, field
import importlib
from math import dist
from pathlib import Path
from typing import Iterable, Mapping, Protocol, Sequence

from core.best_photo_finder.config import SelectorConfig
from core.best_photo_finder.devices import ResolvedDevice, resolve_device
from core.best_photo_finder.errors import MissingDependencyError, SelectionError
from core.best_photo_finder.models import TechnicalMetrics

LEFT_EYE_INDICES = (33, 160, 158, 133, 153, 144)
RIGHT_EYE_INDICES = (362, 385, 387, 263, 373, 380)


class TechnicalScorer(Protocol):
    def score(self, path: Path, config: SelectorConfig) -> TechnicalMetrics:
        """Compute blur and face-aware metrics for a single image."""

    def score_image(self, path: Path, image, config: SelectorConfig) -> TechnicalMetrics:
        """Compute blur and face-aware metrics for a preloaded image."""


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


def _load_face_mesh_factory():
    try:
        mediapipe = _require_module("mediapipe")
    except MissingDependencyError:
        raise

    solutions = getattr(mediapipe, "solutions", None)
    if solutions is not None:
        face_mesh_module = getattr(solutions, "face_mesh", None)
        face_mesh_factory = getattr(face_mesh_module, "FaceMesh", None)
        if face_mesh_factory is not None:
            return face_mesh_factory

    for module_name in (
        "mediapipe.python.solutions.face_mesh",
        "mediapipe.solutions.face_mesh",
    ):
        try:
            module = importlib.import_module(module_name)
        except ImportError:
            continue
        face_mesh_factory = getattr(module, "FaceMesh", None)
        if face_mesh_factory is not None:
            return face_mesh_factory

    version = getattr(mediapipe, "__version__", "unknown")
    raise MissingDependencyError(
        "Installed MediaPipe does not expose the legacy FaceMesh API required by this project "
        f"(detected version: {version}). Reinstall a compatible release, for example "
        "`pip install 'mediapipe<0.10.30'`."
    )


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


@dataclass(frozen=True, slots=True)
class FaceBox:
    x: int
    y: int
    width: int
    height: int

    @property
    def area(self) -> int:
        return self.width * self.height


@dataclass(slots=True)
class OpenCvMediapipeTechnicalScorer:
    _face_mesh: object | None = field(default=None, init=False, repr=False)
    _face_mesh_error: str | None = field(default=None, init=False, repr=False)
    _face_mesh_disabled: bool = field(default=False, init=False, repr=False)
    _face_cascade: object | None = field(default=None, init=False, repr=False)
    _eye_cascade: object | None = field(default=None, init=False, repr=False)

    def _get_cascade(self, cv2, filename: str):
        cascade_path = Path(cv2.data.haarcascades) / filename
        cascade = cv2.CascadeClassifier(str(cascade_path))
        if cascade.empty():
            raise SelectionError(f"Could not load OpenCV cascade: {cascade_path}")
        return cascade

    def _get_face_cascade(self, cv2):
        if self._face_cascade is None:
            self._face_cascade = self._get_cascade(
                cv2, "haarcascade_frontalface_default.xml"
            )
        return self._face_cascade

    def _get_eye_cascade(self, cv2):
        if self._eye_cascade is None:
            self._eye_cascade = self._get_cascade(
                cv2, "haarcascade_eye_tree_eyeglasses.xml"
            )
        return self._eye_cascade

    def _detect_faces(self, gray_image, cv2) -> list[FaceBox]:
        min_side = max(48, min(gray_image.shape[:2]) // 12)
        detections = self._get_face_cascade(cv2).detectMultiScale(
            gray_image,
            scaleFactor=1.1,
            minNeighbors=5,
            minSize=(min_side, min_side),
        )
        faces = [
            FaceBox(int(x), int(y), int(width), int(height))
            for x, y, width, height in detections
        ]
        return sorted(faces, key=lambda face: face.area, reverse=True)

    def _expand_face_box(
        self,
        face: FaceBox,
        image_width: int,
        image_height: int,
        padding_ratio: float = 0.18,
    ) -> FaceBox:
        pad_x = int(face.width * padding_ratio)
        pad_y = int(face.height * padding_ratio)
        x0 = max(0, face.x - pad_x)
        y0 = max(0, face.y - pad_y)
        x1 = min(image_width, face.x + face.width + pad_x)
        y1 = min(image_height, face.y + face.height + pad_y)
        return FaceBox(x=x0, y=y0, width=max(1, x1 - x0), height=max(1, y1 - y0))

    def _detect_open_eyes_in_face(self, face_gray, cv2) -> int:
        eyes = self._get_eye_cascade(cv2).detectMultiScale(
            face_gray,
            scaleFactor=1.05,
            minNeighbors=3,
            minSize=(12, 12),
        )
        return len(eyes)

    def _face_area_ratio_from_box(
        self, face: FaceBox, image_width: int, image_height: int
    ) -> float:
        return _clamp(face.area / max(1, image_width * image_height), 0.0, 1.0)

    def _score_face_crop(
        self,
        face_box: FaceBox,
        rgb_image,
        gray_image,
        image_width: int,
        image_height: int,
        config: SelectorConfig,
        cv2,
    ):
        expanded = self._expand_face_box(face_box, image_width, image_height)
        x0, y0 = expanded.x, expanded.y
        x1, y1 = x0 + expanded.width, y0 + expanded.height
        face_rgb = rgb_image[y0:y1, x0:x1]
        face_gray = gray_image[y0:y1, x0:x1]

        max_face_area_ratio = self._face_area_ratio_from_box(
            face_box, image_width, image_height
        )
        issues: list[str] = []
        closed = False

        face_mesh = self._get_face_mesh()
        if face_mesh is not None and face_rgb.size:
            results = face_mesh.process(face_rgb)
            face_landmarks_list = results.multi_face_landmarks or []
            if face_landmarks_list:
                best = max(
                    face_landmarks_list,
                    key=lambda item: _face_area_ratio(item.landmark),
                )
                landmarks = best.landmark
                left_ear = _eye_aspect_ratio(landmarks, LEFT_EYE_INDICES)
                right_ear = _eye_aspect_ratio(landmarks, RIGHT_EYE_INDICES)
                closed = min(left_ear, right_ear) < config.eye_closed_threshold
                landmark_ratio = _face_area_ratio(
                    landmarks
                ) * self._face_area_ratio_from_box(expanded, image_width, image_height)
                max_face_area_ratio = max(max_face_area_ratio, landmark_ratio)
                return closed, max_face_area_ratio, tuple(issues)

        if self._face_mesh_error:
            issues.append(f"landmark fallback used: {self._face_mesh_error}")

        upper_face = face_gray[: max(1, face_gray.shape[0] // 2), :]
        eye_count = self._detect_open_eyes_in_face(upper_face, cv2)
        closed = eye_count < 2
        return closed, max_face_area_ratio, tuple(issues)

    def _get_face_mesh(self):
        if self._face_mesh_disabled:
            return None
        if self._face_mesh is not None:
            return self._face_mesh

        face_mesh_factory = _load_face_mesh_factory()
        try:
            self._face_mesh = face_mesh_factory(
                static_image_mode=True,
                max_num_faces=10,
                refine_landmarks=True,
            )
        except RuntimeError as exc:
            self._face_mesh_disabled = True
            self._face_mesh_error = str(exc)
            return None
        return self._face_mesh

    def score(self, path: Path, config: SelectorConfig) -> TechnicalMetrics:
        cv2 = _require_module("cv2")

        image = cv2.imread(str(path))
        if image is None:
            raise SelectionError(f"Could not read image: {path}")
        return self._score_loaded_image(path, image, config, cv2)

    def score_image(self, path: Path, image, config: SelectorConfig) -> TechnicalMetrics:
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

    def _score_loaded_image(self, path: Path, image, config: SelectorConfig, cv2) -> TechnicalMetrics:
        if image is None:
            raise SelectionError(f"Could not read image: {path}")

        height, width = image.shape[:2]
        gray = cv2.cvtColor(image, cv2.COLOR_BGR2GRAY)
        blur_variance = float(cv2.Laplacian(gray, cv2.CV_64F).var())
        blur_penalty = _normalized_blur_penalty(blur_variance, config)

        rgb = cv2.cvtColor(image, cv2.COLOR_BGR2RGB)
        faces = self._detect_faces(gray, cv2)
        face_count = 0
        closed_face_count = 0
        max_face_area_ratio = 0.0
        issues: list[str] = []

        for face in faces:
            face_count += 1
            closed, face_area_ratio, face_issues = self._score_face_crop(
                face, rgb, gray, width, height, config, cv2
            )
            if closed:
                closed_face_count += 1
            max_face_area_ratio = max(max_face_area_ratio, face_area_ratio)
            issues.extend(face_issues)

        if not faces and self._face_mesh_error:
            issues.append(f"face analysis unavailable: {self._face_mesh_error}")

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

        model_path = snapshot_download(self.model_name)
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

            for (path, _image), probability in zip(batch_items, probabilities, strict=True):
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

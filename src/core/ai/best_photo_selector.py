"""
Experimental multi-model pipeline that ranks similar shots by overall quality.

Pipeline overview
-----------------
1. **Face detection** (qualcomm/MediaPipe-Face-Detection ONNX) is used to locate
   the primary subject plus the six BlazeFace keypoints.
2. **Eye-state classification** (MichalMlodawski/open-closed-eye-classification-mobilev2)
   determines whether the subject's eyes are open.
3. **Technical + aesthetic scoring** relies on the CLIP-based
   `shunk031/aesthetics-predictor` head. The predictor produces an aesthetic
   score and normalized CLIP embeddings for every image/crop, which are then
   used for framing analysis (cosine similarity between full image and face
   crops) plus the downstream composite ranking.

Every metric is normalized to `[0, 1]` and combined via a simple weighting
scheme, prioritising sharp, open-eye photos over purely aesthetic scores. The
implementation is intentionally modular so that the UI or future automation
can inject mocked detectors for tests or swap in custom weighting profiles.

The bundled BlazeFace anchor tensor originates from MediaPipePyTorch
(Apache License 2.0). The aesthetic head is based on the open-source model by
shunk031 (Apache 2.0).
"""

from __future__ import annotations

import importlib.util
import json
import logging
import os
import sys
import types
from dataclasses import dataclass, field
from typing import Callable, Dict, Iterable, List, Optional, Sequence, Tuple

import cv2  # type: ignore
import numpy as np
from PIL import Image, ImageOps

logger = logging.getLogger(__name__)


PROJECT_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..", ".."))
DEFAULT_MODELS_ROOT = os.environ.get(
    "PHOTOSORT_MODELS_DIR", os.path.join(PROJECT_ROOT, "models")
)

SUPPORTED_IMAGE_EXTENSIONS = {
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

DEFAULT_COMPOSITE_WEIGHTS = {
    "eyes_open": 0.35,
    "technical": 0.25,
    "aesthetic": 0.25,
    "framing": 0.15,
}

# Anchor tensor copied from MediaPipePyTorch (Apache 2.0).
ANCHOR_RESOURCE_PATH = os.path.join(
    os.path.dirname(__file__), "data", "blazeface_anchors.npy"
)


def _first_existing_path(candidates: Iterable[str]) -> Optional[str]:
    for candidate in candidates:
        if candidate and os.path.exists(candidate):
            return candidate
    return None


def _clamp(value: float, min_value: float = 0.0, max_value: float = 1.0) -> float:
    return float(max(min_value, min(max_value, value)))


def _cosine_similarity(vec_a: np.ndarray, vec_b: np.ndarray) -> float:
    denom = float(np.linalg.norm(vec_a) * np.linalg.norm(vec_b))
    if denom == 0.0:
        return 0.0
    return float(np.dot(vec_a, vec_b) / denom)


def _default_focus_score(image: Image.Image) -> float:
    """Normalized Laplacian variance focus metric."""
    gray = image.convert("L")
    arr = np.array(gray, dtype=np.uint8)
    if arr.size == 0:
        return 0.0
    try:
        variance = cv2.Laplacian(arr, cv2.CV_64F).var()
    except cv2.error as exc:
        logger.warning("Laplacian focus metric failed: %s", exc)
        return 0.0
    return float(variance / (variance + 300.0))


@dataclass
class FaceDetectionResult:
    score: float
    bbox: Tuple[int, int, int, int]  # (left, top, right, bottom) in pixels
    bbox_normalized: Tuple[float, float, float, float]  # (ymin, xmin, ymax, xmax)
    keypoints: List[Tuple[float, float]]  # normalized x/y pairs
    image_size: Tuple[int, int]

    def crop_face(self, image: Image.Image) -> Image.Image:
        return image.crop(self.bbox).copy()

    def to_dict(self) -> Dict[str, object]:
        return {
            "score": self.score,
            "bbox": self.bbox,
            "bbox_normalized": self.bbox_normalized,
            "image_size": self.image_size,
            "keypoints": self.keypoints,
        }


@dataclass
class QualityScore:
    raw: float
    normalized: float
    embedding: Optional[np.ndarray] = None


@dataclass
class BestShotResult:
    image_path: str
    composite_score: float
    metrics: Dict[str, float] = field(default_factory=dict)
    raw_metrics: Dict[str, float] = field(default_factory=dict)
    face: Optional[FaceDetectionResult] = None

    def to_dict(self) -> Dict[str, object]:
        payload: Dict[str, object] = {
            "image_path": self.image_path,
            "composite_score": self.composite_score,
            "metrics": self.metrics,
            "raw_metrics": self.raw_metrics,
        }
        if self.face:
            payload["face"] = self.face.to_dict()
        return payload


class BlazeFaceDetector:
    """Thin wrapper around the Qualcomm MediaPipe face detector (ONNX)."""

    def __init__(
        self,
        models_root: Optional[str] = None,
        model_path: Optional[str] = None,
        min_score: float = 0.6,
        iou_threshold: float = 0.3,
        max_faces: int = 5,
    ):
        self.models_root = models_root or DEFAULT_MODELS_ROOT
        self.model_path = model_path or _first_existing_path(
            [
                os.path.join(self.models_root, "job_jgzjewkop_optimized_onnx", "model.onnx"),
                os.path.join(
                    self.models_root,
                    "MediaPipe-Face-Detection_FaceDetector_float",
                    "model.onnx",
                ),
            ]
        )
        self.min_score = min_score
        self.iou_threshold = iou_threshold
        self.max_faces = max_faces

        self._session = None
        self._input_name: Optional[str] = None
        self._output_names: Optional[List[str]] = None
        self._anchors: Optional[np.ndarray] = None

    def _ensure_ready(self):
        if self._session is not None:
            return
        if not self.model_path:
            raise FileNotFoundError(
                "Face detector ONNX model not found. Expected it under the 'models/' "
                "folder (e.g. job_*_onnx/model.onnx from qualcomm/MediaPipe-Face-Detection)."
            )
        try:
            import onnxruntime as ort  # type: ignore
        except ImportError as exc:  # pragma: no cover - environment specific
            raise RuntimeError("onnxruntime is required for face detection") from exc

        providers = ["CPUExecutionProvider"]
        self._session = ort.InferenceSession(self.model_path, providers=providers)
        inputs = self._session.get_inputs()
        outputs = self._session.get_outputs()
        self._input_name = inputs[0].name
        self._output_names = [out.name for out in outputs]

        anchors_path = (
            os.path.join(self.models_root, "blazeface_anchors.npy")
            if os.path.exists(os.path.join(self.models_root, "blazeface_anchors.npy"))
            else ANCHOR_RESOURCE_PATH
        )
        if not os.path.exists(anchors_path):
            raise FileNotFoundError(
                "BlazeFace anchors file missing. Expected either "
                f"{anchors_path} or models/blazeface_anchors.npy."
            )
        self._anchors = np.load(anchors_path).astype(np.float32)

    def detect_faces(
        self,
        image: Image.Image,
        image_path: Optional[str] = None,
        max_faces: Optional[int] = None,
    ) -> List[FaceDetectionResult]:
        self._ensure_ready()
        assert self._session is not None
        assert self._input_name is not None
        assert self._output_names is not None
        assert self._anchors is not None

        width, height = image.size
        np_img = np.asarray(image.convert("RGB"), dtype=np.float32) / 255.0
        resized = cv2.resize(np_img, (256, 256), interpolation=cv2.INTER_AREA)
        tensor = np.transpose(resized, (2, 0, 1))[None, ...]

        outputs = self._session.run(self._output_names, {self._input_name: tensor})
        box_coords = np.concatenate(outputs[:2], axis=1)[0]
        box_scores = np.concatenate(outputs[2:], axis=1)[0, :, 0]
        box_scores = 1.0 / (1.0 + np.exp(-box_scores))

        decoded = self._decode_boxes(box_coords, self._anchors)
        mask = box_scores >= self.min_score
        decoded = decoded[mask]
        scores = box_scores[mask]

        if decoded.size == 0:
            return []

        keep_indices = self._weighted_nms(decoded[:, :4], scores, max_faces)
        results: List[FaceDetectionResult] = []
        for idx in keep_indices:
            box = decoded[idx, :4]
            keypoints = decoded[idx, 4:].reshape(-1, 2).tolist()
            ymin, xmin, ymax, xmax = [float(_clamp(v)) for v in box]
            left = int(round(xmin * width))
            top = int(round(ymin * height))
            right = int(round(xmax * width))
            bottom = int(round(ymax * height))
            if right <= left or bottom <= top:
                continue
            results.append(
                FaceDetectionResult(
                    score=float(scores[idx]),
                    bbox=(left, top, right, bottom),
                    bbox_normalized=(ymin, xmin, ymax, xmax),
                    keypoints=keypoints,
                    image_size=(width, height),
                )
            )
        return results

    @staticmethod
    def _decode_boxes(raw_boxes: np.ndarray, anchors: np.ndarray) -> np.ndarray:
        x_scale = 128.0
        y_scale = 128.0
        h_scale = 128.0
        w_scale = 128.0

        boxes = np.zeros_like(raw_boxes)
        x_center = raw_boxes[:, 0] / x_scale * anchors[:, 2] + anchors[:, 0]
        y_center = raw_boxes[:, 1] / y_scale * anchors[:, 3] + anchors[:, 1]
        w = raw_boxes[:, 2] / w_scale * anchors[:, 2]
        h = raw_boxes[:, 3] / h_scale * anchors[:, 3]

        boxes[:, 0] = y_center - h / 2.0
        boxes[:, 1] = x_center - w / 2.0
        boxes[:, 2] = y_center + h / 2.0
        boxes[:, 3] = x_center + w / 2.0

        for k in range(6):
            offset = 4 + k * 2
            boxes[:, offset] = (
                raw_boxes[:, offset] / x_scale * anchors[:, 2] + anchors[:, 0]
            )
            boxes[:, offset + 1] = (
                raw_boxes[:, offset + 1] / y_scale * anchors[:, 3] + anchors[:, 1]
            )

        return boxes

    def _weighted_nms(
        self, boxes: np.ndarray, scores: np.ndarray, max_faces: Optional[int]
    ) -> List[int]:
        order = scores.argsort()[::-1]
        keep: List[int] = []
        max_candidates = max_faces or self.max_faces

        while order.size > 0 and len(keep) < max_candidates:
            idx = order[0]
            keep.append(int(idx))
            if order.size == 1:
                break
            ious = self._iou(boxes[idx], boxes[order[1:]])
            order = order[1:][ious < self.iou_threshold]
        return keep

    @staticmethod
    def _iou(box: np.ndarray, others: np.ndarray) -> np.ndarray:
        ymin = np.maximum(box[0], others[:, 0])
        xmin = np.maximum(box[1], others[:, 1])
        ymax = np.minimum(box[2], others[:, 2])
        xmax = np.minimum(box[3], others[:, 3])

        inter = np.maximum(0.0, ymax - ymin) * np.maximum(0.0, xmax - xmin)
        box_area = (box[2] - box[0]) * (box[3] - box[1])
        other_area = (others[:, 2] - others[:, 0]) * (others[:, 3] - others[:, 1])
        union = box_area + other_area - inter + 1e-6
        return inter / union


class EyeStateClassifier:
    """Wrapper around the MobilenetV2 eye open/closed classifier."""

    def __init__(self, model_dir: Optional[str] = None):
        self.model_dir = model_dir or os.path.join(
            DEFAULT_MODELS_ROOT, "open-closed-eye-classification-mobilev2"
        )
        if not os.path.isdir(self.model_dir):
            raise FileNotFoundError(
                "Eye classifier checkpoint not found. "
                "Download MichalMlodawski/open-closed-eye-classification-mobilev2 "
                "into the 'models/open-closed-eye-classification-mobilev2' folder."
            )
        self._device = None
        self._processor = None
        self._model = None
        self._ensure_ready()

    def _ensure_ready(self):
        if self._model is not None:
            return
        try:
            import torch  # type: ignore
            from transformers import (  # type: ignore
                AutoImageProcessor,
                MobileNetV2ForImageClassification,
            )
        except ImportError as exc:  # pragma: no cover
            raise RuntimeError(
                "transformers and torch are required for the eye-state classifier"
            ) from exc

        self._device = torch.device("cuda" if torch.cuda.is_available() else "cpu")
        self._processor = AutoImageProcessor.from_pretrained(
            self.model_dir, local_files_only=True
        )
        self._model = MobileNetV2ForImageClassification.from_pretrained(
            self.model_dir, local_files_only=True
        )
        self._model.to(self._device)
        self._model.eval()

    def predict_open_probability(
        self, eye_image: Image.Image, image_path: Optional[str] = None
    ) -> float:
        import torch  # type: ignore

        assert self._processor is not None and self._model is not None and self._device is not None
        inputs = self._processor(images=eye_image, return_tensors="pt")
        inputs = {k: v.to(self._device) for k, v in inputs.items()}
        with torch.no_grad():
            logits = self._model(**inputs).logits
            probs = torch.softmax(logits, dim=-1)
        # Class index 1 == eyes open
        return float(probs[0, 1].item())


class QualityFusionModel:
    """Wraps the local AestheticsPredictor V2 model for scoring + embeddings."""

    def __init__(
        self,
        models_root: Optional[str] = None,
        predictor_dir: Optional[str] = None,
    ):
        models_root = models_root or DEFAULT_MODELS_ROOT
        self.predictor_dir = predictor_dir or os.path.join(
            models_root, "aesthetic_predictor"
        )
        if not os.path.isdir(self.predictor_dir):
            raise FileNotFoundError(
                "Aesthetic predictor not found. "
                "Download shunk031/aesthetics-predictor-v2 (linear) into "
                "models/aesthetic_predictor."
            )
        self._package_name = f"photosort_aesthetic_predictor_{abs(hash(self.predictor_dir))}"
        if self._package_name not in sys.modules:
            package = types.ModuleType(self._package_name)
            package.__path__ = [self.predictor_dir]
            sys.modules[self._package_name] = package
        self._device = None
        self._processor = None
        self._model = None
        self._load_predictor()

    def _load_local_module(self, module_name: str, file_path: str):
        spec = importlib.util.spec_from_file_location(module_name, file_path)
        if spec is None or spec.loader is None:
            raise ImportError(f"Could not load module from {file_path}")
        module = importlib.util.module_from_spec(spec)
        sys.modules[module_name] = module
        spec.loader.exec_module(module)
        return module

    def _load_predictor(self):
        import torch  # type: ignore
        from safetensors.torch import load_file  # type: ignore
        from transformers import CLIPImageProcessor  # type: ignore

        package_prefix = self._package_name
        config_module = self._load_local_module(
            f"{package_prefix}.configuration_predictor",
            os.path.join(self.predictor_dir, "configuration_predictor.py"),
        )
        model_module = self._load_local_module(
            f"{package_prefix}.modeling_v2",
            os.path.join(self.predictor_dir, "modeling_v2.py"),
        )
        AestheticsPredictorConfig = getattr(
            config_module, "AestheticsPredictorConfig"
        )
        PredictorModel = getattr(model_module, "AestheticsPredictorV2Linear")

        config = AestheticsPredictorConfig.from_pretrained(self.predictor_dir)
        model = PredictorModel(config)
        state_dict = load_file(os.path.join(self.predictor_dir, "model.safetensors"))
        model.load_state_dict(state_dict, strict=False)

        self._device = torch.device("cuda" if torch.cuda.is_available() else "cpu")
        self._model = model.to(self._device)
        self._model.eval()
        self._processor = CLIPImageProcessor.from_pretrained(
            self.predictor_dir, local_files_only=True
        )

    def score(self, image: Image.Image, return_embedding: bool = False) -> QualityScore:
        import torch  # type: ignore

        assert self._model is not None
        assert self._processor is not None
        assert self._device is not None

        inputs = self._processor(images=image, return_tensors="pt")
        pixel_values = inputs["pixel_values"].to(self._device)
        with torch.no_grad():
            outputs = self._model(
                pixel_values=pixel_values,
                return_dict=True,
            )
            logits = outputs.logits
            embedding_tensor = outputs.hidden_states

        raw = float(logits.squeeze().item())
        normalized = _clamp((raw - 1.0) / 9.0)
        embedding_np = (
            embedding_tensor.squeeze().detach().cpu().numpy()
            if return_embedding
            else None
        )
        return QualityScore(raw=raw, normalized=normalized, embedding=embedding_np)


class BestPhotoSelector:
    """High-level orchestrator that ranks images by composite quality."""

    def __init__(
        self,
        face_detector: Optional[BlazeFaceDetector] = None,
        eye_classifier: Optional[EyeStateClassifier] = None,
        quality_model: Optional[QualityFusionModel] = None,
        models_root: Optional[str] = None,
        weights: Optional[Dict[str, float]] = None,
        image_loader: Optional[Callable[[str], Image.Image]] = None,
        focus_metric_fn: Optional[Callable[[Image.Image], float]] = None,
    ):
        self.models_root = models_root or DEFAULT_MODELS_ROOT
        self.face_detector = face_detector
        self.eye_classifier = eye_classifier
        self.quality_model = quality_model
        self.weights = weights or DEFAULT_COMPOSITE_WEIGHTS
        self._image_loader = image_loader or self._default_loader
        self._focus_metric = focus_metric_fn or _default_focus_score

        if self.face_detector is None:
            try:
                self.face_detector = BlazeFaceDetector(models_root=self.models_root)
            except FileNotFoundError as exc:
                logger.warning("Face detector disabled: %s", exc)
        if self.eye_classifier is None:
            try:
                self.eye_classifier = EyeStateClassifier(
                    os.path.join(self.models_root, "open-closed-eye-classification-mobilev2")
                )
            except FileNotFoundError as exc:
                logger.warning("Eye-state classifier disabled: %s", exc)
        if self.quality_model is None:
            try:
                self.quality_model = QualityFusionModel(models_root=self.models_root)
            except FileNotFoundError as exc:
                logger.error("Quality model unavailable: %s", exc)
                raise

    def rank_directory(self, directory: str, recursive: bool = False) -> List[BestShotResult]:
        image_paths: List[str] = []
        if recursive:
            for root, _, files in os.walk(directory):
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
            result = self._analyze_image(path)
            if result:
                results.append(result)
        return sorted(results, key=lambda r: r.composite_score, reverse=True)

    def _is_supported_file(self, filename: str) -> bool:
        _, ext = os.path.splitext(filename)
        return ext.lower() in SUPPORTED_IMAGE_EXTENSIONS

    def _default_loader(self, image_path: str) -> Image.Image:
        with Image.open(image_path) as img:
            prepared = ImageOps.exif_transpose(img).convert("RGB")
            prepared.info["source_path"] = image_path
            prepared.info["region"] = "full"
            return prepared.copy()

    def _analyze_image(self, image_path: str) -> Optional[BestShotResult]:
        try:
            image = self._image_loader(image_path)
            image.info.setdefault("source_path", image_path)
            image.info.setdefault("region", "full")
        except Exception as exc:
            logger.error("Failed to load %s: %s", image_path, exc)
            return None

        assert self.quality_model is not None
        try:
            full_quality = self.quality_model.score(image, return_embedding=True)
        except Exception as exc:
            logger.error("Quality scoring failed for %s: %s", image_path, exc)
            image.close()
            return None

        metrics: Dict[str, float] = {"aesthetic": full_quality.normalized}
        raw_metrics: Dict[str, float] = {"quality_full_raw": full_quality.raw}
        focus_full = self._focus_metric(image)
        raw_metrics["focus_full"] = focus_full

        face_result: Optional[FaceDetectionResult] = None
        technical_score = focus_full
        framing_score: Optional[float] = None
        if self.face_detector:
            try:
                detections = self.face_detector.detect_faces(image, image_path=image_path)
            except Exception as exc:
                logger.warning("Face detection failed for %s: %s", image_path, exc)
                detections = []
            if detections:
                face_result = detections[0]
                face_crop = face_result.crop_face(image)
                face_crop.info["source_path"] = image_path
                face_crop.info["region"] = "face"

                focus_face = self._focus_metric(face_crop)
                raw_metrics["focus_face"] = focus_face

                try:
                    face_quality = self.quality_model.score(face_crop, return_embedding=True)
                    raw_metrics["quality_face_raw"] = face_quality.raw
                    technical_score = 0.6 * focus_face + 0.4 * face_quality.normalized
                    if (
                        full_quality.embedding is not None
                        and face_quality.embedding is not None
                    ):
                        framing_score = _clamp(
                            (_cosine_similarity(full_quality.embedding, face_quality.embedding) + 1.0)
                            / 2.0
                        )
                except Exception as exc:
                    logger.warning("Subject quality scoring failed for %s: %s", image_path, exc)
                finally:
                    face_crop.close()

                if self.eye_classifier:
                    eye_crop = self._extract_eye_region(image, face_result)
                    if eye_crop is not None:
                        eye_crop.info["source_path"] = image_path
                        eye_crop.info["region"] = "eyes"
                        try:
                            eyes_open_prob = self.eye_classifier.predict_open_probability(
                                eye_crop, image_path=image_path
                            )
                            metrics["eyes_open"] = eyes_open_prob
                            raw_metrics["eyes_open_probability"] = eyes_open_prob
                        except Exception as exc:
                            logger.warning("Eye-state classification failed for %s: %s", image_path, exc)
                        finally:
                            eye_crop.close()

        metrics["technical"] = _clamp(technical_score)
        if framing_score is not None:
            metrics["framing"] = framing_score

        composite = self._combine_scores(metrics)
        result = BestShotResult(
            image_path=image_path,
            composite_score=composite,
            metrics=metrics,
            raw_metrics=raw_metrics,
            face=face_result,
        )
        image.close()
        return result

    def _combine_scores(self, metrics: Dict[str, float]) -> float:
        numerator = 0.0
        denom = 0.0
        for key, weight in self.weights.items():
            if key in metrics:
                numerator += metrics[key] * weight
                denom += weight
        return numerator / denom if denom else 0.0

    def _extract_eye_region(
        self, image: Image.Image, detection: FaceDetectionResult, padding_ratio: float = 0.35
    ) -> Optional[Image.Image]:
        if len(detection.keypoints) < 2:
            return None
        width, height = detection.image_size
        right_eye = detection.keypoints[0]
        left_eye = detection.keypoints[1]

        xs = [right_eye[0], left_eye[0]]
        ys = [right_eye[1], left_eye[1]]
        x_min = min(xs)
        x_max = max(xs)
        if x_max <= x_min:
            return None
        eye_width = x_max - x_min
        pad_x = eye_width * padding_ratio
        pad_y = eye_width * (padding_ratio + 0.1)
        center_y = sum(ys) / len(ys)

        x0 = _clamp(x_min - pad_x)
        x1 = _clamp(x_max + pad_x)
        y0 = _clamp(center_y - pad_y)
        y1 = _clamp(center_y + pad_y)

        left = int(round(x0 * width))
        right = int(round(x1 * width))
        top = int(round(y0 * height))
        bottom = int(round(y1 * height))

        if right <= left or bottom <= top:
            return None
        return image.crop((left, top, right, bottom)).copy()


__all__ = [
    "BestPhotoSelector",
    "BestShotResult",
    "FaceDetectionResult",
    "QualityScore",
]

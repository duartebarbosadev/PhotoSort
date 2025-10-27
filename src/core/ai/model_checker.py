"""
Model availability checker for best-shot analysis models.

Verifies that all required external models (face detector, eye classifier,
aesthetic predictor) are present before attempting to instantiate the
BestPhotoSelector. Raises ModelDependencyError with actionable messages
when any model is missing.
"""

from __future__ import annotations

import logging
import os
from dataclasses import dataclass
from typing import List, Optional

logger = logging.getLogger(__name__)

PROJECT_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..", ".."))
DEFAULT_MODELS_ROOT = os.environ.get(
    "PHOTOSORT_MODELS_DIR", os.path.join(PROJECT_ROOT, "models")
)


class ModelDependencyError(Exception):
    """Raised when one or more required models are missing."""

    def __init__(self, missing_models: List["MissingModelInfo"]):
        self.missing_models = missing_models
        super().__init__(self._format_message())

    def _format_message(self) -> str:
        model_names = ", ".join(m.name for m in self.missing_models)
        return f"Required models not found: {model_names}"


@dataclass
class MissingModelInfo:
    """Information about a missing model dependency."""

    name: str
    description: str
    expected_path: str
    download_url: str


def check_best_shot_models(models_root: Optional[str] = None) -> List[MissingModelInfo]:
    """
    Check for the presence of all required best-shot analysis models.

    Args:
        models_root: Root directory where models are stored. Defaults to
                     PHOTOSORT_MODELS_DIR env var or PROJECT_ROOT/models.

    Returns:
        List of MissingModelInfo for each missing model. Empty list if all present.
    """
    models_root = models_root or DEFAULT_MODELS_ROOT
    missing: List[MissingModelInfo] = []

    # 1. Face detector (BlazeFace ONNX)
    face_detector_paths = [
        os.path.join(models_root, "job_jgzjewkop_optimized_onnx", "model.onnx"),
        os.path.join(
            models_root,
            "MediaPipe-Face-Detection_FaceDetector_float",
            "model.onnx",
        ),
    ]
    if not any(os.path.exists(p) for p in face_detector_paths):
        missing.append(
            MissingModelInfo(
                name="Face Detector",
                description="MediaPipe BlazeFace ONNX model for face detection",
                expected_path=os.path.join(models_root, "job_*/model.onnx"),
                download_url="https://huggingface.co/qualcomm/MediaPipe-Face-Detection",
            )
        )

    # 2. Eye-state classifier
    eye_classifier_dir = os.path.join(
        models_root, "open-closed-eye-classification-mobilev2"
    )
    if not os.path.isdir(eye_classifier_dir):
        missing.append(
            MissingModelInfo(
                name="Eye Classifier",
                description="MobileNetV2 model for open/closed eye classification",
                expected_path=eye_classifier_dir,
                download_url="https://huggingface.co/MichalMlodawski/open-closed-eye-classification-mobilev2",
            )
        )

    # 3. Aesthetic predictor
    aesthetic_dir = os.path.join(models_root, "aesthetic_predictor")
    if not os.path.isdir(aesthetic_dir):
        missing.append(
            MissingModelInfo(
                name="Aesthetic Predictor",
                description="CLIP-based aesthetic scoring model",
                expected_path=aesthetic_dir,
                download_url="https://huggingface.co/shunk031/aesthetics-predictor-v2-sac-logos-ava1-l14-linearMSE",
            )
        )

    # 4. BlazeFace anchors (bundled, but check just in case)
    bundled_anchors = os.path.join(
        os.path.dirname(__file__), "data", "blazeface_anchors.npy"
    )
    user_anchors = os.path.join(models_root, "blazeface_anchors.npy")
    if not os.path.exists(bundled_anchors) and not os.path.exists(user_anchors):
        missing.append(
            MissingModelInfo(
                name="BlazeFace Anchors",
                description="Anchor tensor for BlazeFace detector (usually bundled)",
                expected_path=user_anchors,
                download_url="https://github.com/duartebarbosadev/PhotoSort",
            )
        )

    if missing:
        logger.warning(
            "Best-shot models check failed: %d model(s) missing",
            len(missing),
        )
    else:
        logger.info("All best-shot models are present.")

    return missing


def ensure_best_shot_models(models_root: Optional[str] = None) -> None:
    """
    Verify all best-shot models are present, raising ModelDependencyError if not.

    Args:
        models_root: Root directory where models are stored.

    Raises:
        ModelDependencyError: If any required model is missing.
    """
    missing = check_best_shot_models(models_root)
    if missing:
        raise ModelDependencyError(missing)


__all__ = [
    "ModelDependencyError",
    "MissingModelInfo",
    "check_best_shot_models",
    "ensure_best_shot_models",
]

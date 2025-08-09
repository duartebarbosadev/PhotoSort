"""Lazy-loading rotation detector for image orientation.

Safe to import without pulling heavy ML deps. Heavy libraries (onnxruntime,
torchvision, Pillow) and the ONNX session are loaded only when first needed.
If anything is missing, detector stays disabled and returns 0.
"""
from __future__ import annotations

import glob
import logging
import os
from dataclasses import dataclass
from typing import Optional, Protocol, runtime_checkable, Any

import numpy as np

from src.core.image_processing.raw_image_processor import (
    is_raw_extension,
    RawImageProcessor,
)
from src.core.app_settings import (
    get_orientation_model_name,
    set_orientation_model_name,
    ROTATION_MODEL_IMAGE_SIZE,
)

logger = logging.getLogger(__name__)

MODEL_SAVE_DIR = "models"
IMAGE_SIZE = ROTATION_MODEL_IMAGE_SIZE

CLASS_TO_ANGLE_MAP = {0: 0, 1: 90, 2: 180, 3: -90}


class ModelNotFoundError(Exception):
    pass


@runtime_checkable
class RotationDetectorProtocol(Protocol):  # pragma: no cover - structural typing
    def predict_rotation_angle(
    self, image_path: str, image: Optional[object] = None, apply_auto_edits: bool = False
    ) -> int: ...


try:  # pragma: no cover - optional pillow import for typing friendliness
    from PIL import Image as ImageType  # type: ignore
except Exception:  # noqa: BLE001
    ImageType = object  # type: ignore


@dataclass
class _LazyState:
    tried_load: bool = False
    load_failed: bool = False
    failure_logged: bool = False
    session: Any = None
    input_name: Optional[str] = None
    output_name: Optional[str] = None
    provider_name: Optional[str] = None
    transforms: Any = None


class ModelRotationDetector(RotationDetectorProtocol):
    _instance: Optional["ModelRotationDetector"] = None

    def __new__(cls, *args, **kwargs):  # singleton pattern
        if cls._instance is None:
            cls._instance = super().__new__(cls)
            cls._instance._state = _LazyState()
        return cls._instance

    # ----------------------------- Public API ----------------------------- #
    def predict_rotation_angle(
        self,
        image_path: str,
    image: Optional[object] = None,
        apply_auto_edits: bool = False,
    ) -> int:
        if not self._ensure_session_loaded():
            return 0

        if image is None:
            image = self._load_image(image_path, apply_auto_edits=apply_auto_edits)

        if image is None:
            return 0

        try:
            input_tensor = self._state.transforms(image).unsqueeze(0)
            input_np = input_tensor.cpu().numpy()
            result = self._state.session.run([self._state.output_name], {self._state.input_name: input_np})
            predicted_idx = int(np.argmax(result[0], axis=1)[0])
            return CLASS_TO_ANGLE_MAP.get(predicted_idx, 0)
        except Exception:  # noqa: BLE001
            if not self._state.failure_logged:
                logger.error("Rotation inference failed; disabling detector.", exc_info=True)
                self._state.failure_logged = True
            return 0

    # --------------------------- Lazy Load Logic -------------------------- #
    def _ensure_session_loaded(self) -> bool:
        s = self._state
        if s.session is not None:
            return True
        if s.tried_load and s.load_failed:
            return False
        s.tried_load = True

        try:  # pragma: no cover
            import onnxruntime as ort  # type: ignore
            import torchvision.transforms as transforms  # type: ignore
        except Exception as e:  # noqa: BLE001
            s.load_failed = True
            if not s.failure_logged:
                logger.warning(
                    "Rotation model dependencies missing (onnxruntime/torchvision); detector disabled: %s", e
                )
                s.failure_logged = True
            return False

        model_path = self._resolve_model_path()
        if not model_path:
            s.load_failed = True
            return False

        try:
            s.transforms = transforms.Compose(
                [
                    transforms.Resize((IMAGE_SIZE + 32, IMAGE_SIZE + 32)),
                    transforms.CenterCrop(IMAGE_SIZE),
                    transforms.ToTensor(),
                    transforms.Normalize(mean=[0.485, 0.456, 0.406], std=[0.229, 0.224, 0.225]),
                ]
            )
            providers_pref = [
                "CUDAExecutionProvider",
                "DmlExecutionProvider",
                "MpsExecutionProvider",
                "ROCmExecutionProvider",
                "CoreMLExecutionProvider",
                "CPUExecutionProvider",
            ]
            available = ort.get_available_providers()
            chosen = "CPUExecutionProvider"
            for p in providers_pref:
                if p in available:
                    chosen = p
                    break
            s.session = ort.InferenceSession(model_path, providers=[chosen])
            s.input_name = s.session.get_inputs()[0].name
            s.output_name = s.session.get_outputs()[0].name
            s.provider_name = chosen
            logger.info("Rotation model loaded with provider %s", chosen)
            return True
        except Exception:  # noqa: BLE001
            s.load_failed = True
            if not s.failure_logged:
                logger.error("Failed to initialize rotation model; detector disabled", exc_info=True)
                s.failure_logged = True
            return False

    # --------------------------- Helper Functions ------------------------- #
    def _resolve_model_path(self) -> Optional[str]:
        model_name = get_orientation_model_name()
        model_path = None
        if model_name:
            candidate = os.path.join(MODEL_SAVE_DIR, model_name)
            if os.path.exists(candidate):
                model_path = candidate
            else:
                logger.warning("Configured rotation model '%s' not found.", model_name)
        if not model_path:
            pattern = os.path.join(MODEL_SAVE_DIR, "orientation_model*.onnx")
            models = glob.glob(pattern)
            if models:
                model_path = max(models, key=os.path.basename)
                set_orientation_model_name(os.path.basename(model_path))
                logger.info("Auto-selected rotation model %s", os.path.basename(model_path))
            else:
                if not self._state.failure_logged:
                    logger.error("No orientation model found; rotation detection disabled.")
                    self._state.failure_logged = True
                return None
        return model_path

    def _load_image(self, path: str, apply_auto_edits: bool):
        try:
            norm = os.path.normpath(path)
            _, ext = os.path.splitext(norm)
            if is_raw_extension(ext):
                return RawImageProcessor.load_raw_as_pil(norm, half_size=True, apply_auto_edits=apply_auto_edits)
            from PIL import Image, ImageOps  # type: ignore
            img = Image.open(norm)
            img = ImageOps.exif_transpose(img)
            if img.mode in ("RGB", "L"):
                return img.convert("RGB")
            rgba = img.convert("RGBA")
            bg = Image.new("RGB", rgba.size, (255, 255, 255))
            bg.paste(rgba, mask=rgba)
            return bg
        except FileNotFoundError:
            logger.error("Rotation detector: file not found %s", os.path.basename(path))
            return None
        except Exception as e:  # noqa: BLE001
            logger.error("Failed loading image for rotation detection %s: %s", os.path.basename(path), e)
            return None


__all__ = ["ModelRotationDetector", "RotationDetectorProtocol", "ModelNotFoundError"]

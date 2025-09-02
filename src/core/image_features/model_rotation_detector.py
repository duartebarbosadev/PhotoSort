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

from core.image_processing.raw_image_processor import (
    is_raw_extension,
    RawImageProcessor,
)
from core.app_settings import (
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
        self,
        image_path: str,
        image: Optional[object] = None,
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
    ) -> int:
        if not self._ensure_session_loaded():
            return 0

        if image is None:
            image = self._load_image(image_path)

        if image is None:
            return 0

        try:
            input_tensor = self._state.transforms(image).unsqueeze(0)
            input_np = input_tensor.cpu().numpy()
            result = self._state.session.run(
                [self._state.output_name], {self._state.input_name: input_np}
            )
            predicted_idx = int(np.argmax(result[0], axis=1)[0])
            return CLASS_TO_ANGLE_MAP.get(predicted_idx, 0)
        except Exception:  # noqa: BLE001
            if not self._state.failure_logged:
                logger.error(
                    "Rotation inference failed; disabling detector.", exc_info=True
                )
                self._state.failure_logged = True
            return 0

    # --------------------------- Lazy Load Logic -------------------------- #
    def _ensure_session_loaded(self) -> bool:
        s = self._state
        if s.session is not None:
            return True

        # Always re-check for the model path and raise if missing so the UI can show the dialog every run
        model_path = self._resolve_model_path()
        if not model_path:
            model_name = get_orientation_model_name()
            raise ModelNotFoundError(
                f"Configured rotation model '{model_name}' not found"
                if model_name
                else "No rotation model found in any known location"
            )

        # If we previously tried and failed due to dependency or init errors (not model-missing), don't retry repeatedly
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
                    "Rotation model dependencies missing (onnxruntime/torchvision); detector disabled: %s",
                    e,
                )
                s.failure_logged = True
            return False

        try:
            s.transforms = transforms.Compose(
                [
                    transforms.Resize((IMAGE_SIZE + 32, IMAGE_SIZE + 32)),
                    transforms.CenterCrop(IMAGE_SIZE),
                    transforms.ToTensor(),
                    transforms.Normalize(
                        mean=[0.485, 0.456, 0.406], std=[0.229, 0.224, 0.225]
                    ),
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
                logger.error(
                    "Failed to initialize rotation model; detector disabled",
                    exc_info=True,
                )
                s.failure_logged = True
            return False

    # --------------------------- Helper Functions ------------------------- #
    def _resolve_model_path(self) -> Optional[str]:
        """Resolve the ONNX model path across source and frozen bundles.

        Search order:
          1) PyInstaller extraction dir (sys._MEIPASS)/models
          2) Project root (two levels up from this file)/models
          3) Current working directory ./models
        """
        # Build candidate base dirs
        base_dirs = []
        try:
            import sys as _sys

            if getattr(_sys, "_MEIPASS", None):  # type: ignore[attr-defined]
                base_dirs.append(os.path.join(_sys._MEIPASS, MODEL_SAVE_DIR))  # type: ignore[attr-defined]
        except Exception:
            pass

        project_root = os.path.abspath(
            os.path.join(os.path.dirname(__file__), "..", "..")
        )
        base_dirs.append(os.path.join(project_root, MODEL_SAVE_DIR))
        base_dirs.append(os.path.join(os.getcwd(), MODEL_SAVE_DIR))

        model_name = get_orientation_model_name()
        # 1) If a specific model is configured, try to find it in candidates
        if model_name:
            for base in base_dirs:
                candidate = os.path.join(base, model_name)
                if os.path.exists(candidate):
                    return candidate
            logger.warning(
                "Configured rotation model '%s' not found in known locations.",
                model_name,
            )

        # 2) Otherwise, glob for any orientation_model*.onnx in candidates
        found_models = []
        for base in base_dirs:
            pattern = os.path.join(base, "orientation_model*.onnx")
            found_models.extend(glob.glob(pattern))

        if found_models:
            # Pick the lexicographically last filename (often the newest version)
            model_path = max(found_models, key=os.path.basename)
            set_orientation_model_name(os.path.basename(model_path))
            logger.info("Auto-selected rotation model %s", os.path.basename(model_path))
            return model_path

        if not self._state.failure_logged:
            logger.error("No orientation model found; rotation detection disabled.")
            self._state.failure_logged = True
        return None

    def _load_image(self, path: str):
        try:
            norm = os.path.normpath(path)
            _, ext = os.path.splitext(norm)
            if is_raw_extension(ext):
                # Always apply auto-edits for RAW files in rotation detection
                return RawImageProcessor.load_raw_as_pil(
                    norm, half_size=True, apply_auto_edits=True
                )
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
            logger.error(
                "Failed loading image for rotation detection %s: %s",
                os.path.basename(path),
                e,
            )
            return None


__all__ = ["ModelRotationDetector", "RotationDetectorProtocol", "ModelNotFoundError"]

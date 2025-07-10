import os
import logging
import glob
import torchvision.transforms as transforms
from PIL import Image, ImageOps
from typing import Optional, Dict
import onnxruntime as ort
import numpy as np

from src.core.image_processing.raw_image_processor import (
    is_raw_extension,
    RawImageProcessor,
)
from src.core.app_settings import (
    get_orientation_model_name,
    set_orientation_model_name,
)


class ModelNotFoundError(Exception):
    """Custom exception for when the ONNX model file is not found."""

    pass


# --- Constants for the model ---
MODEL_SAVE_DIR = "models"
IMAGE_SIZE = 384

CLASS_TO_ANGLE_MAP = {
    0: 0,    # Correctly oriented
    1: 90,   # Needs 90° Clockwise rotation to be correct
    2: 180,  # Needs 180° rotation to be correct
    3: -90,  # Needs 90° Counter-Clockwise rotation to be correct
}


def get_data_transforms() -> Dict[str, transforms.Compose]:
    """
    Returns a dictionary of data transformations for validation.
    """
    return {
        "val": transforms.Compose(
            [
                transforms.Resize((IMAGE_SIZE + 32, IMAGE_SIZE + 32)),
                transforms.CenterCrop(IMAGE_SIZE),
                transforms.ToTensor(),
                transforms.Normalize(
                    mean=[0.485, 0.456, 0.406], std=[0.229, 0.224, 0.225]
                ),
            ]
        ),
    }


def load_image_safely(path: str, apply_auto_edits: bool) -> Optional[Image.Image]:
    """
    Loads an image, respects EXIF orientation, and safely converts it to a
    3-channel RGB format. It handles palletized images and images with
    transparency by compositing them onto a white background.
    Handles both standard and RAW image formats.
    """
    normalized_path = os.path.normpath(path)
    _, ext = os.path.splitext(normalized_path)

    try:
        if is_raw_extension(ext):
            # RAW processing is handled by the dedicated processor
            logging.debug(
                f"Using RawImageProcessor to load {normalized_path} for rotation detection."
            )
            return RawImageProcessor.load_raw_as_pil(
                normalized_path, half_size=True, apply_auto_edits=apply_auto_edits
            )
        else:
            # Standard image loading
            img = Image.open(normalized_path)

            # Respect the EXIF orientation tag before any other processing.
            img = ImageOps.exif_transpose(img)

            # If the image is already in a simple mode that can be directly
            # converted to RGB, do it and return.
            if img.mode in ("RGB", "L"):  # L is grayscale
                return img.convert("RGB")

            # For all other modes (P, PA, RGBA), convert to RGBA first to
            # standardize and handle transparency correctly.
            rgba_img = img.convert("RGBA")

            # Create a new white background and paste the image onto it.
            background = Image.new("RGB", rgba_img.size, (255, 255, 255))
            background.paste(rgba_img, mask=rgba_img)

            return background

    except FileNotFoundError:
        logging.error(f"File not found during safe load: {normalized_path}")
        return None
    except Exception as e:
        logging.error(f"Error loading image {normalized_path} safely: {e}")
        return None


def find_best_orientation_model() -> Optional[str]:
    """
    Finds the best orientation model in the models directory.
    'Best' is defined as the one with the highest version number or score.
    """
    model_pattern = os.path.join(MODEL_SAVE_DIR, "orientation_model*.onnx")
    models = glob.glob(model_pattern)
    if not models:
        return None
    # Simple sort will work if file names are consistent (e.g., v1, v2)
    return max(models, key=os.path.basename)


class ModelRotationDetector:
    """
    Detects image orientation using a pre-trained ONNX model.
    This class is a singleton to ensure the model is loaded only once.
    """

    _instance = None

    def __new__(cls, *args, **kwargs):
        if not cls._instance:
            cls._instance = super(ModelRotationDetector, cls).__new__(cls)
            cls._instance.initialized = False
        return cls._instance

    def __init__(self):
        if self.initialized:
            return

        self.session: Optional[ort.InferenceSession] = None
        self.input_name: Optional[str] = None
        self.output_name: Optional[str] = None
        self.provider_name: Optional[str] = None

        model_name = get_orientation_model_name()
        model_path = None

        if model_name:
            model_path = os.path.join(MODEL_SAVE_DIR, model_name)
            if not os.path.exists(model_path):
                logging.warning(
                    f"Saved model '{model_name}' not found. Searching for a new one."
                )
                model_path = None # Reset to trigger auto-detection

        if not model_path:
            model_path = find_best_orientation_model()
            if model_path:
                set_orientation_model_name(os.path.basename(model_path))
                logging.info(f"Auto-detected orientation model: {os.path.basename(model_path)}")


        if not model_path:
             logging.warning("No orientation model found. Rotation detection disabled.")
             self.initialized = True # Mark as initialized to prevent re-attempts
             return

        try:
            self.transforms = get_data_transforms()["val"]
            self.session, self.input_name, self.output_name = self._load_onnx_session(
                model_path
            )
            self.initialized = True
            logging.info("ModelRotationDetector initialized with ONNX.")
        except ModelNotFoundError:
            # Re-raise to be caught by the calling worker
            raise
        except Exception as e:
            logging.error(
                f"An unexpected error occurred during ModelRotationDetector initialization: {e}"
            )

    def _load_onnx_session(
        self, model_path: str
    ) -> tuple[Optional[ort.InferenceSession], Optional[str], Optional[str]]:
        """
        Loads the ONNX model, selecting the best available provider.
        """
        if not os.path.exists(model_path):
            logging.error(
                f"ONNX model file not found at {model_path}. Rotation detection will be disabled. "
                f"Please download the model from https://github.com/duartebarbosadev/deep-image-orientation-detection "
                f"and place it in the '{MODEL_SAVE_DIR}' directory."
            )
            raise ModelNotFoundError(model_path)

        # Define a priority list for execution providers.
        PREFERRED_PROVIDERS = [
            # "TensorrtExecutionProvider",  # For NVIDIA GPUs with TensorRT support (commented out for now)
            "CUDAExecutionProvider",  # For NVIDIA GPUs
            "DmlExecutionProvider",  # For DirectML on Windows
            "MpsExecutionProvider",  # For Apple Silicon (M1/M2/M3) GPUs
            "ROCmExecutionProvider",  # For AMD GPUs
            "CoreMLExecutionProvider",  # For Apple devices (can use Neural Engine)
            "CPUExecutionProvider",  # Universal fallback
        ]

        try:
            available_providers = ort.get_available_providers()
            logging.info(f"Available ONNX Runtime providers: {available_providers}")

            chosen_provider = "CPUExecutionProvider"  # Default fallback
            for provider in PREFERRED_PROVIDERS:
                if provider in available_providers:
                    chosen_provider = provider
                    break

            logging.info(
                f"Attempting to load ONNX model with provider: {chosen_provider}"
            )

            # Load the ONNX model with the single, highest-priority available provider
            session = ort.InferenceSession(model_path, providers=[chosen_provider])

            actual_provider = session.get_providers()[0]
            self.provider_name = actual_provider
            logging.info(
                f"Successfully loaded ONNX model from {model_path} using provider: {actual_provider}"
            )

            if (
                chosen_provider != actual_provider
                and actual_provider == "CPUExecutionProvider"
            ):
                logging.warning(
                    f"Warning: ONNX Runtime fell back to CPU. The chosen provider "
                    f"'{chosen_provider}' might not be correctly configured."
                )

            input_name = session.get_inputs()[0].name
            output_name = session.get_outputs()[0].name
            return session, input_name, output_name

        except Exception as e:
            logging.error(f"Error loading ONNX model {model_path}: {e}")
            logging.error(
                "If you are trying to use a GPU provider (CUDA, DML, ROCm, MPS), "
                "please ensure the correct onnxruntime package is installed and any necessary drivers are up to date."
            )
            return None, None, None

    def predict_rotation_angle(
        self,
        image_path: str,
        image: Optional[Image.Image] = None,
        apply_auto_edits: bool = False,
    ) -> int:
        """
        Predicts the rotation angle for a single image using the ONNX model.
        If an image object is provided, it's used directly. Otherwise, the image is loaded.
        Returns 0, 90, 180, or -90.
        """
        if self.session is None:
            logging.warning("ONNX session is not loaded. Skipping prediction.")
            return 0

        if image is None:
            image = load_image_safely(image_path, apply_auto_edits=apply_auto_edits)

        if image is None:
            logging.warning(
                f"Could not load or receive image for {image_path}. Skipping rotation detection."
            )
            return 0

        # Transform image and convert to numpy
        input_tensor = self.transforms(image).unsqueeze(0)
        input_np = input_tensor.cpu().numpy()

        # Run inference
        try:
            result = self.session.run([self.output_name], {self.input_name: input_np})
            predicted_idx = np.argmax(result[0], axis=1)[0]

            predicted_class = int(predicted_idx)
            angle = CLASS_TO_ANGLE_MAP.get(predicted_class, 0)

            logging.info(
                f"Model predicting rotation for: {os.path.basename(image_path)}"
            )
            logging.debug(
                f"-> Image: '{os.path.basename(image_path)}' | Prediction: {predicted_class} -> Angle: {angle}°"
            )

            return angle
        except Exception as e:
            logging.error(f"Error during ONNX inference for {image_path}: {e}")
            return 0

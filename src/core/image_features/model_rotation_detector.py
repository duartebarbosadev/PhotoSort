import os
import logging
import torch
import torchvision.transforms as transforms
from PIL import Image
from typing import Optional, Dict
import onnxruntime as ort
import numpy as np

from src.core.image_processing.raw_image_processor import is_raw_extension, RawImageProcessor
 
class ModelNotFoundError(Exception):
    """Custom exception for when the ONNX model file is not found."""
    pass

# --- Constants for the model ---
MODEL_SAVE_DIR = "models"
MODEL_PATH = os.path.join(MODEL_SAVE_DIR, "orientation_model_v1_0.9753.onnx")
IMAGE_SIZE = 384
NUM_CLASSES = 4

# Map model output index to a rotation angle in degrees for Qt (positive is clockwise).
CLASS_TO_ANGLE_MAP = {
    0: 0,    # Correctly oriented (0°)
    1: -90,  # Needs 90° Counter-Clockwise rotation
    2: 180,  # Needs 180° rotation
    3: 90,   # Needs 90° Clockwise rotation
}


def get_data_transforms() -> Dict[str, transforms.Compose]:
    """
    Returns a dictionary of data transformations for validation.
    """
    return {
        'val': transforms.Compose([
            transforms.Resize((IMAGE_SIZE + 32, IMAGE_SIZE + 32)),
            transforms.CenterCrop(IMAGE_SIZE),
            transforms.ToTensor(),
            transforms.Normalize(mean=[0.485, 0.456, 0.406], std=[0.229, 0.224, 0.225]),
        ]),
    }

def load_image_safely(path: str) -> Optional[Image.Image]:
    """
    Loads an image, safely converting it to a 3-channel RGB format.
    Handles both standard and RAW image formats.
    """
    normalized_path = os.path.normpath(path)
    _, ext = os.path.splitext(normalized_path)

    try:
        if is_raw_extension(ext):
            # Use the project's RawImageProcessor for RAW files
            logging.debug(f"Using RawImageProcessor to load {normalized_path} for rotation detection.")
            return RawImageProcessor.load_raw_as_pil(normalized_path, half_size=True, apply_auto_edits=True)
        else:
            # Use standard Pillow loading for other formats
            img = Image.open(normalized_path)
            if img.mode in ('RGB', 'L'):
                return img.convert('RGB')
            
            # Handle formats with transparency like PNG
            rgba_img = img.convert('RGBA')
            background = Image.new("RGB", rgba_img.size, (255, 255, 255))
            background.paste(rgba_img, mask=rgba_img)
            return background

    except FileNotFoundError:
        logging.error(f"File not found during safe load: {normalized_path}")
        return None
    except Exception as e:
        logging.error(f"Error loading image {normalized_path} safely: {e}")
        return None

class ModelRotationDetector:
    """
    Detects image orientation using a pre-trained ONNX model.
    This class is a singleton to ensure the model is loaded only once.
    """
    _instance = None
    
    def __new__(cls, *args, **kwargs):
        if not cls._instance:
            cls._instance = super(ModelRotationDetector, cls).__new__(cls)
        return cls._instance

    def __init__(self, model_path: str = MODEL_PATH):
        if hasattr(self, 'initialized') and self.initialized:
            return
        
        self.session = None
        self.input_name = None
        self.output_name = None
        self.initialized = False

        try:
            self.transforms = get_data_transforms()['val']
            self.session, self.input_name, self.output_name = self._load_onnx_session(model_path)
            self.initialized = True
            logging.info("ModelRotationDetector initialized with ONNX.")
        except ModelNotFoundError:
            # Re-raise to be caught by the calling worker
            raise
        except Exception as e:
            logging.error(f"An unexpected error occurred during ModelRotationDetector initialization: {e}")


    def _get_onnx_providers(self) -> list[str]:
        """Gets available ONNX Runtime providers, preferring GPU if available."""
        providers = ['CPUExecutionProvider']
        available_providers = ort.get_available_providers()
        
        if 'CUDAExecutionProvider' in available_providers:
            providers.insert(0, 'CUDAExecutionProvider')
            logging.info("ONNX Runtime: Using CUDAExecutionProvider.")
        elif 'DmlExecutionProvider' in available_providers:
            providers.insert(0, 'DmlExecutionProvider')
            logging.info("ONNX Runtime: Using DmlExecutionProvider for DirectML.")
        # MPS is for macOS, might not be relevant but good to have
        elif 'CoreMLExecutionProvider' in available_providers:
            providers.insert(0, 'CoreMLExecutionProvider')
            logging.info("ONNX Runtime: Using CoreMLExecutionProvider.")
            
        return providers

    def _load_onnx_session(self, model_path: str) -> tuple[Optional[ort.InferenceSession], Optional[str], Optional[str]]:
        """Loads the ONNX model into an InferenceSession."""
        if not os.path.exists(model_path):
            logging.error(
                f"ONNX model file not found at {model_path}. Rotation detection will be disabled. "
                f"Please download the model from https://github.com/duartebarbosadev/deep-image-orientation-detection "
                f"and place it in the '{MODEL_SAVE_DIR}' directory."
            )
            raise ModelNotFoundError(model_path)
        
        try:
            providers = self._get_onnx_providers()
            session = ort.InferenceSession(model_path, providers=providers)
            input_name = session.get_inputs()[0].name
            output_name = session.get_outputs()[0].name
            logging.info(f"Successfully loaded ONNX model from {model_path} with providers: {session.get_providers()}")
            return session, input_name, output_name
        except Exception as e:
            logging.error(f"Error loading ONNX model from {model_path}: {e}")
            return None, None, None

    def predict_rotation_angle(self, image_path: str, image: Optional[Image.Image] = None) -> int:
        """
        Predicts the rotation angle for a single image using the ONNX model.
        If an image object is provided, it's used directly. Otherwise, the image is loaded.
        Returns 0, 90, 180, or -90.
        """
        if self.session is None:
            logging.warning("ONNX session is not loaded. Skipping prediction.")
            return 0
        
        if image is None:
            image = load_image_safely(image_path)
        
        if image is None:
            logging.warning(f"Could not load or receive image for {image_path}. Skipping rotation detection.")
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

            logging.info(f"Model predicting rotation for: {os.path.basename(image_path)}")
            logging.debug(f"-> Image: '{os.path.basename(image_path)}' | Prediction: {predicted_class} -> Angle: {angle}°")

            return angle
        except Exception as e:
            logging.error(f"Error during ONNX inference for {image_path}: {e}")
            return 0
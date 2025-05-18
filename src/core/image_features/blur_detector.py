import os
import cv2
import numpy as np
from PIL import Image, UnidentifiedImageError
from typing import Optional, Tuple, List, Dict, Callable, Any
import concurrent.futures

# Assuming these processors are in the new structure
from src.core.image_processing.raw_image_processor import RawImageProcessor, is_raw_extension
from src.core.image_processing.standard_image_processor import StandardImageProcessor, SUPPORTED_STANDARD_EXTENSIONS

# Default size for the image used in blur detection
BLUR_DETECTION_PREVIEW_SIZE: Tuple[int, int] = (640, 480)
DEFAULT_NUM_WORKERS = min(os.cpu_count() or 4, 8) # Default number of workers for batch processing

class BlurDetector:
    """Detects blurriness in images."""

    @staticmethod
    def _load_image_for_detection(
        image_path: str,
        target_size: Tuple[int, int] = BLUR_DETECTION_PREVIEW_SIZE,
        apply_auto_edits_for_raw: bool = False
    ) -> Optional[Image.Image]:
        """
        Loads and prepares a PIL image (RGB) for blur detection, scaled to target_size.
        Uses RawImageProcessor or StandardImageProcessor based on file type.
        """
        normalized_path = os.path.normpath(image_path)
        pil_img: Optional[Image.Image] = None
        
        try:
            ext = os.path.splitext(normalized_path)[1].lower()

            if is_raw_extension(ext):
                pil_img = RawImageProcessor.load_raw_for_blur_detection(
                    normalized_path,
                    target_size=target_size,
                    apply_auto_edits=apply_auto_edits_for_raw
                )
            elif ext in SUPPORTED_STANDARD_EXTENSIONS:
                pil_img = StandardImageProcessor.load_for_blur_detection(
                    normalized_path,
                    target_size=target_size
                )
            else:
                # Fallback for unknown extensions, try opening with Pillow directly
                # This part might be redundant if StandardImageProcessor handles more or
                # if we decide unsupported types are not processed for blur.
                try:
                    print(f"[BlurDetector] Unknown extension '{ext}', attempting to load with Pillow for blur detection.")
                    img = Image.open(normalized_path)
                    # StandardImageProcessor.load_for_blur_detection already handles exif_transpose
                    # So, if we directly use Image.open, we should also apply it.
                    from src.core.image_processing.image_orientation_handler import ImageOrientationHandler # Local import
                    img = ImageOrientationHandler.exif_transpose(img)
                    img.thumbnail(target_size, Image.Resampling.LANCZOS)
                    pil_img = img.convert("RGB")
                except UnidentifiedImageError:
                    print(f"[BlurDetector] Pillow could not identify unknown image type for blur: {normalized_path}")
                    return None
                except FileNotFoundError:
                    print(f"[BlurDetector] File not found for unknown type blur detection: {normalized_path}")
                    return None
            
            return pil_img

        except Exception as e:
            print(f"[BlurDetector] Error in _load_image_for_detection for {normalized_path}: {e}")
            return None

    @staticmethod
    def is_image_blurred(
        image_path: str,
        threshold: float = 100.0,
        apply_auto_edits_for_raw_preview: bool = False,
        target_size: Tuple[int, int] = BLUR_DETECTION_PREVIEW_SIZE
    ) -> Optional[bool]:
        """
        Detects if an image is blurred using the variance of the Laplacian method.
        Operates on a smaller, efficiently loaded preview of the image.

        Args:
            image_path (str): The path to the image file.
            threshold (float): The threshold for blur detection. Lower values indicate more blur.
            apply_auto_edits_for_raw_preview (bool): If RAW, whether to apply auto edits
                                                     to the preview used for blur detection.
            target_size (Tuple[int, int]): The target size for the image used in blur detection.

        Returns:
            Optional[bool]: True if blurred, False if not, None if an error occurs.
        """
        normalized_path = os.path.normpath(image_path)
        if not os.path.isfile(normalized_path):
            print(f"[BlurDetector] Error: File does not exist for blur detection: {normalized_path}")
            return None

        try:
            pil_image_rgb = BlurDetector._load_image_for_detection(
                normalized_path,
                target_size=target_size,
                apply_auto_edits_for_raw=apply_auto_edits_for_raw_preview
            )

            if pil_image_rgb is None:
                # _load_image_for_detection would have printed an error
                return None

            # Convert PIL Image (RGB) to OpenCV format (BGR)
            open_cv_image = cv2.cvtColor(np.array(pil_image_rgb), cv2.COLOR_RGB2BGR)

            if open_cv_image is None: # Should not happen if pil_image_rgb is valid
                print(f"[BlurDetector] OpenCV could not convert PIL data for {normalized_path}")
                return None

            gray = cv2.cvtColor(open_cv_image, cv2.COLOR_BGR2GRAY)
            laplacian_var = cv2.Laplacian(gray, cv2.CV_64F).var()
            
            print(f"[BlurDetector] Blur detection for {os.path.basename(normalized_path)}: Laplacian Variance = {laplacian_var:.2f}, Threshold = {threshold}, Blurred = {laplacian_var < threshold}")

            return laplacian_var < threshold

        except UnidentifiedImageError: # Should be caught by _load_image_for_detection
            print(f"[BlurDetector] Pillow could not identify image file: {normalized_path}")
            return None
        except Exception as e:
            print(f"[BlurDetector] Error during blur detection for {normalized_path}: {e} (Type: {type(e).__name__})")
            return None

    @staticmethod
    def _detect_blur_task(
        image_path: str,
        threshold: float,
        apply_auto_edits_for_raw_preview: bool,
        target_size: Tuple[int, int],
        status_update_callback: Optional[Callable[[str, Optional[bool]], None]]
    ) -> None:
        """
        Worker task for detecting blur in a single image and calling the status callback.
        """
        try:
            is_blurred = BlurDetector.is_image_blurred(
                image_path,
                threshold,
                apply_auto_edits_for_raw_preview,
                target_size
            )
            if status_update_callback:
                status_update_callback(image_path, is_blurred)
        except Exception as e:
            print(f"[BlurDetectorTask] Error processing {image_path}: {e}")
            if status_update_callback:
                status_update_callback(image_path, None) # Report error as None

    @staticmethod
    def detect_blur_in_batch(
        image_paths: List[str],
        threshold: float = 100.0,
        apply_auto_edits_for_raw_preview: bool = False,
        target_size: Tuple[int, int] = BLUR_DETECTION_PREVIEW_SIZE,
        status_update_callback: Optional[Callable[[str, Optional[bool]], None]] = None,
        progress_callback: Optional[Callable[[int, int, str], None]] = None,
        should_continue_callback: Optional[Callable[[], bool]] = None,
        num_workers: Optional[int] = None
    ) -> None:
        """
        Detects blurriness for a batch of images in parallel.
        Invokes status_update_callback for each image result and progress_callback periodically.
        """
        total_files = len(image_paths)
        processed_count = 0
        
        effective_num_workers = num_workers if num_workers is not None else DEFAULT_NUM_WORKERS
        print(f"[BlurDetectorBatch] Starting for {total_files} files, workers: {effective_num_workers}")

        with concurrent.futures.ThreadPoolExecutor(max_workers=effective_num_workers) as executor:
            futures_map: Dict[concurrent.futures.Future, str] = {}
            for image_path in image_paths:
                if should_continue_callback and not should_continue_callback():
                    print("[BlurDetectorBatch] Cancellation requested, stopping new tasks.")
                    break
                
                future = executor.submit(
                    BlurDetector._detect_blur_task,
                    image_path,
                    threshold,
                    apply_auto_edits_for_raw_preview,
                    target_size,
                    status_update_callback # Pass the callback to the task
                )
                futures_map[future] = image_path
            
            for future in concurrent.futures.as_completed(futures_map):
                path_for_future = futures_map[future]
                try:
                    future.result() # _detect_blur_task doesn't return, it calls callback. Wait for completion/exception.
                except Exception as e:
                    print(f"[BlurDetectorBatch] Error processing future for {path_for_future}: {e}")
                    if status_update_callback: # Ensure callback for error if task itself didn't catch it
                        status_update_callback(path_for_future, None)
                
                processed_count += 1
                if progress_callback:
                    progress_callback(processed_count, total_files, os.path.basename(path_for_future))
                
                if should_continue_callback and not should_continue_callback():
                    for f_cancel in futures_map: # Cancel remaining futures
                        if not f_cancel.done(): f_cancel.cancel()
                    print("[BlurDetectorBatch] Processing cancelled during completion.")
                    break
        print(f"[BlurDetectorBatch] Finished. Processed {processed_count}/{total_files} files.")

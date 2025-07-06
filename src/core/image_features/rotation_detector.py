import os
import concurrent.futures
import logging
from typing import Optional, Tuple, List, Callable, Dict

# Import the new model-based detector
from src.core.image_features.model_rotation_detector import ModelRotationDetector

# Default number of workers for parallel processing
DEFAULT_NUM_WORKERS = min(os.cpu_count() or 4, 8)

from src.core.image_pipeline import ImagePipeline


class RotationDetector:
    """
    Detects image orientation using a pre-trained deep learning model.
    This class manages the batch processing of images and delegates the
    actual prediction to the ModelRotationDetector.
    """
    def __init__(self, image_pipeline: ImagePipeline):
        """
        Initializes the RotationDetector with an ImagePipeline instance.
        """
        logging.info("Initializing RotationDetector...")
        self.image_pipeline = image_pipeline
        self.model_detector = ModelRotationDetector()
        logging.info("RotationDetector initialized.")

    def _detect_rotation_task(
        self,
        image_path: str,
        result_callback: Optional[Callable[[str, int], None]],
        apply_auto_edits: bool
    ) -> None:
        """
        Worker task for detecting rotation for a single image using the model.
        It uses the ImagePipeline to get a cached preview for speed.
        """
        try:
            # Use the pipeline to get a cached PIL image for processing
            pil_image = self.image_pipeline.get_pil_image_for_processing(
                image_path,
                apply_auto_edits=apply_auto_edits, # Use model-specific loading
                use_preloaded_preview_if_available=False, # Use fresh load to ensure no EXIF
                apply_exif_transpose=False # Pass the raw pixel data to the model
            )
 
             # Pass the pre-loaded image to the model detector
            suggested_rotation = self.model_detector.predict_rotation_angle(image_path, image=pil_image, apply_auto_edits=apply_auto_edits)
            
            if result_callback:
                result_callback(image_path, suggested_rotation)
        except Exception as e:
            logging.error(f"Error processing rotation for {image_path}: {e}")
            if result_callback:
                result_callback(image_path, 0)  # Default to no rotation on error

    def detect_rotation_in_batch(
        self,
        image_paths: List[str],
        result_callback: Optional[Callable[[str, int], None]] = None,
        progress_callback: Optional[Callable[[int, int, str], None]] = None,
        should_continue_callback: Optional[Callable[[], bool]] = None,
        num_workers: Optional[int] = None,
        **kwargs  # Accept and ignore extra arguments for compatibility
    ) -> None:
        """
        Detects rotation suggestions for a batch of images in parallel using the model.
        """
        total_files = len(image_paths)
        processed_count = 0
        apply_auto_edits = kwargs.get('apply_auto_edits', False)
        
        effective_num_workers = num_workers if num_workers is not None else DEFAULT_NUM_WORKERS
        logging.info(f"Starting model-based rotation detection for {total_files} files, workers: {effective_num_workers}")

        with concurrent.futures.ThreadPoolExecutor(max_workers=effective_num_workers) as executor:
            futures_map: Dict[concurrent.futures.Future, str] = {}
            for image_path in image_paths:
                if should_continue_callback and not should_continue_callback():
                    logging.info("Cancellation requested, stopping new rotation detection tasks.")
                    break
                
                future = executor.submit(
                    self._detect_rotation_task,
                    image_path,
                    result_callback,
                    apply_auto_edits
                )
                futures_map[future] = image_path
            
            for future in concurrent.futures.as_completed(futures_map):
                path_for_future = futures_map[future]
                try:
                    future.result()  # Wait for completion and handle exceptions
                except Exception as e:
                    logging.error(f"Error processing rotation future for {path_for_future}: {e}")
                    if result_callback:
                        result_callback(path_for_future, 0)
                
                processed_count += 1
                if progress_callback:
                    progress_callback(processed_count, total_files, os.path.basename(path_for_future))
                
                if should_continue_callback and not should_continue_callback():
                    for f_cancel in futures_map:
                        if not f_cancel.done():
                            f_cancel.cancel()
                    logging.info("Rotation detection cancelled during completion.")
                    break
        
        logging.info(f"Rotation detection finished. Processed {processed_count}/{total_files} files.")
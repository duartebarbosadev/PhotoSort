import os
import concurrent.futures
import logging
from typing import Optional, List, Callable, Dict

from src.core.image_features.model_rotation_detector import ModelRotationDetector
from src.core.metadata_processor import MetadataProcessor
from src.core.image_pipeline import ImagePipeline
from src.core.caching.exif_cache import ExifCache

DEFAULT_NUM_WORKERS = min(os.cpu_count() or 4, 8)

class RotationDetector:
    """
    Detects image orientation using a pre-trained deep learning model.
    This class manages batch processing and considers EXIF orientation.
    """
    def __init__(self, image_pipeline: ImagePipeline, exif_cache: ExifCache):
        logging.info("Initializing RotationDetector...")
        self.image_pipeline = image_pipeline
        self.model_detector = ModelRotationDetector()
        self.exif_cache = exif_cache
        logging.info("RotationDetector initialized.")

    def _detect_rotation_task(
        self,
        image_path: str,
        result_callback: Optional[Callable[[str, int], None]],
        apply_auto_edits: bool
    ) -> None:
        """
        Worker task for detecting rotation for a single image.
        It compares the model's prediction with the current EXIF data
        to suggest the correct net rotation.
        """
        try:
            pil_image = self.image_pipeline.get_pil_image_for_processing(
                image_path,
                apply_auto_edits=apply_auto_edits,
                use_preloaded_preview_if_available=False,
                apply_exif_transpose=False
            )
            
            model_suggested_rotation = self.model_detector.predict_rotation_angle(
                image_path, image=pil_image, apply_auto_edits=apply_auto_edits
            )
            
            current_orientation = MetadataProcessor.get_orientation(image_path, self.exif_cache) or 1
            
            EXIF_TO_ANGLE_CW = {
                1: 0, 2: 0, 3: 180, 4: 180, 5: 270, 6: 90, 7: 90, 8: 270,
            }
            current_angle_cw = EXIF_TO_ANGLE_CW.get(current_orientation, 0)
            
            model_angle_cw = (model_suggested_rotation + 360) % 360
            
            net_rotation_cw = (model_angle_cw - current_angle_cw + 360) % 360
            
            final_suggested_rotation = -90 if net_rotation_cw == 270 else net_rotation_cw
            
            if result_callback:
                result_callback(image_path, final_suggested_rotation)

        except Exception as e:
            logging.error(f"Error processing rotation for {image_path}: {e}")
            if result_callback:
                result_callback(image_path, 0)

    def detect_rotation_in_batch(
        self,
        image_paths: List[str],
        result_callback: Optional[Callable[[str, int], None]] = None,
        progress_callback: Optional[Callable[[int, int, str], None]] = None,
        should_continue_callback: Optional[Callable[[], bool]] = None,
        num_workers: Optional[int] = None,
        **kwargs
    ) -> None:
        """
        Detects rotation suggestions for a batch of images in parallel.
        """
        total_files = len(image_paths)
        processed_count = 0
        apply_auto_edits = kwargs.get('apply_auto_edits', False)
        
        effective_num_workers = num_workers if num_workers is not None else DEFAULT_NUM_WORKERS
        logging.info(f"Starting model-based rotation detection for {total_files} files, workers: {effective_num_workers}")

        with concurrent.futures.ThreadPoolExecutor(max_workers=effective_num_workers) as executor:
            futures_map: Dict[concurrent.futures.Future, str] = {
                executor.submit(self._detect_rotation_task, path, result_callback, apply_auto_edits): path
                for path in image_paths
                if not (should_continue_callback and not should_continue_callback())
            }
            
            for future in concurrent.futures.as_completed(futures_map):
                path_for_future = futures_map[future]
                try:
                    future.result()
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
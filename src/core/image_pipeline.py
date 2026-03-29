import os
import time
import logging
import threading
from typing import Optional, List, Dict, Tuple, Callable
from PIL import Image, ImageDraw
import cv2

try:  # Optional; some minimal Pillow builds may omit ImageQt
    from PIL.ImageQt import ImageQt  # type: ignore
except (ImportError, ModuleNotFoundError):
    ImageQt = None  # type: ignore
from PyQt6.QtGui import QPixmap
import concurrent.futures

from .image_processing.raw_image_processor import RawImageProcessor, is_raw_extension
from .image_processing.standard_image_processor import (
    StandardImageProcessor,
    SUPPORTED_STANDARD_EXTENSIONS,
)
from .image_processing.image_orientation_handler import ImageOrientationHandler
from .caching.thumbnail_cache import ThumbnailCache
from .caching.preview_cache import PreviewCache
from .media_utils import is_video_extension

logger = logging.getLogger(__name__)
PREVIEW_GENERATION_LOG_INTERVAL = 250
_preview_generation_log_lock = threading.Lock()
_preview_generation_count = 0


def _record_preview_generation_log(duration: float, basename: str) -> None:
    global _preview_generation_count
    with _preview_generation_log_lock:
        _preview_generation_count += 1
        count = _preview_generation_count
    if count == 1 or count % PREVIEW_GENERATION_LOG_INTERVAL == 0:
        logger.debug(
            "Generated previews: %d (latest: %s in %.2fs)",
            count,
            basename,
            duration,
        )


# Default sizes and resolutions (can be made configurable or passed in)
THUMBNAIL_MAX_SIZE: Tuple[int, int] = (256, 256)
PRELOAD_MAX_RESOLUTION: Tuple[int, int] = (1920, 1200)
# DISPLAY_MAX_RESOLUTION might be different, e.g., based on UI element size


class ImagePipeline:
    """
    Orchestrates image processing, caching, and retrieval.
    Acts as a facade for image-related operations.
    """

    def __init__(
        self,
        thumbnail_cache_dir: Optional[str] = None,
        preview_cache_dir: Optional[str] = None,
    ):
        init_start_time = time.perf_counter()
        logger.info("Initializing ImagePipeline...")

        tc_start_time = time.perf_counter()
        self.thumbnail_cache = (
            ThumbnailCache(cache_dir=thumbnail_cache_dir)
            if thumbnail_cache_dir
            else ThumbnailCache()
        )
        logger.debug(
            f"ThumbnailCache instantiated in {time.perf_counter() - tc_start_time:.4f}s"
        )

        pc_start_time = time.perf_counter()
        self.preview_cache = (
            PreviewCache(cache_dir=preview_cache_dir)
            if preview_cache_dir
            else PreviewCache()
        )
        logger.debug(
            f"PreviewCache instantiated in {time.perf_counter() - pc_start_time:.4f}s"
        )

        self.image_orientation_handler = (
            ImageOrientationHandler()
        )  # Instantiate if it has non-static methods or state
        logger.debug("ImageOrientationHandler instantiated.")

        # For concurrent operations - use performance mode
        from core.app_settings import calculate_max_workers

        self._num_workers = calculate_max_workers(min_workers=4, max_workers=16)
        logger.info(
            f"ImagePipeline initialized in {time.perf_counter() - init_start_time:.4f}s (workers: {self._num_workers})"
        )

    def _get_pil_thumbnail(
        self,
        image_path: str,
        apply_orientation: bool = False,
    ) -> Optional[Image.Image]:
        """
        Internal method to get/generate a PIL thumbnail.
        Checks cache first, then generates and caches.
        Automatically applies auto-edits for RAW files.
        """
        normalized_path = os.path.normpath(image_path)
        ext = os.path.splitext(normalized_path)[1].lower()

        # Automatically determine if auto-edits should be applied based on file type
        apply_auto_edits = is_raw_extension(ext)

        # Add apply_orientation to the cache key to store oriented/unoriented versions separately
        cache_key = (normalized_path, apply_auto_edits, apply_orientation)

        cached_img = self.thumbnail_cache.get(cache_key)
        if cached_img:
            logger.debug(
                f"Thumbnail cache HIT for: {os.path.basename(normalized_path)} (Auto-Edits: {apply_auto_edits}, Orientation: {apply_orientation})"
            )
            return cached_img

        pil_img: Optional[Image.Image] = None

        if is_raw_extension(ext):
            pil_img = RawImageProcessor.process_raw_for_thumbnail(
                normalized_path, apply_auto_edits, THUMBNAIL_MAX_SIZE
            )
        elif is_video_extension(ext):
            pil_img = self._extract_video_thumbnail_with_overlay(normalized_path)
        elif ext in SUPPORTED_STANDARD_EXTENSIONS:
            # For standard images, we pass apply_orientation down, as the processor
            # is responsible for reading the EXIF data.
            pil_img = StandardImageProcessor.process_for_thumbnail(
                normalized_path, THUMBNAIL_MAX_SIZE, apply_orientation
            )
        else:
            logger.warning(
                f"Unsupported extension for thumbnail: {ext} for '{os.path.basename(normalized_path)}'"
            )
            return None

        if pil_img:
            # If the processor for standard images did not handle orientation,
            # or for RAW images where it's applied post-processing, handle it here.
            # The StandardImageProcessor now handles it, so this is mainly for RAW
            # or as a fallback.
            if (
                apply_orientation and ext not in SUPPORTED_STANDARD_EXTENSIONS
            ):  # Assume standard processor handled it
                pil_img = self.image_orientation_handler.exif_transpose(pil_img)

            self.thumbnail_cache.set(cache_key, pil_img)
        return pil_img

    def _extract_video_thumbnail_with_overlay(
        self,
        video_path: str,
    ) -> Optional[Image.Image]:
        """Extract first decodable frame and apply a play badge overlay."""
        capture = cv2.VideoCapture(video_path)
        if not capture.isOpened():
            logger.debug(
                "Video thumbnail extraction failed to open %s",
                os.path.basename(video_path),
            )
            return None

        try:
            ok, frame = capture.read()
        except Exception as exc:
            logger.debug(
                "Video thumbnail extraction read error for %s: %s",
                os.path.basename(video_path),
                exc,
            )
            return None
        finally:
            capture.release()

        if not ok or frame is None:
            logger.debug(
                "Video thumbnail extraction found no frame for %s",
                os.path.basename(video_path),
            )
            return None

        try:
            frame_rgb = cv2.cvtColor(frame, cv2.COLOR_BGR2RGB)
            pil_img = Image.fromarray(frame_rgb)
            pil_img.thumbnail(THUMBNAIL_MAX_SIZE, Image.Resampling.LANCZOS)
            return self._add_video_play_overlay(pil_img)
        except Exception as exc:
            logger.debug(
                "Video thumbnail conversion failed for %s: %s",
                os.path.basename(video_path),
                exc,
            )
            return None

    def _add_video_play_overlay(self, pil_img: Image.Image) -> Image.Image:
        """Draw a compact center play badge so video thumbnails are visually distinct."""
        base = pil_img.convert("RGBA")
        draw = ImageDraw.Draw(base, "RGBA")

        width, height = base.size
        badge_radius = max(10, min(width, height) // 7)
        center_x = width // 2
        center_y = height // 2

        draw.ellipse(
            (
                center_x - badge_radius,
                center_y - badge_radius,
                center_x + badge_radius,
                center_y + badge_radius,
            ),
            fill=(0, 0, 0, 145),
            outline=(255, 255, 255, 180),
            width=max(1, badge_radius // 8),
        )

        triangle_half_height = max(4, badge_radius // 2)
        triangle_half_width = max(4, badge_radius // 3)
        triangle_center_x = center_x + max(1, badge_radius // 8)
        draw.polygon(
            [
                (
                    triangle_center_x - triangle_half_width,
                    center_y - triangle_half_height,
                ),
                (
                    triangle_center_x - triangle_half_width,
                    center_y + triangle_half_height,
                ),
                (triangle_center_x + triangle_half_width, center_y),
            ],
            fill=(255, 255, 255, 230),
        )

        return base.convert("RGB")

    def get_thumbnail_qpixmap(
        self,
        image_path: str,
        apply_orientation: bool = False,
    ) -> Optional[QPixmap]:
        """
        Gets a QPixmap thumbnail for the given image path.
        Automatically applies auto-edits for RAW files.

        Args:
            image_path: The path to the image.
            apply_orientation: Whether to apply EXIF orientation to the thumbnail.
        """
        if not os.path.isfile(image_path):
            logger.error(f"File does not exist: {image_path}")
            return None

        pil_img = self._get_pil_thumbnail(image_path, apply_orientation)
        if pil_img:
            try:
                return QPixmap.fromImage(ImageQt(pil_img))
            except Exception:
                logger.error(
                    f"Error converting PIL thumbnail to QPixmap for {os.path.basename(image_path)}",
                    exc_info=True,
                )
        return None

    def _generate_pil_preview_for_display(
        self,
        image_path: str,
        display_max_size: Optional[Tuple[int, int]],
        force_default_brightness: bool = False,
    ) -> Optional[Image.Image]:
        """
        Generates a PIL image sized for display, without using preload cache.
        This is the fallback if no suitable cached version (display or preloaded) is found.
        Automatically applies auto-edits for RAW files.
        """
        normalized_path = os.path.normpath(image_path)
        pil_img: Optional[Image.Image] = None
        ext = os.path.splitext(normalized_path)[1].lower()

        # Automatically determine if auto-edits should be applied based on file type
        apply_auto_edits = is_raw_extension(ext)

        # Determine resolution for on-demand generation
        # If display_max_size is None, it means full available resolution (up to a reasonable limit)
        # For now, let's assume PRELOAD_MAX_RESOLUTION is also a good upper bound for on-demand full previews.
        # A more sophisticated system might have different limits.
        target_resolution = (
            display_max_size if display_max_size else PRELOAD_MAX_RESOLUTION
        )

        if is_raw_extension(ext):
            # process_raw_for_preview uses PRELOAD_MAX_RESOLUTION by default if not overridden
            # We need a version or parameters for specific target_resolution
            # For now, let's assume load_raw_as_pil can be used carefully.
            # RawImageProcessor.process_raw_for_preview can be adapted or a new method created.
            # Let's refine this: load_raw_as_pil with half_size=False for better quality for display generation.
            temp_pil_img = RawImageProcessor.load_raw_as_pil(
                normalized_path,
                target_mode="RGBA",
                apply_auto_edits=apply_auto_edits,
                half_size=False,  # Use full processing for better quality display master
                force_default_brightness=force_default_brightness,
            )
            if temp_pil_img:
                temp_pil_img.thumbnail(target_resolution, Image.Resampling.LANCZOS)
                pil_img = temp_pil_img

        elif ext in SUPPORTED_STANDARD_EXTENSIONS:
            # StandardImageProcessor.process_for_preview uses PRELOAD_MAX_RESOLUTION
            # StandardImageProcessor.load_as_pil can be used for full, then thumbnail
            temp_pil_img = StandardImageProcessor.load_as_pil(
                normalized_path, target_mode="RGBA"
            )
            if temp_pil_img:
                temp_pil_img.thumbnail(target_resolution, Image.Resampling.LANCZOS)
                pil_img = temp_pil_img
        else:
            logger.warning(
                f"Unsupported extension for display preview: {ext} for '{os.path.basename(normalized_path)}'"
            )
            return None

        # Orientation should be handled by the processors.
        return pil_img

    def get_preview_image(
        self,
        image_path: str,
        display_max_size: Optional[Tuple[int, int]] = None,
        force_regenerate: bool = False,
        force_default_brightness: bool = False,
    ) -> Optional[Image.Image]:
        """Return a PIL image suitable for analysis/display, leveraging preview cache."""
        logger.debug(f"Obtaining preview PIL image called for: {image_path}")
        normalized_path = os.path.normpath(image_path)
        if not os.path.isfile(normalized_path):
            logger.error(f"File does not exist: {normalized_path}")
            return None

        ext = os.path.splitext(normalized_path)[1].lower()
        apply_auto_edits = is_raw_extension(ext)
        key_display_size = (
            display_max_size if display_max_size is not None else PRELOAD_MAX_RESOLUTION
        )
        display_cache_key = (normalized_path, key_display_size, apply_auto_edits)

        if not force_regenerate:
            cached_display_pil = self.preview_cache.get(display_cache_key)
            if cached_display_pil:
                logger.debug(
                    f"Display cache HIT (PIL): {os.path.basename(normalized_path)} (Size: {key_display_size})"
                )
                result_image = cached_display_pil.copy()
                if result_image.mode != "RGB":
                    result_image = result_image.convert("RGB")
                result_image.info.setdefault("source_path", normalized_path)
                result_image.info.setdefault("region", "full")
                return result_image

        preload_cache_key = (normalized_path, PRELOAD_MAX_RESOLUTION, apply_auto_edits)
        cached_high_res_pil = self.preview_cache.get(preload_cache_key)
        if cached_high_res_pil:
            logger.debug(
                f"Preview cache HIT (High-Res PIL): {os.path.basename(normalized_path)}. Resizing for display."
            )
            display_pil_img = cached_high_res_pil.copy()
            if display_max_size:
                display_pil_img.thumbnail(display_max_size, Image.Resampling.LANCZOS)
            self.preview_cache.set(display_cache_key, display_pil_img.copy())
            if display_pil_img.mode != "RGB":
                display_pil_img = display_pil_img.convert("RGB")
            display_pil_img.info.setdefault("source_path", normalized_path)
            display_pil_img.info.setdefault("region", "full")
            return display_pil_img

        logger.debug(
            f"Preview cache MISS for {os.path.basename(normalized_path)}. Generating PIL preview on-demand..."
        )
        generated_display_pil = self._generate_pil_preview_for_display(
            normalized_path,
            display_max_size,
            force_default_brightness,
        )
        if generated_display_pil:
            self.preview_cache.set(display_cache_key, generated_display_pil.copy())
            if generated_display_pil.mode != "RGB":
                generated_display_pil = generated_display_pil.convert("RGB")
            generated_display_pil.info.setdefault("source_path", normalized_path)
            generated_display_pil.info.setdefault("region", "full")
            return generated_display_pil

        logger.error(
            f"Failed to generate or retrieve preview PIL for {os.path.basename(normalized_path)}",
            exc_info=True,
        )
        return None

    def get_preview_qpixmap(
        self,
        image_path: str,
        display_max_size: Optional[
            Tuple[int, int]
        ],  # Max size for the QPixmap to be displayed
        force_regenerate: bool = False,
        force_default_brightness: bool = False,
    ) -> Optional[QPixmap]:
        """
        Gets a QPixmap preview for the image path, scaled to display_max_size.
        Automatically applies auto-edits for RAW files.
        1. Checks cache for display-sized PIL image.
        2. Checks cache for high-resolution preloaded PIL image, then resizes.
        3. Generates fresh PIL image for display size, caches it, then converts.
        """
        logger.debug(f"Obtaining preview QPixmap called for: {image_path}")
        normalized_path = os.path.normpath(image_path)
        if not os.path.isfile(normalized_path):
            logger.error(f"File does not exist: {normalized_path}")
            return None

        # Automatically determine if auto-edits should be applied based on file type
        ext = os.path.splitext(normalized_path)[1].lower()
        apply_auto_edits = is_raw_extension(ext)

        # Cache key for the final display-sized PIL image
        # Ensure display_max_size is a tuple for the cache key, even if None was passed
        key_display_size = (
            display_max_size if display_max_size is not None else PRELOAD_MAX_RESOLUTION
        )
        display_cache_key = (normalized_path, key_display_size, apply_auto_edits)

        # 1. Check if display-sized version is already cached
        if not force_regenerate:
            cached_display_pil = self.preview_cache.get(display_cache_key)
            if cached_display_pil:
                logger.debug(
                    f"Display cache HIT: {os.path.basename(normalized_path)} (Size: {key_display_size})"
                )
                return QPixmap.fromImage(ImageQt(cached_display_pil))
            else:
                logger.debug(
                    f"Display cache MISS: {os.path.basename(normalized_path)} (Size: {key_display_size})"
                )

        # 2. Check if a high-resolution PRELOADED version is cached
        # Key for preloaded high-res version (uses PRELOAD_MAX_RESOLUTION)
        preload_cache_key = (normalized_path, PRELOAD_MAX_RESOLUTION, apply_auto_edits)
        cached_high_res_pil = self.preview_cache.get(preload_cache_key)

        if cached_high_res_pil:
            logger.debug(
                f"Preview cache HIT (High-Res): {os.path.basename(normalized_path)}. Resizing for display."
            )
            display_pil_img = cached_high_res_pil.copy()
            if display_max_size:
                display_pil_img.thumbnail(display_max_size, Image.Resampling.LANCZOS)

            self.preview_cache.set(
                display_cache_key, display_pil_img
            )  # Cache the resized version
            return QPixmap.fromImage(ImageQt(display_pil_img))

        # 3. Generate fresh for display size, then cache
        logger.debug(
            f"Preview cache MISS for {os.path.basename(normalized_path)}. Generating on-demand..."
        )
        generated_display_pil = self._generate_pil_preview_for_display(
            normalized_path,
            display_max_size,
            force_default_brightness,
        )
        if generated_display_pil:
            self.preview_cache.set(display_cache_key, generated_display_pil)
            return QPixmap.fromImage(ImageQt(generated_display_pil))

        logger.error(
            f"Failed to generate or retrieve preview for {os.path.basename(normalized_path)}",
            exc_info=True,
        )
        return None

    def _ensure_thumbnail_generated_and_cached(self, image_path: str) -> None:
        """Worker function for preload_thumbnails."""
        self._get_pil_thumbnail(
            image_path
        )  # This handles generation and caching with automatic RAW detection

    def preload_thumbnails(
        self,
        image_paths: List[str],
        progress_callback: Optional[Callable[[int, int], None]] = None,
        should_continue_callback: Optional[Callable[[], bool]] = None,
    ) -> None:
        """Preloads thumbnails for a list of media paths in parallel."""
        total_files = len(image_paths)
        processed_count = 0

        logger.info(
            f"Preloading thumbnails for {total_files} files (workers: {self._num_workers})..."
        )

        with concurrent.futures.ThreadPoolExecutor(
            max_workers=self._num_workers
        ) as executor:
            futures_map: Dict[concurrent.futures.Future, str] = {}
            for image_path in image_paths:
                if should_continue_callback and not should_continue_callback():
                    logger.info(
                        "Thumbnail preload cancelled by request. Halting new tasks."
                    )
                    break
                future = executor.submit(
                    self._ensure_thumbnail_generated_and_cached,
                    image_path,
                )
                futures_map[future] = image_path

            for future in concurrent.futures.as_completed(futures_map):
                _ = futures_map[future]  # path, if needed for logging
                try:
                    future.result()  # Check for exceptions from worker
                except Exception:
                    logger.error(
                        "Error during thumbnail preloading task", exc_info=True
                    )

                processed_count += 1
                if progress_callback:
                    progress_callback(processed_count, total_files)
                if should_continue_callback and not should_continue_callback():
                    # Cancel remaining futures if any
                    for f_cancel in futures_map:
                        if not f_cancel.done():
                            f_cancel.cancel()
                    logger.info("Thumbnail preload cancelled during processing.")
                    break
        logger.info(
            f"Thumbnail preloading finished. Processed {processed_count}/{total_files}."
        )

    def _ensure_preview_generated_and_cached(self, image_path: str) -> bool:
        """
        Worker function for preload_previews. Generates and caches one preview at PRELOAD_MAX_RESOLUTION.
        Automatically applies auto-edits for RAW files.
        Returns True if successful or already cached, False on error.
        """
        normalized_path = os.path.normpath(image_path)

        # Automatically determine if auto-edits should be applied based on file type
        ext = os.path.splitext(normalized_path)[1].lower()
        apply_auto_edits = is_raw_extension(ext)

        # Cache key for preloaded high-res version
        preload_cache_key = (normalized_path, PRELOAD_MAX_RESOLUTION, apply_auto_edits)

        if preload_cache_key in self.preview_cache:
            return True

        pil_img: Optional[Image.Image] = None
        ext = os.path.splitext(normalized_path)[1].lower()

        start_time = time.time()
        if is_raw_extension(ext):
            pil_img = RawImageProcessor.process_raw_for_preview(
                normalized_path, apply_auto_edits, PRELOAD_MAX_RESOLUTION
            )
        elif ext in SUPPORTED_STANDARD_EXTENSIONS:
            pil_img = StandardImageProcessor.process_for_preview(
                normalized_path, PRELOAD_MAX_RESOLUTION
            )
        else:
            logger.warning(
                f"Unsupported extension for preview preload: {ext} for '{os.path.basename(normalized_path)}'"
            )
            return False  # Or log error

        if pil_img:
            # pil_img = self.image_orientation_handler.exif_transpose(pil_img) # Processors should handle this.
            self.preview_cache.set(preload_cache_key, pil_img)
            duration = time.time() - start_time
            _record_preview_generation_log(duration, os.path.basename(normalized_path))
            return True
        else:
            logger.error(
                f"Failed to generate preview for {os.path.basename(normalized_path)}",
                exc_info=True,
            )
            return False

    def preload_previews(
        self,
        image_paths: List[str],
        progress_callback: Optional[Callable[[int, int], None]] = None,
        should_continue_callback: Optional[Callable[[], bool]] = None,
    ) -> None:
        """Preloads preview PIL images (at PRELOAD_MAX_RESOLUTION) in parallel.
        Automatically applies auto-edits for RAW files."""
        total_files = len(image_paths)
        processed_count = 0

        logger.info(
            f"Preloading previews for {total_files} files (workers: {self._num_workers})..."
        )

        with concurrent.futures.ThreadPoolExecutor(
            max_workers=self._num_workers
        ) as executor:
            futures_map: Dict[concurrent.futures.Future, str] = {}
            for image_path in image_paths:
                if should_continue_callback and not should_continue_callback():
                    logger.info(
                        "Preview preload cancelled by request. Halting new tasks."
                    )
                    break
                future = executor.submit(
                    self._ensure_preview_generated_and_cached,
                    image_path,
                )
                futures_map[future] = image_path

            for future in concurrent.futures.as_completed(futures_map):
                _ = futures_map[future]  # path
                try:
                    future.result()  # Check for exceptions
                except Exception:
                    logger.error("Error during preview preloading task", exc_info=True)

                processed_count += 1
                if progress_callback:
                    progress_callback(processed_count, total_files)
                if should_continue_callback and not should_continue_callback():
                    for f_cancel in futures_map:
                        if not f_cancel.done():
                            f_cancel.cancel()
                    logger.info("Preview preload cancelled during processing.")
                    break
        logger.info(
            f"Preview preloading finished. Processed {processed_count}/{total_files}."
        )

    def get_pil_image_for_processing(
        self,
        image_path: str,
        target_mode: str = "RGB",
        use_preloaded_preview_if_available: bool = True,
        apply_exif_transpose: bool = True,
    ) -> Optional[Image.Image]:
        """
        Gets a PIL image for general processing (e.g., similarity engine, blur detection).
        Tries to use a cached high-resolution preview if available and `use_preloaded_preview_if_available` is True.
        Otherwise, loads the image directly (potentially slower for RAWs).
        Automatically applies auto-edits for RAW files.
        """
        normalized_path = os.path.normpath(image_path)

        # Automatically determine if auto-edits should be applied based on file type
        ext = os.path.splitext(normalized_path)[1].lower()
        apply_auto_edits = is_raw_extension(ext)

        if use_preloaded_preview_if_available:
            # Check for preloaded high-res version
            preload_key = (normalized_path, PRELOAD_MAX_RESOLUTION, apply_auto_edits)
            cached_preview = self.preview_cache.get(preload_key)
            if cached_preview:
                logger.debug(
                    f"Using preloaded preview for processing: {os.path.basename(normalized_path)}"
                )
                return (
                    cached_preview.convert(target_mode)
                    if cached_preview.mode != target_mode
                    else cached_preview
                )

            # Log the cache miss only when we intended to use the cache
            logger.debug(
                f"No preloaded preview found. Loading directly: {os.path.basename(normalized_path)}"
            )

        # Fallback to loading directly
        pil_img: Optional[Image.Image] = None
        ext = os.path.splitext(normalized_path)[1].lower()

        if is_raw_extension(ext):
            # For processing, usually want good quality, so half_size=False unless performance is critical and tested.
            pil_img = RawImageProcessor.load_raw_as_pil(
                normalized_path,
                target_mode=target_mode,
                apply_auto_edits=apply_auto_edits,
                half_size=False,  # Or make this a parameter if varying quality is needed
            )
        elif ext in SUPPORTED_STANDARD_EXTENSIONS:
            pil_img = StandardImageProcessor.load_as_pil(
                normalized_path,
                target_mode=target_mode,
                apply_exif_transpose=apply_exif_transpose,
            )
        else:
            try:  # Last resort for unknown types
                with Image.open(normalized_path) as img:
                    if apply_exif_transpose:
                        img: Image.Image = (
                            self.image_orientation_handler.exif_transpose(img)
                        )
                    # Copy to preserve image data after context exit
                    pil_img = img.convert(target_mode).copy()
            except Exception:
                logger.warning(
                    f"Unsupported extension for processing: {ext} for '{os.path.basename(normalized_path)}'"
                )

        return pil_img

    def clear_all_image_caches(self):
        """Clears both thumbnail and preview caches."""
        logger.warning("Clearing all image caches (thumbnails and previews)...")
        self.thumbnail_cache.clear()
        self.preview_cache.clear()
        logger.info("All image caches have been cleared.")

    def reinitialize_preview_cache_from_settings(self):
        """Reinitializes the preview cache using current application settings."""
        self.preview_cache.reinitialize_from_settings()

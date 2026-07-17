import os
import time
import logging
import threading
from collections.abc import Callable
from PIL import Image, ImageDraw

try:  # Optional; some minimal Pillow builds may omit ImageQt
    from PIL.ImageQt import ImageQt  # type: ignore
except ImportError, ModuleNotFoundError:
    ImageQt = None  # type: ignore
from PyQt6.QtGui import QPixmap
import concurrent.futures
from collections import OrderedDict

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
THUMBNAIL_MAX_SIZE: tuple[int, int] = (256, 256)
PRELOAD_MAX_RESOLUTION: tuple[int, int] = (1920, 1200)
ANALYSIS_CACHE_RESOLUTION: tuple[int, int] = (1024, 1024)
CACHE_SCHEMA_VERSION = 2
# DISPLAY_MAX_RESOLUTION might be different, e.g., based on UI element size


class ImagePipeline:
    """
    Orchestrates image processing, caching, and retrieval.
    Acts as a facade for image-related operations.
    """

    def __init__(
        self,
        thumbnail_cache_dir: str | None = None,
        preview_cache_dir: str | None = None,
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

        from core.app_settings import (
            IMAGE_MEMORY_CACHE_SIZE_BYTES,
            calculate_high_memory_decode_workers,
            calculate_thumbnail_workers,
        )

        self._memory_cache: OrderedDict[tuple, Image.Image] = OrderedDict()
        self._memory_cache_bytes = 0
        self._memory_cache_limit_bytes = IMAGE_MEMORY_CACHE_SIZE_BYTES
        self._memory_cache_lock = threading.RLock()
        self._generation_locks = [threading.Lock() for _ in range(64)]
        self._high_memory_decode_workers = calculate_high_memory_decode_workers()
        self._high_memory_decode_gate = threading.BoundedSemaphore(
            self._high_memory_decode_workers
        )

        # Image decoding is memory-heavy. More threads reduce responsiveness and can
        # multiply full-resolution buffers without improving useful throughput.
        self._num_workers = calculate_thumbnail_workers()
        logger.info(
            "ImagePipeline initialized in %.4fs (thumbnail workers: %d, "
            "full-decode workers: %d)",
            time.perf_counter() - init_start_time,
            self._num_workers,
            self._high_memory_decode_workers,
        )

    @property
    def thumbnail_worker_count(self) -> int:
        """Return the current shared thumbnail concurrency budget."""
        return self._num_workers

    @staticmethod
    def _file_fingerprint(image_path: str) -> tuple[int, int]:
        try:
            stat_result = os.stat(image_path)
            return stat_result.st_size, stat_result.st_mtime_ns
        except OSError:
            return 0, 0

    def thumbnail_cache_key(
        self,
        image_path: str,
        apply_orientation: bool = False,
        *,
        file_size: int | None = None,
        mtime_ns: int | None = None,
    ) -> tuple:
        normalized_path = os.path.normpath(image_path)
        if file_size is None or mtime_ns is None:
            file_size, mtime_ns = self._file_fingerprint(normalized_path)
        apply_auto_edits = is_raw_extension(
            os.path.splitext(normalized_path)[1].lower()
        )
        return (
            normalized_path,
            "thumbnail",
            CACHE_SCHEMA_VERSION,
            int(file_size),
            int(mtime_ns),
            apply_auto_edits,
            apply_orientation,
        )

    def preview_cache_key(self, image_path: str, resolution: tuple[int, int]) -> tuple:
        normalized_path = os.path.normpath(image_path)
        file_size, mtime_ns = self._file_fingerprint(normalized_path)
        apply_auto_edits = is_raw_extension(
            os.path.splitext(normalized_path)[1].lower()
        )
        return (
            normalized_path,
            "preview",
            CACHE_SCHEMA_VERSION,
            file_size,
            mtime_ns,
            tuple(resolution),
            apply_auto_edits,
        )

    def analysis_cache_key(
        self,
        image_path: str,
        target_size: tuple[int, int],
    ) -> tuple:
        """Return a fingerprinted key for neutral, model-sized image inputs."""
        normalized_path = os.path.normpath(image_path)
        file_size, mtime_ns = self._file_fingerprint(normalized_path)
        return (
            normalized_path,
            "analysis",
            CACHE_SCHEMA_VERSION,
            int(file_size),
            int(mtime_ns),
            tuple(target_size),
        )

    @staticmethod
    def _image_memory_size(image: Image.Image) -> int:
        return image.width * image.height * max(1, len(image.getbands()))

    @staticmethod
    def _qpixmap_from_pil(image: Image.Image) -> QPixmap:
        """Create a pixmap that owns its storage independently of Pillow."""
        qt_image = ImageQt(image).copy()
        return QPixmap.fromImage(qt_image).copy()

    def _memory_get(self, key: tuple) -> Image.Image | None:
        with self._memory_cache_lock:
            image = self._memory_cache.pop(key, None)
            if image is None:
                return None
            self._memory_cache[key] = image
            return image.copy()

    def _memory_set(self, key: tuple, image: Image.Image) -> None:
        stored = image.copy()
        stored_size = self._image_memory_size(stored)
        if stored_size > self._memory_cache_limit_bytes:
            return
        with self._memory_cache_lock:
            previous = self._memory_cache.pop(key, None)
            if previous is not None:
                self._memory_cache_bytes -= self._image_memory_size(previous)
            self._memory_cache[key] = stored
            self._memory_cache_bytes += stored_size
            while self._memory_cache_bytes > self._memory_cache_limit_bytes:
                _, evicted = self._memory_cache.popitem(last=False)
                self._memory_cache_bytes -= self._image_memory_size(evicted)

    def _cache_get(self, cache: object, key: tuple) -> Image.Image | None:
        memory_image = self._memory_get(key)
        if memory_image is not None:
            return memory_image
        image = cache.get(key)
        if image is not None:
            self._memory_set(key, image)
        return image

    def _cache_set(self, cache: object, key: tuple, image: Image.Image) -> None:
        self._memory_set(key, image)
        cache.set(key, image)

    def _generation_lock(self, key: tuple) -> threading.Lock:
        return self._generation_locks[hash(key) % len(self._generation_locks)]

    def _get_pil_thumbnail(
        self,
        image_path: str,
        apply_orientation: bool = False,
        *,
        promote_to_memory: bool = True,
    ) -> Image.Image | None:
        """
        Internal method to get/generate a PIL thumbnail.
        Checks cache first, then generates and caches.
        Automatically applies auto-edits for RAW files.
        """
        normalized_path = os.path.normpath(image_path)
        ext = os.path.splitext(normalized_path)[1].lower()

        # Automatically determine if auto-edits should be applied based on file type
        apply_auto_edits = is_raw_extension(ext)

        cache_key = self.thumbnail_cache_key(normalized_path, apply_orientation)

        cached_img = self._memory_get(cache_key)
        if cached_img is None and promote_to_memory:
            cached_img = self._cache_get(self.thumbnail_cache, cache_key)
        # If cache hit, return immediately
        if cached_img is not None:
            return cached_img

        with self._generation_lock(cache_key):
            cached_img = self._memory_get(cache_key)
            if cached_img is None and promote_to_memory:
                cached_img = self._cache_get(self.thumbnail_cache, cache_key)
            if cached_img is not None:
                return cached_img

            pil_img: Image.Image | None = None
            raw_format = is_raw_extension(ext)
            high_memory_format = ext in {".heic", ".heif"}
            decode_gate = self._high_memory_decode_gate if high_memory_format else None
            if decode_gate:
                decode_gate.acquire()
            try:
                if raw_format:
                    pil_img = RawImageProcessor.process_raw_for_thumbnail(
                        normalized_path,
                        apply_auto_edits,
                        THUMBNAIL_MAX_SIZE,
                        full_decode_gate=self._high_memory_decode_gate,
                    )
                elif is_video_extension(ext):
                    pil_img = self._extract_video_thumbnail_with_overlay(
                        normalized_path
                    )
                elif ext in SUPPORTED_STANDARD_EXTENSIONS:
                    pil_img = StandardImageProcessor.process_for_thumbnail(
                        normalized_path, THUMBNAIL_MAX_SIZE, apply_orientation
                    )
                else:
                    logger.warning(
                        f"Unsupported extension for thumbnail: {ext} for '{os.path.basename(normalized_path)}'"
                    )
                    return None
            finally:
                if decode_gate:
                    decode_gate.release()

            if pil_img:
                if apply_orientation and ext not in SUPPORTED_STANDARD_EXTENSIONS:
                    pil_img = self.image_orientation_handler.exif_transpose(pil_img)
                if promote_to_memory:
                    self._cache_set(self.thumbnail_cache, cache_key, pil_img)
                else:
                    self.thumbnail_cache.set(cache_key, pil_img)
            return pil_img

    def ensure_thumbnail_cached(
        self,
        image_path: str,
        *,
        promote_to_memory: bool = True,
    ) -> bool:
        """Ensure one thumbnail exists without requiring a UI-thread cache read.

        ``promote_to_memory=False`` is intended for low-priority folder warming:
        it checks and fills the disk cache without displacing hot viewport images.
        """
        if not promote_to_memory:
            normalized_path = os.path.normpath(image_path)
            cache_key = self.thumbnail_cache_key(normalized_path, False)
            if self._memory_get(cache_key) is not None:
                return True
            # Membership avoids decoding a disk hit into the shared memory LRU.
            if cache_key in self.thumbnail_cache:
                return True
        return (
            self._get_pil_thumbnail(
                image_path,
                promote_to_memory=promote_to_memory,
            )
            is not None
        )

    def get_cached_thumbnail_qpixmap(
        self,
        image_path: str,
        apply_orientation: bool = False,
        *,
        file_size: int | None = None,
        mtime_ns: int | None = None,
        memory_only: bool = False,
    ) -> QPixmap | None:
        """
        Returns a thumbnail QPixmap only if it is already cached.
        Never generates a new thumbnail on cache miss.
        ``memory_only`` also prevents disk-cache reads for latency-sensitive UI calls.
        """
        normalized_path = os.path.normpath(image_path)
        if not os.path.isfile(normalized_path):
            logger.error(f"File does not exist: {normalized_path}")
            return None

        cache_key = self.thumbnail_cache_key(
            normalized_path,
            apply_orientation,
            file_size=file_size,
            mtime_ns=mtime_ns,
        )
        cached_img = (
            self._memory_get(cache_key)
            if memory_only
            else self._cache_get(self.thumbnail_cache, cache_key)
        )
        if cached_img is None and apply_orientation:
            # Folder warming intentionally creates one canonical, un-oriented
            # thumbnail.  Review surfaces still need display orientation, so
            # derive that tiny variant from the shared cached image instead of
            # decoding the source file again.
            source_key = self.thumbnail_cache_key(
                normalized_path,
                False,
                file_size=file_size,
                mtime_ns=mtime_ns,
            )
            source_img = (
                self._memory_get(source_key)
                if memory_only
                else self._cache_get(self.thumbnail_cache, source_key)
            )
            if source_img is not None:
                cached_img = self.image_orientation_handler.exif_transpose(source_img)
                self._memory_set(cache_key, cached_img)
        if cached_img is None:
            return None

        try:
            return self._qpixmap_from_pil(cached_img)
        except Exception:
            logger.error(
                "Error converting cached PIL thumbnail to QPixmap for %s",
                os.path.basename(normalized_path),
                exc_info=True,
            )
            return None

    def _extract_video_thumbnail_with_overlay(
        self,
        video_path: str,
    ) -> Image.Image | None:
        """Extract first decodable frame and apply a play badge overlay."""
        import cv2

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
    ) -> QPixmap | None:
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
                return self._qpixmap_from_pil(pil_img)
            except Exception:
                logger.error(
                    f"Error converting PIL thumbnail to QPixmap for {os.path.basename(image_path)}",
                    exc_info=True,
                )
        return None

    def _generate_pil_preview_for_display(
        self,
        image_path: str,
        display_max_size: tuple[int, int] | None,
        force_default_brightness: bool = False,
    ) -> Image.Image | None:
        """
        Generates a PIL image sized for display, without using preload cache.
        This is the authoritative decode path when no suitable cached image exists.
        Automatically applies auto-edits for RAW files.
        """
        normalized_path = os.path.normpath(image_path)
        pil_img: Image.Image | None = None
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
            # Use the same bounded preview path as background prefetch. It
            # prefers the camera's embedded JPEG and falls back to half-size
            # demosaicing, avoiding a full-resolution RAW decode for display.
            pil_img = RawImageProcessor.process_raw_for_preview(
                normalized_path,
                apply_auto_edits,
                target_resolution,
                force_default_brightness=force_default_brightness,
            )

        elif ext in SUPPORTED_STANDARD_EXTENSIONS:
            pil_img = StandardImageProcessor.process_for_preview(
                normalized_path,
                target_resolution,
            )
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
        display_max_size: tuple[int, int] | None = None,
        force_regenerate: bool = False,
        force_default_brightness: bool = False,
    ) -> Image.Image | None:
        """Return a PIL image suitable for analysis/display, leveraging preview cache."""
        normalized_path = os.path.normpath(image_path)
        if not os.path.isfile(normalized_path):
            logger.error(f"File does not exist: {normalized_path}")
            return None

        key_display_size = (
            display_max_size if display_max_size is not None else PRELOAD_MAX_RESOLUTION
        )
        display_cache_key = self.preview_cache_key(normalized_path, key_display_size)

        if not force_regenerate:
            cached_display_pil = self._cache_get(self.preview_cache, display_cache_key)
            if cached_display_pil:
                result_image = cached_display_pil.copy()
                if result_image.mode != "RGB":
                    result_image = result_image.convert("RGB")
                result_image.info.setdefault("source_path", normalized_path)
                result_image.info.setdefault("region", "full")
                return result_image

        preload_cache_key = self.preview_cache_key(
            normalized_path, PRELOAD_MAX_RESOLUTION
        )
        cached_high_res_pil = self._cache_get(self.preview_cache, preload_cache_key)
        if cached_high_res_pil:
            logger.debug(
                f"Preview cache HIT (High-Res PIL): {os.path.basename(normalized_path)}. Resizing for display."
            )
            display_pil_img = cached_high_res_pil.copy()
            if display_max_size:
                display_pil_img.thumbnail(display_max_size, Image.Resampling.LANCZOS)
            self._cache_set(
                self.preview_cache, display_cache_key, display_pil_img.copy()
            )
            if display_pil_img.mode != "RGB":
                display_pil_img = display_pil_img.convert("RGB")
            display_pil_img.info.setdefault("source_path", normalized_path)
            display_pil_img.info.setdefault("region", "full")
            return display_pil_img

        logger.debug(
            f"Preview cache MISS for {os.path.basename(normalized_path)}. Generating PIL preview on-demand..."
        )
        with self._generation_lock(display_cache_key):
            generated_display_pil = None
            if not force_regenerate:
                generated_display_pil = self._cache_get(
                    self.preview_cache, display_cache_key
                )
            if generated_display_pil is None:
                generated_display_pil = self._generate_pil_preview_for_display(
                    normalized_path,
                    display_max_size,
                    force_default_brightness,
                )
        if generated_display_pil:
            self._cache_set(
                self.preview_cache, display_cache_key, generated_display_pil.copy()
            )
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

    def get_cached_preview_qpixmap(
        self,
        image_path: str,
        display_max_size: tuple[int, int] | None = None,
        *,
        memory_only: bool = False,
    ) -> QPixmap | None:
        """
        Returns a preview QPixmap only if a suitable preview already exists in cache.
        Never generates a new preview on cache miss.

        Brightness is baked in when previews are generated and stored in the cache.
        ``memory_only`` also prevents disk-cache reads for latency-sensitive UI calls.
        """
        normalized_path = os.path.normpath(image_path)
        if not os.path.isfile(normalized_path):
            logger.error(f"File does not exist: {normalized_path}")
            return None

        key_display_size = (
            display_max_size if display_max_size is not None else PRELOAD_MAX_RESOLUTION
        )
        display_cache_key = self.preview_cache_key(normalized_path, key_display_size)

        cached_display_pil = (
            self._memory_get(display_cache_key)
            if memory_only
            else self._cache_get(self.preview_cache, display_cache_key)
        )
        if cached_display_pil is not None:
            try:
                return self._qpixmap_from_pil(cached_display_pil)
            except Exception:
                logger.error(
                    "Error converting cached display preview to QPixmap for %s",
                    os.path.basename(normalized_path),
                    exc_info=True,
                )
                return None

        preload_cache_key = self.preview_cache_key(
            normalized_path, PRELOAD_MAX_RESOLUTION
        )
        cached_high_res_pil = (
            self._memory_get(preload_cache_key)
            if memory_only
            else self._cache_get(self.preview_cache, preload_cache_key)
        )
        if cached_high_res_pil is None:
            return None

        display_pil_img = cached_high_res_pil.copy()
        if display_max_size:
            display_pil_img.thumbnail(display_max_size, Image.Resampling.LANCZOS)

        if memory_only:
            self._memory_set(display_cache_key, display_pil_img)
        else:
            self._cache_set(
                self.preview_cache,
                display_cache_key,
                display_pil_img.copy(),
            )
        try:
            return self._qpixmap_from_pil(display_pil_img)
        except Exception:
            logger.error(
                "Error converting cached high-res preview to QPixmap for %s",
                os.path.basename(normalized_path),
                exc_info=True,
            )
            return None

    def get_preview_qpixmap(
        self,
        image_path: str,
        display_max_size: tuple[int, int]
        | None,  # Max size for the QPixmap to be displayed
        force_regenerate: bool = False,
        force_default_brightness: bool = False,
    ) -> QPixmap | None:
        """
        Gets a QPixmap preview for the image path, scaled to display_max_size.
        Automatically applies auto-edits for RAW files.
        1. Checks cache for display-sized PIL image.
        2. Checks cache for high-resolution preloaded PIL image, then resizes.
        3. Generates fresh PIL image for display size, caches it, then converts.
        """
        normalized_path = os.path.normpath(image_path)
        if not os.path.isfile(normalized_path):
            logger.error(f"File does not exist: {normalized_path}")
            return None

        # Automatically determine if auto-edits should be applied based on file type
        # Cache key for the final display-sized PIL image
        # Ensure display_max_size is a tuple for the cache key, even if None was passed
        key_display_size = (
            display_max_size if display_max_size is not None else PRELOAD_MAX_RESOLUTION
        )
        display_cache_key = self.preview_cache_key(normalized_path, key_display_size)

        # 1. Check if display-sized version is already cached
        if not force_regenerate:
            cached_display_pil = self._cache_get(self.preview_cache, display_cache_key)
            if cached_display_pil:
                return self._qpixmap_from_pil(cached_display_pil)
            else:
                logger.debug(
                    f"Display cache MISS: {os.path.basename(normalized_path)} (Size: {key_display_size})"
                )

        # 2. Check if a high-resolution PRELOADED version is cached
        # Key for preloaded high-res version (uses PRELOAD_MAX_RESOLUTION)
        preload_cache_key = self.preview_cache_key(
            normalized_path, PRELOAD_MAX_RESOLUTION
        )
        cached_high_res_pil = self._cache_get(self.preview_cache, preload_cache_key)

        if cached_high_res_pil:
            logger.debug(
                f"Preview cache HIT (High-Res): {os.path.basename(normalized_path)}. Resizing for display."
            )
            display_pil_img = cached_high_res_pil.copy()
            if display_max_size:
                display_pil_img.thumbnail(display_max_size, Image.Resampling.LANCZOS)

            self._cache_set(self.preview_cache, display_cache_key, display_pil_img)
            return self._qpixmap_from_pil(display_pil_img)

        # 3. Generate fresh for display size, then cache
        logger.debug(
            f"Preview cache MISS for {os.path.basename(normalized_path)}. Generating on-demand..."
        )
        with self._generation_lock(display_cache_key):
            generated_display_pil = None
            if not force_regenerate:
                generated_display_pil = self._cache_get(
                    self.preview_cache, display_cache_key
                )
            if generated_display_pil is None:
                generated_display_pil = self._generate_pil_preview_for_display(
                    normalized_path,
                    display_max_size,
                    force_default_brightness,
                )
        if generated_display_pil:
            self._cache_set(
                self.preview_cache, display_cache_key, generated_display_pil
            )
            return self._qpixmap_from_pil(generated_display_pil)

        logger.error(
            f"Failed to generate or retrieve preview for {os.path.basename(normalized_path)}",
            exc_info=True,
        )
        return None

    def _ensure_thumbnail_generated_and_cached(self, image_path: str) -> None:
        """Worker function for preload_thumbnails."""
        self.ensure_thumbnail_cached(image_path)

    def ensure_preview_cached(
        self,
        image_path: str,
        *,
        force_default_brightness: bool = False,
    ) -> bool:
        """
        Generate and cache one navigation-sized preview when it is missing.

        This method is safe to call from a background worker. It deliberately
        stores a bounded PRELOAD_MAX_RESOLUTION image instead of decoding at the
        larger display size, which keeps navigation prefetch memory predictable.
        Automatically applies auto-edits for RAW files.
        Returns True if successful or already cached, False on error.
        """
        normalized_path = os.path.normpath(image_path)

        # Automatically determine if auto-edits should be applied based on file type
        ext = os.path.splitext(normalized_path)[1].lower()
        apply_auto_edits = is_raw_extension(ext)

        preload_cache_key = self.preview_cache_key(
            normalized_path, PRELOAD_MAX_RESOLUTION
        )

        if (
            not force_default_brightness
            and self._cache_get(self.preview_cache, preload_cache_key) is not None
        ):
            return True

        with self._generation_lock(preload_cache_key):
            if (
                not force_default_brightness
                and self._cache_get(self.preview_cache, preload_cache_key) is not None
            ):
                return True

            pil_img: Image.Image | None = None
            ext = os.path.splitext(normalized_path)[1].lower()
            start_time = time.time()
            high_memory_format = is_raw_extension(ext) or ext in {".heic", ".heif"}
            decode_gate = self._high_memory_decode_gate if high_memory_format else None
            if decode_gate:
                decode_gate.acquire()
            try:
                if is_raw_extension(ext):
                    pil_img = RawImageProcessor.process_raw_for_preview(
                        normalized_path,
                        apply_auto_edits,
                        PRELOAD_MAX_RESOLUTION,
                        force_default_brightness=force_default_brightness,
                    )
                elif ext in SUPPORTED_STANDARD_EXTENSIONS:
                    pil_img = StandardImageProcessor.process_for_preview(
                        normalized_path, PRELOAD_MAX_RESOLUTION
                    )
                else:
                    logger.warning(
                        f"Unsupported extension for preview preload: {ext} for '{os.path.basename(normalized_path)}'"
                    )
                    return False
            finally:
                if decode_gate:
                    decode_gate.release()

            if pil_img:
                self._cache_set(self.preview_cache, preload_cache_key, pil_img)
                duration = time.time() - start_time
                _record_preview_generation_log(
                    duration, os.path.basename(normalized_path)
                )
                return True

            logger.error(
                f"Failed to generate preview for {os.path.basename(normalized_path)}",
                exc_info=True,
            )
            return False

    def preload_previews(
        self,
        image_paths: list[str],
        progress_callback: Callable[[int, int], None] | None = None,
        should_continue_callback: Callable[[], bool] | None = None,
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
            futures_map: dict[concurrent.futures.Future, str] = {}
            for image_path in image_paths:
                if should_continue_callback and not should_continue_callback():
                    logger.info(
                        "Preview preload cancelled by request. Halting new tasks."
                    )
                    break
                future = executor.submit(
                    self.ensure_preview_cached,
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
    ) -> Image.Image | None:
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
            preload_key = self.preview_cache_key(
                normalized_path, PRELOAD_MAX_RESOLUTION
            )
            cached_preview = self._cache_get(self.preview_cache, preload_key)
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

        # Decode directly after a cache miss.
        pil_img: Image.Image | None = None
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

    def get_analysis_image(
        self,
        image_path: str,
        target_size: tuple[int, int],
        target_mode: str = "RGB",
    ) -> Image.Image | None:
        """Return a small, neutral image for ML and measurement workloads.

        Unlike display previews, analysis inputs do not apply cosmetic RAW edits.
        RAW files prefer their embedded JPEG and only fall back to half-size
        post-processing. Cached results are invalidated when the source changes.
        """
        normalized_path = os.path.normpath(image_path)
        if not os.path.isfile(normalized_path):
            logger.error("File does not exist: %s", normalized_path)
            return None

        cache_key = self.analysis_cache_key(normalized_path, ANALYSIS_CACHE_RESOLUTION)
        cached_image = self._cache_get(self.preview_cache, cache_key)
        if cached_image is not None:
            return self._prepare_analysis_result(cached_image, target_size, target_mode)

        with self._generation_lock(cache_key):
            cached_image = self._cache_get(self.preview_cache, cache_key)
            if cached_image is None:
                ext = os.path.splitext(normalized_path)[1].lower()
                high_memory_format = is_raw_extension(ext) or ext in {
                    ".heic",
                    ".heif",
                }
                decode_gate = (
                    self._high_memory_decode_gate if high_memory_format else None
                )
                if decode_gate:
                    decode_gate.acquire()
                try:
                    if is_raw_extension(ext):
                        cached_image = RawImageProcessor.load_raw_for_blur_detection(
                            normalized_path,
                            target_size=ANALYSIS_CACHE_RESOLUTION,
                            apply_auto_edits=False,
                        )
                    elif ext in SUPPORTED_STANDARD_EXTENSIONS:
                        cached_image = StandardImageProcessor.load_for_blur_detection(
                            normalized_path,
                            target_size=ANALYSIS_CACHE_RESOLUTION,
                        )
                    else:
                        logger.warning(
                            "Unsupported analysis image extension %s for %s",
                            ext,
                            os.path.basename(normalized_path),
                        )
                finally:
                    if decode_gate:
                        decode_gate.release()

                if cached_image is not None:
                    if cached_image.mode != "RGB":
                        cached_image = cached_image.convert("RGB")
                    self._cache_set(
                        self.preview_cache,
                        cache_key,
                        cached_image.copy(),
                    )

        if cached_image is None:
            return None
        return self._prepare_analysis_result(cached_image, target_size, target_mode)

    @staticmethod
    def _prepare_analysis_result(
        image: Image.Image,
        target_size: tuple[int, int],
        target_mode: str,
    ) -> Image.Image:
        result = image.copy()
        result.thumbnail(target_size, Image.Resampling.LANCZOS)
        if result.mode != target_mode:
            result = result.convert(target_mode)
        return result

    def get_cached_analysis_qpixmap(
        self,
        image_path: str,
        target_size: tuple[int, int] = ANALYSIS_CACHE_RESOLUTION,
        *,
        memory_only: bool = False,
    ) -> QPixmap | None:
        """Return an existing shared analysis image without generating work."""
        normalized_path = os.path.normpath(image_path)
        if not os.path.isfile(normalized_path):
            return None
        cache_key = self.analysis_cache_key(normalized_path, ANALYSIS_CACHE_RESOLUTION)
        cached_image = (
            self._memory_get(cache_key)
            if memory_only
            else self._cache_get(self.preview_cache, cache_key)
        )
        if cached_image is None:
            return None
        result = self._prepare_analysis_result(cached_image, target_size, "RGB")
        try:
            return self._qpixmap_from_pil(result)
        except Exception:
            logger.error(
                "Error converting cached analysis image to QPixmap for %s",
                os.path.basename(normalized_path),
                exc_info=True,
            )
            return None

    def get_cached_review_qpixmap(
        self,
        image_path: str,
        *,
        thumbnail_apply_orientation: bool = True,
        memory_only: bool = True,
    ) -> QPixmap | None:
        """Return the best cached review image without generating or decoding work."""
        pixmap = self.get_cached_analysis_qpixmap(
            image_path,
            memory_only=memory_only,
        )
        if pixmap is not None and not pixmap.isNull():
            return pixmap
        pixmap = self.get_cached_preview_qpixmap(
            image_path,
            memory_only=memory_only,
        )
        if pixmap is not None and not pixmap.isNull():
            return pixmap
        pixmap = self.get_cached_thumbnail_qpixmap(
            image_path,
            apply_orientation=thumbnail_apply_orientation,
            memory_only=memory_only,
        )
        if pixmap is not None and not pixmap.isNull():
            return pixmap
        return None

    def clear_all_image_caches(self):
        """Clears both thumbnail and preview caches."""
        logger.warning("Clearing all image caches (thumbnails and previews)...")
        with self._memory_cache_lock:
            self._memory_cache.clear()
            self._memory_cache_bytes = 0
        self.thumbnail_cache.clear()
        self.preview_cache.clear()
        logger.info("All image caches have been cleared.")

    def invalidate_path(self, file_path: str) -> None:
        """Remove all memory and disk cache variants for one source file."""
        normalized_path = os.path.normpath(file_path)
        with self._memory_cache_lock:
            keys = [key for key in self._memory_cache if key[0] == normalized_path]
            for key in keys:
                image = self._memory_cache.pop(key)
                self._memory_cache_bytes -= self._image_memory_size(image)
        self.thumbnail_cache.delete_all_for_path(normalized_path)
        self.preview_cache.delete_all_for_path(normalized_path)

    def reinitialize_preview_cache_from_settings(self):
        """Reinitializes the preview cache using current application settings."""
        self.preview_cache.reinitialize_from_settings()

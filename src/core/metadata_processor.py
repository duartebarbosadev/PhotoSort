import pyexiv2
import os
import re
import time
import logging
import unicodedata
from datetime import datetime as dt_parser, date as date_obj
from typing import Dict, Any, Optional, List, Tuple
import concurrent.futures
import threading
import sys

from core.caching.rating_cache import RatingCache
from core.caching.exif_cache import ExifCache
from core.image_processing.image_rotator import ImageRotator, RotationDirection
from core.app_settings import METADATA_PROCESSING_CHUNK_SIZE

logger = logging.getLogger(__name__)

# Global lock to guard pyexiv2 access, which can be sensitive in some frozen/multi-threaded contexts
_PYEXIV2_LOCK = threading.Lock()

# Preferred EXIF/XMP date tags in order of preference
DATE_TAGS_PREFERENCE: List[str] = [
    "Exif.Photo.DateTimeOriginal",
    "Xmp.xmp.CreateDate",
    "Exif.Image.DateTime",
    "Exif.Photo.DateTime",
    "Xmp.photoshop.DateCreated",
]

# Comprehensive metadata tags for extraction (pyexiv2 format)
COMPREHENSIVE_METADATA_TAGS: List[str] = [
    # Basic file info
    "Exif.Image.Make",
    "Exif.Image.Model",
    "Exif.Photo.LensModel",
    "Exif.Photo.LensSpecification",
    "Exif.Photo.FocalLength",
    "Exif.Photo.FNumber",
    "Exif.Photo.ApertureValue",
    "Exif.Photo.ShutterSpeedValue",
    "Exif.Photo.ExposureTime",
    "Exif.Photo.ISOSpeedRatings",
    "Exif.Photo.Flash",
    "Exif.Image.ImageWidth",
    "Exif.Image.ImageLength",
    "Exif.Image.ColorSpace",
    "Exif.Image.Orientation",
    "Exif.Image.BitsPerSample",
    "Exif.Photo.ExposureCompensation",
    "Exif.Photo.MeteringMode",
    "Exif.Photo.WhiteBalance",
    # GPS
    "Exif.GPSInfo.GPSLatitude",
    "Exif.GPSInfo.GPSLongitude",
    "Exif.GPSInfo.GPSLatitudeRef",
    "Exif.GPSInfo.GPSLongitudeRef",
    # XMP data
    "Xmp.xmp.Rating",
    "Xmp.xmp.Label",
    "Xmp.dc.subject",
    "Xmp.lr.hierarchicalSubject",
] + DATE_TAGS_PREFERENCE


def _parse_exif_date(date_string: str) -> Optional[date_obj]:
    """
    Attempts to parse various EXIF/XMP date string formats.
    Returns a datetime.date object or None.
    """
    if not date_string or not isinstance(date_string, str):
        return None

    s = date_string.strip()
    if not s:
        return None

    # Strip trailing fractional seconds for common cases while keeping delimiters
    # e.g. "2023-12-25 14:30:45.123" -> "2023-12-25 14:30:45"
    s = re.sub(r"(\d{2}:\d{2}:\d{2})\.\d+$", r"\1", s)

    # Strip trailing 'Z' UTC suffix
    if s.endswith("Z"):
        s = s[:-1]

    # Remove timezone offset like +01:00 or -0300 at the end
    s = re.sub(r"[+-]\d{2}:?\d{2}$", "", s).strip()

    # Try date-time formats first, then date-only formats
    datetime_formats = [
        "%Y:%m:%d %H:%M:%S",
        "%Y-%m-%d %H:%M:%S",
        "%Y/%m/%d %H:%M:%S",
        "%Y.%m.%d %H:%M:%S",
        "%Y-%m-%dT%H:%M:%S",
    ]
    date_only_formats = [
        "%Y:%m:%d",
        "%Y-%m-%d",
        "%Y/%m/%d",
        "%Y.%m.%d",
    ]

    # First pass: try to parse as full datetime
    for fmt in datetime_formats:
        try:
            return dt_parser.strptime(s, fmt).date()
        except (ValueError, TypeError):
            continue

    # Second pass: if there's a time component, trim it and try date-only formats
    s_date_part = s.split("T")[0].split(" ")[0]
    for fmt in date_only_formats:
        try:
            return dt_parser.strptime(s_date_part, fmt).date()
        except (ValueError, TypeError):
            continue

    return None


def _parse_date_from_filename(filename: str) -> Optional[date_obj]:
    """
    Attempts to parse a date (YYYY, MM, DD) from common filename patterns.
    Returns a datetime.date object or None.
    """
    match1 = re.search(r"(\d{4})(\d{2})(\d{2})(?:[_ \-T]|$)", filename)
    match2 = re.search(r"(\d{4})[-_\.](\d{2})[-_\.](\d{2})", filename)
    year, month, day = None, None, None

    def validate_and_assign(y_str, m_str, d_str):
        nonlocal year, month, day
        try:
            y, m, d = int(y_str), int(m_str), int(d_str)
            # Validate date components by trying to create a datetime object
            if (
                1900 <= y <= dt_parser.now().year + 10 and 1 <= m <= 12 and 1 <= d <= 31
            ):  # Extended year range slightly
                dt_parser(y, m, d)
                year, month, day = y, m, d
                return True
        except (ValueError, IndexError, TypeError):  # Catch TypeError
            pass
        return False

    if match1 and validate_and_assign(
        match1.group(1), match1.group(2), match1.group(3)
    ):
        pass
    elif match2 and validate_and_assign(
        match2.group(1), match2.group(2), match2.group(3)
    ):
        pass

    if year and month and day:
        try:
            return date_obj(year, month, day)
        except ValueError:  # Handles invalid date like Feb 30
            return None
    return None


def _parse_rating(value: Any) -> int:
    """
    Safely converts a metadata rating value to an integer between 0 and 5.
    """
    if value is None:
        return 0
    try:
        rating_val = int(
            float(str(value))
        )  # str(value) handles pyexiv2 Fraction or other types
        return max(0, min(5, rating_val))
    except (ValueError, TypeError):
        return 0


class MetadataProcessor:
    @staticmethod
    def _resolve_path_forms(original_path: str) -> Optional[Tuple[str, str]]:
        """
        Resolves an image path to its operational form (that os.path.isfile works with)
        and a canonical NFC form for caching.

        Tries original (normpathed), then its NFC variant, then its NFD variant.
        The first one found via os.path.isfile is the 'operational_path'.
        The 'canonical_cache_path' is always the NFC normalization of the operational_path.

        Returns:
            A tuple (operational_path, canonical_cache_path) if the file is found.
            None if the file cannot be found.
        """
        if not original_path:  # Handle empty input path
            return None

        base_path = os.path.normpath(original_path)
        paths_to_try = [base_path]

        nfc_variant = unicodedata.normalize("NFC", base_path)
        if nfc_variant not in paths_to_try:
            paths_to_try.append(nfc_variant)

        nfd_variant = unicodedata.normalize("NFD", base_path)
        if nfd_variant not in paths_to_try:
            paths_to_try.append(nfd_variant)

        operational_path_found = None
        for p_variant in paths_to_try:
            try:
                if os.path.isfile(p_variant):
                    operational_path_found = p_variant
                    logger.debug(
                        f"Found operational path: '{p_variant}' (from original '{original_path}')"
                    )
                    break
            except Exception as e:
                logger.debug(f"Error checking path variant '{p_variant}': {e}")
                continue

        if operational_path_found:
            canonical_cache_path = unicodedata.normalize("NFC", operational_path_found)
            return operational_path_found, canonical_cache_path
        else:
            logger.warning(f"Could not find accessible file for '{original_path}'")
            return None

    @staticmethod
    def get_batch_display_metadata(
        image_paths: List[str],
        rating_disk_cache: Optional[RatingCache] = None,
        exif_disk_cache: Optional[ExifCache] = None,
    ) -> Dict[str, Dict[str, Any]]:
        """
        Fetches and parses essential metadata for a batch of images.
        Uses ExifCache first, then pyexiv2 in parallel for remaining files.
        Populates caches and applies date fallbacks.

        Returns per canonical NFC path a dict with keys at least:
        - rating: int
        - label: Optional[str]
        - date: Optional[date]
        """
        results: Dict[str, Dict[str, Any]] = {}
        # Stores operational_path -> cache_key_path mapping for files needing extraction
        operational_to_cache_key_map: Dict[str, str] = {}
        paths_for_pyexiv2_extraction: List[str] = []  # Stores operational paths

        start_time = time.perf_counter()
        logger.info(f"Starting batch metadata fetch for {len(image_paths)} files.")

        def _init_result_dict() -> Dict[str, Any]:
            # Ensure required keys exist for tests and callers
            return {"rating": 0, "label": None, "date": None, "raw_metadata": None}

        for image_path_input in image_paths:
            resolved = MetadataProcessor._resolve_path_forms(image_path_input)

            # Use NFC of original input as the key for the results dict if resolution fails,
            # for consistency if the caller expects a result for every input path.
            # If resolution succeeds, cache_key_path (which is NFC of operational) is used.
            result_key_for_this_file = unicodedata.normalize(
                "NFC", os.path.normpath(image_path_input)
            )

            if not resolved:
                results[result_key_for_this_file] = _init_result_dict()
                minimal_data = {
                    "file_path": result_key_for_this_file,
                    "file_size": "Unknown",
                    "error": "File not found or inaccessible during path resolution",
                }
                if exif_disk_cache:
                    exif_disk_cache.set(
                        result_key_for_this_file, minimal_data
                    )  # Cache "not found" state
                results[result_key_for_this_file]["date"] = _parse_date_from_filename(
                    os.path.basename(result_key_for_this_file)
                )
                results[result_key_for_this_file]["raw_metadata"] = minimal_data
                continue

            operational_path, cache_key_path = resolved
            results[cache_key_path] = _init_result_dict()  # Use canonical key

            cached_metadata: Optional[Dict[str, Any]] = None
            if exif_disk_cache:
                cached_metadata = exif_disk_cache.get(cache_key_path)

            if cached_metadata:
                logger.debug(f"ExifCache HIT for: {os.path.basename(operational_path)}")
                results[cache_key_path]["raw_metadata"] = cached_metadata
            else:
                logger.debug(
                    f"ExifCache MISS for: {os.path.basename(operational_path)}"
                )
                paths_for_pyexiv2_extraction.append(operational_path)
                operational_to_cache_key_map[operational_path] = cache_key_path

        CHUNK_SIZE = METADATA_PROCESSING_CHUNK_SIZE
        # Concurrency tuning: reduce to 1 worker in frozen builds by default (can override)
        default_workers = min(6, (os.cpu_count() or 1) * 2)
        if getattr(sys, "frozen", False):
            try:
                env_workers = int(os.environ.get("PHOTOSORT_METADATA_MAX_WORKERS", "1"))
                MAX_WORKERS = max(1, env_workers)
            except Exception:
                MAX_WORKERS = 1
        else:
            MAX_WORKERS = default_workers

        def process_chunk(chunk_paths: List[str]) -> List[Dict[str, Any]]:
            chunk_results = []
            for op_path in chunk_paths:  # op_path is the operational_path
                # Quick existence guard to avoid noisy errors for removed/missing files
                if not os.path.isfile(op_path):
                    logger.warning(
                        f"Skipping metadata extraction for missing file: {op_path}"
                    )
                    chunk_results.append(
                        {
                            "file_path": op_path,
                            "file_size": "Unknown",
                            "error": "File missing at extraction time",
                        }
                    )
                    continue
                try:
                    # Guard ALL pyexiv2 operations with a lock for stability in threaded runs
                    with _PYEXIV2_LOCK:
                        with pyexiv2.Image(
                            op_path, encoding="utf-8"
                        ) as img:  # Use operational_path
                            combined_metadata = {
                                "file_path": op_path,  # Store operational_path used for extraction
                                "pixel_width": img.get_pixel_width(),
                                "pixel_height": img.get_pixel_height(),
                                "mime_type": img.get_mime_type(),
                                "file_size": os.path.getsize(op_path)
                                if os.path.isfile(op_path)
                                else "Unknown",
                                **(img.read_exif() or {}),  # Ensure dicts even if empty
                                **(img.read_iptc() or {}),
                                **(img.read_xmp() or {}),
                            }
                            chunk_results.append(combined_metadata)
                            logger.debug(
                                f"Successfully extracted metadata for {os.path.basename(op_path)}"
                            )
                except Exception as e:
                    # pyexiv2 raises RuntimeError for many IO issues; downshift errno=2 to warning without traceback
                    msg = str(e)
                    is_missing = (
                        ("No such file or directory" in msg)
                        or ("errno = 2" in msg)
                        or (not os.path.isfile(op_path))
                    )
                    if is_missing:
                        logger.warning(
                            f"Skipping missing file during metadata extraction: {op_path} ({msg})"
                        )
                        chunk_results.append(
                            {
                                "file_path": op_path,
                                "file_size": "Unknown",
                                "error": f"Extraction skipped (missing): {msg}",
                            }
                        )
                    else:
                        logger.error(
                            f"Error extracting metadata for {os.path.basename(op_path)}: {e}",
                            exc_info=True,
                        )
                        chunk_results.append(
                            {
                                "file_path": op_path,
                                "file_size": os.path.getsize(op_path)
                                if os.path.isfile(op_path)
                                else "Unknown",
                                "error": f"Extraction failed: {e}",
                            }
                        )
            return chunk_results

        all_metadata_results: List[Dict[str, Any]] = []

        if paths_for_pyexiv2_extraction:
            # Decide whether to run in parallel. On Windows and frozen builds, default to sequential
            # unless explicitly overridden via PHOTOSORT_METADATA_PARALLEL=true.
            parallel_override = os.environ.get(
                "PHOTOSORT_METADATA_PARALLEL", "false"
            ).lower() in {"1", "true", "yes"}
            # Only force sequential on frozen builds unless explicitly overridden
            should_run_sequential = (
                getattr(sys, "frozen", False) and not parallel_override
            )

            if should_run_sequential:
                context_reason = "frozen build"
                logger.info(
                    f"Extracting metadata for {len(paths_for_pyexiv2_extraction)} files sequentially ({context_reason})."
                )
                try:
                    all_metadata_results = process_chunk(paths_for_pyexiv2_extraction)
                except Exception as exc:
                    logger.error(
                        f"Sequential metadata extraction failed with error: {exc}",
                        exc_info=True,
                    )
                    all_metadata_results = []
            else:
                logger.info(
                    f"Extracting metadata for {len(paths_for_pyexiv2_extraction)} files in parallel..."
                )
                # Parallel execution logic (non-frozen or explicitly enabled)
                with concurrent.futures.ThreadPoolExecutor(
                    max_workers=MAX_WORKERS
                ) as executor:
                    future_to_chunk = {
                        executor.submit(
                            process_chunk,
                            paths_for_pyexiv2_extraction[i : i + CHUNK_SIZE],
                        ): paths_for_pyexiv2_extraction[i : i + CHUNK_SIZE]
                        for i in range(0, len(paths_for_pyexiv2_extraction), CHUNK_SIZE)
                    }
                    for future in concurrent.futures.as_completed(future_to_chunk):
                        try:
                            all_metadata_results.extend(future.result())
                        except Exception as exc:
                            logger.error(
                                f"A chunk of files failed during metadata extraction: {exc}"
                            )

        if all_metadata_results:
            for metadata_dict in all_metadata_results:
                op_path_processed = metadata_dict.get("file_path")
                if op_path_processed:
                    # Get the cache_key_path associated with this operational_path
                    current_cache_key = operational_to_cache_key_map.get(
                        op_path_processed
                    )
                    if current_cache_key:
                        results[current_cache_key]["raw_metadata"] = metadata_dict
                        if exif_disk_cache:
                            exif_disk_cache.set(current_cache_key, metadata_dict)
                    else:
                        logger.error(
                            f"Could not find cache key for operational path: {op_path_processed}",
                            exc_info=True,
                        )
                else:  # Should not happen if process_chunk always includes file_path
                    logger.warning(
                        "Metadata result is missing the 'file_path' key.",
                        exc_info=True,
                    )

        final_results_for_caller: Dict[str, Dict[str, Any]] = {}
        for cache_key, data_dict in results.items():  # cache_key is NFC normalized
            filename_only = os.path.basename(cache_key)  # For logging
            parsed_rating, parsed_date = 0, None
            parsed_label: Optional[str] = None
            raw_metadata = data_dict["raw_metadata"]

            if raw_metadata and "error" not in raw_metadata:
                # Rating
                rating_raw_val = raw_metadata.get("Xmp.xmp.Rating")
                parsed_rating = _parse_rating(rating_raw_val)
                # Date
                for date_tag in DATE_TAGS_PREFERENCE:
                    date_string = raw_metadata.get(date_tag)
                    if date_string:
                        dt_obj_val = _parse_exif_date(str(date_string))
                        if dt_obj_val:
                            parsed_date = dt_obj_val
                            break
                # Label
                label_val = raw_metadata.get("Xmp.xmp.Label")
                if label_val is not None:
                    try:
                        parsed_label = (
                            str(label_val) if str(label_val).strip() != "" else None
                        )
                    except Exception:
                        parsed_label = None

            # Date fallback from filename
            if parsed_date is None:
                parsed_date = _parse_date_from_filename(filename_only)

            # Filesystem date fallback if we have an operational path
            op_path_for_stat = raw_metadata.get("file_path") if raw_metadata else None
            if (
                parsed_date is None
                and op_path_for_stat
                and os.path.isfile(op_path_for_stat)
            ):
                try:
                    fs_timestamp: Optional[float] = None
                    stat_result = os.stat(op_path_for_stat)
                    if (
                        hasattr(stat_result, "st_birthtime")
                        and stat_result.st_birthtime > 0
                    ):
                        fs_timestamp = stat_result.st_birthtime
                    if fs_timestamp is None or fs_timestamp < 1000000:
                        fs_timestamp = stat_result.st_mtime
                    if fs_timestamp:
                        parsed_date = dt_parser.fromtimestamp(fs_timestamp).date()
                except Exception as e_fs:
                    logger.warning(
                        f"Filesystem date fallback error for {filename_only}: {e_fs}"
                    )
            final_results_for_caller[cache_key] = {
                "rating": parsed_rating,
                "date": parsed_date,
            }
            logger.debug(
                f"Processed {filename_only}: Rating={parsed_rating}, Label={parsed_label}, Date={parsed_date}"
            )

        duration = time.perf_counter() - start_time
        logger.info(
            f"Finished batch metadata fetch for {len(image_paths)} files in {duration:.4f}s."
        )
        return final_results_for_caller

    @staticmethod
    def set_rating(
        image_path: str,
        rating: int,
        rating_disk_cache: Optional[RatingCache] = None,
        exif_disk_cache: Optional[ExifCache] = None,
    ) -> bool:
        """
        Sets the rating (0-5) using pyexiv2.
        Updates rating_disk_cache and invalidates exif_disk_cache if provided.
        Returns True on apparent success, False on failure.
        """
        try:
            rating_int = int(rating)
        except (ValueError, TypeError):
            logger.error(f"Invalid rating value '{rating}'. Must be an integer 0-5.")
            return False
        if not (0 <= rating_int <= 5):
            logger.error(f"Invalid rating value {rating_int}. Must be 0-5.")
            return False

        resolved = MetadataProcessor._resolve_path_forms(image_path)
        if not resolved:
            return False
        operational_path, cache_key_path = resolved

        success = False
        logger.info(
            f"Setting rating for {os.path.basename(operational_path)} to {rating_int}."
        )
        try:
            # Guard pyexiv2 operations with global lock
            with _PYEXIV2_LOCK:
                with pyexiv2.Image(operational_path, encoding="utf-8") as img:
                    img.modify_xmp({"Xmp.xmp.Rating": str(rating_int)})
                    success = True
        except Exception as e:
            logger.error(
                f"Error setting rating for {os.path.basename(operational_path)}: {e}",
                exc_info=True,
            )

        if success:
            if rating_disk_cache:
                rating_disk_cache.set(cache_key_path, rating_int)
            if exif_disk_cache:
                exif_disk_cache.delete(cache_key_path)
        return success

    @staticmethod
    def get_detailed_metadata(
        image_path: str, exif_disk_cache: Optional[ExifCache] = None
    ) -> Optional[Dict[str, Any]]:
        """
        Fetches detailed metadata for a single image for sidebar display.
        Since batch loading now fetches all detailed metadata, this should mostly be cache hits.
        """
        resolved = MetadataProcessor._resolve_path_forms(image_path)
        if not resolved:
            # If caller expects a specific structure for "not found"
            # return {"file_path": unicodedata.normalize('NFC', os.path.normpath(image_path)), "error": "File not found"}
            return None
        operational_path, cache_key_path = resolved

        logger.debug(
            f"Fetching detailed metadata for: {os.path.basename(operational_path)}"
        )

        if exif_disk_cache:
            cached_data = exif_disk_cache.get(cache_key_path)
            if cached_data is not None:
                logger.debug(
                    f"ExifCache HIT for detailed metadata: {os.path.basename(cache_key_path)}"
                )
                return cached_data
            else:
                logger.debug(
                    f"ExifCache MISS for detailed metadata: {os.path.basename(cache_key_path)}"
                )

        # Existence guard first
        if not os.path.isfile(operational_path):
            msg = "File missing at detailed metadata read"
            logger.info(f"{msg}: {operational_path}")
            minimal_result = {
                "file_path": operational_path,
                "file_size": "Unknown",
                "error": msg,
            }
            if exif_disk_cache:
                exif_disk_cache.set(cache_key_path, minimal_result)
            return minimal_result

        try:
            # RAW files are processed normally; no special skip under pytest on Windows
            # Guard pyexiv2 operations with global lock
            with _PYEXIV2_LOCK:
                with pyexiv2.Image(operational_path, encoding="utf-8") as img:
                    metadata = {
                        "file_path": operational_path,  # Store operational path used
                        "pixel_width": img.get_pixel_width(),
                        "pixel_height": img.get_pixel_height(),
                        "mime_type": img.get_mime_type(),
                        "file_size": os.path.getsize(operational_path)
                        if os.path.isfile(operational_path)
                        else "Unknown",
                    }
                    # ... (rest of metadata fetching as before: exif, iptc, xmp) ...
                    try:
                        metadata.update(img.read_exif() or {})
                    except Exception:
                        logger.debug(
                            f"No EXIF for {os.path.basename(operational_path)}"
                        )
                    try:
                        metadata.update(img.read_iptc() or {})
                    except Exception:
                        logger.debug(
                            f"No IPTC for {os.path.basename(operational_path)}"
                        )
                    try:
                        metadata.update(img.read_xmp() or {})
                    except Exception:
                        logger.debug(f"No XMP for {os.path.basename(operational_path)}")

                    if exif_disk_cache:
                        exif_disk_cache.set(cache_key_path, metadata)
                    return metadata
        except Exception as e:
            msg = str(e)
            is_missing = (
                ("No such file or directory" in msg)
                or ("errno = 2" in msg)
                or (not os.path.isfile(operational_path))
            )
            if is_missing:
                logger.warning(
                    f"Skipping missing file during detailed metadata read: {operational_path} ({msg})"
                )
            else:
                logger.error(
                    f"Error fetching detailed metadata for {os.path.basename(operational_path)}: {e}",
                    exc_info=True,
                )
            minimal_result = {
                "file_path": operational_path,
                "file_size": "Unknown",
                "error": str(e),
            }
            if exif_disk_cache:
                exif_disk_cache.set(cache_key_path, minimal_result)  # Cache error state
            return minimal_result

    @staticmethod
    def rotate_image(
        image_path: str,
        direction: RotationDirection,
        update_metadata_only: bool = False,
        exif_disk_cache: Optional[ExifCache] = None,
    ) -> bool:
        """
        Rotate an image using the ImageRotator.
        Invalidates exif_disk_cache after rotation.

        Args:
            image_path: Path to the image file
            direction: Rotation direction ('clockwise', 'counterclockwise', '180')
            update_metadata_only: If True, only update orientation metadata without rotating pixels
            exif_disk_cache: Optional cache to invalidate after rotation

        Returns:
            True if rotation was successful, False otherwise
        """
        resolved = MetadataProcessor._resolve_path_forms(image_path)
        if not resolved:
            return False
        operational_path, cache_key_path = resolved

        try:
            rotator = ImageRotator()
            # ImageRotator must also be able to handle the operational_path correctly.
            # If it internally uses pyexiv2, it should also use encoding='utf-8'.
            success, message = rotator.rotate_image(
                operational_path, direction, update_metadata_only
            )

            if success:
                logger.info(
                    f"Rotation of '{os.path.basename(operational_path)}' reported: {message}"
                )
                if exif_disk_cache:
                    exif_disk_cache.delete(cache_key_path)
            else:
                logger.error(
                    f"Rotation failed for '{os.path.basename(operational_path)}': {message}"
                )
            return success
        except Exception as e:
            logger.error(
                f"Exception during rotation for {os.path.basename(operational_path)}: {e}",
                exc_info=True,
            )
            return False

    @staticmethod
    def rotate_clockwise(
        image_path: str,
        update_metadata_only: bool = False,
        exif_disk_cache: Optional[ExifCache] = None,
    ) -> bool:
        """Rotate image 90° clockwise."""
        return MetadataProcessor.rotate_image(
            image_path, "clockwise", update_metadata_only, exif_disk_cache
        )

    @staticmethod
    def rotate_counterclockwise(
        image_path: str,
        update_metadata_only: bool = False,
        exif_disk_cache: Optional[ExifCache] = None,
    ) -> bool:
        """Rotate image 90° counterclockwise."""
        return MetadataProcessor.rotate_image(
            image_path, "counterclockwise", update_metadata_only, exif_disk_cache
        )

    @staticmethod
    def rotate_180(
        image_path: str,
        update_metadata_only: bool = False,
        exif_disk_cache: Optional[ExifCache] = None,
    ) -> bool:
        """Rotate image 180°."""
        return MetadataProcessor.rotate_image(
            image_path, "180", update_metadata_only, exif_disk_cache
        )

    @staticmethod
    def try_metadata_rotation_first(
        image_path: str,
        direction: RotationDirection,
        exif_disk_cache: Optional[ExifCache] = None,
    ) -> Tuple[bool, bool, str]:
        """
        Try metadata-only rotation first (preferred lossless method).

        Args:
            image_path: Path to the image file
            direction: Rotation direction
            exif_disk_cache: Optional cache to invalidate if successful

        Returns:
            Tuple of (metadata_rotation_succeeded: bool, needs_lossy_rotation: bool, message: str)
        """
        resolved = MetadataProcessor._resolve_path_forms(image_path)
        if not resolved:
            return False, False, f"Could not resolve path: {image_path}"

        operational_path, cache_key_path = resolved

        try:
            rotator = ImageRotator()
            success, needs_lossy, message = rotator.try_metadata_rotation_first(
                operational_path, direction
            )

            if success and exif_disk_cache:
                # Invalidate cache after successful metadata rotation
                exif_disk_cache.delete(cache_key_path)

            return success, needs_lossy, message
        except Exception as e:
            error_msg = f"Exception during metadata rotation attempt for {os.path.basename(operational_path)}: {e}"
            logger.error(error_msg, exc_info=True)
            return False, False, error_msg

    @staticmethod
    def is_rotation_supported(image_path: str) -> bool:
        """Check if rotation is supported for the given image format."""

        resolved = MetadataProcessor._resolve_path_forms(image_path)
        if not resolved:
            return False
        operational_path, _ = resolved  # cache_key_path not needed here

        try:
            rotator = ImageRotator()
            return rotator.is_rotation_supported(operational_path)
        except Exception as e:
            logger.error(
                f"Error checking rotation support for {os.path.basename(operational_path)}: {e}",
                exc_info=True,
            )
            return False

    @staticmethod
    def set_orientation(
        image_path: str, orientation: int, exif_disk_cache: Optional[ExifCache] = None
    ) -> bool:
        """
        Sets the EXIF orientation tag directly.

        Args:
            image_path: Path to the image file
            orientation: The EXIF orientation value (1-8)
            exif_disk_cache: Optional cache to invalidate

        Returns:
            True if successful, False otherwise
        """
        if not (1 <= orientation <= 8):
            logger.error(
                f"Invalid EXIF orientation value: {orientation}. Must be between 1 and 8."
            )
            return False

        resolved = MetadataProcessor._resolve_path_forms(image_path)
        if not resolved:
            return False
        operational_path, cache_key_path = resolved

        # Existence guard
        if not os.path.isfile(operational_path):
            logger.warning(
                f"Cannot set EXIF orientation; file missing: {operational_path}"
            )
            return False
        try:
            # Guard pyexiv2 operations with global lock
            with _PYEXIV2_LOCK:
                with pyexiv2.Image(operational_path, encoding="utf-8") as img:
                    img.modify_exif({"Exif.Image.Orientation": orientation})
                    logger.info(
                        f"Set EXIF orientation for {os.path.basename(operational_path)} to {orientation}"
                    )

            if exif_disk_cache:
                exif_disk_cache.delete(cache_key_path)
            return True
        except Exception as e:
            msg = str(e)
            if (
                ("No such file or directory" in msg)
                or ("errno = 2" in msg)
                or (not os.path.isfile(operational_path))
            ):
                logger.warning(
                    f"File missing while setting EXIF orientation: {operational_path} ({msg})"
                )
            else:
                logger.error(
                    f"Error setting EXIF orientation for {os.path.basename(operational_path)}: {e}",
                    exc_info=True,
                )
            return False

    @staticmethod
    def get_orientation(
        image_path: str, exif_disk_cache: Optional[ExifCache] = None
    ) -> Optional[int]:
        """
        Retrieves the EXIF orientation value from an image.

        Args:
            image_path: Path to the image file
            exif_disk_cache: Optional cache to check first

        Returns:
            The orientation value (1-8) or None if not found or on error.
        """
        resolved = MetadataProcessor._resolve_path_forms(image_path)
        if not resolved:
            return None
        operational_path, cache_key_path = resolved

        # Check cache first
        if exif_disk_cache:
            cached_data = exif_disk_cache.get(cache_key_path)
            if cached_data and "Exif.Image.Orientation" in cached_data:
                try:
                    return int(cached_data["Exif.Image.Orientation"])
                except (ValueError, TypeError):
                    pass  # Fall through to direct read if cached value is invalid

        # If not in cache or cache is not provided, read from file
        # Existence guard
        if not os.path.isfile(operational_path):
            logger.info(
                f"File missing when querying EXIF orientation: {operational_path}"
            )
            return None
        try:
            # Guard pyexiv2 operations with global lock
            with _PYEXIV2_LOCK:
                with pyexiv2.Image(operational_path, encoding="utf-8") as img:
                    exif_data = img.read_exif()
                    orientation = exif_data.get("Exif.Image.Orientation")
                    if orientation:
                        return int(orientation)
                    return None
        except Exception as e:
            msg = str(e)
            if (
                ("No such file or directory" in msg)
                or ("errno = 2" in msg)
                or (not os.path.isfile(operational_path))
            ):
                logger.warning(
                    f"File missing while reading EXIF orientation: {operational_path} ({msg})"
                )
            else:
                logger.error(
                    f"Error getting EXIF orientation for {os.path.basename(operational_path)}: {e}",
                    exc_info=True,
                )
            return None

    @staticmethod
    def get_orientation_and_dimensions(
        image_path: str, exif_disk_cache: Optional[ExifCache] = None
    ) -> Tuple[Optional[int], Optional[int], Optional[int]]:
        """
        Retrieves orientation, pixel width, and pixel height efficiently.

        Args:
            image_path: Path to the image file
            exif_disk_cache: Optional cache to check first

        Returns:
            A tuple (orientation, width, height). Values can be None on error.
        """
        resolved = MetadataProcessor._resolve_path_forms(image_path)
        if not resolved:
            return None, None, None
        operational_path, cache_key_path = resolved

        # Check cache first
        if exif_disk_cache:
            cached_data = exif_disk_cache.get(cache_key_path)
            if cached_data:
                try:
                    orientation = (
                        int(cached_data["Exif.Image.Orientation"])
                        if cached_data.get("Exif.Image.Orientation") is not None
                        else None
                    )
                    width = (
                        int(cached_data["pixel_width"])
                        if cached_data.get("pixel_width") is not None
                        else None
                    )
                    height = (
                        int(cached_data["pixel_height"])
                        if cached_data.get("pixel_height") is not None
                        else None
                    )
                    if (
                        orientation is not None
                        and width is not None
                        and height is not None
                    ):
                        return orientation, width, height
                except (ValueError, TypeError):
                    pass  # Fall through to direct read

        # If not in cache or cache is not provided, read from file
        # Existence guard
        if not os.path.isfile(operational_path):
            logger.info(
                f"File missing when reading orientation/dimensions: {operational_path}"
            )
            return None, None, None
        try:
            # Guard pyexiv2 operations with global lock
            with _PYEXIV2_LOCK:
                with pyexiv2.Image(operational_path, encoding="utf-8") as img:
                    orientation = img.read_exif().get("Exif.Image.Orientation")
                    orientation = int(orientation) if orientation else None
                    width = img.get_pixel_width()
                    height = img.get_pixel_height()
                    return orientation, width, height
        except Exception as e:
            msg = str(e)
            if (
                ("No such file or directory" in msg)
                or ("errno = 2" in msg)
                or (not os.path.isfile(operational_path))
            ):
                logger.warning(
                    f"File missing while reading orientation/dimensions: {operational_path} ({msg})"
                )
            else:
                logger.error(
                    f"Error getting orientation/dimensions for {os.path.basename(operational_path)}: {e}",
                    exc_info=True,
                )
            return None, None, None

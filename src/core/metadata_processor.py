import pyexiv2
import os
import re
import time
import logging
import unicodedata
from datetime import datetime as dt_parser, date as date_obj
from typing import Dict, Any, Optional, List, Tuple
import concurrent.futures

from src.core.caching.rating_cache import RatingCache
from src.core.caching.exif_cache import ExifCache
from src.core.image_processing.image_rotator import ImageRotator, RotationDirection

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

    date_string = date_string.strip()
    date_string_no_frac = date_string.split(".")[0]
    date_string_no_tz = date_string_no_frac
    if "Z" in date_string_no_tz:  # Handle 'Z' for UTC
        date_string_no_tz = date_string_no_tz.split("Z")[0]
    # Robust timezone offset removal
    tz_match = re.search(r"[+-]\d{2}(:?\d{2})?$", date_string_no_tz)
    if tz_match:
        date_string_no_tz = date_string_no_tz[: tz_match.start()]

    formats_to_try = [
        "%Y:%m:%d %H:%M:%S",
        "%Y-%m-%d %H:%M:%S",
        "%Y/%m/%d %H:%M:%S",
        "%Y.%m.%d %H:%M:%S",
        "%Y-%m-%dT%H:%M:%S",
        "%Y:%m:%d",
        "%Y-%m-%d",
        "%Y/%m/%d",
        "%Y.%m.%d",
    ]
    parsed_date = None
    for fmt in formats_to_try:
        try:
            string_to_parse = date_string_no_tz
            # If format is date-only, try parsing only the date part of the string
            if "T" not in fmt and " " not in fmt:
                if "T" in string_to_parse:
                    string_to_parse = string_to_parse.split("T")[0]
                elif " " in string_to_parse:
                    string_to_parse = string_to_parse.split(" ")[0]
            parsed_date = dt_parser.strptime(string_to_parse, fmt).date()
            break
        except (ValueError, TypeError):  # Catch TypeError as well
            continue
    return parsed_date


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
                    logging.debug(
                        f"[MetadataProcessor._resolve_path_forms] Found operational path: '{p_variant}' (from original '{original_path}', tried variants: {paths_to_try})"
                    )
                    break
            except Exception as e:
                logging.debug(
                    f"[MetadataProcessor._resolve_path_forms] Error checking path variant '{p_variant}': {e}"
                )
                continue

        if operational_path_found:
            canonical_cache_path = unicodedata.normalize("NFC", operational_path_found)
            return operational_path_found, canonical_cache_path
        else:
            logging.warning(
                f"[MetadataProcessor._resolve_path_forms] Could not find an accessible file for original path: '{original_path}'. Checked variants: {paths_to_try}"
            )
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
        """
        results: Dict[str, Dict[str, Any]] = {}
        # Stores operational_path -> cache_key_path mapping for files needing extraction
        operational_to_cache_key_map: Dict[str, str] = {}
        paths_for_pyexiv2_extraction: List[str] = []  # Stores operational paths

        start_time = time.perf_counter()
        logging.info(
            f"[MetadataProcessor] Starting batch metadata for {len(image_paths)} files."
        )

        for image_path_input in image_paths:
            resolved = MetadataProcessor._resolve_path_forms(image_path_input)

            # Use NFC of original input as the key for the results dict if resolution fails,
            # for consistency if the caller expects a result for every input path.
            # If resolution succeeds, cache_key_path (which is NFC of operational) is used.
            result_key_for_this_file = unicodedata.normalize(
                "NFC", os.path.normpath(image_path_input)
            )

            if not resolved:
                results[result_key_for_this_file] = {
                    "rating": 0,
                    "date": None,
                    "raw_metadata": None,
                }
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
            results[cache_key_path] = {
                "rating": 0,
                "date": None,
                "raw_metadata": None,
            }  # Use canonical key

            cached_metadata: Optional[Dict[str, Any]] = None
            if exif_disk_cache:
                cached_metadata = exif_disk_cache.get(cache_key_path)

            if cached_metadata:
                logging.debug(
                    f"[MetadataProcessor] ExifCache HIT for {os.path.basename(operational_path)} (cache key: {os.path.basename(cache_key_path)})."
                )
                results[cache_key_path]["raw_metadata"] = cached_metadata
            else:
                logging.debug(
                    f"[MetadataProcessor] ExifCache MISS for {os.path.basename(operational_path)} (cache key: {os.path.basename(cache_key_path)})."
                )
                paths_for_pyexiv2_extraction.append(operational_path)
                operational_to_cache_key_map[operational_path] = cache_key_path

        CHUNK_SIZE = 25
        MAX_WORKERS = min(6, (os.cpu_count() or 1) * 2)

        if paths_for_pyexiv2_extraction:
            logging.info(
                f"[MetadataProcessor] Need to extract metadata for {len(paths_for_pyexiv2_extraction)} files. Processing in parallel."
            )

            def process_chunk(chunk_paths: List[str]) -> List[Dict[str, Any]]:
                chunk_results = []
                for op_path in chunk_paths:  # op_path is the operational_path
                    # file_ext = os.path.splitext(op_path)[1].lower() # F841: Local variable `file_ext` is assigned to but never used
                    try:
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
                            logging.info(
                                f"[MetadataProcessor] Successfully extracted metadata for {os.path.basename(op_path)}"
                            )
                    except Exception as e:
                        logging.error(
                            f"[MetadataProcessor] Error extracting metadata for {os.path.basename(op_path)}: {e}",
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

            all_metadata_results = []
            # ... (parallel execution logic as before, using paths_for_pyexiv2_extraction) ...
            with concurrent.futures.ThreadPoolExecutor(
                max_workers=MAX_WORKERS
            ) as executor:
                future_to_chunk = {
                    executor.submit(
                        process_chunk, paths_for_pyexiv2_extraction[i : i + CHUNK_SIZE]
                    ): paths_for_pyexiv2_extraction[i : i + CHUNK_SIZE]
                    for i in range(0, len(paths_for_pyexiv2_extraction), CHUNK_SIZE)
                }
                for future in concurrent.futures.as_completed(future_to_chunk):
                    # ... (error handling for future.result() as before) ...
                    try:
                        all_metadata_results.extend(future.result())
                    except Exception as exc:
                        chunk_paths_failed = future_to_chunk[future]
                        logging.error(
                            f"[MetadataProcessor] Chunk (first file: {os.path.basename(chunk_paths_failed[0]) if chunk_paths_failed else 'N/A'}) generated an exception: {exc}"
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
                            logging.error(
                                f"[MetadataProcessor] Could not find cache key for operational path: {op_path_processed}"
                            )
                    else:  # Should not happen if process_chunk always includes file_path
                        logging.warning(
                            "[MetadataProcessor] Metadata result dict missing 'file_path' key."
                        )

        final_results_for_caller: Dict[str, Dict[str, Any]] = {}
        for cache_key, data_dict in results.items():  # cache_key is NFC normalized
            filename_only = os.path.basename(
                cache_key
            )  # Basename of the cache key for logging
            parsed_rating, parsed_date = 0, None
            raw_metadata = data_dict["raw_metadata"]

            if (
                raw_metadata and "error" not in raw_metadata
            ):  # Check if metadata is valid
                # ... (parsing logic for rating, date from raw_metadata as before) ...
                rating_raw_val = raw_metadata.get("Xmp.xmp.Rating")  # etc.
                parsed_rating = _parse_rating(rating_raw_val)
                for date_tag in DATE_TAGS_PREFERENCE:
                    date_string = raw_metadata.get(date_tag)
                    if date_string:
                        dt_obj_val = _parse_exif_date(str(date_string))
                        if dt_obj_val:
                            parsed_date = dt_obj_val
                            break

            # Date fallbacks (applied whether raw_metadata was present or not, if date still None)
            if parsed_date is None:
                parsed_date = _parse_date_from_filename(
                    filename_only
                )  # Use basename of cache_key

            # Filesystem date fallback: requires an operational_path.
            # This is tricky here as we only have cache_key. For files that were resolved,
            # we'd need to retrieve their operational_path. For now, skip if only cache_key is available.
            # Or, if raw_metadata contains 'file_path' which is operational_path, use it.
            op_path_for_stat = raw_metadata.get("file_path") if raw_metadata else None
            if (
                parsed_date is None
                and op_path_for_stat
                and os.path.isfile(op_path_for_stat)
            ):
                try:
                    # ... (filesystem date logic using op_path_for_stat as before) ...
                    fs_timestamp: Optional[float] = None
                    stat_result = os.stat(op_path_for_stat)
                    # Prefer birthtime if available and seems valid, else mtime
                    if (
                        hasattr(stat_result, "st_birthtime")
                        and stat_result.st_birthtime > 0
                    ):
                        fs_timestamp = stat_result.st_birthtime
                    if (
                        fs_timestamp is None or fs_timestamp < 1000000
                    ):  # Heuristic for invalid birthtime values
                        fs_timestamp = stat_result.st_mtime
                    if fs_timestamp:
                        parsed_date = dt_parser.fromtimestamp(fs_timestamp).date()
                except Exception as e_fs:
                    logging.warning(
                        f"[MetadataProcessor] Filesystem date error for {filename_only} (op_path: {op_path_for_stat}): {e_fs}"
                    )

            final_results_for_caller[cache_key] = {
                "rating": parsed_rating,
                "date": parsed_date,
            }
            logging.debug(
                f"[MetadataProcessor] Processed {filename_only}: R={parsed_rating}, D={parsed_date}"
            )

        duration = time.perf_counter() - start_time
        logging.info(
            f"[MetadataProcessor] Finished batch metadata for {len(image_paths)} files in {duration:.4f}s."
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
            logging.error(f"Invalid rating value '{rating}'. Must be an integer 0-5.")
            return False
        if not (0 <= rating_int <= 5):
            logging.error(f"Invalid rating value {rating_int}. Must be 0-5.")
            return False

        resolved = MetadataProcessor._resolve_path_forms(image_path)
        if not resolved:
            return False
        operational_path, cache_key_path = resolved

        success = False
        logging.info(
            f"[MetadataProcessor] Setting rating for {os.path.basename(operational_path)} to {rating_int}"
        )
        try:
            with pyexiv2.Image(operational_path, encoding="utf-8") as img:
                img.modify_xmp({"Xmp.xmp.Rating": str(rating_int)})
                success = True
        except Exception as e:
            logging.error(
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
    def check_availability() -> bool:
        """
        Checks if pyexiv2 is available and working.
        Tries to create a simple Image instance as a test.
        Returns True if pyexiv2 works, False otherwise.
        """
        try:
            pyexiv2.set_log_level(4)

            test_dir = os.path.dirname(__file__) if __file__ else "."
            test_path = os.path.join(test_dir, "test_availability_pyexiv2.jpg")
            try:
                with open(test_path, "wb") as f:
                    f.write(
                        b"\xff\xd8\xff\xe0\x00\x10JFIF\x00\x01\x01\x01\x00H\x00H\x00\x00\xff\xd9"
                    )
                with pyexiv2.Image(test_path, encoding="utf-8") as img:  # Add encoding
                    _ = img.get_mime_type()
                logging.info(
                    "[MetadataProcessor] pyexiv2 availability check successful."
                )
                return True
            except Exception as e_inner:
                logging.error(
                    f"[MetadataProcessor] pyexiv2 availability check: test file operation failed: {e_inner}",
                    exc_info=True,
                )
                return False
            finally:
                if os.path.exists(test_path):
                    try:
                        os.remove(test_path)
                    except OSError as e_rm:
                        logging.warning(
                            f"[MetadataProcessor] Could not remove test file {test_path}: {e_rm}"
                        )
        except ImportError:
            logging.error("[MetadataProcessor] pyexiv2 not installed (ImportError).")
            return False
        except Exception as e:  # Catch other pyexiv2 related errors
            logging.error(
                f"[MetadataProcessor] pyexiv2 availability check failed: {e}",
                exc_info=True,
            )
            return False

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

        logging.info(
            f"[MetadataProcessor] get_detailed_metadata for op_path: {os.path.basename(operational_path)} (cache_key: {os.path.basename(cache_key_path)})"
        )

        if exif_disk_cache:
            cached_data = exif_disk_cache.get(cache_key_path)
            if cached_data is not None:
                logging.info(
                    f"[MetadataProcessor] ExifCache HIT for detailed metadata: {os.path.basename(cache_key_path)}"
                )
                return cached_data
            else:
                logging.info(
                    f"[MetadataProcessor] ExifCache MISS for detailed metadata: {os.path.basename(cache_key_path)}"
                )

        try:
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
                    logging.debug(f"No EXIF for {os.path.basename(operational_path)}")
                try:
                    metadata.update(img.read_iptc() or {})
                except Exception:
                    logging.debug(f"No IPTC for {os.path.basename(operational_path)}")
                try:
                    metadata.update(img.read_xmp() or {})
                except Exception:
                    logging.debug(f"No XMP for {os.path.basename(operational_path)}")

                if exif_disk_cache:
                    exif_disk_cache.set(cache_key_path, metadata)
                return metadata
        except Exception as e:
            logging.error(
                f"[MetadataProcessor] Error fetching detailed metadata for {os.path.basename(operational_path)}: {e}",
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
                logging.info(
                    f"[MetadataProcessor] Rotation of '{os.path.basename(operational_path)}' reported: {message}"
                )
                if exif_disk_cache:
                    exif_disk_cache.delete(cache_key_path)
            else:
                logging.error(
                    f"[MetadataProcessor] Rotation failed for '{os.path.basename(operational_path)}': {message}"
                )
            return success
        except Exception as e:
            logging.error(
                f"[MetadataProcessor] Error rotating {os.path.basename(operational_path)}: {e}",
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
            error_msg = f"Error during metadata rotation attempt for {os.path.basename(operational_path)}: {e}"
            logging.error(f"[MetadataProcessor] {error_msg}", exc_info=True)
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
            logging.error(
                f"[MetadataProcessor] Error checking rotation support for {os.path.basename(operational_path)}: {e}",
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
            logging.error(
                f"Invalid EXIF orientation value: {orientation}. Must be between 1 and 8."
            )
            return False

        resolved = MetadataProcessor._resolve_path_forms(image_path)
        if not resolved:
            return False
        operational_path, cache_key_path = resolved

        try:
            with pyexiv2.Image(operational_path, encoding="utf-8") as img:
                img.modify_exif({"Exif.Image.Orientation": orientation})
                logging.info(
                    f"[MetadataProcessor] Set EXIF orientation for {os.path.basename(operational_path)} to {orientation}"
                )

            if exif_disk_cache:
                exif_disk_cache.delete(cache_key_path)
            return True
        except Exception as e:
            logging.error(
                f"[MetadataProcessor] Error setting EXIF orientation for {os.path.basename(operational_path)}: {e}",
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
        try:
            with pyexiv2.Image(operational_path, encoding="utf-8") as img:
                exif_data = img.read_exif()
                orientation = exif_data.get("Exif.Image.Orientation")
                if orientation:
                    return int(orientation)
                return None
        except Exception as e:
            logging.error(
                f"[MetadataProcessor] Error getting EXIF orientation for {os.path.basename(operational_path)}: {e}",
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
                        (
                            int(cached_data["Exif.Image.Orientation"])
                            if cached_data.get("Exif.Image.Orientation") is not None
                            else None
                        )
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
        try:
            with pyexiv2.Image(operational_path, encoding="utf-8") as img:
                orientation = img.read_exif().get("Exif.Image.Orientation")
                orientation = int(orientation) if orientation else None
                width = img.get_pixel_width()
                height = img.get_pixel_height()
                return orientation, width, height
        except Exception as e:
            logging.error(
                f"[MetadataProcessor] Error getting orientation/dimensions for {os.path.basename(operational_path)}: {e}",
                exc_info=True,
            )
            return None, None, None

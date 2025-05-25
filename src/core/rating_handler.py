import exiftool
from exiftool.exceptions import ExifToolExecuteError # Import the specific exception
import os
import re
import time
import logging
import unicodedata
from datetime import datetime as dt_parser, date as date_obj
from typing import Dict, Any, Optional, List

from src.core.caching.rating_cache import RatingCache
from src.core.caching.exif_cache import ExifCache
from src.core.app_settings import get_exiftool_executable_path # Added import

# Preferred EXIF/XMP date tags in order of preference
DATE_TAGS_PREFERENCE: List[str] = [
    'EXIF:DateTimeOriginal',
    'XMP:DateCreated',
    'EXIF:CreateDate',
    'QuickTime:CreateDate',
    'H264:DateTimeOriginal',
]

# Comprehensive EXIF tags for metadata extraction (used for both batch and detailed fetching)
COMPREHENSIVE_EXIF_TAGS: List[str] = list(dict.fromkeys([
    "SourceFile", "XMP:Rating", "XMP:Label", "XMP:Keywords", "FileSize", "ImageSize",
    "EXIF:Make", "EXIF:Model", "EXIF:LensModel", "EXIF:LensInfo",
    "EXIF:FocalLength", "EXIF:FNumber", "EXIF:ApertureValue",
    "EXIF:ShutterSpeedValue", "EXIF:ExposureTime", "EXIF:ISO", "EXIF:ISOSpeedRatings",
    "EXIF:Flash", "EXIF:ImageWidth", "EXIF:ImageHeight", "EXIF:ColorSpace",
    "EXIF:Orientation", "EXIF:BitsPerSample", "EXIF:ExposureCompensation",
    "EXIF:MeteringMode", "EXIF:WhiteBalance", "EXIF:GPSLatitude", "EXIF:GPSLongitude",
    # Add some alternative tag names that might be used
    "Make", "Model", "LensModel", "FocalLength", "FNumber", "ExposureTime", "ISO"
] + DATE_TAGS_PREFERENCE))

# Log the tags being used for debugging
logging.info(f"[MetadataHandler] COMPREHENSIVE_EXIF_TAGS defined with {len(COMPREHENSIVE_EXIF_TAGS)} tags: {COMPREHENSIVE_EXIF_TAGS}")

def _parse_exif_date(date_string: str) -> Optional[date_obj]:
    """
    Attempts to parse various EXIF/XMP date string formats.
    Returns a datetime.date object or None.
    """
    if not date_string or not isinstance(date_string, str):
        return None

    date_string = date_string.strip()
    date_string_no_frac = date_string.split('.')[0]
    date_string_no_tz = date_string_no_frac
    if 'Z' in date_string_no_tz:
        date_string_no_tz = date_string_no_tz.split('Z')[0]
    tz_match = re.search(r'[+-](\d{2}:?\d{2})$', date_string_no_tz)
    if tz_match:
        date_string_no_tz = date_string_no_tz[:tz_match.start()]

    formats_to_try = [
        "%Y:%m:%d %H:%M:%S", "%Y-%m-%d %H:%M:%S", "%Y/%m/%d %H:%M:%S",
        "%Y.%m.%d %H:%M:%S", "%Y-%m-%dT%H:%M:%S", "%Y:%m:%d",
        "%Y-%m-%d", "%Y/%m/%d", "%Y.%m.%d",
    ]

    parsed_date = None
    for fmt in formats_to_try:
        try:
            string_to_parse = date_string_no_tz
            if 'T' not in fmt and ' ' not in fmt:
                 if 'T' in string_to_parse: string_to_parse = string_to_parse.split('T')[0]
                 elif ' ' in string_to_parse: string_to_parse = string_to_parse.split(' ')[0]
            parsed_date = dt_parser.strptime(string_to_parse, fmt).date()
            break
        except (ValueError, Exception):
            continue
    return parsed_date

def _parse_date_from_filename(filename: str) -> Optional[date_obj]:
    """
    Attempts to parse a date (YYYY, MM, DD) from common filename patterns.
    Returns a datetime.date object or None.
    """
    match1 = re.search(r'(\d{4})(\d{2})(\d{2})(?:[_ \-T]|$)', filename)
    match2 = re.search(r'(\d{4})[-_\.](\d{2})[-_\.](\d{2})', filename)
    year, month, day = None, None, None

    def validate_and_assign(y_str, m_str, d_str):
        nonlocal year, month, day
        try:
            y, m, d = int(y_str), int(m_str), int(d_str)
            if 1900 <= y <= dt_parser.now().year + 5 and 1 <= m <= 12 and 1 <= d <= 31:
                 dt_parser(y, m, d)
                 year, month, day = y, m, d
                 return True
        except (ValueError, IndexError):
            pass
        return False

    if match1 and validate_and_assign(match1.group(1), match1.group(2), match1.group(3)):
        pass
    elif match2 and validate_and_assign(match2.group(1), match2.group(2), match2.group(3)):
        pass
    
    if year and month and day:
        try:
            return date_obj(year, month, day)
        except ValueError:
            return None
    return None

def _parse_rating(value: Any) -> int:
    """
    Safely converts a metadata rating value to an integer between 0 and 5.
    """
    if value is None:
        return 0
    try:
        rating_val = int(float(str(value)))
        return max(0, min(5, rating_val))
    except (ValueError, TypeError):
        return 0

class MetadataHandler:
    """
    Handles reading and writing XMP metadata (ratings, labels, dates)
    using the ExifToolHelper context manager.
    """

    @staticmethod
    def _get_exiftool_helper_instance() -> exiftool.ExifToolHelper:
        """Creates an ExifToolHelper instance, using configured executable path if available."""
        executable_path = get_exiftool_executable_path()
        common_args = ["-charset", "UTF8"] # Common args for all instances
        encoding = "utf-8"

        if executable_path and os.path.isfile(executable_path):
            logging.info(f"[MetadataHandler] Using ExifTool executable from settings: {executable_path}")
            return exiftool.ExifToolHelper(executable=executable_path, common_args=common_args, encoding=encoding)
        else:
            if executable_path: # Path was set but not valid
                logging.warning(f"[MetadataHandler] ExifTool path from settings ('{executable_path}') is not a valid file. Falling back to PATH.")
            else: # Path not set in settings
                logging.info("[MetadataHandler] ExifTool executable path not set in settings. Using ExifTool from system PATH.")
            return exiftool.ExifToolHelper(common_args=common_args, encoding=encoding)

    @staticmethod
    def get_batch_display_metadata(
        image_paths: List[str],
        rating_disk_cache: Optional[RatingCache] = None,
        exif_disk_cache: Optional[ExifCache] = None
    ) -> Dict[str, Dict[str, Any]]:
        """
        Fetches and parses essential metadata for a batch of images.
        Uses ExifCache first, then ExifTool in batch for remaining files.
        Populates caches and applies date fallbacks.
        """
        results: Dict[str, Dict[str, Any]] = {}
        paths_needing_exiftool: List[str] = []
        start_time = time.perf_counter()
        logging.info(f"[MetadataHandler] Starting batch metadata for {len(image_paths)} files.")

        # 1. Check ExifCache for each file and prepare list for ExifTool
        for image_path in image_paths:
            norm_path = unicodedata.normalize('NFC', os.path.normpath(image_path))
            results[norm_path] = {'rating': 0, 'label': None, 'date': None, 'raw_exif': None} # Store raw_exif temp

            if not os.path.isfile(norm_path):
                logging.warning(f"[MetadataHandler] File not found: {norm_path}")
                results[norm_path]['date'] = _parse_date_from_filename(os.path.basename(norm_path))
                continue

            cached_exif_data: Optional[Dict[str, Any]] = None
            if exif_disk_cache:
                cached_exif_data = exif_disk_cache.get(norm_path)
            
            if cached_exif_data:
                logging.debug(f"[MetadataHandler] ExifCache HIT for {os.path.basename(norm_path)}.")
                results[norm_path]['raw_exif'] = cached_exif_data
            else:
                logging.debug(f"[MetadataHandler] ExifCache MISS for {os.path.basename(norm_path)}")
                paths_needing_exiftool.append(norm_path)
        
        # 2. Parallel Batch ExifTool call for files not in ExifCache
        CHUNK_SIZE = 50 # Smaller chunk size for parallel processing to avoid too many concurrent exiftool processes
        MAX_WORKERS = min(8, (os.cpu_count() or 1) * 2) # Adjust max workers based on CPU, up to a limit

        if paths_needing_exiftool:
            logging.info(f"[MetadataHandler] Need to call ExifTool for {len(paths_needing_exiftool)} files. Processing in parallel chunks of {CHUNK_SIZE} with up to {MAX_WORKERS} workers.")
            
            all_chunk_exiftool_results: List[List[Dict[str, Any]]] = []

            def process_chunk(chunk_paths_to_process: List[str]) -> List[Dict[str, Any]]:
                chunk_results_list = []
                try:
                    # Use the helper method to get ExifToolHelper instance
                    with MetadataHandler._get_exiftool_helper_instance() as et:
                        encoded_chunk_paths = [p.encode('utf-8', errors='surrogateescape') for p in chunk_paths_to_process]
                        # et.get_tags can return List[Dict[str, Any]]
                        chunk_results_list = et.get_tags(encoded_chunk_paths, tags=COMPREHENSIVE_EXIF_TAGS)
                        logging.info(f"[MetadataHandler Worker] ExifTool chunk ({len(chunk_paths_to_process)} files) returned {len(chunk_results_list)} results.")
                except exiftool.ExifToolExecuteError as ete_thread:
                    logging.error(f"[MetadataHandler Worker] ExifTool execution error on chunk: {ete_thread}")
                except Exception as e_thread:
                    logging.error(f"[MetadataHandler Worker] Error during ExifTool chunk processing: {e_thread}", exc_info=True)
                return chunk_results_list or [] # Ensure it always returns a list

            import concurrent.futures
            with concurrent.futures.ThreadPoolExecutor(max_workers=MAX_WORKERS) as executor:
                future_to_chunk = {
                    executor.submit(process_chunk, paths_needing_exiftool[i:i + CHUNK_SIZE]): paths_needing_exiftool[i:i + CHUNK_SIZE]
                    for i in range(0, len(paths_needing_exiftool), CHUNK_SIZE)
                }
                for future in concurrent.futures.as_completed(future_to_chunk):
                    chunk_data = future_to_chunk[future]
                    try:
                        chunk_exif_results = future.result() # This is List[Dict[str, Any]]
                        all_chunk_exiftool_results.extend(chunk_exif_results)
                    except Exception as exc:
                        logging.error(f"[MetadataHandler] Chunk (first file: {os.path.basename(chunk_data[0]) if chunk_data else 'N/A'}) generated an exception: {exc}")
            
            # Process aggregated results
            if all_chunk_exiftool_results:
                sourcefile_to_meta_map_aggregated = {}
                for meta_dict in all_chunk_exiftool_results:
                    source_file_raw = meta_dict.get("SourceFile")
                    if source_file_raw:
                        norm_source_file = unicodedata.normalize('NFC', os.path.normpath(source_file_raw))
                        sourcefile_to_meta_map_aggregated[norm_source_file] = meta_dict
                    else:
                        logging.warning(f"[MetadataHandler] Aggregated ExifTool result missing SourceFile: {meta_dict}")

                for path_processed_by_exiftool in paths_needing_exiftool: # Iterate over original list of paths needing exiftool
                    raw_meta = sourcefile_to_meta_map_aggregated.get(path_processed_by_exiftool)
                    if raw_meta:
                        # logging.debug(f"[MetadataHandler] ExifTool data for {os.path.basename(path_processed_by_exiftool)}: {raw_meta}")
                        results[path_processed_by_exiftool]['raw_exif'] = raw_meta
                        if exif_disk_cache:
                            exif_disk_cache.set(path_processed_by_exiftool, raw_meta)
                    else:
                        # This path might have failed in its chunk or not returned by exiftool
                        logging.warning(f"[MetadataHandler] No ExifTool result mapped for {os.path.basename(path_processed_by_exiftool)} after parallel processing.")
                        results[path_processed_by_exiftool]['raw_exif'] = None # Ensure it's None if processing failed for it

        # 3. Parse data from raw_exif (either from cache or new ExifTool call) and apply fallbacks
        final_results: Dict[str, Dict[str, Any]] = {}
        for norm_path, data_dict in results.items():
            filename_only = os.path.basename(norm_path)
            parsed_rating = 0
            parsed_label = None
            parsed_date = None
            
            raw_exif_data = data_dict['raw_exif']

            if raw_exif_data:
                # Parse Rating - Try "XMP:Rating" first, then "Rating" as a fallback
                rating_raw_val = raw_exif_data.get("XMP:Rating")
                log_source_tag = "XMP:Rating"
                if rating_raw_val is None: # If XMP:Rating is not found or is None
                    rating_raw_val = raw_exif_data.get("Rating") # Try the general 'Rating' tag
                    log_source_tag = "Rating" if rating_raw_val is not None else "XMP:Rating (None)"

                logging.info(f"[MetadataHandler] Raw rating value from '{log_source_tag}' for {filename_only}: '{rating_raw_val}' (type: {type(rating_raw_val)})")
                parsed_rating = _parse_rating(rating_raw_val)
                logging.info(f"[MetadataHandler] Parsed rating for {filename_only}: {parsed_rating}")
                
                if rating_disk_cache: # Keep rating_disk_cache consistent
                    cached_rating_val = rating_disk_cache.get(norm_path)
                    if cached_rating_val is None or cached_rating_val != parsed_rating:
                         logging.info(f"[MetadataHandler] Updating rating_disk_cache for {filename_only} from {cached_rating_val} to {parsed_rating}")
                         rating_disk_cache.set(norm_path, parsed_rating)
                
                # Parse Label
                label_raw_val = raw_exif_data.get("XMP:Label")
                parsed_label = str(label_raw_val) if label_raw_val is not None else None
                
                # Parse Date from metadata
                for date_tag in DATE_TAGS_PREFERENCE:
                    date_string = raw_exif_data.get(date_tag)
                    if date_string:
                        dt_obj_val = _parse_exif_date(str(date_string))
                        if dt_obj_val:
                            parsed_date = dt_obj_val
                            break
            
            # Date fallbacks
            if parsed_date is None:
                parsed_date = _parse_date_from_filename(filename_only)
            
            if parsed_date is None and os.path.isfile(norm_path): # Filesystem time fallback
                try:
                    fs_timestamp: Optional[float] = None
                    stat_result = os.stat(norm_path)
                    if hasattr(stat_result, 'st_birthtime'): fs_timestamp = stat_result.st_birthtime
                    if fs_timestamp is None or fs_timestamp < 1000000: fs_timestamp = stat_result.st_mtime
                    if fs_timestamp: parsed_date = dt_parser.fromtimestamp(fs_timestamp).date()
                except Exception as e_fs:
                    logging.warning(f"[MetadataHandler] Filesystem date error for {filename_only}: {e_fs}")

            final_results[norm_path] = {'rating': parsed_rating, 'label': parsed_label, 'date': parsed_date}
            logging.debug(f"[MetadataHandler] Processed {filename_only}: R={parsed_rating}, L='{parsed_label}', D={parsed_date}")

        duration = time.perf_counter() - start_time
        logging.info(f"[MetadataHandler] Finished batch metadata for {len(image_paths)} files in {duration:.4f}s.")
        return final_results

    @staticmethod
    def set_rating(image_path: str, rating: int,
                   rating_disk_cache: Optional[RatingCache] = None,
                   exif_disk_cache: Optional[ExifCache] = None) -> bool:
        """
        Sets the rating (0-5) using ExifToolHelper.
        Updates rating_disk_cache and invalidates exif_disk_cache if provided.
        Returns True on apparent success, False on failure.
        """
        if not (0 <= rating <= 5):
            logging.error(f"Invalid rating value {rating}. Must be 0-5.")
            return False
        if not os.path.isfile(image_path):
            logging.error(f"File not found when setting rating: {image_path}")
            return False
            
        norm_path = unicodedata.normalize('NFC', os.path.normpath(image_path))
        exif_success = False
        logging.info(f"[MetadataHandler] Setting rating for {os.path.basename(norm_path)} to {rating}")
        try:
            rating_str = str(rating)
            param = f"-XMP:Rating={rating_str}".encode('utf-8')
            filename_bytes = norm_path.encode('utf-8', errors='surrogateescape')

            # Use the helper method to get ExifToolHelper instance
            with MetadataHandler._get_exiftool_helper_instance() as et:
                et.execute(param, b"-overwrite_original", filename_bytes)
                exif_success = True
                logging.info(f"[MetadataHandler] ExifTool successfully set rating for {os.path.basename(norm_path)}")
        except Exception as e:
            logging.error(f"Error setting rating for {os.path.basename(norm_path)}: {e}", exc_info=True)
            exif_success = False
        
        if exif_success:
            if rating_disk_cache:
                logging.info(f"[MetadataHandler] Updating rating_disk_cache for {os.path.basename(norm_path)} to {rating}")
                rating_disk_cache.set(norm_path, rating)
            if exif_disk_cache:
                logging.info(f"[MetadataHandler] Deleting from exif_disk_cache for {os.path.basename(norm_path)} due to rating change.")
                exif_disk_cache.delete(norm_path) # Invalidate specific entry
        return exif_success

    @staticmethod
    def set_label(image_path: str, label: Optional[str], exif_disk_cache: Optional[ExifCache] = None) -> bool:
        """
        Sets the XMP:Label using ExifToolHelper.
        An empty string or None removes the label.
        Invalidates exif_disk_cache if provided.
        Returns True on apparent success, False on failure.
        """
        if not os.path.isfile(image_path):
            logging.error(f"File not found when setting label: {image_path}")
            return False

        norm_path = unicodedata.normalize('NFC', os.path.normpath(image_path))
        success = False
        try:
            label_str = label if label else "" 
            param = f"-XMP:Label={label_str}".encode('utf-8')
            filename_bytes = norm_path.encode('utf-8', errors='surrogateescape')

            # Use the helper method to get ExifToolHelper instance
            with MetadataHandler._get_exiftool_helper_instance() as et:
                et.execute(param, b"-overwrite_original", filename_bytes)
                success = True
        except Exception as e:
            logging.error(f"Error setting label for {os.path.basename(norm_path)}: {e}", exc_info=True)
            success = False
        
        if success and exif_disk_cache:
            exif_disk_cache.delete(norm_path) # Invalidate specific entry
        return success

    @staticmethod
    def check_exiftool_availability() -> bool:
        """
        Checks if ExifTool is available and executable.
        Tries to get its version as a simple test.
        Returns True if ExifTool is found and works, False otherwise.
        """
        try:
            with MetadataHandler._get_exiftool_helper_instance() as et:
                # Execute a simple command like -ver to check if exiftool is running
                version_output = et.execute(b"-ver") # Renamed variable
                if version_output:
                    version_output_str = ""
                    if isinstance(version_output, bytes):
                        version_output_str = version_output.decode('utf-8', errors='ignore').strip()
                    elif isinstance(version_output, str):
                        version_output_str = version_output.strip()
                    else:
                        logging.warning(f"[MetadataHandler] ExifTool -ver returned unexpected type: {type(version_output)}")
                        return False

                    if re.match(r"^\d+\.\d+$", version_output_str): # Check if output looks like a version number
                        logging.info(f"[MetadataHandler] ExifTool check successful. Version: {version_output_str}")
                        return True
                    else:
                        logging.warning(f"[MetadataHandler] ExifTool check: -ver command returned unexpected output: {version_output_str}")
                        return False
                else:
                    logging.warning("[MetadataHandler] ExifTool check: -ver command returned no output.")
                    return False
        except ExifToolExecuteError as e: # Use the imported exception
            logging.error(f"[MetadataHandler] ExifTool availability check failed: ExifToolExecuteError - {e}. "
                          "This usually means ExifTool was not found in PATH or the configured path is incorrect/not executable.")
            return False
        except FileNotFoundError: # This will be raised by ExifToolHelper if executable is not found
            logging.error("[MetadataHandler] ExifTool availability check failed: FileNotFoundError. ExifTool executable not found (either in PATH or specified location).")
            return False
        except Exception as e:
            logging.error(f"[MetadataHandler] ExifTool availability check failed with an unexpected error: {e}", exc_info=True)
            return False

    @staticmethod
    def get_detailed_metadata(image_path: str, exif_disk_cache: Optional[ExifCache] = None) -> Optional[Dict[str, Any]]:
        """
        Fetches detailed metadata for a single image for sidebar display.
        Since batch loading now fetches all detailed metadata, this should mostly be cache hits.
        """
        if not os.path.isfile(image_path):
            logging.warning(f"[MetadataHandler] File not found for detailed metadata: {image_path}")
            return None

        norm_path = unicodedata.normalize('NFC', os.path.normpath(image_path))
        logging.info(f"[MetadataHandler] get_detailed_metadata called for: {os.path.basename(norm_path)}")
        
        # Try cache first - should usually hit since batch loading fetches detailed metadata
        if exif_disk_cache:
            cached_data = exif_disk_cache.get(norm_path)
            if cached_data:
                logging.info(f"[MetadataHandler] ExifCache HIT for detailed metadata: {os.path.basename(norm_path)}")
                logging.info(f"[MetadataHandler] Cached data has {len(cached_data)} keys: {list(cached_data.keys())}")
                return cached_data
            else:
                logging.info(f"[MetadataHandler] ExifCache MISS for detailed metadata: {os.path.basename(norm_path)}")
        
        # Fallback: fetch with ExifTool if not in cache (shouldn't happen often now)
        logging.warning(f"[MetadataHandler] Cache miss - fetching detailed metadata on-demand for: {os.path.basename(norm_path)}")
        
        try:
            with MetadataHandler._get_exiftool_helper_instance() as et:
                encoded_path = norm_path.encode('utf-8', errors='surrogateescape')
                
                # First try with comprehensive tags, if that fails, try with basic tags
                try:
                    result = et.get_tags([encoded_path], tags=COMPREHENSIVE_EXIF_TAGS)
                except ExifToolExecuteError as ete:
                    logging.warning(f"[MetadataHandler] Comprehensive EXIF fetch failed for {os.path.basename(norm_path)}, trying basic tags: {ete}")
                    # Fallback to basic tags that are more likely to work
                    basic_tags = ["SourceFile", "XMP:Rating", "XMP:Label", "XMP:Keywords", "FileSize",
                                "EXIF:Make", "EXIF:Model", "EXIF:ImageWidth", "EXIF:ImageHeight"] + DATE_TAGS_PREFERENCE
                    result = et.get_tags([encoded_path], tags=basic_tags)
                
                if result and len(result) > 0:
                    metadata = result[0]
                    logging.info(f"[MetadataHandler] On-demand fetch: Got {len(metadata)} keys")
                    
                    # Cache the result
                    if exif_disk_cache:
                        exif_disk_cache.set(norm_path, metadata)
                    
                    return metadata
                else:
                    logging.warning(f"[MetadataHandler] No metadata returned for: {os.path.basename(norm_path)}")
                    return {}  # Return empty dict instead of None so sidebar can still show file info
                    
        except Exception as e:
            logging.error(f"[MetadataHandler] Error fetching detailed metadata for {os.path.basename(norm_path)}: {e}", exc_info=True)
            return {}  # Return empty dict instead of None so sidebar can still show file info
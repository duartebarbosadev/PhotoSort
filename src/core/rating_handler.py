import exiftool
import os
import re
import time # Add time import
from datetime import datetime as dt_parser, date as date_obj
from typing import Dict, Any, Optional, List

from src.core.caching.rating_cache import RatingCache
from src.core.caching.exif_cache import ExifCache # Import ExifCache

# Preferred EXIF/XMP date tags in order of preference
DATE_TAGS_PREFERENCE: List[str] = [
    'EXIF:DateTimeOriginal',
    'XMP:DateCreated',
    'EXIF:CreateDate', # Often same as DateTimeOriginal, but good fallback
    'QuickTime:CreateDate', # For MOV files etc.
    'H264:DateTimeOriginal', # For some video formats
]

def _parse_exif_date(date_string: str) -> Optional[date_obj]:
    """
    Attempts to parse various EXIF/XMP date string formats.
    Returns a datetime.date object or None.
    """
    if not date_string or not isinstance(date_string, str):
        return None

    date_string = date_string.strip()
    # Handle potential fractional seconds by splitting before parsing
    date_string_no_frac = date_string.split('.')[0]

    # Handle potential timezone info (simple removal for now)
    date_string_no_tz = date_string_no_frac
    if 'Z' in date_string_no_tz: # UTC Zulu time
        date_string_no_tz = date_string_no_tz.split('Z')[0]
    # Check for timezone offsets like +HH:MM or -HH:MM
    # Regex to find HH:MM or HHMM at the end of string preceded by + or -
    tz_match = re.search(r'[+-](\d{2}:?\d{2})$', date_string_no_tz)
    if tz_match:
        date_string_no_tz = date_string_no_tz[:tz_match.start()]

    formats_to_try = [
        "%Y:%m:%d %H:%M:%S",  # EXIF standard
        "%Y-%m-%d %H:%M:%S",  # ISO-like XMP
        "%Y/%m/%d %H:%M:%S",
        "%Y.%m.%d %H:%M:%S",
        "%Y-%m-%dT%H:%M:%S",  # ISO 8601 without TZ
        "%Y:%m:%d",           # Date only
        "%Y-%m-%d",           # Date only
        "%Y/%m/%d",           # Date only
        "%Y.%m.%d",           # Date only
    ]

    parsed_date = None
    for fmt in formats_to_try:
        try:
            # Attempt to parse the string that has had timezone and fractions stripped
            # Ensure the string is long enough for the format
            # This is a basic check; more robust would be to try parsing and catch ValueError
            # For date-only formats, only the date part of string_to_try matters.
            string_to_parse = date_string_no_tz
            if 'T' not in fmt and ' ' not in fmt: # Date-only format
                 if 'T' in string_to_parse: string_to_parse = string_to_parse.split('T')[0]
                 elif ' ' in string_to_parse: string_to_parse = string_to_parse.split(' ')[0]
            
            parsed_date = dt_parser.strptime(string_to_parse, fmt).date()
            break 
        except ValueError:
            continue
        except Exception: # Catch other potential errors during parsing
            continue
    return parsed_date

def _parse_date_from_filename(filename: str) -> Optional[date_obj]:
    """
    Attempts to parse a date (YYYY, MM, DD) from common filename patterns.
    Returns a datetime.date object or None.
    """
    # Pattern 1: YYYYMMDD (possibly followed by _, -, T, or space, or end of string)
    match1 = re.search(r'(\d{4})(\d{2})(\d{2})(?:[_ \-T]|$)', filename)
    # Pattern 2: YYYY-MM-DD or YYYY_MM_DD or YYYY.MM.DD
    match2 = re.search(r'(\d{4})[-_\.](\d{2})[-_\.](\d{2})', filename)

    year, month, day = None, None, None

    def validate_and_assign(y_str, m_str, d_str):
        nonlocal year, month, day
        try:
            y, m, d = int(y_str), int(m_str), int(d_str)
            # Basic validation for sensibility
            if 1900 <= y <= dt_parser.now().year + 5 and 1 <= m <= 12 and 1 <= d <= 31:
                 dt_parser(y, m, d) # Will raise ValueError if invalid date like Feb 30
                 year, month, day = y, m, d
                 return True
        except (ValueError, IndexError):
            pass
        return False

    if match1:
        if validate_and_assign(match1.group(1), match1.group(2), match1.group(3)):
            pass # Date assigned
    
    if year is None and match2: # Only try pattern 2 if pattern 1 failed
         if validate_and_assign(match2.group(1), match2.group(2), match2.group(3)):
            pass # Date assigned
    
    if year and month and day:
        try:
            return date_obj(year, month, day)
        except ValueError: # Handles cases like Feb 30 that passed initial checks
            return None
    return None

def _parse_rating(value: Any) -> int:
    """
    Safely converts a metadata rating value to an integer between 0 and 5.
    """
    if value is None:
        return 0
    try:
        # Handle cases where rating might be a float string like "3.0" or just "3"
        rating_val = int(float(str(value))) 
        return max(0, min(5, rating_val)) # Clamp between 0 and 5
    except (ValueError, TypeError):
        return 0

class MetadataHandler:
    """
    Handles reading and writing XMP metadata (ratings, labels, dates)
    using the ExifToolHelper context manager.
    """

    @staticmethod
    def get_display_metadata(image_path: str,
                             rating_disk_cache: Optional[RatingCache] = None,
                             exif_disk_cache: Optional[ExifCache] = None) -> Dict[str, Any]:
        """
        Fetches and parses essential metadata (rating, label, date) for display.
        Uses exif_disk_cache first, then rating_disk_cache, then ExifTool.
        Includes fallbacks for date.
        """
        result: Dict[str, Any] = {'rating': 0, 'label': None, 'date': None}
        filename_only = os.path.basename(image_path)
        exif_called = False
        overall_start_time_mdh = time.perf_counter()

        if not os.path.isfile(image_path):
            print(f"[MetadataHandler] File not found: {image_path}")
            result['date'] = _parse_date_from_filename(filename_only)
            return result

        # 1. Try to get full metadata from ExifCache
        cached_exif_data: Optional[Dict[str, Any]] = None
        if exif_disk_cache:
            exif_cache_start_time = time.perf_counter()
            cached_exif_data = exif_disk_cache.get(image_path)
            exif_cache_duration = time.perf_counter() - exif_cache_start_time
            if cached_exif_data:
                print(f"[PERF][MetadataHandler] ExifCache HIT for {filename_only} in {exif_cache_duration:.4f}s")
                # Parse rating, label, date from this cached_exif_data
                rating_raw = cached_exif_data.get("XMP:Rating")
                result['rating'] = _parse_rating(rating_raw)
                
                # Ensure rating_disk_cache is consistent if EXIF cache had a rating
                if rating_disk_cache and rating_disk_cache.get(image_path) != result['rating']:
                    rating_disk_cache.set(image_path, result['rating'])

                label_raw = cached_exif_data.get("XMP:Label")
                result['label'] = str(label_raw) if label_raw is not None else None

                extracted_date_from_meta = None
                for date_tag in DATE_TAGS_PREFERENCE:
                    date_string = cached_exif_data.get(date_tag)
                    if date_string:
                        parsed_dt = _parse_exif_date(str(date_string))
                        if parsed_dt:
                            extracted_date_from_meta = parsed_dt
                            break
                result['date'] = extracted_date_from_meta
                # If all data is found from exif_cache, no need to proceed to exiftool or rating_cache for individual items
                # Date fallback will still apply if date wasn't in cached_exif_data
                if result['date'] is None: # (exif_called and result['date'] is None) or not exif_called:
                    # Date fallbacks below will handle this
                    pass
                else: # Date was found in EXIF cache. If rating and label are also present, we might be done.
                    # If rating was found, no need to check rating_disk_cache again.
                    # Fallbacks will still be checked if any essential part is missing.
                    pass # Fallback logic will handle if parts are missing.
            else: # ExifCache MISS
                if exif_disk_cache: # Ensure exif_disk_cache was checked
                     print(f"[PERF][MetadataHandler] ExifCache MISS for {filename_only} in {exif_cache_duration:.4f}s")

        # 2. If full EXIF not in ExifCache, try to get rating from RatingCache
        # This section is now secondary to checking ExifCache first.
        # If cached_exif_data was found, result['rating'] is already set.
        if cached_exif_data is None and rating_disk_cache:
            rating_cache_start_time = time.perf_counter()
            cached_rating_val = rating_disk_cache.get(image_path)
            rating_cache_duration = time.perf_counter() - rating_cache_start_time
            if cached_rating_val is not None:
                result['rating'] = cached_rating_val
                print(f"[PERF][MetadataHandler] RatingCache HIT for {filename_only} in {rating_cache_duration:.4f}s. Rating: {cached_rating_val}")
            else:
                print(f"[PERF][MetadataHandler] RatingCache MISS for {filename_only} in {rating_cache_duration:.4f}s")


        # 3. If data (especially if full EXIF wasn't cached) is still needed, call ExifTool
        # We call ExifTool if cached_exif_data is None.
        # If cached_exif_data was present, we assume it's sufficient unless parts were missing (e.g. date).
        # The check for 'tags_to_fetch' ensures we only call exiftool if necessary.
        
        # Only proceed to ExifTool if full EXIF was not in exif_disk_cache
        if cached_exif_data is None:
            exiftool_call_required = False
            # Determine if ExifTool call is truly needed based on what's still missing
            if result['rating'] == 0 and (not rating_disk_cache or rating_disk_cache.get(image_path) is None):
                exiftool_call_required = True
            if result['label'] is None:
                exiftool_call_required = True
            if result['date'] is None: # If date is still missing after potential cache hits
                exiftool_call_required = True

            if exiftool_call_required:
                exiftool_start_time = time.perf_counter()
                try:
                    # Fetch all potentially useful tags if we are calling exiftool anyway, then cache them all.
                    all_relevant_tags = list(dict.fromkeys(["XMP:Rating", "XMP:Label"] + DATE_TAGS_PREFERENCE))
                    
                    print(f"[PERF][MetadataHandler] ExifTool CALL for {filename_only}. Tags: {all_relevant_tags}")
                    with exiftool.ExifToolHelper() as et:
                        filename_bytes = image_path.encode('utf-8', errors='surrogateescape')
                        metadata_list = et.get_tags(filename_bytes, tags=all_relevant_tags)
                    exif_called = True

                    if metadata_list:
                        raw_exif_data_from_tool = metadata_list[0]

                        # Store all fetched raw metadata in ExifCache
                        if exif_disk_cache:
                            exif_cache_set_start = time.perf_counter()
                            exif_disk_cache.set(image_path, raw_exif_data_from_tool)
                            exif_cache_set_duration = time.perf_counter() - exif_cache_set_start
                            print(f"[PERF][MetadataHandler] ExifCache SET for {filename_only} in {exif_cache_set_duration:.4f}s")

                        # Parse rating if it wasn't already set from rating_disk_cache
                        if "XMP:Rating" in raw_exif_data_from_tool:
                            rating_raw = raw_exif_data_from_tool.get("XMP:Rating")
                            parsed_rating = _parse_rating(rating_raw)
                            if result['rating'] != parsed_rating : # If ExifTool gives a different rating
                                result['rating'] = parsed_rating
                                if rating_disk_cache: # Update rating_disk_cache too
                                    rating_disk_cache.set(image_path, result['rating'])
                        
                        # Parse Label if not already set
                        if result['label'] is None and "XMP:Label" in raw_exif_data_from_tool:
                            label_raw = raw_exif_data_from_tool.get("XMP:Label")
                            result['label'] = str(label_raw) if label_raw is not None else None

                        # Parse Date from metadata if not already set
                        if result['date'] is None:
                            extracted_date_from_meta = None
                            for date_tag in DATE_TAGS_PREFERENCE:
                                date_string = raw_exif_data_from_tool.get(date_tag)
                                if date_string:
                                    parsed_dt = _parse_exif_date(str(date_string))
                                    if parsed_dt:
                                        extracted_date_from_meta = parsed_dt
                                        break
                            result['date'] = extracted_date_from_meta
                except FileNotFoundError:
                    print(f"[MetadataHandler] File disappeared during EXIF metadata fetch: {image_path}")
                except exiftool.ExifToolExecuteError as ete:
                    print(f"[MetadataHandler] ExifTool execution error for {filename_only}: {ete}")
                except Exception as e:
                    print(f"[MetadataHandler] Error fetching EXIF metadata for {filename_only}: {e} (Type: {type(e).__name__})")
                finally:
                    exiftool_duration = time.perf_counter() - exiftool_start_time
                    if exif_called: # Only print duration if call was actually made
                         print(f"[PERF][MetadataHandler] ExifTool actual CALL for {filename_only} completed in {exiftool_duration:.4f}s")


        # Date fallbacks if not found in metadata (from any cache or ExifTool)
        if result['date'] is None:
            # print(f"[MetadataHandler] Date not found in EXIF for {filename_only}, trying fallbacks.")
            parsed_from_filename = _parse_date_from_filename(filename_only)
            if parsed_from_filename:
                result['date'] = parsed_from_filename
            elif os.path.isfile(image_path): # Only try filesystem if file still exists
                # Filesystem time fallback
                try:
                    # Prefer modification time (st_mtime) as it's more universally available
                    # and often reflects image content change/creation more reliably than st_ctime on Unix.
                    # st_birthtime is ideal but not on all platforms/filesystems.
                    fs_timestamp: Optional[float] = None
                    stat_result = os.stat(image_path)
                    
                    if hasattr(stat_result, 'st_birthtime'): # macOS, some BSDs
                        fs_timestamp = stat_result.st_birthtime
                    
                    # If birthtime is not available or seems invalid (e.g., epoch 0), use modification time.
                    # A very small timestamp might indicate an invalid or uninitialized birthtime.
                    if fs_timestamp is None or fs_timestamp < 1000000: # Heuristic for old/invalid birthtime
                        fs_timestamp = stat_result.st_mtime
                    
                    if fs_timestamp:
                        result['date'] = dt_parser.fromtimestamp(fs_timestamp).date()

                except FileNotFoundError: # Should ideally not happen if os.path.isfile check passed
                    print(f"[MetadataHandler] File disappeared for filesystem date fallback: {image_path}")
                except Exception as e_fs:
                    print(f"[MetadataHandler] Warning: Could not get filesystem time for {filename_only}: {e_fs}")
        
        overall_duration_mdh = time.perf_counter() - overall_start_time_mdh
        print(f"[PERF][MetadataHandler] get_display_metadata for {filename_only} took {overall_duration_mdh:.4f}s. EXIF called: {exif_called}")
        return result

    @staticmethod
    def set_rating(image_path: str, rating: int,
                   rating_disk_cache: Optional[RatingCache] = None,
                   exif_disk_cache: Optional[ExifCache] = None) -> bool:
        """
        Sets the rating (0-5) using ExifToolHelper.
        Updates rating_disk_cache and invalidates exif_disk_cache if provided.
        Rating 0 means "no stars".
        Returns True on apparent success, False on failure.
        """
        if not (0 <= rating <= 5):
            print(f"Error: Invalid rating value {rating}. Must be 0-5.")
            return False

        if not os.path.isfile(image_path):
            print(f"Error: File not found when setting rating: {image_path}")
            return False
            
        exif_success = False
        try:
            rating_str = str(rating)
            param = f"-XMP:Rating={rating_str}".encode('utf-8')
            filename_bytes = image_path.encode('utf-8', errors='surrogateescape')

            with exiftool.ExifToolHelper() as et:
                et.execute(param, b"-overwrite_original", filename_bytes)
                exif_success = True
        except Exception as e:
            print(f"Error setting rating for {os.path.basename(image_path)} via ExifTool: {e} (Type: {type(e).__name__})")
            exif_success = False
        
        if exif_success:
            if rating_disk_cache:
                try:
                    rating_disk_cache.set(image_path, rating)
                    # print(f"[MetadataHandler] Rating for {os.path.basename(image_path)} updated in rating_disk_cache: {rating}")
                except Exception as ce_rating:
                    print(f"Error updating rating_disk_cache for {os.path.basename(image_path)}: {ce_rating}")
            
            if exif_disk_cache:
                try:
                    exif_disk_cache.delete(image_path)
                    # print(f"[MetadataHandler] EXIF data for {os.path.basename(image_path)} invalidated in exif_disk_cache.")
                except Exception as ce_exif:
                    print(f"Error invalidating exif_disk_cache for {os.path.basename(image_path)}: {ce_exif}")
        
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
            print(f"Error: File not found when setting label: {image_path}")
            return False

        success = False
        try:
            label_str = label if label else "" # Empty string to clear the label
            param = f"-XMP:Label={label_str}".encode('utf-8')
            filename_bytes = image_path.encode('utf-8', errors='surrogateescape')

            with exiftool.ExifToolHelper() as et:
                et.execute(param, b"-overwrite_original", filename_bytes)
                # print(f"Executed set label '{label_str}' for {os.path.basename(image_path)}.")
                success = True
        except Exception as e:
            print(f"Error setting label for {os.path.basename(image_path)}: {e} (Type: {type(e).__name__})")
            success = False
        
        if success and exif_disk_cache:
            try:
                exif_disk_cache.delete(image_path)
                # print(f"[MetadataHandler] EXIF data for {os.path.basename(image_path)} invalidated in exif_disk_cache due to label change.")
            except Exception as ce_exif_label:
                print(f"Error invalidating exif_disk_cache for {os.path.basename(image_path)} after label set: {ce_exif_label}")
        return success

if __name__ == '__main__':
    # Create a dummy image file for testing (requires Pillow)
    test_image_path = "test_metadata_handler_image.jpg"
    try:
        from PIL import Image as PILImage
        img = PILImage.new('RGB', (60, 30), color = 'red')
        img.save(test_image_path, quality=90)
        print(f"Created test image: {test_image_path}")

        # Test 1: Set and Get Metadata
        print("\n--- Test: Set and Get Metadata ---")
        set_rating_ok = MetadataHandler.set_rating(test_image_path, 4)
        print(f"Set rating to 4: {'Success' if set_rating_ok else 'Failed'}")
        set_label_ok = MetadataHandler.set_label(test_image_path, "Green")
        print(f"Set label to Green: {'Success' if set_label_ok else 'Failed'}")

        metadata = MetadataHandler.get_display_metadata(test_image_path)
        print(f"Fetched metadata: {metadata}")
        assert metadata['rating'] == 4
        assert metadata['label'] == "Green"
        # Date might be from filesystem for a new file
        if metadata['date']:
            print(f"Date found: {metadata['date']}")
        else:
            print("Date not found (as expected for new file without EXIF date).")


        # Test 2: Clear Metadata
        print("\n--- Test: Clear Metadata ---")
        set_rating_ok_clear = MetadataHandler.set_rating(test_image_path, 0) # Set to 0 stars
        print(f"Set rating to 0: {'Success' if set_rating_ok_clear else 'Failed'}")
        set_label_ok_clear = MetadataHandler.set_label(test_image_path, None) # Clear label
        print(f"Cleared label: {'Success' if set_label_ok_clear else 'Failed'}")
        
        metadata_cleared = MetadataHandler.get_display_metadata(test_image_path)
        print(f"Fetched metadata after clearing: {metadata_cleared}")
        assert metadata_cleared['rating'] == 0
        assert metadata_cleared['label'] is None
        
        # Test 3: Date parsing from filename
        print("\n--- Test: Date Parsing from Filename ---")
        filenames_dates = {
            "IMG_20230515_103000.jpg": date_obj(2023, 5, 15),
            "MyPhoto_2022-11-01.png": date_obj(2022, 11, 1),
            "Vacation 2021.12.25 Highlights.tif": date_obj(2021, 12, 25),
            "NoDateHere.gif": None,
            "20231231.jpg": date_obj(2023,12,31),
            "Screenshot 20240105T142010.png": date_obj(2024,1,5),
            "X20200229_valid_leap.jpg": date_obj(2020,2,29),
            "IMG20210229_invalid_leap.jpg": None, # 2021 not leap
        }
        for fname, expected_date in filenames_dates.items():
            parsed = _parse_date_from_filename(fname)
            print(f"File: '{fname}', Expected: {expected_date}, Got: {parsed} -> {'OK' if parsed == expected_date else 'FAIL'}")
            assert parsed == expected_date

        # Test 4: EXIF Date String Parsing
        print("\n--- Test: EXIF Date String Parsing ---")
        exif_dates_strings = {
            "2023:05:16 10:30:45": date_obj(2023, 5, 16),
            "2022-11-01 23:15:00": date_obj(2022, 11, 1),
            "2021-12-25T08:00:30": date_obj(2021, 12, 25),
            "2020:01:20": date_obj(2020, 1, 20),
            "2019-07-04": date_obj(2019, 7, 4),
            "2023:02:28 12:00:00.795659": date_obj(2023,2,28), # Fractional seconds
            "2024-03-10T10:00:00Z": date_obj(2024,3,10), # Zulu time
            "2024-03-10T05:00:00-05:00": date_obj(2024,3,10), # Timezone offset
            "Invalid Date String": None,
            "2023:13:01 10:00:00": None, # Invalid month
        }
        for date_str, expected_dt_obj in exif_dates_strings.items():
            parsed_dt = _parse_exif_date(date_str)
            print(f"String: '{date_str}', Expected: {expected_dt_obj}, Got: {parsed_dt} -> {'OK' if parsed_dt == expected_dt_obj else 'FAIL'}")
            assert parsed_dt == expected_dt_obj

    except ImportError:
        print("Pillow (PIL) or ExifTool is not installed. Skipping MetadataHandler tests.")
    except Exception as e:
        print(f"An error occurred during MetadataHandler tests: {e}")
    finally:
        if os.path.exists(test_image_path):
            os.remove(test_image_path)
            print(f"Cleaned up test image: {test_image_path}")
        print("\n--- MetadataHandler Tests Complete ---")
import pyexiv2
import os
import re
import time
import logging
import unicodedata
from datetime import datetime as dt_parser, date as date_obj
from typing import Dict, Any, Optional, List
import concurrent.futures

from src.core.caching.rating_cache import RatingCache
from src.core.caching.exif_cache import ExifCache
from src.core.image_processing.image_rotator import ImageRotator, RotationDirection

# Preferred EXIF/XMP date tags in order of preference
DATE_TAGS_PREFERENCE: List[str] = [
   'Exif.Photo.DateTimeOriginal',
   'Xmp.xmp.CreateDate', 
   'Exif.Image.DateTime',
   'Exif.Photo.DateTime',
   'Xmp.photoshop.DateCreated',
]

# Comprehensive metadata tags for extraction (pyexiv2 format)
COMPREHENSIVE_METADATA_TAGS: List[str] = [
   # Basic file info
   "Exif.Image.Make", "Exif.Image.Model", "Exif.Photo.LensModel", "Exif.Photo.LensSpecification",
   "Exif.Photo.FocalLength", "Exif.Photo.FNumber", "Exif.Photo.ApertureValue",
   "Exif.Photo.ShutterSpeedValue", "Exif.Photo.ExposureTime", "Exif.Photo.ISOSpeedRatings",
   "Exif.Photo.Flash", "Exif.Image.ImageWidth", "Exif.Image.ImageLength", "Exif.Image.ColorSpace",
   "Exif.Image.Orientation", "Exif.Image.BitsPerSample", "Exif.Photo.ExposureCompensation",
   "Exif.Photo.MeteringMode", "Exif.Photo.WhiteBalance", 
   # GPS
   "Exif.GPSInfo.GPSLatitude", "Exif.GPSInfo.GPSLongitude", "Exif.GPSInfo.GPSLatitudeRef", "Exif.GPSInfo.GPSLongitudeRef",
   # XMP data
   "Xmp.xmp.Rating", "Xmp.xmp.Label", "Xmp.dc.subject", "Xmp.lr.hierarchicalSubject",
] + DATE_TAGS_PREFERENCE

logging.info(f"[MetadataProcessor] COMPREHENSIVE_METADATA_TAGS defined with {len(COMPREHENSIVE_METADATA_TAGS)} tags")

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

class MetadataProcessor:
   """
   Processes image metadata including reading and writing EXIF, IPTC, and XMP data.
   Handles ratings, labels, dates, and comprehensive metadata extraction using pyexiv2.
   Provides batch processing with caching support for optimal performance.
   """

   @staticmethod
   def get_batch_display_metadata(
       image_paths: List[str],
       rating_disk_cache: Optional[RatingCache] = None,
       exif_disk_cache: Optional[ExifCache] = None
   ) -> Dict[str, Dict[str, Any]]:
       """
       Fetches and parses essential metadata for a batch of images.
       Uses ExifCache first, then pyexiv2 in parallel for remaining files.
       Populates caches and applies date fallbacks.
       """
       results: Dict[str, Dict[str, Any]] = {}
       paths_needing_extraction: List[str] = []
       start_time = time.perf_counter()
       logging.info(f"[MetadataProcessor] Starting batch metadata for {len(image_paths)} files.")

       # 1. Check ExifCache for each file and prepare list for pyexiv2
       for image_path in image_paths:
           norm_path = unicodedata.normalize('NFC', os.path.normpath(image_path))
           results[norm_path] = {'rating': 0, 'label': None, 'date': None, 'raw_metadata': None}

           if not os.path.isfile(norm_path):
               logging.warning(f"[MetadataProcessor] File not found: {norm_path}")
               minimal_data = {"file_path": norm_path, "file_size": "Unknown"}
               if exif_disk_cache:
                   exif_disk_cache.set(norm_path, minimal_data)
               results[norm_path]['date'] = _parse_date_from_filename(os.path.basename(norm_path))
               results[norm_path]['raw_metadata'] = minimal_data
               continue

           cached_metadata: Optional[Dict[str, Any]] = None
           if exif_disk_cache:
               cached_metadata = exif_disk_cache.get(norm_path)
           
           if cached_metadata:
               logging.debug(f"[MetadataProcessor] ExifCache HIT for {os.path.basename(norm_path)}.")
               results[norm_path]['raw_metadata'] = cached_metadata
           else:
               logging.debug(f"[MetadataProcessor] ExifCache MISS for {os.path.basename(norm_path)}")
               paths_needing_extraction.append(norm_path)
       
       # 2. Parallel Batch pyexiv2 call for files not in ExifCache
       CHUNK_SIZE = 25  # Smaller chunks for memory management
       MAX_WORKERS = min(6, (os.cpu_count() or 1) * 2)

       if paths_needing_extraction:
           logging.info(f"[MetadataProcessor] Need to extract metadata for {len(paths_needing_extraction)} files. Processing in parallel chunks of {CHUNK_SIZE} with up to {MAX_WORKERS} workers.")
           
           def process_chunk(chunk_paths: List[str]) -> List[Dict[str, Any]]:
               chunk_results = []
               for path in chunk_paths:
                   file_ext = os.path.splitext(path)[1].lower()
                   try:
                       logging.info(f"[MetadataProcessor] Processing {os.path.basename(path)} (extension: {file_ext}) with pyexiv2...")
                       # Open image with pyexiv2
                       with pyexiv2.Image(path, encoding='utf-8') as img:
                           # Get basic image info
                           pixel_width = img.get_pixel_width()
                           pixel_height = img.get_pixel_height()
                           mime_type = img.get_mime_type()
                           
                           logging.info(f"[MetadataProcessor] Basic info for {os.path.basename(path)}: {pixel_width}x{pixel_height}, mime: {mime_type}")
                           
                           # Read all metadata types
                           exif_data = {}
                           iptc_data = {}
                           xmp_data = {}
                           
                           try:
                               exif_data = img.read_exif()
                               logging.info(f"[MetadataProcessor] EXIF data for {os.path.basename(path)}: {len(exif_data)} keys")
                               if file_ext == '.arw':
                                   logging.info(f"[MetadataProcessor] ARW EXIF keys for {os.path.basename(path)}: {list(exif_data.keys())[:10]}...")
                           except Exception as e:
                               logging.warning(f"[MetadataProcessor] No EXIF data for {os.path.basename(path)}: {e}")
                           
                           try:
                               iptc_data = img.read_iptc()
                               logging.debug(f"[MetadataProcessor] IPTC data for {os.path.basename(path)}: {len(iptc_data)} keys")
                           except Exception as e:
                               logging.debug(f"[MetadataProcessor] No IPTC data for {os.path.basename(path)}: {e}")
                           
                           try:
                               xmp_data = img.read_xmp()
                               logging.info(f"[MetadataProcessor] XMP data for {os.path.basename(path)}: {len(xmp_data)} keys")
                               if file_ext == '.arw':
                                   logging.info(f"[MetadataProcessor] ARW XMP keys for {os.path.basename(path)}: {list(xmp_data.keys())[:10]}...")
                           except Exception as e:
                               logging.debug(f"[MetadataProcessor] No XMP data for {os.path.basename(path)}: {e}")
                           
                           # Combine all metadata
                           combined_metadata = {
                               "file_path": path,
                               "pixel_width": pixel_width,
                               "pixel_height": pixel_height,
                               "mime_type": mime_type,
                               "file_size": os.path.getsize(path) if os.path.isfile(path) else "Unknown",
                               **exif_data,
                               **iptc_data,
                               **xmp_data
                           }
                           
                           chunk_results.append(combined_metadata)
                           logging.info(f"[MetadataProcessor] Successfully extracted {len(combined_metadata)} total metadata fields for {os.path.basename(path)}")
                           
                   except Exception as e:
                       logging.error(f"[MetadataProcessor] Error extracting metadata for {os.path.basename(path)} (ext: {file_ext}): {e}", exc_info=True)
                       # Add minimal entry for failed extractions
                       chunk_results.append({
                           "file_path": path,
                           "file_size": os.path.getsize(path) if os.path.isfile(path) else "Unknown"
                       })
               
               return chunk_results

           # Execute parallel processing
           all_metadata_results = []
           with concurrent.futures.ThreadPoolExecutor(max_workers=MAX_WORKERS) as executor:
               future_to_chunk = {
                   executor.submit(process_chunk, paths_needing_extraction[i:i + CHUNK_SIZE]): paths_needing_extraction[i:i + CHUNK_SIZE]
                   for i in range(0, len(paths_needing_extraction), CHUNK_SIZE)
               }
               for future in concurrent.futures.as_completed(future_to_chunk):
                   chunk_paths = future_to_chunk[future]
                   try:
                       chunk_metadata_results = future.result()
                       all_metadata_results.extend(chunk_metadata_results)
                   except Exception as exc:
                       logging.error(f"[MetadataProcessor] Chunk (first file: {os.path.basename(chunk_paths[0]) if chunk_paths else 'N/A'}) generated an exception: {exc}")
           
           # Process aggregated results
           if all_metadata_results:
               filepath_to_metadata_map = {}
               for metadata_dict in all_metadata_results:
                   file_path = metadata_dict.get("file_path")
                   if file_path:
                       norm_file_path = unicodedata.normalize('NFC', os.path.normpath(file_path))
                       filepath_to_metadata_map[norm_file_path] = metadata_dict

               for path_processed in paths_needing_extraction:
                   raw_metadata = filepath_to_metadata_map.get(path_processed)
                   if raw_metadata:
                       results[path_processed]['raw_metadata'] = raw_metadata
                       if exif_disk_cache:
                           exif_disk_cache.set(path_processed, raw_metadata)
                   else:
                       logging.warning(f"[MetadataProcessor] No metadata result for {os.path.basename(path_processed)}")
                       results[path_processed]['raw_metadata'] = None

       # 3. Parse data from raw_metadata and apply fallbacks
       final_results: Dict[str, Dict[str, Any]] = {}
       for norm_path, data_dict in results.items():
           filename_only = os.path.basename(norm_path)
           parsed_rating = 0
           parsed_label = None
           parsed_date = None
           
           raw_metadata = data_dict['raw_metadata']

           if raw_metadata:
               # Parse Rating - Try XMP:Rating first, then alternatives
               rating_raw_val = raw_metadata.get("Xmp.xmp.Rating")
               log_source_tag = "Xmp.xmp.Rating"
               if rating_raw_val is None:
                   rating_raw_val = raw_metadata.get("Exif.Image.Rating")
                   log_source_tag = "Exif.Image.Rating" if rating_raw_val is not None else "Xmp.xmp.Rating (None)"

               logging.debug(f"[MetadataProcessor] Raw rating value from '{log_source_tag}' for {filename_only}: '{rating_raw_val}' (type: {type(rating_raw_val)})")
               parsed_rating = _parse_rating(rating_raw_val)
               logging.debug(f"[MetadataProcessor] Parsed rating for {filename_only}: {parsed_rating}")
               
               if rating_disk_cache:
                   cached_rating_val = rating_disk_cache.get(norm_path)
                   if cached_rating_val is None or cached_rating_val != parsed_rating:
                        logging.debug(f"[MetadataProcessor] Updating rating_disk_cache for {filename_only} from {cached_rating_val} to {parsed_rating}")
                        rating_disk_cache.set(norm_path, parsed_rating)
               
               # Parse Label
               label_raw_val = raw_metadata.get("Xmp.xmp.Label")
               parsed_label = str(label_raw_val) if label_raw_val is not None else None
               
               # Parse Date from metadata
               for date_tag in DATE_TAGS_PREFERENCE:
                   date_string = raw_metadata.get(date_tag)
                   if date_string:
                       dt_obj_val = _parse_exif_date(str(date_string))
                       if dt_obj_val:
                           parsed_date = dt_obj_val
                           break
           
           # Date fallbacks
           if parsed_date is None:
               parsed_date = _parse_date_from_filename(filename_only)
           
           if parsed_date is None and os.path.isfile(norm_path):
               try:
                   fs_timestamp: Optional[float] = None
                   stat_result = os.stat(norm_path)
                   if hasattr(stat_result, 'st_birthtime'): fs_timestamp = stat_result.st_birthtime
                   if fs_timestamp is None or fs_timestamp < 1000000: fs_timestamp = stat_result.st_mtime
                   if fs_timestamp: parsed_date = dt_parser.fromtimestamp(fs_timestamp).date()
               except Exception as e_fs:
                   logging.warning(f"[MetadataProcessor] Filesystem date error for {filename_only}: {e_fs}")

           final_results[norm_path] = {'rating': parsed_rating, 'label': parsed_label, 'date': parsed_date}
           logging.debug(f"[MetadataProcessor] Processed {filename_only}: R={parsed_rating}, L='{parsed_label}', D={parsed_date}")

       duration = time.perf_counter() - start_time
       logging.info(f"[MetadataProcessor] Finished batch metadata for {len(image_paths)} files in {duration:.4f}s.")
       return final_results

   @staticmethod
   def set_rating(image_path: str, rating: int,
                  rating_disk_cache: Optional[RatingCache] = None,
                  exif_disk_cache: Optional[ExifCache] = None) -> bool:
       """
       Sets the rating (0-5) using pyexiv2.
       Updates rating_disk_cache and invalidates exif_disk_cache if provided.
       Returns True on apparent success, False on failure.
       """
       # Type check and convert to int if possible
       try:
           rating = int(rating)
       except (ValueError, TypeError):
           logging.error(f"Invalid rating value {rating}. Must be 0-5.")
           return False
           
       if not (0 <= rating <= 5):
           logging.error(f"Invalid rating value {rating}. Must be 0-5.")
           return False
       if not os.path.isfile(image_path):
           logging.error(f"File not found when setting rating: {image_path}")
           return False
           
       norm_path = unicodedata.normalize('NFC', os.path.normpath(image_path))
       success = False
       logging.info(f"[MetadataProcessor] Setting rating for {os.path.basename(norm_path)} to {rating}")
       
       try:
           with pyexiv2.Image(norm_path, encoding='utf-8') as img:
               # Set XMP rating
               rating_str = str(rating)
               img.modify_xmp({"Xmp.xmp.Rating": rating_str})
               success = True
               logging.info(f"[MetadataProcessor] pyexiv2 successfully set rating for {os.path.basename(norm_path)}")
       except Exception as e:
           logging.error(f"Error setting rating for {os.path.basename(norm_path)}: {e}", exc_info=True)
           success = False
       
       if success:
           if rating_disk_cache:
               logging.debug(f"[MetadataProcessor] Updating rating_disk_cache for {os.path.basename(norm_path)} to {rating}")
               rating_disk_cache.set(norm_path, rating)
           if exif_disk_cache:
               logging.debug(f"[MetadataProcessor] Deleting from exif_disk_cache for {os.path.basename(norm_path)} due to rating change.")
               exif_disk_cache.delete(norm_path)
       return success

   @staticmethod
   def set_label(image_path: str, label: Optional[str], exif_disk_cache: Optional[ExifCache] = None) -> bool:
       """
       Sets the XMP:Label using pyexiv2.
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
           with pyexiv2.Image(norm_path, encoding='utf-8') as img:
               if label:
                   img.modify_xmp({"Xmp.xmp.Label": label})
               else:
                   # Remove label by setting empty string
                   img.modify_xmp({"Xmp.xmp.Label": ""})
               success = True
       except Exception as e:
           logging.error(f"Error setting label for {os.path.basename(norm_path)}: {e}", exc_info=True)
           success = False
       
       if success and exif_disk_cache:
           exif_disk_cache.delete(norm_path)
       return success

   @staticmethod
   def check_availability() -> bool:
       """
       Checks if pyexiv2 is available and working.
       Tries to create a simple Image instance as a test.
       Returns True if pyexiv2 works, False otherwise.
       """
       try:
           # Test with a simple operation - this will fail if pyexiv2 isn't properly installed
           pyexiv2.set_log_level(4)  # Suppress warnings during test
           
           # Try to create a temporary image object to test the library
           test_path = os.path.join(os.path.dirname(__file__), 'test_availability.tmp')
           
           # Create a minimal test file to verify pyexiv2 works
           try:
               with open(test_path, 'wb') as f:
                   # Write minimal JPEG header to test
                   f.write(b'\xff\xd8\xff\xe0\x00\x10JFIF\x00\x01\x01\x01\x00H\x00H\x00\x00\xff\xd9')
               
               # Try to open with pyexiv2
               with pyexiv2.Image(test_path) as img:
                   _ = img.get_mime_type()  # Simple operation to verify it works
               
               logging.info("[MetadataProcessor] pyexiv2 availability check successful.")
               return True
               
           finally:
               # Clean up test file
               if os.path.exists(test_path):
                   try:
                       os.remove(test_path)
                   except:
                       pass
                       
       except ImportError:
           logging.error("[MetadataProcessor] pyexiv2 availability check failed: ImportError. pyexiv2 library not installed.")
           return False
       except Exception as e:
           logging.error(f"[MetadataProcessor] pyexiv2 availability check failed: {e}", exc_info=True)
           return False

   @staticmethod
   def get_detailed_metadata(image_path: str, exif_disk_cache: Optional[ExifCache] = None) -> Optional[Dict[str, Any]]:
       """
       Fetches detailed metadata for a single image for sidebar display.
       Since batch loading now fetches all detailed metadata, this should mostly be cache hits.
       """
       if not os.path.isfile(image_path):
           logging.warning(f"[MetadataProcessor] File not found for detailed metadata: {image_path}")
           return None

       norm_path = unicodedata.normalize('NFC', os.path.normpath(image_path))
       file_ext = os.path.splitext(norm_path)[1].lower()
       logging.info(f"[MetadataProcessor] get_detailed_metadata called for: {os.path.basename(norm_path)} (extension: {file_ext})")
       
       # Try cache first
       if exif_disk_cache:
           cached_data = exif_disk_cache.get(norm_path)
           if cached_data is not None:
               logging.info(f"[MetadataProcessor] ExifCache HIT for detailed metadata: {os.path.basename(norm_path)} - Found {len(cached_data)} keys")
               logging.debug(f"[MetadataProcessor] Cached keys for {os.path.basename(norm_path)}: {list(cached_data.keys())[:10]}...")
               return cached_data
           else:
               logging.warning(f"[MetadataProcessor] ExifCache MISS for detailed metadata: {os.path.basename(norm_path)}")
       else:
           logging.warning(f"[MetadataProcessor] No exif_disk_cache provided for: {os.path.basename(norm_path)}")
       
       # Fallback: fetch with pyexiv2 if not in cache
       logging.warning(f"[MetadataProcessor] Cache miss - fetching detailed metadata on-demand for: {os.path.basename(norm_path)}")
       
       try:
           logging.info(f"[MetadataProcessor] Opening {os.path.basename(norm_path)} with pyexiv2...")
           with pyexiv2.Image(norm_path, encoding='utf-8') as img:
               logging.info(f"[MetadataProcessor] Successfully opened {os.path.basename(norm_path)} with pyexiv2")
               
               # Get all metadata
               metadata = {
                   "file_path": norm_path,
                   "pixel_width": img.get_pixel_width(),
                   "pixel_height": img.get_pixel_height(),
                   "mime_type": img.get_mime_type(),
                   "file_size": os.path.getsize(norm_path) if os.path.isfile(norm_path) else "Unknown"
               }
               logging.info(f"[MetadataProcessor] Basic metadata for {os.path.basename(norm_path)}: {metadata}")
               
               # Add EXIF data
               try:
                   exif_data = img.read_exif()
                   logging.info(f"[MetadataProcessor] EXIF data for {os.path.basename(norm_path)}: {len(exif_data)} keys")
                   if exif_data:
                       logging.debug(f"[MetadataProcessor] EXIF keys for {os.path.basename(norm_path)}: {list(exif_data.keys())[:10]}...")
                       metadata.update(exif_data)
                   else:
                       logging.warning(f"[MetadataProcessor] EXIF data is empty for {os.path.basename(norm_path)}")
               except Exception as e:
                   logging.warning(f"[MetadataProcessor] No EXIF data for {os.path.basename(norm_path)}: {e}")
               
               # Add IPTC data
               try:
                   iptc_data = img.read_iptc()
                   logging.info(f"[MetadataProcessor] IPTC data for {os.path.basename(norm_path)}: {len(iptc_data)} keys")
                   if iptc_data:
                       metadata.update(iptc_data)
               except Exception as e:
                   logging.debug(f"[MetadataProcessor] No IPTC data for {os.path.basename(norm_path)}: {e}")
               
               # Add XMP data
               try:
                   xmp_data = img.read_xmp()
                   logging.info(f"[MetadataProcessor] XMP data for {os.path.basename(norm_path)}: {len(xmp_data)} keys")
                   if xmp_data:
                       metadata.update(xmp_data)
               except Exception as e:
                   logging.debug(f"[MetadataProcessor] No XMP data for {os.path.basename(norm_path)}: {e}")
               
               logging.info(f"[MetadataProcessor] On-demand fetch: Got {len(metadata)} total keys for {os.path.basename(norm_path)}")
               
               # Cache the result
               if exif_disk_cache:
                   exif_disk_cache.set(norm_path, metadata)
                   logging.info(f"[MetadataProcessor] Cached metadata for {os.path.basename(norm_path)}")
               
               return metadata
               
       except Exception as e:
           logging.error(f"[MetadataProcessor] Error fetching detailed metadata for {os.path.basename(norm_path)}: {e}", exc_info=True)
           # Cache empty result to avoid repeated attempts
           empty_result = {"file_path": norm_path, "file_size": "Unknown"}
           if exif_disk_cache:
               exif_disk_cache.set(norm_path, empty_result)
               logging.warning(f"[MetadataProcessor] Cached empty result for {os.path.basename(norm_path)}")
           return empty_result

   @staticmethod
   def rotate_image(image_path: str, direction: RotationDirection,
                   update_metadata_only: bool = False,
                   exif_disk_cache: Optional[ExifCache] = None) -> bool:
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
       if not os.path.isfile(image_path):
           logging.error(f"File not found when rotating: {image_path}")
           return False
           
       norm_path = unicodedata.normalize('NFC', os.path.normpath(image_path))
       
       try:
           rotator = ImageRotator()
           success, message = rotator.rotate_image(norm_path, direction, update_metadata_only)
           
           if success:
               logging.info(f"[MetadataProcessor] {message}")
               # Invalidate cache since image metadata has changed
               if exif_disk_cache:
                   exif_disk_cache.delete(norm_path)
                   logging.debug(f"[MetadataProcessor] Invalidated cache for rotated image: {os.path.basename(norm_path)}")
           else:
               logging.error(f"[MetadataProcessor] {message}")
           
           return success
           
       except Exception as e:
           logging.error(f"[MetadataProcessor] Error rotating {os.path.basename(norm_path)}: {e}", exc_info=True)
           return False

   @staticmethod
   def rotate_clockwise(image_path: str, update_metadata_only: bool = False,
                       exif_disk_cache: Optional[ExifCache] = None) -> bool:
       """Rotate image 90° clockwise."""
       return MetadataProcessor.rotate_image(image_path, 'clockwise', update_metadata_only, exif_disk_cache)

   @staticmethod
   def rotate_counterclockwise(image_path: str, update_metadata_only: bool = False,
                              exif_disk_cache: Optional[ExifCache] = None) -> bool:
       """Rotate image 90° counterclockwise."""
       return MetadataProcessor.rotate_image(image_path, 'counterclockwise', update_metadata_only, exif_disk_cache)

   @staticmethod
   def rotate_180(image_path: str, update_metadata_only: bool = False,
                 exif_disk_cache: Optional[ExifCache] = None) -> bool:
       """Rotate image 180°."""
       return MetadataProcessor.rotate_image(image_path, '180', update_metadata_only, exif_disk_cache)

   @staticmethod
   def is_rotation_supported(image_path: str) -> bool:
       """Check if rotation is supported for the given image format."""
       try:
           rotator = ImageRotator()
           return rotator.is_rotation_supported(image_path)
       except Exception as e:
           logging.error(f"[MetadataProcessor] Error checking rotation support for {os.path.basename(image_path)}: {e}")
           return False
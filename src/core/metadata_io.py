import os
import threading
import logging
from typing import Dict, Optional, Tuple

# Centralized pyexiv2 usage. Tests and main app ensure early import of pyexiv2
# where necessary to avoid Windows issues; importing here is acceptable.
import pyexiv2

logger = logging.getLogger(__name__)


class MetadataIO:
    """Thin, threadsafe facade around pyexiv2 operations.

    Responsibilities:
    - Open images safely with a global lock (pyexiv2/thread/frozen stability)
    - Read merged EXIF/IPTC/XMP along with basic properties
    - Read/Write specific tags (rating, orientation)
    - Provide small utility reads (orientation + dimensions)

    All methods expect an operational filesystem path (existing file path).
    Path normalization/resolution is intentionally kept in higher layers.
    """

    _LOCK = threading.Lock()
    _WARMED_UP = False
    _FIRST_REAL_ACCESS_LOGGED = False

    @classmethod
    def warmup(cls) -> None:
        """Perform a one-time, main-thread friendly initialization of pyexiv2.

        Rationale: On some Windows systems / frozen builds, the very first call
        into pyexiv2.Exiv2 (especially from a background thread that starts
        after Qt has been initialized) can trigger an access violation. Doing a
        trivial, locked interaction with pyexiv2 early in the main thread makes
        subsequent background usage stable.

        This function is idempotent and very fast (creates a tiny temp JPEG and
        opens it). Failures are logged but never raised.
        """
        if cls._WARMED_UP:
            return
        try:
            import tempfile
            from PIL import Image

            with cls._LOCK:
                if cls._WARMED_UP:  # Double-checked inside lock
                    return
                with tempfile.NamedTemporaryFile(suffix=".jpg", delete=True) as tmp:
                    # Create a minimal valid JPEG
                    img = Image.new("RGB", (2, 2), color=(0, 0, 0))
                    img.save(tmp.name, format="JPEG")
                    # Open via pyexiv2 to force library init
                    try:
                        with pyexiv2.Image(tmp.name, encoding="utf-8"):
                            pass
                    except Exception as e_open:
                        logger.debug(f"Warmup open failed (continuing): {e_open}")
                cls._WARMED_UP = True
                logger.info("MetadataIO warmup completed.")
        except Exception as e:
            logger.warning(f"MetadataIO warmup encountered an issue (non-fatal): {e}")

    @classmethod
    def read_raw_metadata(cls, operational_path: str) -> Dict:
        """Return a merged metadata dict for the given image path.

        The dict includes at least:
        - file_path, pixel_width, pixel_height, mime_type, file_size
        - merged EXIF/IPTC/XMP dictionaries (keys in their native pyexiv2 form)
        On error, returns a dict with 'file_path', 'file_size' and an 'error' string.
        """
        if not os.path.isfile(operational_path):
            return {
                "file_path": operational_path,
                "file_size": "Unknown",
                "error": "File missing at extraction time",
            }
        try:
            with cls._LOCK:
                if not cls._FIRST_REAL_ACCESS_LOGGED:
                    import threading

                    logger.info(
                        "MetadataIO first real pyexiv2 access (warmed_up=%s, thread=%s)",
                        cls._WARMED_UP,
                        threading.current_thread().name,
                    )
                    cls._FIRST_REAL_ACCESS_LOGGED = True
                with pyexiv2.Image(operational_path, encoding="utf-8") as img:
                    md = {
                        "file_path": operational_path,
                        "pixel_width": img.get_pixel_width(),
                        "pixel_height": img.get_pixel_height(),
                        "mime_type": img.get_mime_type(),
                        "file_size": os.path.getsize(operational_path)
                        if os.path.isfile(operational_path)
                        else "Unknown",
                    }
                    try:
                        exif = img.read_exif() or {}
                        if exif:
                            md.update(exif)
                    except Exception:
                        logger.debug(
                            f"No EXIF for {os.path.basename(operational_path)}"
                        )
                    try:
                        iptc = img.read_iptc() or {}
                        if iptc:
                            md.update(iptc)
                    except Exception:
                        logger.debug(
                            f"No IPTC for {os.path.basename(operational_path)}"
                        )
                    try:
                        xmp = img.read_xmp() or {}
                        if xmp:
                            md.update(xmp)
                    except Exception:
                        logger.debug(f"No XMP for {os.path.basename(operational_path)}")
                    return md
        except Exception as e:
            msg = str(e)
            is_missing = (
                ("No such file or directory" in msg)
                or ("errno = 2" in msg)
                or (not os.path.isfile(operational_path))
            )
            level = logger.warning if is_missing else logger.error
            level(
                f"Error extracting metadata for {os.path.basename(operational_path)}: {e}",
                exc_info=not is_missing,
            )
            return {
                "file_path": operational_path,
                "file_size": os.path.getsize(operational_path)
                if os.path.isfile(operational_path)
                else "Unknown",
                "error": f"Extraction failed: {e}",
            }

    @classmethod
    def set_xmp_rating(cls, operational_path: str, rating: int) -> bool:
        """Set XMP rating (0-5). Returns True if succeeded."""
        if not os.path.isfile(operational_path):
            logger.warning(f"Cannot set rating; file missing: {operational_path}")
            return False
        try:
            with cls._LOCK:
                with pyexiv2.Image(operational_path, encoding="utf-8") as img:
                    img.modify_xmp({"Xmp.xmp.Rating": str(int(rating))})
                    return True
        except Exception as e:
            logger.error(
                f"Error setting rating for {os.path.basename(operational_path)}: {e}",
                exc_info=True,
            )
            return False

    @classmethod
    def read_exif_orientation(cls, operational_path: str) -> Optional[int]:
        """Read EXIF orientation value if present (1-8)."""
        if not os.path.isfile(operational_path):
            logger.info(
                f"File missing when querying EXIF orientation: {operational_path}"
            )
            return None
        try:
            with cls._LOCK:
                with pyexiv2.Image(operational_path, encoding="utf-8") as img:
                    exif = img.read_exif() or {}
                    val = exif.get("Exif.Image.Orientation")
                    return int(val) if val is not None else None
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

    @classmethod
    def set_exif_orientation(cls, operational_path: str, orientation: int) -> bool:
        """Write EXIF orientation (1-8). Returns True if succeeded."""
        if not os.path.isfile(operational_path):
            logger.warning(
                f"Cannot set EXIF orientation; file missing: {operational_path}"
            )
            return False
        try:
            with cls._LOCK:
                with pyexiv2.Image(operational_path, encoding="utf-8") as img:
                    img.modify_exif({"Exif.Image.Orientation": int(orientation)})
                    return True
        except Exception as e:
            logger.error(
                f"Error setting EXIF orientation for {os.path.basename(operational_path)}: {e}",
                exc_info=True,
            )
            return False

    @classmethod
    def set_xmp_orientation(cls, operational_path: str, orientation: int) -> bool:
        """Attempt to set XMP tiff Orientation. Returns True if succeeded."""
        if not os.path.isfile(operational_path):
            logger.warning(
                f"Cannot set XMP orientation; file missing: {operational_path}"
            )
            return False
        try:
            with cls._LOCK:
                with pyexiv2.Image(operational_path, encoding="utf-8") as img:
                    img.modify_xmp({"Xmp.tiff.Orientation": str(int(orientation))})
                    return True
        except Exception as e:
            logger.debug(
                f"Could not set XMP orientation for {os.path.basename(operational_path)}: {e}"
            )
            return False

    @classmethod
    def read_orientation_and_dimensions(
        cls, operational_path: str
    ) -> Tuple[Optional[int], Optional[int], Optional[int]]:
        """Efficiently read orientation, pixel width, and pixel height."""
        if not os.path.isfile(operational_path):
            logger.info(
                f"File missing when reading orientation/dimensions: {operational_path}"
            )
            return None, None, None
        try:
            with cls._LOCK:
                with pyexiv2.Image(operational_path, encoding="utf-8") as img:
                    exif = img.read_exif() or {}
                    orientation = exif.get("Exif.Image.Orientation")
                    orientation_val = int(orientation) if orientation else None
                    width = img.get_pixel_width()
                    height = img.get_pixel_height()
                    return orientation_val, width, height
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

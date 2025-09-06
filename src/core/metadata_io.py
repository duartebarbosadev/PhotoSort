# Centralized pyexiv2 usage. Tests and main app ensure early import of pyexiv2
# where necessary to avoid Windows issues; importing here is acceptable.
from core.pyexiv2_wrapper import PyExiv2Operations, safe_pyexiv2_image

import os
import sys
import threading
import logging
import queue
from typing import Dict, Optional, Callable, Any

logger = logging.getLogger(__name__)


def _is_file_missing_error(exception: Exception, file_path: str) -> bool:
    """
    Check if an exception indicates a missing/inaccessible file.

    Args:
        exception: The exception to check
        file_path: Path to the file that was being accessed

    Returns:
        True if the error indicates the file is missing or inaccessible
    """
    msg = str(exception)
    return (
        ("No such file or directory" in msg)
        or ("errno = 2" in msg)
        or (not os.path.isfile(file_path))
    )


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
    _FIRST_REAL_ACCESS_LOGGED = False

    # Single-thread dispatcher to avoid cross-thread usage of pyexiv2 (not thread-safe)
    _TASK_QUEUE: Optional["queue.Queue[tuple]"] = None
    _WORKER_THREAD: Optional[threading.Thread] = None
    _WORKER_READY_EVENT = threading.Event()
    _STOP_EVENT = threading.Event()
    _THREAD_NAME = "pyexiv2-io"

    @classmethod
    def _should_use_worker_thread(cls) -> bool:
        """Use the dedicated single thread on Windows or in frozen builds (safest).

        Can be forced via PHOTOSORT_FORCE_PYEXIV2_THREAD=true.
        """
        try:
            force = os.environ.get(
                "PHOTOSORT_FORCE_PYEXIV2_THREAD", "false"
            ).lower() in {"1", "true", "yes"}
        except Exception:
            force = False
        return force or sys.platform.startswith("win") or getattr(sys, "frozen", False)

    @classmethod
    def start_worker_thread(cls) -> None:
        """Start the dedicated pyexiv2 IO thread (idempotent)."""
        if not cls._should_use_worker_thread():
            return
        with cls._LOCK:
            if cls._WORKER_THREAD and cls._WORKER_THREAD.is_alive():
                return
            cls._TASK_QUEUE = queue.Queue()
            cls._STOP_EVENT.clear()
            cls._WORKER_READY_EVENT.clear()

            def _worker_loop():
                logger.info("MetadataIO worker thread starting...")
                # Signal that the worker is ready to accept tasks
                try:
                    cls._WORKER_READY_EVENT.set()
                except Exception:
                    pass

                # Main task processing loop
                try:
                    while not cls._STOP_EVENT.is_set():
                        try:
                            item = cls._TASK_QUEUE.get(timeout=0.2)  # type: ignore[arg-type]
                        except queue.Empty:
                            continue
                        if item is None:
                            break
                        fn, args, kwargs, reply_q = item
                        try:
                            result = fn(*args, **kwargs)
                            reply_q.put((True, result))
                        except Exception as e:
                            reply_q.put((False, e))
                        finally:
                            try:
                                cls._TASK_QUEUE.task_done()  # type: ignore[union-attr]
                            except Exception:
                                pass
                except Exception as loop_err:
                    logger.error(
                        f"MetadataIO worker loop error: {loop_err}", exc_info=True
                    )
                finally:
                    logger.info("MetadataIO worker thread exiting.")

            t = threading.Thread(
                target=_worker_loop, name=cls._THREAD_NAME, daemon=True
            )
            t.start()
            cls._WORKER_THREAD = t
        # Wait briefly for readiness
        try:
            cls._WORKER_READY_EVENT.wait(timeout=3.0)
        except Exception:
            pass

    @classmethod
    def _call_in_worker(cls, fn: Callable[..., Any], *args: Any, **kwargs: Any) -> Any:
        """Dispatch a callable to the dedicated worker and wait for result."""
        if not cls._should_use_worker_thread():
            # Fallback to direct call (non-Windows, non-frozen dev runs)
            return fn(*args, **kwargs)
        # If we're already on the worker thread, execute directly to avoid deadlock
        try:
            if threading.current_thread().name == cls._THREAD_NAME:
                return fn(*args, **kwargs)
        except Exception:
            pass
        cls.start_worker_thread()
        if not cls._TASK_QUEUE:
            raise RuntimeError("MetadataIO worker queue not initialized")
        reply_q: "queue.Queue[tuple]" = queue.Queue(maxsize=1)
        cls._TASK_QUEUE.put((fn, args, kwargs, reply_q))
        ok, payload = reply_q.get()
        if ok:
            return payload
        raise payload

    @classmethod
    def _read_raw_metadata_inner(cls, operational_path: str) -> Dict:
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
            if not cls._FIRST_REAL_ACCESS_LOGGED:
                import threading as _th

                logger.info(
                    "MetadataIO first real pyexiv2 access (thread=%s)",
                    _th.current_thread().name,
                )
                cls._FIRST_REAL_ACCESS_LOGGED = True

            # Use the new pyexiv2 wrapper for safe operations
            md = PyExiv2Operations.get_comprehensive_metadata(operational_path)
            return md
        except Exception as e:
            is_missing = _is_file_missing_error(e, operational_path)
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
    def read_raw_metadata(cls, operational_path: str) -> Dict:
        """Public API: dispatch to the dedicated worker thread when enabled."""
        return cls._call_in_worker(cls._read_raw_metadata_inner, operational_path)

    @classmethod
    def _set_xmp_rating_inner(cls, operational_path: str, rating: int) -> bool:
        """Set XMP rating (0-5). Returns True if succeeded."""
        if not os.path.isfile(operational_path):
            logger.warning(f"Cannot set rating; file missing: {operational_path}")
            return False
        try:
            with safe_pyexiv2_image(operational_path) as img:
                img.modify_xmp({"Xmp.xmp.Rating": str(int(rating))})
                return True
        except Exception as e:
            logger.error(
                f"Error setting rating for {os.path.basename(operational_path)}: {e}",
                exc_info=True,
            )
            return False

    @classmethod
    def set_xmp_rating(cls, operational_path: str, rating: int) -> bool:
        return cls._call_in_worker(cls._set_xmp_rating_inner, operational_path, rating)

    @classmethod
    def _read_exif_orientation_inner(cls, operational_path: str) -> Optional[int]:
        """Read EXIF orientation value if present (1-8)."""
        if not os.path.isfile(operational_path):
            logger.info(
                f"File missing when querying EXIF orientation: {operational_path}"
            )
            return None
        try:
            with safe_pyexiv2_image(operational_path) as img:
                exif = img.read_exif() or {}
                val = exif.get("Exif.Image.Orientation")
                return int(val) if val is not None else None
        except Exception as e:
            if _is_file_missing_error(e, operational_path):
                logger.warning(
                    f"File missing while reading EXIF orientation: {operational_path} ({str(e)})"
                )
            else:
                logger.error(
                    f"Error getting EXIF orientation for {os.path.basename(operational_path)}: {e}",
                    exc_info=True,
                )
            return None

    @classmethod
    def read_exif_orientation(cls, operational_path: str) -> Optional[int]:
        return cls._call_in_worker(cls._read_exif_orientation_inner, operational_path)

    @classmethod
    def _set_exif_orientation_inner(
        cls, operational_path: str, orientation: int
    ) -> bool:
        """Write EXIF orientation (1-8). Returns True if succeeded."""
        if not os.path.isfile(operational_path):
            logger.warning(
                f"Cannot set EXIF orientation; file missing: {operational_path}"
            )
            return False
        try:
            with safe_pyexiv2_image(operational_path) as img:
                img.modify_exif({"Exif.Image.Orientation": int(orientation)})
                return True
        except Exception as e:
            logger.error(
                f"Error setting EXIF orientation for {os.path.basename(operational_path)}: {e}",
                exc_info=True,
            )
            return False

    @classmethod
    def set_exif_orientation(cls, operational_path: str, orientation: int) -> bool:
        return cls._call_in_worker(
            cls._set_exif_orientation_inner, operational_path, orientation
        )

    @classmethod
    def _set_xmp_orientation_inner(
        cls, operational_path: str, orientation: int
    ) -> bool:
        """Attempt to set XMP tiff Orientation. Returns True if succeeded."""
        if not os.path.isfile(operational_path):
            logger.warning(
                f"Cannot set XMP orientation; file missing: {operational_path}"
            )
            return False
        try:
            with safe_pyexiv2_image(operational_path) as img:
                img.modify_xmp({"Xmp.tiff.Orientation": str(int(orientation))})
                return True
        except Exception as e:
            logger.debug(
                f"Could not set XMP orientation for {os.path.basename(operational_path)}: {e}"
            )
            return False

    @classmethod
    def set_xmp_orientation(cls, operational_path: str, orientation: int) -> bool:
        return cls._call_in_worker(
            cls._set_xmp_orientation_inner, operational_path, orientation
        )

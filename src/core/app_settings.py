"""
Application Settings Module
Manages persistent application settings using QSettings.
"""

import ctypes
import os
import sys
from collections.abc import Mapping
from dataclasses import dataclass
from enum import Enum
from PyQt6.QtCore import QSettings
from core.runtime_paths import resolve_user_cache_dir


class PerformanceMode(Enum):
    """
    Performance modes for controlling thread pool sizes and resource usage.

    - BALANCED: Uses 85% of available CPU cores to keep system responsive
    - PERFORMANCE: Uses all available CPU cores for maximum speed
    - CUSTOM: Uses a user-defined number of threads
    """

    BALANCED = "balanced"
    PERFORMANCE = "performance"
    CUSTOM = "custom"

    @classmethod
    def from_string(cls, value: str) -> PerformanceMode:
        """Convert string to PerformanceMode enum, defaulting to BALANCED."""
        try:
            return cls(value.lower())
        except ValueError, AttributeError:
            return cls.BALANCED


# --- Settings Constants ---

# Settings organization and application name
SETTINGS_ORGANIZATION = "PhotoSort"
SETTINGS_APPLICATION = "PhotoSort"

# Settings keys
PREVIEW_CACHE_SIZE_GB_KEY = "Cache/PreviewCacheSizeGB"
EXIF_CACHE_SIZE_MB_KEY = "Cache/ExifCacheSizeMB"  # For EXIF metadata cache
ROTATION_CONFIRM_LOSSY_KEY = "UI/RotationConfirmLossy"  # Ask before lossy rotation
SHOW_WORKFLOW_SHORTCUTS_KEY = "UI/ShowWorkflowShortcuts"
WORKFLOW_STEP_VISIBILITY_KEYS = {
    "organize": "Workflow/ShowOrganize",
    "easy_delete": "Workflow/ShowEasyDelete",
    "fix_rotation": "Workflow/ShowFixRotation",
    "pick_best": "Workflow/ShowPickBest",
    "cull": "Workflow/ShowCull",
}
RECENT_FOLDERS_KEY = "UI/RecentFolders"  # Key for recent folders list
INTRO_VIDEO_SHOWN_KEY = "UI/IntroVideoShown"  # Whether the first-run intro video played
ORIENTATION_MODEL_NAME_KEY = (
    "Models/OrientationModelName"  # Key for the orientation model file name
)
SIMILARITY_EMBEDDING_MODEL_KEY = "Models/SimilarityEmbeddingModel"
SIMILARITY_CLUSTERING_EPS_KEY = "Models/SimilarityClusteringEps"
UPDATE_CHECK_ENABLED_KEY = "Updates/CheckEnabled"  # Enable automatic update checks
UPDATE_LAST_CHECK_KEY = "Updates/LastCheckTime"  # Last time updates were checked
PERFORMANCE_MODE_KEY = (
    "Performance/Mode"  # Performance mode (balanced/performance/custom)
)
CUSTOM_THREAD_COUNT_KEY = (
    "Performance/CustomThreadCount"  # User-defined thread count for custom mode
)
OPENAI_API_KEY_KEY = "AI/OpenAIKey"
OPENAI_MODEL_KEY = "AI/OpenAIModel"
OPENAI_BASE_URL_KEY = "AI/OpenAIBaseUrl"
OPENAI_MAX_TOKENS_KEY = "AI/OpenAIMaxTokens"
OPENAI_TIMEOUT_KEY = "AI/OpenAITimeout"
OPENAI_MAX_WORKERS_KEY = "AI/OpenAIMaxWorkers"
OPENAI_BEST_SHOT_PROMPT_KEY = "AI/BestShotPrompt"
OPENAI_RATING_PROMPT_KEY = "AI/RatingPrompt"
BEST_SHOT_BATCH_SIZE_KEY = "AI/BestShotBatchSize"
LOCATION_GROUPING_DEPTH_KEY = "Grouping/LocationDepth"
COMPANION_FILES_PREFERENCE_KEY = "Grouping/CompanionFilesPreference"
EASY_DELETE_BLUR_THRESHOLD_KEY = "EasyDelete/BlurThreshold"
EASY_DELETE_DARK_THRESHOLD_KEY = "EasyDelete/DarkThreshold"
EASY_DELETE_WHITE_THRESHOLD_KEY = "EasyDelete/WhiteThreshold"
EASY_DELETE_DUPLICATE_DISTANCE_KEY = "EasyDelete/DuplicateCosineDistance"


# Cache directories
def get_huggingface_cache_dir() -> str:
    """Return the Hugging Face cache directory, resolved lazily on first call."""
    return resolve_user_cache_dir("hf")


# Default values
DEFAULT_PREVIEW_CACHE_SIZE_GB = 2.0  # Default to 2 GB for preview cache
DEFAULT_EXIF_CACHE_SIZE_MB = 2048  # Default to 2 GB for EXIF cache
MAX_EXIF_CACHE_SIZE_MB = 5120  # Largest selectable EXIF cache limit (5 GB)
DEFAULT_ROTATION_CONFIRM_LOSSY = True  # Default to asking before lossy rotation
DEFAULT_SHOW_WORKFLOW_SHORTCUTS = True
DEFAULT_WORKFLOW_STEP_VISIBILITY = dict.fromkeys(WORKFLOW_STEP_VISIBILITY_KEYS, True)
MAX_RECENT_FOLDERS = 10  # Max number of recent folders to store
DEFAULT_ORIENTATION_MODEL_NAME = None  # Default to None, so we can auto-detect
ROTATION_MODEL_DOWNLOAD_URL = (
    "https://github.com/duartebarbosadev/deep-image-orientation-detection/releases"
)
DEFAULT_UPDATE_CHECK_ENABLED = True  # Default to enable automatic update checks
DEFAULT_PERFORMANCE_MODE = PerformanceMode.BALANCED  # Default to balanced mode
DEFAULT_CUSTOM_THREAD_COUNT = 4  # Default custom thread count
DEFAULT_OPENAI_API_KEY = ""
DEFAULT_OPENAI_MODEL = "Qwen3-VL-30B-A3B-Instruct-MLX-4bit"
DEFAULT_OPENAI_BASE_URL = "http://127.0.0.1:8000/v1"
DEFAULT_OPENAI_MAX_TOKENS = 200
DEFAULT_OPENAI_TIMEOUT = 600
DEFAULT_OPENAI_MAX_WORKERS = 4
DEFAULT_BEST_SHOT_BATCH_SIZE = 3

# Image inspection quality progression. Originals stay memory-only and are
# bounded across the complete visible comparison set.
INSPECTION_DETAIL_DWELL_MS = 1000
INSPECTION_DETAIL_BUDGET_BYTES = 512 * 1024 * 1024


@dataclass(frozen=True, slots=True)
class LocalBestShotConstants:
    model_stride: int = 32
    tensor_cache_key: str = "_photosort_pyiqa_tensor"


_LOCAL_BEST_SHOT_CONSTANTS = LocalBestShotConstants()

# --- UI Constants ---
# Grid view settings
FIXED_ICON_SIZE = 96  # Fixed icon size for grid view
FIXED_GRID_WIDTH = 128  # Fixed grid cell width
FIXED_GRID_HEIGHT = 148  # Fixed grid cell height
GRID_SPACING = 4  # Spacing between grid items

# Main window layout
LEFT_PANEL_STRETCH = 1  # Left panel stretch factor
CENTER_PANEL_STRETCH = 3  # Center panel stretch factor
RIGHT_PANEL_STRETCH = 1  # Right panel stretch factor

# --- Processing Constants ---
# Blur detection
DEFAULT_BLUR_DETECTION_THRESHOLD = 100.0  # Default threshold for blur detection

# Easy Delete step detection thresholds
EASY_DELETE_BLUR_THRESHOLD = (
    100.0  # min acceptable peak local tile sharpness; below = blurry
)
EASY_DELETE_BLUR_TILE_GRID = 4  # NxN grid; blur score = max per-tile Laplacian variance
EASY_DELETE_DARK_CLIP_FRACTION = (
    0.85  # fraction of pixels below dark cutoff; above = near-black
)
EASY_DELETE_DARK_CLIP_VALUE = 10  # 0-255; pixels at/below this count as "dark"
EASY_DELETE_WHITE_CLIP_FRACTION = (
    0.85  # fraction of pixels above white cutoff; above = overexposed
)
EASY_DELETE_WHITE_CLIP_VALUE = 245  # 0-255; pixels at/above this count as "white"
EASY_DELETE_DARK_MEAN_THRESHOLD = 15.0  # 0-255 mean brightness; below = near-black
EASY_DELETE_WHITE_MEAN_THRESHOLD = 248.0  # 0-255 mean brightness; above = overexposed
EASY_DELETE_DUPLICATE_COSINE_DISTANCE = 0.01  # cosine distance; below = near-identical

# Fix Rotation step
FIX_ROTATION_MIN_CONFIDENCE = 0.70  # model confidence; below = skip suggestion

# Iteration limits for safety
DEFAULT_MAX_ITERATIONS = 5000  # Default maximum iterations for various loops
DEFAULT_SAFETY_ITERATION_MULTIPLIER = 2  # Multiplier for safety iteration limits

# Batch processing
METADATA_PROCESSING_CHUNK_SIZE = 25  # Chunk size for metadata processing
METADATA_EMIT_BATCH_SIZE = 50  # Batch size for metadata emission
METADATA_PROGRESS_EMIT_INTERVAL = 20  # Limit cross-thread progress signals
FILE_SCAN_EMIT_BATCH_SIZE = 64  # Reduce cross-thread/UI work during discovery

# UI Population Settings (for large folders)
LARGE_FOLDER_THRESHOLD = 500  # Items above this use chunked UI population
UI_POPULATION_CHUNK_SIZE = 25  # Items to process before calling processEvents() - smaller for more frequent updates

# Grouping Step Drag-and-Drop
GROUPING_DROP_HIGHLIGHT_COLOR = (98, 196, 160, 80)  # RGBA teal with alpha

# Thumbnail Lazy Loading Settings
THUMBNAIL_PRELOAD_ENABLED = True  # Enable background thumbnail preloading
THUMBNAIL_PRELOAD_BATCH_SIZE = 20  # Number of thumbnails to generate per batch
THUMBNAIL_PRELOAD_VISIBLE_MARGIN = (
    10  # Number of items above/below visible area to preload
)
IMAGE_MEMORY_CACHE_SIZE_BYTES = 256 * 1024 * 1024  # Shared hot-image budget
NAVIGATION_PREVIEW_LOOKAHEAD = 4  # Selected image plus a small directional buffer

# Preview size estimation
# Preview cache payload for this app is usually well below original image bytes,
# especially with large RAW sources. Keep modest headroom to avoid noisy warnings.
PREVIEW_ESTIMATED_SIZE_FACTOR = 0.30  # Estimate preview cache as 30% of source bytes

# --- AI/ML Constants ---
# DBSCAN clustering parameters
DBSCAN_EPS = (
    0.055  # For cosine metric: 1 - cosine_similarity. Smaller eps = higher similarity
)
DBSCAN_MIN_SAMPLES = 2  # Minimum number of images to form a dense region (cluster)
DEFAULT_SIMILARITY_BATCH_SIZE = 16  # Default batch size for similarity processing
MIN_SIMILARITY_CLUSTERING_EPS = 0.02
MAX_SIMILARITY_CLUSTERING_EPS = 0.20
DEFAULT_SIMILARITY_CLUSTERING_EPS = DBSCAN_EPS

# RAW image processing
RAW_AUTO_EDIT_BRIGHTNESS_STANDARD = (
    1.15  # Standard brightness adjustment for RAW auto-edits
)
RAW_AUTO_EDIT_BRIGHTNESS_ENHANCED = (
    1.25  # Enhanced brightness adjustment for RAW auto-edits
)

# Model settings
ROTATION_MODEL_IMAGE_SIZE = 384  # Image size for rotation detection model
SUPPORTED_SIMILARITY_EMBEDDING_MODELS = (
    "facebook/dinov2-small",
    "facebook/dinov2-base",
)
DEFAULT_SIMILARITY_EMBEDDING_MODEL = "facebook/dinov2-small"

# --- Cache Constants ---
# Thumbnail cache
DEFAULT_THUMBNAIL_CACHE_SIZE_BYTES = (
    2**30
)  # 1 GiB (1,073,741,824 bytes) default for thumbnail cache
THUMBNAIL_MIN_FILE_SIZE = 1024 * 1024  # 1 MB minimum file size for disk caching

# Preview cache
PREVIEW_CACHE_MIN_FILE_SIZE = 256 * 1024  # 256 KB minimum file size for disk caching

# EXIF cache
EXIF_CACHE_MIN_FILE_SIZE = 4096  # 4 KB minimum file size for disk caching

# Rating cache
DEFAULT_RATING_CACHE_SIZE_LIMIT_MB = 256  # Default 256MB limit for rating cache

# --- File Operation Constants ---

# --- Image Processing Constants ---
# Image size settings
THUMBNAIL_MAX_SIZE = (256, 256)  # Maximum size for thumbnails
PRELOAD_MAX_RESOLUTION = (1920, 1200)  # Fixed high resolution for preloading
DISPLAY_MAX_RESOLUTION = (2560, 2560)  # Enough for a sharp screen preview/zoom
BLUR_DETECTION_PREVIEW_SIZE = (640, 480)  # Size for image used in blur detection

# --- Update Check Constants ---
UPDATE_CHECK_INTERVAL_HOURS = 24  # Check for updates every 24 hours
UPDATE_CHECK_TIMEOUT_SECONDS = 10  # Timeout for update check requests
GITHUB_REPO_OWNER = "duartebarbosadev"  # GitHub repository owner
GITHUB_REPO_NAME = "PhotoSort"  # GitHub repository name


def _get_settings() -> QSettings:
    """Get a QSettings instance with the application's organization and name."""
    return QSettings(SETTINGS_ORGANIZATION, SETTINGS_APPLICATION)


# --- Preview Cache Size ---
def get_preview_cache_size_gb() -> float:
    """Gets the configured preview cache size in GB from settings."""
    settings = _get_settings()
    return settings.value(
        PREVIEW_CACHE_SIZE_GB_KEY, DEFAULT_PREVIEW_CACHE_SIZE_GB, type=float
    )


def set_preview_cache_size_gb(size_gb: float):
    """Sets the preview cache size in GB in settings."""
    settings = _get_settings()
    settings.setValue(PREVIEW_CACHE_SIZE_GB_KEY, size_gb)


def get_preview_cache_size_bytes() -> int:
    """Gets the configured preview cache size in bytes."""
    return int(get_preview_cache_size_gb() * 1024 * 1024 * 1024)


# --- EXIF Cache Size ---
def get_exif_cache_size_mb() -> int:
    """Gets the configured EXIF cache size in MB from settings."""
    settings = _get_settings()
    return settings.value(EXIF_CACHE_SIZE_MB_KEY, DEFAULT_EXIF_CACHE_SIZE_MB, type=int)


def set_exif_cache_size_mb(size_mb: int):
    """Sets the EXIF cache size in MB in settings."""
    settings = _get_settings()
    settings.setValue(EXIF_CACHE_SIZE_MB_KEY, size_mb)


def get_exif_cache_size_bytes() -> int:
    """Gets the configured EXIF cache size in bytes."""
    return get_exif_cache_size_mb() * 1024 * 1024


# --- PyTorch/CUDA Information ---
# --- Rotation Settings ---
def get_rotation_confirm_lossy() -> bool:
    """Get whether to confirm lossy rotations."""
    settings = _get_settings()
    return settings.value(
        ROTATION_CONFIRM_LOSSY_KEY, DEFAULT_ROTATION_CONFIRM_LOSSY, type=bool
    )


def set_rotation_confirm_lossy(confirm: bool):
    """Set whether to confirm lossy rotations."""
    settings = _get_settings()
    settings.setValue(ROTATION_CONFIRM_LOSSY_KEY, confirm)


def get_show_workflow_shortcuts() -> bool:
    """Return whether the shared workflow shortcut footer is visible."""
    settings = _get_settings()
    return settings.value(
        SHOW_WORKFLOW_SHORTCUTS_KEY, DEFAULT_SHOW_WORKFLOW_SHORTCUTS, type=bool
    )


def set_show_workflow_shortcuts(visible: bool) -> None:
    """Persist whether the shared workflow shortcut footer is visible."""
    settings = _get_settings()
    settings.setValue(SHOW_WORKFLOW_SHORTCUTS_KEY, bool(visible))


def get_workflow_step_visibility() -> dict[str, bool]:
    """Return persisted workflow navigation visibility for every step."""

    settings = _get_settings()
    visibility = {
        step: settings.value(key, DEFAULT_WORKFLOW_STEP_VISIBILITY[step], type=bool)
        for step, key in WORKFLOW_STEP_VISIBILITY_KEYS.items()
    }
    # These endpoints keep the application usable before and after review.
    visibility["organize"] = True
    visibility["cull"] = True
    return visibility


def set_workflow_step_visibility(visibility: Mapping[str, bool]) -> None:
    """Persist optional workflow steps while keeping endpoints available."""

    settings = _get_settings()
    for step, key in WORKFLOW_STEP_VISIBILITY_KEYS.items():
        value = visibility.get(step, DEFAULT_WORKFLOW_STEP_VISIBILITY[step])
        if step in {"organize", "cull"}:
            value = True
        settings.setValue(key, bool(value))


# --- Easy Delete Detection Thresholds ---
def get_easy_delete_blur_threshold() -> float:
    """Min acceptable peak local tile sharpness; below this an image is flagged blurry."""
    settings = _get_settings()
    return settings.value(
        EASY_DELETE_BLUR_THRESHOLD_KEY, EASY_DELETE_BLUR_THRESHOLD, type=float
    )


def set_easy_delete_blur_threshold(value: float):
    """Set the Easy Delete blur sharpness threshold."""
    settings = _get_settings()
    settings.setValue(EASY_DELETE_BLUR_THRESHOLD_KEY, float(value))


def get_easy_delete_dark_threshold() -> float:
    """Mean-brightness cutoff (0-255); images darker than this are flagged near-black."""
    settings = _get_settings()
    return settings.value(
        EASY_DELETE_DARK_THRESHOLD_KEY, EASY_DELETE_DARK_MEAN_THRESHOLD, type=float
    )


def set_easy_delete_dark_threshold(value: float):
    """Set the Easy Delete near-black brightness threshold."""
    settings = _get_settings()
    settings.setValue(EASY_DELETE_DARK_THRESHOLD_KEY, float(value))


def get_easy_delete_white_threshold() -> float:
    """Mean-brightness cutoff (0-255); images brighter than this are flagged overexposed."""
    settings = _get_settings()
    return settings.value(
        EASY_DELETE_WHITE_THRESHOLD_KEY, EASY_DELETE_WHITE_MEAN_THRESHOLD, type=float
    )


def set_easy_delete_white_threshold(value: float):
    """Set the Easy Delete overexposed brightness threshold."""
    settings = _get_settings()
    settings.setValue(EASY_DELETE_WHITE_THRESHOLD_KEY, float(value))


def get_easy_delete_duplicate_distance() -> float:
    """Cosine-distance cutoff; pairs closer than this are flagged near-duplicates."""
    settings = _get_settings()
    return settings.value(
        EASY_DELETE_DUPLICATE_DISTANCE_KEY,
        EASY_DELETE_DUPLICATE_COSINE_DISTANCE,
        type=float,
    )


def set_easy_delete_duplicate_distance(value: float):
    """Set the Easy Delete near-duplicate cosine-distance threshold."""
    settings = _get_settings()
    settings.setValue(EASY_DELETE_DUPLICATE_DISTANCE_KEY, float(value))


# --- First-Run Intro Video ---
def get_intro_video_shown() -> bool:
    """Whether the first-run intro video has already been shown to this user."""
    settings = _get_settings()
    return settings.value(INTRO_VIDEO_SHOWN_KEY, False, type=bool)


def set_intro_video_shown(value: bool):
    """Record that the first-run intro video has been shown (or reset it)."""
    settings = _get_settings()
    settings.setValue(INTRO_VIDEO_SHOWN_KEY, bool(value))


# --- Recent Folders ---
def get_recent_folders() -> list[str]:
    """Gets the list of recent folders from settings."""
    settings = _get_settings()
    recent_folders = settings.value(RECENT_FOLDERS_KEY, [], type=list)
    # Filter out folders that no longer exist
    return [folder for folder in recent_folders if os.path.isdir(folder)]


def add_recent_folder(path: str):
    """Adds a folder to the top of the recent folders list."""
    if not path or not os.path.isdir(path):
        return

    settings = _get_settings()
    # Get the list of current, valid recent folders
    recent_folders = get_recent_folders()

    # Use os.path.normpath to handle platform-specific path separators (e.g., / vs \)
    normalized_path = os.path.normpath(path)

    # Remove if already exists (case-insensitive on Windows)
    # and normalize existing paths for comparison
    recent_folders = [
        p
        for p in recent_folders
        if os.path.normpath(p).lower() != normalized_path.lower()
    ]

    # Add the new path to the beginning of the list
    recent_folders.insert(0, normalized_path)

    # Trim the list to the maximum size
    if len(recent_folders) > MAX_RECENT_FOLDERS:
        recent_folders = recent_folders[:MAX_RECENT_FOLDERS]

    settings.setValue(RECENT_FOLDERS_KEY, recent_folders)


def is_pytorch_cuda_available() -> bool:
    """Check if PyTorch with CUDA support is available."""
    try:
        import torch

        return torch.cuda.is_available()
    except ImportError:
        return False


def get_preferred_torch_device() -> str:
    """Return the fastest available torch.device string (cuda, mps, or cpu).

    Set PHOTOSORT_TORCH_DEVICE env var to override (e.g. 'cpu', 'mps', 'cuda').
    Set PHOTOSORT_FORCE_CPU=1 to force CPU regardless of hardware.
    """
    import os

    env_device = os.environ.get("PHOTOSORT_TORCH_DEVICE", "").strip().lower()
    if env_device in ("cpu", "mps", "cuda"):
        import logging

        logging.getLogger(__name__).info(
            f"PHOTOSORT_TORCH_DEVICE={env_device} → forcing device '{env_device}'"
        )
        return env_device

    if os.environ.get("PHOTOSORT_FORCE_CPU", "").strip() == "1":
        import logging

        logging.getLogger(__name__).info("PHOTOSORT_FORCE_CPU=1 → using CPU")
        return "cpu"

    try:
        import torch
    except ImportError:
        return "cpu"

    if torch.cuda.is_available():
        return "cuda"

    mps_backend = getattr(getattr(torch, "backends", None), "mps", None)
    if mps_backend is not None:
        try:
            if mps_backend.is_available():  # type: ignore[attr-defined]
                return "mps"
        except Exception:
            pass

    return "cpu"


# --- Orientation Model ---
def get_orientation_model_name() -> str | None:
    """Gets the configured orientation model name from settings."""
    settings = _get_settings()
    return settings.value(
        ORIENTATION_MODEL_NAME_KEY, DEFAULT_ORIENTATION_MODEL_NAME, type=str
    )


def set_orientation_model_name(model_name: str):
    """Sets the orientation model name in settings."""
    settings = _get_settings()
    settings.setValue(ORIENTATION_MODEL_NAME_KEY, model_name)


def get_similarity_embedding_model_name() -> str:
    """Gets the configured visual embedding model for similarity analysis."""
    settings = _get_settings()
    model_name = settings.value(
        SIMILARITY_EMBEDDING_MODEL_KEY,
        DEFAULT_SIMILARITY_EMBEDDING_MODEL,
        type=str,
    )
    if model_name not in SUPPORTED_SIMILARITY_EMBEDDING_MODELS:
        return DEFAULT_SIMILARITY_EMBEDDING_MODEL
    return model_name


def set_similarity_embedding_model_name(model_name: str):
    """Sets the visual embedding model for similarity analysis."""
    if model_name not in SUPPORTED_SIMILARITY_EMBEDDING_MODELS:
        raise ValueError(f"Unsupported similarity embedding model: {model_name}")
    settings = _get_settings()
    settings.setValue(SIMILARITY_EMBEDDING_MODEL_KEY, model_name)


def get_similarity_clustering_eps() -> float:
    """Gets the DBSCAN cosine-distance threshold used for similarity clustering."""
    settings = _get_settings()
    eps = settings.value(
        SIMILARITY_CLUSTERING_EPS_KEY,
        DEFAULT_SIMILARITY_CLUSTERING_EPS,
        type=float,
    )
    try:
        eps = float(eps)
    except TypeError, ValueError:
        return DEFAULT_SIMILARITY_CLUSTERING_EPS
    return max(
        MIN_SIMILARITY_CLUSTERING_EPS,
        min(MAX_SIMILARITY_CLUSTERING_EPS, eps),
    )


def set_similarity_clustering_eps(eps: float):
    """Sets the DBSCAN cosine-distance threshold used for similarity clustering."""
    eps = float(eps)
    if not MIN_SIMILARITY_CLUSTERING_EPS <= eps <= MAX_SIMILARITY_CLUSTERING_EPS:
        raise ValueError(
            "Similarity clustering threshold must be between "
            f"{MIN_SIMILARITY_CLUSTERING_EPS:.3f} and "
            f"{MAX_SIMILARITY_CLUSTERING_EPS:.3f}"
        )
    settings = _get_settings()
    settings.setValue(SIMILARITY_CLUSTERING_EPS_KEY, eps)


# --- Update Check Settings ---
def get_update_check_enabled() -> bool:
    """Gets whether automatic update checks are enabled."""
    settings = _get_settings()
    return settings.value(
        UPDATE_CHECK_ENABLED_KEY, DEFAULT_UPDATE_CHECK_ENABLED, type=bool
    )


def set_update_check_enabled(enabled: bool):
    """Sets whether automatic update checks are enabled."""
    settings = _get_settings()
    settings.setValue(UPDATE_CHECK_ENABLED_KEY, enabled)


def get_last_update_check_time() -> int:
    """Gets the timestamp of the last update check (seconds since epoch)."""
    settings = _get_settings()
    return settings.value(UPDATE_LAST_CHECK_KEY, 0, type=int)


def set_last_update_check_time(timestamp: int):
    """Sets the timestamp of the last update check."""
    settings = _get_settings()
    settings.setValue(UPDATE_LAST_CHECK_KEY, timestamp)


# --- Performance Mode Settings ---
def get_performance_mode() -> PerformanceMode:
    """Gets the configured performance mode."""
    settings = _get_settings()
    mode_str = settings.value(
        PERFORMANCE_MODE_KEY, DEFAULT_PERFORMANCE_MODE.value, type=str
    )
    return PerformanceMode.from_string(mode_str)


def set_performance_mode(mode: PerformanceMode):
    """Sets the performance mode."""
    settings = _get_settings()
    settings.setValue(PERFORMANCE_MODE_KEY, mode.value)


def get_custom_thread_count() -> int:
    """Gets the user-defined custom thread count."""
    settings = _get_settings()
    return settings.value(
        CUSTOM_THREAD_COUNT_KEY, DEFAULT_CUSTOM_THREAD_COUNT, type=int
    )


def set_custom_thread_count(count: int):
    """Sets the custom thread count. Must be between 1 and system CPU count."""
    max_threads = get_available_cpu_count()
    if not (1 <= count <= max_threads):
        raise ValueError(
            f"Thread count must be between 1 and {max_threads}, got {count}"
        )
    settings = _get_settings()
    settings.setValue(CUSTOM_THREAD_COUNT_KEY, count)


def calculate_max_workers(min_workers: int = 1, max_workers: int | None = None) -> int:
    """
    Calculate the optimal number of worker threads based on the current performance mode.

    Args:
        min_workers: Minimum number of workers to return (default: 1)
        max_workers: Maximum number of workers to return (default: None/unlimited)

    Returns:
        Number of worker threads based on performance mode:
        - Performance: 100% of CPU cores
        - Balanced: 85% of CPU cores (keeps system responsive)
        - Custom: User-specified thread count

    Examples:
        calculate_max_workers()  # Uses performance mode settings
        calculate_max_workers(min_workers=2, max_workers=8)  # Constrained to 2-8 range
    """
    cpu_count = get_available_cpu_count()
    mode = get_performance_mode()

    if mode == PerformanceMode.PERFORMANCE:
        # Performance mode: use all cores
        workers = cpu_count
    elif mode == PerformanceMode.CUSTOM:
        # Custom mode: use user-defined thread count
        workers = get_custom_thread_count()
    else:  # BALANCED
        # Balanced mode: use 85% of cores to keep system responsive
        workers = max(1, int(cpu_count * 0.85))

    # Apply min/max constraints
    workers = max(min_workers, workers)
    if max_workers is not None:
        workers = min(max_workers, workers)

    return workers


def get_available_cpu_count() -> int:
    """Return CPUs available to this process across desktops and containers."""
    count = os.process_cpu_count()
    if count:
        return max(1, count)
    return max(1, os.cpu_count() or 4)


def _get_windows_physical_memory_bytes() -> int | None:
    """Return installed memory on Windows without an optional dependency."""
    if os.name != "nt":
        return None

    class MemoryStatusEx(ctypes.Structure):
        _fields_ = [
            ("dwLength", ctypes.c_ulong),
            ("dwMemoryLoad", ctypes.c_ulong),
            ("ullTotalPhys", ctypes.c_ulonglong),
            ("ullAvailPhys", ctypes.c_ulonglong),
            ("ullTotalPageFile", ctypes.c_ulonglong),
            ("ullAvailPageFile", ctypes.c_ulonglong),
            ("ullTotalVirtual", ctypes.c_ulonglong),
            ("ullAvailVirtual", ctypes.c_ulonglong),
            ("ullAvailExtendedVirtual", ctypes.c_ulonglong),
        ]

    status = MemoryStatusEx()
    status.dwLength = ctypes.sizeof(status)
    try:
        if ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(status)):
            return int(status.ullTotalPhys)
    except AttributeError, OSError:
        pass
    return None


def get_total_physical_memory_bytes() -> int | None:
    """Return installed physical memory on Windows, macOS, or Linux."""
    if os.name == "nt":
        return _get_windows_physical_memory_bytes()
    try:
        page_size = os.sysconf("SC_PAGE_SIZE")
        page_count = os.sysconf("SC_PHYS_PAGES")
        total = int(page_size) * int(page_count)
        return total if total > 0 else None
    except AttributeError, OSError, TypeError, ValueError:
        return None


def _get_linux_cgroup_memory_limit_bytes() -> int | None:
    """Return the effective Linux container memory ceiling when constrained."""
    if not sys.platform.startswith("linux"):
        return None
    for path in (
        "/sys/fs/cgroup/memory.max",
        "/sys/fs/cgroup/memory/memory.limit_in_bytes",
    ):
        try:
            with open(path, encoding="ascii") as limit_file:
                raw_value = limit_file.read().strip()
            if raw_value == "max":
                continue
            limit = int(raw_value)
            # Some cgroup v1 hosts use a huge sentinel for "unlimited".
            if 0 < limit < 1 << 60:
                return limit
        except OSError, ValueError:
            continue
    return None


def get_usable_memory_bytes() -> int | None:
    """Return memory usable by this process, respecting Linux containers."""
    physical = get_total_physical_memory_bytes()
    container_limit = _get_linux_cgroup_memory_limit_bytes()
    if physical and container_limit:
        return min(physical, container_limit)
    return physical or container_limit


def calculate_thumbnail_workers() -> int:
    """Choose cheap thumbnail concurrency from CPU and usable memory budgets."""
    cpu_budget = calculate_max_workers(min_workers=1)
    usable_memory = get_usable_memory_bytes()
    if usable_memory is None:
        return cpu_budget
    # Avoid excessive in-flight buffers on CPU-rich, low-memory hosts.
    memory_budget = max(1, usable_memory // (1024**3))
    return max(1, min(cpu_budget, memory_budget))


def calculate_high_memory_decode_workers() -> int:
    """Choose a memory-safe concurrency limit for full RAW/HEIC decodes.

    Performance mode may use one full decoder per 8 GiB of usable memory.
    Balanced mode uses at most half that capacity. Custom mode respects both
    the requested thread count and the memory-derived ceiling.
    """
    usable_memory = get_usable_memory_bytes()
    memory_slots = max(1, usable_memory // (8 * 1024**3)) if usable_memory else 1
    mode = get_performance_mode()
    cpu_budget = calculate_max_workers(min_workers=1)
    if mode == PerformanceMode.BALANCED:
        memory_slots = max(1, memory_slots // 2)
    return max(1, min(cpu_budget, memory_slots))


def get_best_shot_batch_size() -> int:
    settings = _get_settings()
    value = settings.value(
        BEST_SHOT_BATCH_SIZE_KEY, DEFAULT_BEST_SHOT_BATCH_SIZE, type=int
    )
    return max(2, int(value))


def set_best_shot_batch_size(batch_size: int) -> None:
    settings = _get_settings()
    settings.setValue(BEST_SHOT_BATCH_SIZE_KEY, max(2, int(batch_size)))


def get_local_best_shot_constants() -> LocalBestShotConstants:
    """Return immutable constants for the local best-shot pipeline."""
    return _LOCAL_BEST_SHOT_CONSTANTS


def get_openai_config() -> dict:
    settings = _get_settings()

    api_key = settings.value(OPENAI_API_KEY_KEY, DEFAULT_OPENAI_API_KEY, type=str)
    model = settings.value(OPENAI_MODEL_KEY, DEFAULT_OPENAI_MODEL, type=str)
    base_url = settings.value(OPENAI_BASE_URL_KEY, DEFAULT_OPENAI_BASE_URL, type=str)
    max_tokens = settings.value(
        OPENAI_MAX_TOKENS_KEY, DEFAULT_OPENAI_MAX_TOKENS, type=int
    )
    timeout = settings.value(OPENAI_TIMEOUT_KEY, DEFAULT_OPENAI_TIMEOUT, type=int)
    max_workers = settings.value(
        OPENAI_MAX_WORKERS_KEY, DEFAULT_OPENAI_MAX_WORKERS, type=int
    )

    best_shot_prompt = settings.value(OPENAI_BEST_SHOT_PROMPT_KEY, None, type=str)
    rating_prompt = settings.value(OPENAI_RATING_PROMPT_KEY, None, type=str)

    config = {
        "api_key": api_key,
        "model": model,
        "base_url": base_url,
        "max_tokens": max_tokens,
        "timeout": timeout,
        "max_workers": max_workers,
        "best_shot_prompt": best_shot_prompt,
        "rating_prompt": rating_prompt,
    }
    # Remove optional None entries for prompts/base_url so dataclass defaults apply
    return {k: v for k, v in config.items() if v is not None or k == "api_key"}


def set_openai_config(
    *,
    api_key: str | None = None,
    model: str | None = None,
    base_url: str | None = None,
    max_tokens: int | None = None,
    timeout: int | None = None,
    max_workers: int | None = None,
    best_shot_prompt: str | None = None,
    rating_prompt: str | None = None,
) -> None:
    settings = _get_settings()

    def _set_or_clear(key: str, value):
        if isinstance(value, str):
            value = value.strip()
        if value is None or value == "":
            settings.remove(key)
        else:
            settings.setValue(key, value)

    if api_key is not None:
        _set_or_clear(OPENAI_API_KEY_KEY, api_key)
    if model is not None:
        _set_or_clear(OPENAI_MODEL_KEY, model)
    if base_url is not None:
        _set_or_clear(OPENAI_BASE_URL_KEY, base_url)
    if max_tokens is not None:
        _set_or_clear(OPENAI_MAX_TOKENS_KEY, max_tokens)
    if timeout is not None:
        _set_or_clear(OPENAI_TIMEOUT_KEY, timeout)
    if max_workers is not None:
        _set_or_clear(OPENAI_MAX_WORKERS_KEY, max_workers)
    if best_shot_prompt is not None:
        _set_or_clear(OPENAI_BEST_SHOT_PROMPT_KEY, best_shot_prompt)
    if rating_prompt is not None:
        _set_or_clear(OPENAI_RATING_PROMPT_KEY, rating_prompt)


def get_location_grouping_depth() -> int:
    settings = _get_settings()
    return max(1, min(3, int(settings.value(LOCATION_GROUPING_DEPTH_KEY, 3, type=int))))


def set_location_grouping_depth(depth: int) -> None:
    _get_settings().setValue(LOCATION_GROUPING_DEPTH_KEY, max(1, min(3, depth)))


def get_companion_files_preference() -> str:
    """Returns 'always', 'never', or 'ask' (default)."""
    settings = _get_settings()
    val = settings.value(COMPANION_FILES_PREFERENCE_KEY, "ask", type=str)
    return val if val in ("always", "never") else "ask"


def set_companion_files_preference(pref: str) -> None:
    """pref must be 'always' or 'never'."""
    if pref in ("always", "never"):
        _get_settings().setValue(COMPANION_FILES_PREFERENCE_KEY, pref)

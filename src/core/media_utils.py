import os

SUPPORTED_IMAGE_EXTENSIONS = {
    ".jpg",
    ".jpeg",
    ".png",
    ".tif",
    ".tiff",  # Standard formats
    ".heic",
    ".heif",  # HEIC/HEIF formats
    ".arw",
    ".cr2",
    ".cr3",
    ".nef",
    ".dng",  # Sony, Canon, Nikon, Adobe RAW
    ".orf",
    ".raf",
    ".rw2",
    ".pef",
    ".srw",  # Olympus, Fuji, Panasonic, Pentax, Samsung RAW
    ".raw",  # Generic RAW
}

SUPPORTED_VIDEO_EXTENSIONS = {
    ".mp4",
    ".mov",
    ".m4v",
    ".avi",
    ".mkv",
    ".mpg",
    ".mpeg",
    ".wmv",
    ".webm",
}

SUPPORTED_MEDIA_EXTENSIONS = SUPPORTED_IMAGE_EXTENSIONS | SUPPORTED_VIDEO_EXTENSIONS


def _normalize_extension(ext_or_path: str) -> str:
    if not ext_or_path:
        return ""
    if ext_or_path.startswith("."):
        return ext_or_path.lower()
    return os.path.splitext(ext_or_path)[1].lower()


def is_video_extension(ext_or_path: str) -> bool:
    return _normalize_extension(ext_or_path) in SUPPORTED_VIDEO_EXTENSIONS


def is_image_extension(ext_or_path: str) -> bool:
    return _normalize_extension(ext_or_path) in SUPPORTED_IMAGE_EXTENSIONS


def infer_media_type(path: str) -> str:
    return "video" if is_video_extension(path) else "image"

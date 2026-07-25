import logging
from collections.abc import Iterable

logger = logging.getLogger(__name__)


def delete_cached_paths(
    paths: Iterable[str],
    *,
    rating_cache=None,
    exif_cache=None,
) -> None:
    """Invalidate path-keyed disk caches from a filesystem worker."""

    for path in dict.fromkeys(path for path in paths if path):
        if rating_cache is not None:
            rating_cache.delete(path)
        if exif_cache is not None:
            exif_cache.delete(path)


def migrate_cached_paths(
    path_updates: dict[str, str] | Iterable[tuple[str, str]],
    *,
    rating_cache=None,
    exif_cache=None,
) -> None:
    """Move disk-cache values to renamed path keys outside the UI thread."""

    pairs = (
        path_updates.items() if isinstance(path_updates, dict) else path_updates
    )
    for old_path, new_path in pairs:
        if not old_path or not new_path or old_path == new_path:
            continue
        if rating_cache is not None:
            rating = rating_cache.get(old_path)
            if rating is not None:
                rating_cache.set(new_path, rating)
                rating_cache.delete(old_path)
        if exif_cache is not None:
            metadata = exif_cache.get(old_path)
            if metadata is not None:
                exif_cache.set(new_path, metadata)
                exif_cache.delete(old_path)

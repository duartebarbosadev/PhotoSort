from typing import Dict, Any, List, Optional
from datetime import date as date_obj
import logging
import os
from core.caching.rating_cache import RatingCache
from core.caching.exif_cache import ExifCache

logger = logging.getLogger(__name__)


class AppState:
    """
    Holds application-level UI state and data caches.
    This helps in making MainWindow less stateful and centralizes data management.
    """

    def __init__(self):
        self.image_files_data: List[
            Dict[str, Any]
        ] = []  # {'path': str, 'is_blurred': Optional[bool]}
        self.rating_cache: Dict[
            str, int
        ] = {}  # This is an in-memory dictionary for quick UI access
        self.date_cache: Dict[str, Optional[date_obj]] = {}
        self.cluster_results: Dict[str, int] = {}  # {image_path: cluster_id}
        self.embeddings_cache: Dict[
            str, List[float]
        ] = {}  # {image_path: embedding_vector}
        self.rating_disk_cache = (
            RatingCache()
        )  # Instance of the new disk cache for ratings
        self.exif_disk_cache = ExifCache()  # Instance of the new disk cache for EXIF data, now reads size from app_settings
        self.marked_for_deletion: set = set()  # Set of file paths marked for deletion

        # Could also hold current folder path, filter states, etc. if desired.
        self.current_folder_path: Optional[str] = None
        self.focused_image_path: Optional[str] = (
            None  # Path of the image in the single/focused viewer
        )

    def clear_all_file_specific_data(self):
        """Clears all data that is specific to a loaded set of files/folder."""
        self.image_files_data.clear()
        self.rating_cache.clear()  # Clears in-memory dict
        self.date_cache.clear()
        self.cluster_results.clear()
        self.embeddings_cache.clear()
        self.marked_for_deletion.clear()  # Clear marked for deletion set
        if self.rating_disk_cache:
            self.rating_disk_cache.clear()  # Decide if folder clear should wipe the whole disk cache
        if self.exif_disk_cache:
            self.exif_disk_cache.clear()  # Decide if folder clear should wipe the whole disk cache
        self.focused_image_path = None
        # self.current_folder_path = None # Optionally reset current folder path

    def remove_data_for_path(self, file_path: str):
        """Removes all cached data associated with a specific file path."""
        logger.info(f"Removing all cached data for file: {os.path.basename(file_path)}")

        original_count = len(self.image_files_data)
        self.image_files_data = [
            fd for fd in self.image_files_data if fd.get("path") != file_path
        ]
        removed_from_image_files = original_count - len(self.image_files_data)

        rating_removed = self.rating_cache.pop(file_path, None)  # In-memory dict
        if self.rating_disk_cache:
            self.rating_disk_cache.delete(file_path)  # Disk cache
        if self.exif_disk_cache:
            self.exif_disk_cache.delete(file_path)  # Exif Disk cache
        date_removed = self.date_cache.pop(file_path, None)
        cluster_removed = self.cluster_results.pop(file_path, None)
        embedding_removed = self.embeddings_cache.pop(file_path, None)

        logger.debug(
            f"Removed data for {os.path.basename(file_path)}: "
            f"image_files_data={removed_from_image_files}, "
            f"rating_cache={rating_removed is not None}, "
            f"date_cache={date_removed is not None}, "
            f"cluster_results={cluster_removed is not None}, "
            f"embeddings_cache={embedding_removed is not None}"
        )

    def update_path(self, old_path: str, new_path: str):
        """Updates all cache entries and data references from an old path to a new path."""
        # Update image_files_data
        file_data = self.get_file_data_by_path(old_path)
        if file_data:
            file_data["path"] = new_path

        # Update in-memory caches
        if old_path in self.rating_cache:
            self.rating_cache[new_path] = self.rating_cache.pop(old_path)
        if old_path in self.date_cache:
            self.date_cache[new_path] = self.date_cache.pop(old_path)
        if old_path in self.cluster_results:
            self.cluster_results[new_path] = self.cluster_results.pop(old_path)
        if old_path in self.embeddings_cache:
            self.embeddings_cache[new_path] = self.embeddings_cache.pop(old_path)

        # Update disk caches
        if self.rating_disk_cache:
            rating_val = self.rating_disk_cache.get(old_path)
            if rating_val is not None:
                self.rating_disk_cache.set(new_path, rating_val)
                self.rating_disk_cache.delete(old_path)

        if self.exif_disk_cache:
            exif_data = self.exif_disk_cache.get(old_path)
            if exif_data is not None:
                self.exif_disk_cache.set(new_path, exif_data)
                self.exif_disk_cache.delete(old_path)

        if self.focused_image_path == old_path:
            self.focused_image_path = new_path

    # Add more methods as needed, e.g., to get specific data,
    # update blur status, etc.
    def update_blur_status(self, file_path: str, is_blurred: Optional[bool]):
        for file_data in self.image_files_data:
            if file_data.get("path") == file_path:
                file_data["is_blurred"] = is_blurred
                return
        # If path not in image_files_data, it might be an error or a new file
        # For now, we assume it should exist if blur status is being updated post-scan.
        logger.warning(
            f"Path not found in image data to update blur status: {file_path}"
        )

    def get_file_data_by_path(self, file_path: str) -> Optional[Dict[str, Any]]:
        for file_data in self.image_files_data:
            if file_data.get("path") == file_path:
                return file_data
        return None

    def mark_for_deletion(self, file_path: str):
        """Marks a file for deletion."""
        logger.info(f"Marking file for deletion: {os.path.basename(file_path)}")
        self.marked_for_deletion.add(file_path)

    def unmark_for_deletion(self, file_path: str):
        """Unmarks a file for deletion."""
        logger.info(f"Unmarking file for deletion: {os.path.basename(file_path)}")
        self.marked_for_deletion.discard(file_path)

    def is_marked_for_deletion(self, file_path: str) -> bool:
        """Checks if a file is marked for deletion."""

        return file_path in self.marked_for_deletion

    def get_marked_files(self) -> List[str]:
        """Returns a list of all files marked for deletion."""
        marked_files = list(self.marked_for_deletion)
        logger.debug(f"Retrieved {len(marked_files)} marked files")
        return marked_files

    def clear_all_deletion_marks(self):
        """Clears all deletion marks."""
        count = len(self.marked_for_deletion)
        logger.info(f"Clearing all deletion marks ({count} files)")
        self.marked_for_deletion.clear()

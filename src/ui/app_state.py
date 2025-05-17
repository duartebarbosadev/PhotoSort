from typing import Dict, Any, List, Optional
from datetime import date as date_obj

from src.core.caching.rating_cache import RatingCache
from src.core.caching.exif_cache import ExifCache # Import ExifCache

class AppState:
    """
    Holds application-level UI state and data caches.
    This helps in making MainWindow less stateful and centralizes data management.
    """
    def __init__(self):
        self.image_files_data: List[Dict[str, Any]] = [] # {'path': str, 'is_blurred': Optional[bool]}
        self.rating_cache: Dict[str, int] = {} # This is an in-memory dictionary for quick UI access
        self.label_cache: Dict[str, Optional[str]] = {}
        self.date_cache: Dict[str, Optional[date_obj]] = {}
        self.cluster_results: Dict[str, int] = {} # {image_path: cluster_id}
        self.embeddings_cache: Dict[str, List[float]] = {} # {image_path: embedding_vector}
        self.rating_disk_cache = RatingCache() # Instance of the new disk cache for ratings
        self.exif_disk_cache = ExifCache() # Instance of the new disk cache for EXIF data
        
        # Could also hold current folder path, filter states, etc. if desired.
        self.current_folder_path: Optional[str] = None

    def clear_all_file_specific_data(self):
        """Clears all data that is specific to a loaded set of files/folder."""
        self.image_files_data.clear()
        self.rating_cache.clear() # Clears in-memory dict
        self.label_cache.clear()
        self.date_cache.clear()
        self.cluster_results.clear()
        self.embeddings_cache.clear()
        if self.rating_disk_cache:
            # self.rating_disk_cache.clear() # Decide if folder clear should wipe the whole disk cache
            pass
        if self.exif_disk_cache:
            # self.exif_disk_cache.clear() # Decide if folder clear should wipe the whole disk cache
            pass # For now, let's assume clearing a folder doesn't wipe persistent disk caches
        # self.current_folder_path = None # Optionally reset current folder path

    def remove_data_for_path(self, file_path: str):
        """Removes all cached data associated with a specific file path."""
        self.image_files_data = [fd for fd in self.image_files_data if fd.get('path') != file_path]
        self.rating_cache.pop(file_path, None) # In-memory dict
        if self.rating_disk_cache:
            self.rating_disk_cache.delete(file_path) # Disk cache
        if self.exif_disk_cache:
            self.exif_disk_cache.delete(file_path) # Exif Disk cache
        self.label_cache.pop(file_path, None)
        self.date_cache.pop(file_path, None)
        self.cluster_results.pop(file_path, None)
        self.embeddings_cache.pop(file_path, None)

    def update_data_for_path_move(self, old_path: str, new_path: str):
        """Updates cache keys when a file is moved."""
        for i, file_data in enumerate(self.image_files_data):
            if file_data.get('path') == old_path:
                self.image_files_data[i]['path'] = new_path
                # is_blurred status is preserved
                break
        
        if old_path in self.rating_cache: # In-memory dict
            self.rating_cache[new_path] = self.rating_cache.pop(old_path)
        if self.rating_disk_cache: # Disk cache for ratings
            rating_val = self.rating_disk_cache.get(old_path)
            if rating_val is not None:
                self.rating_disk_cache.delete(old_path)
                self.rating_disk_cache.set(new_path, rating_val)
        
        if self.exif_disk_cache: # Disk cache for EXIF
            exif_data = self.exif_disk_cache.get(old_path)
            if exif_data is not None:
                self.exif_disk_cache.delete(old_path)
                self.exif_disk_cache.set(new_path, exif_data)

        if old_path in self.label_cache:
            self.label_cache[new_path] = self.label_cache.pop(old_path)
        if old_path in self.date_cache:
            self.date_cache[new_path] = self.date_cache.pop(old_path)
        if old_path in self.cluster_results:
            self.cluster_results[new_path] = self.cluster_results.pop(old_path)
        if old_path in self.embeddings_cache:
            self.embeddings_cache[new_path] = self.embeddings_cache.pop(old_path)

    # Add more methods as needed, e.g., to get specific data,
    # update blur status, etc.
    def update_blur_status(self, file_path: str, is_blurred: Optional[bool]):
        for file_data in self.image_files_data:
            if file_data.get('path') == file_path:
                file_data['is_blurred'] = is_blurred
                return
        # If path not in image_files_data, it might be an error or a new file
        # For now, we assume it should exist if blur status is being updated post-scan.
        print(f"[AppState] Warning: Path {file_path} not found in image_files_data to update blur status.")

    def get_file_data_by_path(self, file_path: str) -> Optional[Dict[str, Any]]:
        for file_data in self.image_files_data:
            if file_data.get('path') == file_path:
                return file_data
        return None

if __name__ == '__main__':
    app_state = AppState()
    app_state.image_files_data.append({'path': '/img/a.jpg', 'is_blurred': False})
    app_state.rating_cache['/img/a.jpg'] = 5
    print(f"Initial state: {app_state.image_files_data}, {app_state.rating_cache}")

    app_state.update_data_for_path_move('/img/a.jpg', '/img_new/a_moved.jpg')
    print(f"After move: {app_state.image_files_data}, {app_state.rating_cache}")
    
    app_state.update_blur_status('/img_new/a_moved.jpg', True)
    print(f"After blur update: {app_state.get_file_data_by_path('/img_new/a_moved.jpg')}")

    app_state.remove_data_for_path('/img_new/a_moved.jpg')
    print(f"After remove: {app_state.image_files_data}, {app_state.rating_cache}")

    app_state.clear_all_file_specific_data()
    print(f"After clear all: {app_state.image_files_data}")
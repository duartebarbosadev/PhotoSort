from dataclasses import dataclass
from typing import Any
from collections.abc import Iterable
from datetime import datetime as datetime_obj
import logging
import os
from core.caching.rating_cache import RatingCache
from core.caching.exif_cache import ExifCache
from core.caching.analysis_cache import AnalysisCache
from core.best_photo_finder.payloads import PickBestResults

logger = logging.getLogger(__name__)


@dataclass(frozen=True, slots=True)
class MediaSummary:
    """Constant-time aggregate information about the loaded media library."""

    total_items: int = 0
    image_count: int = 0
    video_count: int = 0
    total_size_bytes: int = 0


class AppState:
    """
    Holds application-level UI state and data caches.
    This helps in making MainWindow less stateful and centralizes data management.
    """

    def __init__(self):
        self._image_files_data: list[dict[str, Any]] = []
        self._file_data_by_path: dict[str, dict[str, Any]] = {}
        self._media_summary = MediaSummary()
        self.rating_cache: dict[
            str, int
        ] = {}  # This is an in-memory dictionary for quick UI access
        self.date_cache: dict[str, datetime_obj | None] = {}
        self.cluster_results: dict[str, int] = {}  # {image_path: cluster_id}
        self.embeddings_cache: dict[
            str, list[float]
        ] = {}  # {image_path: embedding_vector}
        self.rating_disk_cache = (
            RatingCache()
        )  # Instance of the new disk cache for ratings
        self.exif_disk_cache = ExifCache()  # Instance of the new disk cache for EXIF data, now reads size from app_settings
        self.analysis_cache = AnalysisCache()
        self.marked_for_deletion: set = set()  # Set of file paths marked for deletion
        self.best_shot_rankings: dict[int, list[dict[str, Any]]] = {}
        self.best_shot_scores_by_path: dict[str, dict[str, Any]] = {}
        self.best_shot_winners: dict[int, dict[str, Any]] = {}
        self.ai_rating_results: dict[str, dict[str, Any]] = {}
        self.pick_best_results: PickBestResults = {}
        self.pick_best_winners_by_path: dict[str, bool] = {}  # path -> True if winner
        self.easy_delete_results: dict[str, dict[str, Any]] | None = (
            None  # None = not analysed; {} = analysed with no issues
        )
        self.fix_rotation_results: dict[str, int] | None = (
            None  # None = not analysed; {} = analysed with no suggestions
        )

        # Could also hold current folder path, filter states, etc. if desired.
        self.current_folder_path: str | None = None
        self.focused_image_path: str | None = (
            None  # Path of the image in the single/focused viewer
        )
        self.workflow_step: str = "organize"
        self.selected_grouping_mode: str = "current"
        self.grouping_output_root: str | None = None
        self.grouping_run_summary: dict[str, Any] | None = None
        self.grouping_source_root: str | None = None
        self.skip_grouping_step_once: bool = False

    @property
    def image_files_data(self) -> list[dict[str, Any]]:
        """Loaded file records.

        Assigning a collection rebuilds the path index and aggregate counters. New
        application code should use :meth:`extend_file_data` for scan batches so
        those structures can be updated incrementally.
        """

        return self._image_files_data

    @image_files_data.setter
    def image_files_data(self, records: Iterable[dict[str, Any]]) -> None:
        self._image_files_data = list(records or [])
        self._rebuild_media_index()

    def _rebuild_media_index(self) -> None:
        self._file_data_by_path = {
            record["path"]: record
            for record in self._image_files_data
            if isinstance(record, dict) and record.get("path")
        }
        video_count = sum(
            1
            for record in self._image_files_data
            if record.get("media_type") == "video"
        )
        self._media_summary = MediaSummary(
            total_items=len(self._image_files_data),
            image_count=len(self._image_files_data) - video_count,
            video_count=video_count,
            total_size_bytes=sum(
                int(record.get("file_size") or 0) for record in self._image_files_data
            ),
        )

    def extend_file_data(self, records: Iterable[dict[str, Any]]) -> None:
        """Add a scan batch while maintaining indexes and counters in O(batch)."""

        batch = list(records)
        if not batch:
            return
        self._image_files_data.extend(batch)
        for record in batch:
            path = record.get("path")
            if path:
                self._file_data_by_path[path] = record

        added_videos = sum(1 for item in batch if item.get("media_type") == "video")
        previous = self._media_summary
        self._media_summary = MediaSummary(
            total_items=previous.total_items + len(batch),
            image_count=previous.image_count + len(batch) - added_videos,
            video_count=previous.video_count + added_videos,
            total_size_bytes=previous.total_size_bytes
            + sum(int(item.get("file_size") or 0) for item in batch),
        )

    def media_summary(self) -> MediaSummary:
        """Return precomputed media counts and total byte size."""

        return self._media_summary

    def clear_all_file_specific_data(self, clear_disk_caches: bool = False):
        """Clears file/folder-scoped state and optionally disk caches."""
        folder_path = self.current_folder_path
        self.image_files_data = []
        self.rating_cache.clear()  # Clears in-memory dict
        self.date_cache.clear()
        self.cluster_results.clear()
        self.embeddings_cache.clear()
        self.marked_for_deletion.clear()  # Clear marked for deletion set
        if clear_disk_caches and self.rating_disk_cache:
            self.rating_disk_cache.clear()
        if clear_disk_caches and self.exif_disk_cache:
            self.exif_disk_cache.clear()
        if clear_disk_caches and folder_path and self.analysis_cache:
            self.analysis_cache.clear_folder(folder_path)
        self.focused_image_path = None
        self.clear_best_shot_results()
        self.clear_pick_best_results()
        self.ai_rating_results.clear()
        self.easy_delete_results = None
        self.fix_rotation_results = None
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
        removed_best = self.best_shot_scores_by_path.pop(file_path, None)
        if cluster_removed is None and removed_best:
            cluster_removed = removed_best.get("cluster_id")
        if cluster_removed is not None:
            rankings = self.best_shot_rankings.get(cluster_removed)
            if rankings:
                self.best_shot_rankings[cluster_removed] = [
                    r for r in rankings if r.get("image_path") != file_path
                ]
                if not self.best_shot_rankings[cluster_removed]:
                    self.best_shot_rankings.pop(cluster_removed, None)
            winner = self.best_shot_winners.get(cluster_removed)
            if winner and winner.get("image_path") == file_path:
                self.best_shot_winners.pop(cluster_removed, None)

        self.ai_rating_results.pop(file_path, None)
        self.marked_for_deletion.discard(file_path)

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
            self._file_data_by_path.pop(old_path, None)
            self._file_data_by_path[new_path] = file_data

        # Update in-memory caches
        if old_path in self.rating_cache:
            self.rating_cache[new_path] = self.rating_cache.pop(old_path)
        if old_path in self.date_cache:
            self.date_cache[new_path] = self.date_cache.pop(old_path)
        if old_path in self.cluster_results:
            self.cluster_results[new_path] = self.cluster_results.pop(old_path)
        if old_path in self.embeddings_cache:
            self.embeddings_cache[new_path] = self.embeddings_cache.pop(old_path)
        if old_path in self.best_shot_scores_by_path:
            self.best_shot_scores_by_path[new_path] = self.best_shot_scores_by_path.pop(
                old_path
            )
        for ranking in self.best_shot_rankings.values():
            for result in ranking:
                if result.get("image_path") == old_path:
                    result["image_path"] = new_path
        for winner in self.best_shot_winners.values():
            if winner.get("image_path") == old_path:
                winner["image_path"] = new_path
        if old_path in self.ai_rating_results:
            self.ai_rating_results[new_path] = self.ai_rating_results.pop(old_path)

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
        if old_path in self.marked_for_deletion:
            self.marked_for_deletion.discard(old_path)
            self.marked_for_deletion.add(new_path)

    # Add more methods as needed, e.g., to get specific data,
    # update blur status, etc.
    def update_blur_status(self, file_path: str, is_blurred: bool | None):
        file_data = self.get_file_data_by_path(file_path)
        if file_data is not None:
            file_data["is_blurred"] = is_blurred
            return
        # If path not in image_files_data, it might be an error or a new file
        # For now, we assume it should exist if blur status is being updated post-scan.
        logger.warning(
            f"Path not found in image data to update blur status: {file_path}"
        )

    def get_file_data_by_path(self, file_path: str) -> dict[str, Any] | None:
        return self._file_data_by_path.get(file_path)

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

    def get_marked_files(self) -> list[str]:
        """Returns a list of all files marked for deletion."""
        marked_files = list(self.marked_for_deletion)
        logger.debug(f"Retrieved {len(marked_files)} marked files")
        return marked_files

    def clear_all_deletion_marks(self):
        """Clears all deletion marks."""
        count = len(self.marked_for_deletion)
        logger.info(f"Clearing all deletion marks ({count} files)")
        self.marked_for_deletion.clear()

    def clear_best_shot_results(self):
        """Resets cached best-shot data."""
        self.best_shot_rankings.clear()
        self.best_shot_scores_by_path.clear()
        self.best_shot_winners.clear()

    def is_best_shot_winner(self, file_path: str) -> bool:
        """Check winner status in O(1) for normal ranked results."""

        score = self.best_shot_scores_by_path.get(file_path)
        if score is not None:
            cluster_id = score.get("cluster_id")
            winner = (
                self.best_shot_winners.get(cluster_id)
                if isinstance(cluster_id, int)
                else None
            )
            if winner is not None:
                return winner.get("image_path") == file_path
        return False

    def clear_pick_best_results(self):
        """Resets pick-best step results."""
        self.pick_best_results.clear()
        self.pick_best_winners_by_path.clear()

    def merge_best_shot_results(
        self, rankings_by_cluster: dict[int, list[dict[str, Any]]]
    ) -> None:
        for cluster_id, rankings in rankings_by_cluster.items():
            if not rankings:
                continue
            normalized_rankings: list[dict[str, Any]] = []
            for entry in rankings:
                if not isinstance(entry, dict):
                    continue
                normalized = dict(entry)
                normalized.setdefault("cluster_id", cluster_id)
                normalized_rankings.append(normalized)
                path = normalized.get("image_path")
                if path:
                    self.best_shot_scores_by_path[path] = normalized
            if not normalized_rankings:
                continue
            self.best_shot_rankings[cluster_id] = normalized_rankings
            self.best_shot_winners[cluster_id] = normalized_rankings[0]

    def set_best_shot_results(
        self, rankings_by_cluster: dict[int, list[dict[str, Any]]]
    ):
        """Persist best-shot rankings emitted by the analysis worker."""
        self.clear_best_shot_results()
        self.merge_best_shot_results(rankings_by_cluster)

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
        self.detailed_metadata_cache: dict[str, dict[str, Any]] = {}
        self.cluster_results: dict[str, int] = {}  # {image_path: cluster_id}
        self.embeddings_cache: dict[
            str, list[float]
        ] = {}  # {image_path: embedding_vector}
        self.regional_embeddings_cache: dict[str, list[list[float]]] = {}
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
        self.easy_delete_pair_assessments: dict[tuple[str, str], dict[str, Any]] = {}
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
        self.detailed_metadata_cache.clear()
        self.cluster_results.clear()
        self.embeddings_cache.clear()
        self.regional_embeddings_cache.clear()
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
        self.easy_delete_pair_assessments.clear()
        self.fix_rotation_results = None
        # self.current_folder_path = None # Optionally reset current folder path

    def remove_data_for_path(self, file_path: str):
        """Remove one path while preserving the batch implementation as owner."""

        self.remove_data_for_paths([file_path])

    def invalidate_similarity_for_paths(
        self,
        file_paths: Iterable[str],
        *,
        invalidate_disk_cache: bool = True,
        preserve_review_results: bool = False,
    ) -> None:
        """Invalidate shared similarity state after source-file mutations.

        Deletion callers may preserve already-pruned Easy Delete and Pick Best
        reviews because removing a file does not change the analysis of
        surviving photos. Content-changing mutations such as rotation continue
        to invalidate those review results completely.
        """

        changed_paths = {path for path in file_paths if isinstance(path, str) and path}
        for path in changed_paths:
            self.embeddings_cache.pop(path, None)
            self.regional_embeddings_cache.pop(path, None)
            record = self.get_file_data_by_path(path)
            if record is not None:
                try:
                    stat_result = os.stat(path)
                except OSError:
                    pass
                else:
                    record["file_size"] = stat_result.st_size
                    record["mtime_ns"] = stat_result.st_mtime_ns

        self.cluster_results.clear()
        ad_hoc_cluster_id = -1
        self.best_shot_rankings = {
            cluster_id: rankings
            for cluster_id, rankings in self.best_shot_rankings.items()
            if cluster_id == ad_hoc_cluster_id
        }
        self.best_shot_winners = {
            cluster_id: winner
            for cluster_id, winner in self.best_shot_winners.items()
            if cluster_id == ad_hoc_cluster_id
        }
        self.best_shot_scores_by_path = {
            path: score
            for path, score in self.best_shot_scores_by_path.items()
            if score.get("cluster_id") == ad_hoc_cluster_id
        }
        if not preserve_review_results:
            self.clear_pick_best_results()
            self.easy_delete_results = None
            self.easy_delete_pair_assessments.clear()

        if invalidate_disk_cache and self.current_folder_path and self.analysis_cache:
            self.analysis_cache.invalidate_similarity(self.current_folder_path)

    def remove_data_for_paths(
        self,
        file_paths: Iterable[str],
        *,
        clear_disk_caches: bool = True,
    ) -> int:
        """Remove file-scoped state in one pass over every shared structure.

        ``clear_disk_caches`` is disabled when a filesystem worker already
        invalidated the disk caches. Keeping that I/O off the UI thread is
        important for large deletion batches.
        """

        removed_paths = {path for path in file_paths if isinstance(path, str) and path}
        if not removed_paths:
            return 0

        original_count = len(self._image_files_data)
        self._image_files_data = [
            record
            for record in self._image_files_data
            if record.get("path") not in removed_paths
        ]
        self._rebuild_media_index()

        for cache in (
            self.rating_cache,
            self.date_cache,
            self.detailed_metadata_cache,
            self.cluster_results,
            self.embeddings_cache,
            self.regional_embeddings_cache,
            self.best_shot_scores_by_path,
            self.ai_rating_results,
        ):
            for path in removed_paths:
                cache.pop(path, None)

        if clear_disk_caches:
            for path in removed_paths:
                if self.rating_disk_cache:
                    self.rating_disk_cache.delete(path)
                if self.exif_disk_cache:
                    self.exif_disk_cache.delete(path)

        for cluster_id, rankings in list(self.best_shot_rankings.items()):
            retained = [
                entry
                for entry in rankings
                if entry.get("image_path") not in removed_paths
            ]
            if retained:
                self.best_shot_rankings[cluster_id] = retained
            else:
                self.best_shot_rankings.pop(cluster_id, None)
        for cluster_id, winner in list(self.best_shot_winners.items()):
            if winner.get("image_path") in removed_paths:
                self.best_shot_winners.pop(cluster_id, None)

        self.marked_for_deletion.difference_update(removed_paths)
        if self.focused_image_path in removed_paths:
            self.focused_image_path = None

        if self.easy_delete_results is not None:
            self.easy_delete_results = {
                path: entry
                for path, entry in self.easy_delete_results.items()
                if path not in removed_paths
                and entry.get("pair_path") not in removed_paths
            }
        self.easy_delete_pair_assessments = {
            pair: assessment
            for pair, assessment in self.easy_delete_pair_assessments.items()
            if not set(pair).intersection(removed_paths)
        }
        if self.fix_rotation_results is not None:
            self.fix_rotation_results = {
                path: angle
                for path, angle in self.fix_rotation_results.items()
                if path not in removed_paths
            }

        invalid_pick_best_paths: set[str] = set()
        for cluster_id, cluster in list(self.pick_best_results.items()):
            cluster_paths = set(cluster.get("all_paths", []))
            if cluster_paths.intersection(removed_paths):
                invalid_pick_best_paths.update(cluster_paths)
                self.pick_best_results.pop(cluster_id, None)
        invalid_pick_best_paths.update(removed_paths)
        for path in invalid_pick_best_paths:
            self.pick_best_winners_by_path.pop(path, None)

        self.invalidate_similarity_for_paths(
            removed_paths,
            invalidate_disk_cache=False,
            preserve_review_results=True,
        )

        removed_count = original_count - len(self._image_files_data)
        logger.info(
            "Removed shared data for %d path(s) in one batch (%d media records).",
            len(removed_paths),
            removed_count,
        )
        return removed_count

    def _remove_path_from_workflow_results(self, file_path: str) -> None:
        """Invalidate analysis results that can no longer be reviewed safely."""

        if self.easy_delete_results is not None:
            invalid_reviews = [
                review_path
                for review_path, entry in self.easy_delete_results.items()
                if review_path == file_path or entry.get("pair_path") == file_path
            ]
            for review_path in invalid_reviews:
                self.easy_delete_results.pop(review_path, None)
        self.easy_delete_pair_assessments = {
            pair: assessment
            for pair, assessment in self.easy_delete_pair_assessments.items()
            if file_path not in pair
        }

        if self.fix_rotation_results is not None:
            self.fix_rotation_results.pop(file_path, None)

        invalid_clusters = [
            cluster_id
            for cluster_id, cluster in self.pick_best_results.items()
            if file_path in cluster.get("all_paths", [])
        ]
        for cluster_id in invalid_clusters:
            self.pick_best_results.pop(cluster_id, None)
        self.pick_best_winners_by_path.pop(file_path, None)

    def update_path(self, old_path: str, new_path: str):
        """Update one path while preserving the batch implementation as owner."""

        self.update_paths({old_path: new_path})

    def update_paths(
        self,
        path_updates: dict[str, str] | Iterable[tuple[str, str]],
        *,
        migrate_disk_caches: bool = True,
    ) -> int:
        """Rename references with one pass over each shared result collection."""

        pairs = path_updates.items() if isinstance(path_updates, dict) else path_updates
        updates = {
            old_path: new_path
            for old_path, new_path in pairs
            if isinstance(old_path, str)
            and isinstance(new_path, str)
            and old_path
            and new_path
            and old_path != new_path
        }
        if not updates:
            return 0

        def remap_optional_path(path: object) -> object:
            if isinstance(path, str):
                return updates.get(path, path)
            return path

        for record in self._image_files_data:
            old_path = record.get("path")
            if old_path in updates:
                record["path"] = updates[old_path]
        self._file_data_by_path = {
            record["path"]: record
            for record in self._image_files_data
            if record.get("path")
        }

        def remap_keys(cache: dict) -> None:
            moved = [
                (updates[path], cache.pop(path))
                for path in tuple(updates)
                if path in cache
            ]
            cache.update(moved)

        for cache in (
            self.rating_cache,
            self.date_cache,
            self.detailed_metadata_cache,
            self.cluster_results,
            self.embeddings_cache,
            self.regional_embeddings_cache,
            self.best_shot_scores_by_path,
            self.ai_rating_results,
            self.pick_best_winners_by_path,
        ):
            remap_keys(cache)

        for rankings in self.best_shot_rankings.values():
            for result in rankings:
                path = result.get("image_path")
                if path in updates:
                    result["image_path"] = updates[path]
        for winner in self.best_shot_winners.values():
            path = winner.get("image_path")
            if path in updates:
                winner["image_path"] = updates[path]

        if migrate_disk_caches:
            for old_path, new_path in updates.items():
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

        if self.focused_image_path in updates:
            self.focused_image_path = updates[self.focused_image_path]
        if self.easy_delete_results is not None:
            self.easy_delete_results = {
                updates.get(path, path): {
                    **entry,
                    "pair_path": remap_optional_path(entry.get("pair_path")),
                }
                for path, entry in self.easy_delete_results.items()
            }
        self.easy_delete_pair_assessments = {
            tuple(
                sorted((updates.get(pair[0], pair[0]), updates.get(pair[1], pair[1])))
            ): (assessment)
            for pair, assessment in self.easy_delete_pair_assessments.items()
        }
        if self.fix_rotation_results is not None:
            self.fix_rotation_results = {
                updates.get(path, path): angle
                for path, angle in self.fix_rotation_results.items()
            }
        for cluster in self.pick_best_results.values():
            winner_path = cluster.get("winner_path")
            if winner_path in updates:
                cluster["winner_path"] = updates[winner_path]
            cluster["all_paths"] = [
                updates.get(path, path) for path in cluster["all_paths"]
            ]
            cluster["unsupported_paths"] = [
                updates.get(path, path) for path in cluster["unsupported_paths"]
            ]
            for entries in (cluster["ranked"], cluster["failed"]):
                for score_entry in entries:
                    path = score_entry.get("path")
                    if path in updates:
                        score_entry["path"] = updates[path]
            mark_state = cluster.get("_mark_state")
            if isinstance(mark_state, dict):
                cluster["_mark_state"] = {
                    updates.get(path, path): marked
                    for path, marked in mark_state.items()
                }

        self.marked_for_deletion = {
            updates.get(path, path) for path in self.marked_for_deletion
        }
        self.invalidate_similarity_for_paths(
            updates.values(),
            invalidate_disk_cache=migrate_disk_caches,
        )
        logger.info("Updated shared references for %d renamed path(s).", len(updates))
        return len(updates)

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

    def set_deletion_marks(self, mark_state: dict[str, bool]) -> int:
        """Apply many deletion marks atomically without per-path logging."""

        to_mark = {
            path
            for path, marked in mark_state.items()
            if marked and path not in self.marked_for_deletion
        }
        to_unmark = {
            path
            for path, marked in mark_state.items()
            if not marked and path in self.marked_for_deletion
        }
        self.marked_for_deletion.update(to_mark)
        self.marked_for_deletion.difference_update(to_unmark)
        changed = len(to_mark) + len(to_unmark)
        if changed:
            logger.info(
                "Updated deletion marks in bulk: %d marked, %d unmarked",
                len(to_mark),
                len(to_unmark),
            )
        return changed

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

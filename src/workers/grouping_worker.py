import logging
from typing import Any

from PyQt6.QtCore import QObject, pyqtSignal

from core.grouping import (
    GroupingAnalysisCancelled,
    GroupingMode,
    augment_grouping_plan_with_filesystem_paths,
    build_grouping_output_root,
    build_grouping_plan,
    execute_grouping_plan,
)
from core.image_pipeline import ImagePipeline
from core.caching.path_cache_ops import migrate_cached_paths

logger = logging.getLogger(__name__)


class GroupingPreviewWorker(QObject):
    progress_update = pyqtSignal(int, str)
    preview_ready = pyqtSignal(object)
    error = pyqtSignal(str)
    finished = pyqtSignal()

    def __init__(
        self,
        items: list[dict[str, Any]],
        mode: str,
        source_root: str | None = None,
        location_depth: int = 3,
        image_pipeline: ImagePipeline | None = None,
        parent=None,
    ):
        super().__init__(parent)
        self.items = items
        self.mode = mode
        self.source_root = source_root
        self.location_depth = location_depth
        self.image_pipeline = image_pipeline
        self._should_stop = False
        self._similarity_engine = None

    def stop(self):
        self._should_stop = True
        if self._similarity_engine is not None:
            self._similarity_engine.stop()

    def run(self):
        try:
            if self._should_stop:
                return
            self.progress_update.emit(10, "Preparing grouping preview...")
            mode = GroupingMode(self.mode)
            if mode in {GroupingMode.SIMILARITY, GroupingMode.MIXED}:
                from core.similarity_engine import SimilarityEngine

                self._similarity_engine = SimilarityEngine(
                    image_pipeline=self.image_pipeline
                )
                if self._should_stop:
                    self._similarity_engine.stop()
                    return
            plan = build_grouping_plan(
                self.items,
                mode,
                progress_callback=self.progress_update.emit,
                source_root=self.source_root,
                location_depth=self.location_depth,
                image_pipeline=self.image_pipeline,
                should_continue=lambda: not self._should_stop,
                similarity_engine=self._similarity_engine,
            )
            plan = augment_grouping_plan_with_filesystem_paths(
                plan,
                self.source_root,
            )
            if self._should_stop:
                return
            self.progress_update.emit(100, "Grouping preview ready.")
            self.preview_ready.emit(plan)
        except GroupingAnalysisCancelled:
            logger.info("Grouping preview cancelled.")
        except Exception as exc:
            logger.error("Grouping preview failed: %s", exc, exc_info=True)
            self.error.emit(str(exc))
        finally:
            self.finished.emit()


class GroupingWorkflowWorker(QObject):
    progress_update = pyqtSignal(int, str)
    completed = pyqtSignal(object)
    error = pyqtSignal(str)
    finished = pyqtSignal()

    def __init__(
        self,
        items: list[dict[str, Any]],
        mode: str,
        source_root: str,
        output_root: str | None = None,
        group_name_overrides: dict[str, str] | None = None,
        prepared_plan=None,
        location_depth: int = 3,
        move_companions: bool = False,
        image_pipeline: ImagePipeline | None = None,
        rating_cache=None,
        exif_cache=None,
        analysis_cache=None,
        parent=None,
    ):
        super().__init__(parent)
        self.items = items
        self.mode = mode
        self.source_root = source_root
        self.output_root = output_root or build_grouping_output_root(source_root, mode)
        self.group_name_overrides = dict(group_name_overrides or {})
        self.prepared_plan = prepared_plan
        self.location_depth = location_depth
        self.move_companions = move_companions
        self.image_pipeline = image_pipeline
        self.rating_cache = rating_cache
        self.exif_cache = exif_cache
        self.analysis_cache = analysis_cache
        self._should_stop = False

    def stop(self):
        self._should_stop = True

    def run(self):
        try:
            if self._should_stop:
                return
            self.progress_update.emit(5, "Analyzing grouping candidates...")
            if self.prepared_plan is not None:
                plan = self.prepared_plan
            else:
                plan = build_grouping_plan(
                    self.items,
                    GroupingMode(self.mode),
                    progress_callback=self.progress_update.emit,
                    source_root=self.source_root,
                    location_depth=self.location_depth,
                    image_pipeline=self.image_pipeline,
                )
            plan = augment_grouping_plan_with_filesystem_paths(
                plan,
                self.source_root,
            )
            plan.apply_group_label_overrides(self.group_name_overrides)
            plan.output_root = self.output_root
            if self._should_stop:
                return
            self.progress_update.emit(20, "Creating grouped folders...")
            summary = execute_grouping_plan(
                plan,
                source_root=self.source_root,
                output_root=self.output_root,
                progress_callback=self.progress_update.emit,
                move_companions=self.move_companions,
            )
            path_updates = {
                entry.original_path: entry.new_path
                for entry in summary.entries
                if entry.new_path
            }
            migrate_cached_paths(
                path_updates,
                rating_cache=self.rating_cache,
                exif_cache=self.exif_cache,
            )
            if self.analysis_cache is not None:
                self.analysis_cache.migrate_folder_paths(
                    self.source_root,
                    self.output_root,
                    path_updates,
                )
            if self._should_stop:
                return
            self.progress_update.emit(100, "Grouping complete.")
            self.completed.emit(summary)
        except Exception as exc:
            logger.error("Grouping workflow failed: %s", exc, exc_info=True)
            self.error.emit(str(exc))
        finally:
            self.finished.emit()

from __future__ import annotations

import logging
from typing import Any, Dict, List, Optional

from PyQt6.QtCore import QObject, pyqtSignal

from core.grouping import (
    GroupingMode,
    build_grouping_output_root,
    build_grouping_plan,
    execute_grouping_plan,
)

logger = logging.getLogger(__name__)


class GroupingPreviewWorker(QObject):
    progress_update = pyqtSignal(int, str)
    preview_ready = pyqtSignal(object)
    error = pyqtSignal(str)
    finished = pyqtSignal()

    def __init__(
        self,
        items: List[Dict[str, Any]],
        mode: str,
        source_root: Optional[str] = None,
        parent=None,
    ):
        super().__init__(parent)
        self.items = items
        self.mode = mode
        self.source_root = source_root
        self._should_stop = False

    def stop(self):
        self._should_stop = True

    def run(self):
        try:
            if self._should_stop:
                return
            self.progress_update.emit(10, "Preparing grouping preview...")
            plan = build_grouping_plan(
                self.items,
                GroupingMode(self.mode),
                progress_callback=self.progress_update.emit,
                source_root=self.source_root,
            )
            if self._should_stop:
                return
            self.progress_update.emit(100, "Grouping preview ready.")
            self.preview_ready.emit(plan)
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
        items: List[Dict[str, Any]],
        mode: str,
        source_root: str,
        output_root: Optional[str] = None,
        group_name_overrides: Optional[Dict[str, str]] = None,
        prepared_plan=None,
        parent=None,
    ):
        super().__init__(parent)
        self.items = items
        self.mode = mode
        self.source_root = source_root
        self.output_root = output_root or build_grouping_output_root(source_root, mode)
        self.group_name_overrides = dict(group_name_overrides or {})
        self.prepared_plan = prepared_plan
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

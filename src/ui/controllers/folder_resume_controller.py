"""Coordinate per-folder, per-workflow presentation bookmarks."""

from copy import deepcopy
from functools import partial
import logging

from PyQt6.QtCore import QObject, QTimer
from PyQt6.QtWidgets import QApplication

from core.folder_view_store import (
    FolderViewStore,
    WORKFLOWS,
    bookmark_paths,
    folder_key,
)

logger = logging.getLogger(__name__)


class FolderResumeController(QObject):
    def __init__(self, context, store=None):
        super().__init__(context)
        self.context = context
        self.store = store or FolderViewStore()
        self.folder = None
        self.bookmark = {"workflow": "organize", "views": {}}
        self._last_saved = None
        self._last_saved_identities = None
        self._loaded_identities = {}
        self._generation = 0
        self._loaded = False
        self._active = False
        self._restoring = False
        self._ready = set()
        self._pending = set()
        self._activation = None
        self._opening_step = None
        self._timer = QTimer(self)
        self._timer.setInterval(1000)
        self._timer.timeout.connect(self.checkpoint)
        self._timer.start()

    def begin_folder(self, folder: str) -> None:
        self.checkpoint()
        self._generation += 1
        generation = self._generation
        self.folder = folder_key(folder)
        self.bookmark = {"workflow": "organize", "views": {}}
        self._last_saved = None
        self._last_saved_identities = None
        self._loaded_identities = {}
        self._loaded = self._active = False
        self._ready.clear()
        self._pending.clear()
        self._activation = None
        self._opening_step = None

        def loaded(bookmark):
            if generation != self._generation:
                return
            if bookmark:
                self._loaded_identities = bookmark.pop("identities", {})
                self.bookmark = bookmark
            self._last_saved = deepcopy(self.bookmark)
            self._last_saved_identities = deepcopy(self._loaded_identities)
            self._loaded = True
            self._pending = set(self.bookmark["views"])
            callback, self._activation = self._activation, None
            if callback:
                callback()

        self.context.worker_manager.submit_folder_view_io(
            partial(
                self.store.load,
                self.folder,
                include_identities=True,
                should_continue=lambda: generation == self._generation,
            ),
            loaded,
        )

    def wait_for_bookmark(self, callback) -> bool:
        if self.folder and not self._loaded:
            self._activation = callback
            return True
        return False

    def activate(self, *, use_saved_workflow=True) -> str:
        self._active = True
        step = self.bookmark["workflow"] if use_saved_workflow else "cull"
        if not self.context._is_workflow_step_visible(step):
            step = next(
                (s for s in WORKFLOWS if self.context._is_workflow_step_visible(s)),
                "organize",
            )
        organize = self.bookmark["views"].get("organize", {})
        mode = organize.get("mode", "current")
        if (
            isinstance(mode, str)
            and mode in self.context.grouping_step_widget._mode_buttons
        ):
            self.context.app_state.selected_grouping_mode = mode
        self._opening_step = step
        return step

    def before_workflow_change(self, destination: str) -> None:
        self.checkpoint()
        if destination in self.bookmark["views"]:
            self._pending.add(destination)

    def prepare_workflow(self, step: str) -> None:
        if self.remembers(step) and step in self._pending:
            adapter = self.context.get_active_image_adapter(step)
            prepare = getattr(adapter, "prepare_view_bookmark", None)
            if prepare:
                self._restoring = True
                try:
                    prepare(self.bookmark["views"][step])
                finally:
                    self._restoring = False

    def data_loading(self, step: str) -> None:
        """Keep a bookmark pending while an existing workflow rebuilds its data."""
        self.checkpoint()
        self._ready.discard(step)
        if self.remembers(step):
            self._pending.add(step)

    def data_ready(self, step: str) -> None:
        self._ready.add(step)

    def remembers(self, step: str) -> bool:
        return self._active and step in self.bookmark["views"]

    def restore_workflow(self, step: str) -> bool:
        """Return True when a bookmark owns focus (including while data loads)."""
        if not self.remembers(step):
            return False
        if step != self.context.app_state.workflow_step:
            return True
        if step not in self._pending or self._restoring:
            return True
        if step not in self._ready:
            return True
        generation = self._generation
        self._restoring = True

        def restore():
            try:
                if (
                    generation != self._generation
                    or step != self.context.app_state.workflow_step
                    or step not in self._ready
                ):
                    return
                with self.context.active_image_controller.restoring_view():
                    adapter = self.context.get_active_image_adapter(step)
                    state = self.bookmark["views"][step]
                    if adapter and adapter.restore_view_bookmark(state):
                        self._pending.discard(step)
                        current = adapter.capture_view_bookmark()
                        if current:
                            self.context.app_state.focused_image_path = current.get(
                                "path"
                            )
            except TypeError, ValueError, KeyError:
                logger.warning("Ignoring invalid %s view bookmark", step, exc_info=True)
                self._pending.discard(step)
            finally:
                self._restoring = False

        QTimer.singleShot(0, restore)
        return True

    def checkpoint(self) -> None:
        if (
            not self._active
            or self._restoring
            or not self.folder
            or QApplication.activeModalWidget() is not None
            or getattr(self.context, "_model_population_total", 0)
        ):
            return
        step = self.context.app_state.workflow_step
        if self._opening_step is not None:
            if step != self._opening_step:
                return
            self._opening_step = None
        if step not in WORKFLOWS:
            return
        if step in self._pending and step in self._ready:
            self.restore_workflow(step)
            return
        self.bookmark["workflow"] = step
        if step in self._ready and step not in self._pending:
            adapter = self.context.get_active_image_adapter(step)
            state = adapter.capture_view_bookmark() if adapter else None
            if state is not None:
                self.bookmark["views"][step] = state
        # Snapshot identities from the scan, not from the current filesystem: an
        # external replacement must not inherit an old selection during autosave.
        identities = {}
        for path in bookmark_paths(self.bookmark):
            record = self.context.app_state.get_file_data_by_path(path)
            identity = self._loaded_identities.get(path)
            if record is not None:
                identity = record.get("bookmark_identity")
            if isinstance(identity, list):
                identities[path] = identity.copy()
        if (
            self.bookmark == self._last_saved
            and identities == self._last_saved_identities
        ):
            return
        snapshot = deepcopy(self.bookmark)
        self._last_saved = snapshot
        self._last_saved_identities = identities
        self.context.worker_manager.submit_folder_view_io(
            partial(self.store.save, self.folder, snapshot, identities=identities)
        )

    def prepare_close(self) -> None:
        self.checkpoint()
        self._timer.stop()
        self._active = False
        self._generation += 1
        self._activation = None

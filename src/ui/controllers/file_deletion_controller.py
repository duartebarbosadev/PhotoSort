import os
import logging
from PyQt6.QtCore import QModelIndex, QItemSelection, QTimer, QItemSelectionModel
from PyQt6.QtWidgets import QAbstractItemView

logger = logging.getLogger(__name__)


class FileDeletionContextProtocol:
    """Protocol-like duck interface used by FileDeletionController.

    We intentionally avoid importing typing.Protocol to keep this lightweight.
    The provided context (MainWindow) must supply these attributes/methods.
    """

    # Attributes expected:
    file_system_model: object
    proxy_model: object
    app_controller: object
    app_state: object
    dialog_manager: object
    advanced_image_viewer: object
    show_folders_mode: bool
    group_by_similarity_mode: bool

    # Methods expected:
    def _get_active_file_view(self): ...  # returns QAbstractItemView or None
    def _get_selected_file_paths_from_view(self) -> list[str]: ...
    def _get_all_visible_image_paths(self) -> list[str]: ...
    def _find_proxy_index_for_path(self, path: str): ...
    def _handle_file_selection_changed(self, override_selected_paths=None): ...
    def _update_image_info_label(self): ...
    def statusBar(self): ...


class FileDeletionController:
    """Encapsulates complex multi-step deletion logic from MainWindow.

    Handles:
      - Determining which paths to delete (focused vs selection)
      - Confirm dialog
      - Removing items from the model in correct order
      - Cleaning up empty group headers
      - Restoring a sensible selection/focus afterwards
    """

    def __init__(self, ctx: FileDeletionContextProtocol):
        self.ctx = ctx
        # Stateful flags mirrored from previous MainWindow logic
        self.original_selection_paths: list[str] = []
        self.was_focused_delete: bool = False

    # Public API
    def move_current_image_to_trash(self):
        view = self.ctx._get_active_file_view()
        if not view:
            return

        logger.debug("Initiating file deletion process (controller).")

        self.original_selection_paths = self.ctx._get_selected_file_paths_from_view()
        visible_paths_before_delete = self.ctx._get_all_visible_image_paths()
        focused_path = self.ctx.advanced_image_viewer.get_focused_image_path_if_any()

        if focused_path:
            target_paths = [focused_path]
            self.was_focused_delete = True
            logger.debug("Deleting focused image: %s", os.path.basename(focused_path))
        else:
            target_paths = self.original_selection_paths
            self.was_focused_delete = False

        if not target_paths:
            self.ctx.statusBar().showMessage("No image(s) selected to delete.", 3000)
            return

        if not self.ctx.dialog_manager.show_confirm_delete_dialog(target_paths):
            return

        start_batch = getattr(self.ctx, "_start_deletion_batch", None)
        if not callable(start_batch):
            logger.error("Background deletion service is unavailable.")
            self.ctx.statusBar().showMessage(
                "Deletion service is unavailable.", 3000
            )
            return

        started = start_batch(
            target_paths,
            {path: [path] for path in target_paths},
            completion=(
                lambda successful_targets, deleted_paths, failures, _resolved: (
                    self._finish_background_delete(
                        view,
                        visible_paths_before_delete,
                        target_paths,
                        successful_targets,
                        deleted_paths,
                        failures,
                    )
                )
            ),
        )
        if not started:
            self.ctx.statusBar().showMessage(
                "A deletion is already in progress.", 3000
            )

    def _finish_background_delete(
        self,
        view: QAbstractItemView,
        visible_paths_before_delete: list[str],
        target_paths: list[str],
        successful_targets: list[str],
        deleted_paths: list[str],
        failures: dict[str, str],
    ) -> None:
        if deleted_paths:
            self.ctx.statusBar().showMessage(
                f"{len(successful_targets)} image(s) moved to trash.", 5000
            )
            view.selectionModel().clearSelection()
            visible_after = self.ctx._get_all_visible_image_paths()
            self._restore_selection_after_delete(
                visible_paths_before_delete,
                visible_after,
                deleted_paths,
                view,
            )
            self.ctx._update_image_info_label()
        if failures:
            self.ctx.statusBar().showMessage(
                f"{len(successful_targets)} moved to Trash; "
                f"{len(failures)} failed.",
                5000,
            )
            show_error = getattr(self.ctx.dialog_manager, "show_error_dialog", None)
            if callable(show_error):
                show_error(
                    "Delete Error",
                    f"Could not move {len(failures)} item(s) to Trash.",
                )
        elif not target_paths:
            self.ctx.statusBar().showMessage(
                "No valid image files were deleted from selection.", 3000
            )

    # --- Internal helpers ---
    def _prune_empty_parent_groups(self, parents):
        for parent_item_candidate in list(parents):
            if parent_item_candidate == self.ctx.file_system_model.invisibleRootItem():
                continue
            if parent_item_candidate.model() is None:
                continue
            is_eligible = False
            user_data = parent_item_candidate.data(0x0100)
            if isinstance(user_data, str):
                if (
                    user_data.startswith("cluster_header_")
                    or user_data.startswith("date_header_")
                    or (
                        self.ctx.show_folders_mode
                        and not self.ctx.group_by_similarity_mode
                    )
                ):
                    is_eligible = True
            if is_eligible and parent_item_candidate.rowCount() == 0:
                row = parent_item_candidate.row()
                actual_parent_qitem = parent_item_candidate.parent()
                if actual_parent_qitem is None:
                    parent_to_operate = self.ctx.file_system_model.invisibleRootItem()
                else:
                    parent_to_operate = actual_parent_qitem
                parent_to_operate.takeRow(row)

    def _restore_selection_after_delete(
        self, visible_before, visible_after, deleted_paths, view: QAbstractItemView
    ):
        handled = False
        if self.was_focused_delete:
            remaining = [p for p in self.original_selection_paths if p in visible_after]
            if remaining:
                self.ctx._handle_file_selection_changed(
                    override_selected_paths=remaining
                )
                selection = QItemSelection()
                first_idx = QModelIndex()
                for p in remaining:
                    proxy = self.ctx._find_proxy_index_for_path(p)
                    if proxy.isValid():  # type: ignore[attr-defined]
                        selection.select(proxy, proxy)
                        if not first_idx.isValid():
                            first_idx = proxy
                if not selection.isEmpty():
                    sel_model = view.selectionModel()
                    sel_model.blockSignals(True)
                    if first_idx.isValid():
                        view.setCurrentIndex(first_idx)
                    sel_model.select(
                        selection, QItemSelectionModel.SelectionFlag.ClearAndSelect
                    )
                    sel_model.blockSignals(False)
                    if first_idx.isValid():
                        view.scrollTo(
                            first_idx, QAbstractItemView.ScrollHint.EnsureVisible
                        )
                    QTimer.singleShot(0, self.ctx._handle_file_selection_changed)
                handled = True
        if handled:
            return
        if not visible_after:
            self.ctx.advanced_image_viewer.clear()
            self.ctx.advanced_image_viewer.setText("No images left to display.")
            self.ctx.statusBar().showMessage("No images left or visible.")
            return
        first_deleted_idx = -1
        if visible_before and deleted_paths:
            try:
                first_deleted_idx = visible_before.index(deleted_paths[0])
            except ValueError:
                first_deleted_idx = 0
        elif visible_before:
            first_deleted_idx = 0
        target_idx = min(first_deleted_idx, len(visible_after) - 1)
        target_idx = max(0, target_idx)
        proxy_target = self.ctx._find_proxy_index_for_path(visible_after[target_idx])
        if proxy_target.isValid():  # type: ignore[attr-defined]
            view.setCurrentIndex(proxy_target)
            view.selectionModel().select(
                proxy_target, QItemSelectionModel.SelectionFlag.ClearAndSelect
            )
            view.scrollTo(proxy_target, QAbstractItemView.ScrollHint.EnsureVisible)
            QTimer.singleShot(0, self.ctx._handle_file_selection_changed)
        else:
            self.ctx.advanced_image_viewer.clear()
            self.ctx.advanced_image_viewer.setText("No valid image to select.")

from types import SimpleNamespace
from unittest.mock import Mock, call

from src.ui.app_controller import AppController
from src.ui.main_window import MainWindow
from src.ui.worker_manager import WorkerManager
from src.ui.workflow_transition import WorkflowTransitionRequest
from src.ui.workflow_transition import WorkflowPendingState
from src.ui.dialog_manager import DialogManager
from PyQt6.QtCore import QObject, QTimer, pyqtSignal
from PyQt6.QtGui import QColor, QIcon, QPixmap
from PyQt6.QtWidgets import (
    QApplication,
    QDialog,
    QLabel,
    QListWidget,
    QPushButton,
    QWidget,
)

app = QApplication.instance() or QApplication([])


class _ThumbnailSignals(QObject):
    thumbnail_session_batch_ready = pyqtSignal(str, object)


def _worker_manager(*, grouping=False, rotations=False):
    return SimpleNamespace(
        is_grouping_workflow_running=lambda: grouping,
        is_rotation_application_running=lambda: rotations,
    )


def test_clean_transition_cancels_current_analysis_and_switches_directly():
    controller = SimpleNamespace(
        is_workflow_analysis_running=lambda workflow: workflow == "easy_delete",
        cancel_workflow_analysis=Mock(),
    )
    window = SimpleNamespace(
        app_state=SimpleNamespace(workflow_step="easy_delete"),
        worker_manager=_worker_manager(),
        app_controller=controller,
        _collect_workflow_pending_state=lambda _source: SimpleNamespace(
            has_resolvable_work=False
        ),
        _show_workflow_destination=Mock(),
    )

    MainWindow._request_workflow_transition(window, "fix_rotation")

    controller.cancel_workflow_analysis.assert_called_once_with("easy_delete")
    window._show_workflow_destination.assert_called_once_with("fix_rotation")


def test_dirty_review_blocks_workflow_switch_before_pending_resolution():
    review = SimpleNamespace(
        has_unconfirmed_changes=lambda: True,
        show_confirm_or_reset_required=Mock(),
    )
    collect_pending = Mock()
    show_destination = Mock()
    window = SimpleNamespace(
        app_state=SimpleNamespace(workflow_step="easy_delete"),
        worker_manager=_worker_manager(),
        app_controller=SimpleNamespace(
            is_workflow_analysis_running=lambda _workflow: False
        ),
        get_active_image_adapter=lambda workflow: (
            review if workflow == "easy_delete" else None
        ),
        update_workflow_navigation=Mock(),
        _collect_workflow_pending_state=collect_pending,
        _show_workflow_destination=show_destination,
    )

    MainWindow._request_workflow_transition(window, "fix_rotation")

    review.show_confirm_or_reset_required.assert_called_once_with()
    window.update_workflow_navigation.assert_called_once_with()
    collect_pending.assert_not_called()
    show_destination.assert_not_called()


def test_stay_here_preserves_pending_work_and_running_analysis():
    controller = SimpleNamespace(
        is_workflow_analysis_running=lambda _workflow: True,
        cancel_workflow_analysis=Mock(),
    )
    window = SimpleNamespace(
        app_state=SimpleNamespace(workflow_step="easy_delete"),
        worker_manager=_worker_manager(),
        app_controller=controller,
        dialog_manager=SimpleNamespace(
            show_workflow_transition_dialog=Mock(return_value=None)
        ),
        _collect_workflow_pending_state=lambda _source: SimpleNamespace(
            has_resolvable_work=True,
            organize_actions=[],
            rotation_count=0,
            trash_paths=["/tmp/a.jpg"],
        ),
        update_workflow_navigation=Mock(),
        _show_workflow_destination=Mock(),
    )

    MainWindow._request_workflow_transition(window, "cull")

    controller.cancel_workflow_analysis.assert_not_called()
    window._show_workflow_destination.assert_not_called()
    window.update_workflow_navigation.assert_called_once()


def test_apply_resolves_pending_work_without_switching_workflow():
    controller = SimpleNamespace(
        is_workflow_analysis_running=lambda _workflow: True,
        cancel_workflow_analysis=Mock(),
    )
    dialog = Mock(return_value={"trash": "commit"})
    pending = SimpleNamespace(
        has_resolvable_work=True,
        organize_actions=[],
        rotation_count=0,
        trash_paths=["/tmp/a.jpg"],
    )
    window = SimpleNamespace(
        app_state=SimpleNamespace(workflow_step="easy_delete"),
        worker_manager=_worker_manager(),
        app_controller=controller,
        dialog_manager=SimpleNamespace(show_workflow_transition_dialog=dialog),
        _collect_workflow_pending_state=lambda _source: pending,
        grouping_step_widget=SimpleNamespace(),
        fix_rotation_step_widget=None,
        _finish_workflow_transition=Mock(return_value=True),
    )

    MainWindow._request_workflow_transition(window, None)

    request = window._finish_workflow_transition.call_args.args[0]
    assert request.destination is None
    assert request.trash_resolution == "commit"
    dialog.assert_called_once_with(
        "Easy Delete",
        "Easy Delete",
        pending,
        switching=False,
    )
    controller.cancel_workflow_analysis.assert_not_called()


def test_combined_discard_and_clear_resolves_every_category_before_switch():
    grouping = SimpleNamespace(discard_unsaved_grouping_edits=Mock())
    window = SimpleNamespace(
        app_state=SimpleNamespace(workflow_step="organize"),
        worker_manager=_worker_manager(),
        app_controller=SimpleNamespace(
            is_workflow_analysis_running=lambda _workflow: False
        ),
        dialog_manager=SimpleNamespace(
            show_workflow_transition_dialog=Mock(
                return_value={"organize": "discard", "trash": "clear"}
            )
        ),
        grouping_step_widget=grouping,
        fix_rotation_step_widget=None,
        _collect_workflow_pending_state=lambda _source: SimpleNamespace(
            has_resolvable_work=True,
            organize_actions=["Move a.jpg"],
            rotation_count=0,
            trash_paths=["/tmp/b.jpg"],
        ),
        _finish_workflow_transition=Mock(return_value=True),
    )

    MainWindow._request_workflow_transition(window, "easy_delete")

    grouping.discard_unsaved_grouping_edits.assert_called_once()
    request = window._finish_workflow_transition.call_args.args[0]
    assert request.destination == "easy_delete"
    assert request.organize_resolution == "discard"
    assert request.trash_resolution == "clear"


def test_apply_rotations_defers_transition_until_worker_completion():
    rotation_widget = SimpleNamespace(apply_pending_rotations=Mock())
    window = SimpleNamespace(
        app_state=SimpleNamespace(workflow_step="fix_rotation"),
        worker_manager=_worker_manager(),
        app_controller=SimpleNamespace(
            is_workflow_analysis_running=lambda _workflow: False
        ),
        dialog_manager=SimpleNamespace(
            show_workflow_transition_dialog=Mock(
                return_value={"rotation": "apply", "trash": "clear"}
            )
        ),
        grouping_step_widget=SimpleNamespace(),
        fix_rotation_step_widget=rotation_widget,
        _collect_workflow_pending_state=lambda _source: SimpleNamespace(
            has_resolvable_work=True,
            organize_actions=[],
            rotation_count=2,
            trash_paths=["/tmp/b.jpg"],
        ),
        _finish_workflow_transition=Mock(),
        _pending_workflow_transition=None,
    )

    MainWindow._request_workflow_transition(window, "pick_best")

    rotation_widget.apply_pending_rotations.assert_called_once()
    assert window._pending_workflow_transition.destination == "pick_best"
    window._finish_workflow_transition.assert_not_called()


def test_failed_rotations_cancel_deferred_switch_and_retain_current_workflow():
    status = SimpleNamespace(showMessage=Mock())
    window = SimpleNamespace(
        _pending_workflow_transition=WorkflowTransitionRequest(
            source="fix_rotation", destination="pick_best"
        ),
        statusBar=lambda: status,
        _finish_workflow_transition=Mock(),
    )

    MainWindow.finish_workflow_transition_after_rotations(window, 1, 1)

    assert window._pending_workflow_transition is None
    window._finish_workflow_transition.assert_not_called()
    assert "failed" in status.showMessage.call_args.args[0]


def test_failed_trash_move_prevents_destination_switch():
    state = SimpleNamespace(get_marked_files=lambda: ["/tmp/a.jpg"])
    status = SimpleNamespace(showMessage=Mock())
    window = SimpleNamespace(
        _pending_workflow_transition=None,
        app_state=state,
        _perform_deletion_of_marked_files=Mock(return_value=False),
        _reset_deletion_workflow_decisions=Mock(),
        _show_workflow_destination=Mock(),
        statusBar=lambda: status,
    )
    request = WorkflowTransitionRequest(
        source="cull", destination="organize", trash_resolution="commit"
    )

    assert MainWindow._finish_workflow_transition(window, request) is False
    window._show_workflow_destination.assert_not_called()
    window._reset_deletion_workflow_decisions.assert_not_called()


def test_successful_in_place_resolution_does_not_open_another_workflow():
    state = SimpleNamespace(get_marked_files=lambda: ["/tmp/a.jpg"])
    window = SimpleNamespace(
        _pending_workflow_transition=None,
        app_state=state,
        _perform_deletion_of_marked_files=Mock(return_value=True),
        _reset_deletion_workflow_decisions=Mock(),
        _show_workflow_destination=Mock(),
        update_workflow_navigation=Mock(),
    )
    request = WorkflowTransitionRequest(
        source="easy_delete", destination=None, trash_resolution="commit"
    )

    assert MainWindow._finish_workflow_transition(window, request) is True
    window._perform_deletion_of_marked_files.assert_called_once_with(["/tmp/a.jpg"])
    window._reset_deletion_workflow_decisions.assert_called_once()
    window._show_workflow_destination.assert_not_called()
    window.update_workflow_navigation.assert_called_once()


def test_deletion_completion_clears_easy_delete_before_file_model_mutation():
    calls = Mock()
    sync_workflows = Mock()
    calls.attach_mock(sync_workflows, "sync_workflows")
    remove_model_paths = Mock()
    calls.attach_mock(remove_model_paths, "remove_model_paths")
    completion = Mock()
    state = SimpleNamespace(
        easy_delete_results=None,
        pick_best_results={},
        remove_data_for_paths=Mock(),
        set_deletion_marks=Mock(),
    )
    window = SimpleNamespace(
        _pending_deletion_context={
            "represented_by_target": {"/tmp/deleted.jpg": ["/tmp/deleted.jpg"]},
            "marks_by_target": {"/tmp/deleted.jpg": ["/tmp/deleted.jpg"]},
            "completion": completion,
        },
        app_state=state,
        _sync_workflow_results_after_file_mutation=sync_workflows,
        image_pipeline=SimpleNamespace(invalidate_path=Mock()),
        thumbnail_loader=SimpleNamespace(invalidate_paths=Mock()),
        _remove_model_paths_batch=remove_model_paths,
        proxy_model=SimpleNamespace(invalidate=Mock()),
        grouping_step_widget=SimpleNamespace(remove_deleted_paths=Mock()),
        mark_cull_model_dirty=Mock(),
        _refresh_workflow_deletion_state=Mock(),
    )
    result = SimpleNamespace(
        successful_targets=["/tmp/deleted.jpg"],
        failures={},
    )

    MainWindow._handle_file_deletion_complete(window, result)

    sync_workflows.assert_called_once_with()
    assert calls.mock_calls[:2] == [
        call.sync_workflows(),
        call.remove_model_paths(["/tmp/deleted.jpg"]),
    ]
    completion.assert_called_once_with(
        ["/tmp/deleted.jpg"],
        ["/tmp/deleted.jpg"],
        {},
        {"/tmp/deleted.jpg"},
    )


def test_file_mutation_sync_updates_every_instantiated_review_workflow():
    easy_delete = SimpleNamespace(sync_results_after_file_mutation=Mock())
    fix_rotation = SimpleNamespace(sync_results_after_file_mutation=Mock())
    pick_best = SimpleNamespace(sync_results_after_file_mutation=Mock())
    state = SimpleNamespace(
        easy_delete_results=None,
        fix_rotation_results={"/tmp/remaining.jpg": 90},
        pick_best_results={},
    )
    window = SimpleNamespace(
        app_state=state,
        easy_delete_step_widget=easy_delete,
        fix_rotation_step_widget=fix_rotation,
        pick_best_step_widget=pick_best,
    )

    MainWindow._sync_workflow_results_after_file_mutation(window)

    easy_delete.sync_results_after_file_mutation.assert_called_once_with(None)
    fix_rotation.sync_results_after_file_mutation.assert_called_once_with(
        state.fix_rotation_results
    )
    pick_best.sync_results_after_file_mutation.assert_called_once_with({})


def test_failed_deletion_does_not_reset_workflow_review_state():
    sync_workflows = Mock()
    completion = Mock()
    state = SimpleNamespace(
        remove_data_for_paths=Mock(),
        set_deletion_marks=Mock(),
    )
    window = SimpleNamespace(
        _pending_deletion_context={
            "represented_by_target": {"/tmp/kept.jpg": ["/tmp/kept.jpg"]},
            "marks_by_target": {"/tmp/kept.jpg": ["/tmp/kept.jpg"]},
            "completion": completion,
        },
        app_state=state,
        _sync_workflow_results_after_file_mutation=sync_workflows,
        image_pipeline=SimpleNamespace(invalidate_path=Mock()),
        thumbnail_loader=SimpleNamespace(invalidate_paths=Mock()),
        _remove_model_paths_batch=Mock(),
        proxy_model=SimpleNamespace(invalidate=Mock()),
        grouping_step_widget=SimpleNamespace(remove_deleted_paths=Mock()),
        mark_cull_model_dirty=Mock(),
        _refresh_workflow_deletion_state=Mock(),
    )
    result = SimpleNamespace(
        successful_targets=[],
        failures={"/tmp/kept.jpg": "Permission denied"},
    )

    MainWindow._handle_file_deletion_complete(window, result)

    sync_workflows.assert_not_called()
    completion.assert_called_once_with(
        [],
        [],
        {"/tmp/kept.jpg": "Permission denied"},
        set(),
    )


def test_cancelled_workflow_discards_late_analysis_results():
    worker = SimpleNamespace(
        request_stop_easy_delete_analysis=Mock(),
        request_stop_similarity_analysis=Mock(),
    )
    state = SimpleNamespace(
        easy_delete_results=None,
        embeddings_cache={},
    )
    main_window = SimpleNamespace(
        easy_delete_step_widget=SimpleNamespace(show_results=Mock())
    )
    controller = AppController(main_window, state, worker)
    controller._easy_delete_pending_after_similarity = True

    controller.cancel_workflow_analysis("easy_delete")
    controller.handle_easy_delete_complete({"/tmp/a.jpg": {"type": "blur"}})
    controller.handle_embeddings_generated({"/tmp/a.jpg": [1.0]})

    assert state.easy_delete_results is None
    assert state.embeddings_cache == {}
    main_window.easy_delete_step_widget.show_results.assert_not_called()
    worker.request_stop_easy_delete_analysis.assert_called_once()
    worker.request_stop_similarity_analysis.assert_called_once()


def test_worker_generation_drops_callback_from_replaced_run():
    signal = SimpleNamespace(emit=Mock())
    manager = SimpleNamespace(_worker_generations={"easy_delete": 3})

    WorkerManager._emit_if_current(manager, "easy_delete", 2, signal, {"stale": True})
    WorkerManager._emit_if_current(manager, "easy_delete", 3, signal, {"current": True})

    signal.emit.assert_called_once_with({"current": True})


def test_transition_dialog_shows_marked_photo_gallery_and_direct_actions():
    parent = QWidget()
    parent.image_pipeline = SimpleNamespace(
        get_cached_thumbnail_qpixmap=lambda *_args, **_kwargs: QPixmap()
    )
    manager = DialogManager(parent)
    observed = {}

    def interact():
        dialog = QApplication.activeModalWidget()
        assert isinstance(dialog, QDialog)
        gallery = dialog.findChild(QListWidget, "workflowTransitionTrashList")
        observed["count"] = gallery.count()
        observed["move_text"] = dialog.findChild(
            QPushButton, "workflowTransitionTrashButton"
        ).text()
        dialog.findChild(QPushButton, "workflowTransitionClearButton").click()

    QTimer.singleShot(0, interact)
    result = manager.show_workflow_transition_dialog(
        "Organize",
        "Easy Delete",
        WorkflowPendingState(trash_paths=["/tmp/a.jpg", "/tmp/b.jpg"]),
    )

    assert observed == {
        "count": 2,
        "move_text": "Move to Trash and Switch",
    }
    assert result == {"trash": "clear"}


def test_transition_dialog_loads_missing_thumbnails_and_updates_live_items(tmp_path):
    photo = tmp_path / "photo.jpg"
    photo.write_bytes(b"image")
    loaded = False
    thumbnail = QPixmap(40, 30)
    thumbnail.fill(QColor("red"))

    def cached_review(_path):
        return thumbnail if loaded else QPixmap()

    parent = QWidget()
    parent.image_pipeline = SimpleNamespace(
        get_cached_review_qpixmap=Mock(side_effect=cached_review),
        get_cached_thumbnail_qpixmap=Mock(return_value=QPixmap()),
    )
    parent.worker_manager = _ThumbnailSignals()
    parent.thumbnail_loader = SimpleNamespace(request_paths=Mock())
    manager = DialogManager(parent)
    observed = {}

    def interact():
        nonlocal loaded
        dialog = QApplication.activeModalWidget()
        assert isinstance(dialog, QDialog)
        gallery = dialog.findChild(QListWidget, "workflowTransitionTrashList")
        item = gallery.item(0)
        loaded = True
        parent.worker_manager.thumbnail_session_batch_ready.emit(
            "dialog-session", [str(photo)]
        )
        QApplication.processEvents()
        rendered = item.icon().pixmap(24, 24).toImage()
        observed["color"] = rendered.pixelColor(
            rendered.width() // 2, rendered.height() // 2
        )
        dialog.findChild(QPushButton, "workflowTransitionClearButton").click()

    QTimer.singleShot(0, interact)
    result = manager.show_workflow_transition_dialog(
        "Easy Delete",
        "Fix Rotation",
        WorkflowPendingState(trash_paths=[str(photo)]),
    )

    parent.thumbnail_loader.request_paths.assert_called_once_with([str(photo)])
    assert observed["color"] == QColor("red")
    assert result == {"trash": "clear"}


def test_delete_confirmation_loads_missing_thumbnails_and_updates_live_items(
    tmp_path,
):
    photo = tmp_path / "photo.arw"
    photo.write_bytes(b"raw image")
    loaded = False
    thumbnail = QPixmap(40, 30)
    thumbnail.fill(QColor("red"))

    def cached_review(_path):
        return thumbnail if loaded else QPixmap()

    parent = QWidget()
    parent.image_pipeline = SimpleNamespace(
        get_cached_review_qpixmap=Mock(side_effect=cached_review),
        get_cached_thumbnail_qpixmap=Mock(return_value=QPixmap()),
    )
    parent.worker_manager = _ThumbnailSignals()
    parent.thumbnail_loader = SimpleNamespace(request_paths=Mock())
    manager = DialogManager(parent)
    observed = {}

    def interact():
        nonlocal loaded
        dialog = QApplication.activeModalWidget()
        assert isinstance(dialog, QDialog)
        gallery = dialog.findChild(QListWidget, "deleteDialogListWidget")
        item = gallery.item(0)
        loaded = True
        parent.worker_manager.thumbnail_session_batch_ready.emit(
            "dialog-session", [str(photo)]
        )
        QApplication.processEvents()
        rendered = item.icon().pixmap(24, 24).toImage()
        observed["color"] = rendered.pixelColor(
            rendered.width() // 2, rendered.height() // 2
        )
        dialog.reject()

    QTimer.singleShot(0, interact)
    result = manager.show_commit_deletions_dialog([str(photo)])

    parent.thumbnail_loader.request_paths.assert_called_once_with([str(photo)])
    assert observed["color"] == QColor("red")
    assert result is False


def test_delete_confirmation_reuses_shared_icon_without_requesting_thumbnail(
    tmp_path,
):
    photo = tmp_path / "photo.arw"
    photo.write_bytes(b"raw image")
    thumbnail = QPixmap(40, 30)
    thumbnail.fill(QColor("blue"))
    cached_icon = QIcon(thumbnail)

    parent = QWidget()
    parent.get_cached_thumbnail_icon = Mock(return_value=cached_icon)
    parent.image_pipeline = SimpleNamespace(
        get_cached_review_qpixmap=Mock(
            side_effect=AssertionError("shared UI icon should be reused first")
        )
    )
    parent.worker_manager = _ThumbnailSignals()
    parent.thumbnail_loader = SimpleNamespace(request_paths=Mock())
    manager = DialogManager(parent)
    observed = {}

    def interact():
        dialog = QApplication.activeModalWidget()
        assert isinstance(dialog, QDialog)
        gallery = dialog.findChild(QListWidget, "deleteDialogListWidget")
        rendered = gallery.item(0).icon().pixmap(24, 24).toImage()
        observed["color"] = rendered.pixelColor(
            rendered.width() // 2, rendered.height() // 2
        )
        dialog.reject()

    QTimer.singleShot(0, interact)
    manager.show_commit_deletions_dialog([str(photo)])

    parent.get_cached_thumbnail_icon.assert_called_once_with(str(photo))
    parent.thumbnail_loader.request_paths.assert_not_called()
    assert observed["color"] == QColor("blue")


def test_close_confirmation_uses_shared_dialog_thumbnail_loader(tmp_path):
    photo = tmp_path / "photo.jpg"
    photo.write_bytes(b"image")
    parent = QWidget()
    parent.image_pipeline = SimpleNamespace(
        get_cached_review_qpixmap=Mock(return_value=QPixmap()),
        get_cached_thumbnail_qpixmap=Mock(return_value=QPixmap()),
    )
    parent.worker_manager = _ThumbnailSignals()
    parent.thumbnail_loader = SimpleNamespace(request_paths=Mock())
    manager = DialogManager(parent)

    QTimer.singleShot(0, lambda: QApplication.activeModalWidget().reject())
    result = manager.show_close_confirmation_dialog([str(photo)])

    parent.thumbnail_loader.request_paths.assert_called_once_with([str(photo)])
    assert result == "cancel"


def test_in_place_resolution_dialog_does_not_offer_switch_actions():
    parent = QWidget()
    parent.image_pipeline = SimpleNamespace(
        get_cached_thumbnail_qpixmap=lambda *_args, **_kwargs: QPixmap()
    )
    manager = DialogManager(parent)
    observed = {}

    def interact():
        dialog = QApplication.activeModalWidget()
        assert isinstance(dialog, QDialog)
        observed["stay_text"] = dialog.findChild(
            QPushButton, "workflowTransitionStayButton"
        ).text()
        observed["move_text"] = dialog.findChild(
            QPushButton, "workflowTransitionTrashButton"
        ).text()
        dialog.findChild(QPushButton, "workflowTransitionTrashButton").click()

    QTimer.singleShot(0, interact)
    result = manager.show_workflow_transition_dialog(
        "Easy Delete",
        "Easy Delete",
        WorkflowPendingState(trash_paths=["/tmp/a.jpg"]),
        switching=False,
    )

    assert observed == {"stay_text": "Keep Reviewing", "move_text": "Move to Trash"}
    assert result == {"trash": "commit"}


def test_rotation_dialog_shows_gallery_and_direct_switch_actions():
    parent = QWidget()
    review_pixmap = QPixmap(24, 12)
    review_pixmap.fill()
    review_cache = Mock(return_value=review_pixmap)
    parent.image_pipeline = SimpleNamespace(
        get_cached_review_qpixmap=review_cache,
        get_cached_thumbnail_qpixmap=Mock(
            side_effect=AssertionError("rotation dialog must use review cache priority")
        ),
    )
    manager = DialogManager(parent)
    observed = {}

    def interact():
        dialog = QApplication.activeModalWidget()
        assert isinstance(dialog, QDialog)
        gallery = dialog.findChild(QListWidget, "workflowTransitionRotationList")
        observed["count"] = gallery.count()
        comparison = gallery.itemWidget(gallery.item(0))
        observed["captions"] = {
            label.text()
            for label in comparison.findChildren(
                QLabel, "workflowTransitionRotationCaption"
            )
        }
        observed["discard_text"] = dialog.findChild(
            QPushButton, "workflowTransitionRotationDiscardButton"
        ).text()
        apply_button = dialog.findChild(
            QPushButton, "workflowTransitionRotationApplyButton"
        )
        observed["apply_text"] = apply_button.text()
        apply_button.click()

    QTimer.singleShot(0, interact)
    result = manager.show_workflow_transition_dialog(
        "Fix Rotation",
        "Pick Best",
        WorkflowPendingState(
            rotation_count=1,
            rotation_changes={"/tmp/a.jpg": 90},
        ),
    )

    assert observed == {
        "count": 1,
        "captions": {"BEFORE", "AFTER"},
        "discard_text": "Discard Rotations and Switch",
        "apply_text": "Apply Rotations and Switch",
    }
    assert result == {"rotation": "apply"}
    review_cache.assert_called_once_with("/tmp/a.jpg")


def test_deletion_preview_keeps_folders_as_bounded_recursive_targets(tmp_path):
    folder = tmp_path / "Trip"
    nested = folder / "Metadata"
    nested.mkdir(parents=True)
    photo = folder / "photo.jpg"
    sidecar = nested / "photo.json"
    photo.write_bytes(b"photo")
    sidecar.write_text("{}", encoding="utf-8")
    empty_after_move = tmp_path / "EmptyAfterMove"
    empty_after_move.mkdir()

    manager = DialogManager(QWidget())
    entries = manager._build_deletion_preview_entries(
        WorkflowPendingState(
            trash_paths=[str(folder)],
            organize_removed_folders=[str(empty_after_move)],
            directory_paths={str(folder), str(empty_after_move)},
        )
    )

    by_path = {
        path: (name, detail, is_directory)
        for path, name, detail, is_directory in entries
    }
    assert set(by_path) == {str(folder), str(empty_after_move)}
    assert by_path[str(folder)][2] is True
    assert "all contents" in by_path[str(folder)][1]
    assert by_path[str(empty_after_move)][1] == "Empty folder removed after organizing"


def test_deletion_preview_includes_every_target_without_filesystem_walk(tmp_path):
    targets = [str(tmp_path / f"{index}.jpg") for index in range(505)]
    manager = DialogManager(QWidget())
    entries = manager._build_deletion_preview_entries(
        WorkflowPendingState(trash_paths=targets)
    )

    assert len(entries) == len(targets)
    assert {entry[0] for entry in entries} == set(targets)
    assert all(entry[0] for entry in entries)


def test_delete_confirmation_gallery_shows_every_marked_image():
    paths = [f"/tmp/photo-{index}.jpg" for index in range(505)]
    parent = QWidget()
    parent.image_pipeline = SimpleNamespace(
        get_cached_review_qpixmap=Mock(return_value=QPixmap()),
        get_cached_thumbnail_qpixmap=Mock(return_value=QPixmap()),
    )
    parent.worker_manager = _ThumbnailSignals()
    parent.thumbnail_loader = SimpleNamespace(request_paths=Mock())
    manager = DialogManager(parent)
    observed = {}

    def interact():
        dialog = QApplication.activeModalWidget()
        assert isinstance(dialog, QDialog)
        gallery = dialog.findChild(QListWidget, "deleteDialogListWidget")
        observed["count"] = gallery.count()
        observed["labels"] = [
            gallery.item(index).text() for index in range(gallery.count())
        ]
        dialog.reject()

    QTimer.singleShot(0, interact)
    manager.show_commit_deletions_dialog(paths)

    assert observed["count"] == len(paths)
    assert not any("more item" in label for label in observed["labels"])
    parent.thumbnail_loader.request_paths.assert_called_once_with(paths)


def test_organize_folder_mark_is_staged_and_unstaged_as_one_target(tmp_path):
    folder = tmp_path / "Trip"
    folder.mkdir()
    photo = folder / "photo.jpg"
    photo.write_bytes(b"photo")
    marks: set[str] = set()
    app_state = SimpleNamespace(is_marked_for_deletion=marks.__contains__)

    def set_paths_marked(mark_state, *_args):
        for path, marked in mark_state.items():
            if marked:
                marks.add(path)
            else:
                marks.discard(path)

    deletion_controller = SimpleNamespace(
        set_paths_marked=set_paths_marked,
    )
    window = SimpleNamespace(
        app_state=app_state,
        grouping_step_widget=SimpleNamespace(
            known_directory_paths=lambda: {str(folder)}
        ),
        deletion_controller=deletion_controller,
        file_system_model=object(),
        proxy_model=SimpleNamespace(invalidate=Mock()),
        _refresh_visible_items_icons=Mock(),
        _refresh_workflow_deletion_state=Mock(),
        statusBar=lambda: SimpleNamespace(showMessage=Mock()),
    )
    targets = [str(folder), str(photo)]

    MainWindow._toggle_organize_deletion_marks(window, targets)
    assert marks == set(targets)

    MainWindow._toggle_organize_deletion_marks(window, targets)
    assert marks == set()


def test_context_menu_mark_handler_sets_mark_without_toggling_back():
    deletion_controller = SimpleNamespace(mark_paths=Mock(return_value=1))
    status = SimpleNamespace(showMessage=Mock())
    window = SimpleNamespace(
        deletion_controller=deletion_controller,
        _find_proxy_index_for_path=Mock(),
        file_system_model=object(),
        proxy_model=SimpleNamespace(invalidate=Mock()),
        statusBar=lambda: status,
    )

    MainWindow._mark_image_for_deletion(window, "/tmp/photo.jpg")

    deletion_controller.mark_paths.assert_called_once_with(
        ["/tmp/photo.jpg"],
        window._find_proxy_index_for_path,
        window.file_system_model,
        window.proxy_model,
    )


def test_context_menu_unmark_handler_sets_unmarked_without_toggling_back():
    deletion_controller = SimpleNamespace(unmark_paths=Mock(return_value=1))
    status = SimpleNamespace(showMessage=Mock())
    window = SimpleNamespace(
        deletion_controller=deletion_controller,
        _is_marked_for_deletion=lambda _path: True,
        _find_proxy_index_for_path=Mock(),
        file_system_model=object(),
        proxy_model=SimpleNamespace(invalidate=Mock()),
        statusBar=lambda: status,
    )

    MainWindow._unmark_image_for_deletion(window, "/tmp/photo.jpg")

    deletion_controller.unmark_paths.assert_called_once_with(
        ["/tmp/photo.jpg"],
        window._find_proxy_index_for_path,
        window.file_system_model,
        window.proxy_model,
    )


def test_deferred_close_waits_for_deletion_thread_shutdown(monkeypatch):
    callbacks = []
    worker_manager = SimpleNamespace(is_file_deletion_running=lambda: True)
    window = SimpleNamespace(worker_manager=worker_manager, close=Mock())
    window._finish_close_after_deletion = lambda: (
        MainWindow._finish_close_after_deletion(window)
    )
    monkeypatch.setattr(
        "ui.main_window.QTimer.singleShot",
        lambda _delay, callback: callbacks.append(callback),
    )

    MainWindow._finish_close_after_deletion(window)

    window.close.assert_not_called()
    assert len(callbacks) == 1
    worker_manager.is_file_deletion_running = lambda: False
    callbacks.pop()()
    window.close.assert_called_once_with()

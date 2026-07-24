from types import SimpleNamespace
from unittest.mock import Mock

from src.ui.app_controller import AppController
from src.ui.main_window import MainWindow
from src.ui.worker_manager import WorkerManager
from src.ui.workflow_transition import WorkflowTransitionRequest
from src.ui.workflow_transition import WorkflowPendingState
from src.ui.dialog_manager import DialogManager
from PyQt6.QtCore import QObject, QTimer, pyqtSignal
from PyQt6.QtGui import QColor, QPixmap
from PyQt6.QtWidgets import (
    QApplication,
    QDialog,
    QLabel,
    QListWidget,
    QPushButton,
    QWidget,
)


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


def test_cancelled_workflow_discards_late_analysis_results():
    worker = SimpleNamespace(
        stop_easy_delete_analysis=Mock(),
        stop_similarity_analysis=Mock(),
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
    worker.stop_easy_delete_analysis.assert_called_once()
    worker.stop_similarity_analysis.assert_called_once()


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


def test_deletion_preview_expands_folders_and_shows_non_media_files(tmp_path):
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
        )
    )

    by_path = {
        path: (name, detail, is_directory)
        for path, name, detail, is_directory in entries
    }
    assert set(by_path) == {
        str(folder),
        str(nested),
        str(photo),
        str(sidecar),
        str(empty_after_move),
    }
    assert by_path[str(folder)][2] is True
    assert by_path[str(sidecar)][1] == "Inside Trip"
    assert by_path[str(empty_after_move)][1] == "Empty folder removed after organizing"


def test_organize_folder_mark_is_staged_and_unstaged_as_one_target(tmp_path):
    folder = tmp_path / "Trip"
    folder.mkdir()
    photo = folder / "photo.jpg"
    photo.write_bytes(b"photo")
    marks: set[str] = set()
    app_state = SimpleNamespace(is_marked_for_deletion=marks.__contains__)
    deletion_controller = SimpleNamespace(
        mark=marks.add,
        unmark=marks.discard,
        toggle_mark=lambda path: (
            marks.discard(path) if path in marks else marks.add(path)
        ),
    )
    window = SimpleNamespace(
        app_state=app_state,
        deletion_controller=deletion_controller,
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

import pyexiv2  # noqa: F401  # Must be first to avoid Windows crashes

from unittest.mock import Mock

import pytest
from PyQt6.QtCore import QCoreApplication, QEventLoop, QThread, QTimer, Qt
from PyQt6.QtTest import QTest
from PyQt6.QtWidgets import QApplication, QPushButton, QWidget

from core.grouping import GroupingGroup, GroupingPlan
from ui.dialog_manager import DialogManager
from ui.grouping_step_widget import GroupingStepWidget
from ui.helpers.ui_dispatch import UiResultDispatcher, bulk_ui_update
from ui.worker_manager import WorkerManager

_app = QApplication.instance() or QApplication([])


def _pump_results():
    loop = QEventLoop()
    QTimer.singleShot(60, loop.quit)
    loop.exec()


@pytest.mark.parametrize("stop", [False, True])
def test_real_interrupt_dialog_stays_clickable_when_grouping_result_arrives(
    monkeypatch, stop
):
    monkeypatch.setattr(QThread, "start", lambda self: None)
    parent = QWidget()
    dialog_manager = DialogManager(parent)
    manager = WorkerManager(Mock())
    widget = GroupingStepWidget()
    widget.set_source_folder("/old")
    plan = GroupingPlan(
        mode="current",
        total_items=1,
        supported_items=1,
        groups=[
            GroupingGroup(group_id="1", group_label="", source_paths=["/old/photo.jpg"])
        ],
        unassigned_paths=[],
        skipped_paths=[],
        source_root="/old",
        filesystem_inventory_complete=True,
    )
    rendered_in_modal = []

    def render(result):
        rendered_in_modal.append(QApplication.activeModalWidget() is not None)
        widget.set_preview_plan(result, "/old")

    manager.grouping_preview_ready.connect(render)
    manager.start_grouping_preview([], "current", "/old")
    worker = manager.grouping_preview_worker
    queued_without_render = []
    clicked = []

    def result_during_dialog():
        worker.preview_ready.emit(plan)
        worker.finished.emit()
        QCoreApplication.processEvents()
        queued_without_render.append(widget._current_plan is None)
        dialog = QApplication.activeModalWidget()
        name = "modelConsentAcceptButton" if stop else "modelConsentCancelButton"
        button = dialog.findChild(QPushButton, name)
        QTest.mouseClick(button, Qt.MouseButton.LeftButton)
        clicked.append(True)

    timer = QTimer(parent)
    timer.setSingleShot(True)
    timer.timeout.connect(result_during_dialog)
    timer.start(0)
    approved = dialog_manager.confirm_interrupt_for_folder_change("/new")
    assert approved is stop
    assert clicked == [True]
    assert queued_without_render == [True]
    if approved:
        manager.request_stop_grouping_preview()
    _pump_results()
    assert rendered_in_modal == ([] if stop else [False])
    assert widget._current_plan is (None if stop else plan)
    manager.stop_all_workers()
    widget.close()
    parent.close()


def test_bulk_view_yields_do_not_reenter_worker_results():
    owner = QWidget()
    dispatcher = UiResultDispatcher(owner)
    calls = []
    with bulk_ui_update():
        dispatcher.dispatch(lambda: calls.append("first"))
        dispatcher.dispatch(lambda: calls.append("second"))
        _pump_results()
        assert calls == []
    _pump_results()
    assert calls == ["first", "second"]


def test_dispatch_preserves_order_when_callback_pumps_events():
    owner = QWidget()
    dispatcher = UiResultDispatcher(owner)
    calls = []

    def first():
        calls.append("first started")
        dispatcher.dispatch(lambda: calls.append("second"))
        _pump_results()
        calls.append("first finished")

    dispatcher.dispatch(first)
    _pump_results()
    assert calls == ["first started", "first finished", "second"]


def test_thumbnail_completion_waits_for_modal_and_is_discarded_after_stop(monkeypatch):
    from PyQt6.QtWidgets import QDialog

    monkeypatch.setattr(QThread, "start", lambda self: None)
    manager = WorkerManager(Mock())
    completed = []
    manager.thumbnail_session_finished.connect(lambda *args: completed.append(args))
    manager.start_thumbnail_session("old", ["/old/photo.jpg"])
    worker = manager.thumbnail_preload_worker
    dialog = QDialog()
    dialog.setModal(True)
    dialog.show()
    _app.processEvents()
    worker.session_finished.emit("old", 1, 0)
    assert completed == []
    manager.request_stop_all_workers()
    dialog.reject()
    _pump_results()
    assert completed == []
    manager.stop_all_workers()

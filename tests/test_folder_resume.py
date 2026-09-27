"""Bookmarks cross folders/workflows without replaying file decisions or work."""

import pyexiv2  # noqa: F401  # Must be first to avoid Windows crashes

import json
import threading
import time
from copy import deepcopy
from types import SimpleNamespace
from unittest.mock import Mock

import pytest
from PyQt6.QtCore import QCoreApplication, QEvent, QObject
from PyQt6.QtWidgets import QApplication

from core.folder_view_store import FolderViewStore
from ui.controllers.folder_resume_controller import FolderResumeController
from ui.controllers.active_image_controller import ActiveImageController
from ui.worker_manager import WorkerManager
from ui.easy_delete_step_widget import EasyDeleteStepWidget
from ui.fix_rotation_step_widget import FixRotationStepWidget
from ui.pick_best_step_widget import PickBestStepWidget

_app = QApplication.instance() or QApplication([])


def drain_until(predicate):
    deadline = time.monotonic() + 4
    while not predicate() and time.monotonic() < deadline:
        _app.processEvents()
        time.sleep(0.005)
    assert predicate()


class Context(QObject):
    def __init__(self):
        super().__init__()
        self.app_state = SimpleNamespace(
            workflow_step="organize",
            focused_image_path=None,
            get_file_data_by_path=lambda _path: {"bookmark_identity": [1, 2, 3, 4, 5]},
        )
        self.worker_manager = WorkerManager(Mock(), self)
        self.grouping_step_widget = SimpleNamespace(_mode_buttons={"current": None})
        self.adapters = {}
        for step in ("organize", "easy_delete", "fix_rotation", "pick_best", "cull"):
            adapter = Mock()
            adapter.capture_view_bookmark.return_value = {"path": step + ".jpg"}
            adapter.restore_view_bookmark.return_value = True
            self.adapters[step] = adapter
        self.active_image_controller = ActiveImageController(self)

    def get_active_image_adapter(self, step):
        return self.adapters.get(step)

    def _is_workflow_step_visible(self, step):
        return True


@pytest.fixture
def context(tmp_path, monkeypatch):
    # Controller tests use synthetic paths. Real filesystem identities are
    # exercised separately at the shared store/scanner boundary.
    monkeypatch.setattr(
        FolderViewStore, "_file_identity", lambda _self, _path: [1, 2, 3, 4, 5]
    )
    ctx = Context()
    ctx.folder_resume_controller = FolderResumeController(
        ctx, FolderViewStore(str(tmp_path))
    )
    yield ctx
    ctx.folder_resume_controller.prepare_close()
    drain_until(lambda: not ctx.worker_manager.has_pending_folder_view_io())
    ctx.deleteLater()
    QCoreApplication.sendPostedEvents(None, QEvent.Type.DeferredDelete)


def open_folder(ctx, folder):
    resume = ctx.folder_resume_controller
    resume.begin_folder(folder)
    drain_until(lambda: resume._loaded)
    ctx.app_state.workflow_step = resume.activate()
    return resume


def test_store_multiple_folders_and_corruption(tmp_path):
    store = FolderViewStore(str(tmp_path))
    photos = [tmp_path / "a.jpg", tmp_path / "b.jpg"]
    for photo in photos:
        photo.touch()
    first = {
        "workflow": "cull",
        "views": {"cull": {"selected": [str(p) for p in photos]}},
    }
    second = {"workflow": "organize", "views": {}}
    store.save("/photos/one", first)
    store.save("/photos/two", second)
    assert store.load("/photos/one/../one") == first
    assert store.load("/photos/two") == second
    path = store._path("/photos/one")
    path.write_text('{"version":')
    assert store.load("/photos/one") == {}
    path.write_text(json.dumps({"version": 999}))
    assert store.load("/photos/one") == {}
    assert store.load("/photos/two") == second


def test_per_folder_per_workflow_round_trip(context):
    resume = open_folder(context, "/photos/one")
    resume.data_ready("organize")
    resume.before_workflow_change("cull")
    context.app_state.workflow_step = "cull"
    resume.data_ready("cull")
    resume.checkpoint()
    open_folder(context, "/photos/two")
    resume.data_ready("organize")
    context.adapters["organize"].capture_view_bookmark.return_value = {
        "path": "second.jpg"
    }
    resume.checkpoint()
    open_folder(context, "/photos/one")
    assert context.app_state.workflow_step == "cull"
    assert resume.bookmark["views"]["organize"]["path"] == "organize.jpg"
    assert resume.bookmark["views"]["cull"]["path"] == "cull.jpg"
    open_folder(context, "/photos/two")
    assert resume.bookmark["views"]["organize"]["path"] == "second.jpg"


def test_restore_waits_for_results_without_starting_more_work(context):
    resume = context.folder_resume_controller
    resume.store.save(
        "/photos",
        {"workflow": "fix_rotation", "views": {"fix_rotation": {"path": "later.jpg"}}},
    )
    open_folder(context, "/photos")
    adapter = context.adapters["fix_rotation"]
    assert context.active_image_controller.sync_workflow("fix_rotation")
    _app.processEvents()
    adapter.restore_view_bookmark.assert_not_called()
    resume.checkpoint()
    assert resume.bookmark["views"]["fix_rotation"]["path"] == "later.jpg"
    resume.data_ready("fix_rotation")
    context.active_image_controller.sync_workflow("fix_rotation")
    drain_until(lambda: adapter.restore_view_bookmark.called)
    adapter.restore_view_bookmark.assert_called_once_with({"path": "later.jpg"})
    # Restoring focus never starts scanners, decoders, models or analysis workers.
    assert not context.worker_manager.is_any_worker_running()
    context.active_image_controller.sync_workflow("fix_rotation")
    _app.processEvents()
    assert adapter.restore_view_bookmark.call_count == 1


def test_stale_folder_load_and_restore_are_ignored(context):
    resume = context.folder_resume_controller
    operations = []
    context.worker_manager.submit_folder_view_io = lambda operation, callback=None: (
        operations.append(callback)
    )
    resume.begin_folder("/old")
    resume.begin_folder("/new")
    operations[0]({"workflow": "cull", "views": {"cull": {"path": "old.jpg"}}})
    assert not resume._loaded
    operations[1]({"workflow": "organize", "views": {"organize": {"path": "new.jpg"}}})
    resume.activate()
    resume.data_ready("organize")
    resume.restore_workflow("organize")
    resume.begin_folder("/third")
    _app.processEvents()
    context.adapters["organize"].restore_view_bookmark.assert_not_called()


def test_unchanged_bookmark_does_not_rewrite_and_io_is_off_ui_thread(context):
    resume = open_folder(context, "/photos")
    threads = []
    original = resume.store.save

    def save(*args, **kwargs):
        threads.append(threading.get_ident())
        original(*args, **kwargs)

    resume.store.save = save
    resume.data_ready("organize")
    for _ in range(5):
        resume.checkpoint()
    drain_until(lambda: not context.worker_manager.has_pending_folder_view_io())
    assert len(threads) == 1
    assert threads[0] != threading.get_ident()


def test_saved_workflows_are_not_overwritten_by_cross_workflow_focus(context):
    resume = open_folder(context, "/photos")
    resume.data_ready("organize")
    resume.checkpoint()
    context.app_state.workflow_step = "cull"
    context.active_image_controller.publish("cull.jpg", source="cull")
    context.adapters["organize"].focus_image.assert_not_called()


@pytest.mark.parametrize(
    "widget_class,results,target",
    [
        (
            EasyDeleteStepWidget,
            {f"/{i}.jpg": {"type": "blur", "suggest_delete": True} for i in range(3)},
            "/2.jpg",
        ),
        (FixRotationStepWidget, {f"/{i}.jpg": 90 for i in range(3)}, "/2.jpg"),
    ],
)
def test_review_bookmarks_restore_position_without_decisions(
    widget_class, results, target
):
    widget = widget_class()
    other = widget_class()
    try:
        widget.show_results(results)
        widget.focus_image(target)
        bookmark = widget.capture_view_bookmark()
        other.show_results(results)
        other.focus_image(target)
        initial_decisions = deepcopy(
            getattr(other, "_marked", getattr(other, "_pending_keep_by_review", {}))
        )
        assert other.restore_view_bookmark(bookmark)
        assert other.capture_view_bookmark()["path"] == target
        assert (
            getattr(other, "_marked", getattr(other, "_pending_keep_by_review", {}))
            == initial_decisions
        )
        assert not getattr(
            other, "_confirmed", getattr(other, "_confirmed_reviews", set())
        )
    finally:
        widget.deleteLater()
        other.deleteLater()
        QCoreApplication.sendPostedEvents(None, QEvent.Type.DeferredDelete)


def test_pick_best_restores_displayed_pair_without_confirming_it():
    paths = ["/a.jpg", "/b.jpg", "/c.jpg", "/d.jpg"]
    results = {
        1: {
            "winner_path": paths[0],
            "all_paths": paths,
            "ranked": [{"path": p, "final_score": 1.0} for p in paths],
            "failed": [],
        }
    }
    widget = PickBestStepWidget()
    try:
        widget.show_results(results)
        marks = Mock()
        widget.deletion_state_requested.connect(marks)
        bookmark = {"path": "/d.jpg", "comparison": ["/c.jpg", "/d.jpg"]}
        assert widget.restore_view_bookmark(bookmark)
        assert set(widget._subset_paths) == {"/c.jpg", "/d.jpg"}
        assert widget.capture_view_bookmark()["path"] == "/d.jpg"
        assert not widget._current_group().confirmed
        marks.assert_not_called()
    finally:
        widget.deleteLater()
        QCoreApplication.sendPostedEvents(None, QEvent.Type.DeferredDelete)


@pytest.fixture
def window(tmp_path, monkeypatch):
    from ui.main_window import MainWindow
    from core import app_settings
    from PyQt6.QtCore import QSettings

    monkeypatch.setattr(
        FolderViewStore, "_file_identity", lambda _self, _path: [1, 2, 3, 4, 5]
    )

    monkeypatch.setenv("PHOTOSORT_CACHE_ROOT", str(tmp_path / "cache"))
    monkeypatch.setenv("PHOTOSORT_DATA_ROOT", str(tmp_path / "data"))
    settings = QSettings(str(tmp_path / "settings.ini"), QSettings.Format.IniFormat)
    monkeypatch.setattr(app_settings, "_get_settings", lambda: settings)
    win = MainWindow()
    monkeypatch.setattr(win.app_controller, "automatic_check_for_updates", lambda: None)
    monkeypatch.setattr(win, "schedule_visible_thumbnail_load", lambda: None)
    monkeypatch.setattr(win.thumbnail_loader, "model_rebuilt", lambda: None)
    monkeypatch.setattr(win, "_handle_file_selection_changed", lambda *a, **kw: None)
    win.show()
    _app.processEvents()
    yield win
    win.app_state.marked_for_deletion.clear()
    win.grouping_step_widget._current_plan = None
    win.close()
    drain_until(lambda: not win._has_active_background_work(include_bookmarks=True))
    win.close()
    win.deleteLater()
    QCoreApplication.sendPostedEvents(None, QEvent.Type.DeferredDelete)


def test_cull_restores_filters_sort_multi_selection_and_scroll(window):
    from datetime import datetime

    window.app_state.image_files_data = [
        {
            "path": f"/photos/img{i:03}.jpg",
            "media_type": "image",
            "date": datetime(2026, 1, 1),
        }
        for i in range(100)
    ]
    window.show_cull_step()
    state = {
        "path": "/photos/img080.jpg",
        "selected": ["/photos/img080.jpg", "/photos/img081.jpg"],
        "mode": "list",
        "rating": "Show All",
        "search": "img",
        "sort": "Similarity then Time",
        "folders": False,
        "similarity": False,
        "scroll": [0, 70],
        "viewer": "single",
    }
    window.prepare_view_bookmark(state)
    _app.processEvents()
    assert window.restore_view_bookmark(state)
    view = window._get_active_file_view()
    assert set(window._get_selected_file_paths_from_view()) == set(state["selected"])
    assert view.currentIndex() == window._find_proxy_index_for_path(state["path"])
    assert window.cluster_sort_combo.currentText() == state["sort"]
    assert window.left_panel.search_input.text() == "img"
    assert view.verticalScrollBar().value() == min(
        70, view.verticalScrollBar().maximum()
    )
    # Missing images are ignored; the existing normal-view fallback remains.
    window.restore_view_bookmark({"selected": ["/missing.jpg"], "path": "/missing.jpg"})
    assert view.currentIndex().isValid()


def test_reopening_folder_uses_one_scan_and_one_shared_asset_session(window):
    folder = "/photos/resume"
    resume = window.folder_resume_controller
    resume.store.save(
        folder,
        {
            "workflow": "cull",
            "views": {
                "cull": {
                    "path": folder + "/b.jpg",
                    "selected": [folder + "/b.jpg"],
                    "mode": "list",
                }
            },
        },
    )
    window.worker_manager.start_file_scan = Mock()
    window.worker_manager.start_rating_load = Mock()
    window.start_thumbnail_warming = Mock(return_value="assets")
    window.app_controller.load_folder(folder)
    window.app_controller.handle_files_found(
        [
            {"path": folder + "/a.jpg", "media_type": "image"},
            {"path": folder + "/b.jpg", "media_type": "image"},
        ]
    )
    window.app_controller.handle_scan_finished()
    window.app_controller.handle_review_asset_finished("assets", 2, 0)
    drain_until(
        lambda: (
            window.app_state.workflow_step == "cull" and "cull" not in resume._pending
        )
    )
    assert window._get_selected_file_paths_from_view() == [folder + "/b.jpg"]
    window.worker_manager.start_file_scan.assert_called_once_with(folder)
    window.start_thumbnail_warming.assert_called_once()
    window.worker_manager.start_rating_load.assert_called_once()


def test_organize_restores_both_tree_selections_without_scanning(tmp_path, monkeypatch):
    from core.grouping import GroupingPlan, GroupingGroup
    from ui.grouping_step_widget import GroupingStepWidget

    root = str(tmp_path)
    paths = [str(tmp_path / name) for name in ("a.jpg", "b.jpg", "c.jpg")]
    widget = GroupingStepWidget()
    try:
        widget.set_source_folder(root)
        plan = GroupingPlan(
            "current",
            3,
            3,
            [GroupingGroup("one", "one", paths)],
            [],
            [],
            output_root=root,
            source_root=root,
            filesystem_inventory_complete=True,
            filesystem_paths=set(paths),
        )
        widget.set_preview_plan(plan, root)
        monkeypatch.setattr(widget, "_update_selected_preview", lambda path: None)
        monkeypatch.setattr(
            "os.walk", Mock(side_effect=AssertionError("resume must not rescan"))
        )
        inspection = Mock()
        widget._parent_window = SimpleNamespace(activate_image_inspection=inspection)
        state = {"path": paths[1], "before": paths[:2], "after": paths[1:]}
        assert widget.restore_view_bookmark(state)
        restored = widget.capture_view_bookmark()
        assert set(restored["before"]) == set(paths[:2])
        assert set(restored["after"]) == set(paths[1:])
        assert restored["path"] == paths[1]
        assert widget._multi_preview_paths == tuple(paths[1:])
        _app.processEvents()
        inspection.assert_called_once()
        assert [spec.path for spec in inspection.call_args.args[1]] == paths[1:]
    finally:
        widget.deleteLater()
        QCoreApplication.sendPostedEvents(None, QEvent.Type.DeferredDelete)


def test_bookmark_io_does_not_block_ui_or_get_cancelled_with_analysis(context):
    from PyQt6.QtCore import QTimer

    release = threading.Event()
    finished = []
    heartbeat = []
    context.worker_manager.submit_folder_view_io(
        lambda: release.wait(3), lambda result: finished.append(result)
    )
    try:
        QTimer.singleShot(0, lambda: heartbeat.append(True))
        drain_until(lambda: bool(heartbeat))
        context.worker_manager.request_stop_all_workers()
        assert context.worker_manager.has_pending_folder_view_io()
        assert not context.worker_manager.is_any_worker_running()
        assert not context.worker_manager.is_any_worker_active()
    finally:
        release.set()
    drain_until(lambda: bool(finished))
    assert finished == [True]


def test_close_during_unfinished_restore_preserves_saved_bookmark(context):
    resume = context.folder_resume_controller
    saved = {"workflow": "easy_delete", "views": {"easy_delete": {"path": "/late.jpg"}}}
    resume.store.save("/photos", saved)
    open_folder(context, "/photos")
    context.app_state.get_file_data_by_path = lambda _path: None
    resume.prepare_close()
    drain_until(lambda: not context.worker_manager.has_pending_folder_view_io())
    assert resume.store.load("/photos") == saved


def test_checkpoint_uses_scan_identity_without_filesystem_io(context):
    resume = open_folder(context, "/photos")
    identity = [9, 8, 7, 6, 5]
    context.app_state.get_file_data_by_path = lambda _path: {
        "bookmark_identity": identity
    }
    resume.store._file_identity = Mock(side_effect=AssertionError("No autosave stat"))
    resume.data_ready("organize")
    resume.checkpoint()
    identity[0] = 123  # A queued save must hold its own immutable snapshot.
    drain_until(lambda: not context.worker_manager.has_pending_folder_view_io())
    document = json.loads(resume.store._path("/photos").read_text())
    assert document["identities"] == {"organize.jpg": [9, 8, 7, 6, 5]}
    resume.store._file_identity.assert_not_called()


def test_grouped_cull_waits_for_clusters_before_restoring_filter_and_selection(window):
    from core.subject_grouping import CullClusteringResult

    resume = window.folder_resume_controller
    folder = "/photos/grouped"
    paths = [folder + "/a.jpg", folder + "/b.jpg"]
    resume.store.save(
        folder,
        {
            "workflow": "cull",
            "views": {
                "cull": {
                    "path": paths[1],
                    "selected": paths,
                    "similarity": True,
                    "cluster": "Cluster 0",
                    "mode": "list",
                    "sort": "Similarity then Time",
                }
            },
        },
    )
    window.app_state.image_files_data = [
        {"path": p, "media_type": "image"} for p in paths
    ]
    window.app_state.current_folder_path = folder
    resume.begin_folder(folder)
    drain_until(lambda: resume._loaded)
    resume.activate()
    window.app_controller.start_cull_similarity_workflow = Mock()
    window.show_cull_step()
    _app.processEvents()
    assert "cull" in resume._pending
    assert "cull" not in resume._ready
    window.app_controller._cull_grouping_fingerprints = (
        window.app_controller._similarity_fingerprints(paths)
    )
    window.app_controller.handle_cull_grouping_complete(
        CullClusteringResult(dict.fromkeys(paths, 0), "test")
    )
    drain_until(lambda: "cull" not in resume._pending)
    assert window.cluster_filter_combo.currentText() == "Cluster 0"
    assert set(window._get_selected_file_paths_from_view()) == set(paths)
    window.app_controller.start_cull_similarity_workflow.assert_called_once()


def test_normal_startup_does_not_open_a_folder(window):
    assert window.app_state.current_folder_path is None
    assert window.folder_resume_controller.folder is None
    assert window.worker_manager.file_scanner is None


def test_background_focus_cannot_replace_saved_active_workflow(context):
    resume = open_folder(context, "/photos")
    resume.data_ready("organize")
    resume.checkpoint()
    context.app_state.focused_image_path = "organize.jpg"
    assert not context.active_image_controller.publish(
        "background.jpg", source="easy_delete"
    )
    assert context.app_state.focused_image_path == "organize.jpg"
    # Explicit active-image changes still synchronize the visible adapter.
    context.active_image_controller.path_updated("organize.jpg", "renamed.jpg")
    context.adapters["organize"].focus_image.assert_called_with("renamed.jpg")


def test_queued_restore_waits_for_organize_rebuild(context):
    resume = open_folder(context, "/photos")
    resume.data_ready("organize")
    resume.checkpoint()
    resume.before_workflow_change("organize")
    resume.restore_workflow("organize")
    # A normal workflow transition begins regenerating its preview after the
    # page is shown. Do not consume the bookmark against the old tree.
    resume.data_loading("organize")
    _app.processEvents()
    context.adapters["organize"].restore_view_bookmark.assert_not_called()
    assert "organize" in resume._pending
    resume.data_ready("organize")
    context.active_image_controller.sync_workflow("organize")
    drain_until(lambda: "organize" not in resume._pending)
    context.adapters["organize"].restore_view_bookmark.assert_called_once()


def test_io_result_can_reenter_event_loop_without_double_shutdown(context):
    manager = context.worker_manager
    delivered = []

    def callback(_result):
        # A UI rebuild may process timer events while delivering a read result.
        manager._deliver_folder_view_jobs()
        delivered.append(True)

    manager.submit_folder_view_io(lambda: {}, callback)
    drain_until(lambda: bool(delivered))
    assert not manager.has_pending_folder_view_io()
    assert manager._folder_view_executor is None

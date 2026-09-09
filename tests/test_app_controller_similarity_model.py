from types import SimpleNamespace
from unittest.mock import Mock

from core.similarity_cache import SimilarityClusteringResult
from core.model_provisioning import EMBEDDING_MODEL
from ui.app_controller import AppController


class _Action:
    def __init__(self):
        self.enabled_values = []

    def setEnabled(self, value):
        self.enabled_values.append(value)


class _StatusBar:
    def __init__(self):
        self.messages = []

    def showMessage(self, message, timeout=0):
        self.messages.append((message, timeout))


class _DialogManager:
    def __init__(self, approve_download):
        self.approve_download = approve_download
        self.requested_model = None
        self.requested_features = []

    def confirm_model_download(self, model_keys, *, feature, fallback=""):
        self.requested_model = list(model_keys)
        self.requested_features.append(feature)
        return self.approve_download

    def confirm_slow_cpu_processing(self, feature):
        return self.approve_download


class _PickBestWidget:
    def __init__(self):
        self.errors = []
        self.loading_updates = []

    def show_loading(self, message, percent=0):
        self.loading_updates.append((message, percent))

    def show_error(self, message):
        self.errors.append(message)


class _MainWindow:
    def __init__(self, approve_download=False):
        self.dialog_manager = _DialogManager(approve_download)
        self.menu_manager = type(
            "MenuManager",
            (),
            {
                "analyze_similarity_action": _Action(),
            },
        )()
        self.pick_best_step_widget = _PickBestWidget()
        self.easy_delete_step_widget = _PickBestWidget()
        self.hidden = 0
        self.shown = []
        self.status_bar = _StatusBar()
        self.cull_dirty = 0
        self.cull_progress = []
        self.grouping_reverted = 0

    def revert_group_by_similarity(self):
        self.grouping_reverted += 1

    def mark_cull_model_dirty(self):
        self.cull_dirty += 1

    def _ensure_cull_model_ready(self):
        self.cull_model_ready_checks = getattr(self, "cull_model_ready_checks", 0) + 1

    def hide_loading_overlay(self):
        self.hidden += 1

    def show_loading_overlay(self, text):
        self.shown.append(text)

    def statusBar(self):
        return self.status_bar

    def show_cull_grouping_progress(self, message, percent):
        self.cull_progress.append((message, percent))

    def fail_cull_grouping_progress(self, message):
        self.cull_progress.append((message, "error"))

    def finish_cull_grouping_progress(self):
        self.cull_progress.append(("finished", 100))

    def cancel_cull_grouping_progress(self, message):
        self.cull_progress.append((message, "cancelled"))


class _WorkerManager:
    def __init__(self):
        self.started = False
        self.kwargs = None
        self.cull_started = False
        self.pick_best_started = False
        self.cull_stop_requested = False
        self.pick_best_stop_requested = False
        self.easy_delete_stop_requested = False
        self.model_environment_probe_requested = False

    def is_easy_delete_running(self):
        return False

    def request_stop_easy_delete_analysis(self):
        self.easy_delete_stop_requested = True

    def is_similarity_worker_running(self):
        return False

    def is_pick_best_running(self):
        return False

    def is_cull_grouping_running(self):
        return self.cull_started

    def start_similarity_analysis(self, paths, **kwargs):
        self.started = True
        self.paths = paths
        self.kwargs = kwargs

    def request_stop_similarity_analysis(self):
        pass

    def request_stop_cull_subject_grouping(self):
        self.cull_stop_requested = True

    def request_stop_pick_best_analysis(self):
        self.pick_best_stop_requested = True

    def start_model_environment_probe(self, model_keys):
        self.model_environment_probe_requested = True

    def is_model_environment_probe_running(self):
        return self.model_environment_probe_requested

    def start_cull_subject_grouping(self, **kwargs):
        self.cull_started = True
        self.cull_kwargs = kwargs

    def start_pick_best_analysis(self, cluster_map, *, allow_model_download=False):
        self.pick_best_started = True
        self.pick_best_cluster_map = cluster_map


class _AppState:
    image_files_data = [{"path": "/tmp/a.jpg", "media_type": "image"}]
    current_folder_path = "/tmp"
    analysis_cache = object()
    cluster_results = {"/tmp/a.jpg": 3}
    cull_cluster_results = {}
    cull_grouping_error = None
    date_cache = {}
    workflow_step = "cull"
    pick_best_results = {}
    marked_for_deletion = set()

    def clear_pick_best_results(self):
        self.pick_best_results = {}


def test_active_grouping_uses_coarse_similarity_outside_cull_workflows():
    controller = SimpleNamespace(
        app_state=SimpleNamespace(workflow_step="organize"),
        start_similarity_analysis=Mock(),
        start_cull_similarity_workflow=Mock(),
    )

    AppController.start_active_similarity_grouping(controller)

    controller.start_similarity_analysis.assert_called_once_with()
    controller.start_cull_similarity_workflow.assert_not_called()


def test_active_grouping_uses_same_subject_analysis_for_cull_and_pick_best():
    controller = SimpleNamespace(
        app_state=SimpleNamespace(workflow_step="cull"),
        start_similarity_analysis=Mock(),
        start_cull_similarity_workflow=Mock(),
    )

    AppController.start_active_similarity_grouping(controller)

    controller.start_cull_similarity_workflow.assert_called_once_with()
    controller.start_similarity_analysis.assert_not_called()


def test_similarity_declined_model_download_cancels_cleanly():
    main_window = _MainWindow(approve_download=False)
    worker_manager = _WorkerManager()
    controller = AppController(main_window, _AppState(), worker_manager)

    controller.start_similarity_analysis()
    assert worker_manager.started is False
    assert worker_manager.model_environment_probe_requested is True

    controller.handle_model_environment_ready((EMBEDDING_MODEL.key,), "mps")

    assert worker_manager.started is False
    assert main_window.dialog_manager.requested_model == [EMBEDDING_MODEL.key]
    assert main_window.hidden == 1
    assert any(
        "Model download was not approved" in message
        for message, _timeout in main_window.status_bar.messages
    )


def test_similarity_approved_model_download_starts_worker_with_download():
    main_window = _MainWindow(approve_download=True)
    worker_manager = _WorkerManager()
    controller = AppController(main_window, _AppState(), worker_manager)

    controller.start_similarity_analysis()
    controller.handle_model_environment_ready((EMBEDDING_MODEL.key,), "mps")

    assert worker_manager.started is True
    assert worker_manager.paths == ["/tmp/a.jpg"]
    assert worker_manager.kwargs == {
        "allow_model_download": True,
        "folder_path": "/tmp",
        "analysis_cache": _AppState.analysis_cache,
        "fingerprints": {},
    }
    assert main_window.shown == ["Starting similarity analysis..."]


def test_probe_resumes_similarity_only_after_cull_worker_releases_pipeline():
    main_window = _MainWindow(approve_download=True)
    worker_manager = _WorkerManager()
    controller = AppController(main_window, _AppState(), worker_manager)
    controller._deferred_starts.arm("cull_grouping")
    controller._deferred_starts.arm("similarity")

    controller.handle_model_environment_ready((), "mps")

    assert worker_manager.cull_started is True
    assert worker_manager.started is False
    assert controller._deferred_starts.is_armed("similarity") is True

    worker_manager.cull_started = False
    controller._resume_similarity_after_cull()

    assert worker_manager.started is True
    assert controller._deferred_starts.is_armed("similarity") is False


def test_declined_cull_models_leave_timeline_ungrouped_and_coarse_results_intact():
    state = _AppState()
    state.cluster_results = {"/tmp/a.jpg": 3}
    state.cull_cluster_results = {}
    state.cull_grouping_error = None
    main_window = _MainWindow(approve_download=False)
    worker_manager = _WorkerManager()
    controller = AppController(main_window, state, worker_manager)

    controller._start_cull_subject_grouping_background()
    controller.handle_model_environment_ready((EMBEDDING_MODEL.key,), "mps")

    assert state.cull_cluster_results == {}
    assert state.cluster_results == {"/tmp/a.jpg": 3}
    assert "requires the local Cull model" in state.cull_grouping_error
    assert main_window.cull_dirty == 1
    assert controller.is_cull_grouping_declined() is True


def test_declining_download_turns_off_the_group_by_similarity_toggle():
    """A stuck toggle would claim photos are grouped while showing them ungrouped."""
    state = _AppState()
    state.cull_cluster_results = {}
    main_window = _MainWindow(approve_download=False)
    controller = AppController(main_window, state, _WorkerManager())

    controller._start_cull_subject_grouping_background()
    controller.handle_model_environment_ready((EMBEDDING_MODEL.key,), "mps")

    assert main_window.grouping_reverted == 1


def test_cull_start_probes_environment_off_the_ui_thread(monkeypatch):
    """Importing torch or resolving snapshots must never block the GUI thread."""

    def _fail(*_args, **_kwargs):
        raise AssertionError("environment probing must run in a worker")

    monkeypatch.setattr(
        "core.subject_grouping_models.are_subject_models_installed", _fail
    )
    monkeypatch.setattr("core.app_settings.get_preferred_torch_device", _fail)
    state = _AppState()
    state.cull_cluster_results = {}
    worker_manager = _WorkerManager()
    controller = AppController(_MainWindow(), state, worker_manager)

    controller.start_cull_similarity_workflow()

    assert worker_manager.model_environment_probe_requested is True
    assert worker_manager.cull_started is False


def test_declined_prerequisites_are_remembered_until_an_explicit_restart():
    state = _AppState()
    state.cull_cluster_results = {}
    main_window = _MainWindow(approve_download=False)
    worker_manager = _WorkerManager()
    controller = AppController(main_window, state, worker_manager)

    controller._start_cull_subject_grouping_background()
    controller.handle_model_environment_ready((EMBEDDING_MODEL.key,), "mps")
    # Opening the Cull page again must not re-prompt for the refused download.
    assert controller.is_cull_grouping_declined() is True

    main_window.dialog_manager.approve_download = True
    controller.start_cull_similarity_workflow()

    assert controller.is_cull_grouping_declined() is False
    assert worker_manager.cull_started is True


def test_cull_consent_starts_single_model_analysis_without_coarse_pass():
    state = _AppState()
    state.cluster_results = {}
    state.cull_cluster_results = {}
    state.cull_grouping_error = None
    main_window = _MainWindow(approve_download=True)
    worker_manager = _WorkerManager()
    controller = AppController(main_window, state, worker_manager)

    controller.start_cull_similarity_workflow()
    controller.handle_model_environment_ready((EMBEDDING_MODEL.key,), "mps")

    assert main_window.dialog_manager.requested_model
    assert worker_manager.started is False
    assert worker_manager.cull_started is True
    assert main_window.cull_progress[0] == (
        "Starting fast DINO same-subject analysis…",
        0,
    )


def test_cull_progress_uses_current_stage_count_instead_of_setup_weight():
    main_window = SimpleNamespace(show_cull_grouping_progress=Mock())
    controller = SimpleNamespace(main_window=main_window)

    AppController.handle_cull_grouping_progress(
        controller, 10, "Encoding DINO features (48/2817)"
    )

    main_window.show_cull_grouping_progress.assert_called_once_with(
        "Encoding DINO features (48/2817)", 1
    )


def test_pick_best_same_subject_progress_updates_its_step():
    main_window = _MainWindow()
    controller = AppController(main_window, _AppState(), _WorkerManager())
    controller._pick_best_pending_after_subject_grouping = True

    controller.handle_cull_grouping_progress(23, "Encoding DINO features (48/2817)")

    assert main_window.pick_best_step_widget.loading_updates == [
        ("Step 1/2: Encoding DINO features (48/2817)", 1),
    ]


def test_pick_best_consumes_same_subject_groups_and_excludes_marked_images():
    paths = ["/tmp/a.jpg", "/tmp/b.jpg", "/tmp/c.jpg"]
    app_state = SimpleNamespace(
        image_files_data=[{"path": path, "media_type": "image"} for path in paths],
        cluster_results={path: 99 for path in paths},
        cull_cluster_results={paths[0]: 7, paths[1]: 7, paths[2]: 8},
        marked_for_deletion={paths[1]},
    )
    controller = AppController(object(), app_state, object())

    groups = controller._build_pick_best_cluster_map()

    assert groups == {7: [paths[0]], 8: [paths[2]]}


def test_regional_embedding_results_respect_cancelled_similarity_run():
    state = SimpleNamespace(regional_embeddings_cache={})
    controller = AppController(object(), state, object())
    controller._ignore_similarity_results = True

    controller.handle_regional_embeddings_generated({"/tmp/a.jpg": [[1.0, 0.0]]})

    assert state.regional_embeddings_cache == {}


def test_pick_best_prepares_shared_same_subject_groups_when_missing():
    state = SimpleNamespace(
        image_files_data=[{"path": "/tmp/a.jpg", "media_type": "image"}],
        current_folder_path="/tmp",
        cluster_results={"/tmp/a.jpg": 1},
        cull_cluster_results={},
        cull_grouping_error=None,
        pick_best_results={},
        marked_for_deletion=set(),
        analysis_cache=object(),
        date_cache={},
        workflow_step="pick_best",
    )
    main_window = _MainWindow()
    worker_manager = _WorkerManager()
    controller = AppController(main_window, state, worker_manager)
    controller.start_pick_best_workflow()
    controller.handle_model_environment_ready((), "mps")

    assert worker_manager.started is False
    assert worker_manager.cull_started is True
    assert controller._pick_best_pending_after_subject_grouping is True
    assert main_window.pick_best_step_widget.loading_updates[0] == (
        "Step 1/2: Preparing same-subject groups…",
        0,
    )


def test_pick_best_reuses_complete_same_subject_groups_without_new_model_pass():
    paths = ["/tmp/a.jpg", "/tmp/b.jpg"]
    state = SimpleNamespace(
        image_files_data=[{"path": path, "media_type": "image"} for path in paths],
        cull_cluster_results={path: 7 for path in paths},
        pick_best_results={},
        marked_for_deletion=set(),
    )
    main_window = _MainWindow()
    worker_manager = _WorkerManager()
    controller = AppController(main_window, state, worker_manager)

    controller.start_pick_best_workflow()
    controller.handle_model_environment_ready((), "mps")

    assert worker_manager.pick_best_started is True
    assert worker_manager.pick_best_cluster_map == {7: paths}
    assert worker_manager.cull_started is False
    assert worker_manager.started is False


def test_leaving_pick_best_cancels_its_pending_same_subject_worker():
    controller = AppController(_MainWindow(), _AppState(), _WorkerManager())
    controller._pick_best_pending_after_subject_grouping = True
    controller._pick_best_owns_subject_grouping = True

    controller.cancel_workflow_analysis("pick_best")

    assert controller._pick_best_pending_after_subject_grouping is False
    assert controller.worker_manager.pick_best_stop_requested is True
    assert controller.worker_manager.cull_stop_requested is True


def test_leaving_pick_best_does_not_cancel_cull_owned_grouping():
    worker_manager = _WorkerManager()
    worker_manager.cull_started = True
    controller = AppController(_MainWindow(), _AppState(), worker_manager)
    controller._pick_best_pending_after_subject_grouping = True
    controller._pick_best_owns_subject_grouping = False

    controller.cancel_workflow_analysis("pick_best")

    assert worker_manager.cull_stop_requested is False


def test_coarse_clusters_are_normalized_without_clearing_pick_best_results():
    action = SimpleNamespace(
        setEnabled=Mock(),
        setChecked=Mock(),
        isChecked=Mock(return_value=False),
    )
    menu = SimpleNamespace(
        analyze_similarity_action=action,
        group_by_similarity_action=action,
        update_cluster_filter_menu=Mock(),
        set_cluster_sort_menu_visible=Mock(),
        set_cluster_sort_menu_enabled=Mock(),
    )
    main_window = SimpleNamespace(
        menu_manager=menu,
        cluster_filter_combo=SimpleNamespace(
            clear=Mock(),
            addItems=Mock(),
            setEnabled=Mock(),
        ),
        cluster_sort_combo=SimpleNamespace(setEnabled=Mock()),
        update_loading_text=Mock(),
        refresh_navigation_shortcut_actions=Mock(),
        group_by_similarity_mode=False,
        hide_loading_overlay=Mock(),
    )
    state = SimpleNamespace(
        cluster_results={},
        clear_pick_best_results=Mock(),
    )
    controller = SimpleNamespace(
        app_state=state,
        main_window=main_window,
        _easy_delete_pending_after_similarity=False,
        _get_image_file_data=Mock(return_value=[{"path": "photo.jpg"}]),
    )

    AppController.handle_clustering_complete(
        controller,
        SimilarityClusteringResult(
            clusters={"legacy.jpg": "1 - 87.34%", "manual.jpg": 2},
            signature="signature",
            reused=False,
        ),
    )

    assert state.cluster_results == {"legacy.jpg": 1, "manual.jpg": 2}
    main_window.cluster_filter_combo.addItems.assert_called_once_with(
        ["All Clusters", "Cluster 1", "Cluster 2"]
    )
    state.clear_pick_best_results.assert_not_called()


def test_cancelling_easy_delete_drops_similarity_start_waiting_on_model_probe():
    main_window = _MainWindow(approve_download=True)
    worker_manager = _WorkerManager()
    controller = AppController(main_window, _AppState(), worker_manager)
    controller._easy_delete_pending_after_similarity = True

    controller.start_similarity_analysis()
    assert controller._deferred_starts.is_armed("similarity") is True

    controller.cancel_workflow_analysis("easy_delete")
    controller.handle_model_environment_ready((), "mps")

    assert worker_manager.started is False
    assert controller._deferred_starts.is_armed("similarity") is False


def test_cancelling_pick_best_drops_scoring_waiting_on_model_probe():
    paths = ["/tmp/a.jpg", "/tmp/b.jpg"]
    state = SimpleNamespace(
        image_files_data=[{"path": path, "media_type": "image"} for path in paths],
        cull_cluster_results={path: 7 for path in paths},
        pick_best_results={},
        marked_for_deletion=set(),
    )
    main_window = _MainWindow(approve_download=True)
    worker_manager = _WorkerManager()
    controller = AppController(main_window, state, worker_manager)

    controller.start_pick_best_workflow()
    assert controller._deferred_starts.is_armed("pick_best_scoring") is True
    assert controller.is_workflow_analysis_running("pick_best") is True

    controller.cancel_workflow_analysis("pick_best")
    controller.handle_model_environment_ready((), "mps")

    assert worker_manager.pick_best_started is False
    assert controller._deferred_starts.is_armed("pick_best_scoring") is False


def test_pick_best_owns_same_subject_grouping_deferred_behind_model_probe():
    state = SimpleNamespace(
        image_files_data=[{"path": "/tmp/a.jpg", "media_type": "image"}],
        current_folder_path="/tmp",
        cluster_results={"/tmp/a.jpg": 1},
        cull_cluster_results={},
        cull_grouping_error=None,
        pick_best_results={},
        marked_for_deletion=set(),
        analysis_cache=object(),
        date_cache={},
        workflow_step="pick_best",
    )
    main_window = _MainWindow(approve_download=True)
    worker_manager = _WorkerManager()
    controller = AppController(main_window, state, worker_manager)

    controller.start_pick_best_workflow()
    assert controller._deferred_starts.is_armed("cull_grouping") is True
    assert controller._pick_best_owns_subject_grouping is True

    controller.cancel_workflow_analysis("pick_best")
    assert worker_manager.cull_stop_requested is True
    assert controller._deferred_starts.is_armed("cull_grouping") is False

    controller.handle_model_environment_ready((), "mps")
    assert worker_manager.cull_started is False

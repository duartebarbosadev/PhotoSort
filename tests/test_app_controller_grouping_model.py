import pyexiv2  # noqa: F401  # Must be first to avoid Windows crash

from core.model_provisioning import EMBEDDING_MODEL
from ui.app_controller import AppController


class _StatusBar:
    def __init__(self):
        self.messages = []

    def showMessage(self, message, timeout=0):
        self.messages.append((message, timeout))


class _DialogManager:
    def __init__(self, approve_download):
        self.approve_download = approve_download
        self.requested_models = []
        self.requested_features = []

    def confirm_model_download(self, model_keys, *, feature, fallback=""):
        self.requested_models.append(list(model_keys))
        self.requested_features.append(feature)
        return self.approve_download

    def confirm_slow_cpu_processing(self, feature):
        return self.approve_download


class _GroupingStepWidget:
    def __init__(self):
        self.loading_states = []
        self.output_root_texts = []

    def set_output_root_text(self, text):
        self.output_root_texts.append(text)

    def set_loading_state(self, message, busy, progress=None):
        self.loading_states.append((message, busy, progress))

    def get_location_depth(self):
        return 3

    def set_preview_plan(self, plan, output_root):
        self.plan = (plan, output_root)


class _MainWindow:
    def __init__(self, approve_download=False):
        self.dialog_manager = _DialogManager(approve_download)
        self.grouping_step_widget = _GroupingStepWidget()
        self.preview_texts = []
        self.status_bar = _StatusBar()

    def update_grouping_preview(self, text):
        self.preview_texts.append(text)

    def statusBar(self):
        return self.status_bar


class _WorkerManager:
    def __init__(self):
        self.preview_calls = []
        self.model_environment_probe_requested = False

    def is_grouping_preview_running(self):
        return False

    def start_grouping_preview(self, items, mode, source_root, **kwargs):
        self.preview_calls.append((items, mode, source_root, kwargs))
        return True

    def start_model_environment_probe(self, model_keys):
        self.model_environment_probe_requested = True

    def is_model_environment_probe_running(self):
        return self.model_environment_probe_requested


class _AppState:
    def __init__(self):
        self.image_files_data = [{"path": "/tmp/a.jpg", "media_type": "image"}]
        self.current_folder_path = "/tmp"
        self.grouping_source_root = "/tmp"
        self.analysis_cache = object()
        self.selected_grouping_mode = "similarity"
        self.workflow_step = "organize"


def _controller(approve_download):
    main_window = _MainWindow(approve_download=approve_download)
    worker_manager = _WorkerManager()
    controller = AppController(main_window, _AppState(), worker_manager)
    return controller, main_window, worker_manager


def test_similarity_grouping_probes_environment_before_starting_preview():
    """The preview must never block the GUI thread importing torch."""
    controller, _main_window, worker_manager = _controller(approve_download=True)

    controller.refresh_grouping_preview()

    assert worker_manager.preview_calls == []
    assert worker_manager.model_environment_probe_requested is True


def test_similarity_grouping_prompts_for_download_and_passes_consent():
    controller, main_window, worker_manager = _controller(approve_download=True)

    controller.refresh_grouping_preview()
    controller.handle_model_environment_ready((EMBEDDING_MODEL.key,), "mps")

    assert main_window.dialog_manager.requested_models == [[EMBEDDING_MODEL.key]]
    assert len(worker_manager.preview_calls) == 1
    _items, mode, _source_root, kwargs = worker_manager.preview_calls[0]
    assert mode == "similarity"
    assert kwargs["allow_model_download"] is True


def test_declined_similarity_grouping_download_does_not_start_the_worker():
    controller, main_window, worker_manager = _controller(approve_download=False)

    controller.refresh_grouping_preview()
    controller.handle_model_environment_ready((EMBEDDING_MODEL.key,), "mps")

    assert worker_manager.preview_calls == []
    assert any("Download it to continue" in text for text in main_window.preview_texts)
    assert main_window.grouping_step_widget.loading_states[-1][1] is False


def test_grouping_modes_without_a_model_never_prompt():
    controller, main_window, worker_manager = _controller(approve_download=False)
    controller.app_state.selected_grouping_mode = "location"

    controller.refresh_grouping_preview()

    assert main_window.dialog_manager.requested_models == []
    assert worker_manager.model_environment_probe_requested is False
    assert len(worker_manager.preview_calls) == 1
    assert worker_manager.preview_calls[0][3]["allow_model_download"] is False


def test_installed_model_starts_preview_without_prompting():
    controller, main_window, worker_manager = _controller(approve_download=False)

    controller.refresh_grouping_preview()
    controller.handle_model_environment_ready((), "mps")

    assert main_window.dialog_manager.requested_models == []
    assert len(worker_manager.preview_calls) == 1
    assert worker_manager.preview_calls[0][3]["allow_model_download"] is False

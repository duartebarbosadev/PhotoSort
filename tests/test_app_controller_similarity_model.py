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

    def confirm_similarity_model_download(self, model_name):
        self.requested_model = model_name
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
                "analyze_best_shots_action": _Action(),
            },
        )()
        self.pick_best_step_widget = _PickBestWidget()
        self.hidden = 0
        self.shown = []
        self.status_bar = _StatusBar()

    def hide_loading_overlay(self):
        self.hidden += 1

    def show_loading_overlay(self, text):
        self.shown.append(text)

    def statusBar(self):
        return self.status_bar


class _WorkerManager:
    def __init__(self):
        self.started = False
        self.kwargs = None

    def is_similarity_worker_running(self):
        return False

    def start_similarity_analysis(self, paths, **kwargs):
        self.started = True
        self.paths = paths
        self.kwargs = kwargs


class _AppState:
    image_files_data = [{"path": "/tmp/a.jpg", "media_type": "image"}]


def test_similarity_declined_model_download_cancels_cleanly(monkeypatch):
    monkeypatch.setattr(
        "ui.app_controller.get_similarity_embedding_model_name",
        lambda: "facebook/dinov2-small",
    )
    monkeypatch.setattr(
        "ui.app_controller.is_similarity_model_installed", lambda _: False
    )

    main_window = _MainWindow(approve_download=False)
    worker_manager = _WorkerManager()
    controller = AppController(main_window, _AppState(), worker_manager)

    controller.start_similarity_analysis()

    assert worker_manager.started is False
    assert main_window.dialog_manager.requested_model == "facebook/dinov2-small"
    assert main_window.hidden == 1
    assert any(
        "Model download was not approved" in message
        for message, _timeout in main_window.status_bar.messages
    )


def test_similarity_approved_model_download_starts_worker_with_download(monkeypatch):
    monkeypatch.setattr(
        "ui.app_controller.get_similarity_embedding_model_name",
        lambda: "facebook/dinov2-small",
    )
    monkeypatch.setattr(
        "ui.app_controller.is_similarity_model_installed", lambda _: False
    )

    main_window = _MainWindow(approve_download=True)
    worker_manager = _WorkerManager()
    controller = AppController(main_window, _AppState(), worker_manager)

    controller.start_similarity_analysis()

    assert worker_manager.started is True
    assert worker_manager.paths == ["/tmp/a.jpg"]
    assert worker_manager.kwargs == {"allow_model_download": True}
    assert main_window.shown == ["Starting similarity analysis..."]


def test_pick_best_similarity_uses_step_progress_not_global_overlay(monkeypatch):
    monkeypatch.setattr(
        "ui.app_controller.get_similarity_embedding_model_name",
        lambda: "facebook/dinov2-small",
    )
    monkeypatch.setattr(
        "ui.app_controller.is_similarity_model_installed", lambda _: True
    )

    main_window = _MainWindow()
    worker_manager = _WorkerManager()
    controller = AppController(main_window, _AppState(), worker_manager)
    controller._pick_best_pending_after_similarity = True

    controller.start_similarity_analysis()
    controller.handle_similarity_progress(42, "Downloading facebook/dinov2-small")
    controller.handle_embeddings_generated({"/tmp/a.jpg": [1.0, 0.0]})

    assert worker_manager.started is True
    assert main_window.shown == []
    assert main_window.hidden == 1
    assert main_window.pick_best_step_widget.loading_updates == [
        ("Step 1/2: Starting similarity analysis...", 0),
        ("Step 1/2: Downloading facebook/dinov2-small", 42),
        ("Step 1/2: Embeddings generated. Clustering...", -1),
    ]

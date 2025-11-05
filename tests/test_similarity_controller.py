from src.ui.controllers.similarity_controller import SimilarityController


class DummyCtx:
    def __init__(self):
        class AppState:
            pass

        self.app_state = AppState()
        self.app_state.embeddings_cache = None
        self.app_state.cluster_results = {}
        self.worker_manager = type(
            "WM", (), {"started": False, "start_similarity_analysis": self._start}
        )()
        self.menu_manager = type(
            "MM",
            (),
            {
                "group_by_similarity_action": type(
                    "A",
                    (),
                    {
                        "setEnabled": lambda *a, **k: None,
                        "setChecked": lambda *a, **k: None,
                    },
                )(),
                "set_cluster_sort_menu_visible": lambda *a, **k: None,
                "set_cluster_sort_menu_enabled": lambda *a, **k: None,
                "update_cluster_filter_menu": lambda *a, **k: None,
            },
        )()
        self.loading = []
        self.statuses = []
        self.rebuilt = 0
        self.cluster_combo_enabled = False
        self.cluster_ids = []

    def _start(self, paths):
        self.worker_manager.started = True
        self.worker_manager.paths = paths

    def show_loading_overlay(self, text):
        self.loading.append(("show", text))

    def hide_loading_overlay(self):
        self.loading.append(("hide",))

    def update_loading_text(self, text):
        self.loading.append(("update", text))

    def status_message(self, msg, timeout=3000):
        self.statuses.append(msg)

    def rebuild_model_view(self):
        self.rebuilt += 1

    def enable_group_by_similarity(self, enabled):
        pass

    def set_group_by_similarity_checked(self, checked):
        pass

    def set_cluster_sort_visible(self, visible):
        pass

    def enable_cluster_sort_combo(self, enabled):
        self.cluster_combo_enabled = enabled

    def populate_cluster_filter(self, cluster_ids):
        self.cluster_ids = cluster_ids


def test_similarity_start_and_clustering_flow():
    ctx = DummyCtx()
    sc = SimilarityController(ctx)
    sc.start(["a.jpg", "b.jpg"])
    assert ctx.worker_manager.started is True
    assert ctx.worker_manager.paths == ["a.jpg", "b.jpg"]
    sc.embeddings_generated({"a.jpg": [0, 1], "b.jpg": [1, 0]})
    assert ctx.app_state.embeddings_cache is not None
    sc.clustering_complete({"a.jpg": 1, "b.jpg": 2}, group_mode=True)
    assert ctx.app_state.cluster_results == {"a.jpg": 1, "b.jpg": 2}
    assert ctx.cluster_ids == [1, 2]
    assert ctx.rebuilt == 1


def test_similarity_no_paths():
    ctx = DummyCtx()
    sc = SimilarityController(ctx)
    sc.start([])
    assert ctx.worker_manager.started is False
    assert any("No valid image paths" in s for s in ctx.statuses)

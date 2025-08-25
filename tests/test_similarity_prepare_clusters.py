from datetime import date as date_obj

from src.ui.controllers.similarity_controller import SimilarityController


class DummyAppState:
    def __init__(self):
        self.image_files_data = []
        self.cluster_results = {}
        self.date_cache = {}
        self.embeddings_cache = {}


class DummyCtx:
    def __init__(self, app_state):
        self.app_state = app_state
        # Unused protocol methods for these tests
        self.worker_manager = None
        self.menu_manager = None

    def show_loading_overlay(self, text):
        pass

    def hide_loading_overlay(self):
        pass

    def update_loading_text(self, text):
        pass

    def status_message(self, msg, timeout=3000):
        pass

    def rebuild_model_view(self):
        pass

    def enable_group_by_similarity(self, enabled):
        pass

    def set_group_by_similarity_checked(self, checked):
        pass

    def set_cluster_sort_visible(self, visible):
        pass

    def enable_cluster_sort_combo(self, enabled):
        pass

    def populate_cluster_filter(self, cluster_ids):
        pass


def _build_images(paths):
    # minimal image file data dicts
    return [{"path": p} for p in paths]


def test_prepare_clusters_empty():
    app_state = DummyAppState()
    ctx = DummyCtx(app_state)
    sc = SimilarityController(ctx)
    info = sc.prepare_clusters("Default")
    assert info["images_by_cluster"] == {}
    assert info["sorted_cluster_ids"] == []
    assert info["total_images"] == 0


def test_prepare_clusters_default_numeric():
    app_state = DummyAppState()
    app_state.image_files_data = _build_images(["a.jpg", "b.jpg", "c.jpg"])
    app_state.cluster_results = {"a.jpg": 2, "b.jpg": 1, "c.jpg": 3}
    ctx = DummyCtx(app_state)
    sc = SimilarityController(ctx)
    info = sc.prepare_clusters("Default")
    assert info["sorted_cluster_ids"] == [1, 2, 3]
    assert info["total_images"] == 3


def test_prepare_clusters_time_sort():
    app_state = DummyAppState()
    app_state.image_files_data = _build_images(["x.jpg", "y.jpg", "z.jpg"])
    app_state.cluster_results = {"x.jpg": 10, "y.jpg": 5, "z.jpg": 10}
    # Provide dates: cluster 10 earliest is 2024-01-01, cluster 5 earliest 2024-06-01 -> 10 should come first
    app_state.date_cache = {
        "x.jpg": date_obj(2024, 1, 1),
        "y.jpg": date_obj(2024, 6, 1),
        "z.jpg": date_obj(2024, 2, 1),
    }
    ctx = DummyCtx(app_state)
    sc = SimilarityController(ctx)
    info = sc.prepare_clusters("Time")
    assert info["sorted_cluster_ids"] == [10, 5]


def test_prepare_clusters_similarity_then_time_with_embeddings():
    app_state = DummyAppState()
    app_state.image_files_data = _build_images(["p.jpg", "q.jpg", "r.jpg", "s.jpg"])
    app_state.cluster_results = {"p.jpg": 1, "q.jpg": 2, "r.jpg": 1, "s.jpg": 3}
    # identical timestamps so PCA/similarity ordering decides before time fallback
    app_state.date_cache = {
        f: date_obj(2024, 1, 1) for f in ["p.jpg", "q.jpg", "r.jpg", "s.jpg"]
    }
    # embeddings: construct 2D vectors so PCA produces deterministic ordering by first component
    app_state.embeddings_cache = {
        "p.jpg": [0.1, 0.0],
        "r.jpg": [0.2, 0.0],  # cluster 1 centroid ~0.15
        "q.jpg": [0.5, 0.0],  # cluster 2 centroid 0.5
        "s.jpg": [0.3, 0.0],  # cluster 3 centroid 0.3
    }
    ctx = DummyCtx(app_state)
    sc = SimilarityController(ctx)
    info = sc.prepare_clusters("Similarity then Time")
    # Expected centroid ordering by PCA (monotonic with x values here): cluster1(~0.15), cluster3(0.3), cluster2(0.5)
    assert info["sorted_cluster_ids"] == [1, 3, 2]


def test_prepare_clusters_similarity_then_time_without_embeddings_fallback_to_time():
    app_state = DummyAppState()
    app_state.image_files_data = _build_images(["m.jpg", "n.jpg", "o.jpg"])
    app_state.cluster_results = {"m.jpg": 7, "n.jpg": 8, "o.jpg": 7}
    app_state.date_cache = {
        "m.jpg": date_obj(2024, 5, 1),
        "n.jpg": date_obj(2024, 4, 1),
        "o.jpg": date_obj(2024, 5, 2),
    }
    # No embeddings -> should fallback to time ordering: cluster 7 earliest 2024-05-01; cluster 8 earliest 2024-04-01 -> 8 first
    ctx = DummyCtx(app_state)
    sc = SimilarityController(ctx)
    info = sc.prepare_clusters("Similarity then Time")
    assert info["sorted_cluster_ids"] == [8, 7]

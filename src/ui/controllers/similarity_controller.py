from __future__ import annotations
from typing import Protocol, Dict, List, Any, Optional
from datetime import date as date_obj

from src.ui.helpers.cluster_utils import ClusterUtils


class SimilarityContext(Protocol):
    app_state: object
    worker_manager: object
    menu_manager: object

    def show_loading_overlay(self, text: str) -> None: ...
    def hide_loading_overlay(self) -> None: ...
    def update_loading_text(self, text: str) -> None: ...
    def status_message(self, msg: str, timeout: int = 3000) -> None: ...
    def rebuild_model_view(self) -> None: ...
    def enable_group_by_similarity(self, enabled: bool) -> None: ...
    def set_group_by_similarity_checked(self, checked: bool) -> None: ...
    def set_cluster_sort_visible(self, visible: bool) -> None: ...
    def enable_cluster_sort_combo(self, enabled: bool) -> None: ...
    def populate_cluster_filter(self, cluster_ids: List[int]) -> None: ...


class AppStateSimilarityView(Protocol):
    """Protocol subset of AppState attributes used by SimilarityController.

    Keeps controller loosely coupled to the concrete AppState implementation.
    """

    image_files_data: List[Dict[str, Any]]
    cluster_results: Dict[str, int]
    date_cache: Dict[str, Optional[date_obj]]
    embeddings_cache: Dict[str, List[float]] | Dict[str, Any]


class SimilarityController:
    def __init__(self, ctx: SimilarityContext):
        self.ctx = ctx

    def start(self, paths: List[str], auto_edits: bool):
        if not paths:
            self.ctx.status_message("No valid image paths for similarity analysis.")
            return
        self.ctx.show_loading_overlay("Starting similarity analysis...")
        self.ctx.worker_manager.start_similarity_analysis(paths, auto_edits)

    def embeddings_generated(self, embeddings_dict):
        self.ctx.app_state.embeddings_cache = embeddings_dict
        self.ctx.update_loading_text("Embeddings generated. Clustering...")

    def clustering_complete(self, cluster_results: Dict[str, int], group_mode: bool):
        self.ctx.app_state.cluster_results = cluster_results
        if not cluster_results:
            self.ctx.hide_loading_overlay()
            self.ctx.status_message("Clustering did not produce results.")
            return
        self.ctx.update_loading_text("Clustering complete. Updating view...")
        cluster_ids = sorted(set(cluster_results.values()))
        self.ctx.populate_cluster_filter(cluster_ids)
        self.ctx.enable_group_by_similarity(True)
        self.ctx.set_group_by_similarity_checked(True)
        if cluster_results:
            self.ctx.set_cluster_sort_visible(True)
            self.ctx.enable_cluster_sort_combo(True)
        if group_mode:
            self.ctx.rebuild_model_view()
        self.ctx.hide_loading_overlay()

    def error(self, message: str):
        self.ctx.status_message(f"Similarity Error: {message}", 8000)
        self.ctx.hide_loading_overlay()

    # --- New extracted logic for cluster grouping / sorting ---
    def get_images_by_cluster(self) -> Dict[int, List[Dict[str, Any]]]:
        """Return mapping cluster_id -> list of image dicts.

        This excludes any paths absent from app_state.image_files_data ensuring
        the UI only renders data with full metadata available.
        """
        app_state = getattr(self.ctx, "app_state", None)
        if not app_state:
            return {}
        return ClusterUtils.group_images_by_cluster(
            getattr(app_state, "image_files_data", []),
            getattr(app_state, "cluster_results", {}) or {},
        )

    def _get_cluster_timestamps(
        self,
        images_by_cluster: Dict[int, List[Dict[str, Any]]],
        date_cache: Dict[str, Optional[date_obj]],
    ) -> Dict[int, date_obj]:
        return ClusterUtils.get_cluster_timestamps(images_by_cluster, date_cache)

    def _sort_by_similarity_time(
        self,
        images_by_cluster: Dict[int, List[Dict[str, Any]]],
        embeddings_cache: Dict[str, List[float]],
        date_cache: Dict[str, Optional[date_obj]],
    ) -> List[int]:
        return ClusterUtils.sort_clusters_by_similarity_time(
            images_by_cluster, embeddings_cache, date_cache
        )

    def sort_cluster_ids(
        self,
        images_by_cluster: Dict[int, List[Dict[str, Any]]],
        sort_method: str,
    ) -> List[int]:
        """Return ordered cluster ids based on sort method.

        Methods:
          Default -> numeric ascending
          Time -> earliest timestamp among items in cluster
          Similarity then Time -> PCA order of centroids then earliest timestamp

        Falls back to Time ordering if embeddings are missing or insufficient
        for PCA (handled inside ClusterUtils).
        """
        cluster_ids = list(images_by_cluster.keys())
        if not cluster_ids:
            return []
        app_state = getattr(self.ctx, "app_state", None)
        date_cache = getattr(app_state, "date_cache", {}) if app_state else {}
        embeddings_cache = (
            getattr(app_state, "embeddings_cache", {}) if app_state else {}
        )

        if sort_method == "Time":
            timestamps = self._get_cluster_timestamps(images_by_cluster, date_cache)
            cluster_ids.sort(key=lambda cid: timestamps.get(cid, date_obj.max))
        elif sort_method == "Similarity then Time":
            if not embeddings_cache:
                timestamps = self._get_cluster_timestamps(images_by_cluster, date_cache)
                cluster_ids.sort(key=lambda cid: timestamps.get(cid, date_obj.max))
            else:
                cluster_ids = self._sort_by_similarity_time(
                    images_by_cluster, embeddings_cache, date_cache
                )
        else:  # Default or unknown
            cluster_ids.sort()
        return cluster_ids

    def prepare_clusters(self, sort_method: str) -> Dict[str, Any]:
        """Return structured cluster info for view construction.

        Returns keys:
          images_by_cluster -> cluster_id -> list[image dict]
          sorted_cluster_ids -> display order
          total_images -> total images assigned to clusters
        """
        images_by_cluster = self.get_images_by_cluster()
        if not images_by_cluster:
            return {
                "images_by_cluster": {},
                "sorted_cluster_ids": [],
                "total_images": 0,
            }
        sorted_cluster_ids = self.sort_cluster_ids(images_by_cluster, sort_method)
        total_images = sum(len(v) for v in images_by_cluster.values())
        return {
            "images_by_cluster": images_by_cluster,
            "sorted_cluster_ids": sorted_cluster_ids,
            "total_images": total_images,
        }

import os
import logging
from datetime import date as date_obj
from typing import Dict, List, Optional, Any

import numpy as np
from sklearn.decomposition import PCA

logger = logging.getLogger(__name__)


class ClusterUtils:
    """Utility helpers for grouping images into clusters and ordering them.

    These are extracted from the previous monolithic MainWindow implementation
    to enable unit testing without a running Qt event loop.
    """

    @staticmethod
    def group_images_by_cluster(
        image_files_data: List[Dict[str, Any]],
        cluster_results: Dict[str, int],
    ) -> Dict[int, List[Dict[str, Any]]]:
        images_by_cluster: Dict[int, List[Dict[str, Any]]] = {}
        if not image_files_data or not cluster_results:
            return images_by_cluster

        image_data_map = {img_data.get("path"): img_data for img_data in image_files_data if isinstance(img_data, dict)}
        for file_path, cluster_id in cluster_results.items():
            if file_path in image_data_map:
                images_by_cluster.setdefault(cluster_id, []).append(image_data_map[file_path])
        return images_by_cluster

    @staticmethod
    def get_cluster_timestamps(
        images_by_cluster: Dict[int, List[Dict[str, Any]]],
        date_cache: Dict[str, Optional[date_obj]],
    ) -> Dict[int, date_obj]:
        cluster_timestamps: Dict[int, date_obj] = {}
        for cluster_id, file_data_list in images_by_cluster.items():
            earliest_date = date_obj.max
            found_date = False
            for file_data in file_data_list:
                path = file_data.get("path") if isinstance(file_data, dict) else None
                if not path:
                    continue
                img_date = date_cache.get(path)
                if img_date and img_date < earliest_date:
                    earliest_date = img_date
                    found_date = True
            cluster_timestamps[cluster_id] = earliest_date if found_date else date_obj.max
        return cluster_timestamps

    @staticmethod
    def calculate_cluster_centroids(
        images_by_cluster: Dict[int, List[Dict[str, Any]]],
        embeddings_cache: Dict[str, List[float]],
    ) -> Dict[int, np.ndarray]:
        centroids: Dict[int, np.ndarray] = {}
        if not embeddings_cache:
            return centroids
        for cluster_id, file_data_list in images_by_cluster.items():
            cluster_embeddings = []
            for file_data in file_data_list:
                path = file_data.get("path") if isinstance(file_data, dict) else None
                if not path:
                    continue
                embedding = embeddings_cache.get(path)
                if embedding is None:
                    continue
                if isinstance(embedding, np.ndarray):
                    cluster_embeddings.append(embedding)
                elif isinstance(embedding, list):
                    try:
                        cluster_embeddings.append(np.array(embedding, dtype=np.float32))
                    except Exception:  # pragma: no cover - defensive
                        pass
            if cluster_embeddings:
                try:
                    centroids[cluster_id] = np.mean(np.stack(cluster_embeddings), axis=0)
                except Exception as e:  # pragma: no cover - defensive
                    logger.error(f"Error calculating centroid for cluster {cluster_id}: {e}")
        return centroids

    @staticmethod
    def sort_clusters_by_similarity_time(
        images_by_cluster: Dict[int, List[Dict[str, Any]]],
        embeddings_cache: Dict[str, List[float]],
        date_cache: Dict[str, Optional[date_obj]],
    ) -> List[int]:
        """Sort clusters using PCA of centroids first, then earliest timestamp.

        If PCA is not feasible (e.g., <2 valid centroids) falls back to timestamp ordering.
        """
        cluster_ids = list(images_by_cluster.keys())
        if not cluster_ids:
            return []
        centroids = ClusterUtils.calculate_cluster_centroids(images_by_cluster, embeddings_cache)
        valid_cluster_ids_for_pca = [
            cid for cid in cluster_ids if isinstance(centroids.get(cid), np.ndarray) and centroids[cid].size > 0
        ]
        if len(valid_cluster_ids_for_pca) < 2:
            # Fallback to time sort
            cluster_timestamps_fb = ClusterUtils.get_cluster_timestamps(images_by_cluster, date_cache)
            return sorted(cluster_ids, key=lambda cid: cluster_timestamps_fb.get(cid, date_obj.max))

        centroid_matrix = np.stack([centroids[cid] for cid in valid_cluster_ids_for_pca])
        pca_scores = {}
        if centroid_matrix.ndim == 2 and centroid_matrix.shape[0] > 1 and centroid_matrix.shape[1] > 0:
            try:
                n_components = 1 if centroid_matrix.shape[0] > 1 else 0
                if n_components > 0:
                    pca = PCA(n_components=n_components)
                    transformed = pca.fit_transform(centroid_matrix)
                    for i, cid in enumerate(valid_cluster_ids_for_pca):
                        pca_scores[cid] = transformed[i, 0]
            except Exception as e:  # pragma: no cover - defensive
                logger.error(f"Error during PCA sorting: {e}")

        # Build timestamp map
        cluster_timestamps = ClusterUtils.get_cluster_timestamps(images_by_cluster, date_cache)
        sortable = []
        for cid in cluster_ids:
            pca_val = pca_scores.get(cid, float("inf"))
            ts_val = cluster_timestamps.get(cid, date_obj.max)
            sortable.append((cid, pca_val, ts_val))
        sortable.sort(key=lambda x: (x[1], x[2]))
        return [cid for cid, _, _ in sortable]

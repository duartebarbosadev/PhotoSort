import os
import time
import pickle  # For caching embeddings
import logging
from PyQt6.QtCore import QObject, pyqtSignal
import numpy as np  # Import numpy for array manipulation
from sklearn.cluster import DBSCAN
from core.utils.time_utils import format_eta

from core.image_pipeline import ANALYSIS_CACHE_RESOLUTION, ImagePipeline
from core.image_file_ops import ImageFileOperations
from core.similarity_embedding_model import (
    SimilarityEmbeddingModel,
    SimilarityModelDownloadError,
    SimilarityModelNotInstalledError,
)
from core.similarity_utils import (
    adaptive_dbscan_eps,
    build_orientation_map,
    normalize_embedding_dict,
    l2_normalize_rows,
    Orientation,
)
from .app_settings import (
    DBSCAN_MIN_SAMPLES,
    DEFAULT_SIMILARITY_BATCH_SIZE,
    get_similarity_clustering_eps,
    get_similarity_embedding_model_name,
)  # Import from app_settings
from .runtime_paths import get_app_cache_root, resolve_user_cache_dir

logger = logging.getLogger(__name__)

_module_init_start_time = time.time()
logger.debug("Initializing SimilarityEngine module...")


class SimilarityEngine(QObject):
    """
    Handles image feature extraction (embeddings) using visual foundation models
    and potentially clustering similar images. Designed for background execution.
    """

    progress_update = pyqtSignal(int, str)
    embeddings_generated = pyqtSignal(dict)
    clustering_complete = pyqtSignal(dict)
    error = pyqtSignal(str)

    def __init__(
        self,
        model_name: str | None = None,
        *,
        allow_model_download: bool = False,
        image_pipeline: ImagePipeline | None = None,
        parent=None,
    ):
        super().__init__(parent)
        init_start_time = time.perf_counter()
        logger.info("Initializing SimilarityEngine...")
        self.model_name = model_name or get_similarity_embedding_model_name()
        self.model = SimilarityEmbeddingModel(
            self.model_name,
            allow_download=allow_model_download,
            progress_callback=self._handle_model_progress,
        )
        self._is_running = False
        self._cache_filename = f"embeddings_{self.model.cache_key}.pkl"
        self._region_cache_filename = f"embeddings_{self.model.region_cache_key}.pkl"
        embedding_cache_dir = resolve_user_cache_dir("embeddings")
        self._cache_path = os.path.join(embedding_cache_dir, self._cache_filename)
        self._region_cache_path = os.path.join(
            embedding_cache_dir, self._region_cache_filename
        )

        self.image_pipeline = image_pipeline or ImagePipeline()
        logger.info(
            f"SimilarityEngine initialized in {time.perf_counter() - init_start_time:.4f}s"
        )

    def stop(self):
        logger.info("Stop request received.")
        self._is_running = False

    def _handle_model_progress(self, percent: int, message: str) -> None:
        self.progress_update.emit(percent, message)

    def run_analysis_sync(
        self,
        file_paths: list[str],
        progress_callback=None,
    ) -> tuple[dict[str, list[float]], dict[str, str]]:
        """Run the existing similarity pipeline synchronously and return its results.

        This wraps the signal-based workflow so non-Qt callers can reuse the
        exact same embedding + clustering implementation as the UI action.
        """
        embeddings_result: dict[str, list[float]] = {}
        cluster_results: dict[str, str] = {}
        errors: list[str] = []

        def _on_progress(percent: int, message: str):
            if progress_callback:
                progress_callback(percent, message)

        def _on_embeddings(data):
            nonlocal embeddings_result
            embeddings_result = dict(data or {})

        def _on_complete(data):
            nonlocal cluster_results
            cluster_results = dict(data or {})

        def _on_error(message: str):
            if message:
                errors.append(message)

        self.progress_update.connect(_on_progress)
        self.embeddings_generated.connect(_on_embeddings)
        self.clustering_complete.connect(_on_complete)
        self.error.connect(_on_error)

        try:
            self.generate_embeddings_for_files(file_paths)
        finally:
            try:
                self.progress_update.disconnect(_on_progress)
            except Exception:
                pass
            try:
                self.embeddings_generated.disconnect(_on_embeddings)
            except Exception:
                pass
            try:
                self.clustering_complete.disconnect(_on_complete)
            except Exception:
                pass
            try:
                self.error.disconnect(_on_error)
            except Exception:
                pass

        if errors and not cluster_results:
            raise RuntimeError(errors[-1])
        return embeddings_result, cluster_results

    def _load_model(self):
        try:
            model_load_start_time = time.perf_counter()
            logger.info("Loading similarity model: %s", self.model_name)
            self.progress_update.emit(0, f"Loading model: {self.model_name}...")
            self.model.load()
            self.progress_update.emit(0, "Model loaded.")
            logger.info(
                "Similarity model loading complete in %.4fs",
                time.perf_counter() - model_load_start_time,
            )
            return True
        except SimilarityModelNotInstalledError as e:
            logger.warning("Similarity model is not installed: %s", e)
            self.error.emit(str(e))
        except SimilarityModelDownloadError as e:
            logger.error("Similarity model download/load failed: %s", e, exc_info=True)
            self.error.emit(str(e))
        except Exception as e:
            error_msg = f"Error loading similarity model '{self.model_name}': {e}"
            logger.error(error_msg, exc_info=True)
            self.error.emit(error_msg)
        return False

    def _load_cached_embeddings(self) -> dict[str, list[float]]:
        if os.path.exists(self._cache_path):
            try:
                cache_load_start_time = time.perf_counter()
                logger.info(f"Loading embeddings cache: {self._cache_path}")
                with open(self._cache_path, "rb") as f:
                    cache_data = pickle.load(f)
                    if isinstance(cache_data, dict) and cache_data:
                        if normalize_embedding_dict(cache_data):
                            logger.info(
                                "Detected legacy non-normalized embeddings. "
                                "Updating cache to normalized vectors."
                            )
                            self._save_embeddings_to_cache(cache_data)
                    logger.info(
                        f"Loaded {len(cache_data)} embeddings from cache in {time.perf_counter() - cache_load_start_time:.4f}s"
                    )
                    return cache_data
            except Exception as e:
                logger.warning(
                    f"Failed to load embedding cache '{self._cache_path}': {e}. A new cache will be created."
                )
        return {}

    def _save_embeddings_to_cache(self, embeddings: dict[str, list[float]]):
        try:
            cache_save_start_time = time.perf_counter()
            logger.info(
                f"Saving {len(embeddings)} embeddings to cache: {self._cache_path}"
            )
            with open(self._cache_path, "wb") as f:
                pickle.dump(embeddings, f)
            logger.info(
                f"Embeddings saved in {time.perf_counter() - cache_save_start_time:.4f}s"
            )
        except Exception as e:
            logger.error(
                f"Failed to save embedding cache '{self._cache_path}': {e}",
                exc_info=True,
            )
            self.error.emit(f"Could not save embeddings cache: {e}")

    def _load_cached_regional_embeddings(self) -> dict[str, list[list[float]]]:
        if os.path.exists(self._region_cache_path):
            try:
                cache_load_start_time = time.perf_counter()
                with open(self._region_cache_path, "rb") as f:
                    cache_data = pickle.load(f)
                if isinstance(cache_data, dict):
                    normalized_cache: dict[str, list[list[float]]] = {}
                    for path, region_vectors in cache_data.items():
                        try:
                            region_matrix = l2_normalize_rows(
                                np.asarray(region_vectors, dtype=np.float32)
                            )
                        except TypeError, ValueError:
                            continue
                        if region_matrix.ndim == 2 and region_matrix.shape[0] > 0:
                            normalized_cache[path] = region_matrix.tolist()
                    logger.info(
                        "Loaded %d regional embedding sets from cache in %.4fs",
                        len(normalized_cache),
                        time.perf_counter() - cache_load_start_time,
                    )
                    return normalized_cache
            except Exception as e:
                logger.warning(
                    "Failed to load regional embedding cache '%s': %s. A new cache will be created.",
                    self._region_cache_path,
                    e,
                )
        return {}

    def _save_regional_embeddings_to_cache(
        self, regional_embeddings: dict[str, list[list[float]]]
    ):
        try:
            cache_save_start_time = time.perf_counter()
            logger.info(
                "Saving %d regional embedding sets to cache: %s",
                len(regional_embeddings),
                self._region_cache_path,
            )
            with open(self._region_cache_path, "wb") as f:
                pickle.dump(regional_embeddings, f)
            logger.info(
                "Regional embeddings saved in %.4fs",
                time.perf_counter() - cache_save_start_time,
            )
        except Exception as e:
            logger.error(
                "Failed to save regional embedding cache '%s': %s",
                self._region_cache_path,
                e,
                exc_info=True,
            )
            self.error.emit(f"Could not save regional embeddings cache: {e}")

    def generate_embeddings_for_files(self, file_paths: list[str]):
        self._is_running = True
        logger.info(f"Starting embedding generation for {len(file_paths)} files.")

        if not self._load_model():
            return

        cached_embeddings = self._load_cached_embeddings()
        cached_regional_embeddings = self._load_cached_regional_embeddings()
        all_embeddings = cached_embeddings.copy()
        all_regional_embeddings = cached_regional_embeddings.copy()
        files_to_process = [
            path
            for path in file_paths
            if (path not in all_embeddings or path not in all_regional_embeddings)
            and self._is_running
        ]

        total_to_process = len(files_to_process)
        logger.info(
            f"Found {len(all_embeddings)} cached. Processing {total_to_process} new files."
        )
        eta_placeholder = "--:--:--"
        self.progress_update.emit(
            0, f"Processing {total_to_process} new images... • ETA {eta_placeholder}"
        )

        processed_count = 0
        new_embeddings = {}
        new_regional_embeddings = {}
        batch_size = DEFAULT_SIMILARITY_BATCH_SIZE
        start_time = time.perf_counter()

        for i in range(0, total_to_process, batch_size):
            if not self._is_running:
                logger.info("Embedding generation stopped.")
                break

            batch_paths = files_to_process[i : i + batch_size]
            batch_images = []
            valid_paths_in_batch = []

            for batch_offset, path in enumerate(batch_paths, start=1):
                img = self.image_pipeline.get_analysis_image(
                    path,
                    target_size=ANALYSIS_CACHE_RESOLUTION,
                )
                if img:
                    logger.debug(
                        f"Successfully got PIL image for '{os.path.basename(path)}' (Size: {img.size})"
                    )
                    batch_images.append(img)
                    valid_paths_in_batch.append(path)
                else:
                    logger.debug(
                        f"Skipping '{os.path.basename(path)}': Could not load preview."
                    )
                    # Ensure skipped files are still counted towards progress if they were in files_to_process
                    # This is handled later by `processed_count += len(valid_paths_in_batch)` and how total_to_process is calculated.
                    # If a file is skipped here, it won't be in valid_paths_in_batch.

                loaded_count = min(i + batch_offset, total_to_process)
                if total_to_process > 0:
                    self.progress_update.emit(
                        int((loaded_count / total_to_process) * 100),
                        f"Preparing images ({loaded_count}/{total_to_process})",
                    )

            if not batch_images:
                # This means all paths in the current batch_paths failed to load a preview.
                logger.warning(
                    f"No valid previews loaded for batch starting at index {i}. Skipping batch."
                )
                continue

            try:
                logger.debug(
                    f"Encoding batch {i // batch_size + 1}/{(total_to_process + batch_size - 1) // batch_size}..."
                )
                batch_embeds, batch_region_embeds = self.model.encode_with_regions(
                    batch_images
                )
                batch_embeds = np.asarray(batch_embeds, dtype=np.float32)
                batch_embeds = l2_normalize_rows(batch_embeds)

                for path_idx, path in enumerate(valid_paths_in_batch):
                    new_embeddings[path] = batch_embeds[path_idx].tolist()
                    new_regional_embeddings[path] = batch_region_embeds[
                        path_idx
                    ].tolist()

                processed_count += len(valid_paths_in_batch)
                progress = (
                    int((processed_count / total_to_process) * 100)
                    if total_to_process > 0
                    else 100
                )
                elapsed = max(0.0, time.perf_counter() - start_time)
                avg = elapsed / processed_count
                remaining = max(0, total_to_process - processed_count)
                eta_seconds = avg * remaining
                eta_text = format_eta(eta_seconds)
                self.progress_update.emit(
                    progress,
                    f"Generating embeddings ({processed_count}/{total_to_process}) • ETA {eta_text}",
                )

            except Exception as e:
                logger.error("Error encoding image batch.", exc_info=True)
                self.error.emit(f"Error during embedding generation: {e}")
                processed_count += len(
                    valid_paths_in_batch
                )  # Still count them for progress

        logger.info("Finished processing new files.")
        all_embeddings.update(new_embeddings)
        all_regional_embeddings.update(new_regional_embeddings)
        if new_embeddings:
            self._save_embeddings_to_cache(all_embeddings)
        if new_regional_embeddings:
            self._save_regional_embeddings_to_cache(all_regional_embeddings)

        final_embeddings_for_requested_files = {
            path: all_embeddings[path] for path in file_paths if path in all_embeddings
        }
        final_regional_embeddings_for_requested_files = {
            path: all_regional_embeddings[path]
            for path in file_paths
            if path in all_regional_embeddings
        }
        self.embeddings_generated.emit(final_embeddings_for_requested_files)
        logger.info(
            f"Finished. Emitted {len(final_embeddings_for_requested_files)} embeddings."
        )

        if self._is_running:  # Only cluster if not stopped
            # Build orientation map for orientation-aware clustering
            logger.info("Building orientation map for %d files...", len(file_paths))
            orientation_map = build_orientation_map(file_paths)
            self.cluster_embeddings(
                final_embeddings_for_requested_files,
                orientation_map,
                final_regional_embeddings_for_requested_files,
            )
        else:
            logger.info("Skipping clustering as stop was requested.")
            self.clustering_complete.emit({})  # Emit empty if stopped before clustering

    def _build_regional_distance_matrix(
        self,
        embeddings: dict[str, list[float]],
        regional_embeddings: dict[str, list[list[float]]],
        subset_paths: list[str],
    ) -> np.ndarray:
        """Build a distance matrix using the best matching large region pair."""
        region_sets: list[np.ndarray] = []
        for path in subset_paths:
            region_vectors = regional_embeddings.get(path)
            if region_vectors:
                region_matrix = np.asarray(region_vectors, dtype=np.float32)
            else:
                region_matrix = np.asarray([embeddings[path]], dtype=np.float32)
            if region_matrix.ndim != 2 or region_matrix.shape[0] == 0:
                region_matrix = np.asarray([embeddings[path]], dtype=np.float32)
            region_sets.append(l2_normalize_rows(region_matrix))

        count = len(subset_paths)
        distances = np.zeros((count, count), dtype=np.float32)
        for i in range(count):
            for j in range(i + 1, count):
                similarity = float(np.max(region_sets[i] @ region_sets[j].T))
                distance = max(0.0, min(2.0, 1.0 - similarity))
                distances[i, j] = distance
                distances[j, i] = distance
        return distances

    def _run_dbscan_on_subset(
        self,
        embeddings: dict[str, list[float]],
        subset_paths: list[str],
        start_cluster_id: int,
        regional_embeddings: dict[str, list[list[float]]] | None = None,
    ) -> tuple[dict[str, int], int]:
        """
        Run DBSCAN on a subset of embeddings.

        Args:
            embeddings: Full embeddings dictionary.
            subset_paths: Paths to cluster.
            start_cluster_id: Starting cluster ID for this subset.

        Returns:
            Tuple of (path_to_cluster_id dict, next_available_cluster_id).
        """
        if not subset_paths:
            return {}, start_cluster_id

        subset_embeddings = [embeddings[path] for path in subset_paths]
        embedding_matrix = np.array(subset_embeddings, dtype=np.float32)
        embedding_matrix = l2_normalize_rows(embedding_matrix)

        if not embedding_matrix.flags["C_CONTIGUOUS"]:
            embedding_matrix = np.ascontiguousarray(embedding_matrix)

        base_eps = get_similarity_clustering_eps()
        if regional_embeddings:
            distance_matrix = self._build_regional_distance_matrix(
                embeddings, regional_embeddings, subset_paths
            )
            adaptive_eps = base_eps
            dbscan = DBSCAN(
                eps=adaptive_eps,
                min_samples=DBSCAN_MIN_SAMPLES,
                metric="precomputed",
            )
            dbscan_labels = dbscan.fit_predict(distance_matrix)
        else:
            adaptive_eps = adaptive_dbscan_eps(
                embedding_matrix, base_eps, DBSCAN_MIN_SAMPLES
            )
            dbscan = DBSCAN(
                eps=adaptive_eps, min_samples=DBSCAN_MIN_SAMPLES, metric="cosine"
            )
            dbscan_labels = dbscan.fit_predict(embedding_matrix)

        # Map DBSCAN labels to our cluster IDs
        label_map: dict[int, int] = {}
        current_id = start_cluster_id
        result: dict[str, int] = {}

        # First pass: map actual clusters (non-noise)
        for original_label in sorted(set(dbscan_labels)):
            if original_label != -1:
                label_map[original_label] = current_id
                current_id += 1

        # Second pass: assign cluster IDs to paths
        next_noise_id = current_id
        for i, path in enumerate(subset_paths):
            original_label = dbscan_labels[i]
            if original_label == -1:
                # Noise point gets unique ID
                result[path] = next_noise_id
                next_noise_id += 1
            else:
                result[path] = label_map[original_label]

        return result, next_noise_id

    def cluster_embeddings(
        self,
        embeddings: dict[str, list[float]],
        orientation_map: dict[str, Orientation] | None = None,
        regional_embeddings: dict[str, list[list[float]]] | None = None,
    ):
        if not self._is_running:
            logger.info("Clustering skipped (stop requested).")
            self.clustering_complete.emit({})
            return

        if not embeddings:
            logger.warning("No embeddings provided for clustering.")
            self.clustering_complete.emit({})
            return

        filepaths = list(embeddings.keys())
        num_samples = len(filepaths)

        # Partition by orientation if orientation_map is provided
        if orientation_map:
            portrait_paths = [
                p for p in filepaths if orientation_map.get(p) == "portrait"
            ]
            # Landscape and square are grouped together
            landscape_paths = [
                p for p in filepaths if orientation_map.get(p) != "portrait"
            ]

            logger.info(
                "Orientation-aware clustering: %d portrait, %d landscape/square images.",
                len(portrait_paths),
                len(landscape_paths),
            )

            path_to_cluster: dict[str, int] = {}

            try:
                # Cluster portrait images first (IDs start at 1)
                if portrait_paths:
                    portrait_clusters, next_id = self._run_dbscan_on_subset(
                        embeddings,
                        portrait_paths,
                        start_cluster_id=1,
                        regional_embeddings=regional_embeddings,
                    )
                    path_to_cluster.update(portrait_clusters)
                    logger.info(
                        "Portrait clustering: %d clusters/groups formed.",
                        len(set(portrait_clusters.values())),
                    )
                else:
                    next_id = 1

                # Cluster landscape images (IDs continue from portrait)
                if landscape_paths:
                    landscape_clusters, _ = self._run_dbscan_on_subset(
                        embeddings,
                        landscape_paths,
                        start_cluster_id=next_id,
                        regional_embeddings=regional_embeddings,
                    )
                    path_to_cluster.update(landscape_clusters)
                    logger.info(
                        "Landscape clustering: %d clusters/groups formed.",
                        len(set(landscape_clusters.values())),
                    )

                # Convert to labels array in original order
                labels = np.array(
                    [path_to_cluster[path] for path in filepaths], dtype=int
                )

            except Exception as e_dbscan:
                error_msg = (
                    f"Error during orientation-aware DBSCAN clustering: {e_dbscan}"
                )
                logger.error(error_msg, exc_info=True)
                self.error.emit(error_msg)
                labels = np.zeros(num_samples, dtype=int)
                logger.warning(
                    "Clustering failed. Assigning all items to a single group."
                )
        else:
            # Original clustering logic without orientation awareness
            embedding_matrix = np.array(list(embeddings.values()), dtype=np.float32)
            embedding_matrix = l2_normalize_rows(embedding_matrix)

            labels = None
            base_eps = get_similarity_clustering_eps()
            regional_distance_matrix = (
                self._build_regional_distance_matrix(
                    embeddings, regional_embeddings, filepaths
                )
                if regional_embeddings
                else None
            )
            adaptive_eps = (
                base_eps
                if regional_distance_matrix is not None
                else adaptive_dbscan_eps(embedding_matrix, base_eps, DBSCAN_MIN_SAMPLES)
            )
            try:
                logger.info(
                    "Running DBSCAN clustering on %d embeddings (eps=%.4f, min_samples=%d).",
                    num_samples,
                    adaptive_eps,
                    DBSCAN_MIN_SAMPLES,
                )
                if not embedding_matrix.flags["C_CONTIGUOUS"]:
                    embedding_matrix = np.ascontiguousarray(embedding_matrix)

                if regional_distance_matrix is not None:
                    dbscan = DBSCAN(
                        eps=adaptive_eps,
                        min_samples=DBSCAN_MIN_SAMPLES,
                        metric="precomputed",
                    )
                    dbscan_labels = dbscan.fit_predict(regional_distance_matrix)
                else:
                    dbscan = DBSCAN(
                        eps=adaptive_eps,
                        min_samples=DBSCAN_MIN_SAMPLES,
                        metric="cosine",
                    )
                    dbscan_labels = dbscan.fit_predict(embedding_matrix)

                final_labels = np.zeros_like(dbscan_labels, dtype=int)
                unique_dbscan_labels = set(dbscan_labels)

                label_map_for_actual_clusters = {}
                current_new_label_id = 1

                sorted_original_actual_cluster_indices = sorted(
                    [label for label in unique_dbscan_labels if label != -1]
                )

                for original_cluster_idx in sorted_original_actual_cluster_indices:
                    label_map_for_actual_clusters[original_cluster_idx] = (
                        current_new_label_id
                    )
                    current_new_label_id += 1

                next_available_group_id_for_noise = current_new_label_id
                noise_points_count = 0

                for i, original_label in enumerate(dbscan_labels):
                    if original_label == -1:
                        final_labels[i] = next_available_group_id_for_noise
                        next_available_group_id_for_noise += 1
                        noise_points_count += 1
                    else:
                        final_labels[i] = label_map_for_actual_clusters[original_label]

                labels = final_labels

                num_actual_clusters_formed = len(label_map_for_actual_clusters)
                log_message_parts = []
                if num_actual_clusters_formed > 0:
                    log_message_parts.append(
                        f"{num_actual_clusters_formed} actual cluster(s)"
                    )
                if noise_points_count > 0:
                    log_message_parts.append(
                        f"{noise_points_count} image(s) assigned to individual groups"
                    )

                clustering_summary_log_message = (
                    ", ".join(log_message_parts)
                    if log_message_parts
                    else "No distinct groups formed"
                )
                logger.info(
                    f"DBSCAN clustering processed. {clustering_summary_log_message}."
                )

            except Exception as e_dbscan:
                error_msg = f"Error during DBSCAN clustering: {e_dbscan}"
                logger.error(error_msg, exc_info=True)
                self.error.emit(error_msg)
                labels = np.zeros(num_samples, dtype=int)
                logger.warning(
                    "DBSCAN clustering failed. Assigning all items to a single group."
                )

        # Group filepaths by their assigned label
        grouped_filepaths: dict[int, list[str]] = {}
        for i, label in enumerate(labels):
            grouped_filepaths.setdefault(int(label), []).append(filepaths[i])

        group_similarities: dict[
            int, str
        ] = {}  # Stores group_id -> average_similarity_percentage

        for group_id, paths_in_group in grouped_filepaths.items():
            if len(paths_in_group) > 1:
                # Get embeddings for this group
                group_embeds = np.array(
                    [embeddings[path] for path in paths_in_group], dtype=np.float32
                )

                # Lazy import for cosine_similarity to defer sklearn.metrics loading
                try:
                    from sklearn.metrics.pairwise import cosine_similarity
                except ImportError as e:
                    logger.error(f"Failed to import cosine_similarity: {e}")
                    pairwise_similarities = np.array(
                        [[1.0]]
                    )  # Fallback for single item or error
                else:
                    # Calculate pairwise cosine similarities
                    pairwise_similarities = cosine_similarity(group_embeds)

                # We want the average of the upper triangle (excluding diagonal which is 1.0)
                upper_triangle_indices = np.triu_indices(
                    pairwise_similarities.shape[0], k=1
                )

                if (
                    upper_triangle_indices[0].size > 0
                ):  # Ensure there are pairs to compare
                    avg_similarity = np.mean(
                        pairwise_similarities[upper_triangle_indices]
                    )
                    group_similarities[group_id] = str(
                        round(float(avg_similarity * 100), 2)
                    )
                else:
                    group_similarities[group_id] = (
                        "100"  # Single image in group, considered 100% similar to itself
                    )
            else:
                group_similarities[group_id] = (
                    "100"  # Single image in group, considered 100% similar to itself
                )

        # Prepare the final results dictionary with similarity percentage
        results = {
            filepaths[i]: f"{labels[i]} - {group_similarities.get(labels[i], '0.0')}%"
            for i in range(num_samples)
        }

        self.clustering_complete.emit(results)

    @staticmethod
    def clear_embedding_cache():
        app_cache_root = get_app_cache_root()
        embedding_cache_dir = os.path.join(app_cache_root, "embeddings")
        logger.info(f"Clearing embedding cache directory: {embedding_cache_dir}")
        if not os.path.isdir(embedding_cache_dir):
            logger.warning(
                "Embedding cache directory not found: %s", embedding_cache_dir
            )
            return

        success, message = ImageFileOperations.clear_directory_contents(
            embedding_cache_dir
        )
        if success:
            logger.info("Embedding cache cleared.")
        else:
            logger.error(message)

import os
import time
import pickle # For caching embeddings
import logging # Added for startup logging
from typing import List, Dict, Tuple, Optional, Union, TYPE_CHECKING
from PyQt6.QtCore import QObject, pyqtSignal, QThread
# from sentence_transformers import SentenceTransformer # Moved to _load_model
from PIL import Image # Explicitly import for type hinting if needed
from src.core.image_pipeline import ImagePipeline # Import ImagePipeline
from sklearn.cluster import DBSCAN # Import DBSCAN
import numpy as np # Import numpy for array manipulation
from sklearn.metrics.pairwise import cosine_similarity # Import for similarity calculation
# import torch # Import torch for CUDA check - No longer needed here for constant
from .app_settings import DEFAULT_CLIP_MODEL, is_pytorch_cuda_available # Import from app_settings

if TYPE_CHECKING:
    from sentence_transformers import SentenceTransformer as _SentenceTransformerType # Use an alias for type hint

# Check PyTorch CUDA availability for SentenceTransformer
_module_init_start_time = time.time()
logging.info("src.core.similarity_engine module - Initializing...")
# PYTORCH_CUDA_AVAILABLE logic removed, will use is_pytorch_cuda_available() from app_settings
logging.info(f"src.core.similarity_engine module - Initialization (CUDA check deferred to app_settings): {time.time() - _module_init_start_time:.4f}s")

# Define constants for model and cache
# DEFAULT_CLIP_MODEL = 'clip-ViT-B-32' # Moved to app_settings
EMBEDDING_CACHE_DIR = os.path.join(os.path.expanduser('~'), '.cache', 'phototagger_embeddings')
os.makedirs(EMBEDDING_CACHE_DIR, exist_ok=True)

# DBSCAN parameters (can be tuned)
DBSCAN_EPS = 0.05  # For cosine metric: 1 - cosine_similarity. Smaller eps = higher similarity.
DBSCAN_MIN_SAMPLES = 2 # Minimum number of images to form a dense region (cluster).

class SimilarityEngine(QObject):
    """
    Handles image feature extraction (embeddings) using CLIP models
    and potentially clustering similar images. Designed for background execution.
    """
    progress_update = pyqtSignal(int, str)
    embeddings_generated = pyqtSignal(dict)
    clustering_complete = pyqtSignal(dict)
    error = pyqtSignal(str)

    def __init__(self, model_name: str = DEFAULT_CLIP_MODEL, parent=None):
        super().__init__(parent)
        init_start_time = time.perf_counter()
        logging.info("SimilarityEngine.__init__ - Start")
        self.model_name = model_name
        self.model: Optional["_SentenceTransformerType"] = None # Use string literal with alias
        self._is_running = False
        self._cache_filename = f"embeddings_{self.model_name.replace('/', '_')}.pkl"
        self._cache_path = os.path.join(EMBEDDING_CACHE_DIR, self._cache_filename)
        
        ip_instantiation_start_time = time.perf_counter()
        self.image_pipeline = ImagePipeline() # Instantiate ImagePipeline
        logging.info(f"SimilarityEngine.__init__ - ImagePipeline instantiated: {time.perf_counter() - ip_instantiation_start_time:.4f}s")
        logging.info(f"SimilarityEngine.__init__ - End (Total: {time.perf_counter() - init_start_time:.4f}s)")

    def stop(self):
        logging.info("Stop requested.")
        self._is_running = False

    def _load_model(self):
        if self.model is None:
            try:
                from sentence_transformers import SentenceTransformer # Local import
                model_load_start_time = time.perf_counter()
                logging.info(f"SimilarityEngine._load_model - Loading model: {self.model_name}...")
                logging.info(f"Loading model: {self.model_name}...")
                self.progress_update.emit(0, f"Loading model: {self.model_name}...")
                
                # Let SentenceTransformer auto-select device based on PyTorch's CUDA availability
                model_device = 'cuda' if is_pytorch_cuda_available() else 'cpu' # Use imported function
                logging.info(f"SimilarityEngine._load_model - Attempting to load model on device: {model_device}")
                logging.info(f"Attempting to load model on device: {model_device}")
                self.model = SentenceTransformer(self.model_name, device=model_device)
                
                # Log actual device model is on
                actual_device_str = "Unknown"
                if hasattr(self.model, '_target_device') and self.model._target_device is not None: # Newer sentence-transformers
                    actual_device_str = str(self.model._target_device)
                elif hasattr(self.model, 'device') and self.model.device is not None: # Older sentence-transformers
                     actual_device_str = str(self.model.device)
                logging.info(f"SimilarityEngine._load_model - Model loaded. Actual device: {actual_device_str}")
                logging.info(f"Model loaded. Actual device: {actual_device_str}")

                # Attempt to set use_fast=True (this part is optional, might not apply to all CLIP models)
                if hasattr(self.model, 'tokenizer') and \
                   hasattr(self.model.tokenizer, 'image_processor') and \
                   hasattr(self.model.tokenizer.image_processor, 'use_fast'):
                    logging.info("SimilarityEngine._load_model - Attempting to set image_processor.use_fast = True")
                    # logging.debug("Attempting to set image_processor.use_fast = True")
                    self.model.tokenizer.image_processor.use_fast = True
                
                self.progress_update.emit(0, "Model loaded.")
                logging.info(f"SimilarityEngine._load_model - Model loading complete: {time.perf_counter() - model_load_start_time:.4f}s")
            except Exception as e:
                error_msg = f"Error loading SentenceTransformer model '{self.model_name}': {e}"
                logging.error(f"SimilarityEngine._load_model - {error_msg}")
                logging.error(error_msg)
                self.error.emit(error_msg)
                self.model = None
        return self.model is not None

    def _load_cached_embeddings(self) -> Dict[str, List[float]]:
        if os.path.exists(self._cache_path):
            try:
                cache_load_start_time = time.perf_counter()
                logging.info(f"SimilarityEngine._load_cached_embeddings - Loading from: {self._cache_path}")
                logging.info(f"Loading embeddings cache from: {self._cache_path}")
                with open(self._cache_path, 'rb') as f:
                    cache_data = pickle.load(f)
                    logging.info(f"SimilarityEngine._load_cached_embeddings - Loaded {len(cache_data)} embeddings from cache in {time.perf_counter() - cache_load_start_time:.4f}s")
                    logging.info(f"Loaded {len(cache_data)} embeddings from cache.")
                    return cache_data
            except Exception as e:
                logging.warning(f"SimilarityEngine._load_cached_embeddings - Error loading cache file '{self._cache_path}': {e}. Starting fresh.")
                logging.warning(f"Error loading cache file '{self._cache_path}': {e}. Starting fresh.")
        return {}

    def _save_embeddings_to_cache(self, embeddings: Dict[str, List[float]]):
        try:
            cache_save_start_time = time.perf_counter()
            logging.info(f"SimilarityEngine._save_embeddings_to_cache - Saving {len(embeddings)} embeddings to: {self._cache_path}")
            logging.info(f"Saving {len(embeddings)} embeddings to cache: {self._cache_path}")
            with open(self._cache_path, 'wb') as f:
                pickle.dump(embeddings, f)
            logging.info(f"SimilarityEngine._save_embeddings_to_cache - Embeddings saved in {time.perf_counter() - cache_save_start_time:.4f}s")
            # logging.debug("Embeddings saved.")
        except Exception as e:
            logging.error(f"SimilarityEngine._save_embeddings_to_cache - Error saving cache file '{self._cache_path}': {e}")
            logging.error(f"Error saving cache file '{self._cache_path}': {e}")
            self.error.emit(f"Could not save embeddings cache: {e}")

    def generate_embeddings_for_files(self, file_paths: List[str], apply_auto_edits: bool = False):
        self._is_running = True
        logging.info(f"Starting embedding generation for {len(file_paths)} files. Apply auto edits: {apply_auto_edits}")
        
        if not self._load_model():
            # self.finished.emit() # QObject has no 'finished' signal by default, use appropriate signal if defined
            self.error.emit("Model could not be loaded. Cannot generate embeddings.")
            return

        cached_embeddings = self._load_cached_embeddings()
        all_embeddings = cached_embeddings.copy()
        files_to_process = [path for path in file_paths if path not in all_embeddings and self._is_running]

        total_to_process = len(files_to_process)
        logging.info(f"Found {len(all_embeddings)} cached. Processing {total_to_process} new files.")
        self.progress_update.emit(0, f"Processing {total_to_process} new images...")

        processed_count = 0
        new_embeddings = {}
        batch_size = 16

        for i in range(0, total_to_process, batch_size):
            if not self._is_running:
                logging.info("Embedding generation stopped.")
                break
            
            batch_paths = files_to_process[i:i+batch_size]
            batch_images = []
            valid_paths_in_batch = []

            for path in batch_paths:
                # Use ImagePipeline to get a suitable PIL image for processing
                logging.debug(f"Attempting to get PIL image for: {path}, use_preloaded_preview_if_available=True, apply_auto_edits={apply_auto_edits}")
                img = self.image_pipeline.get_pil_image_for_processing(
                    path,
                    apply_auto_edits=apply_auto_edits,
                    target_mode="RGB",
                    use_preloaded_preview_if_available=True # Prioritize cached previews
                )
                if img:
                    logging.debug(f"Successfully got PIL image for: {path}. Dimensions: {img.size}")
                    batch_images.append(img)
                    valid_paths_in_batch.append(path)
                else:
                    logging.warning(f"Skipping {path}, PIL image could not be obtained/loaded (apply_auto_edits: {apply_auto_edits}).")
                    # Ensure skipped files are still counted towards progress if they were in files_to_process
                    # This is handled later by `processed_count += len(valid_paths_in_batch)` and how total_to_process is calculated.
                    # If a file is skipped here, it won't be in valid_paths_in_batch.

            if not batch_images:
                # This means all paths in the current batch_paths failed to load a preview.
                logging.warning(f"No valid previews loaded for batch starting at index {i}. Skipping batch.")
                continue

            try:
                logging.debug(f"Encoding batch {i//batch_size + 1}/{(total_to_process + batch_size - 1)//batch_size}...")
                # SentenceTransformer will use its configured device (GPU if available)
                # Ensure model is not None for type checker
                if self.model is None:
                    raise RuntimeError("SentenceTransformer model is not loaded.")
                batch_embeds = self.model.encode(batch_images, show_progress_bar=False, convert_to_numpy=True)
                
                for path_idx, path in enumerate(valid_paths_in_batch):
                    new_embeddings[path] = batch_embeds[path_idx].tolist()

                processed_count += len(valid_paths_in_batch)
                progress = int((processed_count / total_to_process) * 100) if total_to_process > 0 else 100
                self.progress_update.emit(progress, f"Generating embeddings ({processed_count}/{total_to_process})...")

            except Exception as e:
                 logging.error(f"Error encoding batch: {e}")
                 self.error.emit(f"Error during embedding generation: {e}")
                 processed_count += len(valid_paths_in_batch) # Still count them for progress

        logging.info("Finished processing new files.")
        all_embeddings.update(new_embeddings)
        if new_embeddings:
            self._save_embeddings_to_cache(all_embeddings)

        final_embeddings_for_requested_files = {path: all_embeddings[path] for path in file_paths if path in all_embeddings}
        self.embeddings_generated.emit(final_embeddings_for_requested_files)
        logging.info(f"Emitted {len(final_embeddings_for_requested_files)} embeddings.")
        
        if self._is_running: # Only cluster if not stopped
            self.cluster_embeddings(final_embeddings_for_requested_files)
        else:
            logging.info("Skipping clustering as stop was requested.")
            self.clustering_complete.emit({}) # Emit empty if stopped before clustering

    def cluster_embeddings(self, embeddings: Dict[str, List[float]]):
        if not self._is_running:
            logging.info("Clustering skipped (stop requested).")
            self.clustering_complete.emit({})
            return

        if not embeddings:
            logging.warning("No embeddings provided for clustering.")
            self.clustering_complete.emit({})
            return

        filepaths = list(embeddings.keys())
        embedding_matrix = np.array(list(embeddings.values()), dtype=np.float32)
        num_samples, _ = embedding_matrix.shape

        labels = None
        try:
            logging.info(f"Attempting DBSCAN clustering: {num_samples} samples, eps={DBSCAN_EPS}, min_samples={DBSCAN_MIN_SAMPLES}.")
            # Ensure embedding_matrix is C-contiguous, which is expected by DBSCAN
            if not embedding_matrix.flags['C_CONTIGUOUS']:
                embedding_matrix = np.ascontiguousarray(embedding_matrix)

            dbscan = DBSCAN(eps=DBSCAN_EPS, min_samples=DBSCAN_MIN_SAMPLES, metric='cosine')
            dbscan_labels = dbscan.fit_predict(embedding_matrix)
            
            final_labels = np.zeros_like(dbscan_labels, dtype=int) # Array to store new group IDs
            unique_dbscan_labels = set(dbscan_labels)

            label_map_for_actual_clusters = {}
            current_new_label_id = 1 # Start actual clusters from ID 1
            
            # Sort original non-noise cluster indices from DBSCAN to assign new IDs consistently
            sorted_original_actual_cluster_indices = sorted([l for l in unique_dbscan_labels if l != -1])

            for original_cluster_idx in sorted_original_actual_cluster_indices:
                label_map_for_actual_clusters[original_cluster_idx] = current_new_label_id
                current_new_label_id += 1
            
            # next_available_group_id_for_noise will start from where actual cluster IDs left off
            next_available_group_id_for_noise = current_new_label_id
            
            noise_points_count = 0
            
            for i, original_label in enumerate(dbscan_labels):
                if original_label == -1: # This is a noise point
                    final_labels[i] = next_available_group_id_for_noise
                    next_available_group_id_for_noise += 1 # Each noise point gets a new, unique ID
                    noise_points_count += 1
                else: # This point belongs to an actual cluster
                    final_labels[i] = label_map_for_actual_clusters[original_label]
            
            labels = final_labels # Use these new labels for results
            
            num_actual_clusters_formed = len(label_map_for_actual_clusters)
            
            log_message_parts = []
            if num_actual_clusters_formed > 0:
                log_message_parts.append(f"{num_actual_clusters_formed} actual cluster(s)")
            if noise_points_count > 0:
                log_message_parts.append(f"{noise_points_count} image(s) assigned to individual groups")
            
            clustering_summary_log_message = ", ".join(log_message_parts) if log_message_parts else "No distinct groups formed"
            logging.info(f"DBSCAN clustering processed. {clustering_summary_log_message}.")

        except Exception as e_dbscan:
            error_msg = f"Error during DBSCAN clustering: {e_dbscan}"
            logging.error(error_msg)
            self.error.emit(error_msg)
            # Minimal fallback: assign all to cluster 0 if DBSCAN fails
            labels = np.zeros(num_samples, dtype=int)
            logging.warning("DBSCAN failed. Assigning all to cluster 0.")

        # Group filepaths by their assigned label
        grouped_filepaths: Dict[int, List[str]] = {}
        for i, label in enumerate(labels):
            grouped_filepaths.setdefault(int(label), []).append(filepaths[i])

        group_similarities: Dict[int, str] = {} # Stores group_id -> average_similarity_percentage

        for group_id, paths_in_group in grouped_filepaths.items():
            if len(paths_in_group) > 1:
                # Get embeddings for this group
                group_embeds = np.array([embeddings[path] for path in paths_in_group], dtype=np.float32)

                # Calculate pairwise cosine similarities
                pairwise_similarities = cosine_similarity(group_embeds)

                # We want the average of the upper triangle (excluding diagonal which is 1.0)
                upper_triangle_indices = np.triu_indices(pairwise_similarities.shape[0], k=1)

                if upper_triangle_indices[0].size > 0: # Ensure there are pairs to compare
                    avg_similarity = np.mean(pairwise_similarities[upper_triangle_indices])
                    group_similarities[group_id] = str(round(float(avg_similarity * 100), 2))
                else:
                    group_similarities[group_id] = "" # Single image in group, considered 100% similar to itself
            else:
                group_similarities[group_id] = "" # Single image in group, considered 100% similar to itself

        # Prepare the final results dictionary with similarity percentage
        results = {filepaths[i]: f"{labels[i]} - {group_similarities.get(labels[i], '0.0')}%" for i in range(num_samples)}

        self.clustering_complete.emit(results)


    @staticmethod
    def clear_embedding_cache():
        logging.info(f"Clearing embedding cache directory: {EMBEDDING_CACHE_DIR}")
        try:
            if os.path.exists(EMBEDDING_CACHE_DIR):
                import shutil
                for item in os.listdir(EMBEDDING_CACHE_DIR):
                    item_path = os.path.join(EMBEDDING_CACHE_DIR, item)
                    try:
                        if os.path.isfile(item_path) or os.path.islink(item_path):
                            os.unlink(item_path)
                        elif os.path.isdir(item_path):
                            shutil.rmtree(item_path)
                    except Exception as e:
                        logging.error(f"Failed to delete {item_path}. Reason: {e}")
                logging.info(f"Embedding cache directory cleared: {EMBEDDING_CACHE_DIR}")
            else:
                logging.warning(f"Embedding cache directory not found: {EMBEDDING_CACHE_DIR}")
        except Exception as e:
            error_msg = f"Error clearing embedding cache directory '{EMBEDDING_CACHE_DIR}': {e}"
            logging.error(error_msg)

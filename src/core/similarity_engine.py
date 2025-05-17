import os
import time
import pickle # For caching embeddings
import logging # Added for startup logging
from typing import List, Dict, Tuple, Optional, TYPE_CHECKING
from PyQt6.QtCore import QObject, pyqtSignal, QThread
# from sentence_transformers import SentenceTransformer # Moved to _load_model
from PIL import Image # Explicitly import for type hinting if needed
from src.core.image_pipeline import ImagePipeline # Import ImagePipeline
from sklearn.cluster import DBSCAN # Import DBSCAN
import numpy as np # Import numpy for array manipulation
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
DBSCAN_EPS = 0.1  # For cosine metric: 1 - cosine_similarity. Smaller eps = higher similarity.
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
        print("[SimilarityEngine] Stop requested.")
        self._is_running = False

    def _load_model(self):
        if self.model is None:
            try:
                from sentence_transformers import SentenceTransformer # Local import
                model_load_start_time = time.perf_counter()
                logging.info(f"SimilarityEngine._load_model - Loading model: {self.model_name}...")
                # print(f"[SimilarityEngine] Loading model: {self.model_name}...") # Replaced by logging
                self.progress_update.emit(0, f"Loading model: {self.model_name}...")
                
                # Let SentenceTransformer auto-select device based on PyTorch's CUDA availability
                model_device = 'cuda' if is_pytorch_cuda_available() else 'cpu' # Use imported function
                logging.info(f"SimilarityEngine._load_model - Attempting to load model on device: {model_device}")
                # print(f"[SimilarityEngine] Attempting to load model on device: {model_device}") # Replaced by logging
                self.model = SentenceTransformer(self.model_name, device=model_device)
                
                # Log actual device model is on
                actual_device_str = "Unknown"
                if hasattr(self.model, '_target_device') and self.model._target_device is not None: # Newer sentence-transformers
                    actual_device_str = str(self.model._target_device)
                elif hasattr(self.model, 'device') and self.model.device is not None: # Older sentence-transformers
                     actual_device_str = str(self.model.device)
                logging.info(f"SimilarityEngine._load_model - Model loaded. Actual device: {actual_device_str}")
                # print(f"[SimilarityEngine] Model loaded. Actual device: {actual_device_str}") # Replaced by logging

                # Attempt to set use_fast=True (this part is optional, might not apply to all CLIP models)
                if hasattr(self.model, 'tokenizer') and \
                   hasattr(self.model.tokenizer, 'image_processor') and \
                   hasattr(self.model.tokenizer.image_processor, 'use_fast'):
                    logging.info("SimilarityEngine._load_model - Attempting to set image_processor.use_fast = True")
                    # print("[SimilarityEngine] Attempting to set image_processor.use_fast = True") # Replaced by logging
                    self.model.tokenizer.image_processor.use_fast = True
                
                self.progress_update.emit(0, "Model loaded.")
                logging.info(f"SimilarityEngine._load_model - Model loading complete: {time.perf_counter() - model_load_start_time:.4f}s")
            except Exception as e:
                error_msg = f"Error loading SentenceTransformer model '{self.model_name}': {e}"
                logging.error(f"SimilarityEngine._load_model - {error_msg}")
                # print(f"[SimilarityEngine] {error_msg}") # Replaced by logging
                self.error.emit(error_msg)
                self.model = None
        return self.model is not None

    def _load_cached_embeddings(self) -> Dict[str, List[float]]:
        if os.path.exists(self._cache_path):
            try:
                cache_load_start_time = time.perf_counter()
                logging.info(f"SimilarityEngine._load_cached_embeddings - Loading from: {self._cache_path}")
                # print(f"[SimilarityEngine] Loading embeddings cache from: {self._cache_path}") # Replaced by logging
                with open(self._cache_path, 'rb') as f:
                    cache_data = pickle.load(f)
                    logging.info(f"SimilarityEngine._load_cached_embeddings - Loaded {len(cache_data)} embeddings from cache in {time.perf_counter() - cache_load_start_time:.4f}s")
                    # print(f"[SimilarityEngine] Loaded {len(cache_data)} embeddings from cache.") # Replaced by logging
                    return cache_data
            except Exception as e:
                logging.warning(f"SimilarityEngine._load_cached_embeddings - Error loading cache file '{self._cache_path}': {e}. Starting fresh.")
                # print(f"[SimilarityEngine] Error loading cache file '{self._cache_path}': {e}. Starting fresh.") # Replaced by logging
        return {}

    def _save_embeddings_to_cache(self, embeddings: Dict[str, List[float]]):
        try:
            cache_save_start_time = time.perf_counter()
            logging.info(f"SimilarityEngine._save_embeddings_to_cache - Saving {len(embeddings)} embeddings to: {self._cache_path}")
            # print(f"[SimilarityEngine] Saving {len(embeddings)} embeddings to cache: {self._cache_path}") # Replaced by logging
            with open(self._cache_path, 'wb') as f:
                pickle.dump(embeddings, f)
            logging.info(f"SimilarityEngine._save_embeddings_to_cache - Embeddings saved in {time.perf_counter() - cache_save_start_time:.4f}s")
            # print("[SimilarityEngine] Embeddings saved.") # Replaced by logging
        except Exception as e:
            logging.error(f"SimilarityEngine._save_embeddings_to_cache - Error saving cache file '{self._cache_path}': {e}")
            # print(f"[SimilarityEngine] Error saving cache file '{self._cache_path}': {e}") # Replaced by logging
            self.error.emit(f"Could not save embeddings cache: {e}")

    def generate_embeddings_for_files(self, file_paths: List[str], apply_auto_edits: bool = False):
        self._is_running = True
        print(f"[SimilarityEngine] Starting embedding generation for {len(file_paths)} files. Apply auto edits: {apply_auto_edits}")
        
        if not self._load_model():
            # self.finished.emit() # QObject has no 'finished' signal by default, use appropriate signal if defined
            self.error.emit("Model could not be loaded. Cannot generate embeddings.")
            return

        cached_embeddings = self._load_cached_embeddings()
        all_embeddings = cached_embeddings.copy()
        files_to_process = [path for path in file_paths if path not in all_embeddings and self._is_running]

        total_to_process = len(files_to_process)
        print(f"[SimilarityEngine] Found {len(all_embeddings)} cached. Processing {total_to_process} new files.")
        self.progress_update.emit(0, f"Processing {total_to_process} new images...")

        processed_count = 0
        new_embeddings = {}
        batch_size = 16

        for i in range(0, total_to_process, batch_size):
            if not self._is_running:
                print("[SimilarityEngine] Embedding generation stopped.")
                break
            
            batch_paths = files_to_process[i:i+batch_size]
            batch_images = []
            valid_paths_in_batch = []

            for path in batch_paths:
                # Use ImagePipeline to get a suitable PIL image for processing
                print(f"[SimilarityEngine] Attempting to get PIL image for: {path}, use_preloaded_preview_if_available=True, apply_auto_edits={apply_auto_edits}")
                img = self.image_pipeline.get_pil_image_for_processing(
                    path,
                    apply_auto_edits=apply_auto_edits,
                    target_mode="RGB",
                    use_preloaded_preview_if_available=True # Prioritize cached previews
                )
                if img:
                    print(f"[SimilarityEngine] Successfully got PIL image for: {path}. Dimensions: {img.size}")
                    batch_images.append(img)
                    valid_paths_in_batch.append(path)
                else:
                    print(f"[SimilarityEngine] Warning: Skipping {path}, PIL image could not be obtained/loaded (apply_auto_edits: {apply_auto_edits}).")
                    # Ensure skipped files are still counted towards progress if they were in files_to_process
                    # This is handled later by `processed_count += len(valid_paths_in_batch)` and how total_to_process is calculated.
                    # If a file is skipped here, it won't be in valid_paths_in_batch.

            if not batch_images:
                # This means all paths in the current batch_paths failed to load a preview.
                # We need to ensure progress reflects these attempts.
                # `processed_count` is incremented by `len(valid_paths_in_batch)` later.
                # If all fail, `valid_paths_in_batch` is empty.
                # To correctly update progress for files that couldn't load previews:
                # We can consider them "processed" in terms of attempt.
                # However, the current logic for `processed_count` only adds `len(valid_paths_in_batch)`.
                # Let's adjust how progress is reported or how skipped files are handled for total count.
                # For now, the existing progress logic based on successful loads will be maintained.
                # If a file is skipped, it simply won't contribute to `new_embeddings`.
                # The `total_to_process` is based on files *not in cache*. If a preview fails, it's effectively skipped.
                # The progress update `processed_count / total_to_process` will reflect successfully processed previews.
                # This seems acceptable as embeddings can't be generated without the image.
                print(f"[SimilarityEngine] Warning: No valid previews loaded for batch starting at index {i}. Skipping batch.")
                continue

            try:
                print(f"[SimilarityEngine] Encoding batch {i//batch_size + 1}/{(total_to_process + batch_size - 1)//batch_size}...")
                # SentenceTransformer will use its configured device (GPU if available)
                batch_embeds = self.model.encode(batch_images, show_progress_bar=False, convert_to_numpy=True)
                
                for path_idx, path in enumerate(valid_paths_in_batch):
                    new_embeddings[path] = batch_embeds[path_idx].tolist()

                processed_count += len(valid_paths_in_batch)
                progress = int((processed_count / total_to_process) * 100) if total_to_process > 0 else 100
                self.progress_update.emit(progress, f"Generating embeddings ({processed_count}/{total_to_process})...")

            except Exception as e:
                 print(f"[SimilarityEngine] Error encoding batch: {e}")
                 self.error.emit(f"Error during embedding generation: {e}")
                 processed_count += len(valid_paths_in_batch) # Still count them for progress

        print("[SimilarityEngine] Finished processing new files.")
        all_embeddings.update(new_embeddings)
        if new_embeddings:
            self._save_embeddings_to_cache(all_embeddings)

        final_embeddings_for_requested_files = {path: all_embeddings[path] for path in file_paths if path in all_embeddings}
        self.embeddings_generated.emit(final_embeddings_for_requested_files)
        print(f"[SimilarityEngine] Emitted {len(final_embeddings_for_requested_files)} embeddings.")
        
        if self._is_running: # Only cluster if not stopped
            self.cluster_embeddings(final_embeddings_for_requested_files)
        else:
            print("[SimilarityEngine] Skipping clustering as stop was requested.")
            self.clustering_complete.emit({}) # Emit empty if stopped before clustering

    def cluster_embeddings(self, embeddings: Dict[str, List[float]]):
        if not self._is_running:
            print("[SimilarityEngine] Clustering skipped (stop requested).")
            self.clustering_complete.emit({})
            return

        if not embeddings:
            print("[SimilarityEngine] No embeddings provided for clustering.")
            self.clustering_complete.emit({})
            return

        filepaths = list(embeddings.keys())
        embedding_matrix = np.array(list(embeddings.values()), dtype=np.float32)
        num_samples, _ = embedding_matrix.shape

        if num_samples < 2: # DBSCAN needs at least 2 samples to potentially form a cluster
            print(f"[SimilarityEngine] Not enough samples ({num_samples}) for DBSCAN clustering. Assigning all to cluster 0.")
            results = {filepath: 0 for filepath in filepaths}
            self.clustering_complete.emit(results)
            return

        labels = None
        try:
            print(f"[SimilarityEngine] Attempting DBSCAN clustering: {num_samples} samples, eps={DBSCAN_EPS}, min_samples={DBSCAN_MIN_SAMPLES}.")
            # Ensure embedding_matrix is C-contiguous, which is expected by DBSCAN
            if not embedding_matrix.flags['C_CONTIGUOUS']:
                embedding_matrix = np.ascontiguousarray(embedding_matrix)

            dbscan = DBSCAN(eps=DBSCAN_EPS, min_samples=DBSCAN_MIN_SAMPLES, metric='cosine')
            dbscan_labels = dbscan.fit_predict(embedding_matrix)
            
            # Map DBSCAN's -1 (noise) labels to 0, and shift other labels up by 1
            # to maintain non-negative cluster IDs and keep 0 for noise/unclustered.
            # Or, if you want to keep -1 as a distinct "noise" group that the UI might handle differently:
            # labels = dbscan_labels
            # For now, let's map noise to 0 and shift others.
            
            # Find the number of actual clusters found (excluding noise)
            unique_labels = set(dbscan_labels)
            num_discovered_clusters = len(unique_labels) - (1 if -1 in unique_labels else 0)
            
            # Create a mapping for labels: noise (-1) -> 0, cluster 0 -> 1, cluster 1 -> 2, etc.
            # This ensures cluster IDs are 0-indexed for noise, and 1-indexed for actual clusters.
            # However, the UI currently expects 0-indexed clusters.
            # Let's re-think: if -1 is noise, we can keep it as -1 or map it to a high number
            # or simply treat it as a special cluster.
            # For simplicity with current UI, let's map noise (-1) to 0, and actual clusters 0, 1, 2... to 1, 2, 3...
            # Then, when emitting, we can shift them back if UI expects 0-indexed clusters for actual groups.
            # The current UI seems to handle "Group 0", "Group 1", etc.
            # Let's make noise cluster 0, and other clusters start from 1.
            
            final_labels = np.zeros_like(dbscan_labels)
            current_new_label = 1 # Start actual clusters from label 1
            label_map = {} # To map original dbscan labels to new sequential ones
            
            # Sort original cluster labels (excluding -1) to assign new labels consistently
            sorted_original_cluster_indices = sorted([l for l in unique_labels if l != -1])

            for original_label in sorted_original_cluster_indices:
                label_map[original_label] = current_new_label
                current_new_label += 1
            
            for i, l in enumerate(dbscan_labels):
                if l == -1:
                    final_labels[i] = 0 # Noise points go to cluster 0
                else:
                    final_labels[i] = label_map[l] # Mapped actual clusters

            labels = final_labels
            print(f"[SimilarityEngine] DBSCAN clustering successful. Found {num_discovered_clusters} clusters (plus noise group 0).")

        except Exception as e_dbscan:
            error_msg = f"Error during DBSCAN clustering: {e_dbscan}"
            print(f"[SimilarityEngine] {error_msg}")
            self.error.emit(error_msg)
            # Minimal fallback: assign all to cluster 0 if DBSCAN fails
            labels = np.zeros(num_samples, dtype=int)
            print("[SimilarityEngine] DBSCAN failed. Assigning all to cluster 0.")

        results = {filepaths[i]: int(labels[i]) for i in range(num_samples)}
        self.clustering_complete.emit(results)


    @staticmethod
    def clear_embedding_cache():
        print(f"[SimilarityEngine] Clearing embedding cache directory: {EMBEDDING_CACHE_DIR}")
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
                        print(f"[SimilarityEngine] Failed to delete {item_path}. Reason: {e}")
                print(f"[SimilarityEngine] Embedding cache directory cleared: {EMBEDDING_CACHE_DIR}")
            else:
                print(f"[SimilarityEngine] Embedding cache directory not found: {EMBEDDING_CACHE_DIR}")
        except Exception as e:
            error_msg = f"Error clearing embedding cache directory '{EMBEDDING_CACHE_DIR}': {e}"
            print(f"[SimilarityEngine] {error_msg}")

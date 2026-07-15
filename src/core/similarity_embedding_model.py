import logging
import time
from dataclasses import dataclass
from collections.abc import Iterable

import numpy as np

from core.app_settings import (
    DEFAULT_SIMILARITY_EMBEDDING_MODEL,
    SUPPORTED_SIMILARITY_EMBEDDING_MODELS,
    get_huggingface_cache_dir,
    get_preferred_torch_device,
)
from core.huggingface_progress import ProgressCallback, build_hf_tqdm_class
from core.similarity_utils import l2_normalize_rows

logger = logging.getLogger(__name__)

SIMILARITY_EMBEDDING_PIPELINE_VERSION = "dinov2-cls-v1"
SIMILARITY_REGION_PIPELINE_VERSION = "dinov2-regions-v1"
SIMILARITY_ENCODE_CHUNK_SIZE = 32


class SimilarityModelNotInstalledError(RuntimeError):
    """Raised when the configured similarity model is not present locally."""


class SimilarityModelDownloadError(RuntimeError):
    """Raised when the configured similarity model cannot be downloaded."""


def sanitize_model_id(model_name: str) -> str:
    return model_name.replace("/", "_").replace(":", "_")


@dataclass(frozen=True, slots=True)
class SimilarityModelSpec:
    model_name: str
    pipeline_version: str = SIMILARITY_EMBEDDING_PIPELINE_VERSION

    @property
    def cache_key(self) -> str:
        return f"{self.pipeline_version}_{sanitize_model_id(self.model_name)}"

    @property
    def region_cache_key(self) -> str:
        return (
            f"{SIMILARITY_REGION_PIPELINE_VERSION}_{sanitize_model_id(self.model_name)}"
        )


def build_similarity_image_regions(image: object) -> list[object]:
    """Build large overlapping regions for occlusion-resistant image matching."""
    if not hasattr(image, "crop") or not hasattr(image, "size"):
        return [image]

    width, height = image.size
    if width <= 1 or height <= 1:
        return [image]

    regions = [image]

    def _box(left: float, top: float, right: float, bottom: float):
        return (
            max(0, min(width - 1, int(round(left * width)))),
            max(0, min(height - 1, int(round(top * height)))),
            max(1, min(width, int(round(right * width)))),
            max(1, min(height, int(round(bottom * height)))),
        )

    crop_boxes = [
        _box(0.10, 0.10, 0.90, 0.90),  # center
        _box(0.00, 0.00, 0.62, 1.00),  # left
        _box(0.38, 0.00, 1.00, 1.00),  # right
        _box(0.00, 0.00, 1.00, 0.62),  # top
        _box(0.00, 0.38, 1.00, 1.00),  # bottom
    ]
    for box in crop_boxes:
        left, top, right, bottom = box
        if right > left and bottom > top:
            regions.append(image.crop(box))
    return regions


def normalize_similarity_model_name(model_name: str | None) -> str:
    if model_name in SUPPORTED_SIMILARITY_EMBEDDING_MODELS:
        return str(model_name)
    return DEFAULT_SIMILARITY_EMBEDDING_MODEL


def resolve_similarity_model_snapshot(
    model_name: str | None = None,
    *,
    allow_download: bool = False,
    progress_callback: ProgressCallback | None = None,
) -> str:
    """Return a local Hugging Face snapshot path for the selected model.

    The first lookup is always local-only. Online access is used only when
    explicitly allowed by the caller after user confirmation.
    """
    resolved_model_name = normalize_similarity_model_name(model_name)
    cache_dir = get_huggingface_cache_dir()

    try:
        from huggingface_hub import snapshot_download
    except ImportError as exc:
        raise SimilarityModelDownloadError(
            "Missing dependency 'huggingface_hub'. Install PhotoSort dependencies and try again."
        ) from exc

    try:
        return snapshot_download(
            resolved_model_name,
            cache_dir=cache_dir,
            local_files_only=True,
        )
    except Exception as local_exc:
        if not allow_download:
            raise SimilarityModelNotInstalledError(
                f"Similarity model '{resolved_model_name}' is not installed locally."
            ) from local_exc

    try:
        logger.info("Downloading similarity model snapshot: %s", resolved_model_name)
        return snapshot_download(
            resolved_model_name,
            cache_dir=cache_dir,
            local_files_only=False,
            tqdm_class=build_hf_tqdm_class(
                progress_callback,
                label=f"Downloading {resolved_model_name}",
            ),
        )
    except Exception as download_exc:
        raise SimilarityModelDownloadError(
            f"Could not download similarity model '{resolved_model_name}'. Check your internet connection and try again."
        ) from download_exc


def is_similarity_model_installed(model_name: str | None = None) -> bool:
    try:
        resolve_similarity_model_snapshot(model_name, allow_download=False)
        return True
    except SimilarityModelNotInstalledError:
        return False
    except Exception:
        logger.exception("Failed to check local similarity model snapshot.")
        return False


class SimilarityEmbeddingModel:
    """DINOv2 visual embedding model for image-to-image similarity."""

    def __init__(
        self,
        model_name: str | None = None,
        *,
        allow_download: bool = False,
        progress_callback: ProgressCallback | None = None,
    ):
        self.spec = SimilarityModelSpec(normalize_similarity_model_name(model_name))
        self.allow_download = allow_download
        self.progress_callback = progress_callback
        self.snapshot_path: str | None = None
        self.processor = None
        self.model = None
        self.device = "cpu"

    @property
    def model_name(self) -> str:
        return self.spec.model_name

    @property
    def cache_key(self) -> str:
        return self.spec.cache_key

    @property
    def region_cache_key(self) -> str:
        return self.spec.region_cache_key

    def load(self) -> None:
        if self.model is not None and self.processor is not None:
            return

        try:
            from transformers import AutoImageProcessor, AutoModel
        except ImportError as exc:
            raise SimilarityModelDownloadError(
                "Missing dependency 'transformers'. Install PhotoSort dependencies and try again."
            ) from exc

        load_start = time.perf_counter()
        self.snapshot_path = resolve_similarity_model_snapshot(
            self.model_name,
            allow_download=self.allow_download,
            progress_callback=self.progress_callback,
        )
        self.device = get_preferred_torch_device()
        logger.info(
            "Loading similarity model '%s' from %s on %s",
            self.model_name,
            self.snapshot_path,
            self.device,
        )
        if self.progress_callback:
            self.progress_callback(-1, f"Loading {self.model_name} weights")

        self.processor = AutoImageProcessor.from_pretrained(
            self.snapshot_path,
            local_files_only=True,
        )
        self.model = AutoModel.from_pretrained(
            self.snapshot_path,
            local_files_only=True,
        )
        self.model.to(self.device)
        self.model.eval()
        logger.info(
            "Similarity model loaded in %.4fs", time.perf_counter() - load_start
        )

    def encode(self, images: Iterable[object]) -> np.ndarray:
        if self.model is None or self.processor is None:
            self.load()
        if self.model is None or self.processor is None:
            raise RuntimeError("Similarity embedding model is not loaded.")

        batch_images: list[object] = list(images)
        if not batch_images:
            return np.empty((0, 0), dtype=np.float32)

        return self._encode_loaded_images(batch_images)

    def encode_with_regions(
        self, images: Iterable[object]
    ) -> tuple[np.ndarray, list[np.ndarray]]:
        """Encode whole images plus large overlapping regions.

        Returns one global embedding per image and one regional embedding matrix per
        image. The first regional embedding is always the full image.
        """
        if self.model is None or self.processor is None:
            self.load()
        if self.model is None or self.processor is None:
            raise RuntimeError("Similarity embedding model is not loaded.")

        batch_images: list[object] = list(images)
        if not batch_images:
            return np.empty((0, 0), dtype=np.float32), []

        all_regions: list[object] = []
        region_counts: list[int] = []
        for image in batch_images:
            regions = build_similarity_image_regions(image)
            all_regions.extend(regions)
            region_counts.append(len(regions))

        encoded_regions = self._encode_loaded_images(all_regions)
        global_embeddings = []
        regional_embeddings: list[np.ndarray] = []
        cursor = 0
        for count in region_counts:
            image_regions = encoded_regions[cursor : cursor + count]
            cursor += count
            regional_embeddings.append(image_regions)
            global_embeddings.append(image_regions[0])

        return np.asarray(global_embeddings, dtype=np.float32), regional_embeddings

    def _encode_loaded_images(self, images: list[object]) -> np.ndarray:
        import torch

        if not images:
            return np.empty((0, 0), dtype=np.float32)

        encoded_chunks = []
        for start in range(0, len(images), SIMILARITY_ENCODE_CHUNK_SIZE):
            chunk = images[start : start + SIMILARITY_ENCODE_CHUNK_SIZE]
            inputs = self.processor(images=chunk, return_tensors="pt")
            inputs = {key: value.to(self.device) for key, value in inputs.items()}
            with torch.no_grad():
                outputs = self.model(**inputs)
                embeddings = outputs.last_hidden_state[:, 0, :]
            encoded_chunks.append(embeddings.detach().cpu().numpy().astype(np.float32))

        embeddings_np = np.vstack(encoded_chunks)
        return l2_normalize_rows(embeddings_np)

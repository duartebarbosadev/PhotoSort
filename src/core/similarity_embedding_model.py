import logging
import time
from dataclasses import dataclass
from collections.abc import Iterable
from typing import Any

import numpy as np

from core.app_settings import get_preferred_torch_device
from core.huggingface_progress import ProgressCallback
from core.model_provisioning import (
    EMBEDDING_MODEL,
    ModelDownloadError as SimilarityModelDownloadError,
    ModelNotInstalledError as SimilarityModelNotInstalledError,
    is_installed,
    resolve_snapshot,
)
from core.similarity_utils import l2_normalize_rows

logger = logging.getLogger(__name__)

SIMILARITY_EMBEDDING_PIPELINE_VERSION = "dinov2-cls-v1"
SIMILARITY_REGION_PIPELINE_VERSION = "dinov2-regions-v1"
SIMILARITY_ENCODE_CHUNK_SIZE = 32

__all__ = [
    "SIMILARITY_EMBEDDING_PIPELINE_VERSION",
    "SIMILARITY_ENCODE_CHUNK_SIZE",
    "SIMILARITY_REGION_PIPELINE_VERSION",
    "SimilarityEmbeddingModel",
    "SimilarityModelDownloadError",
    "SimilarityModelNotInstalledError",
    "SimilarityModelSpec",
    "build_similarity_image_regions",
    "is_similarity_model_installed",
    "resolve_similarity_model_snapshot",
    "sanitize_model_id",
]


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
    """Return the single embedding model id, ignoring legacy overrides."""

    if model_name is not None and model_name != EMBEDDING_MODEL.repo_id:
        logger.debug(
            "Ignoring legacy similarity model '%s'; using %s.",
            model_name,
            EMBEDDING_MODEL.repo_id,
        )
    return EMBEDDING_MODEL.repo_id


def resolve_similarity_model_snapshot(
    model_name: str | None = None,
    *,
    allow_download: bool = False,
    progress_callback: ProgressCallback | None = None,
) -> str:
    """Return a local snapshot path for the shared embedding model.

    Provisioning lives in :mod:`core.model_provisioning` so similarity, Cull and
    Pick Best resolve identical weights through one pinned, validated path.
    """

    normalize_similarity_model_name(model_name)
    return resolve_snapshot(
        EMBEDDING_MODEL,
        allow_download=allow_download,
        progress_callback=progress_callback,
    )


def is_similarity_model_installed(model_name: str | None = None) -> bool:
    normalize_similarity_model_name(model_name)
    return is_installed(EMBEDDING_MODEL)


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
        self.processor: Any | None = None
        self.model: Any | None = None
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

    def load(self, *, snapshot_path: str | None = None) -> None:
        """Load the weights, optionally from an already resolved snapshot.

        Callers that resolved the snapshot earlier (to download it with consent
        and progress) pass it in so provisioning is never repeated, while model
        construction, device placement and eval mode stay owned here.
        """

        if self.model is not None and self.processor is not None:
            return

        try:
            from transformers import AutoImageProcessor, AutoModel
        except ImportError as exc:
            raise SimilarityModelDownloadError(
                "Missing dependency 'transformers'. Install PhotoSort dependencies and try again."
            ) from exc

        load_start = time.perf_counter()
        self.snapshot_path = snapshot_path or resolve_similarity_model_snapshot(
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

        processor = AutoImageProcessor.from_pretrained(
            self.snapshot_path,
            local_files_only=True,
        )
        model = AutoModel.from_pretrained(
            self.snapshot_path,
            local_files_only=True,
        )
        model.to(self.device)
        model.eval()
        self.processor = processor
        self.model = model
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
        if self.processor is None or self.model is None:
            raise RuntimeError("Similarity embedding model is not loaded.")

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

    def encode_with_patches(
        self, images: Iterable[object]
    ) -> tuple[np.ndarray, list[np.ndarray]]:
        """Encode global CLS and normalized dense patch tokens for each image."""

        if self.model is None or self.processor is None:
            self.load()
        if self.model is None or self.processor is None:
            raise RuntimeError("Similarity embedding model is not loaded.")
        batch_images = list(images)
        if not batch_images:
            return np.empty((0, 0), dtype=np.float32), []

        import torch

        globals_out: list[np.ndarray] = []
        patches_out: list[np.ndarray] = []
        for start in range(0, len(batch_images), SIMILARITY_ENCODE_CHUNK_SIZE):
            chunk = batch_images[start : start + SIMILARITY_ENCODE_CHUNK_SIZE]
            inputs = self.processor(images=chunk, return_tensors="pt")
            inputs = {key: value.to(self.device) for key, value in inputs.items()}
            with torch.no_grad():
                hidden = self.model(**inputs).last_hidden_state
            global_chunk = hidden[:, 0, :].detach().cpu().numpy().astype(np.float32)
            patch_chunk = hidden[:, 1:, :].detach().cpu().numpy().astype(np.float32)
            globals_out.append(l2_normalize_rows(global_chunk))
            patches_out.extend(
                l2_normalize_rows(patch_values) for patch_values in patch_chunk
            )
        return np.vstack(globals_out), patches_out

"""Single-model DINOv2 adapters for fast Cull same-subject grouping."""

from __future__ import annotations

from collections.abc import Callable, Sequence
import hashlib
from pathlib import Path

import cv2
import numpy as np
from PIL import Image

from core.app_settings import get_preferred_torch_device
from core.model_provisioning import EMBEDDING_MODEL, is_installed, resolve_snapshot
from core.image_features.face_analysis import FaceAnalysisService, FaceDescriptor
from core.image_pipeline import ANALYSIS_CACHE_RESOLUTION, ImagePipeline
from core.similarity_embedding_model import (
    SimilarityEmbeddingModel,
)
from core.subject_grouping import GeometryEvidence, SubjectArtifact, SubjectEvidence


CULL_DINO_MODEL = EMBEDDING_MODEL.repo_id
CULL_SUBJECT_MODEL_IDS = (CULL_DINO_MODEL,)
CULL_SUBJECT_MODEL_REVISIONS = {CULL_DINO_MODEL: EMBEDDING_MODEL.revision}
SUBJECT_MODEL_PIPELINE_VERSION = "dinov2-small-dense-grid-v2"
DINO_EXTRACTION_BATCH_SIZE = 32
DENSE_SUBJECT_GRID_SIZE = 4
ARTIFACT_CHECKPOINT_INTERVAL = 128


def resolve_subject_model_snapshots(
    *,
    allow_download: bool,
    progress_callback: Callable[[int, str], None] | None = None,
    should_cancel: Callable[[], bool] | None = None,
) -> dict[str, str]:
    """Resolve the shared, pinned DINO checkpoint used by Cull."""

    return {
        CULL_DINO_MODEL: resolve_snapshot(
            EMBEDDING_MODEL,
            allow_download=allow_download,
            progress_callback=progress_callback,
            should_cancel=should_cancel,
        )
    }


def are_subject_models_installed() -> bool:
    return is_installed(EMBEDDING_MODEL)


def subject_model_signature(snapshots: dict[str, str]) -> str:
    digest = hashlib.sha256(SUBJECT_MODEL_PIPELINE_VERSION.encode())
    for model_id in CULL_SUBJECT_MODEL_IDS:
        snapshot = Path(snapshots[model_id])
        digest.update(model_id.encode())
        digest.update(CULL_SUBJECT_MODEL_REVISIONS[model_id].encode())
        digest.update(snapshot.name.encode())
        for config_name in ("config.json", "preprocessor_config.json"):
            config = snapshot / config_name
            if config.is_file():
                digest.update(config.read_bytes())
    return f"{SUBJECT_MODEL_PIPELINE_VERSION}:{digest.hexdigest()}"


def _normalized(values: np.ndarray) -> np.ndarray:
    array = np.asarray(values, dtype=np.float32)
    norms = np.linalg.norm(array, axis=-1, keepdims=True)
    return array / np.maximum(norms, np.finfo(np.float32).eps)


def _descriptor_tuple(values: np.ndarray) -> tuple[float, ...]:
    """Use cache-equivalent float16 precision for deterministic cold/warm runs."""

    quantized = np.asarray(values, dtype=np.float16).astype(np.float32)
    return tuple(float(item) for item in quantized)


def _dense_grid_evidence(
    patch_tokens: np.ndarray,
    *,
    grid_size: int = DENSE_SUBJECT_GRID_SIZE,
) -> tuple[SubjectEvidence, ...]:
    """Pool DINO patches into a compact spatial subject-set representation."""

    patch_count, descriptor_size = patch_tokens.shape
    side = int(round(np.sqrt(patch_count)))
    if side * side != patch_count:
        pooled = _normalized(patch_tokens.mean(axis=0, keepdims=True))[0]
        return (
            SubjectEvidence(
                descriptor=_descriptor_tuple(pooled),
                bbox=(0.0, 0.0, 1.0, 1.0),
                area_fraction=1.0,
                kind="region",
            ),
        )

    grid = patch_tokens.reshape(side, side, descriptor_size)
    edges: np.ndarray = np.linspace(0, side, min(grid_size, side) + 1, dtype=int)
    evidence: list[SubjectEvidence] = []
    for row in range(len(edges) - 1):
        for column in range(len(edges) - 1):
            top, bottom = int(edges[row]), int(edges[row + 1])
            left, right = int(edges[column]), int(edges[column + 1])
            descriptor = _normalized(
                grid[top:bottom, left:right]
                .reshape(-1, descriptor_size)
                .mean(axis=0, keepdims=True)
            )[0]
            bbox = (left / side, top / side, right / side, bottom / side)
            evidence.append(
                SubjectEvidence(
                    descriptor=_descriptor_tuple(descriptor),
                    bbox=bbox,
                    area_fraction=float(
                        (right - left) * (bottom - top) / (side * side)
                    ),
                    kind="region",
                )
            )
    return tuple(evidence)


def _aligned_face_crop(
    image: Image.Image,
    bbox: tuple[float, float, float, float],
    landmarks: Sequence[tuple[float, float]],
) -> Image.Image:
    width, height = image.size
    left, top, right, bottom = bbox
    if len(landmarks) > 263:
        first_eye = landmarks[33]
        second_eye = landmarks[263]
        angle = np.degrees(
            np.arctan2(
                (second_eye[1] - first_eye[1]) * height,
                (second_eye[0] - first_eye[0]) * width,
            )
        )
        center = (
            (first_eye[0] + second_eye[0]) * width / 2,
            (first_eye[1] + second_eye[1]) * height / 2,
        )
        image = image.rotate(
            float(angle), center=center, resample=Image.Resampling.BICUBIC
        )
    face_width = max(1.0, (right - left) * width)
    face_height = max(1.0, (bottom - top) * height)
    padding_x = face_width * 0.25
    padding_y = face_height * 0.25
    return image.crop(
        (
            max(0, int(left * width - padding_x)),
            max(0, int(top * height - padding_y)),
            min(width, int(np.ceil(right * width + padding_x))),
            min(height, int(np.ceil(bottom * height + padding_y))),
        )
    )


class HighAccuracySubjectModels:
    """Own one batched DINO model for a complete Cull run.

    Cull shares the model, its provisioning, its loader and the decoded analysis
    image with the similarity pipeline, but keeps its own artifact cache on
    purpose: it needs the dense patch tokens of every image, which are orders of
    magnitude larger than the single embedding similarity stores. Merging the two
    records would inflate the similarity cache for results it never reads.
    """

    def __init__(
        self,
        *,
        snapshots: dict[str, str],
        image_pipeline: ImagePipeline,
    ) -> None:
        self.snapshots = snapshots
        self.image_pipeline = image_pipeline
        self.device = get_preferred_torch_device()
        self.model_signature = subject_model_signature(snapshots)
        self._dino: SimilarityEmbeddingModel | None = None
        self._face_service = FaceAnalysisService()

    def _load(self) -> None:
        if self._dino is not None:
            return
        dino = SimilarityEmbeddingModel(CULL_DINO_MODEL, allow_download=False)
        # The snapshot was resolved earlier with consent and progress reporting,
        # so it is injected rather than resolved a second time.
        dino.load(snapshot_path=self.snapshots[CULL_DINO_MODEL])
        self.device = dino.device
        self._dino = dino

    def close(self) -> None:
        self._face_service.close()
        self._dino = None

    def _image(self, path: str) -> Image.Image:
        image = self.image_pipeline.get_analysis_image(
            path, target_size=ANALYSIS_CACHE_RESOLUTION
        )
        if image is None:
            raise RuntimeError(f"Could not load analysis image: {path}")
        return image.convert("RGB")

    def extract_artifacts(
        self,
        paths: Sequence[str],
        fingerprints: dict[str, tuple[int, int]],
        *,
        should_cancel: Callable[[], bool] | None = None,
        progress_callback: Callable[[int, int], None] | None = None,
        artifact_callback: Callable[[SubjectArtifact], None] | None = None,
    ) -> dict[str, SubjectArtifact]:
        """Decode and encode images/faces in accelerator-friendly batches."""

        self._load()
        if self._dino is None:
            raise RuntimeError("Cull DINO model did not initialize")
        ordered = list(paths)
        output: dict[str, SubjectArtifact] = {}
        completed = 0
        for start in range(0, len(ordered), DINO_EXTRACTION_BATCH_SIZE):
            if should_cancel and should_cancel():
                from core.subject_grouping import SubjectGroupingCancelled

                raise SubjectGroupingCancelled
            batch_paths = ordered[start : start + DINO_EXTRACTION_BATCH_SIZE]
            images = [self._image(path) for path in batch_paths]
            globals_out, patch_outputs = self._dino.encode_with_patches(images)

            face_records: list[tuple[int, FaceDescriptor, Image.Image]] = []
            faces_by_image: list[list[SubjectEvidence]] = [[] for _ in images]
            for image_index, image in enumerate(images):
                descriptor = self._face_service.describe(np.asarray(image))
                face_records.extend(
                    (
                        image_index,
                        face,
                        _aligned_face_crop(image, face.bbox, face.landmarks),
                    )
                    for face in descriptor.faces
                )
            if face_records:
                face_vectors = self._dino.encode_with_patches(
                    [record[2] for record in face_records]
                )[0]
                for (image_index, face, _crop), vector in zip(
                    face_records, face_vectors, strict=True
                ):
                    left, top, right, bottom = face.bbox
                    faces_by_image[image_index].append(
                        SubjectEvidence(
                            descriptor=_descriptor_tuple(vector),
                            bbox=face.bbox,
                            area_fraction=max(0.0, (right - left) * (bottom - top)),
                            kind="face",
                        )
                    )

            for image_index, path in enumerate(batch_paths):
                artifact = SubjectArtifact(
                    path=path,
                    fingerprint=fingerprints[path],
                    model_signature=self.model_signature,
                    global_descriptor=_descriptor_tuple(globals_out[image_index]),
                    subjects=_dense_grid_evidence(patch_outputs[image_index]),
                    faces=tuple(faces_by_image[image_index]),
                    local_features=None,
                )
                output[path] = artifact
                completed += 1
                if artifact_callback:
                    artifact_callback(artifact)
                if progress_callback:
                    progress_callback(completed, len(ordered))
        return output

    def extract_artifact(
        self,
        path: str,
        fingerprint: tuple[int, int],
    ) -> SubjectArtifact:
        return self.extract_artifacts([path], {path: fingerprint})[path]

    def verify_geometry(
        self,
        first: SubjectArtifact,
        second: SubjectArtifact,
    ) -> GeometryEvidence:
        """Verify spatial consistency directly from the cached DINO region grid."""

        if len(first.subjects) < 4 or len(second.subjects) < 4:
            return GeometryEvidence(evaluated=True)
        descriptors_a = _normalized(
            np.asarray([item.descriptor for item in first.subjects], dtype=np.float32)
        )
        descriptors_b = _normalized(
            np.asarray([item.descriptor for item in second.subjects], dtype=np.float32)
        )
        similarities = descriptors_a @ descriptors_b.T
        forward = np.argmax(similarities, axis=1)
        reverse = np.argmax(similarities, axis=0)
        matches = [
            (index, int(other))
            for index, other in enumerate(forward)
            if reverse[int(other)] == index and similarities[index, int(other)] >= 0.72
        ]
        if len(matches) < 4:
            return GeometryEvidence(evaluated=True)

        def center(subject: SubjectEvidence) -> tuple[float, float]:
            left, top, right, bottom = subject.bbox
            return ((left + right) / 2, (top + bottom) / 2)

        points_a: np.ndarray = np.asarray(
            [center(first.subjects[left]) for left, _right in matches],
            dtype=np.float32,
        )
        points_b: np.ndarray = np.asarray(
            [center(second.subjects[right]) for _left, right in matches],
            dtype=np.float32,
        )
        _homography, inlier_mask = cv2.findHomography(
            points_a, points_b, cv2.RANSAC, 0.12
        )
        if inlier_mask is None:
            return GeometryEvidence(evaluated=True)
        inliers = inlier_mask.reshape(-1).astype(bool)
        count = int(inliers.sum())

        def coverage(points: np.ndarray) -> float:
            selected = points[inliers]
            if len(selected) < 3:
                return 0.0
            return float(cv2.contourArea(cv2.convexHull(selected)))

        return GeometryEvidence(
            inlier_count=count,
            inlier_ratio=float(count / len(matches)),
            coverage_a=coverage(points_a),
            coverage_b=coverage(points_b),
            evaluated=True,
        )

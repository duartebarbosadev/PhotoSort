import logging
import os
import hashlib
import time
from collections import OrderedDict

import cv2
import numpy as np

from PyQt6.QtCore import QObject, pyqtSignal

from core import app_settings
from core.image_features.blur_detector import BLUR_DETECTION_PREVIEW_SIZE, BlurDetector
from core.image_features.face_analysis import (
    FaceAnalysisService,
    SubjectDescriptor,
    face_descriptor_signature,
)
from core.image_features.near_duplicate import (
    NearDuplicateAssessment,
    NearDuplicateDecision,
    SubjectSafeNearDuplicateComparator,
)
from core.image_pipeline import ANALYSIS_CACHE_RESOLUTION, ImagePipeline
from core.similarity_utils import cosine_similarity

logger = logging.getLogger(__name__)

_SHARPNESS_SCORE_WEIGHT = (
    1_000_000_000_000  # One sharpness point dominates tie-breakers.
)
_EXIF_FIELD_SCORE_WEIGHT = (
    1_000_000_000  # One EXIF field dominates file-size differences.
)
_MAX_EXIF_FIELDS_FOR_SCORE = 999
_MAX_FILE_SIZE_SCORE = _EXIF_FIELD_SCORE_WEIGHT - 1
_ANALYSIS_RGB_HOT_CACHE_SIZE = 32


class EasyDeleteWorker(QObject):
    """Detects obviously bad images: blurry, near-black, overexposed, near-duplicates."""

    progress_update = pyqtSignal(int, str)
    completed = pyqtSignal(dict)  # {path: {type, pair_path, suggest_delete, reason}}
    assessments_ready = pyqtSignal(dict)  # {(path_a, path_b): assessment metrics}
    error = pyqtSignal(str)
    finished = pyqtSignal()

    def __init__(
        self,
        image_paths: list[str],
        cluster_map: dict[int, list[str]] | None = None,
        embeddings_cache: dict | None = None,
        exif_disk_cache=None,
        image_pipeline: ImagePipeline | None = None,
        analysis_cache=None,
        folder_path: str | None = None,
        fingerprints: dict[str, tuple[int, int]] | None = None,
        face_analysis_service: FaceAnalysisService | None = None,
        parent: QObject | None = None,
    ):
        super().__init__(parent)
        self.image_paths = list(image_paths)
        self.cluster_map = cluster_map or {}
        self.embeddings_cache = embeddings_cache or {}
        self.exif_disk_cache = exif_disk_cache
        self.image_pipeline = image_pipeline
        self.analysis_cache = analysis_cache
        self.folder_path = folder_path
        self.fingerprints = dict(fingerprints or {})
        self._face_analysis_service = face_analysis_service
        self._should_stop = False
        self._sharpness_cache: dict[str, float] = {}
        self._analysis_rgb_cache: OrderedDict[str, np.ndarray | None] = OrderedDict()
        self._subject_descriptor_cache: dict[str, SubjectDescriptor | None] = {}
        self._pending_subject_descriptors: dict[str, dict[str, object]] = {}
        self._hash_cache: dict[str, str | None] = {}
        self._near_duplicate_comparator = SubjectSafeNearDuplicateComparator(
            lambda: self._should_stop
        )
        self._face_descriptor_signature: str | None = None
        self.pair_assessments: dict[tuple[str, str], dict[str, object]] = {}

    def stop(self) -> None:
        self._should_stop = True

    def run(self) -> None:
        try:
            self._run()
        except Exception as exc:
            logger.error("EasyDeleteWorker: unexpected error", exc_info=True)
            self.error.emit(str(exc))
        finally:
            self._flush_subject_descriptors()
            if self._face_analysis_service is not None:
                self._face_analysis_service.close()
            self.finished.emit()

    def _run(self) -> None:
        results: dict[str, dict] = {}
        total = len(self.image_paths)
        if total == 0:
            self.completed.emit(results)
            return

        for i, path in enumerate(self.image_paths):
            if self._should_stop:
                break
            percent = int((i / total) * 60)
            self.progress_update.emit(
                percent, f"Analyzing {os.path.basename(path)}… ({i + 1}/{total})"
            )
            issue = self._detect_issue(path)
            if issue:
                results[path] = issue

        if not self._should_stop and self.cluster_map and self.embeddings_cache:
            self.progress_update.emit(60, "Detecting near-duplicates…")
            for path, entry in self._detect_duplicates().items():
                if path not in results:
                    results[path] = entry

        if not self._should_stop:
            self.progress_update.emit(100, "Detection complete.")
            self.assessments_ready.emit(dict(self.pair_assessments))
            self.completed.emit(results)

    def _detect_issue(self, path: str) -> dict | None:
        gray = self._load_gray_for_detection(path)
        if gray is None:
            return None

        sharpness = self._sharpness_for_gray(path, gray)

        mean_brightness = float(gray.mean())
        black_fraction = float(
            np.count_nonzero(gray <= app_settings.EASY_DELETE_DARK_CLIP_VALUE)
            / gray.size
        )
        if mean_brightness < app_settings.get_easy_delete_dark_threshold():
            if black_fraction >= app_settings.EASY_DELETE_DARK_CLIP_FRACTION:
                return {
                    "type": "dark",
                    "pair_path": None,
                    "suggest_delete": True,
                    "reason": (
                        "Effectively black image "
                        f"(mean brightness: {mean_brightness:.1f}/255; "
                        f"{black_fraction:.1%} of pixels at or below "
                        f"{app_settings.EASY_DELETE_DARK_CLIP_VALUE}/255)"
                    ),
                    "sharpness": sharpness,
                    "mean_brightness": mean_brightness,
                    "black_fraction": black_fraction,
                }
            # Low-light previews can have a misleadingly low blur score. Preserve any
            # dark frame with visible tonal variation for exposure recovery or Cull.
            return None

        if sharpness < app_settings.get_easy_delete_blur_threshold():
            return {
                "type": "blur",
                "pair_path": None,
                "suggest_delete": True,
                "reason": f"Blurry image (peak local sharpness score: {sharpness:.1f})",
                "sharpness": sharpness,
            }

        if mean_brightness > app_settings.get_easy_delete_white_threshold():
            return {
                "type": "white",
                "pair_path": None,
                "suggest_delete": True,
                "reason": f"Overexposed/white image (mean brightness: {mean_brightness:.1f}/255)",
                "sharpness": sharpness,
            }
        return None

    @staticmethod
    def _compute_local_sharpness(gray: np.ndarray) -> float:
        """Return the peak Laplacian variance across a configured tile grid."""
        grid = app_settings.EASY_DELETE_BLUR_TILE_GRID
        height, width = gray.shape[:2]
        if grid <= 1 or height < grid or width < grid:
            return float(cv2.Laplacian(gray, cv2.CV_64F).var())

        max_variance = 0.0
        for rows in np.array_split(gray, grid, axis=0):
            for tile in np.array_split(rows, grid, axis=1):
                if tile.size == 0:
                    continue
                variance = float(cv2.Laplacian(tile, cv2.CV_64F).var())
                max_variance = max(max_variance, variance)
        return max_variance

    def _load_gray_for_detection(self, path: str) -> np.ndarray | None:
        rgb = self._get_analysis_rgb(path)
        if rgb is None:
            return None
        height, width = rgb.shape[:2]
        target_width, target_height = BLUR_DETECTION_PREVIEW_SIZE
        scale = min(target_width / width, target_height / height, 1.0)
        if scale < 1.0:
            rgb = cv2.resize(
                rgb,
                (max(1, round(width * scale)), max(1, round(height * scale))),
                interpolation=cv2.INTER_AREA,
            )
        gray = cv2.cvtColor(rgb, cv2.COLOR_RGB2GRAY)
        return gray

    def _get_analysis_rgb(self, path: str) -> np.ndarray | None:
        if path in self._analysis_rgb_cache:
            self._analysis_rgb_cache.move_to_end(path)
            return self._analysis_rgb_cache[path]
        try:
            if self.image_pipeline is not None:
                image = self.image_pipeline.get_analysis_image(
                    path,
                    target_size=ANALYSIS_CACHE_RESOLUTION,
                )
            else:
                image = BlurDetector._load_image_for_detection(
                    path,
                    target_size=ANALYSIS_CACHE_RESOLUTION,
                    apply_auto_edits_for_raw=False,
                )
            rgb = (
                np.ascontiguousarray(np.asarray(image.convert("RGB")))
                if image is not None
                else None
            )
        except Exception:
            logger.debug(
                "EasyDeleteWorker: failed to load analysis image for %s",
                path,
                exc_info=True,
            )
            rgb = None
        self._analysis_rgb_cache[path] = rgb
        self._analysis_rgb_cache.move_to_end(path)
        while len(self._analysis_rgb_cache) > _ANALYSIS_RGB_HOT_CACHE_SIZE:
            self._analysis_rgb_cache.popitem(last=False)
        return rgb

    def _fingerprint(self, path: str) -> tuple[int, int] | None:
        supplied = self.fingerprints.get(path)
        if supplied is not None:
            return tuple(supplied)
        try:
            stat_result = os.stat(path)
        except OSError:
            return None
        fingerprint = (int(stat_result.st_size), int(stat_result.st_mtime_ns))
        self.fingerprints[path] = fingerprint
        return fingerprint

    def _subject_descriptor(
        self, path: str, rgb: np.ndarray | None
    ) -> SubjectDescriptor | None:
        if path in self._subject_descriptor_cache:
            return self._subject_descriptor_cache[path]
        if rgb is None or self._should_stop:
            self._subject_descriptor_cache[path] = None
            return None

        fingerprint = self._fingerprint(path)
        if self._face_descriptor_signature is None:
            try:
                self._face_descriptor_signature = face_descriptor_signature()
            except OSError:
                logger.warning(
                    "EasyDeleteWorker: face model signature unavailable",
                    exc_info=True,
                )
                self._subject_descriptor_cache[path] = None
                return None
        descriptor_signature = self._face_descriptor_signature
        if (
            fingerprint is not None
            and self.analysis_cache is not None
            and self.folder_path
        ):
            cached = self.analysis_cache.load_subject_descriptor(
                self.folder_path,
                path,
                fingerprint=fingerprint,
                signature=descriptor_signature,
            )
            descriptor = SubjectDescriptor.from_dict(cached)
            if descriptor is not None:
                self._subject_descriptor_cache[path] = descriptor
                return descriptor

        try:
            if self._face_analysis_service is None:
                self._face_analysis_service = FaceAnalysisService()
            descriptor = self._face_analysis_service.describe(rgb)
        except Exception:
            logger.warning(
                "EasyDeleteWorker: face analysis unavailable for %s",
                path,
                exc_info=True,
            )
            descriptor = None
        self._subject_descriptor_cache[path] = descriptor
        if (
            descriptor is not None
            and fingerprint is not None
            and self.analysis_cache is not None
            and self.folder_path
        ):
            self._pending_subject_descriptors[path] = {
                "fingerprint": fingerprint,
                "signature": descriptor_signature,
                "descriptor": descriptor.to_dict(),
            }
        return descriptor

    def _flush_subject_descriptors(self) -> None:
        if (
            not self._pending_subject_descriptors
            or self.analysis_cache is None
            or not self.folder_path
        ):
            return
        pending = self._pending_subject_descriptors
        self._pending_subject_descriptors = {}
        try:
            self.analysis_cache.save_subject_descriptors_batch(
                self.folder_path,
                pending,
            )
        except Exception:
            logger.warning(
                "EasyDeleteWorker: failed to persist subject descriptors",
                exc_info=True,
            )

    def _sharpness_for_gray(self, path: str, gray: np.ndarray) -> float:
        sharpness = self._compute_local_sharpness(gray)
        self._sharpness_cache[path] = sharpness
        return sharpness

    def _get_sharpness(self, path: str) -> float:
        if path in self._sharpness_cache:
            return self._sharpness_cache[path]

        try:
            gray = self._load_gray_for_detection(path)
            if gray is None:
                self._sharpness_cache[path] = 0.0
                return 0.0
            return self._sharpness_for_gray(path, gray)
        except Exception:
            logger.debug(
                "EasyDeleteWorker: failed to compute sharpness for %s",
                path,
                exc_info=True,
            )
            self._sharpness_cache[path] = 0.0
            return 0.0

    def _detect_duplicates(self) -> dict[str, dict]:
        results: dict[str, dict] = {}
        assigned_paths: set[str] = set()
        duplicate_distance = app_settings.get_easy_delete_duplicate_distance()
        rejected_counts = {
            NearDuplicateDecision.SUBJECT_CHANGED: 0,
            NearDuplicateDecision.UNCERTAIN: 0,
        }
        started_at = time.perf_counter()
        total_pairs = sum(
            embedded_count * (embedded_count - 1) // 2
            for paths in self.cluster_map.values()
            if (embedded_count := sum(path in self.embeddings_cache for path in paths))
                >= 2
        )
        processed_pairs = 0
        progress_interval = max(1, total_pairs // 100)

        for paths in self.cluster_map.values():
            if len(paths) < 2 or self._should_stop:
                continue

            embedded = [
                (p, np.array(self.embeddings_cache[p], dtype=np.float32))
                for p in paths
                if p in self.embeddings_cache
            ]
            if len(embedded) < 2:
                continue

            candidates: list[
                tuple[
                    bool,
                    float,
                    int,
                    int,
                    str,
                    str,
                    bool,
                    float,
                    NearDuplicateAssessment,
                ]
            ] = []
            for i in range(len(embedded)):
                for j in range(i + 1, len(embedded)):
                    if self._should_stop:
                        break
                    processed_pairs += 1
                    if (
                        processed_pairs == 1
                        or processed_pairs == total_pairs
                        or processed_pairs % progress_interval == 0
                    ):
                        percent = 60 + int(
                            39 * processed_pairs / max(total_pairs, 1)
                        )
                        self.progress_update.emit(
                            percent,
                            "Checking subject-safe near-duplicates… "
                            f"({processed_pairs}/{total_pairs})",
                        )
                    path_i, emb_i = embedded[i]
                    path_j, emb_j = embedded[j]

                    similarity = cosine_similarity(emb_i, emb_j)
                    if similarity is None:
                        continue
                    cosine_dist = max(0.0, 1.0 - similarity)
                    identical = False
                    if cosine_dist < duplicate_distance:
                        identical = self._files_are_identical(path_i, path_j)
                    if identical:
                        assessment = self._near_duplicate_comparator.assess(
                            path_i,
                            path_j,
                            self._fingerprint(path_i),
                            self._fingerprint(path_j),
                            None,
                            None,
                            identical=True,
                        )
                    elif (
                        similarity
                        >= app_settings.EASY_DELETE_SAME_FRAME_MIN_COSINE_SIMILARITY
                    ):
                        first_rgb = self._get_analysis_rgb(path_i)
                        second_rgb = self._get_analysis_rgb(path_j)
                        assessment = self._near_duplicate_comparator.assess(
                            path_i,
                            path_j,
                            self._fingerprint(path_i),
                            self._fingerprint(path_j),
                            first_rgb,
                            second_rgb,
                            descriptor_loader_a=lambda path=path_i, rgb=first_rgb: (
                                self._subject_descriptor(path, rgb)
                            ),
                            descriptor_loader_b=lambda path=path_j, rgb=second_rgb: (
                                self._subject_descriptor(path, rgb)
                            ),
                        )
                    else:
                        continue

                    pair_key = tuple(sorted((path_i, path_j)))
                    self.pair_assessments[pair_key] = assessment.result_metrics()
                    if assessment.accepted:
                        structural_similarity = assessment.structural_similarity
                        visual_distance = min(
                            cosine_dist,
                            1.0 - structural_similarity
                            if structural_similarity is not None
                            else cosine_dist,
                        )
                        candidates.append(
                            (
                                not identical,
                                visual_distance,
                                i,
                                j,
                                path_i,
                                path_j,
                                identical,
                                similarity,
                                assessment,
                            )
                        )
                    elif assessment.decision in rejected_counts:
                        rejected_counts[assessment.decision] += 1

            # Exact duplicates come first, then the visually closest pairs.
            # Stable source indexes make equal-distance choices deterministic.
            candidates.sort(key=lambda candidate: candidate[:4])
            for (
                _near_duplicate,
                _distance,
                _i,
                _j,
                path_i,
                path_j,
                identical,
                cosine_match,
                assessment,
            ) in candidates:
                if self._should_stop:
                    break
                if path_i in assigned_paths or path_j in assigned_paths:
                    continue
                assigned_paths.update((path_i, path_j))

                score_i = self._keep_score(path_i)
                score_j = self._keep_score(path_j)
                if score_i >= score_j:
                    delete_path, keep_path = path_j, path_i
                else:
                    delete_path, keep_path = path_i, path_j

                duplicate_kind = "exact" if identical else "near"
                classification_label = (
                    "Exact copy"
                    if identical
                    else "Safe near-duplicate · indistinguishable at normal view"
                )
                delete_suggestion_reason, keep_suggestion_reason = (
                    self._duplicate_suggestion_reasons(
                        delete_path, keep_path, identical=identical
                    )
                )

                results[delete_path] = {
                    "type": "duplicate",
                    "pair_path": keep_path,
                    "suggest_delete": True,
                    "duplicate_kind": duplicate_kind,
                    "classification_label": classification_label,
                    "cosine_similarity": cosine_match,
                    **assessment.result_metrics(),
                    "reason": self._duplicate_reason(
                        delete_path, keep_path, identical=identical
                    ),
                    "delete_suggestion_reason": delete_suggestion_reason,
                    "keep_suggestion_reason": keep_suggestion_reason,
                    "sharpness": self._get_sharpness(delete_path),
                }
                results[keep_path] = {
                    "type": "duplicate",
                    "pair_path": delete_path,
                    "suggest_delete": False,
                    "duplicate_kind": duplicate_kind,
                    "classification_label": classification_label,
                    "cosine_similarity": cosine_match,
                    **assessment.result_metrics(),
                    "reason": "Suggested to keep this photo",
                    "delete_suggestion_reason": delete_suggestion_reason,
                    "keep_suggestion_reason": keep_suggestion_reason,
                    "sharpness": self._get_sharpness(keep_path),
                }
                logger.info(
                    "Easy Delete accepted pair: %s ↔ %s classification=%s "
                    "reason=%s cosine=%.6f structural=%s alignment=%s "
                    "normal_view_change=%s",
                    os.path.basename(path_i),
                    os.path.basename(path_j),
                    assessment.decision.value,
                    assessment.reason_code,
                    cosine_match,
                    assessment.structural_similarity,
                    assessment.metrics.get("alignment_correlation"),
                    assessment.metrics.get("normal_view_change_fraction"),
                )

        alignment_seconds = sum(
            float(metrics.get("alignment_seconds") or 0.0)
            for metrics in self.pair_assessments.values()
        )
        perceptual_seconds = sum(
            float(metrics.get("perceptual_seconds") or 0.0)
            for metrics in self.pair_assessments.values()
        )
        face_seconds = sum(
            float(metrics.get("face_seconds") or 0.0)
            for metrics in self.pair_assessments.values()
        )
        logger.info(
            "Easy Delete near-duplicate assessment finished in %.3fs: "
            "accepted_pairs=%d subject_changed=%d uncertain=%d "
            "alignment=%.3fs perceptual=%.3fs face=%.3fs",
            time.perf_counter() - started_at,
            len(results) // 2,
            rejected_counts[NearDuplicateDecision.SUBJECT_CHANGED],
            rejected_counts[NearDuplicateDecision.UNCERTAIN],
            alignment_seconds,
            perceptual_seconds,
            face_seconds,
        )
        return results

    def _keep_score(self, path: str) -> int:
        """Higher = prefer to keep. Sharpness first, then EXIF richness, then file size."""
        sharpness_component = round(self._get_sharpness(path))
        exif_component = min(self._exif_field_count(path), _MAX_EXIF_FIELDS_FOR_SCORE)
        file_size_component = min(self._file_size(path), _MAX_FILE_SIZE_SCORE)
        return (
            sharpness_component * _SHARPNESS_SCORE_WEIGHT
            + exif_component * _EXIF_FIELD_SCORE_WEIGHT
            + file_size_component
        )

    def _exif_field_count(self, path: str) -> int:
        exif_count = 0
        if self.exif_disk_cache:
            try:
                data = self.exif_disk_cache.get(path)
                if data:
                    exif_count = sum(
                        1
                        for v in data.values()
                        if v is not None and v != "" and str(v) != "None"
                    )
            except Exception:
                pass

        return exif_count

    def _file_size(self, path: str) -> int:
        try:
            return os.path.getsize(path)
        except OSError:
            return 0

    def _file_hash(self, path: str) -> str | None:
        """Return a SHA-256 hex digest of the file's bytes (cached), or None on error."""
        if path in self._hash_cache:
            return self._hash_cache[path]
        digest: str | None = None
        try:
            hasher = hashlib.sha256()
            with open(path, "rb") as handle:
                for chunk in iter(lambda: handle.read(1024 * 1024), b""):
                    hasher.update(chunk)
            digest = hasher.hexdigest()
        except OSError:
            logger.debug("EasyDeleteWorker: failed to hash %s", path, exc_info=True)
        self._hash_cache[path] = digest
        return digest

    def _files_are_identical(self, path_a: str, path_b: str) -> bool:
        """True only if both files are byte-for-byte identical (same size and hash)."""
        size_a = self._file_size(path_a)
        if size_a == 0 or size_a != self._file_size(path_b):
            return False
        hash_a = self._file_hash(path_a)
        return hash_a is not None and hash_a == self._file_hash(path_b)

    def _duplicate_reason(
        self, delete_path: str, keep_path: str, *, identical: bool | None = None
    ) -> str:
        if identical is None:
            identical = self._files_are_identical(delete_path, keep_path)
        if identical:
            return "The files are byte-for-byte identical"

        reasons = []
        delete_sharpness = self._get_sharpness(delete_path)
        keep_sharpness = self._get_sharpness(keep_path)
        if round(keep_sharpness) > round(delete_sharpness):
            reasons.append(
                f"lower sharpness ({delete_sharpness:.1f} vs {keep_sharpness:.1f})"
            )

        delete_exif = self._exif_field_count(delete_path)
        keep_exif = self._exif_field_count(keep_path)
        if keep_exif > delete_exif:
            reasons.append(f"less EXIF data ({delete_exif} vs {keep_exif} fields)")

        try:
            delete_size = os.path.getsize(delete_path)
            keep_size = os.path.getsize(keep_path)
            if keep_size > delete_size:
                reasons.append(
                    f"smaller file ({delete_size // 1024}KB vs {keep_size // 1024}KB)"
                )
        except OSError:
            pass

        if not reasons:
            reasons.append("the files are visually almost identical")

        return f"Suggested choice: {', '.join(reasons)}"

    def _duplicate_suggestion_reasons(
        self, delete_path: str, keep_path: str, *, identical: bool
    ) -> tuple[str, str]:
        """Explain the decisive keep-score signal from each photo's perspective."""
        if identical:
            reason = "byte-for-byte identical"
            return reason, reason

        delete_sharpness = self._get_sharpness(delete_path)
        keep_sharpness = self._get_sharpness(keep_path)
        if round(keep_sharpness) > round(delete_sharpness):
            values = f"{keep_sharpness:.1f} vs {delete_sharpness:.1f}"
            return (
                f"lower sharpness ({delete_sharpness:.1f} vs {keep_sharpness:.1f})",
                f"higher sharpness ({values})",
            )

        delete_exif = self._exif_field_count(delete_path)
        keep_exif = self._exif_field_count(keep_path)
        if keep_exif > delete_exif:
            return (
                f"less EXIF data ({delete_exif} vs {keep_exif} fields)",
                f"more EXIF data ({keep_exif} vs {delete_exif} fields)",
            )

        delete_size = self._file_size(delete_path)
        keep_size = self._file_size(keep_path)
        if min(keep_size, _MAX_FILE_SIZE_SCORE) > min(
            delete_size, _MAX_FILE_SIZE_SCORE
        ):
            return (
                f"smaller file ({delete_size // 1024}KB vs {keep_size // 1024}KB)",
                f"larger file ({keep_size // 1024}KB vs {delete_size // 1024}KB)",
            )

        reason = "quality signals tied; pair order used as the tie-breaker"
        return reason, reason

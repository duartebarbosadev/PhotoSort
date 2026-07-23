import logging
import os
import hashlib

import cv2
import numpy as np

from PyQt6.QtCore import QObject, pyqtSignal

from core import app_settings
from core.image_features.blur_detector import BLUR_DETECTION_PREVIEW_SIZE, BlurDetector
from core.image_features.structural_similarity import (
    aligned_structural_similarity,
    prepare_same_frame_preview,
)
from core.image_pipeline import ImagePipeline
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


class EasyDeleteWorker(QObject):
    """Detects obviously bad images: blurry, near-black, overexposed, near-duplicates."""

    progress_update = pyqtSignal(int, str)
    completed = pyqtSignal(dict)  # {path: {type, pair_path, suggest_delete, reason}}
    error = pyqtSignal(str)
    finished = pyqtSignal()

    def __init__(
        self,
        image_paths: list[str],
        cluster_map: dict[int, list[str]] | None = None,
        embeddings_cache: dict | None = None,
        exif_disk_cache=None,
        image_pipeline: ImagePipeline | None = None,
        parent: QObject | None = None,
    ):
        super().__init__(parent)
        self.image_paths = list(image_paths)
        self.cluster_map = cluster_map or {}
        self.embeddings_cache = embeddings_cache or {}
        self.exif_disk_cache = exif_disk_cache
        self.image_pipeline = image_pipeline
        self._should_stop = False
        self._sharpness_cache: dict[str, float] = {}
        self._structural_preview_cache: dict[str, np.ndarray | None] = {}
        self._hash_cache: dict[str, str | None] = {}

    def stop(self) -> None:
        self._should_stop = True

    def run(self) -> None:
        try:
            self._run()
        except Exception as exc:
            logger.error("EasyDeleteWorker: unexpected error", exc_info=True)
            self.error.emit(str(exc))
        finally:
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
            self.completed.emit(results)

    def _detect_issue(self, path: str) -> dict | None:
        gray = self._load_gray_for_detection(path)
        if gray is None:
            return None

        sharpness = self._sharpness_for_gray(path, gray)
        self._structural_preview_cache[path] = prepare_same_frame_preview(gray)

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
        if self.image_pipeline is not None:
            pil_img = self.image_pipeline.get_analysis_image(
                path,
                target_size=BLUR_DETECTION_PREVIEW_SIZE,
            )
        else:
            pil_img = BlurDetector._load_image_for_detection(
                path,
                target_size=BLUR_DETECTION_PREVIEW_SIZE,
                apply_auto_edits_for_raw=False,
            )
        if pil_img is None:
            return None

        bgr = cv2.cvtColor(np.array(pil_img), cv2.COLOR_RGB2BGR)
        return cv2.cvtColor(bgr, cv2.COLOR_BGR2GRAY)

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

    def _get_structural_preview(self, path: str) -> np.ndarray | None:
        if path in self._structural_preview_cache:
            return self._structural_preview_cache[path]
        try:
            gray = self._load_gray_for_detection(path)
            preview = (
                prepare_same_frame_preview(gray) if gray is not None else None
            )
        except Exception:
            logger.debug(
                "EasyDeleteWorker: failed to prepare structural preview for %s",
                path,
                exc_info=True,
            )
            preview = None
        self._structural_preview_cache[path] = preview
        return preview

    def _same_frame_similarity(self, path_a: str, path_b: str) -> float | None:
        first = self._get_structural_preview(path_a)
        second = self._get_structural_preview(path_b)
        if first is None or second is None:
            return None
        return aligned_structural_similarity(first, second)

    def _detect_duplicates(self) -> dict[str, dict]:
        results: dict[str, dict] = {}
        assigned_paths: set[str] = set()
        duplicate_distance = app_settings.get_easy_delete_duplicate_distance()

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
                    float | None,
                ]
            ] = []
            for i in range(len(embedded)):
                for j in range(i + 1, len(embedded)):
                    if self._should_stop:
                        break
                    path_i, emb_i = embedded[i]
                    path_j, emb_j = embedded[j]

                    similarity = cosine_similarity(emb_i, emb_j)
                    if similarity is None:
                        continue
                    cosine_dist = max(0.0, 1.0 - similarity)
                    identical = False
                    if cosine_dist < duplicate_distance:
                        identical = self._files_are_identical(path_i, path_j)
                    structural_similarity = None
                    if (
                        not identical
                        and similarity
                        >= app_settings.EASY_DELETE_SAME_FRAME_MIN_COSINE_SIMILARITY
                    ):
                        structural_similarity = self._same_frame_similarity(
                            path_i, path_j
                        )
                    same_frame = (
                        structural_similarity is not None
                        and structural_similarity
                        >= app_settings.EASY_DELETE_SAME_FRAME_SIMILARITY
                    )
                    cosine_fallback = (
                        structural_similarity is None
                        and cosine_dist < duplicate_distance
                    )
                    if identical or same_frame or cosine_fallback:
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
                                structural_similarity,
                            )
                        )

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
                structural_match,
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
                    "cosine_similarity": cosine_match,
                    "structural_similarity": structural_match,
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
                    "cosine_similarity": cosine_match,
                    "structural_similarity": structural_match,
                    "reason": "Suggested to keep this photo",
                    "delete_suggestion_reason": delete_suggestion_reason,
                    "keep_suggestion_reason": keep_suggestion_reason,
                    "sharpness": self._get_sharpness(keep_path),
                }

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

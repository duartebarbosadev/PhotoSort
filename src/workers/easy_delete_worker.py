import logging
import os
import hashlib

import cv2
import numpy as np

from PyQt6.QtCore import QObject, pyqtSignal

from core import app_settings
from core.image_features.blur_detector import BLUR_DETECTION_PREVIEW_SIZE, BlurDetector
from core.image_pipeline import ImagePipeline

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

        if sharpness < app_settings.get_easy_delete_blur_threshold():
            return {
                "type": "blur",
                "pair_path": None,
                "suggest_delete": True,
                "reason": f"Blurry image (peak local sharpness score: {sharpness:.1f})",
                "sharpness": sharpness,
            }

        mean_brightness = float(gray.mean())
        if mean_brightness < app_settings.get_easy_delete_dark_threshold():
            return {
                "type": "dark",
                "pair_path": None,
                "suggest_delete": True,
                "reason": f"Near-black image (mean brightness: {mean_brightness:.1f}/255)",
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

    def _detect_duplicates(self) -> dict[str, dict]:
        results: dict[str, dict] = {}
        # Track which paths are already part of a reported pair to avoid duplicates
        already_paired: set = set()
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

            for i in range(len(embedded)):
                for j in range(i + 1, len(embedded)):
                    if self._should_stop:
                        break
                    path_i, emb_i = embedded[i]
                    path_j, emb_j = embedded[j]

                    pair_key = frozenset((path_i, path_j))
                    if pair_key in already_paired:
                        continue

                    norm_i = float(np.linalg.norm(emb_i))
                    norm_j = float(np.linalg.norm(emb_j))
                    if norm_i == 0 or norm_j == 0:
                        continue

                    cosine_sim = float(np.dot(emb_i, emb_j) / (norm_i * norm_j))
                    cosine_dist = max(0.0, 1.0 - cosine_sim)

                    if cosine_dist < duplicate_distance:
                        already_paired.add(pair_key)
                        score_i = self._keep_score(path_i)
                        score_j = self._keep_score(path_j)
                        if score_i >= score_j:
                            delete_path, keep_path = path_j, path_i
                        else:
                            delete_path, keep_path = path_i, path_j

                        results[delete_path] = {
                            "type": "duplicate",
                            "pair_path": keep_path,
                            "suggest_delete": True,
                            "reason": self._duplicate_reason(delete_path, keep_path),
                            "sharpness": self._get_sharpness(delete_path),
                        }
                        if keep_path not in results:
                            identical = self._files_are_identical(
                                delete_path, keep_path
                            )
                            dup_label = "Exact" if identical else "Near"
                            results[keep_path] = {
                                "type": "duplicate",
                                "pair_path": delete_path,
                                "suggest_delete": False,
                                "reason": f"{dup_label}-duplicate of {os.path.basename(delete_path)} — suggested to keep",
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

    def _duplicate_reason(self, delete_path: str, keep_path: str) -> str:
        keep_name = os.path.basename(keep_path)
        if self._files_are_identical(delete_path, keep_path):
            return f"Exact duplicate of {keep_name} — identical file"

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
            reasons.append("near-identical duplicate")

        return f"Near-duplicate of {keep_name} — {', '.join(reasons)}"

import logging
import os
from typing import Dict, List, Optional

import cv2
import numpy as np

from PyQt6.QtCore import QObject, pyqtSignal

from core.app_settings import (
    EASY_DELETE_BLUR_THRESHOLD,
    EASY_DELETE_DARK_MEAN_THRESHOLD,
    EASY_DELETE_DUPLICATE_COSINE_DISTANCE,
    EASY_DELETE_TERRIBLE_AESTHETIC_THRESHOLD,
    EASY_DELETE_WHITE_MEAN_THRESHOLD,
)
from core.image_features.blur_detector import BLUR_DETECTION_PREVIEW_SIZE, BlurDetector

logger = logging.getLogger(__name__)


class EasyDeleteWorker(QObject):
    """Detects obviously bad images: blurry, near-black, overexposed, near-duplicates."""

    progress_update = pyqtSignal(int, str)
    completed = pyqtSignal(dict)  # {path: {type, pair_path, suggest_delete, reason}}
    error = pyqtSignal(str)
    finished = pyqtSignal()

    def __init__(
        self,
        image_paths: List[str],
        cluster_map: Optional[Dict[int, List[str]]] = None,
        embeddings_cache: Optional[Dict] = None,
        exif_disk_cache=None,
        parent: Optional[QObject] = None,
    ):
        super().__init__(parent)
        self.image_paths = list(image_paths)
        self.cluster_map = cluster_map or {}
        self.embeddings_cache = embeddings_cache or {}
        self.exif_disk_cache = exif_disk_cache
        self._should_stop = False

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
        results: Dict[str, dict] = {}
        total = len(self.image_paths)
        if total == 0:
            self.completed.emit(results)
            return

        for i, path in enumerate(self.image_paths):
            if self._should_stop:
                break
            percent = int((i / total) * 60)
            self.progress_update.emit(percent, f"Analyzing {os.path.basename(path)}… ({i + 1}/{total})")
            issue = self._detect_issue(path)
            if issue:
                results[path] = issue

        if not self._should_stop and self.cluster_map and self.embeddings_cache:
            self.progress_update.emit(60, "Detecting near-duplicates…")
            for path, entry in self._detect_duplicates().items():
                if path not in results:
                    results[path] = entry

        if not self._should_stop:
            self.progress_update.emit(70, "Scoring aesthetic quality…")
            self._detect_terrible_aesthetic(results)

        if not self._should_stop:
            self.progress_update.emit(100, "Detection complete.")
            self.completed.emit(results)

    def _detect_issue(self, path: str) -> Optional[dict]:
        pil_img = BlurDetector._load_image_for_detection(
            path, target_size=BLUR_DETECTION_PREVIEW_SIZE, apply_auto_edits_for_raw=False
        )
        if pil_img is None:
            return None

        bgr = cv2.cvtColor(np.array(pil_img), cv2.COLOR_RGB2BGR)
        gray = cv2.cvtColor(bgr, cv2.COLOR_BGR2GRAY)
        laplacian_var = float(cv2.Laplacian(gray, cv2.CV_64F).var())

        if laplacian_var < EASY_DELETE_BLUR_THRESHOLD:
            return {
                "type": "blur",
                "pair_path": None,
                "suggest_delete": True,
                "reason": f"Blurry image (sharpness score: {laplacian_var:.1f})",
            }

        mean_brightness = float(gray.mean())
        if mean_brightness < EASY_DELETE_DARK_MEAN_THRESHOLD:
            return {
                "type": "dark",
                "pair_path": None,
                "suggest_delete": True,
                "reason": f"Near-black image (mean brightness: {mean_brightness:.1f}/255)",
            }
        if mean_brightness > EASY_DELETE_WHITE_MEAN_THRESHOLD:
            return {
                "type": "white",
                "pair_path": None,
                "suggest_delete": True,
                "reason": f"Overexposed/white image (mean brightness: {mean_brightness:.1f}/255)",
            }
        return None

    def _detect_duplicates(self) -> Dict[str, dict]:
        results: Dict[str, dict] = {}
        # Track which paths are already part of a reported pair to avoid duplicates
        already_paired: set = set()

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

                    if cosine_dist < EASY_DELETE_DUPLICATE_COSINE_DISTANCE:
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
                        }
                        if keep_path not in results:
                            results[keep_path] = {
                                "type": "duplicate",
                                "pair_path": delete_path,
                                "suggest_delete": False,
                                "reason": f"Near-duplicate of {os.path.basename(delete_path)} — suggested to keep",
                            }

        return results

    def _keep_score(self, path: str) -> int:
        """Higher = prefer to keep. EXIF richness * 10000 + file size."""
        exif_count = 0
        if self.exif_disk_cache:
            try:
                data = self.exif_disk_cache.get(path)
                if data:
                    exif_count = sum(
                        1 for v in data.values()
                        if v is not None and v != "" and str(v) != "None"
                    )
            except Exception:
                pass

        file_size = 0
        try:
            file_size = os.path.getsize(path)
        except OSError:
            pass

        return exif_count * 10000 + file_size

    def _duplicate_reason(self, delete_path: str, keep_path: str) -> str:
        reasons = []
        try:
            delete_size = os.path.getsize(delete_path)
            keep_size = os.path.getsize(keep_path)
            if keep_size > delete_size:
                reasons.append(f"smaller file ({delete_size // 1024}KB vs {keep_size // 1024}KB)")
        except OSError:
            pass

        delete_exif = self._keep_score(delete_path) // 10000
        keep_exif = self._keep_score(keep_path) // 10000
        if keep_exif > delete_exif:
            reasons.append(f"less EXIF data ({delete_exif} vs {keep_exif} fields)")

        if not reasons:
            reasons.append("near-identical duplicate")

        keep_name = os.path.basename(keep_path)
        return f"Near-duplicate of {keep_name} — {', '.join(reasons)}"

    def _detect_terrible_aesthetic(self, results: Dict[str, dict]) -> None:
        try:
            import torch  # noqa: F401 - check availability
            from core.best_photo_finder.scorers import HuggingFaceAestheticScorer
            from core.best_photo_finder.config import SelectorConfig
            from pathlib import Path
        except ImportError:
            logger.debug("Aesthetic scoring skipped: torch or best_photo_finder not available.")
            return

        unscored = [p for p in self.image_paths if p not in results]
        if not unscored or self._should_stop:
            logger.info("Aesthetic scoring: no unscored images to evaluate or stopped.")
            return

        logger.info(f"Aesthetic scoring: evaluating {len(unscored)} images "
                    f"(threshold < {EASY_DELETE_TERRIBLE_AESTHETIC_THRESHOLD})")

        try:
            scorer = HuggingFaceAestheticScorer()
            config = SelectorConfig()
            total = len(unscored)
            batch_size = config.aesthetic_batch_size
            logger.info(f"Aesthetic scoring: using batch size {batch_size}")
            flagged_count = 0

            for batch_start in range(0, total, batch_size):
                if self._should_stop:
                    break
                batch = unscored[batch_start : batch_start + batch_size]
                percent = 70 + int((batch_start / total) * 28)
                self.progress_update.emit(percent, f"Scoring aesthetics… ({batch_start + 1}/{total})")
                try:
                    scores = scorer.score_batch([Path(p) for p in batch], config)
                    for path_obj, score in scores.items():
                        path_str = str(path_obj)
                        logger.debug(f"Aesthetic score: {score:.4f} — {os.path.basename(path_str)}")
                        if score < EASY_DELETE_TERRIBLE_AESTHETIC_THRESHOLD and path_str not in results:
                            results[path_str] = {
                                "type": "terrible",
                                "pair_path": None,
                                "suggest_delete": True,
                                "reason": f"Low aesthetic quality (score: {score:.2f})",
                            }
                            flagged_count += 1
                            logger.info(f"Flagged TERRIBLE: {os.path.basename(path_str)} (score: {score:.4f})")
                except Exception as e:
                    logger.warning(f"Aesthetic batch scoring failed: {e}")
            logger.info(f"Aesthetic scoring complete: {flagged_count}/{total} flagged as terrible")
        except Exception as e:
            logger.warning(f"Aesthetic scoring phase failed: {e}")

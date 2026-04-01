import logging
import os
from pathlib import Path
from typing import Dict, List, Optional

from PyQt6.QtCore import QObject, pyqtSignal

from core.best_photo_finder.errors import NoScorableImagesError, NoSupportedImagesError
from core.best_photo_finder.pipeline import PhotoSelector
from core.image_processing.raw_image_processor import is_raw_extension
from core.image_processing.standard_image_processor import SUPPORTED_STANDARD_EXTENSIONS
from core.media_utils import is_video_extension

logger = logging.getLogger(__name__)


class PickBestWorker(QObject):
    """Background worker that scores similarity clusters and identifies the best image."""

    progress_update = pyqtSignal(int, str)  # percent, message
    completed = pyqtSignal(dict)  # Dict[int, dict] — per-cluster results
    error = pyqtSignal(str)
    finished = pyqtSignal()

    def __init__(
        self,
        cluster_map: Dict[int, List[str]],
        image_pipeline=None,
        parent: Optional[QObject] = None,
    ):
        super().__init__(parent)
        self.cluster_map = cluster_map
        self.image_pipeline = image_pipeline
        self._should_stop = False

    def stop(self) -> None:
        self._should_stop = True

    def run(self) -> None:
        try:
            self._run()
        finally:
            self.finished.emit()

    def _run(self) -> None:
        # Only process clusters with 2+ images
        scorable_clusters = {
            cid: paths for cid, paths in self.cluster_map.items() if len(paths) >= 2
        }

        if not scorable_clusters:
            logger.info("PickBestWorker: no clusters with 2+ images, nothing to score.")
            self.completed.emit({})
            return

        total = len(scorable_clusters)
        logger.info(f"PickBestWorker: scoring {total} clusters.")

        # Share one PhotoSelector instance so the aesthetic model loads once
        selector = PhotoSelector(preview_loader=self._load_preview_image)
        results: Dict[int, dict] = {}
        processed = 0

        for cluster_id in sorted(scorable_clusters.keys()):
            if self._should_stop:
                logger.info("PickBestWorker: stop requested.")
                break

            paths = scorable_clusters[cluster_id]

            # Filter to supported extensions only
            supported_paths = [p for p in paths if self._is_supported_path(p)]
            all_paths = list(paths)

            percent = int((processed / total) * 100)
            base_names = [os.path.basename(p) for p in paths[:3]]
            preview = ", ".join(base_names)
            self.progress_update.emit(
                percent, f"Scoring cluster {processed + 1}/{total}: {preview}…"
            )

            cluster_result: dict = {
                "winner_path": None,
                "ranked": [],
                "failed": [],
                "all_paths": all_paths,
                "unsupported_paths": [
                    p
                    for p in all_paths
                    if not self._is_supported_path(p)
                ],
            }

            if len(supported_paths) >= 2:
                try:
                    selection = selector.select(supported_paths)
                    cluster_result["winner_path"] = selection.winner.path
                    cluster_result["ranked"] = [
                        img.to_dict() for img in selection.ranked_images
                    ]
                    cluster_result["failed"] = [
                        img.to_dict() for img in selection.failed_images
                    ]
                    logger.debug(
                        f"Cluster {cluster_id}: winner={os.path.basename(selection.winner.path)}"
                    )
                except (NoSupportedImagesError, NoScorableImagesError) as exc:
                    logger.warning(f"Cluster {cluster_id}: skipped — {exc}")
                    # No winner; treat whole cluster as unscored
                except Exception as exc:
                    logger.error(
                        f"Cluster {cluster_id}: scoring failed — {exc}", exc_info=True
                    )
                    # Don't abort all — continue with remaining clusters
            else:
                logger.debug(
                    f"Cluster {cluster_id}: fewer than 2 supported images, skipping scoring "
                    f"({len(supported_paths)}/{len(all_paths)} supported)."
                )

            results[cluster_id] = cluster_result
            processed += 1

        if not self._should_stop:
            self.progress_update.emit(100, "Scoring complete.")
            self.completed.emit(results)

    def _is_supported_path(self, path: str) -> bool:
        ext = Path(path).suffix.lower()
        return (
            ext in SUPPORTED_STANDARD_EXTENSIONS or is_raw_extension(ext)
        ) and not is_video_extension(ext)

    def _load_preview_image(self, path: Path):
        if self.image_pipeline is None:
            return None
        return self.image_pipeline.get_preview_image(str(path))

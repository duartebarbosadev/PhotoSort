"""
Cluster Best Shot Worker
Runs the AI best shot picker across every similarity cluster without blocking the UI.
"""

from __future__ import annotations

import logging
from typing import Any, Dict, List, Optional

from PyQt6.QtCore import QObject, pyqtSignal

from core import app_settings
from core.ai.best_shot_picker import BestShotPicker, BestShotPickerError, BestShotResult

logger = logging.getLogger(__name__)


class ClusterBestShotWorker(QObject):
    """Background worker that iterates through similarity clusters."""

    progress = pyqtSignal(int, int, str)  # current cluster, total clusters, message
    cluster_result_ready = pyqtSignal(object)  # payload describing a cluster result
    finished = pyqtSignal(bool, object)  # success flag, summary list
    error = pyqtSignal(str)  # fatal error message

    def __init__(self, parent: Optional[QObject] = None):
        super().__init__(parent)
        self._is_running = True
        self._picker: Optional[BestShotPicker] = None

    def stop(self):
        """Signal the worker to stop after the current iteration."""
        self._is_running = False

    # pylint: disable=too-many-locals
    def analyze_clusters(self, clusters: List[Dict[str, Any]]):
        """Analyze each cluster and emit results progressively."""

        summary: List[Dict[str, Any]] = []
        total_clusters = len(clusters)

        if total_clusters == 0:
            self.finished.emit(True, summary)
            return

        try:
            self._is_running = True

            self._picker = BestShotPicker(
                base_url=app_settings.get_ai_best_shot_api_url(),
                api_key=app_settings.get_ai_best_shot_api_key(),
                model=app_settings.get_ai_best_shot_model(),
                timeout=app_settings.get_ai_best_shot_timeout(),
            )

            requires_api = any(
                len(cluster.get("image_paths", [])) > 1 for cluster in clusters
            )

            if requires_api:
                self.progress.emit(0, total_clusters, "Testing AI service connectivity...")
                if not self._picker.test_connection():
                    error_msg = (
                        "Failed to connect to AI service. Verify settings under "
                        "Preferences → AI Best Shot and ensure the server is running."
                    )
                    logger.error(error_msg)
                    self.error.emit(error_msg)
                    self.finished.emit(False, summary)
                    return

            for index, cluster in enumerate(clusters, start=1):
                if not self._is_running:
                    logger.info("Cluster best shot analysis cancelled by user")
                    self.finished.emit(False, summary)
                    return

                cluster_id = cluster.get("cluster_id")
                image_paths: List[str] = [
                    path
                    for path in cluster.get("image_paths", [])
                    if isinstance(path, str) and path
                ]

                image_count = len(image_paths)
                status_message = (
                    f"Cluster {cluster_id} ({image_count} image"
                    f"{'s' if image_count != 1 else ''})"
                )
                self.progress.emit(index, total_clusters, status_message)

                if not image_paths:
                    logger.info("Skipping empty cluster %s", cluster_id)
                    continue

                try:
                    if image_count == 1:
                        # No API call needed; only one candidate.
                        only_path = image_paths[0]
                        result = BestShotResult(
                            best_image_index=0,
                            best_image_path=only_path,
                            reasoning=(
                                "Cluster contains a single image. Selected by default."
                            ),
                            confidence="High",
                            raw_response="Single image cluster",
                        )
                    else:
                        preview_payloads = self._build_preview_overrides(
                            cluster.get("preview_payloads"),
                            image_paths,
                        )
                        result = self._picker.select_best_image(  # type: ignore[union-attr]
                            image_paths,
                            preview_overrides=preview_payloads or None,
                        )
                except BestShotPickerError as exc:
                    error_msg = f"Cluster {cluster_id}: {exc}"
                    logger.error(error_msg)
                    self.error.emit(error_msg)
                    self.finished.emit(False, summary)
                    return
                except Exception as exc:  # pragma: no cover - defensive
                    logger.exception("Unexpected failure analyzing cluster %s", cluster_id)
                    self.error.emit(str(exc))
                    self.finished.emit(False, summary)
                    return

                payload = {
                    "cluster_id": cluster_id,
                    "result": result,
                    "image_paths": image_paths,
                    "index": index,
                    "total": total_clusters,
                }
                summary.append(payload)
                self.cluster_result_ready.emit(payload)

            self.finished.emit(True, summary)

        except Exception as exc:  # pragma: no cover - defensive
            logger.exception("Cluster best shot worker crashed: %s", exc)
            self.error.emit(str(exc))
            self.finished.emit(False, summary)

    def _build_preview_overrides(
        self,
        preview_payloads: Optional[Dict[str, Dict[str, Any]]],
        image_paths: List[str],
    ) -> Dict[str, Dict[str, Any]]:
        if not preview_payloads or not self._picker:
            return {}

        overrides: Dict[str, Dict[str, Any]] = {}
        for image_path in image_paths:
            payload = preview_payloads.get(image_path)
            if not payload:
                continue
            pil_image = payload.get("pil_image")
            if pil_image is None:
                continue
            mime_type = payload.get("mime_type", "image/jpeg")
            overlay_label = payload.get("overlay_label")
            try:
                base64_data, effective_mime = self._picker.prepare_preview_payload(  # type: ignore[union-attr]
                    image_path=image_path,
                    pil_image=pil_image,
                    overlay_label=overlay_label,
                    mime_type=mime_type,
                )
            except BestShotPickerError as exc:
                logger.debug(
                    "Skipping cached preview override for %s: %s", image_path, exc
                )
                continue

            override_entry: Dict[str, Any] = {
                "base64": base64_data,
                "mime_type": effective_mime,
            }
            if overlay_label:
                override_entry["overlay_label"] = overlay_label
            overrides[image_path] = override_entry
        return overrides

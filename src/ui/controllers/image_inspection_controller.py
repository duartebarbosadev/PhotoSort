"""Shared quality progression for every interactive image inspection view."""

from __future__ import annotations

import logging
import os
import weakref
from collections.abc import Iterable
from dataclasses import dataclass
from enum import IntEnum
from typing import TYPE_CHECKING

from PyQt6.QtCore import QObject, QTimer

from core.app_settings import INSPECTION_DETAIL_DWELL_MS

if TYPE_CHECKING:
    from core.image_pipeline import ImagePipeline
    from ui.advanced_image_viewer import SynchronizedImageViewer
    from ui.controllers.preview_load_controller import PreviewLoadController

logger = logging.getLogger(__name__)


class InspectionQuality(IntEnum):
    PLACEHOLDER = 0
    PREVIEW = 1
    DETAIL = 2


@dataclass(frozen=True, slots=True)
class InspectionImageSpec:
    path: str
    rotation_degrees: int = 0
    label: str | None = None
    media_type: str = "image"
    rating: int = 0


class ImageInspectionController(QObject):
    """Own one cancellable, monotonic inspection session application-wide."""

    def __init__(
        self,
        image_pipeline: ImagePipeline,
        loader: PreviewLoadController,
        parent: QObject | None = None,
    ) -> None:
        super().__init__(parent)
        self._pipeline = image_pipeline
        self._loader = loader
        self._viewer: SynchronizedImageViewer | None = None
        self._specs: tuple[InspectionImageSpec, ...] = ()
        self._paths: tuple[str, ...] = ()
        self._quality: dict[str, InspectionQuality] = {}
        self._pixel_area: dict[str, int] = {}
        self._pending_detail_images: dict[str, object] = {}
        self._detail_requested = False
        self._pending_actual_size = False
        self._registered_viewers: weakref.WeakSet[SynchronizedImageViewer] = (
            weakref.WeakSet()
        )
        self._timer = QTimer(self)
        self._timer.setSingleShot(True)
        self._timer.setInterval(INSPECTION_DETAIL_DWELL_MS)
        self._timer.timeout.connect(lambda: self.request_detail(reason="dwell"))
        loader.preview_ready.connect(self._on_preview_ready)
        loader.preview_failed.connect(self._on_preview_failed)
        loader.detail_ready.connect(self._on_detail_ready)
        loader.detail_failed.connect(self._on_detail_failed)
        loader.detail_batch_finished.connect(self._on_detail_batch_finished)

    @property
    def active_paths(self) -> tuple[str, ...]:
        return self._paths

    def activate(
        self,
        viewer: SynchronizedImageViewer,
        specs: list[InspectionImageSpec] | tuple[InspectionImageSpec, ...],
    ) -> None:
        normalized = tuple(spec for spec in specs if spec.path)
        if viewer is self._viewer and normalized == self._specs:
            return

        self.clear()
        self._viewer = viewer
        self._specs = normalized
        self._paths = tuple(
            dict.fromkeys(
                spec.path for spec in normalized if spec.media_type != "video"
            )
        )
        if viewer not in self._registered_viewers:
            viewer.detail_requested.connect(
                lambda reason, source=viewer: self.request_detail(source, reason)
            )
            self._registered_viewers.add(viewer)
        viewer.defer_actual_size_until_detail()

        pixmaps = {}
        for path in self._paths:
            try:
                pixmap, cached = self._pipeline.get_immediate_review_qpixmap(path)
            except Exception:
                logger.debug(
                    "Immediate inspection frame failed: %s", path, exc_info=True
                )
                pixmap, cached = None, False
            if pixmap is not None and not pixmap.isNull():
                pixmaps[path] = pixmap
                self._quality[path] = (
                    InspectionQuality.PREVIEW
                    if cached
                    else InspectionQuality.PLACEHOLDER
                )
                self._pixel_area[path] = pixmap.width() * pixmap.height()
            else:
                self._quality[path] = InspectionQuality.PLACEHOLDER
                self._pixel_area[path] = 0

        viewer.set_inspection_images(normalized, pixmaps)
        if self._paths:
            self._loader.request(self._paths)
            self._timer.start()

    def request_detail(
        self,
        viewer: SynchronizedImageViewer | None = None,
        reason: str = "zoom",
    ) -> None:
        if viewer is not None and viewer is not self._viewer:
            return
        if not self._paths:
            return
        if reason == "actual_size":
            self._pending_actual_size = True
        paths_needing_detail = tuple(
            path
            for path in self._paths
            if self._quality.get(path) != InspectionQuality.DETAIL
        )
        if not paths_needing_detail:
            if self._pending_actual_size and self._viewer is not None:
                self._pending_actual_size = False
                self._viewer.zoom_to_actual_size()
            return
        self._timer.stop()
        if self._detail_requested:
            return
        self._detail_requested = True
        self._loader.request_details(paths_needing_detail)

    def refresh_paths(
        self,
        image_paths: Iterable[str],
    ) -> tuple[str, ...]:
        """Refresh active sources that changed without rebuilding the viewer.

        File mutations such as rotation keep the same path, so normal activation
        deduplication cannot distinguish the new source from the frame already on
        screen. Reset only the changed paths' quality and restart cancellable
        background loading while retaining the current layout, focus, and view.
        """

        if self._viewer is None or not self._paths:
            return ()

        active_path_set = set(self._paths)
        changed_paths = tuple(
            path
            for path in dict.fromkeys(path for path in image_paths if path)
            if path in active_path_set
        )
        if not changed_paths:
            return ()

        self._timer.stop()
        self._loader.cancel()
        self._loader.cancel_details()
        self._pending_detail_images.clear()
        self._detail_requested = False

        for path in changed_paths:
            self._quality[path] = InspectionQuality.PLACEHOLDER
            self._pixel_area[path] = 0
            try:
                pixmap, cached = self._pipeline.get_immediate_review_qpixmap(path)
            except Exception:
                logger.debug(
                    "Immediate inspection refresh failed: %s", path, exc_info=True
                )
                continue
            if pixmap is None or pixmap.isNull():
                continue
            if self._viewer.update_image_pixmap(
                path,
                pixmap,
                preserve_view=True,
            ):
                self._quality[path] = (
                    InspectionQuality.PREVIEW
                    if cached
                    else InspectionQuality.PLACEHOLDER
                )
                self._pixel_area[path] = pixmap.width() * pixmap.height()

        self._loader.request(changed_paths)
        self._timer.start()
        return changed_paths

    def clear(self, viewer: SynchronizedImageViewer | None = None) -> None:
        if viewer is not None and viewer is not self._viewer:
            return
        self._timer.stop()
        self._loader.cancel_details()
        self._viewer = None
        self._specs = ()
        self._paths = ()
        self._quality.clear()
        self._pixel_area.clear()
        self._pending_detail_images.clear()
        self._detail_requested = False
        self._pending_actual_size = False

    def reset(self) -> None:
        self.clear()
        self._loader.reset()

    def _on_preview_ready(self, path: str) -> None:
        if path not in self._paths or self._viewer is None:
            return
        if (
            self._quality.get(path, InspectionQuality.PLACEHOLDER)
            >= InspectionQuality.DETAIL
        ):
            return
        pixmap = self._pipeline.get_cached_preview_qpixmap(path, memory_only=True)
        if pixmap is None or pixmap.isNull():
            return
        if self._viewer.update_image_pixmap(
            path,
            pixmap,
            preserve_view=True,
            smooth_transition=True,
        ):
            self._quality[path] = InspectionQuality.PREVIEW
            self._pixel_area[path] = pixmap.width() * pixmap.height()

    def _on_preview_failed(self, path: str) -> None:
        # Retain the immediate frame; a failed upgrade must never blank it.
        return

    def _on_detail_ready(self, path: str, image) -> None:
        if path not in self._paths or self._viewer is None:
            return
        area = int(getattr(image, "width", 0)) * int(getattr(image, "height", 0))
        if area <= self._pixel_area.get(path, 0):
            return
        # Hold every decoded source until the worker finishes the complete visible
        # set. Presenting them from one UI event keeps comparisons visually in sync.
        self._pending_detail_images[path] = image

    def _on_detail_failed(self, path: str) -> None:
        if path in self._paths and self._viewer is not None:
            window = self._viewer.window()
            status_bar = getattr(window, "statusBar", lambda: None)()
            if status_bar is not None:
                status_bar.showMessage(
                    f"Could not load original detail for {os.path.basename(path)}; using the prepared preview.",
                    5000,
                )

    def _on_detail_batch_finished(self) -> None:
        if self._viewer is None:
            self._pending_detail_images.clear()
            return
        prepared: list[tuple[str, object, int]] = []
        for path in self._paths:
            image = self._pending_detail_images.get(path)
            if image is None:
                continue
            area = int(getattr(image, "width", 0)) * int(getattr(image, "height", 0))
            if area <= self._pixel_area.get(path, 0):
                continue
            try:
                pixmap = self._pipeline.qpixmap_from_pil(image)
            except Exception:
                self._on_detail_failed(path)
                continue
            prepared.append((path, pixmap, area))
        self._pending_detail_images.clear()

        # All pixmaps are prepared before any slot changes. Qt paints after this
        # handler returns, so every visible successful detail starts together.
        for path, pixmap, area in prepared:
            if self._viewer.update_image_pixmap(
                path,
                pixmap,
                preserve_view=True,
                smooth_transition=True,
            ):
                self._quality[path] = InspectionQuality.DETAIL
                self._pixel_area[path] = area
        self._detail_requested = False
        if self._pending_actual_size:
            self._pending_actual_size = False
            self._viewer.zoom_to_actual_size()

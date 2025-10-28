"""Controller for running the AI best shot picker across similarity clusters."""

from __future__ import annotations

import logging
from dataclasses import dataclass
from typing import List, Optional, TYPE_CHECKING

from PyQt6.QtCore import Qt, pyqtSignal, QItemSelectionModel
from PyQt6.QtWidgets import (
    QDialog,
    QHBoxLayout,
    QLabel,
    QListWidget,
    QListWidgetItem,
    QMessageBox,
    QPushButton,
    QVBoxLayout,
    QApplication,
)
from PyQt6.QtCore import QEventLoop

from core.ai.best_shot_picker import BestShotResult

if TYPE_CHECKING:
    from ui.main_window import MainWindow

logger = logging.getLogger(__name__)


class _ClusterBestShotProgressDialog(QDialog):
    """Progress dialog specialised for cluster-wide analysis."""

    cancelled = pyqtSignal()

    def __init__(self, parent=None):
        super().__init__(parent)
        self.setModal(True)
        self.setWindowTitle("AI Cluster Best Shots")
        self.setObjectName("aiClusterBestShotProgressDialog")
        self.setWindowFlags(self.windowFlags() | Qt.WindowType.FramelessWindowHint)
        self.setMinimumWidth(380)

        layout = QVBoxLayout(self)
        layout.setSpacing(16)
        layout.setContentsMargins(24, 24, 24, 24)

        title_label = QLabel("Analyzing Similarity Groups")
        title_label.setStyleSheet("font-weight: bold; font-size: 14px;")
        layout.addWidget(title_label)

        self.cluster_label = QLabel("Cluster 0 of 0")
        self.cluster_label.setStyleSheet("font-size: 12px; color: #4a4a4a;")
        layout.addWidget(self.cluster_label)

        self.status_label = QLabel("Preparing analysis...")
        self.status_label.setWordWrap(True)
        self.status_label.setStyleSheet("color: #767676; font-size: 11px;")
        layout.addWidget(self.status_label)

        layout.addStretch()

        button_row = QHBoxLayout()
        button_row.addStretch()
        self.cancel_button = QPushButton("Cancel Analysis")
        self.cancel_button.clicked.connect(self.cancelled.emit)
        button_row.addWidget(self.cancel_button)
        layout.addLayout(button_row)

    def update_cluster_position(self, current: int, total: int):
        self.cluster_label.setText(f"Cluster {current} of {total}")

    def update_status(self, message: str):
        if message:
            self.status_label.setText(message)

    def set_cancel_enabled(self, enabled: bool):
        self.cancel_button.setEnabled(enabled)


@dataclass
class ClusterBestShotSummaryItem:
    cluster_id: int
    result: BestShotResult
    image_paths: List[str]
    index: int
    total: int


class ClusterBestShotSummaryDialog(QDialog):
    """Displays the results for each cluster and offers quick navigation."""

    def __init__(
        self,
        items: List[ClusterBestShotSummaryItem],
        select_callback,
        parent=None,
    ):
        super().__init__(parent)
        self._items = items
        self._select_callback = select_callback
        self.setWindowTitle("AI Cluster Best Shots")
        self.setMinimumWidth(500)

        layout = QVBoxLayout(self)
        layout.setSpacing(14)
        layout.setContentsMargins(20, 20, 20, 20)

        heading = QLabel(
            f"Analyzed {len(items)} cluster{'s' if len(items) != 1 else ''}."
        )
        heading.setStyleSheet("font-weight: bold; font-size: 13px;")
        layout.addWidget(heading)

        self.list_widget = QListWidget()
        self.list_widget.setSelectionMode(QListWidget.SelectionMode.ExtendedSelection)
        self.list_widget.itemActivated.connect(self._handle_item_activated)
        layout.addWidget(self.list_widget)

        for item in items:
            filename = item.result.best_image_path.split("/")[-1]
            text = (
                f"Group {item.cluster_id}: {filename} — Confidence {item.result.confidence}"
            )
            list_item = QListWidgetItem(text)
            list_item.setToolTip(item.result.reasoning)
            list_item.setData(Qt.ItemDataRole.UserRole, item.result.best_image_path)
            self.list_widget.addItem(list_item)

        button_row = QHBoxLayout()
        self.select_button = QPushButton("Select Winners in Viewer")
        self.select_button.clicked.connect(self._select_all)
        button_row.addWidget(self.select_button)

        close_button = QPushButton("Close")
        close_button.clicked.connect(self.accept)
        button_row.addWidget(close_button)

        layout.addLayout(button_row)

    def _select_all(self):
        paths = [
            item.result.best_image_path
            for item in self._items
            if item.result.best_image_path
        ]
        self._select_callback(paths)

    def _handle_item_activated(self, list_item: QListWidgetItem):
        path = list_item.data(Qt.ItemDataRole.UserRole)
        if path:
            self._select_callback([path])


class ClusterBestShotController:
    """Coordinates running the AI best shot picker across similarity clusters."""

    def __init__(self, main_window: "MainWindow"):
        self.main_window = main_window
        self.worker_manager = main_window.worker_manager
        self.progress_dialog: Optional[_ClusterBestShotProgressDialog] = None
        self._results: List[ClusterBestShotSummaryItem] = []
        self._total_clusters = 0

        self.worker_manager.best_shot_clusters_progress.connect(self._on_progress)
        self.worker_manager.best_shot_clusters_result.connect(self._on_cluster_result)
        self.worker_manager.best_shot_clusters_error.connect(self._on_error)
        self.worker_manager.best_shot_clusters_finished.connect(self._on_finished)

    def start_analysis(self):
        if self.worker_manager.is_best_shot_clusters_running():
            QMessageBox.information(
                self.main_window,
                "Analysis In Progress",
                "AI cluster best shot analysis is already running.",
            )
            return

        cluster_payloads = self._build_cluster_input()
        if not cluster_payloads:
            QMessageBox.information(
                self.main_window,
                "No Similarity Groups",
                "Similarity analysis has not been run or produced no groups.",
            )
            return

        self.main_window.update_best_shot_labels([], replace=True)
        self._results.clear()
        self._total_clusters = len(cluster_payloads)
        self._show_progress_dialog()
        self._set_action_enabled(False)

        try:
            self.worker_manager.start_best_shot_clusters(cluster_payloads)
        except ValueError as exc:
            logger.error("Failed to start cluster best shot analysis: %s", exc)
            self._close_progress_dialog()
            self._set_action_enabled(True)
            QMessageBox.critical(self.main_window, "Analysis Failed", str(exc))

    def _build_cluster_input(self) -> List[tuple[int, List[str]]]:
        cluster_results = getattr(self.main_window.app_state, "cluster_results", {})
        if not cluster_results:
            return []

        sort_mode = self.main_window.cluster_sort_combo.currentText()
        cluster_info = self.main_window.similarity_controller.prepare_clusters(
            sort_mode
        )
        images_by_cluster = cluster_info.get("images_by_cluster", {})
        sorted_cluster_ids = cluster_info.get("sorted_cluster_ids", [])

        cluster_payloads: List[tuple[int, List[str]]] = []
        for cluster_id in sorted_cluster_ids:
            file_data_list = images_by_cluster.get(cluster_id, [])
            image_paths = [
                file_data.get("path")
                for file_data in file_data_list
                if isinstance(file_data, dict) and file_data.get("path")
            ]
            if image_paths:
                cluster_payloads.append((cluster_id, image_paths))
        return cluster_payloads

    def _show_progress_dialog(self):
        self._close_progress_dialog()
        self.progress_dialog = _ClusterBestShotProgressDialog(self.main_window)
        self.progress_dialog.cancelled.connect(self._on_cancel)
        self.progress_dialog.update_cluster_position(0, self._total_clusters)
        self.progress_dialog.update_status("Connecting to AI service...")
        self.progress_dialog.show()
        self._process_ui_events()

    def _on_progress(self, current: int, total: int, message: str):
        if self.progress_dialog:
            self.progress_dialog.update_cluster_position(current, total)
            self.progress_dialog.update_status(message)
            self._process_ui_events()

    def _on_cluster_result(self, payload: object):
        if not isinstance(payload, dict):
            return
        result_obj = payload.get("result")
        if not isinstance(result_obj, BestShotResult):
            return
        summary_item = ClusterBestShotSummaryItem(
            cluster_id=payload.get("cluster_id"),
            result=result_obj,
            image_paths=payload.get("image_paths", []),
            index=payload.get("index", 0),
            total=payload.get("total", self._total_clusters),
        )
        self._results.append(summary_item)
        if self.progress_dialog:
            filename = result_obj.best_image_path.split("/")[-1]
            self.progress_dialog.update_status(
                f"Selected {filename} for group {summary_item.cluster_id}"
            )
            self._process_ui_events()
        if result_obj.best_image_path:
            self.main_window.update_best_shot_labels(
                [result_obj.best_image_path], replace=False
            )

    def _on_error(self, message: str):
        logger.error("Cluster best shot analysis error: %s", message)
        self._close_progress_dialog()
        self._set_action_enabled(True)
        QMessageBox.critical(
            self.main_window,
            "Analysis Failed",
            f"Failed to analyze clusters:\n\n{message}",
        )

    def _on_finished(self, success: bool, summary: object):
        logger.info("Cluster best shot analysis finished (success: %s)", success)
        self._set_action_enabled(True)
        self._close_progress_dialog()
        if not success and not self._results:
            return

        dialog = ClusterBestShotSummaryDialog(self._results, self._select_paths_in_ui)
        dialog.exec()

    def _on_cancel(self):
        logger.info("User requested cluster best shot cancellation")
        self.worker_manager.stop_best_shot_clusters()
        if self.progress_dialog:
            self.progress_dialog.set_cancel_enabled(False)
            self.progress_dialog.update_status("Cancelling analysis...")
        self.main_window.statusBar().showMessage(
            "Cancelling AI cluster best shot analysis...",
            3000,
        )
        self._process_ui_events()

    def cleanup(self):
        self.worker_manager.stop_best_shot_clusters()
        self._close_progress_dialog()
        self._set_action_enabled(True)
        self._results.clear()

    def _close_progress_dialog(self):
        if self.progress_dialog:
            try:
                self.progress_dialog.close()
            finally:
                self.progress_dialog.deleteLater()
                self.progress_dialog = None

    def _set_action_enabled(self, enabled: bool):
        try:
            action = self.main_window.menu_manager.pick_best_shots_for_clusters_action
            action.setEnabled(enabled)
        except Exception:  # pragma: no cover - defensive
            logger.debug("Failed to toggle cluster best shot action", exc_info=True)

    def _process_ui_events(self):
        try:
            QApplication.processEvents(
                QEventLoop.ProcessEventsFlag.ExcludeUserInputEvents
                | QEventLoop.ProcessEventsFlag.ExcludeSocketNotifiers
            )
        except Exception:
            logger.debug("Failed to pump UI events", exc_info=True)

    def _select_paths_in_ui(self, paths: List[str]):
        if not paths:
            return
        view = self.main_window._get_active_file_view()
        if not view:
            return
        selection_model = view.selectionModel()
        if not selection_model:
            return

        selection_model.clearSelection()
        first_proxy = None
        for path in paths:
            proxy_index = self.main_window._find_proxy_index_for_path(path)
            if proxy_index and proxy_index.isValid():
                selection_model.select(
                    proxy_index,
                    QItemSelectionModel.SelectionFlag.Select
                    | QItemSelectionModel.SelectionFlag.Rows,
                )
                if first_proxy is None:
                    first_proxy = proxy_index
        if first_proxy:
            view.scrollTo(first_proxy)
        self.main_window.statusBar().showMessage(
            f"Selected {len(paths)} best image{'s' if len(paths) != 1 else ''}.",
            4000,
        )

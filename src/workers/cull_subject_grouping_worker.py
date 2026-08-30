"""Background orchestration for high-precision Cull same-subject grouping."""

from __future__ import annotations

from datetime import datetime
import logging

from PyQt6.QtCore import QObject, pyqtSignal

from core.app_settings import CullGroupingStrictness
from core.model_download import ModelDownloadCancelled
from core.caching.analysis_cache import MANUAL_OVERRIDE_NAMESPACE_CULL
from core.subject_grouping import (
    CullClusteringResult,
    PairVerification,
    SUBJECT_GROUPING_PIPELINE_VERSION,
    SubjectArtifact,
    SubjectGroupingCancelled,
    SubjectGroupingService,
    build_cull_grouping_signature,
    build_cull_pair_context_signature,
)
from core.subject_grouping_models import (
    ARTIFACT_CHECKPOINT_INTERVAL,
    HighAccuracySubjectModels,
    resolve_subject_model_snapshots,
    subject_model_signature,
)

logger = logging.getLogger(__name__)


def _pair_cache_key(first: str, second: str) -> str:
    left, right = (first, second) if first < second else (second, first)
    return f"{left}\0{right}"


def _parse_pair_cache_key(value: str) -> tuple[str, str] | None:
    parts = value.split("\0", 1)
    return (parts[0], parts[1]) if len(parts) == 2 and all(parts) else None


class CullSubjectGroupingWorker(QObject):
    progress_update = pyqtSignal(int, str)
    completed = pyqtSignal(object)
    error = pyqtSignal(str)
    finished = pyqtSignal()

    def __init__(
        self,
        *,
        paths: list[str],
        fingerprints: dict[str, tuple[int, int]],
        timestamps: dict[str, datetime | None],
        strictness: CullGroupingStrictness,
        image_pipeline,
        analysis_cache,
        folder_path: str,
        allow_model_download: bool,
        parent: QObject | None = None,
    ) -> None:
        super().__init__(parent)
        self.paths = list(paths)
        self.fingerprints = dict(fingerprints)
        self.timestamps = dict(timestamps)
        self.strictness = strictness
        self.image_pipeline = image_pipeline
        self.analysis_cache = analysis_cache
        self.folder_path = folder_path
        self.allow_model_download = allow_model_download
        self._should_stop = False
        self._models: HighAccuracySubjectModels | None = None

    def stop(self) -> None:
        self._should_stop = True

    def run(self) -> None:
        try:
            self._run()
        except (SubjectGroupingCancelled, ModelDownloadCancelled):
            logger.info("Cull same-subject grouping cancelled.")
        except Exception as exc:
            logger.error("Cull same-subject grouping failed", exc_info=True)
            self.error.emit(str(exc))
        finally:
            if self._models is not None:
                self._models.close()
            self.finished.emit()

    def _run(self) -> None:
        def model_progress(percent: int, message: str) -> None:
            if self._should_stop:
                raise SubjectGroupingCancelled
            mapped = -1 if percent < 0 else int(percent * 0.10)
            self.progress_update.emit(mapped, message)

        snapshots = resolve_subject_model_snapshots(
            allow_download=self.allow_model_download,
            progress_callback=model_progress,
            should_cancel=lambda: self._should_stop,
        )
        model_signature = subject_model_signature(snapshots)
        pair_context_signature = build_cull_pair_context_signature(
            self.fingerprints,
            timestamps=self.timestamps,
        )
        grouping_signature = build_cull_grouping_signature(
            self.fingerprints,
            timestamps=self.timestamps,
            model_signature=model_signature,
            strictness=self.strictness,
        )
        cached_state = (
            self.analysis_cache.load_cull_grouping_state(self.folder_path)
            if self.analysis_cache is not None
            else {}
        )
        cached_clusters = cached_state.get("cull_cluster_results")
        overrides = (
            self.analysis_cache.get_manual_overrides(
                self.folder_path, namespace=MANUAL_OVERRIDE_NAMESPACE_CULL
            )
            if self.analysis_cache is not None
            else {}
        )
        if (
            cached_state.get("cull_grouping_signature") == grouping_signature
            and isinstance(cached_clusters, dict)
            and set(cached_clusters) == set(self.paths)
        ):
            clusters = {path: int(value) for path, value in cached_clusters.items()}
            for path, cluster_id in overrides.items():
                if path in clusters:
                    clusters[path] = int(cluster_id)
            self.completed.emit(
                CullClusteringResult(
                    clusters=clusters, signature=grouping_signature, reused=True
                )
            )
            return

        artifacts: dict[str, SubjectArtifact] = {}
        raw_artifacts = cached_state.get("cull_subject_artifacts")
        if cached_state.get("cull_model_signature") == model_signature and isinstance(
            raw_artifacts, dict
        ):
            for path, value in raw_artifacts.items():
                parsed = SubjectArtifact.from_dict(value)
                if (
                    parsed is not None
                    and path in self.fingerprints
                    and parsed.fingerprint == self.fingerprints[path]
                    and parsed.model_signature == model_signature
                ):
                    artifacts[path] = parsed

        pairs: dict[tuple[str, str], PairVerification] = {}
        raw_pairs = cached_state.get("cull_pair_verifications")
        if (
            cached_state.get("cull_model_signature") == model_signature
            and cached_state.get("cull_pair_pipeline_version")
            == SUBJECT_GROUPING_PIPELINE_VERSION
            and cached_state.get("cull_pair_context_signature")
            == pair_context_signature
            and isinstance(raw_pairs, dict)
        ):
            for key, value in raw_pairs.items():
                parsed_key = _parse_pair_cache_key(str(key))
                parsed_pair = PairVerification.from_dict(value)
                if (
                    parsed_key is not None
                    and parsed_pair is not None
                    and parsed_key[0] in artifacts
                    and parsed_key[1] in artifacts
                ):
                    pairs[parsed_key] = parsed_pair

        self._models = HighAccuracySubjectModels(
            snapshots=snapshots,
            image_pipeline=self.image_pipeline,
        )
        models = self._models
        missing_paths = [path for path in self.paths if path not in artifacts]
        pending_checkpoint: dict[str, object] = {}

        def flush_checkpoint() -> None:
            if self.analysis_cache is None or not pending_checkpoint:
                return
            self.analysis_cache.merge_cull_artifacts_checkpoint(
                self.folder_path,
                artifacts=dict(pending_checkpoint),
                model_signature=model_signature,
            )
            pending_checkpoint.clear()

        def checkpoint_artifact(artifact: SubjectArtifact) -> None:
            artifacts[artifact.path] = artifact
            pending_checkpoint[artifact.path] = artifact.to_dict()
            if len(pending_checkpoint) >= ARTIFACT_CHECKPOINT_INTERVAL:
                flush_checkpoint()

        if missing_paths:
            models.extract_artifacts(
                missing_paths,
                self.fingerprints,
                should_cancel=lambda: self._should_stop,
                progress_callback=lambda current, total: self.progress_update.emit(
                    10 + int(50 * current / max(1, total)),
                    f"Encoding DINO features ({current}/{total})",
                ),
                artifact_callback=checkpoint_artifact,
            )
            flush_checkpoint()
        service = SubjectGroupingService(
            artifact_provider=lambda path: models.extract_artifact(
                path, self.fingerprints[path]
            ),
            geometry_provider=models.verify_geometry,
            should_cancel=lambda: self._should_stop,
            progress_callback=lambda percent, message: self.progress_update.emit(
                10 + int(percent * 0.90), message
            ),
        )
        result, artifacts, pairs = service.group(
            self.paths,
            fingerprints=self.fingerprints,
            timestamps=self.timestamps,
            strictness=self.strictness,
            model_signature=model_signature,
            cached_artifacts=artifacts,
            cached_pairs=pairs,
        )
        if self._should_stop:
            raise SubjectGroupingCancelled

        if self.analysis_cache is not None:
            automatic_clusters = dict(result.clusters)
            self.analysis_cache.save_cull_grouping_state(
                self.folder_path,
                artifacts={
                    path: artifact.to_dict() for path, artifact in artifacts.items()
                },
                pair_verifications={
                    _pair_cache_key(*key): verification.to_dict()
                    for key, verification in pairs.items()
                },
                clusters=automatic_clusters,
                grouping_signature=result.signature,
                model_signature=model_signature,
                pair_pipeline_version=SUBJECT_GROUPING_PIPELINE_VERSION,
                pair_context_signature=pair_context_signature,
            )
            for path, cluster_id in overrides.items():
                if path in result.clusters:
                    result.clusters[path] = int(cluster_id)
        self.completed.emit(result)

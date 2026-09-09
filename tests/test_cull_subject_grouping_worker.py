import pyexiv2  # noqa: F401  # Must be first to avoid Windows crash

from unittest.mock import Mock

import pytest

from core.app_settings import CullGroupingStrictness
from core.caching.analysis_cache import MANUAL_OVERRIDE_NAMESPACE_CULL
from core.subject_grouping import (
    SUBJECT_GROUPING_PIPELINE_VERSION,
    GeometryEvidence,
    PairVerification,
    SubjectArtifact,
    SubjectEvidence,
)
from workers import cull_subject_grouping_worker as worker_module
from workers.cull_subject_grouping_worker import CullSubjectGroupingWorker


def _artifact(path: str) -> SubjectArtifact:
    return SubjectArtifact(
        path=path,
        fingerprint=(10, 20),
        model_signature="signature",
        global_descriptor=(1.0, 0.0),
        subjects=(SubjectEvidence((1.0, 0.0), (0.1, 0.1, 0.8, 0.8), 0.49, "object"),),
        faces=(),
    )


class _Models:
    """Stand-in for the DINO adapter that records the paths it encoded."""

    def __init__(self, **_kwargs):
        self.encoded: list[str] = []

    def extract_artifacts(
        self,
        paths,
        _fingerprints,
        *,
        should_cancel,
        progress_callback,
        artifact_callback,
    ):
        for index, path in enumerate(paths, start=1):
            if should_cancel():
                return
            self.encoded.append(path)
            progress_callback(index, len(paths))
            artifact_callback(_artifact(path))

    def extract_artifact(self, path, _fingerprint):
        self.encoded.append(path)
        return _artifact(path)

    def verify_geometry(self, _first, _second):
        from core.subject_grouping import GeometryEvidence

        return GeometryEvidence()

    def close(self):
        pass


@pytest.fixture
def patched_models(monkeypatch):
    created: list[_Models] = []

    def _factory(**kwargs):
        models = _Models(**kwargs)
        created.append(models)
        return models

    monkeypatch.setattr(
        worker_module, "resolve_subject_model_snapshots", lambda **_kwargs: {}
    )
    monkeypatch.setattr(
        worker_module, "subject_model_signature", lambda _s: "signature"
    )
    monkeypatch.setattr(worker_module, "HighAccuracySubjectModels", _factory)
    monkeypatch.setattr(worker_module, "ARTIFACT_CHECKPOINT_INTERVAL", 2)
    return created


def _worker(cache, paths):
    return CullSubjectGroupingWorker(
        paths=paths,
        fingerprints={path: (10, 20) for path in paths},
        timestamps={path: None for path in paths},
        strictness=CullGroupingStrictness.CONSERVATIVE,
        image_pipeline=object(),
        analysis_cache=cache,
        folder_path="/photos",
        allow_model_download=False,
    )


def test_checkpoints_persist_only_newly_encoded_artifacts(patched_models):
    cache = Mock()
    cache.load_cull_grouping_state.return_value = {}
    cache.get_manual_overrides.return_value = {}
    paths = [f"/photos/p{index}.jpg" for index in range(5)]

    _worker(cache, paths).run()

    checkpoints = [
        call.kwargs["artifacts"]
        for call in cache.merge_cull_artifacts_checkpoint.call_args_list
    ]
    assert checkpoints, "extraction must checkpoint progress"
    # Each checkpoint carries only the batch since the previous flush, never the
    # whole accumulated set, so cost stays linear in the number of new photos.
    assert all(len(batch) <= 2 for batch in checkpoints)
    assert sum(len(batch) for batch in checkpoints) == len(paths)


def test_worker_reads_overrides_from_the_cull_namespace(patched_models):
    cache = Mock()
    cache.load_cull_grouping_state.return_value = {}
    cache.get_manual_overrides.return_value = {}

    _worker(cache, ["/photos/a.jpg"]).run()

    cache.get_manual_overrides.assert_called_once_with(
        "/photos", namespace=MANUAL_OVERRIDE_NAMESPACE_CULL
    )


def test_cached_artifacts_are_not_re_encoded(patched_models):
    paths = ["/photos/a.jpg", "/photos/b.jpg"]
    cache = Mock()
    cache.load_cull_grouping_state.return_value = {
        "cull_model_signature": "signature",
        "cull_subject_artifacts": {path: _artifact(path).to_dict() for path in paths},
    }
    cache.get_manual_overrides.return_value = {}

    _worker(cache, paths).run()

    assert patched_models[0].encoded == []
    cache.merge_cull_artifacts_checkpoint.assert_not_called()


def test_changed_timestamp_context_discards_cached_pair_evidence(patched_models):
    paths = ["/photos/a.jpg", "/photos/b.jpg"]
    stale_pair = PairVerification(
        path_a=paths[0],
        path_b=paths[1],
        subject_similarity=0.0,
        face_similarity=None,
        scene_similarity=0.0,
        subject_set_complete=True,
        geometry=GeometryEvidence(evaluated=True),
        time_delta_seconds=999.0,
        confidence=0.0,
        accepted=False,
        reason="stale",
    )
    cache = Mock()
    cache.load_cull_grouping_state.return_value = {
        "cull_model_signature": "signature",
        "cull_subject_artifacts": {path: _artifact(path).to_dict() for path in paths},
        "cull_pair_pipeline_version": SUBJECT_GROUPING_PIPELINE_VERSION,
        "cull_pair_context_signature": "outdated-timestamps",
        "cull_pair_verifications": {f"{paths[0]}\0{paths[1]}": stale_pair.to_dict()},
    }
    cache.get_manual_overrides.return_value = {}

    _worker(cache, paths).run()

    saved_pairs = cache.save_cull_grouping_state.call_args.kwargs["pair_verifications"]
    assert saved_pairs[f"{paths[0]}\0{paths[1]}"]["accepted"] is True
    assert saved_pairs[f"{paths[0]}\0{paths[1]}"]["reason"] != "stale"

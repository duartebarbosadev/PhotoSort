import pyexiv2  # noqa: F401  # Must be first to avoid Windows crash

from datetime import datetime, timedelta
from types import SimpleNamespace

import numpy as np
import pytest
from PIL import Image

from core.app_settings import CullGroupingStrictness
from core.subject_grouping import (
    GeometryEvidence,
    PairVerification,
    SubjectArtifact,
    SubjectGroupingCancelled,
    SubjectGroupingService,
    SubjectEvidence,
    build_cull_grouping_signature,
    build_cull_pair_context_signature,
    complete_link_clusters,
    generate_candidate_pairs,
    verify_pair,
)
from core.subject_grouping_models import (
    HighAccuracySubjectModels,
    _dense_grid_evidence,
)


def _subject(values=(1.0, 0.0), *, kind="object"):
    return SubjectEvidence(tuple(values), (0.1, 0.1, 0.8, 0.8), 0.49, kind)


def _artifact(path, *, subjects=None, faces=(), global_values=(1.0, 0.0)):
    return SubjectArtifact(
        path=path,
        fingerprint=(10, 20),
        model_signature="models-v1",
        global_descriptor=tuple(global_values),
        subjects=tuple(subjects if subjects is not None else [_subject()]),
        faces=tuple(faces),
    )


def _verification(first, second, confidence=0.99, accepted=True):
    return PairVerification(
        path_a=first,
        path_b=second,
        subject_similarity=0.99,
        face_similarity=None,
        scene_similarity=0.99,
        subject_set_complete=True,
        geometry=GeometryEvidence(30, 0.8, 0.4, 0.4),
        time_delta_seconds=None,
        confidence=confidence,
        accepted=accepted,
        reason="same_subject" if accepted else "subject_identity_uncertain",
    )


def test_complete_link_does_not_admit_similarity_bridge():
    pairs = {
        ("a", "b"): _verification("a", "b", 0.99),
        ("b", "c"): _verification("b", "c", 0.98),
        ("a", "c"): _verification("a", "c", 0.40, accepted=False),
    }

    clusters = complete_link_clusters(["a", "b", "c"], pairs)

    assert clusters["a"] == clusters["b"]
    assert clusters["c"] != clusters["a"]


def test_cull_cache_signatures_change_when_capture_timestamps_change():
    fingerprints = {"a.jpg": (10, 20), "b.jpg": (30, 40)}
    initial = {
        "a.jpg": datetime(2026, 1, 1, 10, 0),
        "b.jpg": datetime(2026, 1, 1, 10, 1),
    }
    updated = {**initial, "b.jpg": datetime(2026, 1, 2, 10, 1)}

    initial_pairs = build_cull_pair_context_signature(fingerprints, timestamps=initial)
    updated_pairs = build_cull_pair_context_signature(fingerprints, timestamps=updated)
    initial_groups = build_cull_grouping_signature(
        fingerprints,
        timestamps=initial,
        model_signature="models-v1",
        strictness=CullGroupingStrictness.CONSERVATIVE,
    )
    updated_groups = build_cull_grouping_signature(
        fingerprints,
        timestamps=updated,
        model_signature="models-v1",
        strictness=CullGroupingStrictness.CONSERVATIVE,
    )

    assert updated_pairs != initial_pairs
    assert updated_groups != initial_groups


def test_complete_link_requires_every_cross_pair_to_exist():
    pairs = {
        ("a", "b"): _verification("a", "b"),
        ("b", "c"): _verification("b", "c"),
    }

    clusters = complete_link_clusters(["a", "b", "c"], pairs)

    assert len(set(clusters.values())) == 2


def test_complete_link_ids_are_deterministic_for_input_order():
    pairs = {
        ("a", "b"): _verification("a", "b"),
        ("c", "d"): _verification("c", "d"),
    }

    assert complete_link_clusters(["d", "b", "c", "a"], pairs) == (
        complete_link_clusters(["a", "b", "c", "d"], pairs)
    )


def test_pair_rejects_added_meaningful_subject():
    first = _artifact("a", subjects=[_subject()])
    second = _artifact("b", subjects=[_subject(), _subject((0.0, 1.0))])

    result = verify_pair(
        first,
        second,
        geometry=GeometryEvidence(40, 0.8, 0.4, 0.4),
        timestamp_a=None,
        timestamp_b=None,
        strictness=CullGroupingStrictness.CONSERVATIVE,
    )

    assert result.accepted is False
    assert result.reason == "subject_set_changed"


def test_pair_rejects_changed_face_set():
    first = _artifact("a", faces=[_subject(kind="face")])
    second = _artifact("b", faces=[])

    result = verify_pair(
        first,
        second,
        geometry=GeometryEvidence(40, 0.8, 0.4, 0.4),
        timestamp_a=None,
        timestamp_b=None,
        strictness=CullGroupingStrictness.CONSERVATIVE,
    )

    assert result.accepted is False
    assert result.reason == "subject_set_changed"


def test_similar_background_cannot_override_different_foreground_subject():
    first = _artifact("a", subjects=[_subject((1.0, 0.0))])
    second = _artifact("b", subjects=[_subject((0.0, 1.0))], global_values=(1.0, 0.0))

    result = verify_pair(
        first,
        second,
        geometry=GeometryEvidence(100, 0.9, 0.8, 0.8),
        timestamp_a=datetime(2026, 1, 1),
        timestamp_b=datetime(2026, 1, 1),
        strictness=CullGroupingStrictness.CONSERVATIVE,
    )

    assert result.accepted is False
    assert result.reason == "subject_identity_uncertain"


def test_strong_geometry_rescues_plausible_regional_dino_variation():
    first = _artifact("a", subjects=[_subject((1.0, 0.0))])
    second = _artifact(
        "b",
        subjects=[_subject((0.68, 0.733212))],
        global_values=(0.88, 0.474974),
    )

    without_geometry = verify_pair(
        first,
        second,
        geometry=GeometryEvidence(),
        timestamp_a=None,
        timestamp_b=None,
        strictness=CullGroupingStrictness.CONSERVATIVE,
    )
    with_geometry = verify_pair(
        first,
        second,
        geometry=GeometryEvidence(15, 1.0, 0.56, 0.56, evaluated=True),
        timestamp_a=None,
        timestamp_b=None,
        strictness=CullGroupingStrictness.CONSERVATIVE,
    )

    assert without_geometry.accepted is False
    assert with_geometry.accepted is True
    assert with_geometry.reason == "same_subject"


def test_geometry_rescue_cannot_override_changed_face_identity():
    first = _artifact("a", faces=[_subject((1.0, 0.0), kind="face")])
    second = _artifact("b", faces=[_subject((0.0, 1.0), kind="face")])

    result = verify_pair(
        first,
        second,
        geometry=GeometryEvidence(16, 1.0, 0.56, 0.56, evaluated=True),
        timestamp_a=None,
        timestamp_b=None,
        strictness=CullGroupingStrictness.CONSERVATIVE,
    )

    assert result.accepted is False
    assert result.reason == "face_identity_uncertain"


def test_cached_borderline_pair_runs_geometry_before_rejection():
    artifacts = {
        "a": _artifact("a", subjects=[_subject((1.0, 0.0))]),
        "b": _artifact(
            "b",
            subjects=[_subject((0.68, 0.733212))],
            global_values=(0.88, 0.474974),
        ),
    }
    cached_pair = verify_pair(
        artifacts["a"],
        artifacts["b"],
        geometry=GeometryEvidence(),
        timestamp_a=None,
        timestamp_b=None,
        strictness=CullGroupingStrictness.CONSERVATIVE,
    )
    geometry_calls = []
    service = SubjectGroupingService(
        artifact_provider=lambda path: artifacts[path],
        geometry_provider=lambda first, second: (
            geometry_calls.append((first.path, second.path))
            or GeometryEvidence(15, 1.0, 0.56, 0.56, evaluated=True)
        ),
    )

    result, _artifacts, pairs = service.group(
        ["a", "b"],
        fingerprints={"a": (10, 20), "b": (10, 20)},
        timestamps={},
        strictness=CullGroupingStrictness.CONSERVATIVE,
        model_signature="models-v1",
        cached_artifacts=artifacts,
        cached_pairs={("a", "b"): cached_pair},
    )

    assert geometry_calls == [("a", "b")]
    assert pairs[("a", "b")].accepted is True
    assert result.clusters["a"] == result.clusters["b"]


def test_same_subject_can_group_far_apart_in_time():
    first = _artifact("a")
    second = _artifact("b")

    result = verify_pair(
        first,
        second,
        geometry=GeometryEvidence(40, 0.8, 0.4, 0.4),
        timestamp_a=datetime(2020, 1, 1),
        timestamp_b=datetime(2026, 1, 1),
        strictness=CullGroupingStrictness.CONSERVATIVE,
    )

    assert result.accepted is True
    assert result.time_delta_seconds > 10 * 60


def test_time_candidates_are_limited_to_ten_minutes():
    artifacts = {
        "a": _artifact("a", global_values=(1.0, 0.0)),
        "b": _artifact("b", global_values=(0.0, 1.0)),
        "c": _artifact("c", global_values=(-1.0, 0.0)),
    }
    started = datetime(2026, 1, 1, 12, 0)
    timestamps = {
        "a": started,
        "b": started + timedelta(minutes=9),
        "c": started + timedelta(minutes=20),
    }

    pairs = generate_candidate_pairs(artifacts, timestamps)

    # Small libraries are all semantic candidates; exercise the temporal boundary
    # independently by verifying the timestamp evidence itself.
    result = verify_pair(
        artifacts["a"],
        artifacts["b"],
        geometry=GeometryEvidence(),
        timestamp_a=timestamps["a"],
        timestamp_b=timestamps["b"],
        strictness=CullGroupingStrictness.CONSERVATIVE,
    )
    far = verify_pair(
        artifacts["a"],
        artifacts["c"],
        geometry=GeometryEvidence(),
        timestamp_a=timestamps["a"],
        timestamp_b=timestamps["c"],
        strictness=CullGroupingStrictness.CONSERVATIVE,
    )
    assert pairs
    assert result.time_delta_seconds == 9 * 60
    assert far.time_delta_seconds == 20 * 60
    assert result.confidence > far.confidence


def test_dense_grid_preserves_complete_spatial_subject_set():
    patches = np.eye(16, dtype=np.float32)

    evidence = _dense_grid_evidence(patches, grid_size=4)

    assert len(evidence) == 16
    assert evidence[0].bbox == (0.0, 0.0, 0.25, 0.25)
    assert evidence[-1].bbox == (0.75, 0.75, 1.0, 1.0)
    assert all(item.kind == "region" for item in evidence)


def test_cached_artifacts_and_pair_evidence_are_reused_across_presets():
    artifacts_by_path = {"a": _artifact("a"), "b": _artifact("b")}
    artifact_calls = []
    geometry_calls = []
    service = SubjectGroupingService(
        artifact_provider=lambda path: (
            artifact_calls.append(path) or artifacts_by_path[path]
        ),
        geometry_provider=lambda first, second: (
            geometry_calls.append((first.path, second.path)) or GeometryEvidence()
        ),
    )
    fingerprints = {"a": (10, 20), "b": (10, 20)}

    _result, artifacts, pairs = service.group(
        ["a", "b"],
        fingerprints=fingerprints,
        timestamps={},
        strictness=CullGroupingStrictness.CONSERVATIVE,
        model_signature="models-v1",
    )
    service.group(
        ["a", "b"],
        fingerprints=fingerprints,
        timestamps={},
        strictness=CullGroupingStrictness.BROAD,
        model_signature="models-v1",
        cached_artifacts=artifacts,
        cached_pairs=pairs,
    )

    assert artifact_calls == ["a", "b"]
    assert geometry_calls == []


def test_temporal_candidates_scan_only_the_time_window():
    """The near-time pass must stay windowed instead of comparing every pair."""

    started = datetime(2026, 1, 1, 12, 0)
    count = 40
    artifacts = {
        f"p{index:02d}": _artifact(
            f"p{index:02d}", global_values=(float(index), float(-index))
        )
        for index in range(count)
    }
    # Each photo is an hour apart, so no photo has a temporal neighbour at all.
    timestamps = {
        f"p{index:02d}": started + timedelta(hours=index) for index in range(count)
    }

    windowed = generate_candidate_pairs(artifacts, timestamps)
    without_time = generate_candidate_pairs(artifacts, {})

    assert windowed == without_time

    # Two photos taken seconds apart are still paired by the temporal pass.
    close_timestamps = dict(timestamps)
    close_timestamps["p39"] = timestamps["p00"] + timedelta(seconds=5)
    close = generate_candidate_pairs(artifacts, close_timestamps)
    assert ("p00", "p39") in close


def test_candidate_generation_is_cancellable():
    artifacts = {f"p{index}": _artifact(f"p{index}") for index in range(5)}

    with pytest.raises(SubjectGroupingCancelled):
        generate_candidate_pairs(artifacts, {}, should_cancel=lambda: True)


def test_reextracted_photo_invalidates_its_cached_pair_evidence():
    """Cached evidence describes old pixels; a changed file must be re-verified."""

    artifacts_by_path = {"a": _artifact("a"), "b": _artifact("b")}
    service = SubjectGroupingService(
        artifact_provider=lambda path: artifacts_by_path[path],
        geometry_provider=lambda _first, _second: GeometryEvidence(),
    )
    stale_pair = _verification("a", "b", confidence=0.99, accepted=True)

    _result, _artifacts, pairs = service.group(
        ["a", "b"],
        # "a" no longer matches its cached fingerprint, forcing re-extraction.
        fingerprints={"a": (11, 21), "b": (10, 20)},
        timestamps={},
        strictness=CullGroupingStrictness.CONSERVATIVE,
        model_signature="models-v1",
        cached_artifacts={"a": _artifact("a"), "b": _artifact("b")},
        cached_pairs={("a", "b"): stale_pair},
    )

    assert pairs[("a", "b")] is not stale_pair


def test_grouping_cancellation_never_returns_partial_results():
    calls = 0

    def cancelled():
        nonlocal calls
        calls += 1
        return calls >= 2

    service = SubjectGroupingService(
        artifact_provider=lambda path: _artifact(path),
        geometry_provider=lambda _first, _second: GeometryEvidence(),
        should_cancel=cancelled,
    )

    with pytest.raises(SubjectGroupingCancelled):
        service.group(
            ["a", "b"],
            fingerprints={"a": (10, 20), "b": (10, 20)},
            timestamps={},
            strictness=CullGroupingStrictness.CONSERVATIVE,
            model_signature="models-v1",
        )


def test_dino_regions_provide_geometry_without_another_model():
    models = HighAccuracySubjectModels.__new__(HighAccuracySubjectModels)
    regions = _dense_grid_evidence(np.eye(16, dtype=np.float32), grid_size=4)
    first = _artifact("a", subjects=regions, global_values=(1.0, 0.0))
    second = _artifact("b", subjects=regions, global_values=(1.0, 0.0))

    geometry = models.verify_geometry(first, second)

    assert geometry.strong
    assert geometry.inlier_count == 16


def test_dino_artifacts_are_extracted_in_shared_batches():
    batch_sizes = []

    class FakeDino:
        def encode_with_patches(self, images):
            batch_sizes.append(len(images))
            globals_out = np.tile(np.eye(1, 16, dtype=np.float32), (len(images), 1))
            patches = [np.eye(16, dtype=np.float32) for _image in images]
            return globals_out, patches

    models = HighAccuracySubjectModels.__new__(HighAccuracySubjectModels)
    models._dino = FakeDino()
    models._load = lambda: None
    models._image = lambda _path: Image.new("RGB", (64, 64))
    models._face_service = SimpleNamespace(
        describe=lambda _image: SimpleNamespace(faces=[])
    )
    models.model_signature = "dino-v2"
    paths = [f"image-{index}.jpg" for index in range(40)]
    fingerprints = {path: (index, index) for index, path in enumerate(paths)}

    artifacts = models.extract_artifacts(paths, fingerprints)

    assert batch_sizes == [32, 8]
    assert set(artifacts) == set(paths)
    assert all(len(artifact.subjects) == 16 for artifact in artifacts.values())


def test_subject_artifact_cache_uses_compact_float16_descriptors():
    artifact = _artifact("a", subjects=[_subject((0.12345, 0.98765))])

    payload = artifact.to_dict()
    restored = SubjectArtifact.from_dict(payload)

    assert "global_descriptor_f16" in payload
    assert "global_descriptor" not in payload
    assert "descriptor_f16" in payload["subjects"][0]
    assert restored is not None
    assert np.allclose(restored.subjects[0].descriptor, (0.12345, 0.98765), atol=5e-4)

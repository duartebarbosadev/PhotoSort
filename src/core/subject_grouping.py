"""Conservative, instance-aware grouping used by the Cull workflow.

The module deliberately separates feature/model extraction from policy.  Heavy
model adapters provide immutable :class:`SubjectArtifact` records; the pure
functions here make candidate, pair, and cluster decisions and are therefore
fast to test without loading any ML runtime.
"""

from __future__ import annotations

from collections.abc import Callable, Mapping, Sequence
import base64
import binascii
from bisect import bisect_left, bisect_right
from dataclasses import asdict, dataclass
from datetime import UTC, datetime, timedelta
import hashlib
import json

import numpy as np
from scipy.optimize import linear_sum_assignment

from core.app_settings import CullGroupingStrictness
from core.similarity_utils import cosine_similarity


SUBJECT_GROUPING_PIPELINE_VERSION = "dino-dense-same-subject-v2"
GROUPING_POLICY_VERSION = "complete-link-geometry-rescue-v1"
CANDIDATE_POLICY_VERSION = "dino-semantic32-time8-v2"
SUBJECT_MIN_AREA_FRACTION = 0.015
SEMANTIC_NEIGHBOUR_COUNT = 32
TEMPORAL_NEIGHBOUR_COUNT = 8
TEMPORAL_WINDOW_SECONDS = 10 * 60


class SubjectGroupingCancelled(Exception):
    """Raised when a cooperative grouping operation is cancelled."""


@dataclass(frozen=True, slots=True)
class SubjectEvidence:
    descriptor: tuple[float, ...]
    bbox: tuple[float, float, float, float]
    area_fraction: float
    kind: str = "object"

    def to_dict(self) -> dict[str, object]:
        return {
            "descriptor_f16": base64.b64encode(
                np.asarray(self.descriptor, dtype="<f2").tobytes()
            ).decode("ascii"),
            "descriptor_size": len(self.descriptor),
            "bbox": list(self.bbox),
            "area_fraction": self.area_fraction,
            "kind": self.kind,
        }

    @classmethod
    def from_dict(cls, value: object) -> SubjectEvidence | None:
        if not isinstance(value, dict):
            return None
        try:
            if isinstance(value.get("descriptor_f16"), str):
                raw = base64.b64decode(value["descriptor_f16"], validate=True)
                expected_size = int(value["descriptor_size"])
                decoded: np.ndarray = np.frombuffer(raw, dtype="<f2")
                if len(decoded) != expected_size:
                    return None
                descriptor = tuple(float(item) for item in decoded)
            else:
                descriptor = tuple(float(item) for item in value["descriptor"])
            bbox = tuple(float(item) for item in value["bbox"])
            area = float(value["area_fraction"])
            kind = str(value.get("kind", "object"))
        except KeyError, TypeError, ValueError, binascii.Error:
            return None
        if (
            not descriptor
            or len(bbox) != 4
            or not np.isfinite((*descriptor, *bbox, area)).all()
        ):
            return None
        return cls(descriptor, bbox, area, kind)


@dataclass(frozen=True, slots=True)
class SubjectArtifact:
    path: str
    fingerprint: tuple[int, int]
    model_signature: str
    global_descriptor: tuple[float, ...]
    subjects: tuple[SubjectEvidence, ...]
    faces: tuple[SubjectEvidence, ...] = ()
    local_features: dict[str, object] | None = None

    def to_dict(self) -> dict[str, object]:
        return {
            "path": self.path,
            "fingerprint": list(self.fingerprint),
            "model_signature": self.model_signature,
            "global_descriptor_f16": base64.b64encode(
                np.asarray(self.global_descriptor, dtype="<f2").tobytes()
            ).decode("ascii"),
            "global_descriptor_size": len(self.global_descriptor),
            "subjects": [subject.to_dict() for subject in self.subjects],
            "faces": [face.to_dict() for face in self.faces],
            "local_features": self.local_features,
        }

    @classmethod
    def from_dict(cls, value: object) -> SubjectArtifact | None:
        if not isinstance(value, dict):
            return None
        subjects = tuple(
            parsed
            for item in value.get("subjects", [])
            if (parsed := SubjectEvidence.from_dict(item)) is not None
        )
        faces = tuple(
            parsed
            for item in value.get("faces", [])
            if (parsed := SubjectEvidence.from_dict(item)) is not None
        )
        try:
            fingerprint = tuple(int(item) for item in value["fingerprint"])
            if isinstance(value.get("global_descriptor_f16"), str):
                raw = base64.b64decode(value["global_descriptor_f16"], validate=True)
                expected_size = int(value["global_descriptor_size"])
                decoded: np.ndarray = np.frombuffer(raw, dtype="<f2")
                if len(decoded) != expected_size:
                    return None
                global_descriptor = tuple(float(item) for item in decoded)
            else:
                global_descriptor = tuple(
                    float(item) for item in value["global_descriptor"]
                )
            path = str(value["path"])
            signature = str(value["model_signature"])
        except KeyError, TypeError, ValueError, binascii.Error:
            return None
        if (
            len(fingerprint) != 2
            or not path
            or not global_descriptor
            or not np.isfinite(global_descriptor).all()
        ):
            return None
        return cls(
            path=path,
            fingerprint=(fingerprint[0], fingerprint[1]),
            model_signature=signature,
            global_descriptor=global_descriptor,
            subjects=subjects,
            faces=faces,
            local_features=(
                dict(value["local_features"])
                if isinstance(value.get("local_features"), dict)
                else None
            ),
        )


@dataclass(frozen=True, slots=True)
class GeometryEvidence:
    inlier_count: int = 0
    inlier_ratio: float = 0.0
    coverage_a: float = 0.0
    coverage_b: float = 0.0
    evaluated: bool = False

    @property
    def strong(self) -> bool:
        return (
            self.inlier_count >= 8
            and self.inlier_ratio >= 0.50
            and min(self.coverage_a, self.coverage_b) >= 0.12
        )


@dataclass(frozen=True, slots=True)
class PairVerification:
    path_a: str
    path_b: str
    subject_similarity: float
    face_similarity: float | None
    scene_similarity: float
    subject_set_complete: bool
    geometry: GeometryEvidence
    time_delta_seconds: float | None
    confidence: float
    accepted: bool
    reason: str

    def to_dict(self) -> dict[str, object]:
        payload = asdict(self)
        payload["geometry"] = asdict(self.geometry)
        return payload

    @classmethod
    def from_dict(cls, value: object) -> PairVerification | None:
        if not isinstance(value, dict):
            return None
        geometry_value = value.get("geometry")
        if not isinstance(geometry_value, dict):
            return None
        try:
            return cls(
                path_a=str(value["path_a"]),
                path_b=str(value["path_b"]),
                subject_similarity=float(value["subject_similarity"]),
                face_similarity=(
                    float(value["face_similarity"])
                    if value.get("face_similarity") is not None
                    else None
                ),
                scene_similarity=float(value["scene_similarity"]),
                subject_set_complete=bool(value["subject_set_complete"]),
                geometry=GeometryEvidence(**geometry_value),
                time_delta_seconds=(
                    float(value["time_delta_seconds"])
                    if value.get("time_delta_seconds") is not None
                    else None
                ),
                confidence=float(value["confidence"]),
                accepted=bool(value["accepted"]),
                reason=str(value["reason"]),
            )
        except KeyError, TypeError, ValueError:
            return None


@dataclass(frozen=True, slots=True)
class CullClusteringResult:
    clusters: dict[str, int]
    signature: str
    reused: bool = False
    failure: str | None = None


@dataclass(frozen=True, slots=True)
class StrictnessPolicy:
    minimum_subject_similarity: float
    minimum_scene_similarity: float
    minimum_exceptional_similarity: float
    minimum_confidence: float
    minimum_geometry_subject_similarity: float
    minimum_geometry_scene_similarity: float


STRICTNESS_POLICIES: dict[CullGroupingStrictness, StrictnessPolicy] = {
    CullGroupingStrictness.CONSERVATIVE: StrictnessPolicy(
        0.88, 0.82, 0.95, 0.88, 0.68, 0.86
    ),
    CullGroupingStrictness.STANDARD: StrictnessPolicy(
        0.84, 0.78, 0.92, 0.84, 0.64, 0.82
    ),
    CullGroupingStrictness.BROAD: StrictnessPolicy(0.80, 0.74, 0.89, 0.80, 0.60, 0.78),
}


def build_cull_grouping_signature(
    fingerprints: Mapping[str, tuple[int, int]],
    *,
    timestamps: Mapping[str, datetime | None],
    model_signature: str,
    strictness: CullGroupingStrictness,
) -> str:
    payload = {
        "pipeline": SUBJECT_GROUPING_PIPELINE_VERSION,
        "policy": GROUPING_POLICY_VERSION,
        "candidate_policy": CANDIDATE_POLICY_VERSION,
        "model_signature": model_signature,
        "strictness": strictness.value,
        "pair_context": build_cull_pair_context_signature(
            fingerprints, timestamps=timestamps
        ),
        "files": [[path, *fingerprints[path]] for path in sorted(fingerprints)],
    }
    encoded = json.dumps(payload, sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(encoded).hexdigest()


def _timestamp_cache_value(value: datetime | None) -> str | None:
    """Return a stable timestamp representation for cache validity checks."""

    if value is None:
        return None
    if value.tzinfo is not None and value.utcoffset() is not None:
        value = value.astimezone(UTC)
    return value.isoformat(timespec="microseconds")


def build_cull_pair_context_signature(
    fingerprints: Mapping[str, tuple[int, int]],
    *,
    timestamps: Mapping[str, datetime | None],
) -> str:
    """Describe the path and time inputs used to generate and score pairs.

    Pair evidence can survive strictness changes, but it cannot survive a change
    from fallback mtimes to EXIF capture dates because time affects both candidate
    selection and confidence.
    """

    payload = {
        "candidate_policy": CANDIDATE_POLICY_VERSION,
        "pair_policy": SUBJECT_GROUPING_PIPELINE_VERSION,
        "files": [
            [path, _timestamp_cache_value(timestamps.get(path))]
            for path in sorted(fingerprints)
        ],
    }
    encoded = json.dumps(payload, sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(encoded).hexdigest()


def _normalized_matrix(values: Sequence[Sequence[float]]) -> np.ndarray:
    matrix: np.ndarray = np.asarray(values, dtype=np.float32)
    if matrix.ndim != 2 or not len(matrix):
        return np.empty((0, 0), dtype=np.float32)
    norms = np.linalg.norm(matrix, axis=1, keepdims=True)
    return matrix / np.maximum(norms, np.finfo(np.float32).eps)


def generate_candidate_pairs(
    artifacts: Mapping[str, SubjectArtifact],
    timestamps: Mapping[str, datetime | None] | None = None,
    *,
    should_cancel: Callable[[], bool] | None = None,
) -> list[tuple[str, str]]:
    """Return deterministic semantic and near-time candidates across orientations."""

    cancelled = should_cancel or (lambda: False)
    paths = sorted(artifacts)
    if len(paths) < 2:
        return []
    matrix: np.ndarray = _normalized_matrix(
        [artifacts[path].global_descriptor for path in paths]
    )
    similarities: np.ndarray = matrix @ matrix.T
    pairs: set[tuple[str, str]] = set()
    for index, path in enumerate(paths):
        if cancelled():
            raise SubjectGroupingCancelled
        order = np.argsort(-similarities[index], kind="stable")
        neighbours = [item for item in order if item != index][
            :SEMANTIC_NEIGHBOUR_COUNT
        ]
        for neighbour in neighbours:
            other = paths[int(neighbour)]
            pairs.add((path, other) if path < other else (other, path))

    dated = sorted(
        (
            (value, path)
            for path, value in (timestamps or {}).items()
            if path in artifacts and value is not None
        ),
        key=lambda item: (item[0], item[1]),
    )
    # ``dated`` is sorted by time, so each photo's temporal window is a
    # contiguous slice instead of a scan over every other photo.
    times = [moment for moment, _path in dated]
    window = timedelta(seconds=TEMPORAL_WINDOW_SECONDS)
    for index, (timestamp, path) in enumerate(dated):
        if cancelled():
            raise SubjectGroupingCancelled
        first = bisect_left(times, timestamp - window)
        last = bisect_right(times, timestamp + window)
        temporal = sorted(
            (
                abs((times[other] - timestamp).total_seconds()),
                dated[other][1],
            )
            for other in range(first, last)
            if other != index
        )
        for _delta, other in temporal[:TEMPORAL_NEIGHBOUR_COUNT]:
            pairs.add((path, other) if path < other else (other, path))
    return sorted(pairs)


def _match_evidence_sets(
    first: Sequence[SubjectEvidence],
    second: Sequence[SubjectEvidence],
) -> tuple[bool, float]:
    """Require a complete one-to-one subject set and return its weakest match."""

    if len(first) != len(second):
        return False, 0.0
    if not first:
        return True, 1.0
    similarities: np.ndarray = np.zeros((len(first), len(second)), dtype=np.float32)
    for row, left in enumerate(first):
        for column, right in enumerate(second):
            if left.kind != right.kind:
                continue
            value = cosine_similarity(left.descriptor, right.descriptor)
            if value is not None:
                left_width = max(1e-6, left.bbox[2] - left.bbox[0])
                left_height = max(1e-6, left.bbox[3] - left.bbox[1])
                right_width = max(1e-6, right.bbox[2] - right.bbox[0])
                right_height = max(1e-6, right.bbox[3] - right.bbox[1])
                area_ratio = min(left.area_fraction, right.area_fraction) / max(
                    left.area_fraction, right.area_fraction, 1e-6
                )
                left_aspect = left_width / left_height
                right_aspect = right_width / right_height
                aspect_ratio = min(left_aspect, right_aspect) / max(
                    left_aspect, right_aspect
                )
                similarities[row, column] = (
                    0.90 * max(0.0, value) + 0.05 * area_ratio + 0.05 * aspect_ratio
                )
    rows, columns = linear_sum_assignment(-similarities)
    matched = similarities[rows, columns]
    if len(matched) != len(first):
        return False, 0.0
    return True, float(np.min(matched))


def verify_pair(
    first: SubjectArtifact,
    second: SubjectArtifact,
    *,
    geometry: GeometryEvidence,
    timestamp_a: datetime | None,
    timestamp_b: datetime | None,
    strictness: CullGroupingStrictness,
) -> PairVerification:
    policy = STRICTNESS_POLICIES[strictness]
    complete_subjects, subject_similarity = _match_evidence_sets(
        first.subjects, second.subjects
    )
    complete_faces, face_similarity_value = _match_evidence_sets(
        first.faces, second.faces
    )
    face_similarity = face_similarity_value if first.faces or second.faces else None
    scene_similarity = cosine_similarity(
        first.global_descriptor, second.global_descriptor
    )
    scene_similarity = scene_similarity if scene_similarity is not None else 0.0
    subject_set_complete = complete_subjects and complete_faces
    time_delta = (
        abs((timestamp_a - timestamp_b).total_seconds())
        if timestamp_a is not None and timestamp_b is not None
        else None
    )
    time_boost = (
        0.03
        * (1.0 - min(time_delta, TEMPORAL_WINDOW_SECONDS) / TEMPORAL_WINDOW_SECONDS)
        if time_delta is not None and time_delta <= TEMPORAL_WINDOW_SECONDS
        else 0.0
    )
    geometry_score = min(
        1.0,
        geometry.inlier_ratio * 1.5 + min(geometry.coverage_a, geometry.coverage_b),
    )
    confidence = min(
        1.0,
        0.68 * subject_similarity
        + 0.22 * scene_similarity
        + 0.10 * geometry_score
        + time_boost,
    )
    accepted, reason = _pair_policy_outcome(
        subject_set_complete=subject_set_complete,
        subject_similarity=subject_similarity,
        face_similarity=face_similarity,
        scene_similarity=scene_similarity,
        geometry=geometry,
        confidence=confidence,
        policy=policy,
    )
    return PairVerification(
        path_a=first.path,
        path_b=second.path,
        subject_similarity=subject_similarity,
        face_similarity=face_similarity,
        scene_similarity=scene_similarity,
        subject_set_complete=subject_set_complete,
        geometry=geometry,
        time_delta_seconds=time_delta,
        confidence=confidence,
        accepted=accepted,
        reason=reason,
    )


def apply_strictness(
    evidence: PairVerification,
    strictness: CullGroupingStrictness,
) -> PairVerification:
    """Reapply policy to cached raw evidence without rerunning any model."""

    policy = STRICTNESS_POLICIES[strictness]
    accepted, reason = _pair_policy_outcome(
        subject_set_complete=evidence.subject_set_complete,
        subject_similarity=evidence.subject_similarity,
        face_similarity=evidence.face_similarity,
        scene_similarity=evidence.scene_similarity,
        geometry=evidence.geometry,
        confidence=evidence.confidence,
        policy=policy,
    )
    return PairVerification(
        path_a=evidence.path_a,
        path_b=evidence.path_b,
        subject_similarity=evidence.subject_similarity,
        face_similarity=evidence.face_similarity,
        scene_similarity=evidence.scene_similarity,
        subject_set_complete=evidence.subject_set_complete,
        geometry=evidence.geometry,
        time_delta_seconds=evidence.time_delta_seconds,
        confidence=evidence.confidence,
        accepted=accepted,
        reason=reason,
    )


def _pair_policy_outcome(
    *,
    subject_set_complete: bool,
    subject_similarity: float,
    face_similarity: float | None,
    scene_similarity: float,
    geometry: GeometryEvidence,
    confidence: float,
    policy: StrictnessPolicy,
) -> tuple[bool, str]:
    """Fuse independent identity evidence without allowing a weak signal to veto."""

    face_identity_safe = (
        face_similarity is None or face_similarity >= policy.minimum_subject_similarity
    )
    exceptional_identity = (
        subject_similarity >= policy.minimum_exceptional_similarity
        or (
            face_similarity is not None
            and face_similarity >= policy.minimum_exceptional_similarity
        )
    )
    semantic_match = (
        subject_similarity >= policy.minimum_subject_similarity
        and (geometry.strong or exceptional_identity)
        and confidence >= policy.minimum_confidence
    )
    geometry_match = (
        geometry.strong
        and subject_similarity >= policy.minimum_geometry_subject_similarity
        and scene_similarity >= policy.minimum_geometry_scene_similarity
    )
    accepted = (
        subject_set_complete
        and face_identity_safe
        and scene_similarity >= policy.minimum_scene_similarity
        and (semantic_match or geometry_match)
    )
    if accepted:
        return True, "same_subject"
    if not subject_set_complete:
        return False, "subject_set_changed"
    if not face_identity_safe:
        return False, "face_identity_uncertain"
    if scene_similarity < policy.minimum_scene_similarity:
        return False, "scene_changed"
    if subject_similarity < policy.minimum_geometry_subject_similarity:
        return False, "subject_identity_uncertain"
    if not geometry.evaluated or not geometry.strong:
        return False, "geometry_uncertain"
    if subject_similarity < policy.minimum_subject_similarity:
        return False, "subject_identity_uncertain"
    return False, "confidence_below_policy"


def _needs_geometry_verification(
    evidence: PairVerification,
    strictness: CullGroupingStrictness,
) -> bool:
    """Return whether geometry could still turn a plausible pair into a match."""

    if (
        evidence.accepted
        or evidence.geometry.evaluated
        or not evidence.subject_set_complete
    ):
        return False
    policy = STRICTNESS_POLICIES[strictness]
    face_identity_safe = (
        evidence.face_similarity is None
        or evidence.face_similarity >= policy.minimum_subject_similarity
    )
    if not face_identity_safe:
        return False
    semantic_candidate = (
        evidence.subject_similarity >= policy.minimum_subject_similarity
        and evidence.scene_similarity >= policy.minimum_scene_similarity
    )
    geometry_rescue_candidate = (
        evidence.subject_similarity >= policy.minimum_geometry_subject_similarity
        and evidence.scene_similarity >= policy.minimum_geometry_scene_similarity
    )
    return semantic_candidate or geometry_rescue_candidate


def complete_link_clusters(
    paths: Sequence[str],
    verifications: Mapping[tuple[str, str], PairVerification],
) -> dict[str, int]:
    """Cluster paths while requiring every cross-pair in a merge to be accepted."""

    ordered_paths = sorted(dict.fromkeys(paths))
    clusters: list[list[str]] = [[path] for path in ordered_paths]

    def pair_key(left: str, right: str) -> tuple[str, str]:
        return (left, right) if left < right else (right, left)

    accepted_edges = sorted(
        (
            (-verification.confidence, key[0], key[1])
            for key, verification in verifications.items()
            if verification.accepted
        )
    )
    for _negative_confidence, left, right in accepted_edges:
        left_index = next(
            (index for index, cluster in enumerate(clusters) if left in cluster), None
        )
        right_index = next(
            (index for index, cluster in enumerate(clusters) if right in cluster), None
        )
        if left_index is None or right_index is None or left_index == right_index:
            continue
        left_cluster = clusters[left_index]
        right_cluster = clusters[right_index]
        if not all(
            (verification := verifications.get(pair_key(a, b))) is not None
            and verification.accepted
            for a in left_cluster
            for b in right_cluster
        ):
            continue
        merged = sorted((*left_cluster, *right_cluster))
        for index in sorted((left_index, right_index), reverse=True):
            clusters.pop(index)
        clusters.append(merged)
        clusters.sort(key=lambda group: (group[0], len(group)))

    clusters.sort(key=lambda group: group[0])
    return {
        path: cluster_id
        for cluster_id, cluster in enumerate(clusters, start=1)
        for path in cluster
    }


class SubjectGroupingService:
    """Orchestrate cached artifact extraction, pair verification, and clustering."""

    def __init__(
        self,
        *,
        artifact_provider: Callable[[str], SubjectArtifact],
        geometry_provider: Callable[
            [SubjectArtifact, SubjectArtifact], GeometryEvidence
        ],
        should_cancel: Callable[[], bool] | None = None,
        progress_callback: Callable[[int, str], None] | None = None,
    ) -> None:
        self.artifact_provider = artifact_provider
        self.geometry_provider = geometry_provider
        self.should_cancel = should_cancel or (lambda: False)
        self.progress_callback = progress_callback or (lambda _percent, _message: None)

    def group(
        self,
        paths: Sequence[str],
        *,
        fingerprints: Mapping[str, tuple[int, int]],
        timestamps: Mapping[str, datetime | None],
        strictness: CullGroupingStrictness,
        model_signature: str,
        cached_artifacts: Mapping[str, SubjectArtifact] | None = None,
        cached_pairs: Mapping[tuple[str, str], PairVerification] | None = None,
    ) -> tuple[
        CullClusteringResult,
        dict[str, SubjectArtifact],
        dict[tuple[str, str], PairVerification],
    ]:
        artifacts = dict(cached_artifacts or {})
        requested = sorted(dict.fromkeys(paths))
        reextracted: set[str] = set()
        for index, path in enumerate(requested):
            if self.should_cancel():
                raise SubjectGroupingCancelled
            cached_artifact = artifacts.get(path)
            if (
                cached_artifact is None
                or cached_artifact.fingerprint != fingerprints.get(path)
                or cached_artifact.model_signature != model_signature
            ):
                artifacts[path] = self.artifact_provider(path)
                reextracted.add(path)
                self.progress_callback(
                    int(55 * (index + 1) / max(1, len(requested))),
                    f"Encoding DINO features ({index + 1}/{len(requested)})",
                )
        if not reextracted:
            self.progress_callback(55, "DINO subject features ready")

        pairs = generate_candidate_pairs(
            artifacts, timestamps, should_cancel=self.should_cancel
        )
        # Cached evidence describes the exact image content it was computed from,
        # so any pair touching a re-extracted photo must be verified again.
        verifications = {
            key: value
            for key, value in (cached_pairs or {}).items()
            if not reextracted.intersection(key)
        }
        for index, (path_a, path_b) in enumerate(pairs):
            if self.should_cancel():
                raise SubjectGroupingCancelled
            key = (path_a, path_b)
            verification = verifications.get(key)
            if verification is None:
                first = artifacts[path_a]
                second = artifacts[path_b]
                verification = verify_pair(
                    first,
                    second,
                    geometry=GeometryEvidence(),
                    timestamp_a=timestamps.get(path_a),
                    timestamp_b=timestamps.get(path_b),
                    strictness=strictness,
                )
            else:
                verification = apply_strictness(verification, strictness)
                first = artifacts[path_a]
                second = artifacts[path_b]
            if _needs_geometry_verification(verification, strictness):
                verification = verify_pair(
                    first,
                    second,
                    geometry=self.geometry_provider(first, second),
                    timestamp_a=timestamps.get(path_a),
                    timestamp_b=timestamps.get(path_b),
                    strictness=strictness,
                )
            verifications[key] = verification
            self.progress_callback(
                55 + int(40 * (index + 1) / max(1, len(pairs))),
                f"Verifying subject pairs ({index + 1}/{len(pairs)})",
            )
        clusters = complete_link_clusters(requested, verifications)
        signature = build_cull_grouping_signature(
            fingerprints,
            timestamps=timestamps,
            model_signature=model_signature,
            strictness=strictness,
        )
        self.progress_callback(100, "Same-subject grouping complete")
        return (
            CullClusteringResult(clusters=clusters, signature=signature),
            artifacts,
            verifications,
        )

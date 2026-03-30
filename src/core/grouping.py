from __future__ import annotations

import json
import logging
import os
import re
import shutil
from dataclasses import asdict, dataclass, field
from datetime import datetime
from enum import Enum
from typing import Any, Dict, Iterable, List, Optional, Sequence, Tuple

import numpy as np
from PIL import Image
from sklearn.cluster import DBSCAN

from core.media_utils import is_video_extension
from core.metadata_processor import MetadataProcessor
from core.similarity_engine import SimilarityEngine

logger = logging.getLogger(__name__)


class GroupingMode(str, Enum):
    CURRENT = "current"
    SIMILARITY = "similarity"
    FACE = "face"
    LOCATION = "location"
    MIXED = "mixed"


@dataclass
class GroupingGroup:
    group_id: str
    group_label: str
    source_paths: List[str]
    destination_folder: str = ""
    skipped_paths: List[str] = field(default_factory=list)


@dataclass
class GroupingPreview:
    mode: str
    total_items: int
    supported_items: int
    predicted_group_count: int
    unassigned_count: int
    skipped_count: int
    summary_text: str


@dataclass
class GroupingPlan:
    mode: str
    total_items: int
    supported_items: int
    groups: List[GroupingGroup]
    unassigned_paths: List[str]
    skipped_paths: List[str]
    output_root: str = ""

    def to_preview(self) -> GroupingPreview:
        return GroupingPreview(
            mode=self.mode,
            total_items=self.total_items,
            supported_items=self.supported_items,
            predicted_group_count=len(self.groups),
            unassigned_count=len(self.unassigned_paths),
            skipped_count=len(self.skipped_paths),
            summary_text=(
                f"{len(self.groups)} group(s) from {self.supported_items} supported item(s). "
                f"Unassigned: {len(self.unassigned_paths)}. Skipped: {len(self.skipped_paths)}."
            ),
        )

    def apply_group_label_overrides(self, overrides: Dict[str, str] | None) -> None:
        if not overrides:
            return
        for group in self.groups:
            override = overrides.get(str(group.group_id))
            if override:
                group.group_label = override.strip()


@dataclass
class GroupingManifestEntry:
    original_path: str
    new_path: Optional[str]
    group_id: Optional[str]
    group_label: Optional[str]
    status: str
    reason: Optional[str] = None


@dataclass
class GroupingRunSummary:
    mode: str
    source_root: str
    output_root: str
    manifest_path: str
    moved_count: int
    unassigned_count: int
    skipped_count: int
    groups: List[GroupingGroup]
    entries: List[GroupingManifestEntry]


def build_grouping_output_root(source_root: str, mode: str) -> str:
    return source_root


def write_grouping_manifest(summary: GroupingRunSummary) -> str:
    manifest_path = os.path.join(summary.output_root, "grouping-manifest.json")
    payload = {
        "mode": summary.mode,
        "source_root": summary.source_root,
        "output_root": summary.output_root,
        "moved_count": summary.moved_count,
        "unassigned_count": summary.unassigned_count,
        "skipped_count": summary.skipped_count,
        "generated_at": datetime.now().isoformat(),
        "groups": [asdict(group) for group in summary.groups],
        "entries": [asdict(entry) for entry in summary.entries],
    }
    os.makedirs(summary.output_root, exist_ok=True)
    with open(manifest_path, "w", encoding="utf-8") as fh:
        json.dump(payload, fh, indent=2, ensure_ascii=True)
    return manifest_path


def _sanitize_folder_component(value: str) -> str:
    cleaned = re.sub(r"[\\/:*?\"<>|]+", "_", (value or "").strip())
    cleaned = re.sub(r"\s+", " ", cleaned).strip(" .")
    return cleaned or "Unnamed"


def _resolve_collision_safe_destination(destination_dir: str, basename: str) -> str:
    os.makedirs(destination_dir, exist_ok=True)
    stem, ext = os.path.splitext(basename)
    candidate = os.path.join(destination_dir, basename)
    suffix = 1
    while os.path.exists(candidate):
        candidate = os.path.join(destination_dir, f"{stem}_{suffix}{ext}")
        suffix += 1
    return candidate


def _cluster_vectors(
    vectors_by_path: Dict[str, np.ndarray],
    *,
    eps: float,
    min_samples: int = 1,
) -> Dict[str, int]:
    if not vectors_by_path:
        return {}
    paths = list(vectors_by_path.keys())
    matrix = np.stack([vectors_by_path[path] for path in paths])
    if len(paths) == 1:
        return {paths[0]: 1}
    clustering = DBSCAN(eps=eps, min_samples=min_samples, metric="cosine")
    labels = clustering.fit_predict(matrix)
    label_to_cluster_id: Dict[int, int] = {}
    next_cluster_id = 1
    assignments: Dict[str, int] = {}
    for idx, label in enumerate(labels):
        if label == -1:
            continue
        if label not in label_to_cluster_id:
            label_to_cluster_id[label] = next_cluster_id
            next_cluster_id += 1
        assignments[paths[idx]] = label_to_cluster_id[label]
    return assignments


def _load_image_for_features(path: str) -> Optional[Image.Image]:
    try:
        with Image.open(path) as img:
            return img.convert("RGB")
    except Exception:
        logger.debug("Unable to load image for grouping features: %s", path, exc_info=True)
        return None


def _compute_face_vector(path: str) -> Tuple[Optional[np.ndarray], bool]:
    image = _load_image_for_features(path)
    if image is None:
        return None, False
    width, height = image.size
    if width < 32 or height < 32:
        return None, False
    crop_w = max(32, int(width * 0.55))
    crop_h = max(32, int(height * 0.55))
    left = max(0, (width - crop_w) // 2)
    top = max(0, (height - crop_h) // 3)
    right = min(width, left + crop_w)
    bottom = min(height, top + crop_h)
    crop = image.crop((left, top, right, bottom)).convert("L").resize((24, 24))
    arr = np.asarray(crop, dtype=np.float32) / 255.0
    variance = float(arr.var())
    gx = np.abs(np.diff(arr, axis=1)).mean()
    gy = np.abs(np.diff(arr, axis=0)).mean()
    edge_strength = float((gx + gy) / 2.0)
    has_face_like_signal = variance >= 0.004 and edge_strength >= 0.035
    if not has_face_like_signal:
        return None, False
    vector = arr.reshape(-1)
    norm = np.linalg.norm(vector)
    if norm <= 0:
        return None, False
    return vector / norm, True


def _parse_gps_coordinate(raw_value: Any, ref_value: Any) -> Optional[float]:
    if raw_value is None:
        return None
    sign = 1.0
    ref_text = str(ref_value or "").strip().upper()
    if ref_text in {"S", "W"}:
        sign = -1.0

    if isinstance(raw_value, (int, float)):
        return float(raw_value) * sign

    text = str(raw_value).strip()
    if not text:
        return None

    try:
        if text.startswith("(") and text.endswith(")"):
            parts = [p.strip() for p in text.strip("()").split(",")]
        else:
            parts = [p.strip() for p in re.split(r"[,\s]+", text) if p.strip()]
        if len(parts) == 1:
            return float(parts[0]) * sign
        degrees = _parse_fraction(parts[0])
        minutes = _parse_fraction(parts[1]) if len(parts) > 1 else 0.0
        seconds = _parse_fraction(parts[2]) if len(parts) > 2 else 0.0
        value = degrees + (minutes / 60.0) + (seconds / 3600.0)
        return value * sign
    except Exception:
        return None


def _parse_fraction(value: str) -> float:
    if "/" in value:
        num, den = value.split("/", 1)
        den_val = float(den)
        if den_val == 0:
            return 0.0
        return float(num) / den_val
    return float(value)


def _location_label_from_metadata(metadata: Dict[str, Any]) -> Optional[str]:
    lat = _parse_gps_coordinate(
        metadata.get("Exif.GPSInfo.GPSLatitude"),
        metadata.get("Exif.GPSInfo.GPSLatitudeRef"),
    )
    lon = _parse_gps_coordinate(
        metadata.get("Exif.GPSInfo.GPSLongitude"),
        metadata.get("Exif.GPSInfo.GPSLongitudeRef"),
    )
    if lat is None or lon is None:
        return None
    lat_bucket = round(lat, 2)
    lon_bucket = round(lon, 2)
    return f"Lat_{lat_bucket:.2f}_Lon_{lon_bucket:.2f}"


def _load_comprehensive_metadata(path: str) -> Dict[str, Any]:
    try:
        resolved = MetadataProcessor._resolve_path_forms(path)  # type: ignore[attr-defined]
        operational = resolved[0] if resolved else path
        from core.pyexiv2_wrapper import PyExiv2Operations

        return PyExiv2Operations.get_comprehensive_metadata(operational)
    except Exception:
        logger.debug("Comprehensive metadata fetch failed for %s", path, exc_info=True)
        return {}


def _build_groups_from_assignments(
    assignments: Dict[str, int],
    label_builder,
) -> List[GroupingGroup]:
    grouped: Dict[int, List[str]] = {}
    for path, cluster_id in assignments.items():
        grouped.setdefault(cluster_id, []).append(path)
    groups: List[GroupingGroup] = []
    for cluster_id in sorted(grouped.keys()):
        groups.append(
            GroupingGroup(
                group_id=str(cluster_id),
                group_label=label_builder(cluster_id),
                source_paths=sorted(grouped[cluster_id]),
            )
        )
    return groups


def _parse_cluster_id(value: Any) -> Optional[int]:
    if isinstance(value, int):
        return value
    if isinstance(value, str):
        try:
            return int(value.split(" - ")[0])
        except Exception:
            try:
                return int(value)
            except Exception:
                return None
    return None


def _run_ml_similarity_pipeline(
    image_paths: Sequence[str],
    progress_callback=None,
    shared_engine: Optional[SimilarityEngine] = None,
) -> Dict[str, int]:
    if not image_paths:
        return {}
    engine = shared_engine or SimilarityEngine()
    _embeddings, cluster_results = engine.run_analysis_sync(
        list(image_paths),
        progress_callback=progress_callback,
    )
    assignments: Dict[str, int] = {}
    for path, raw_cluster in cluster_results.items():
        cluster_id = _parse_cluster_id(raw_cluster)
        if cluster_id is not None:
            assignments[path] = cluster_id
    return assignments


def build_grouping_plan(
    items: Sequence[Dict[str, Any]],
    mode: GroupingMode | str,
    progress_callback=None,
    source_root: Optional[str] = None,
) -> GroupingPlan:
    mode_value = GroupingMode(mode)
    valid_items = [item for item in items if isinstance(item, dict) and item.get("path")]
    total_items = len(valid_items)
    image_paths = [
        item["path"] for item in valid_items if not is_video_extension(item["path"])
    ]
    skipped_paths = [
        item["path"] for item in valid_items if is_video_extension(item["path"])
    ]

    if mode_value == GroupingMode.CURRENT:
        return _build_current_structure_plan(
            total_items,
            image_paths,
            skipped_paths,
            source_root=source_root,
        )
    if mode_value == GroupingMode.LOCATION:
        return _build_location_plan(total_items, image_paths, skipped_paths)
    if mode_value == GroupingMode.FACE:
        return _build_face_plan(total_items, image_paths, skipped_paths)
    if mode_value == GroupingMode.MIXED:
        return _build_mixed_plan(
            total_items,
            image_paths,
            skipped_paths,
            progress_callback=progress_callback,
        )
    return _build_similarity_plan(
        total_items,
        image_paths,
        skipped_paths,
        progress_callback=progress_callback,
    )


def _build_current_structure_plan(
    total_items: int,
    image_paths: Sequence[str],
    skipped_paths: Sequence[str],
    *,
    source_root: Optional[str] = None,
) -> GroupingPlan:
    groups_by_label: Dict[str, List[str]] = {}
    resolved_source_root = (
        os.path.normpath(source_root)
        if source_root
        else (
            os.path.commonpath(list(image_paths))
            if image_paths
            else ""
        )
    )
    root_label = (
        os.path.basename(resolved_source_root.rstrip(os.sep))
        if resolved_source_root
        else "Root"
    ) or "Root"

    for path in image_paths:
        parent_dir = os.path.dirname(path)
        if resolved_source_root:
            try:
                rel_dir = os.path.relpath(parent_dir, resolved_source_root)
            except Exception:
                rel_dir = parent_dir
        else:
            rel_dir = parent_dir
        label = root_label if rel_dir in {".", ""} else rel_dir
        groups_by_label.setdefault(label, []).append(path)

    groups: List[GroupingGroup] = []
    for index, label in enumerate(sorted(groups_by_label.keys()), start=1):
        groups.append(
            GroupingGroup(
                group_id=str(index),
                group_label=label,
                source_paths=sorted(groups_by_label[label]),
            )
        )

    return GroupingPlan(
        mode=GroupingMode.CURRENT.value,
        total_items=total_items,
        supported_items=len(image_paths),
        groups=groups,
        unassigned_paths=[],
        skipped_paths=list(skipped_paths),
    )


def _build_similarity_plan(
    total_items: int,
    image_paths: Sequence[str],
    skipped_paths: Sequence[str],
    progress_callback=None,
) -> GroupingPlan:
    assignments = _run_ml_similarity_pipeline(
        image_paths,
        progress_callback=progress_callback,
    )
    grouped_paths = set(assignments.keys())
    unassigned = sorted([path for path in image_paths if path not in grouped_paths])
    groups = _build_groups_from_assignments(
        assignments, lambda cluster_id: f"Group {cluster_id:03d}"
    )
    return GroupingPlan(
        mode=GroupingMode.SIMILARITY.value,
        total_items=total_items,
        supported_items=len(image_paths),
        groups=groups,
        unassigned_paths=unassigned,
        skipped_paths=list(skipped_paths),
    )


def _build_face_plan(
    total_items: int,
    image_paths: Sequence[str],
    skipped_paths: Sequence[str],
) -> GroupingPlan:
    vectors: Dict[str, np.ndarray] = {}
    unassigned: List[str] = []
    for path in image_paths:
        vector, has_face_like_signal = _compute_face_vector(path)
        if vector is None or not has_face_like_signal:
            unassigned.append(path)
            continue
        vectors[path] = vector
    assignments = _cluster_vectors(vectors, eps=0.10, min_samples=1)
    assigned_paths = set(assignments.keys())
    for path in vectors.keys():
        if path not in assigned_paths:
            unassigned.append(path)
    groups = _build_groups_from_assignments(
        assignments, lambda cluster_id: f"Person {cluster_id:03d}"
    )
    return GroupingPlan(
        mode=GroupingMode.FACE.value,
        total_items=total_items,
        supported_items=len(image_paths),
        groups=groups,
        unassigned_paths=sorted(set(unassigned)),
        skipped_paths=list(skipped_paths),
    )


def _build_location_plan(
    total_items: int,
    image_paths: Sequence[str],
    skipped_paths: Sequence[str],
) -> GroupingPlan:
    buckets: Dict[str, List[str]] = {}
    unassigned: List[str] = []
    for path in image_paths:
        metadata = _load_comprehensive_metadata(path)
        label = _location_label_from_metadata(metadata)
        if not label:
            unassigned.append(path)
            continue
        buckets.setdefault(label, []).append(path)
    groups: List[GroupingGroup] = []
    for idx, label in enumerate(sorted(buckets.keys()), start=1):
        groups.append(
            GroupingGroup(
                group_id=str(idx),
                group_label=label,
                source_paths=sorted(buckets[label]),
            )
        )
    return GroupingPlan(
        mode=GroupingMode.LOCATION.value,
        total_items=total_items,
        supported_items=len(image_paths),
        groups=groups,
        unassigned_paths=sorted(unassigned),
        skipped_paths=list(skipped_paths),
    )


def _build_mixed_plan(
    total_items: int,
    image_paths: Sequence[str],
    skipped_paths: Sequence[str],
    progress_callback=None,
) -> GroupingPlan:
    location_buckets: Dict[str, List[str]] = {}
    unassigned: List[str] = []
    for path in image_paths:
        metadata = _load_comprehensive_metadata(path)
        label = _location_label_from_metadata(metadata)
        if not label:
            unassigned.append(path)
            continue
        location_buckets.setdefault(label, []).append(path)

    groups: List[GroupingGroup] = []
    group_counter = 1
    similarity_engine = SimilarityEngine()
    for location_label in sorted(location_buckets.keys()):
        def _bucket_progress(percent: int, message: str):
            if progress_callback:
                progress_callback(
                    percent,
                    f"{location_label}: {message}",
                )

        assignments = _run_ml_similarity_pipeline(
            location_buckets[location_label],
            progress_callback=_bucket_progress if progress_callback else None,
            shared_engine=similarity_engine,
        )
        grouped_by_cluster: Dict[int, List[str]] = {}
        for path, cluster_id in assignments.items():
            grouped_by_cluster.setdefault(cluster_id, []).append(path)
        for path in location_buckets[location_label]:
            if path not in assignments:
                unassigned.append(path)
        for local_cluster_id in sorted(grouped_by_cluster.keys()):
            groups.append(
                GroupingGroup(
                    group_id=str(group_counter),
                    group_label=os.path.join(
                        _sanitize_folder_component(location_label),
                        f"Group {local_cluster_id:03d}",
                    ),
                    source_paths=sorted(grouped_by_cluster[local_cluster_id]),
                )
            )
            group_counter += 1
    return GroupingPlan(
        mode=GroupingMode.MIXED.value,
        total_items=total_items,
        supported_items=len(image_paths),
        groups=groups,
        unassigned_paths=sorted(set(unassigned)),
        skipped_paths=list(skipped_paths),
    )


def execute_grouping_plan(
    plan: GroupingPlan,
    *,
    source_root: str,
    output_root: str,
    progress_callback=None,
) -> GroupingRunSummary:
    os.makedirs(output_root, exist_ok=True)
    entries: List[GroupingManifestEntry] = []
    total_moves = sum(len(group.source_paths) for group in plan.groups) + len(
        plan.unassigned_paths
    )
    moved_count = 0

    def report(message: str) -> None:
        if progress_callback:
            percent = int((moved_count / total_moves) * 100) if total_moves else 100
            progress_callback(percent, message)

    for group in plan.groups:
        destination_dir = os.path.join(
            output_root, *[_sanitize_folder_component(part) for part in group.group_label.split(os.sep)]
        )
        group.destination_folder = destination_dir
        for source_path in group.source_paths:
            basename = os.path.basename(source_path)
            desired_destination_path = os.path.join(destination_dir, basename)
            if os.path.normcase(os.path.normpath(source_path)) == os.path.normcase(
                os.path.normpath(desired_destination_path)
            ):
                entries.append(
                    GroupingManifestEntry(
                        original_path=source_path,
                        new_path=source_path,
                        group_id=group.group_id,
                        group_label=group.group_label,
                        status="unchanged",
                    )
                )
                report(f"Keeping {basename} in place...")
                continue
            destination_path = _resolve_collision_safe_destination(destination_dir, basename)
            shutil.move(source_path, destination_path)
            moved_count += 1
            entries.append(
                GroupingManifestEntry(
                    original_path=source_path,
                    new_path=destination_path,
                    group_id=group.group_id,
                    group_label=group.group_label,
                    status="moved",
                )
            )
            report(f"Grouping {basename}...")

    if plan.unassigned_paths:
        unassigned_dir = os.path.join(output_root, "Unassigned")
        for source_path in plan.unassigned_paths:
            basename = os.path.basename(source_path)
            destination_path = _resolve_collision_safe_destination(unassigned_dir, basename)
            shutil.move(source_path, destination_path)
            moved_count += 1
            entries.append(
                GroupingManifestEntry(
                    original_path=source_path,
                    new_path=destination_path,
                    group_id=None,
                    group_label="Unassigned",
                    status="unassigned",
                )
            )
            report(f"Moving unassigned {basename}...")

    for source_path in plan.skipped_paths:
        entries.append(
            GroupingManifestEntry(
                original_path=source_path,
                new_path=None,
                group_id=None,
                group_label=None,
                status="skipped",
                reason="unsupported media",
            )
        )

    summary = GroupingRunSummary(
        mode=plan.mode,
        source_root=source_root,
        output_root=output_root,
        manifest_path="",
        moved_count=moved_count,
        unassigned_count=len(plan.unassigned_paths),
        skipped_count=len(plan.skipped_paths),
        groups=plan.groups,
        entries=entries,
    )
    summary.manifest_path = write_grouping_manifest(summary)
    return summary

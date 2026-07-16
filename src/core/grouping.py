import logging
import os
import re
from functools import lru_cache
from dataclasses import dataclass, field
from datetime import datetime
from enum import StrEnum
from typing import TYPE_CHECKING, Any
from collections.abc import Iterable, Sequence

import numpy as np
from PIL import Image

from core.image_file_ops import ImageFileOperations
from core.media_utils import is_video_extension
from core.metadata_processor import (
    DATE_TAGS_PREFERENCE,
    MetadataProcessor,
    _parse_exif_date,
    _parse_date_from_filename,
)

if TYPE_CHECKING:
    from core.image_pipeline import ImagePipeline
    from core.similarity_engine import SimilarityEngine

logger = logging.getLogger(__name__)


class GroupingMode(StrEnum):
    CURRENT = "current"
    SIMILARITY = "similarity"
    FACE = "face"
    LOCATION = "location"
    MIXED = "mixed"


@dataclass(slots=True)
class GroupingGroup:
    group_id: str
    group_label: str
    source_paths: list[str]
    destination_folder: str = ""
    skipped_paths: list[str] = field(default_factory=list)


@dataclass(slots=True)
class GroupingPreview:
    mode: str
    total_items: int
    supported_items: int
    predicted_group_count: int
    unassigned_count: int
    skipped_count: int
    summary_text: str


@dataclass(slots=True)
class GroupingPlan:
    mode: str
    total_items: int
    supported_items: int
    groups: list[GroupingGroup]
    unassigned_paths: list[str]
    skipped_paths: list[str]
    output_root: str = ""
    file_name_overrides: dict[str, str] = field(default_factory=dict)
    deleted_paths: list[str] = field(default_factory=list)
    # The Organize UI must not walk the source tree again when the preview
    # worker has already included non-media files that need to move with it.
    filesystem_inventory_complete: bool = False
    source_root: str = ""

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

    def apply_group_label_overrides(self, overrides: dict[str, str] | None) -> None:
        if not overrides:
            return
        for group in self.groups:
            override = overrides.get(str(group.group_id))
            if override:
                group.group_label = override.strip()

    def filename_for_path(self, source_path: str) -> str:
        override = self.file_name_overrides.get(source_path, "").strip()
        if override:
            return override
        return os.path.basename(source_path)


def augment_grouping_plan_with_filesystem_paths(
    plan: GroupingPlan,
    source_root: str | None,
) -> GroupingPlan:
    """Include unmanaged files in a grouping plan outside the UI thread.

    The scanner intentionally returns supported media. Organize also needs to
    preserve other files in directories that contain scanned media (for
    example JSON metadata). This inventory is prepared by the grouping worker
    so the widget can render the plan without performing its own filesystem
    walks.
    """
    root = os.path.normpath(source_root or "")
    if plan.filesystem_inventory_complete:
        return plan
    plan.source_root = root
    if not root or not os.path.isdir(root):
        plan.filesystem_inventory_complete = True
        return plan

    planned_paths = list(
        dict.fromkeys(
            [path for group in plan.groups for path in group.source_paths]
            + list(plan.unassigned_paths)
            + list(plan.skipped_paths)
        )
    )
    normalized_root = os.path.normcase(root)
    candidate_directories: list[str] = []
    seen_directories: set[str] = set()
    for path in planned_paths:
        directory = os.path.normpath(os.path.dirname(path))
        normalized_directory = os.path.normcase(directory)
        try:
            if (
                os.path.commonpath([normalized_directory, normalized_root])
                != normalized_root
            ):
                continue
        except ValueError, OSError:
            continue
        if normalized_directory in seen_directories or not os.path.isdir(directory):
            continue
        seen_directories.add(normalized_directory)
        candidate_directories.append(directory)

    # If a directory is already covered by an ancestor walk, do not traverse
    # it again. Mixed root/nested media layouts otherwise duplicate most I/O.
    walk_roots: list[str] = []
    for directory in sorted(
        candidate_directories, key=lambda value: value.count(os.sep)
    ):
        normalized_directory = os.path.normcase(os.path.normpath(directory))
        if any(
            normalized_directory == existing
            or normalized_directory.startswith(existing + os.sep)
            for existing in (
                os.path.normcase(os.path.normpath(value)) for value in walk_roots
            )
        ):
            continue
        walk_roots.append(directory)

    deleted_paths = [
        os.path.normcase(os.path.normpath(path))
        for path in (plan.deleted_paths or [])
        if path
    ]

    def is_pending_deletion(path: str) -> bool:
        normalized_path = os.path.normcase(os.path.normpath(path))
        for deleted_path in deleted_paths:
            if normalized_path == deleted_path:
                return True
            try:
                if os.path.commonpath([normalized_path, deleted_path]) == deleted_path:
                    return True
            except ValueError, OSError:
                continue
        return False

    discovered_paths: list[str] = []
    for walk_root in walk_roots:
        for current_root, _dirnames, filenames in os.walk(walk_root):
            for filename in filenames:
                # XMP files follow the separate companion-file preference.
                if os.path.splitext(filename)[1].lower() == ".xmp":
                    continue
                path = os.path.join(current_root, filename)
                if not is_pending_deletion(path):
                    discovered_paths.append(path)

    normalized_planned = {
        os.path.normcase(os.path.normpath(path)) for path in planned_paths
    }
    extra_paths = sorted(
        path
        for path in dict.fromkeys(discovered_paths)
        if os.path.normcase(os.path.normpath(path)) not in normalized_planned
    )

    groups_by_label = {
        os.path.normpath(group.group_label or ""): group for group in plan.groups
    }
    used_group_ids = {str(group.group_id) for group in plan.groups}
    next_group_id = 1

    for path in extra_paths:
        relative_directory = os.path.relpath(os.path.dirname(path), root)
        label = "" if relative_directory in {"", "."} else relative_directory
        normalized_label = os.path.normpath(label or "")
        group = groups_by_label.get(normalized_label)
        if group is None:
            while str(next_group_id) in used_group_ids:
                next_group_id += 1
            group_id = str(next_group_id)
            next_group_id += 1
            used_group_ids.add(group_id)
            group = GroupingGroup(
                group_id=group_id,
                group_label=label,
                source_paths=[],
            )
            plan.groups.append(group)
            groups_by_label[normalized_label] = group
        group.source_paths.append(path)

    for group in plan.groups:
        group.source_paths = sorted(dict.fromkeys(group.source_paths))
    plan.filesystem_inventory_complete = True
    logger.info(
        "Grouping filesystem inventory prepared off the UI thread: planned=%d extra=%d roots=%d",
        len(planned_paths),
        len(extra_paths),
        len(walk_roots),
    )
    return plan


@dataclass(slots=True)
class GroupingRunEntry:
    original_path: str
    new_path: str | None
    group_id: str | None
    group_label: str | None
    status: str
    reason: str | None = None


@dataclass(slots=True)
class GroupingRunSummary:
    mode: str
    source_root: str
    output_root: str
    moved_count: int
    deleted_count: int
    unassigned_count: int
    skipped_count: int
    groups: list[GroupingGroup]
    entries: list[GroupingRunEntry]


@dataclass(frozen=True, slots=True)
class GroupingDirectoryRename:
    group_id: str
    source_dir: str
    target_dir: str


def build_grouping_output_root(source_root: str, mode: str) -> str:
    return source_root


def _sanitize_folder_component(value: str) -> str:
    cleaned = re.sub(r"[\\/:*?\"<>|]+", "_", (value or "").strip())
    cleaned = re.sub(r"\s+", " ", cleaned).strip(" .")
    return cleaned or "Unnamed"


def _is_same_path(left: str, right: str) -> bool:
    return os.path.normcase(os.path.normpath(left)) == os.path.normcase(
        os.path.normpath(right)
    )


def _is_path_within_dir(path: str, directory: str) -> bool:
    try:
        return _is_same_path(os.path.commonpath([path, directory]), directory)
    except Exception:
        return False


def find_directory_rename_candidates(
    plan: GroupingPlan,
    *,
    source_root: str,
    output_root: str,
) -> dict[str, GroupingDirectoryRename]:
    if str(plan.mode) != GroupingMode.CURRENT.value:
        return {}

    normalized_source_root = os.path.normpath(source_root)
    active_paths = [
        path for group in plan.groups for path in group.source_paths
    ] + list(plan.unassigned_paths)
    candidates: dict[str, GroupingDirectoryRename] = {}
    reserved_target_dirs: set[str] = set()

    for group in plan.groups:
        if not group.source_paths:
            continue
        if any(path in plan.file_name_overrides for path in group.source_paths):
            continue

        source_dirs = {os.path.dirname(path) for path in group.source_paths}
        if len(source_dirs) != 1:
            continue
        source_dir = next(iter(source_dirs))
        if _is_same_path(source_dir, normalized_source_root):
            continue

        source_dir_paths = set(group.source_paths)
        if any(
            path not in source_dir_paths and _is_path_within_dir(path, source_dir)
            for path in active_paths
        ):
            continue

        target_dir = os.path.join(
            output_root,
            *[
                _sanitize_folder_component(part)
                for part in re.split(r"[\\/]+", group.group_label)
                if part
            ],
        )
        if not target_dir or _is_same_path(source_dir, target_dir):
            continue
        normalized_target_dir = os.path.normcase(os.path.normpath(target_dir))
        if normalized_target_dir in reserved_target_dirs:
            continue
        if os.path.exists(target_dir):
            continue

        candidates[str(group.group_id)] = GroupingDirectoryRename(
            group_id=str(group.group_id),
            source_dir=source_dir,
            target_dir=target_dir,
        )
        reserved_target_dirs.add(normalized_target_dir)

    return candidates


def _resolve_collision_safe_destination(destination_dir: str, basename: str) -> str:
    os.makedirs(destination_dir, exist_ok=True)
    stem, ext = os.path.splitext(basename)
    candidate = os.path.join(destination_dir, basename)
    suffix = 1
    while os.path.exists(candidate):
        candidate = os.path.join(destination_dir, f"{stem}_{suffix}{ext}")
        suffix += 1
    return candidate


def _find_companion_files(photo_path: str) -> list[str]:
    """Find companion files with same stem: .xmp sidecars and same-stem images in other formats."""
    from core.media_utils import SUPPORTED_IMAGE_EXTENSIONS

    stem, src_ext = os.path.splitext(photo_path)
    src_ext_lower = src_ext.lower()
    companions: list[str] = []
    seen_norm: set = set()

    for xmp_ext in (".xmp", ".XMP"):
        c = stem + xmp_ext
        norm = os.path.normcase(c)
        if norm not in seen_norm and os.path.isfile(c):
            companions.append(c)
            seen_norm.add(norm)
            break

    for ext in SUPPORTED_IMAGE_EXTENSIONS:
        if ext == src_ext_lower:
            continue
        for candidate in (stem + ext, stem + ext.upper()):
            norm = os.path.normcase(candidate)
            if norm in seen_norm:
                continue
            if os.path.isfile(candidate):
                companions.append(candidate)
                seen_norm.add(norm)
                break

    return companions


def _move_companion_files_if_present(source_path: str, destination_dir: str) -> None:
    for companion in _find_companion_files(source_path):
        dest = _resolve_collision_safe_destination(
            destination_dir, os.path.basename(companion)
        )
        success, message = ImageFileOperations.move_path(companion, dest)
        if not success:
            raise RuntimeError(message)


def _iter_parent_directories(path: str, *, stop_at: str) -> Iterable[str]:
    normalized_stop = os.path.normcase(os.path.normpath(stop_at))
    current = os.path.normpath(path)
    while current:
        normalized_current = os.path.normcase(os.path.normpath(current))
        if normalized_current == normalized_stop:
            break
        yield current
        parent = os.path.dirname(current)
        if parent == current:
            break
        current = parent


def _empty_directory_candidates_from_entries(
    entries: Sequence[GroupingRunEntry],
    *,
    source_root: str,
) -> list[str]:
    candidates: dict[str, str] = {}
    for entry in entries:
        if entry.status not in {"moved", "unassigned", "deleted"}:
            continue
        original_path = (entry.original_path or "").strip()
        if not original_path:
            continue
        original_dir = os.path.dirname(original_path)
        for candidate in _iter_parent_directories(original_dir, stop_at=source_root):
            normalized = os.path.normcase(os.path.normpath(candidate))
            candidates[normalized] = candidate
    return list(candidates.values())


def _prepare_delete_targets(paths: Sequence[str]) -> list[str]:
    unique_paths: list[str] = []
    seen: set[str] = set()
    for path in paths:
        normalized = os.path.normcase(os.path.normpath(path))
        if not path or normalized in seen:
            continue
        seen.add(normalized)
        unique_paths.append(path)

    directory_targets = [path for path in unique_paths if os.path.isdir(path)]
    filtered_paths: list[str] = []
    for path in unique_paths:
        if any(
            not _is_same_path(path, directory_path)
            and _is_path_within_dir(path, directory_path)
            for directory_path in directory_targets
        ):
            continue
        filtered_paths.append(path)

    return sorted(
        filtered_paths,
        key=lambda path: (
            0 if os.path.isdir(path) else 1,
            -path.count(os.sep),
            path,
        ),
    )


def _cluster_vectors(
    vectors_by_path: dict[str, np.ndarray],
    *,
    eps: float,
    min_samples: int = 1,
) -> dict[str, int]:
    from sklearn.cluster import DBSCAN

    if not vectors_by_path:
        return {}
    paths = list(vectors_by_path.keys())
    matrix = np.stack([vectors_by_path[path] for path in paths])
    if len(paths) == 1:
        return {paths[0]: 1}
    clustering = DBSCAN(eps=eps, min_samples=min_samples, metric="cosine")
    labels = clustering.fit_predict(matrix)
    label_to_cluster_id: dict[int, int] = {}
    next_cluster_id = 1
    assignments: dict[str, int] = {}
    for idx, label in enumerate(labels):
        if label == -1:
            continue
        if label not in label_to_cluster_id:
            label_to_cluster_id[label] = next_cluster_id
            next_cluster_id += 1
        assignments[paths[idx]] = label_to_cluster_id[label]
    return assignments


def _load_image_for_features(path: str) -> Image.Image | None:
    try:
        with Image.open(path) as img:
            return img.convert("RGB")
    except Exception:
        logger.debug(
            "Unable to load image for grouping features: %s",
            path,
            exc_info=True,
        )
        return None


@lru_cache(maxsize=1)
def _load_opencv_face_cascades() -> tuple[Any, ...]:
    try:
        import cv2  # type: ignore
    except Exception:
        logger.debug("OpenCV unavailable for face grouping", exc_info=True)
        return ()

    cascade_dir = getattr(getattr(cv2, "data", None), "haarcascades", "")
    cascade_names = (
        "haarcascade_frontalface_alt2.xml",
        "haarcascade_frontalface_default.xml",
        "haarcascade_profileface.xml",
    )
    cascades: list[Any] = []
    for name in cascade_names:
        cascade_path = os.path.join(cascade_dir, name)
        if not os.path.exists(cascade_path):
            continue
        cascade = cv2.CascadeClassifier(cascade_path)
        if getattr(cascade, "empty", lambda: True)():
            continue
        cascades.append(cascade)
    return tuple(cascades)


def _score_face_candidate(
    bbox: tuple[int, int, int, int],
    *,
    image_width: int,
    image_height: int,
) -> float:
    x, y, w, h = bbox
    area = float(w * h)
    center_x = x + (w / 2.0)
    center_y = y + (h / 2.0)
    dx = abs(center_x - (image_width / 2.0)) / max(image_width / 2.0, 1.0)
    dy = abs(center_y - (image_height / 2.2)) / max(image_height / 2.2, 1.0)
    center_bonus = max(0.0, 1.0 - ((dx * 0.7) + (dy * 0.3)))
    return area * (1.0 + center_bonus)


def _detect_primary_face_bbox(
    image: Image.Image,
) -> tuple[int, int, int, int] | None:
    cascades = _load_opencv_face_cascades()
    if not cascades:
        return None

    try:
        import cv2  # type: ignore
    except Exception:
        return None

    gray = np.asarray(image.convert("L"), dtype=np.uint8)
    equalized = cv2.equalizeHist(gray)
    min_side = min(image.size)
    min_face = max(24, int(min_side * 0.12))
    candidates: list[tuple[int, int, int, int]] = []
    for cascade in cascades:
        for source in (equalized, gray):
            detected = cascade.detectMultiScale(
                source,
                scaleFactor=1.05,
                minNeighbors=4,
                minSize=(min_face, min_face),
            )
            if detected is None or len(detected) == 0:
                continue
            candidates.extend(tuple(int(v) for v in bbox) for bbox in detected)

    if not candidates:
        return None

    width, height = image.size
    return max(
        candidates,
        key=lambda bbox: _score_face_candidate(
            bbox, image_width=width, image_height=height
        ),
    )


def _expand_bbox(
    bbox: tuple[int, int, int, int],
    *,
    image_size: tuple[int, int],
    horizontal_pad: float = 0.22,
    top_pad: float = 0.32,
    bottom_pad: float = 0.18,
) -> tuple[int, int, int, int]:
    x, y, w, h = bbox
    image_width, image_height = image_size
    left = max(0, int(round(x - (w * horizontal_pad))))
    top = max(0, int(round(y - (h * top_pad))))
    right = min(image_width, int(round(x + w + (w * horizontal_pad))))
    bottom = min(image_height, int(round(y + h + (h * bottom_pad))))
    return left, top, right, bottom


def _compute_face_vector_from_crop(
    crop: Image.Image,
) -> tuple[np.ndarray | None, bool]:
    face_crop = crop.convert("L").resize((48, 48))
    arr = np.asarray(face_crop, dtype=np.float32) / 255.0
    variance = float(arr.var())
    gx = float(np.abs(np.diff(arr, axis=1)).mean())
    gy = float(np.abs(np.diff(arr, axis=0)).mean())
    edge_strength = (gx + gy) / 2.0
    has_face_like_signal = variance >= 0.0025 and edge_strength >= 0.02
    if not has_face_like_signal:
        return None, False

    coarse = (
        np.asarray(face_crop.resize((24, 24)), dtype=np.float32).reshape(-1) / 255.0
    )
    histogram, _ = np.histogram(arr, bins=16, range=(0.0, 1.0), density=True)
    vector = np.concatenate([coarse, histogram.astype(np.float32)])
    norm = np.linalg.norm(vector)
    if norm <= 0:
        return None, False
    return vector / norm, True


def _compute_face_vector(
    path: str,
    image_pipeline: ImagePipeline | None = None,
) -> tuple[np.ndarray | None, bool]:
    if image_pipeline is not None:
        from core.image_pipeline import ANALYSIS_CACHE_RESOLUTION

        image = image_pipeline.get_analysis_image(
            path,
            target_size=ANALYSIS_CACHE_RESOLUTION,
        )
    else:
        image = _load_image_for_features(path)
    if image is None:
        return None, False
    width, height = image.size
    if width < 32 or height < 32:
        return None, False
    detected_bbox = _detect_primary_face_bbox(image)
    if detected_bbox is not None:
        crop_bounds = _expand_bbox(detected_bbox, image_size=image.size)
        return _compute_face_vector_from_crop(image.crop(crop_bounds))

    crop_w = max(32, int(width * 0.62))
    crop_h = max(32, int(height * 0.68))
    left = max(0, (width - crop_w) // 2)
    top = max(0, int((height - crop_h) * 0.18))
    right = min(width, left + crop_w)
    bottom = min(height, top + crop_h)
    return _compute_face_vector_from_crop(image.crop((left, top, right, bottom)))


def _parse_gps_coordinate(raw_value: Any, ref_value: Any) -> float | None:
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


_reverse_geocode_available: bool | None = None
_reverse_geocode_resolved_count: int = 0
_REVERSE_GEOCODE_LOG_INTERVAL: int = 10


def _ensure_reverse_geocode() -> bool:
    global _reverse_geocode_available
    if _reverse_geocode_available is None:
        try:
            import reverse_geocode as _rg  # noqa: F401

            _reverse_geocode_available = True
        except ImportError:
            _reverse_geocode_available = False
            logger.warning("reverse-geocode not installed — location names unavailable")
    return _reverse_geocode_available


@lru_cache(maxsize=512)
def _reverse_geocode_parts(lat_bucket: float, lon_bucket: float) -> tuple:
    """Returns (country, state, city) strings or None for missing levels."""
    global _reverse_geocode_resolved_count
    if not _ensure_reverse_geocode():
        return (None, None, None)

    result = _reverse_geocode_offline(lat_bucket, lon_bucket)
    _reverse_geocode_resolved_count += 1
    if _reverse_geocode_resolved_count % _REVERSE_GEOCODE_LOG_INTERVAL == 0:
        logger.info(
            "Resolved %d unique locations",
            _reverse_geocode_resolved_count,
        )
    return result


def _reverse_geocode_offline(lat_bucket: float, lon_bucket: float) -> tuple:
    try:
        import reverse_geocode as rg

        r = rg.get((lat_bucket, lon_bucket))
        if not r:
            return (None, None, None)
        country = r.get("country") or None
        state = r.get("state") or None
        city = r.get("city") or None
        return (
            _sanitize_folder_component(country) if country else None,
            _sanitize_folder_component(state) if state else None,
            _sanitize_folder_component(city) if city else None,
        )
    except Exception:
        logger.debug(
            "Reverse geocode failed for %.2f, %.2f",
            lat_bucket,
            lon_bucket,
            exc_info=True,
        )
        return (None, None, None)


def _location_label_from_metadata(
    metadata: dict[str, Any], depth: int = 3
) -> str | None:
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

    country, state, city = _reverse_geocode_parts(lat_bucket, lon_bucket)
    parts = [country, city] if depth == 2 else [country, state, city]
    available = [p for p in parts if p]
    if available:
        selected = available[:depth] if depth < len(available) else available
        return "/".join(selected)

    return f"Lat_{lat_bucket:.2f}_Lon_{lon_bucket:.2f}"


def _load_comprehensive_metadata(path: str) -> dict[str, Any]:
    try:
        resolved = MetadataProcessor._resolve_path_forms(path)  # type: ignore[attr-defined]
        operational = resolved[0] if resolved else path
        from core.pyexiv2_wrapper import PyExiv2Operations

        return PyExiv2Operations.get_comprehensive_metadata(operational)
    except Exception:
        logger.debug("Comprehensive metadata fetch failed for %s", path, exc_info=True)
        return {}


def _build_groups_from_assignments(
    assignments: dict[str, int],
    label_builder,
) -> list[GroupingGroup]:
    grouped: dict[int, list[str]] = {}
    for path, cluster_id in assignments.items():
        grouped.setdefault(cluster_id, []).append(path)
    return [
        GroupingGroup(
            group_id=str(cluster_id),
            group_label=label_builder(cluster_id),
            source_paths=sorted(grouped[cluster_id]),
        )
        for cluster_id in sorted(grouped)
    ]


def _parse_cluster_id(value: Any) -> int | None:
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
    shared_engine: SimilarityEngine | None = None,
    image_pipeline: ImagePipeline | None = None,
) -> dict[str, int]:
    if not image_paths:
        return {}
    if shared_engine is None:
        from core.similarity_engine import SimilarityEngine

        engine = SimilarityEngine(image_pipeline=image_pipeline)
    else:
        engine = shared_engine
    _embeddings, cluster_results = engine.run_analysis_sync(
        list(image_paths),
        progress_callback=progress_callback,
    )
    assignments: dict[str, int] = {}
    for path, raw_cluster in cluster_results.items():
        cluster_id = _parse_cluster_id(raw_cluster)
        if cluster_id is not None:
            assignments[path] = cluster_id
    return assignments


def build_grouping_plan(
    items: Sequence[dict[str, Any]],
    mode: GroupingMode | str,
    progress_callback=None,
    source_root: str | None = None,
    location_depth: int = 3,
    image_pipeline: ImagePipeline | None = None,
) -> GroupingPlan:
    mode_value = GroupingMode(mode)
    valid_items = [
        item for item in items if isinstance(item, dict) and item.get("path")
    ]
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
        return _build_location_plan(
            total_items, image_paths, skipped_paths, location_depth
        )
    if mode_value == GroupingMode.FACE:
        return _build_face_plan(
            total_items,
            image_paths,
            skipped_paths,
            image_pipeline=image_pipeline,
        )
    if mode_value == GroupingMode.MIXED:
        return _build_mixed_plan(
            total_items,
            image_paths,
            skipped_paths,
            progress_callback=progress_callback,
            image_pipeline=image_pipeline,
        )
    return _build_similarity_plan(
        total_items,
        image_paths,
        skipped_paths,
        progress_callback=progress_callback,
        image_pipeline=image_pipeline,
    )


def _build_current_structure_plan(
    total_items: int,
    image_paths: Sequence[str],
    skipped_paths: Sequence[str],
    *,
    source_root: str | None = None,
) -> GroupingPlan:
    groups_by_label: dict[str, list[str]] = {}
    resolved_source_root = (
        os.path.normpath(source_root)
        if source_root
        else (os.path.commonpath(list(image_paths)) if image_paths else "")
    )
    for path in image_paths:
        parent_dir = os.path.dirname(path)
        if resolved_source_root:
            try:
                rel_dir = os.path.relpath(parent_dir, resolved_source_root)
            except Exception:
                rel_dir = parent_dir
        else:
            rel_dir = parent_dir
        label = "" if rel_dir in {".", ""} else rel_dir
        groups_by_label.setdefault(label, []).append(path)

    groups: list[GroupingGroup] = []
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
    image_pipeline: ImagePipeline | None = None,
) -> GroupingPlan:
    assignments = _run_ml_similarity_pipeline(
        image_paths,
        progress_callback=progress_callback,
        image_pipeline=image_pipeline,
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
    image_pipeline: ImagePipeline | None = None,
) -> GroupingPlan:
    vectors: dict[str, np.ndarray] = {}
    unassigned: list[str] = []
    for path in image_paths:
        vector, has_face_like_signal = _compute_face_vector(
            path,
            image_pipeline=image_pipeline,
        )
        if vector is None or not has_face_like_signal:
            unassigned.append(path)
            continue
        vectors[path] = vector
    assignments = _cluster_vectors(vectors, eps=0.16, min_samples=1)
    assigned_paths = set(assignments)
    unassigned.extend(path for path in vectors if path not in assigned_paths)
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
    location_depth: int = 3,
) -> GroupingPlan:
    buckets: dict[str, list[str]] = {}
    unassigned: list[str] = []
    for path in image_paths:
        metadata = _load_comprehensive_metadata(path)
        label = _location_label_from_metadata(metadata, depth=location_depth)
        if not label:
            unassigned.append(path)
            continue
        buckets.setdefault(label, []).append(path)
    groups: list[GroupingGroup] = []
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


def _extract_date_label(path: str) -> str | None:
    """Extract a YYYY-MM-DD date label from EXIF, filename, or filesystem mtime."""
    metadata = _load_comprehensive_metadata(path)
    for tag in DATE_TAGS_PREFERENCE:
        raw = metadata.get(tag)
        if raw:
            parsed = _parse_exif_date(str(raw))
            if parsed:
                return parsed.strftime("%Y-%m-%d")
    filename = os.path.basename(path)
    parsed = _parse_date_from_filename(filename)
    if parsed:
        return parsed.strftime("%Y-%m-%d")
    try:
        mtime = os.path.getmtime(path)
        return datetime.fromtimestamp(mtime).strftime("%Y-%m-%d")
    except OSError:
        return None


def _build_mixed_plan(
    total_items: int,
    image_paths: Sequence[str],
    skipped_paths: Sequence[str],
    progress_callback=None,
    image_pipeline: ImagePipeline | None = None,
) -> GroupingPlan:
    date_buckets: dict[str, list[str]] = {}
    undated: list[str] = []
    for path in image_paths:
        label = _extract_date_label(path)
        if not label:
            undated.append(path)
            continue
        date_buckets.setdefault(label, []).append(path)

    groups: list[GroupingGroup] = []
    group_counter = 1
    for date_label in sorted(date_buckets.keys()):
        bucket_paths = date_buckets[date_label]

        def _bucket_progress(
            percent: int, message: str, bucket_label: str = date_label
        ):
            if progress_callback:
                progress_callback(percent, f"{bucket_label}: {message}")

        assignments = _run_ml_similarity_pipeline(
            bucket_paths,
            progress_callback=_bucket_progress if progress_callback else None,
            image_pipeline=image_pipeline,
        )
        grouped_by_cluster: dict[int, list[str]] = {}
        for path, cluster_id in assignments.items():
            grouped_by_cluster.setdefault(cluster_id, []).append(path)

        # Files not picked up by the similarity pipeline go to the date folder directly
        standalone: list[str] = [p for p in bucket_paths if p not in assignments]

        local_group_idx = 1
        for cluster_id in sorted(grouped_by_cluster.keys()):
            cluster_paths = grouped_by_cluster[cluster_id]
            if len(cluster_paths) < 2:
                standalone.extend(cluster_paths)
                continue
            groups.append(
                GroupingGroup(
                    group_id=str(group_counter),
                    group_label=os.path.join(date_label, f"group-{local_group_idx}"),
                    source_paths=sorted(cluster_paths),
                )
            )
            group_counter += 1
            local_group_idx += 1

        if standalone:
            groups.append(
                GroupingGroup(
                    group_id=str(group_counter),
                    group_label=date_label,
                    source_paths=sorted(standalone),
                )
            )
            group_counter += 1

    if undated:
        groups.append(
            GroupingGroup(
                group_id=str(group_counter),
                group_label="undated",
                source_paths=sorted(undated),
            )
        )

    return GroupingPlan(
        mode=GroupingMode.MIXED.value,
        total_items=total_items,
        supported_items=len(image_paths),
        groups=groups,
        unassigned_paths=[],
        skipped_paths=list(skipped_paths),
    )


def execute_grouping_plan(
    plan: GroupingPlan,
    *,
    source_root: str,
    output_root: str,
    progress_callback=None,
    move_companions: bool = False,
) -> GroupingRunSummary:
    os.makedirs(output_root, exist_ok=True)
    entries: list[GroupingRunEntry] = []
    directory_renames = find_directory_rename_candidates(
        plan,
        source_root=source_root,
        output_root=output_root,
    )
    delete_targets = _prepare_delete_targets(getattr(plan, "deleted_paths", []) or [])
    total_moves = (
        sum(len(group.source_paths) for group in plan.groups)
        + len(plan.unassigned_paths)
        + len(delete_targets)
    )
    moved_count = 0
    deleted_count = 0

    def report(message: str) -> None:
        if progress_callback:
            percent = int((moved_count / total_moves) * 100) if total_moves else 100
            progress_callback(percent, message)

    for group in plan.groups:
        directory_rename = directory_renames.get(str(group.group_id))
        destination_dir = (
            directory_rename.target_dir
            if directory_rename is not None
            else os.path.join(
                output_root,
                *[
                    _sanitize_folder_component(part)
                    for part in re.split(r"[\\/]+", group.group_label)
                    if part
                ],
            )
        )
        group.destination_folder = destination_dir
        if directory_rename is not None:
            success, message = ImageFileOperations.move_path(
                directory_rename.source_dir, destination_dir
            )
            if not success:
                raise RuntimeError(message)
            for source_path in group.source_paths:
                basename = plan.filename_for_path(source_path)
                destination_path = os.path.join(destination_dir, basename)
                moved_count += 1
                entries.append(
                    GroupingRunEntry(
                        original_path=source_path,
                        new_path=destination_path,
                        group_id=group.group_id,
                        group_label=group.group_label,
                        status="moved",
                    )
                )
                report(f"Renaming folder for {basename}...")
            for skipped_path in plan.skipped_paths:
                if _is_path_within_dir(skipped_path, directory_rename.source_dir):
                    skipped_rel_path = os.path.relpath(
                        skipped_path, directory_rename.source_dir
                    )
                    entries.append(
                        GroupingRunEntry(
                            original_path=skipped_path,
                            new_path=os.path.join(destination_dir, skipped_rel_path),
                            group_id=None,
                            group_label=None,
                            status="skipped",
                            reason="unsupported media",
                        )
                    )
            continue
        for source_path in group.source_paths:
            basename = plan.filename_for_path(source_path)
            desired_destination_path = os.path.join(destination_dir, basename)
            if os.path.normcase(os.path.normpath(source_path)) == os.path.normcase(
                os.path.normpath(desired_destination_path)
            ):
                entries.append(
                    GroupingRunEntry(
                        original_path=source_path,
                        new_path=source_path,
                        group_id=group.group_id,
                        group_label=group.group_label,
                        status="unchanged",
                    )
                )
                report(f"Keeping {basename} in place...")
                continue
            destination_path = _resolve_collision_safe_destination(
                destination_dir, basename
            )
            success, message = ImageFileOperations.move_path(
                source_path, destination_path
            )
            if not success:
                raise RuntimeError(message)
            if move_companions:
                _move_companion_files_if_present(source_path, destination_dir)
            moved_count += 1
            entries.append(
                GroupingRunEntry(
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
            basename = plan.filename_for_path(source_path)
            destination_path = _resolve_collision_safe_destination(
                unassigned_dir, basename
            )
            success, message = ImageFileOperations.move_path(
                source_path, destination_path
            )
            if not success:
                raise RuntimeError(message)
            if move_companions:
                _move_companion_files_if_present(source_path, unassigned_dir)
            moved_count += 1
            entries.append(
                GroupingRunEntry(
                    original_path=source_path,
                    new_path=destination_path,
                    group_id=None,
                    group_label="Unassigned",
                    status="unassigned",
                )
            )
            report(f"Moving unassigned {basename}...")

    for target_path in delete_targets:
        basename = os.path.basename(target_path.rstrip(os.sep)) or target_path
        success, message = ImageFileOperations.move_to_trash(target_path)
        if not success:
            raise RuntimeError(message or f"Failed to delete {target_path}.")
        deleted_count += 1
        entries.append(
            GroupingRunEntry(
                original_path=target_path,
                new_path=None,
                group_id=None,
                group_label=None,
                status="deleted",
            )
        )
        report(f"Deleting {basename}...")

    renamed_skipped_paths = {
        entry.original_path
        for entry in entries
        if entry.status == "skipped" and entry.new_path is not None
    }
    for source_path in plan.skipped_paths:
        if source_path in renamed_skipped_paths:
            continue
        entries.append(
            GroupingRunEntry(
                original_path=source_path,
                new_path=None,
                group_id=None,
                group_label=None,
                status="skipped",
                reason="unsupported media",
            )
        )

    _remove_empty_directories(
        source_root,
        candidate_dirs=_empty_directory_candidates_from_entries(
            entries,
            source_root=source_root,
        ),
    )

    summary = GroupingRunSummary(
        mode=plan.mode,
        source_root=source_root,
        output_root=output_root,
        moved_count=moved_count,
        deleted_count=deleted_count,
        unassigned_count=len(plan.unassigned_paths),
        skipped_count=len(plan.skipped_paths),
        groups=plan.groups,
        entries=entries,
    )
    return summary


def _remove_empty_directories(
    root_path: str,
    *,
    candidate_dirs: Sequence[str] | None = None,
) -> None:
    if not root_path or not os.path.isdir(root_path):
        return
    normalized_root = os.path.normcase(os.path.normpath(root_path))
    if candidate_dirs is None:
        walk_roots = [
            current_root for current_root, _dirnames, _filenames in os.walk(root_path)
        ]
    else:
        walk_roots = sorted(
            {
                os.path.normpath(path)
                for path in candidate_dirs
                if path
                and _is_path_within_dir(path, root_path)
                and os.path.normcase(os.path.normpath(path)) != normalized_root
            },
            key=lambda path: path.count(os.sep),
            reverse=True,
        )
    for current_root in walk_roots:
        normalized_current = os.path.normcase(os.path.normpath(current_root))
        if normalized_current == normalized_root:
            continue
        try:
            if not os.path.isdir(current_root):
                continue
            if os.listdir(current_root):
                continue
            ImageFileOperations.remove_empty_directory(current_root)
        except OSError:
            continue

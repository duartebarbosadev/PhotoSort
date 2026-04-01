import json
import os

import numpy as np
from PIL import Image, ImageDraw

from src.core.grouping import (
    GroupingMode,
    GroupingGroup,
    GroupingPlan,
    build_grouping_output_root,
    build_grouping_plan,
    execute_grouping_plan,
)


def _create_solid_image(path: str, color: tuple[int, int, int]) -> None:
    Image.new("RGB", (120, 120), color).save(path)


def _create_face_like_image(path: str, eye_offset: int = 0) -> None:
    image = Image.new("RGB", (160, 160), (230, 210, 190))
    draw = ImageDraw.Draw(image)
    draw.ellipse((35 + eye_offset, 45, 60 + eye_offset, 70), fill=(20, 20, 20))
    draw.ellipse((95 + eye_offset, 45, 120 + eye_offset, 70), fill=(20, 20, 20))
    draw.arc((50, 80, 110, 120), 10, 170, fill=(80, 20, 20), width=5)
    image.save(path)


def _create_offset_face_like_image(
    path: str,
    shift_x: int = 0,
    shift_y: int = 0,
) -> None:
    image = Image.new("RGB", (240, 180), (245, 245, 245))
    draw = ImageDraw.Draw(image)
    face_left = 20 + shift_x
    face_top = 20 + shift_y
    draw.ellipse(
        (face_left, face_top, face_left + 90, face_top + 110),
        fill=(230, 210, 190),
    )
    draw.ellipse(
        (face_left + 18, face_top + 28, face_left + 34, face_top + 44),
        fill=(20, 20, 20),
    )
    draw.ellipse(
        (face_left + 56, face_top + 28, face_left + 72, face_top + 44),
        fill=(20, 20, 20),
    )
    draw.arc(
        (face_left + 20, face_top + 56, face_left + 72, face_top + 90),
        15,
        165,
        fill=(80, 20, 20),
        width=4,
    )
    image.save(path)


def test_similarity_grouping_plan_uses_ml_similarity_pipeline(tmp_path, monkeypatch):
    red_a = tmp_path / "red_a.jpg"
    red_b = tmp_path / "red_b.jpg"
    blue_a = tmp_path / "blue_a.jpg"
    blue_b = tmp_path / "blue_b.jpg"
    _create_solid_image(str(red_a), (220, 40, 40))
    _create_solid_image(str(red_b), (225, 45, 45))
    _create_solid_image(str(blue_a), (40, 40, 220))
    _create_solid_image(str(blue_b), (45, 45, 225))

    monkeypatch.setattr(
        "src.core.grouping._run_ml_similarity_pipeline",
        lambda paths, progress_callback=None, shared_engine=None: {
            str(red_a): 1,
            str(red_b): 1,
            str(blue_a): 2,
            str(blue_b): 2,
        },
    )

    plan = build_grouping_plan(
        [
            {"path": str(red_a)},
            {"path": str(red_b)},
            {"path": str(blue_a)},
            {"path": str(blue_b)},
        ],
        GroupingMode.SIMILARITY,
    )

    assert len(plan.groups) == 2
    sizes = sorted(len(group.source_paths) for group in plan.groups)
    assert sizes == [2, 2]
    assert plan.unassigned_paths == []


def test_current_structure_grouping_plan_preserves_relative_folders(tmp_path):
    source_root = tmp_path / "source"
    day_one = source_root / "day_one"
    day_two = source_root / "day_two"
    day_one.mkdir(parents=True)
    day_two.mkdir(parents=True)
    first = day_one / "a.jpg"
    second = day_two / "b.jpg"
    _create_solid_image(str(first), (220, 40, 40))
    _create_solid_image(str(second), (40, 40, 220))

    plan = build_grouping_plan(
        [{"path": str(first)}, {"path": str(second)}],
        GroupingMode.CURRENT,
        source_root=str(source_root),
    )

    labels = sorted(group.group_label for group in plan.groups)
    assert labels == ["day_one", "day_two"]


def test_current_structure_grouping_plan_keeps_root_level_files_at_root(tmp_path):
    source_root = tmp_path / "source"
    source_root.mkdir(parents=True)
    first = source_root / "a.jpg"
    _create_solid_image(str(first), (220, 40, 40))

    plan = build_grouping_plan(
        [{"path": str(first)}],
        GroupingMode.CURRENT,
        source_root=str(source_root),
    )

    assert len(plan.groups) == 1
    assert plan.groups[0].group_label == ""

    summary = execute_grouping_plan(
        plan,
        source_root=str(source_root),
        output_root=str(source_root),
    )

    assert summary.moved_count == 0
    assert first.exists()
    assert summary.entries[0].status == "unchanged"
    assert summary.entries[0].new_path == str(first)


def test_face_grouping_plan_assigns_face_like_images_and_unassigns_flat_image(tmp_path):
    face_a = tmp_path / "face_a.jpg"
    face_b = tmp_path / "face_b.jpg"
    blank = tmp_path / "blank.jpg"
    _create_face_like_image(str(face_a), 0)
    _create_face_like_image(str(face_b), 1)
    _create_solid_image(str(blank), (180, 180, 180))

    plan = build_grouping_plan(
        [{"path": str(face_a)}, {"path": str(face_b)}, {"path": str(blank)}],
        GroupingMode.FACE,
    )

    assert len(plan.groups) == 1
    assert len(plan.groups[0].source_paths) == 2
    assert str(blank) in plan.unassigned_paths


def test_face_grouping_plan_uses_detected_face_region_for_offset_faces(
    tmp_path, monkeypatch
):
    face_a = tmp_path / "offset_face_a.jpg"
    face_b = tmp_path / "offset_face_b.jpg"
    blank = tmp_path / "blank.jpg"
    _create_offset_face_like_image(str(face_a), 0, 0)
    _create_offset_face_like_image(str(face_b), 8, 6)
    _create_solid_image(str(blank), (240, 240, 240))

    bbox_by_size = {
        (240, 180): (18, 18, 100, 120),
    }

    monkeypatch.setattr(
        "src.core.grouping._detect_primary_face_bbox",
        lambda image: bbox_by_size.get(image.size),
    )
    monkeypatch.setattr(
        "src.core.grouping._compute_face_vector_from_crop",
        lambda crop: (
            (np.array([1.0, 0.0], dtype=np.float32), True)
            if crop.size == (140, 160)
            else (None, False)
        ),
    )

    plan = build_grouping_plan(
        [{"path": str(face_a)}, {"path": str(face_b)}, {"path": str(blank)}],
        GroupingMode.FACE,
    )

    assert len(plan.groups) == 1
    assert sorted(plan.groups[0].source_paths) == sorted([str(face_a), str(face_b)])
    assert str(blank) in plan.unassigned_paths


def test_location_grouping_plan_uses_metadata_and_marks_missing_items_unassigned(
    tmp_path, monkeypatch
):
    a = tmp_path / "a.jpg"
    b = tmp_path / "b.jpg"
    c = tmp_path / "c.jpg"
    _create_solid_image(str(a), (10, 20, 30))
    _create_solid_image(str(b), (20, 30, 40))
    _create_solid_image(str(c), (30, 40, 50))

    metadata_by_path = {
        str(a): {
            "Exif.GPSInfo.GPSLatitude": "(27/1, 28/1, 0/1)",
            "Exif.GPSInfo.GPSLatitudeRef": "S",
            "Exif.GPSInfo.GPSLongitude": "(153/1, 2/1, 0/1)",
            "Exif.GPSInfo.GPSLongitudeRef": "E",
        },
        str(b): {
            "Exif.GPSInfo.GPSLatitude": "(27/1, 28/1, 10/1)",
            "Exif.GPSInfo.GPSLatitudeRef": "S",
            "Exif.GPSInfo.GPSLongitude": "(153/1, 2/1, 5/1)",
            "Exif.GPSInfo.GPSLongitudeRef": "E",
        },
        str(c): {},
    }

    monkeypatch.setattr(
        "src.core.grouping._load_comprehensive_metadata",
        lambda path: metadata_by_path[str(path)],
    )

    plan = build_grouping_plan(
        [{"path": str(a)}, {"path": str(b)}, {"path": str(c)}],
        GroupingMode.LOCATION,
    )

    assert len(plan.groups) == 1
    assert len(plan.groups[0].source_paths) == 2
    assert plan.unassigned_paths == [str(c)]


def test_mixed_grouping_partitions_by_date_then_similarity(tmp_path, monkeypatch):
    a = tmp_path / "day1_red.jpg"
    b = tmp_path / "day1_blue.jpg"
    c = tmp_path / "day2_red.jpg"
    _create_solid_image(str(a), (230, 20, 20))
    _create_solid_image(str(b), (20, 20, 230))
    _create_solid_image(str(c), (235, 25, 25))

    date_by_path = {
        str(a): "2025-03-15",
        str(b): "2025-03-15",
        str(c): "2025-03-16",
    }
    monkeypatch.setattr(
        "src.core.grouping._extract_date_label",
        lambda path: date_by_path[str(path)],
    )
    monkeypatch.setattr(
        "src.core.grouping._run_ml_similarity_pipeline",
        lambda paths, progress_callback=None, shared_engine=None: (
            {str(a): 1, str(b): 1} if set(paths) == {str(a), str(b)} else {str(c): 1}
        ),
    )

    plan = build_grouping_plan(
        [{"path": str(a)}, {"path": str(b)}, {"path": str(c)}],
        GroupingMode.MIXED,
    )

    # day1 has a cluster of 2 -> group-1, day2 has singleton -> date folder directly
    cluster_groups = [g for g in plan.groups if "group-" in g.group_label]
    date_groups = [g for g in plan.groups if g.group_label == "2025-03-16"]
    assert len(cluster_groups) == 1
    assert cluster_groups[0].group_label == os.path.join("2025-03-15", "group-1")
    assert sorted(cluster_groups[0].source_paths) == sorted([str(a), str(b)])
    assert len(date_groups) == 1
    assert str(c) in date_groups[0].source_paths
    assert plan.unassigned_paths == []


def test_execute_grouping_plan_moves_files_handles_name_collisions_and_writes_manifest(
    tmp_path,
):
    source_root = tmp_path / "source"
    folder_a = source_root / "a"
    folder_b = source_root / "b"
    folder_a.mkdir(parents=True)
    folder_b.mkdir(parents=True)
    first = folder_a / "same.jpg"
    second = folder_b / "same.jpg"
    _create_solid_image(str(first), (220, 50, 50))
    _create_solid_image(str(second), (225, 55, 55))

    plan = GroupingPlan(
        mode=GroupingMode.SIMILARITY.value,
        total_items=2,
        supported_items=2,
        groups=[
            GroupingGroup(
                group_id="1",
                group_label="Group 001",
                source_paths=[str(first), str(second)],
            )
        ],
        unassigned_paths=[],
        skipped_paths=[],
    )
    output_root = build_grouping_output_root(str(source_root), "similarity")
    summary = execute_grouping_plan(
        plan, source_root=str(source_root), output_root=output_root
    )

    moved_paths = sorted(entry.new_path for entry in summary.entries if entry.new_path)
    assert len(moved_paths) == 2
    assert os.path.exists(moved_paths[0])
    assert os.path.exists(moved_paths[1])
    assert moved_paths[0] != moved_paths[1]
    assert os.path.exists(summary.manifest_path)
    with open(summary.manifest_path, "r", encoding="utf-8") as fh:
        manifest = json.load(fh)
    assert manifest["moved_count"] == 2
    assert len(manifest["entries"]) == 2


def test_build_grouping_output_root_uses_source_root_directly(tmp_path):
    source_root = tmp_path / "source"
    source_root.mkdir()

    output_root = build_grouping_output_root(str(source_root), "similarity")

    assert output_root == str(source_root)


def test_execute_grouping_plan_keeps_current_mode_files_in_place_when_already_grouped(
    tmp_path,
):
    source_root = tmp_path / "source"
    day_one = source_root / "day_one"
    day_one.mkdir(parents=True)
    image_path = day_one / "a.jpg"
    _create_solid_image(str(image_path), (220, 40, 40))

    plan = build_grouping_plan(
        [{"path": str(image_path)}],
        GroupingMode.CURRENT,
        source_root=str(source_root),
    )
    summary = execute_grouping_plan(
        plan,
        source_root=str(source_root),
        output_root=str(source_root),
    )

    assert summary.moved_count == 0
    assert image_path.exists()
    assert summary.entries[0].status == "unchanged"
    assert summary.entries[0].new_path == str(image_path)


def test_execute_grouping_plan_removes_empty_source_directories(tmp_path):
    source_root = tmp_path / "source"
    old_dir = source_root / "old_folder"
    new_dir = source_root / "new_folder"
    old_dir.mkdir(parents=True)
    new_dir.mkdir(parents=True)
    image_path = old_dir / "a.jpg"
    _create_solid_image(str(image_path), (220, 40, 40))

    plan = GroupingPlan(
        mode=GroupingMode.SIMILARITY.value,
        total_items=1,
        supported_items=1,
        groups=[
            GroupingGroup(
                group_id="1",
                group_label="new_folder",
                source_paths=[str(image_path)],
            )
        ],
        unassigned_paths=[],
        skipped_paths=[],
    )

    summary = execute_grouping_plan(
        plan,
        source_root=str(source_root),
        output_root=str(source_root),
    )

    assert summary.moved_count == 1
    assert not old_dir.exists()
    assert (new_dir / "a.jpg").exists()


def test_execute_grouping_plan_keeps_unrelated_empty_directories(tmp_path):
    source_root = tmp_path / "source"
    old_dir = source_root / "old_folder"
    new_dir = source_root / "new_folder"
    placeholder_dir = source_root / "placeholder"
    old_dir.mkdir(parents=True)
    new_dir.mkdir(parents=True)
    placeholder_dir.mkdir(parents=True)
    image_path = old_dir / "a.jpg"
    _create_solid_image(str(image_path), (220, 40, 40))

    plan = GroupingPlan(
        mode=GroupingMode.SIMILARITY.value,
        total_items=1,
        supported_items=1,
        groups=[
            GroupingGroup(
                group_id="1",
                group_label="new_folder",
                source_paths=[str(image_path)],
            )
        ],
        unassigned_paths=[],
        skipped_paths=[],
    )

    execute_grouping_plan(
        plan,
        source_root=str(source_root),
        output_root=str(source_root),
    )

    assert placeholder_dir.exists()


def test_execute_grouping_plan_applies_file_name_overrides(tmp_path):
    source_root = tmp_path / "source"
    old_dir = source_root / "old_folder"
    old_dir.mkdir(parents=True)
    image_path = old_dir / "a.jpg"
    _create_solid_image(str(image_path), (220, 40, 40))

    plan = build_grouping_plan(
        [{"path": str(image_path)}],
        GroupingMode.CURRENT,
        source_root=str(source_root),
    )
    plan.file_name_overrides[str(image_path)] = "renamed.jpg"

    summary = execute_grouping_plan(
        plan,
        source_root=str(source_root),
        output_root=str(source_root),
    )

    assert summary.moved_count == 1
    assert not image_path.exists()
    assert (old_dir / "renamed.jpg").exists()
    assert summary.entries[0].new_path == str(old_dir / "renamed.jpg")


def test_execute_grouping_plan_applies_pending_deletions(tmp_path, monkeypatch):
    source_root = tmp_path / "source"
    delete_dir = source_root / "delete_me"
    keep_dir = source_root / "keep"
    delete_dir.mkdir(parents=True)
    keep_dir.mkdir(parents=True)
    image_path = delete_dir / "a.jpg"
    keep_path = keep_dir / "b.jpg"
    _create_solid_image(str(image_path), (220, 40, 40))
    _create_solid_image(str(keep_path), (40, 40, 220))

    trashed_paths = []
    monkeypatch.setattr(
        "src.core.grouping.ImageFileOperations.move_to_trash",
        lambda path: (trashed_paths.append(path) or True, "Moved to trash."),
    )

    plan = GroupingPlan(
        mode=GroupingMode.CURRENT.value,
        total_items=2,
        supported_items=2,
        groups=[
            GroupingGroup(
                group_id="1", group_label="keep", source_paths=[str(keep_path)]
            )
        ],
        unassigned_paths=[],
        skipped_paths=[],
        deleted_paths=[str(delete_dir)],
    )

    summary = execute_grouping_plan(
        plan,
        source_root=str(source_root),
        output_root=str(source_root),
    )

    assert trashed_paths == [str(delete_dir)]
    assert summary.deleted_count == 1
    assert any(entry.status == "deleted" for entry in summary.entries)


def test_execute_grouping_plan_renames_entire_folder_and_keeps_unmanaged_files(
    tmp_path,
):
    source_root = tmp_path / "source"
    old_dir = source_root / "old_folder"
    old_dir.mkdir(parents=True)
    image_path = old_dir / "a.jpg"
    zip_path = old_dir / "archive.zip"
    _create_solid_image(str(image_path), (220, 40, 40))
    zip_path.write_bytes(b"zip-data")

    plan = build_grouping_plan(
        [{"path": str(image_path)}],
        GroupingMode.CURRENT,
        source_root=str(source_root),
    )
    plan.groups[0].group_label = "renamed_folder"

    summary = execute_grouping_plan(
        plan,
        source_root=str(source_root),
        output_root=str(source_root),
    )

    assert summary.moved_count == 1
    assert not old_dir.exists()
    assert (source_root / "renamed_folder" / "a.jpg").exists()
    assert (source_root / "renamed_folder" / "archive.zip").exists()


def test_execute_grouping_plan_falls_back_to_file_moves_for_duplicate_rename_targets(
    tmp_path,
):
    source_root = tmp_path / "source"
    dir_a = source_root / "A"
    dir_b = source_root / "B"
    dir_a.mkdir(parents=True)
    dir_b.mkdir(parents=True)
    first = dir_a / "a.jpg"
    second = dir_b / "b.jpg"
    _create_solid_image(str(first), (220, 40, 40))
    _create_solid_image(str(second), (40, 40, 220))

    plan = build_grouping_plan(
        [{"path": str(first)}, {"path": str(second)}],
        GroupingMode.CURRENT,
        source_root=str(source_root),
    )
    for group in plan.groups:
        group.group_label = "Merged"

    summary = execute_grouping_plan(
        plan,
        source_root=str(source_root),
        output_root=str(source_root),
    )

    assert summary.moved_count == 2
    assert (source_root / "Merged" / "a.jpg").exists()
    assert (source_root / "Merged" / "b.jpg").exists()
    assert not (source_root / "Merged" / "B").exists()


def test_execute_grouping_plan_merges_duplicate_rename_targets_with_name_collisions(
    tmp_path,
):
    source_root = tmp_path / "source"
    dir_a = source_root / "A"
    dir_b = source_root / "B"
    dir_a.mkdir(parents=True)
    dir_b.mkdir(parents=True)
    first = dir_a / "same.jpg"
    second = dir_b / "same.jpg"
    _create_solid_image(str(first), (220, 40, 40))
    _create_solid_image(str(second), (40, 40, 220))

    plan = build_grouping_plan(
        [{"path": str(first)}, {"path": str(second)}],
        GroupingMode.CURRENT,
        source_root=str(source_root),
    )
    for group in plan.groups:
        group.group_label = "Merged"

    summary = execute_grouping_plan(
        plan,
        source_root=str(source_root),
        output_root=str(source_root),
    )

    moved_paths = sorted(entry.new_path for entry in summary.entries if entry.new_path)
    assert summary.moved_count == 2
    assert moved_paths == [
        str(source_root / "Merged" / "same.jpg"),
        str(source_root / "Merged" / "same_1.jpg"),
    ]
    assert not (source_root / "Merged" / "B").exists()

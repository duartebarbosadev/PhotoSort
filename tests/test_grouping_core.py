import json
import os

from PIL import Image, ImageDraw

from src.core.grouping import (
    GroupingMode,
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


def test_mixed_grouping_partitions_by_location_then_similarity(tmp_path, monkeypatch):
    a = tmp_path / "loc1_red.jpg"
    b = tmp_path / "loc1_blue.jpg"
    c = tmp_path / "loc2_red.jpg"
    _create_solid_image(str(a), (230, 20, 20))
    _create_solid_image(str(b), (20, 20, 230))
    _create_solid_image(str(c), (235, 25, 25))

    metadata_by_path = {
        str(a): {
            "Exif.GPSInfo.GPSLatitude": "10",
            "Exif.GPSInfo.GPSLatitudeRef": "N",
            "Exif.GPSInfo.GPSLongitude": "20",
            "Exif.GPSInfo.GPSLongitudeRef": "E",
        },
        str(b): {
            "Exif.GPSInfo.GPSLatitude": "10",
            "Exif.GPSInfo.GPSLatitudeRef": "N",
            "Exif.GPSInfo.GPSLongitude": "20",
            "Exif.GPSInfo.GPSLongitudeRef": "E",
        },
        str(c): {
            "Exif.GPSInfo.GPSLatitude": "11",
            "Exif.GPSInfo.GPSLatitudeRef": "N",
            "Exif.GPSInfo.GPSLongitude": "21",
            "Exif.GPSInfo.GPSLongitudeRef": "E",
        },
    }
    monkeypatch.setattr(
        "src.core.grouping._load_comprehensive_metadata",
        lambda path: metadata_by_path[str(path)],
    )
    monkeypatch.setattr(
        "src.core.grouping._run_ml_similarity_pipeline",
        lambda paths, progress_callback=None, shared_engine=None: (
            {str(a): 1, str(b): 2} if set(paths) == {str(a), str(b)} else {str(c): 1}
        ),
    )

    plan = build_grouping_plan(
        [{"path": str(a)}, {"path": str(b)}, {"path": str(c)}],
        GroupingMode.MIXED,
    )

    assert len(plan.groups) == 3
    labels = sorted(group.group_label for group in plan.groups)
    assert labels[0].startswith("Lat_10.00_Lon_20.00")
    assert labels[-1].startswith("Lat_11.00_Lon_21.00")


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

    plan = build_grouping_plan(
        [{"path": str(first)}, {"path": str(second)}],
        GroupingMode.SIMILARITY,
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

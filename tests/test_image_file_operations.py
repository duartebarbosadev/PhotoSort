from core.image_file_ops import ImageFileOperations


def test_move_path_uses_exact_destination_and_creates_parent(tmp_path):
    source = tmp_path / "source.jpg"
    source.write_bytes(b"photo")
    destination = tmp_path / "nested" / "renamed.jpg"

    success, result = ImageFileOperations.move_path(str(source), str(destination))

    assert success is True
    assert result == str(destination)
    assert destination.read_bytes() == b"photo"
    assert not source.exists()


def test_move_image_preserves_collision_resolution(tmp_path):
    source_dir = tmp_path / "source"
    destination_dir = tmp_path / "destination"
    source_dir.mkdir()
    destination_dir.mkdir()
    source = source_dir / "photo.jpg"
    source.write_bytes(b"new")
    (destination_dir / "photo.jpg").write_bytes(b"existing")

    success, result = ImageFileOperations.move_image(str(source), str(destination_dir))

    expected = destination_dir / "photo_1.jpg"
    assert success is True
    assert result == str(expected)
    assert expected.read_bytes() == b"new"
    assert (destination_dir / "photo.jpg").read_bytes() == b"existing"


def test_replace_file_overwrites_destination(tmp_path):
    source = tmp_path / "replacement.jpg"
    destination = tmp_path / "photo.jpg"
    source.write_bytes(b"replacement")
    destination.write_bytes(b"original")

    success, message = ImageFileOperations.replace_file(str(source), str(destination))

    assert success is True
    assert message == "File replaced successfully."
    assert destination.read_bytes() == b"replacement"
    assert not source.exists()

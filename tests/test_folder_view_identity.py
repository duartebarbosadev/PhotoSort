"""Real-file regressions for the shared bookmark identity boundary."""

import pyexiv2  # noqa: F401  # Must be first to avoid Windows crashes

import json
import os
from copy import deepcopy
from unittest.mock import Mock, patch

import pytest

from core.file_scanner import FileScanner
from core.folder_view_store import FolderViewStore, WORKFLOWS, bookmark_file_identity


@pytest.fixture
def library(tmp_path):
    folder = tmp_path / "photos"
    folder.mkdir()
    photos = [folder / name for name in ("one.jpg", "two.jpg")]
    for photo in photos:
        photo.write_bytes(b"original")
    store = FolderViewStore(str(tmp_path / "bookmarks"))
    return str(folder), photos, store


def view_for(paths):
    # All fields carrying image references across the five workflow adapters.
    return {
        "path": paths[0],
        "review": paths[0],
        "selected": paths,
        "before": paths,
        "after": paths,
        "comparison": paths,
        "scroll": [0, 120],
        "sort": "Time",
        "search": "one",
        "mode": "list",
    }


@pytest.mark.parametrize("workflow", WORKFLOWS)
def test_same_name_replacement_cannot_inherit_selection(library, workflow):
    folder, photos, store = library
    paths = [str(p) for p in photos]
    state = view_for(paths)
    bookmark = {"workflow": workflow, "views": {workflow: state}}
    store.save(folder, bookmark)
    original = photos[0].stat()
    replacement = photos[0].with_suffix(".new")
    replacement.write_bytes(b"replaced")
    os.utime(replacement, ns=(original.st_atime_ns, original.st_mtime_ns))
    replacement.replace(photos[0])

    restored = store.load(folder)
    expected = {**state, "path": None, "review": None, "comparison": []}
    for key in ("selected", "before", "after"):
        expected[key] = [paths[1]]
    assert restored == {"workflow": workflow, "views": {workflow: expected}}
    assert bookmark["views"][workflow]["path"] == paths[0]


def test_unchanged_photos_restore_across_restart(library):
    folder, photos, store = library
    view = view_for([str(p) for p in photos])
    bookmark = {"workflow": "cull", "views": {w: deepcopy(view) for w in WORKFLOWS}}
    store.save(folder, bookmark)
    assert FolderViewStore(store.directory).load(folder) == bookmark


def test_all_replaced_with_different_names_keeps_view_settings(library):
    folder, photos, store = library
    state = view_for([str(p) for p in photos])
    store.save(folder, {"workflow": "cull", "views": {"cull": state}})
    for photo in photos:
        photo.unlink()
        photo.with_name("new-" + photo.name).write_bytes(b"new")
    restored = store.load(folder)["views"]["cull"]
    assert restored == {
        **state,
        "path": None,
        "review": None,
        "selected": [],
        "before": [],
        "after": [],
        "comparison": [],
    }


def test_autosave_uses_scan_identity_not_replacement_identity(library):
    folder, photos, store = library
    path = str(photos[0])
    scanned = {path: bookmark_file_identity(photos[0].stat())}
    photos[0].write_bytes(b"new contents before autosave")
    # A late save, including one triggered by changing another workflow, must
    # retain the identity of the image which was originally selected.
    store.save(
        folder,
        {"workflow": "cull", "views": {"cull": {"path": path}}},
        identities=scanned,
    )
    assert store.load(folder)["views"]["cull"]["path"] is None


@pytest.mark.parametrize("version", [1, 2])
def test_missing_identity_never_restores_legacy_selection(library, version):
    folder, photos, store = library
    state = {"path": str(photos[0]), "scroll": [0, 12], "search": "one"}
    store.save(folder, {"workflow": "cull", "views": {"cull": state}})
    document = json.loads(store._path(folder).read_text())
    document["version"] = version
    document.pop("identities")
    store._path(folder).write_text(json.dumps(document))
    assert store.load(folder)["views"]["cull"] == {**state, "path": None}


def test_same_name_in_another_folder_does_not_match(library, tmp_path):
    folder, photos, store = library
    state = {"path": str(photos[0])}
    store.save(folder, {"workflow": "cull", "views": {"cull": state}})
    other = tmp_path / "other"
    other.mkdir()
    photos[0].rename(other / photos[0].name)
    assert store.load(folder)["views"]["cull"]["path"] is None
    assert store.load(str(other)) == {}


def test_scan_is_reused_and_validation_stats_each_photo_once(library):
    folder, photos, store = library
    scanner = FileScanner(Mock())
    records = []
    scanner.files_found.connect(records.extend)
    real_stat = os.stat
    with patch("os.stat", wraps=real_stat) as stats:
        scanner.scan_directory(folder)
    for photo in photos:
        assert sum(call.args[0] == str(photo) for call in stats.call_args_list) == 1
    identities = {r["path"]: r["bookmark_identity"] for r in records}
    assert identities == {str(p): bookmark_file_identity(p.stat()) for p in photos}
    view = view_for([str(p) for p in photos])
    bookmark = {"workflow": "cull", "views": {w: deepcopy(view) for w in WORKFLOWS}}
    with patch.object(
        store, "_file_identity", side_effect=AssertionError("No save-time stat")
    ):
        store.save(folder, bookmark, identities=identities)
    with patch.object(store, "_file_identity", wraps=store._file_identity) as reads:
        assert store.load(folder) == bookmark
    assert reads.call_count == 2
    assert scanner.image_pipeline.mock_calls == []


def test_cancelled_validation_stops_before_remaining_files(library):
    folder, photos, store = library
    store.save(
        folder,
        {"workflow": "cull", "views": {"cull": view_for([str(p) for p in photos])}},
    )
    with patch.object(store, "_file_identity", wraps=store._file_identity) as reads:
        result = store.load(folder, should_continue=lambda: reads.call_count < 1)
    assert result == {}
    assert reads.call_count == 1


def test_corrupt_path_and_identity_fields_fail_closed(library):
    folder, photos, store = library
    store.save(
        folder, {"workflow": "cull", "views": {"cull": {"path": str(photos[0])}}}
    )
    document = json.loads(store._path(folder).read_text())
    document["identities"] = []
    document["views"]["cull"].update(path=["invalid"], selected=[{}, str(photos[0])])
    store._path(folder).write_text(json.dumps(document))
    assert store.load(folder)["views"]["cull"] == {"path": None, "selected": []}


def test_rating_write_refreshes_identity_before_completion(library, monkeypatch):
    from types import SimpleNamespace
    from ui.app_controller import AppController
    from workers.rating_writer_worker import RatingWriterWorker

    folder, photos, store = library
    photo = photos[0]
    path = str(photo)
    record = {"bookmark_identity": bookmark_file_identity(photo.stat())}
    controller = SimpleNamespace(
        app_state=SimpleNamespace(get_file_data_by_path=lambda _path: record)
    )
    worker = RatingWriterWorker()
    worker.source_file_updated.connect(
        lambda p, identity: AppController.handle_rating_source_file_updated(
            controller, p, identity
        )
    )

    def write(*_args):
        photo.write_bytes(b"image with updated rating metadata")
        return True

    monkeypatch.setattr(
        "workers.rating_writer_worker.MetadataProcessor.set_rating", write
    )
    snapshots = []
    worker.rating_written.connect(lambda *_args: snapshots.append(deepcopy(record)))
    worker.write_ratings([(path, 5)])
    assert snapshots == [{"bookmark_identity": bookmark_file_identity(photo.stat())}]
    bookmark = {"workflow": "cull", "views": {"cull": {"path": path}}}
    store.save(folder, bookmark, identities={path: record["bookmark_identity"]})
    assert store.load(folder) == bookmark

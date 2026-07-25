from unittest.mock import Mock

from workers.file_deletion_worker import FileDeletionWorker


def test_deletion_worker_invalidates_only_successful_targets():
    trash = Mock(
        side_effect=[
            (True, "ok"),
            (False, "busy"),
        ]
    )
    rating_cache = Mock()
    exif_cache = Mock()
    analysis_cache = Mock()
    results = []
    worker = FileDeletionWorker(
        ["folder", "failed.jpg"],
        cache_paths_by_target={
            "folder": ["folder/a.jpg", "folder/b.jpg"],
            "failed.jpg": ["failed.jpg"],
        },
        rating_cache=rating_cache,
        exif_cache=exif_cache,
        analysis_cache=analysis_cache,
        folder_path="/photos",
        trash_operation=trash,
    )
    worker.completed.connect(results.append)

    worker.run()

    result = results[0]
    assert result.successful_targets == ["folder"]
    assert result.failures == {"failed.jpg": "busy"}
    assert [call.args[0] for call in rating_cache.delete.call_args_list] == [
        "folder/a.jpg",
        "folder/b.jpg",
    ]
    assert [call.args[0] for call in exif_cache.delete.call_args_list] == [
        "folder/a.jpg",
        "folder/b.jpg",
    ]
    analysis_cache.remove_paths.assert_called_once_with(
        "/photos",
        {"folder/a.jpg", "folder/b.jpg"},
    )

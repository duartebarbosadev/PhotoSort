from types import SimpleNamespace
from unittest.mock import Mock

from workers.grouping_worker import GroupingWorkflowWorker


def test_grouping_worker_migrates_disk_cache_paths_before_completion(monkeypatch):
    plan = SimpleNamespace(
        apply_group_label_overrides=Mock(),
        output_root="",
    )
    summary = SimpleNamespace(
        entries=[
            SimpleNamespace(original_path="/source/a.jpg", new_path="/output/a.jpg"),
            SimpleNamespace(original_path="/source/b.jpg", new_path=None),
        ]
    )
    migration = Mock()
    monkeypatch.setattr(
        "workers.grouping_worker.augment_grouping_plan_with_filesystem_paths",
        lambda prepared, _root: prepared,
    )
    monkeypatch.setattr(
        "workers.grouping_worker.execute_grouping_plan",
        lambda *_args, **_kwargs: summary,
    )
    monkeypatch.setattr(
        "workers.grouping_worker.migrate_cached_paths",
        migration,
    )

    rating_cache = object()
    exif_cache = object()
    analysis_cache = Mock()
    worker = GroupingWorkflowWorker(
        items=[],
        mode="current",
        source_root="/source",
        output_root="/output",
        prepared_plan=plan,
        rating_cache=rating_cache,
        exif_cache=exif_cache,
        analysis_cache=analysis_cache,
    )
    completed = Mock()
    worker.completed.connect(completed)

    worker.run()

    migration.assert_called_once_with(
        {"/source/a.jpg": "/output/a.jpg"},
        rating_cache=rating_cache,
        exif_cache=exif_cache,
    )
    completed.assert_called_once_with(summary)
    analysis_cache.migrate_folder_paths.assert_called_once_with(
        "/source",
        "/output",
        {"/source/a.jpg": "/output/a.jpg"},
    )

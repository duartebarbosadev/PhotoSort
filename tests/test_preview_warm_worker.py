from unittest.mock import Mock

from workers.preview_warm_worker import PreviewWarmWorker


def test_preview_warmer_deduplicates_folder_paths_and_uses_shared_pipeline():
    pipeline = Mock()
    worker = PreviewWarmWorker(pipeline, ["a.jpg", "b.jpg", "a.jpg"])
    finished = []
    worker.finished.connect(
        lambda processed, total: finished.append((processed, total))
    )

    def preload(paths, progress_callback, should_continue_callback):
        assert should_continue_callback()
        for index, _path in enumerate(paths, start=1):
            progress_callback(index, len(paths))

    pipeline.preload_previews.side_effect = preload
    worker.run()

    assert pipeline.preload_previews.call_count == 1
    assert pipeline.preload_previews.call_args.args[0] == ["a.jpg", "b.jpg"]
    assert finished == [(2, 2)]

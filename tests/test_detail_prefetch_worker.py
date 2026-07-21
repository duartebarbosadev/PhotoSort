from threading import Event
from unittest.mock import Mock, call

from PIL import Image

from workers.detail_prefetch_worker import DetailPrefetchWorker


def test_detail_worker_keeps_full_resolution_when_pair_fits_budget():
    pipeline = Mock()
    pipeline.get_source_dimensions.side_effect = [(4000, 3000), (3000, 2000)]
    pipeline.load_detail_image.side_effect = lambda _path, _target: Image.new(
        "RGBA", (20, 10)
    )
    worker = DetailPrefetchWorker(
        ["left.jpg", "right.jpg"],
        pipeline,
        Event(),
        1,
        max_display_bytes=512 * 1024 * 1024,
    )

    worker.run()

    assert pipeline.load_detail_image.call_args_list == [
        call("left.jpg", None),
        call("right.jpg", None),
    ]


def test_detail_worker_scales_pair_proportionally_to_combined_budget():
    pipeline = Mock()
    pipeline.get_source_dimensions.side_effect = [(10000, 10000), (10000, 10000)]
    pipeline.load_detail_image.side_effect = lambda _path, _target: Image.new(
        "RGBA", (20, 10)
    )
    worker = DetailPrefetchWorker(
        ["left.jpg", "right.jpg"],
        pipeline,
        Event(),
        2,
        max_display_bytes=200 * 1024 * 1024,
    )

    worker.run()

    left_target = pipeline.load_detail_image.call_args_list[0].args[1]
    right_target = pipeline.load_detail_image.call_args_list[1].args[1]
    assert left_target == right_target
    assert left_target[0] < 10000
    combined_bytes = 2 * left_target[0] * left_target[1] * 4
    assert combined_bytes <= 200 * 1024 * 1024

from threading import Event, Lock, Thread
import time
from unittest.mock import Mock

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

    assert {call.args for call in pipeline.load_detail_image.call_args_list} == {
        ("left.jpg", None),
        ("right.jpg", None),
    }


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


def test_detail_worker_decodes_at_most_four_images_in_parallel():
    paths = [f"{index}.jpg" for index in range(8)]
    pipeline = Mock()
    pipeline.get_source_dimensions.return_value = (100, 100)
    lock = Lock()
    active = 0
    peak_active = 0

    def load_detail(_path, _target):
        nonlocal active, peak_active
        with lock:
            active += 1
            peak_active = max(peak_active, active)
        time.sleep(0.03)
        with lock:
            active -= 1
        return Image.new("RGBA", (20, 10))

    pipeline.load_detail_image.side_effect = load_detail
    worker = DetailPrefetchWorker(
        paths,
        pipeline,
        Event(),
        3,
        max_display_bytes=512 * 1024 * 1024,
    )

    worker.run()

    assert peak_active == 4
    assert pipeline.load_detail_image.call_count == 8


def test_detail_worker_cancellation_does_not_start_the_next_decode_group():
    paths = [f"{index}.jpg" for index in range(8)]
    pipeline = Mock()
    pipeline.get_source_dimensions.return_value = (100, 100)
    cancel_event = Event()
    four_started = Event()
    release_active = Event()
    lock = Lock()
    started = 0

    def load_detail(_path, _target):
        nonlocal started
        with lock:
            started += 1
            if started == 4:
                four_started.set()
        release_active.wait(timeout=2)
        return Image.new("RGBA", (20, 10))

    pipeline.load_detail_image.side_effect = load_detail
    worker = DetailPrefetchWorker(
        paths,
        pipeline,
        cancel_event,
        4,
        max_display_bytes=512 * 1024 * 1024,
    )
    thread = Thread(target=worker.run)
    thread.start()
    assert four_started.wait(timeout=2)

    cancel_event.set()
    release_active.set()
    thread.join(timeout=2)

    assert not thread.is_alive()
    assert pipeline.load_detail_image.call_count == 4

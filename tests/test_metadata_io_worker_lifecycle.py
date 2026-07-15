import threading

from core.metadata_io import MetadataIO


def test_metadata_worker_uses_queue_shutdown_and_can_restart(monkeypatch):
    monkeypatch.setenv("PHOTOSORT_FORCE_PYEXIV2_THREAD", "true")
    MetadataIO.shutdown_worker_thread(immediate=True)

    try:
        assert MetadataIO._call_in_worker(lambda value: value + 1, 4) == 5
        first_thread = MetadataIO._WORKER_THREAD
        assert first_thread is not None
        assert first_thread.is_alive()

        MetadataIO.shutdown_worker_thread()
        assert not first_thread.is_alive()
        assert MetadataIO._TASK_QUEUE is None

        assert MetadataIO._call_in_worker(threading.current_thread) is not None
        second_thread = MetadataIO._WORKER_THREAD
        assert second_thread is not None
        assert second_thread is not first_thread
        assert second_thread.is_alive()
    finally:
        MetadataIO.shutdown_worker_thread(immediate=True)

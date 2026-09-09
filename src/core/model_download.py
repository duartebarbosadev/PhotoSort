"""Isolate network transfers so cancellation never waits for a blocked SDK call.

Only Hugging Face cache writes run in the child, never image/file mutations or
model inference. Hugging Face owns its partial files and cache locks; stopping
the child releases those locks and leaves completed cache entries reusable.
"""

from collections.abc import Callable
import multiprocessing
import threading

from core.huggingface_progress import ProgressCallback, build_hf_tqdm_class


class ModelDownloadCancelled(RuntimeError):
    """The caller cancelled a model transfer."""


def _download_child(connection, repo_id: str, options: dict, label: str) -> None:
    # Spawned without Qt/model instances. HF reports from several threads, so
    # serialize pipe writes to keep progress messages intact.
    from huggingface_hub import snapshot_download

    lock = threading.Lock()

    def send(message):
        with lock:
            connection.send(message)

    try:
        snapshot = snapshot_download(
            repo_id,
            local_files_only=False,
            tqdm_class=build_hf_tqdm_class(
                lambda percent, message: send(("progress", percent, message)),
                label=label,
            ),
            **options,
        )
        send(("result", str(snapshot)))
    except Exception as exc:
        send(("error", str(exc)))
    finally:
        connection.close()


def download_snapshot(
    repo_id: str,
    *,
    options: dict,
    label: str,
    progress_callback: ProgressCallback | None = None,
    should_cancel: Callable[[], bool] | None = None,
) -> str:
    """Wait on a transfer off the UI thread, polling cancellation every 100ms."""
    if should_cancel and should_cancel():
        raise ModelDownloadCancelled
    context = multiprocessing.get_context("spawn")
    receive, send = context.Pipe(duplex=False)
    process = context.Process(
        target=_download_child, args=(send, repo_id, options, label), daemon=True
    )
    started = False
    try:
        process.start()
        started = True
        send.close()
        while True:
            if should_cancel and should_cancel():
                raise ModelDownloadCancelled
            if receive.poll(0.1):
                try:
                    message = receive.recv()
                except EOFError as exc:
                    raise OSError(
                        "Model download process exited without a result."
                    ) from exc
                if message[0] == "result":
                    return message[1]
                if message[0] == "error":
                    raise OSError(message[1])
                if progress_callback:
                    progress_callback(message[1], message[2])
            elif not process.is_alive():
                raise OSError("Model download process exited without a result.")
    finally:
        if started:
            # No QThread termination: this process owns only a model transfer.
            if process.is_alive():
                process.terminate()
            process.join(timeout=1)
            if process.is_alive():
                process.kill()
                process.join(timeout=1)
            if not process.is_alive():
                process.close()
        receive.close()
        send.close()

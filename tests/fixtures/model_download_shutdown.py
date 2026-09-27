"""Run isolated download lifecycles so resource-tracker stderr is observable."""

from pathlib import Path
import sys
import threading
import time

from core import model_download


_download_child = model_download._download_child


def simulated_transfer(connection, repo_id, options, label):
    import huggingface_hub

    mode = options["mode"]

    def download(*args, tqdm_class, **kwargs):
        with tqdm_class(total=10, unit="B") as progress:
            progress.update(5)
            if mode == "cancel":
                threading.Event().wait(30)
            if mode == "error":
                raise OSError("connection lost")
            progress.update(5)
        return "/simulated/snapshot"

    huggingface_hub.snapshot_download = download
    _download_child(connection, repo_id, options, label)
    # Represent cleanup after the terminal message, before the child exits.
    time.sleep(0.05)
    Path(options["marker"]).write_text("cleaned up", encoding="utf-8")


if __name__ == "__main__":
    mode, marker = sys.argv[1:]
    model_download._download_child = simulated_transfer
    for _ in range(2):
        cancel = threading.Event()
        try:
            result = model_download.download_snapshot(
                "test/model",
                options={"mode": mode, "marker": marker},
                label="Test",
                progress_callback=lambda *_: cancel.set() if mode == "cancel" else None,
                should_cancel=cancel.is_set,
            )
            assert mode == "success" and result == "/simulated/snapshot"
        except model_download.ModelDownloadCancelled:
            assert mode == "cancel"
        except OSError as exc:
            assert mode == "error" and "connection lost" in str(exc)
        if mode != "cancel":
            assert Path(marker).read_text(encoding="utf-8") == "cleaned up"
            Path(marker).unlink()

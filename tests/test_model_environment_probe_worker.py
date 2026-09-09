import pyexiv2  # noqa: F401  # Must be first to avoid Windows crash

from workers.model_environment_probe_worker import ModelEnvironmentProbeWorker


def _run(worker):
    results = []
    finished = []
    worker.completed.connect(lambda missing, device: results.append((missing, device)))
    worker.finished.connect(lambda: finished.append(True))
    worker.run()
    assert finished == [True]
    return results[0]


def test_probe_reports_missing_models_and_device(monkeypatch):
    from core import model_provisioning

    monkeypatch.setattr(
        model_provisioning,
        "missing_models",
        lambda models: [model for model in models if model.key == "aesthetic"],
    )
    monkeypatch.setattr(
        "core.app_settings.get_preferred_torch_device",
        lambda: "mps",
    )

    missing, device = _run(ModelEnvironmentProbeWorker(["embedding", "aesthetic"]))

    assert missing == ("aesthetic",)
    assert device == "mps"


def test_probe_reports_nothing_missing_when_everything_is_installed(monkeypatch):
    from core import model_provisioning

    monkeypatch.setattr(model_provisioning, "missing_models", lambda models: [])
    monkeypatch.setattr("core.app_settings.get_preferred_torch_device", lambda: "cuda")

    missing, device = _run(ModelEnvironmentProbeWorker(["embedding"]))

    assert missing == ()
    assert device == "cuda"


def test_probe_fails_closed_when_the_environment_cannot_be_read(monkeypatch):
    def _boom(_models):
        raise RuntimeError("no hub access")

    from core import model_provisioning

    monkeypatch.setattr(model_provisioning, "missing_models", _boom)

    missing, device = _run(ModelEnvironmentProbeWorker(["embedding"]))

    # A failed probe must never block the UI, but it must also not claim the
    # models are present: that would skip the download prompt and let a workflow
    # start only to die on a missing model. Fail closed and let the user decide.
    assert missing == ("embedding",)
    assert device == "cpu"

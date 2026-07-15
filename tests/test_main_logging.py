import logging

from src.main import _resolve_log_level


def test_application_logging_defaults_to_info(monkeypatch):
    monkeypatch.delenv("PHOTOSORT_LOG_LEVEL", raising=False)

    assert _resolve_log_level() == logging.INFO


def test_application_logging_supports_opt_in_debug(monkeypatch):
    monkeypatch.setenv("PHOTOSORT_LOG_LEVEL", "debug")

    assert _resolve_log_level() == logging.DEBUG
    assert _resolve_log_level("not-a-level") == logging.INFO

"""Tests for the best-shot dependency checker."""

from __future__ import annotations

import pytest

from core.ai.model_checker import (
    ModelDependencyError,
    MissingModelInfo,
    check_best_shot_models,
    ensure_best_shot_models,
)


def test_check_best_shot_models_all_present(monkeypatch):
    monkeypatch.setattr(
        "core.ai.model_checker._module_available",
        lambda name: True,
    )

    missing = check_best_shot_models()

    assert missing == []


def test_check_best_shot_models_missing_pyiqa(monkeypatch):
    def fake_availability(name: str) -> bool:
        return name != "pyiqa"

    monkeypatch.setattr(
        "core.ai.model_checker._module_available",
        fake_availability,
    )

    missing = check_best_shot_models()

    assert len(missing) == 1
    assert "pyiqa" in missing[0].name.lower()


def test_ensure_best_shot_models_raises(monkeypatch):
    monkeypatch.setattr(
        "core.ai.model_checker._module_available",
        lambda name: False,
    )

    with pytest.raises(ModelDependencyError) as excinfo:
        ensure_best_shot_models()

    assert excinfo.value.missing_models
    assert "Required best-shot dependencies not found" in str(excinfo.value)


def test_missing_model_info_structure():
    info = MissingModelInfo(
        name="Dependency",
        description="A dependency",
        expected_path="pip install something",
        download_url="https://example.com",
    )

    assert info.name == "Dependency"
    assert "dependency" in info.description.lower()


def test_model_dependency_error_message():
    missing = [
        MissingModelInfo(
            name="torch",
            description="",
            expected_path="",
            download_url="",
        ),
        MissingModelInfo(
            name="pyiqa",
            description="",
            expected_path="",
            download_url="",
        ),
    ]

    error = ModelDependencyError(missing)

    assert "torch" in str(error)
    assert "pyiqa" in str(error)

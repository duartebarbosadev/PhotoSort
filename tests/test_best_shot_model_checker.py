"""Tests for best-shot model dependency checker."""

from __future__ import annotations

import os
from unittest.mock import patch

import pytest

from core.ai.model_checker import (
    ModelDependencyError,
    MissingModelInfo,
    check_best_shot_models,
    ensure_best_shot_models,
)


@pytest.fixture
def temp_models_root(tmp_path):
    """Create a temporary models directory."""
    models_dir = tmp_path / "models"
    models_dir.mkdir()
    return str(models_dir)


def test_check_all_models_present(temp_models_root):
    """Test that check passes when all models are present."""
    import numpy as np
    
    # Create all required model directories and files
    face_dir = os.path.join(temp_models_root, "job_jgzjewkop_optimized_onnx")
    os.makedirs(face_dir)
    with open(os.path.join(face_dir, "model.onnx"), "w") as f:
        f.write("fake model")

    eye_dir = os.path.join(temp_models_root, "open-closed-eye-classification-mobilev2")
    os.makedirs(eye_dir)

    aesthetic_dir = os.path.join(temp_models_root, "aesthetic_predictor")
    os.makedirs(aesthetic_dir)

    # Create anchors in the models root
    np.save(os.path.join(temp_models_root, "blazeface_anchors.npy"), np.array([[1, 2, 3]]))

    missing = check_best_shot_models(temp_models_root)
    assert len(missing) == 0


def test_check_all_models_missing(temp_models_root):
    """Test that all models are reported missing when none are present."""
    missing = check_best_shot_models(temp_models_root)

    # Should find 4 missing models (face, eye, aesthetic, anchors)
    assert len(missing) >= 3  # At least 3, anchors might be bundled
    model_names = {m.name for m in missing}
    assert "Face Detector" in model_names
    assert "Eye Classifier" in model_names
    assert "Aesthetic Predictor" in model_names


def test_check_face_detector_missing(temp_models_root):
    """Test face detector missing detection."""
    import numpy as np
    
    # Create only eye classifier and aesthetic predictor
    eye_dir = os.path.join(temp_models_root, "open-closed-eye-classification-mobilev2")
    os.makedirs(eye_dir)

    aesthetic_dir = os.path.join(temp_models_root, "aesthetic_predictor")
    os.makedirs(aesthetic_dir)

    # Create anchors
    np.save(os.path.join(temp_models_root, "blazeface_anchors.npy"), np.array([[1, 2, 3]]))

    missing = check_best_shot_models(temp_models_root)

    assert len(missing) == 1
    assert missing[0].name == "Face Detector"
    assert "qualcomm/MediaPipe-Face-Detection" in missing[0].download_url


def test_check_eye_classifier_missing(temp_models_root):
    """Test eye classifier missing detection."""
    import numpy as np
    
    # Create only face detector and aesthetic predictor
    face_dir = os.path.join(temp_models_root, "job_jgzjewkop_optimized_onnx")
    os.makedirs(face_dir)
    with open(os.path.join(face_dir, "model.onnx"), "w") as f:
        f.write("fake model")

    aesthetic_dir = os.path.join(temp_models_root, "aesthetic_predictor")
    os.makedirs(aesthetic_dir)

    # Create anchors
    np.save(os.path.join(temp_models_root, "blazeface_anchors.npy"), np.array([[1, 2, 3]]))

    missing = check_best_shot_models(temp_models_root)

    assert len(missing) == 1
    assert missing[0].name == "Eye Classifier"
    assert "MichalMlodawski" in missing[0].download_url


def test_check_aesthetic_predictor_missing(temp_models_root):
    """Test aesthetic predictor missing detection."""
    import numpy as np
    
    # Create only face detector and eye classifier
    face_dir = os.path.join(temp_models_root, "job_jgzjewkop_optimized_onnx")
    os.makedirs(face_dir)
    with open(os.path.join(face_dir, "model.onnx"), "w") as f:
        f.write("fake model")

    eye_dir = os.path.join(temp_models_root, "open-closed-eye-classification-mobilev2")
    os.makedirs(eye_dir)

    # Create anchors
    np.save(os.path.join(temp_models_root, "blazeface_anchors.npy"), np.array([[1, 2, 3]]))

    missing = check_best_shot_models(temp_models_root)

    assert len(missing) == 1
    assert missing[0].name == "Aesthetic Predictor"
    assert "shunk031" in missing[0].download_url


def test_ensure_models_raises_on_missing(temp_models_root):
    """Test that ensure_best_shot_models raises ModelDependencyError."""
    with pytest.raises(ModelDependencyError) as excinfo:
        ensure_best_shot_models(temp_models_root)

    assert len(excinfo.value.missing_models) >= 3
    assert "Face Detector" in str(excinfo.value)


def test_ensure_models_passes_when_present(temp_models_root):
    """Test that ensure_best_shot_models doesn't raise when all models present."""
    import numpy as np
    
    # Create all required model directories and files
    face_dir = os.path.join(temp_models_root, "job_jgzjewkop_optimized_onnx")
    os.makedirs(face_dir)
    with open(os.path.join(face_dir, "model.onnx"), "w") as f:
        f.write("fake model")

    eye_dir = os.path.join(temp_models_root, "open-closed-eye-classification-mobilev2")
    os.makedirs(eye_dir)

    aesthetic_dir = os.path.join(temp_models_root, "aesthetic_predictor")
    os.makedirs(aesthetic_dir)

    # Create anchors
    np.save(os.path.join(temp_models_root, "blazeface_anchors.npy"), np.array([[1, 2, 3]]))

    # Should not raise
    ensure_best_shot_models(temp_models_root)


def test_missing_model_info_structure():
    """Test the structure of MissingModelInfo."""
    info = MissingModelInfo(
        name="Test Model",
        description="A test model",
        expected_path="/path/to/model",
        download_url="https://example.com",
    )

    assert info.name == "Test Model"
    assert info.description == "A test model"
    assert info.expected_path == "/path/to/model"
    assert info.download_url == "https://example.com"


def test_model_dependency_error_message():
    """Test that ModelDependencyError formats message correctly."""
    missing = [
        MissingModelInfo(
            name="Model A",
            description="First model",
            expected_path="/path/a",
            download_url="https://a.com",
        ),
        MissingModelInfo(
            name="Model B",
            description="Second model",
            expected_path="/path/b",
            download_url="https://b.com",
        ),
    ]

    error = ModelDependencyError(missing)
    assert "Model A" in str(error)
    assert "Model B" in str(error)
    assert len(error.missing_models) == 2


def test_alternative_face_detector_path(temp_models_root):
    """Test that alternative face detector path is recognized."""
    import numpy as np
    
    # Create face detector in alternative location
    face_dir = os.path.join(
        temp_models_root, "MediaPipe-Face-Detection_FaceDetector_float"
    )
    os.makedirs(face_dir)
    with open(os.path.join(face_dir, "model.onnx"), "w") as f:
        f.write("fake model")

    eye_dir = os.path.join(temp_models_root, "open-closed-eye-classification-mobilev2")
    os.makedirs(eye_dir)

    aesthetic_dir = os.path.join(temp_models_root, "aesthetic_predictor")
    os.makedirs(aesthetic_dir)

    # Create anchors
    np.save(os.path.join(temp_models_root, "blazeface_anchors.npy"), np.array([[1, 2, 3]]))

    missing = check_best_shot_models(temp_models_root)

    # Should not report face detector as missing
    model_names = {m.name for m in missing}
    assert "Face Detector" not in model_names

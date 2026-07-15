from types import SimpleNamespace
from unittest.mock import Mock, patch

from PIL import Image

from core.app_settings import ROTATION_MODEL_IMAGE_SIZE
from core.image_features.rotation_detector import RotationDetector
from core.image_pipeline import ANALYSIS_CACHE_RESOLUTION, ImagePipeline
from ui.app_controller import AppController


def test_rotation_detector_requests_bounded_shared_analysis_image():
    pipeline = Mock()
    pipeline.get_analysis_image.return_value = Image.new("RGB", (416, 300))
    result_callback = Mock()

    with patch(
        "core.image_features.rotation_detector.ModelRotationDetector"
    ) as model_class:
        model_class.return_value.predict_rotation_angle.return_value = 90
        detector = RotationDetector(image_pipeline=pipeline, exif_cache=Mock())
        detector._detect_rotation_task("photo.arw", result_callback)

    expected_size = ROTATION_MODEL_IMAGE_SIZE + 32
    pipeline.get_analysis_image.assert_called_once_with(
        "photo.arw",
        target_size=(expected_size, expected_size),
    )
    model_class.return_value.predict_rotation_angle.assert_called_once()
    result_callback.assert_called_once_with("photo.arw", 90)


def test_rotation_detector_does_not_retry_failed_analysis_with_full_decode():
    pipeline = Mock()
    pipeline.get_analysis_image.return_value = None
    result_callback = Mock()

    with patch(
        "core.image_features.rotation_detector.ModelRotationDetector"
    ) as model_class:
        detector = RotationDetector(image_pipeline=pipeline, exif_cache=Mock())
        detector._detect_rotation_task("broken.arw", result_callback)

    model_class.return_value.predict_rotation_angle.assert_not_called()
    result_callback.assert_called_once_with("broken.arw", 0)


def test_analysis_image_uses_neutral_raw_loader_and_reuses_cache(tmp_path):
    source = tmp_path / "photo.arw"
    source.write_bytes(b"raw-placeholder")
    pipeline = ImagePipeline(
        thumbnail_cache_dir=str(tmp_path / "thumbs"),
        preview_cache_dir=str(tmp_path / "previews"),
    )
    loaded = Image.new("RGB", (1024, 700), "navy")

    with (
        patch("core.image_pipeline.is_raw_extension", return_value=True),
        patch(
            "core.image_pipeline.RawImageProcessor.load_raw_for_blur_detection",
            return_value=loaded,
        ) as raw_loader,
    ):
        first = pipeline.get_analysis_image(str(source), (416, 416))
        second = pipeline.get_analysis_image(str(source), (640, 480))

    assert first is not None and max(first.size) <= 416
    assert second is not None and second.size[0] <= 640 and second.size[1] <= 480
    raw_loader.assert_called_once_with(
        str(source),
        target_size=ANALYSIS_CACHE_RESOLUTION,
        apply_auto_edits=False,
    )


def test_completed_empty_rotation_result_is_not_recomputed():
    widget = SimpleNamespace(show_results=Mock(), show_loading=Mock())
    worker_manager = SimpleNamespace(
        is_fix_rotation_running=lambda: False,
        start_fix_rotation_detection=Mock(),
    )
    controller = SimpleNamespace(
        app_state=SimpleNamespace(
            image_files_data=[{"path": "photo.jpg", "media_type": "image"}],
            fix_rotation_results={},
        ),
        main_window=SimpleNamespace(
            fix_rotation_step_widget=widget,
            statusBar=lambda: Mock(),
        ),
        worker_manager=worker_manager,
        _get_image_paths=lambda: ["photo.jpg"],
    )

    AppController.start_fix_rotation_workflow(controller)

    widget.show_results.assert_called_once_with({})
    worker_manager.start_fix_rotation_detection.assert_not_called()

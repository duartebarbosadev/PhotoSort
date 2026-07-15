from pathlib import Path
from types import SimpleNamespace
from unittest.mock import Mock

import numpy as np
from PIL import Image

from core.image_pipeline import ANALYSIS_CACHE_RESOLUTION
from core.similarity_engine import SimilarityEngine
from ui.app_controller import AppController
from workers.easy_delete_worker import EasyDeleteWorker
from workers.pick_best_worker import PickBestWorker


def test_easy_delete_reuses_shared_neutral_analysis_image():
    pipeline = Mock()
    pipeline.get_analysis_image.return_value = Image.new("RGB", (640, 480), "gray")
    worker = EasyDeleteWorker(["photo.arw"], image_pipeline=pipeline)

    gray = worker._load_gray_for_detection("photo.arw")

    assert gray is not None and gray.shape == (480, 640)
    pipeline.get_analysis_image.assert_called_once_with(
        "photo.arw",
        target_size=(640, 480),
    )


def test_completed_empty_easy_delete_result_is_not_recomputed():
    widget = SimpleNamespace(show_results=Mock(), show_loading=Mock())
    worker_manager = SimpleNamespace(
        is_easy_delete_running=lambda: False,
        start_easy_delete_analysis=Mock(),
    )
    controller = SimpleNamespace(
        app_state=SimpleNamespace(
            image_files_data=[{"path": "photo.jpg", "media_type": "image"}],
            easy_delete_results={},
            cluster_results={"photo.jpg": 1},
        ),
        main_window=SimpleNamespace(
            easy_delete_step_widget=widget,
            statusBar=lambda: Mock(),
        ),
        worker_manager=worker_manager,
        _easy_delete_pending_after_similarity=False,
        _start_easy_delete_detection=Mock(),
    )

    AppController.start_easy_delete_workflow(controller)

    widget.show_results.assert_called_once_with({})
    controller._start_easy_delete_detection.assert_not_called()


def test_pick_best_reuses_shared_analysis_image():
    pipeline = Mock()
    expected = Image.new("RGB", (1024, 700), "navy")
    pipeline.get_analysis_image.return_value = expected
    worker = PickBestWorker({}, image_pipeline=pipeline)

    result = worker._load_preview_image(Path("photo.arw"))

    assert result is expected
    pipeline.get_analysis_image.assert_called_once_with(
        "photo.arw",
        target_size=ANALYSIS_CACHE_RESOLUTION,
    )


def test_similarity_uses_shared_analysis_images_instead_of_full_processing():
    pipeline = Mock()
    pipeline.get_analysis_image.return_value = Image.new("RGB", (1024, 700), "teal")
    engine = SimilarityEngine(image_pipeline=pipeline)
    engine._load_model = Mock(return_value=True)
    engine._load_cached_embeddings = Mock(return_value={})
    engine._load_cached_regional_embeddings = Mock(return_value={})
    engine._save_embeddings_to_cache = Mock()
    engine._save_regional_embeddings_to_cache = Mock()
    engine.model.encode_with_regions = Mock(
        return_value=(
            np.asarray([[1.0, 0.0]], dtype=np.float32),
            [np.asarray([[1.0, 0.0]], dtype=np.float32)],
        )
    )

    engine.generate_embeddings_for_files(["photo.arw"])

    pipeline.get_analysis_image.assert_called_once_with(
        "photo.arw",
        target_size=ANALYSIS_CACHE_RESOLUTION,
    )
    assert not pipeline.get_pil_image_for_processing.called

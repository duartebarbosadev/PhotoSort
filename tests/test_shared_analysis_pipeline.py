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


def test_easy_delete_duplicate_result_has_one_authoritative_classification(
    tmp_path, monkeypatch
):
    first = tmp_path / "first.jpg"
    second = tmp_path / "second.jpg"
    first.write_bytes(b"identical image bytes")
    second.write_bytes(b"identical image bytes")
    embeddings = {
        str(first): [1.0, 0.0],
        str(second): [1.0, 0.0],
    }
    worker = EasyDeleteWorker(
        [str(first), str(second)],
        cluster_map={1: [str(first), str(second)]},
        embeddings_cache=embeddings,
    )
    monkeypatch.setattr(worker, "_get_sharpness", lambda _path: 10.0)

    results = worker._detect_duplicates()

    assert {entry["duplicate_kind"] for entry in results.values()} == {"exact"}
    suggested = next(entry for entry in results.values() if entry["suggest_delete"])
    assert suggested["reason"] == "The files are byte-for-byte identical"
    assert suggested["delete_suggestion_reason"] == "byte-for-byte identical"
    assert suggested["keep_suggestion_reason"] == "byte-for-byte identical"


def test_easy_delete_duplicate_result_explains_sharpness_recommendation(
    tmp_path, monkeypatch
):
    softer = tmp_path / "softer.jpg"
    sharper = tmp_path / "sharper.jpg"
    softer.write_bytes(b"soft image")
    sharper.write_bytes(b"sharp image")
    worker = EasyDeleteWorker(
        [str(softer), str(sharper)],
        cluster_map={1: [str(softer), str(sharper)]},
        embeddings_cache={str(softer): [1.0, 0.0], str(sharper): [1.0, 0.0]},
    )
    sharpness = {str(softer): 10.0, str(sharper): 25.0}
    monkeypatch.setattr(worker, "_get_sharpness", sharpness.__getitem__)

    results = worker._detect_duplicates()

    suggested = results[str(softer)]
    assert suggested["suggest_delete"]
    assert suggested["delete_suggestion_reason"] == "lower sharpness (10.0 vs 25.0)"
    assert suggested["keep_suggestion_reason"] == "higher sharpness (25.0 vs 10.0)"


def test_easy_delete_pairs_closest_available_images_first(tmp_path, monkeypatch):
    paths = [tmp_path / name for name in ("a.jpg", "c.jpg", "b.jpg", "d.jpg")]
    for index, path in enumerate(paths):
        path.write_bytes(f"distinct-{index}".encode())

    angles = {"a.jpg": 0.0, "b.jpg": 0.01, "c.jpg": 0.10, "d.jpg": 0.11}
    embeddings = {
        str(path): [np.cos(angles[path.name]), np.sin(angles[path.name])]
        for path in paths
    }
    worker = EasyDeleteWorker(
        [str(path) for path in paths],
        cluster_map={1: [str(path) for path in paths]},
        embeddings_cache=embeddings,
    )
    monkeypatch.setattr(worker, "_get_sharpness", lambda _path: 10.0)

    results = worker._detect_duplicates()

    selected_pairs = {
        frozenset((path, entry["pair_path"]))
        for path, entry in results.items()
        if entry["suggest_delete"]
    }
    assert selected_pairs == {
        frozenset((str(tmp_path / "a.jpg"), str(tmp_path / "b.jpg"))),
        frozenset((str(tmp_path / "c.jpg"), str(tmp_path / "d.jpg"))),
    }
    assert len(results) == 4


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


def test_easy_delete_rebuilds_embeddings_after_restoring_clusters_only():
    widget = SimpleNamespace(show_results=Mock(), show_loading=Mock())
    controller = SimpleNamespace(
        app_state=SimpleNamespace(
            image_files_data=[{"path": "photo.jpg", "media_type": "image"}],
            easy_delete_results=None,
            cluster_results={"photo.jpg": 1},
            embeddings_cache={},
        ),
        main_window=SimpleNamespace(
            easy_delete_step_widget=widget,
            statusBar=lambda: Mock(),
        ),
        worker_manager=SimpleNamespace(is_easy_delete_running=lambda: False),
        _easy_delete_pending_after_similarity=False,
        _get_image_paths=Mock(return_value=["photo.jpg"]),
        _start_easy_delete_detection=Mock(),
        start_similarity_analysis=Mock(),
    )

    AppController.start_easy_delete_workflow(controller)

    assert controller._easy_delete_pending_after_similarity
    widget.show_loading.assert_called_once_with(
        "Step 1/2: Computing similarity embeddings and clusters…", 0
    )
    controller.start_similarity_analysis.assert_called_once_with()
    controller._start_easy_delete_detection.assert_not_called()


def test_easy_delete_reuses_complete_similarity_inputs_without_recomputing():
    controller = SimpleNamespace(
        app_state=SimpleNamespace(
            image_files_data=[{"path": "photo.jpg", "media_type": "image"}],
            easy_delete_results=None,
            cluster_results={"photo.jpg": 1},
            embeddings_cache={"photo.jpg": [1.0, 0.0]},
        ),
        main_window=SimpleNamespace(
            easy_delete_step_widget=SimpleNamespace(show_loading=Mock()),
            statusBar=lambda: Mock(),
        ),
        worker_manager=SimpleNamespace(is_easy_delete_running=lambda: False),
        _easy_delete_pending_after_similarity=False,
        _get_image_paths=Mock(return_value=["photo.jpg"]),
        _start_easy_delete_detection=Mock(),
        start_similarity_analysis=Mock(),
    )

    AppController.start_easy_delete_workflow(controller)

    controller._start_easy_delete_detection.assert_called_once_with()
    controller.start_similarity_analysis.assert_not_called()


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
    regional_results = []
    engine.regional_embeddings_generated.connect(regional_results.append)

    engine.generate_embeddings_for_files(["photo.arw"])

    pipeline.get_analysis_image.assert_called_once_with(
        "photo.arw",
        target_size=ANALYSIS_CACHE_RESOLUTION,
    )
    assert regional_results == [{"photo.arw": [[1.0, 0.0]]}]
    assert not pipeline.get_pil_image_for_processing.called

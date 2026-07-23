from pathlib import Path
from types import SimpleNamespace
from unittest.mock import Mock

import numpy as np
from PIL import Image

from core import app_settings
from core.image_pipeline import ANALYSIS_CACHE_RESOLUTION
from core.image_features.structural_similarity import (
    aligned_structural_similarity,
    prepare_same_frame_preview,
)
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


def test_easy_delete_dark_detection_requires_almost_complete_black_clipping(
    monkeypatch,
):
    worker = EasyDeleteWorker(["photo.arw"])
    monkeypatch.setattr(worker, "_compute_local_sharpness", lambda _gray: 200.0)

    nearly_black = np.zeros((100, 100), dtype=np.uint8)
    nearly_black[:1, :] = 50
    monkeypatch.setattr(worker, "_load_gray_for_detection", lambda _path: nearly_black)

    result = worker._detect_issue("photo.arw")

    assert result is not None and result["type"] == "dark"
    assert result["black_fraction"] == 0.99
    assert "Effectively black image" in result["reason"]


def test_easy_delete_preserves_dark_city_with_visible_lights(monkeypatch):
    worker = EasyDeleteWorker(["night-city.arw"])
    monkeypatch.setattr(worker, "_compute_local_sharpness", lambda _gray: 0.0)
    night_city = np.full((100, 100), 5, dtype=np.uint8)
    night_city[:10, :] = 80
    monkeypatch.setattr(worker, "_load_gray_for_detection", lambda _path: night_city)

    result = worker._detect_issue("night-city.arw")

    assert night_city.mean() < app_settings.get_easy_delete_dark_threshold()
    assert result is None


def test_easy_delete_preserves_underexposed_preview_with_shadow_variation(
    monkeypatch,
):
    worker = EasyDeleteWorker(["underexposed.arw"])
    monkeypatch.setattr(worker, "_compute_local_sharpness", lambda _gray: 0.0)
    shadow_gradient = np.tile(
        np.linspace(1, 20, 100, dtype=np.uint8),
        (100, 1),
    )
    monkeypatch.setattr(
        worker, "_load_gray_for_detection", lambda _path: shadow_gradient
    )

    result = worker._detect_issue("underexposed.arw")

    assert shadow_gradient.mean() < app_settings.get_easy_delete_dark_threshold()
    assert result is None


def test_same_frame_similarity_tolerates_noise_but_rejects_moved_subject():
    rng = np.random.default_rng(3)
    base = np.full((480, 640), 40, dtype=np.uint8)
    for x in range(20, 620, 40):
        base[:, x : x + 2] = 90 + (x % 100)
    base[160:390, 220:360] = 190
    noisy = np.clip(
        base.astype(np.int16) + 5 + rng.normal(0, 5, base.shape),
        0,
        255,
    ).astype(np.uint8)
    moved = base.copy()
    moved[150:400, 200:370] = 40
    moved[160:390, 360:500] = 190

    base_preview = prepare_same_frame_preview(base)
    same_score = aligned_structural_similarity(
        base_preview, prepare_same_frame_preview(noisy)
    )
    moved_score = aligned_structural_similarity(
        base_preview, prepare_same_frame_preview(moved)
    )

    assert same_score is not None and same_score >= 0.98
    assert moved_score is not None and moved_score < 0.98


def test_easy_delete_uses_same_framing_when_cosine_is_outside_cutoff(
    tmp_path, monkeypatch
):
    first = tmp_path / "first.jpg"
    second = tmp_path / "second.jpg"
    first.write_bytes(b"first")
    second.write_bytes(b"second")
    worker = EasyDeleteWorker(
        [str(first), str(second)],
        cluster_map={1: [str(first), str(second)]},
        embeddings_cache={
            str(first): [1.0, 0.0],
            str(second): [0.9811, 0.19350191],
        },
    )
    monkeypatch.setattr(worker, "_same_frame_similarity", lambda *_paths: 0.985)
    monkeypatch.setattr(worker, "_get_sharpness", lambda _path: 10.0)

    results = worker._detect_duplicates()

    assert set(results) == {str(first), str(second)}
    assert {entry["structural_similarity"] for entry in results.values()} == {0.985}
    assert all(entry["cosine_similarity"] < 0.995 for entry in results.values())


def test_easy_delete_rejects_moved_subject_even_when_cosine_is_inside_cutoff(
    tmp_path, monkeypatch
):
    first = tmp_path / "first.jpg"
    second = tmp_path / "second.jpg"
    first.write_bytes(b"first")
    second.write_bytes(b"second")
    worker = EasyDeleteWorker(
        [str(first), str(second)],
        cluster_map={1: [str(first), str(second)]},
        embeddings_cache={str(first): [1.0, 0.0], str(second): [1.0, 0.0]},
    )
    monkeypatch.setattr(worker, "_same_frame_similarity", lambda *_paths: 0.90)

    assert worker._detect_duplicates() == {}


def test_easy_delete_structural_check_reuses_first_pass_analysis_images(monkeypatch):
    pipeline = Mock()
    pipeline.get_analysis_image.return_value = Image.new("RGB", (640, 480), "gray")
    paths = ["first.arw", "second.arw"]
    worker = EasyDeleteWorker(
        paths,
        cluster_map={1: paths},
        embeddings_cache={
            paths[0]: [1.0, 0.0],
            paths[1]: [0.9811, 0.19350191],
        },
        image_pipeline=pipeline,
    )
    monkeypatch.setattr(worker, "_files_are_identical", lambda *_paths: False)

    worker._run()

    assert pipeline.get_analysis_image.call_count == 2


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

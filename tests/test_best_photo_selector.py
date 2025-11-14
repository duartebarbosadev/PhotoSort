from __future__ import annotations

import os
from pathlib import Path
from typing import Dict
from urllib.parse import urlparse

from PIL import Image

from core.ai.best_photo_selector import BestPhotoSelector, MetricSpec


def _loader_factory():
    def _loader(image_path: str) -> Image.Image:
        img = Image.new("RGB", (32, 32), color="white")
        img.info["source_path"] = image_path
        return img

    return _loader


def _scorer(scores: Dict[str, float]):
    def _score(image: Image.Image) -> float:
        path = image.info["source_path"]
        return scores[path]

    return _score


def test_selector_ranks_images_by_weighted_iqa(tmp_path):
    img_a = str(tmp_path / "a.jpg")
    img_b = str(tmp_path / "b.jpg")

    metric_specs = (
        MetricSpec(name="musiq", weight=0.6, min_score=0.0, max_score=100.0),
        MetricSpec(name="maniqa", weight=0.4, min_score=0.0, max_score=1.0),
    )
    metric_factories = {
        "musiq": _scorer({img_a: 82.0, img_b: 78.0}),
        "maniqa": _scorer({img_a: 0.85, img_b: 0.35}),
    }

    selector = BestPhotoSelector(
        metric_specs=metric_specs,
        metric_factories=metric_factories,
        image_loader=_loader_factory(),
        enable_eye_detection=False,
    )

    results = selector.rank_images([img_b, img_a])
    assert [r.image_path for r in results] == [img_a, img_b]
    assert results[0].metrics["musiq"] > results[1].metrics["musiq"]
    assert results[0].metrics["maniqa"] > results[1].metrics["maniqa"]


def test_selector_clamps_scores_outside_known_range(tmp_path):
    img_a = str(tmp_path / "a.jpg")
    img_b = str(tmp_path / "b.jpg")

    metric_specs = (
        MetricSpec(name="liqe", weight=1.0, min_score=0.0, max_score=100.0),
    )
    metric_factories = {
        "liqe": _scorer({img_a: 150.0, img_b: -10.0}),
    }

    selector = BestPhotoSelector(
        metric_specs=metric_specs,
        metric_factories=metric_factories,
        image_loader=_loader_factory(),
        enable_eye_detection=False,
    )

    results = selector.rank_images([img_b, img_a])
    assert results[0].metrics["liqe"] == 1.0  # clamped upper bound
    assert results[1].metrics["liqe"] == 0.0  # clamped lower bound


def test_selector_handles_partial_metric_failures(tmp_path):
    img_a = str(tmp_path / "a.jpg")
    img_b = str(tmp_path / "b.jpg")

    musiq_scores = {img_a: 75.0, img_b: 80.0}

    def flaky_maniqa(image: Image.Image) -> float:
        if image.info["source_path"] == img_b:
            raise RuntimeError("simulated metric failure")
        return 0.9

    selector = BestPhotoSelector(
        metric_specs=(
            MetricSpec(name="musiq", weight=0.5, min_score=0.0, max_score=100.0),
            MetricSpec(name="maniqa", weight=0.5, min_score=0.0, max_score=1.0),
        ),
        metric_factories={
            "musiq": _scorer(musiq_scores),
            "maniqa": flaky_maniqa,
        },
        image_loader=_loader_factory(),
        enable_eye_detection=False,
    )

    results = selector.rank_images([img_b, img_a])
    assert len(results) == 2
    maniqa_present = {
        result.image_path: ("maniqa" in result.metrics) for result in results
    }
    assert maniqa_present[img_a] is True
    assert maniqa_present[img_b] is False
    musiq_metrics = {result.image_path: result.metrics["musiq"] for result in results}
    assert musiq_metrics[img_a] == 0.75
    assert musiq_metrics[img_b] == 0.8


def test_selector_notifies_weight_download(monkeypatch, tmp_path):
    import torch
    import pyiqa  # type: ignore
    import pyiqa.utils.download_util as download_util  # type: ignore

    messages: list[str] = []

    def status_cb(message: str) -> None:
        messages.append(message)

    cache_dir = tmp_path / "pyiqa"
    monkeypatch.setattr(
        download_util, "DEFAULT_CACHE_DIR", str(cache_dir), raising=False
    )

    def fake_loader(url, model_dir=None, progress=True, file_name=None):
        target_dir = model_dir or str(cache_dir)
        os.makedirs(target_dir, exist_ok=True)
        filename = file_name or os.path.basename(urlparse(url).path)
        destination = os.path.join(target_dir, filename)
        Path(destination).write_bytes(b"weights")
        return destination

    monkeypatch.setattr(download_util, "load_file_from_url", fake_loader)

    class DummyMetric:
        def eval(self):
            return self

        def __call__(self, tensor):
            return torch.tensor([0.5])

    def fake_create_metric(*_, **__):
        download_util.load_file_from_url(
            "https://example.com/musiq_koniq_ckpt-e95806b9.pth"
        )
        return DummyMetric()

    monkeypatch.setattr(pyiqa, "create_metric", fake_create_metric)

    selector = BestPhotoSelector(
        image_loader=_loader_factory(),
        metric_specs=(
            MetricSpec(name="musiq", weight=1.0, min_score=0.0, max_score=1.0),
        ),
        metric_factories={},
        status_callback=status_cb,
        enable_eye_detection=False,
    )

    img_path = str(tmp_path / "a.jpg")
    results = selector.rank_images([img_path])
    assert results
    assert any("Downloading MUSIQ" in msg for msg in messages)
    assert any("MUSIQ weights cached" in msg for msg in messages)


def test_eye_open_probability_influences_ranking(tmp_path):
    img_a = str(tmp_path / "closed.jpg")
    img_b = str(tmp_path / "open.jpg")

    class EyeStub:
        def __init__(self, mapping):
            self.mapping = mapping

        def predict_open_probability(self, image: Image.Image):
            return self.mapping.get(image.info["source_path"], 0.5)

    metric_specs = (
        MetricSpec(name="musiq", weight=1.0, min_score=0.0, max_score=100.0),
    )
    constant_scores = {img_a: 60.0, img_b: 60.0}

    selector = BestPhotoSelector(
        metric_specs=metric_specs,
        metric_factories={"musiq": _scorer(constant_scores)},
        image_loader=_loader_factory(),
        eye_state_analyzer=EyeStub({img_a: 0.1, img_b: 0.9}),
    )

    results = selector.rank_images([img_a, img_b])
    assert [r.image_path for r in results] == [img_b, img_a]
    assert results[0].metrics["eyes_open"] == 0.9

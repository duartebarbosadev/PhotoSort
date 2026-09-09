"""Numerical and batching contracts at the shared model-adapter boundary."""

from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from unittest.mock import Mock

import numpy as np
from PIL import Image
import pytest
import torch
from transformers import AutoModelForImageClassification, BeitConfig

from core.best_photo_finder.config import SelectorConfig
from core.best_photo_finder.devices import ResolvedDevice
from core.best_photo_finder.scorers import HuggingFaceAestheticScorer


def reference_pixels(images, size):
    pixels = []
    for image in images:
        image = image.resize((size, size), Image.Resampling.BICUBIC)
        array = np.asarray(image, dtype=np.float32) / 255.0
        array = (array - np.array([0.5] * 3, dtype=np.float32)) / np.array(
            [0.5] * 3, dtype=np.float32
        )
        pixels.append(torch.from_numpy(array).permute(2, 0, 1))
    return torch.stack(pixels)


@pytest.mark.parametrize("batch_size", [1, 3, 8])
def test_aesthetic_batches_preserve_scores_and_reuse_processor(monkeypatch, batch_size):
    torch.manual_seed(7)
    model = AutoModelForImageClassification.from_config(
        BeitConfig(
            image_size=28,
            patch_size=14,
            hidden_size=24,
            num_hidden_layers=1,
            num_attention_heads=3,
            intermediate_size=48,
            id2label={0: "not aesthetic", 1: "aesthetic"},
        )
    ).eval()
    rng = np.random.default_rng(7)
    images = [
        Image.fromarray(rng.integers(0, 256, (19 + i, 41 - i, 3), dtype=np.uint8))
        for i in range(7)
    ]
    inputs = {Path(f"{index}.jpg"): image for index, image in enumerate(images)}
    reference = []
    with torch.no_grad():
        for start in range(0, len(images), batch_size):
            logits = model(
                pixel_values=reference_pixels(images[start : start + batch_size], 28)
            ).logits
            reference.extend(torch.softmax(logits, dim=-1)[:, 1].tolist())
    scorer = HuggingFaceAestheticScorer()
    scorer._model = model
    scorer._resolved_device = ResolvedDevice("cpu", "cpu", "cpu", "float32")
    scorer._aesthetic_label_index = 1
    np.testing.assert_allclose(
        scorer._preprocess_batch(images),
        reference_pixels(images, 28),
        rtol=0,
        atol=1e-7,
    )
    processor = scorer._image_processor
    wrapped = Mock(wraps=processor)
    scorer._image_processor = wrapped
    forward_modes = []
    handle = model.register_forward_pre_hook(
        lambda _model, _inputs: forward_modes.append(torch.is_inference_mode_enabled())
    )
    original_cpu = torch.Tensor.cpu
    transfers = []

    def record_cpu(tensor, *args, **kwargs):
        transfers.append(tensor.shape)
        return original_cpu(tensor, *args, **kwargs)

    monkeypatch.setattr(torch.Tensor, "cpu", record_cpu)
    config = SelectorConfig(device="cpu", aesthetic_batch_size=batch_size)
    try:
        with ThreadPoolExecutor(max_workers=1) as worker:
            scores = worker.submit(
                scorer.score_batch_from_images, inputs, config
            ).result(timeout=30)
            second = worker.submit(
                scorer.score_batch_from_images, inputs, config
            ).result(timeout=30)
    finally:
        handle.remove()
    expected_batches = (len(images) + batch_size - 1) // batch_size
    assert list(scores) == list(inputs)
    np.testing.assert_allclose(list(scores.values()), reference, rtol=0, atol=1e-6)
    assert sorted(scores, key=scores.get) == sorted(
        inputs, key=lambda p: reference[int(p.stem)]
    )
    assert scores == second
    assert scorer._model is model
    assert scorer._image_processor is wrapped
    assert wrapped.call_count == expected_batches * 2
    assert forward_modes == [True] * (expected_batches * 2)
    assert len(transfers) == expected_batches * 2
    assert all(len(shape) == 1 and shape[0] <= batch_size for shape in transfers)

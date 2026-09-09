"""Exercise installed dependency APIs through PhotoSort's shared adapters."""

import pyexiv2  # noqa: F401 - initialize native metadata libraries before Qt

import json
from pathlib import Path
from unittest.mock import Mock

import numpy as np
from PIL import Image


def test_dinov2_adapter_loads_local_weights_and_reuses_model(tmp_path, monkeypatch):
    from transformers import AutoModel, BitImageProcessor, Dinov2Config

    from core.similarity_embedding_model import SimilarityEmbeddingModel

    config = Dinov2Config(
        image_size=28,
        patch_size=14,
        hidden_size=24,
        num_hidden_layers=1,
        num_attention_heads=3,
    )
    AutoModel.from_config(config).save_pretrained(tmp_path)
    BitImageProcessor(
        size={"shortest_edge": 28}, crop_size={"height": 28, "width": 28}
    ).save_pretrained(tmp_path)
    monkeypatch.setattr(
        "core.similarity_embedding_model.get_preferred_torch_device", lambda: "cpu"
    )
    adapter = SimilarityEmbeddingModel()
    adapter.load(snapshot_path=str(tmp_path))
    model, processor = adapter.model, adapter.processor
    adapter.load(snapshot_path=str(tmp_path / "must-not-be-loaded"))
    assert adapter.model is model
    assert adapter.processor is processor
    image = Image.new("RGB", (56, 56), "teal")
    vectors, patches = adapter.encode_with_patches([image])
    assert vectors.shape == (1, 24)
    assert patches[0].shape == (4, 24)
    assert np.isfinite(vectors).all()
    np.testing.assert_allclose(np.linalg.norm(vectors, axis=1), 1, atol=1e-5)


def test_aesthetic_adapter_scores_with_local_beit_and_reuses_model(
    tmp_path, monkeypatch
):
    from transformers import AutoModelForImageClassification, BeitConfig

    from core.best_photo_finder.config import SelectorConfig
    from core.best_photo_finder.scorers import HuggingFaceAestheticScorer

    model_config = BeitConfig(
        image_size=28,
        patch_size=14,
        hidden_size=24,
        num_hidden_layers=1,
        num_attention_heads=3,
        intermediate_size=48,
        id2label={0: "not aesthetic", 1: "aesthetic"},
    )
    AutoModelForImageClassification.from_config(model_config).save_pretrained(tmp_path)
    scorer = HuggingFaceAestheticScorer()
    resolve = Mock(return_value=str(tmp_path))
    monkeypatch.setattr(HuggingFaceAestheticScorer, "_resolve_model_snapshot", resolve)
    config = SelectorConfig(device="cpu")
    path = Path("photo.jpg")
    images = {path: Image.new("RGB", (56, 56), "teal")}
    first = scorer.score_batch_from_images(images, config)
    second = scorer.score_batch_from_images(images, config)
    assert 0 <= first[path] <= 1
    assert first == second
    resolve.assert_called_once_with()


def test_mediapipe_adapter_runs_bundled_face_model():
    from core.best_photo_finder.scorers import MediaPipeTasksFaceLandmarker
    from core.runtime_paths import resolve_face_landmarker_model_path

    adapter = MediaPipeTasksFaceLandmarker(resolve_face_landmarker_model_path())
    try:
        assert not adapter.detect_landmarks(np.zeros((64, 64, 3), dtype=np.uint8))
    finally:
        adapter.close()


def test_openai_sdk_serializes_rating_request_and_parses_tool_response(monkeypatch):
    import httpx2
    import openai

    from core.ai.ai_rating_pipeline import LLMConfig, LLMAiRatingStrategy

    requests = []

    def respond(request):
        requests.append(request)
        if request.url.path.endswith("/models"):
            return httpx2.Response(
                200, json={"object": "list", "data": [{"id": "test-model"}]}
            )
        return httpx2.Response(
            200,
            json={
                "id": "test-completion",
                "object": "chat.completion",
                "created": 0,
                "model": "test-model",
                "choices": [
                    {
                        "index": 0,
                        "finish_reason": "tool_calls",
                        "message": {
                            "role": "assistant",
                            "content": None,
                            "tool_calls": [
                                {
                                    "id": "rating",
                                    "type": "function",
                                    "function": {
                                        "name": "rate_photo",
                                        "arguments": json.dumps(
                                            {
                                                "overall_rating": 4,
                                                "notes": "Sharp image",
                                            }
                                        ),
                                    },
                                }
                            ],
                        },
                    }
                ],
            },
        )

    real_client = openai.OpenAI
    transport = httpx2.MockTransport(respond)
    monkeypatch.setattr(
        openai,
        "OpenAI",
        lambda **kw: real_client(
            **kw, http_client=httpx2.Client(transport=transport), max_retries=0
        ),
    )
    pipeline = Mock()
    pipeline.get_analysis_image.return_value = Image.new("RGB", (8, 8), "teal")
    strategy = LLMAiRatingStrategy(
        pipeline,
        LLMConfig(
            api_key="test-key",
            model="test-model",
            base_url="https://example.invalid/v1",
        ),
    )
    try:
        strategy.validate_connection()
        result = strategy.rate_image("photo.jpg")
        assert result["rating"] == 4
        assert len(requests) == 2
        body = json.loads(requests[1].content)
        assert body["model"] == "test-model"
        assert body["tool_choice"] == "required"
        assert body["messages"][-1]["content"][1]["image_url"]["url"].startswith(
            "data:image/png;base64,"
        )
        pipeline.get_analysis_image.assert_called_once()
    finally:
        strategy.shutdown()
    assert strategy._client.is_closed()


def test_heif_preview_reuses_shared_cache(tmp_path, monkeypatch):
    from pillow_heif import register_heif_opener

    from core.image_pipeline import ImagePipeline
    from core.image_processing.standard_image_processor import StandardImageProcessor

    register_heif_opener()
    path = tmp_path / "photo.heic"
    Image.new("RGB", (80, 40), "teal").save(path, format="HEIF")
    pipeline = ImagePipeline(
        thumbnail_cache_dir=str(tmp_path / "thumbnails"),
        preview_cache_dir=str(tmp_path / "previews"),
    )
    decode = Mock(wraps=StandardImageProcessor.load_as_pil)
    monkeypatch.setattr(StandardImageProcessor, "load_as_pil", decode)
    try:
        first = pipeline.get_preview_image(str(path), (40, 40))
        second = pipeline.get_preview_image(str(path), (40, 40))
        assert first.size == second.size == (40, 20)
        assert first.tobytes() == second.tobytes()
        assert decode.call_count == 1
    finally:
        pipeline.thumbnail_cache._cache.close()
        pipeline.preview_cache._cache.close()


def test_jpeg_metadata_rating_round_trip(tmp_path):
    from core.pyexiv2_wrapper import PyExiv2Operations

    path = tmp_path / "photo.jpg"
    Image.new("RGB", (80, 40), "teal").save(path)
    assert PyExiv2Operations.set_rating(str(path), 4)
    assert PyExiv2Operations.get_rating(str(path)) == 4

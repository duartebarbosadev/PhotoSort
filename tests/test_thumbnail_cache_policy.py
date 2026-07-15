from unittest.mock import MagicMock, Mock

from core.image_pipeline import ImagePipeline


def _pipeline_for_policy(cache_contains: bool):
    pipeline = ImagePipeline.__new__(ImagePipeline)
    pipeline.thumbnail_cache = MagicMock()
    pipeline.thumbnail_cache.__contains__.return_value = cache_contains
    pipeline.thumbnail_cache_key = Mock(return_value=("image.jpg", False, 1, 2))
    pipeline._memory_get = Mock(return_value=None)
    pipeline._get_pil_thumbnail = Mock(return_value=object())
    return pipeline


def test_disk_only_warming_does_not_decode_or_promote_existing_cache_entry():
    pipeline = _pipeline_for_policy(cache_contains=True)

    assert pipeline.ensure_thumbnail_cached("image.jpg", promote_to_memory=False)

    pipeline._get_pil_thumbnail.assert_not_called()


def test_disk_only_cache_miss_generates_without_memory_promotion():
    pipeline = _pipeline_for_policy(cache_contains=False)

    assert pipeline.ensure_thumbnail_cached("image.jpg", promote_to_memory=False)

    pipeline._get_pil_thumbnail.assert_called_once_with(
        "image.jpg", promote_to_memory=False
    )

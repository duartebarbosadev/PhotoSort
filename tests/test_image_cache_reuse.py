import os
import time
from concurrent.futures import ThreadPoolExecutor
from types import SimpleNamespace
from unittest.mock import Mock, patch

from PIL import Image

from core.caching.preview_cache import PreviewCache
from core.caching.thumbnail_cache import ThumbnailCache
from core.image_pipeline import CACHE_SCHEMA_VERSION, ImagePipeline
from core.grouping import _run_ml_similarity_pipeline


def test_review_pixmap_reuses_highest_quality_cache_without_redundant_lookups():
    analysis = Mock()
    analysis.isNull.return_value = False
    pipeline = SimpleNamespace(
        get_cached_analysis_qpixmap=Mock(return_value=analysis),
        get_cached_preview_qpixmap=Mock(),
        get_cached_thumbnail_qpixmap=Mock(),
    )

    result = ImagePipeline.get_cached_review_qpixmap(
        pipeline,
        "/tmp/photo.jpg",
        thumbnail_apply_orientation=True,
    )

    assert result is analysis
    pipeline.get_cached_analysis_qpixmap.assert_called_once_with(
        "/tmp/photo.jpg", memory_only=True
    )
    pipeline.get_cached_preview_qpixmap.assert_not_called()
    pipeline.get_cached_thumbnail_qpixmap.assert_not_called()


def test_disk_caches_store_compressed_payloads_and_return_images(tmp_path):
    image = Image.new("RGB", (800, 600), "teal")
    thumbnail_cache = ThumbnailCache(str(tmp_path / "thumb"))
    preview_cache = PreviewCache(str(tmp_path / "preview"))
    thumbnail_key = ("source.jpg", "thumbnail", CACHE_SCHEMA_VERSION)
    preview_key = ("source.jpg", "preview", CACHE_SCHEMA_VERSION)

    thumbnail_cache.set(thumbnail_key, image)
    preview_cache.set(preview_key, image)

    thumbnail_payload = thumbnail_cache._cache.get(thumbnail_key)
    preview_payload = preview_cache._cache.get(preview_key)
    assert isinstance(thumbnail_payload, bytes)
    assert isinstance(preview_payload, bytes)
    assert len(thumbnail_payload) < image.width * image.height
    assert len(preview_payload) < image.width * image.height
    assert thumbnail_cache.get(thumbnail_key).size == image.size
    assert preview_cache.get(preview_key).size == image.size


def test_cache_key_changes_when_source_file_changes(tmp_path):
    source = tmp_path / "source.jpg"
    source.write_bytes(b"first")
    pipeline = ImagePipeline(
        thumbnail_cache_dir=str(tmp_path / "thumb"),
        preview_cache_dir=str(tmp_path / "preview"),
    )
    first_key = pipeline.preview_cache_key(str(source), (800, 600))

    source.write_bytes(b"second version")
    updated_ns = time.time_ns() + 1_000_000
    os.utime(source, ns=(updated_ns, updated_ns))
    second_key = pipeline.preview_cache_key(str(source), (800, 600))

    assert first_key != second_key
    assert first_key[0] == second_key[0]
    assert second_key[2] == CACHE_SCHEMA_VERSION


def test_memory_cache_avoids_repeated_disk_decoding(tmp_path):
    pipeline = ImagePipeline(
        thumbnail_cache_dir=str(tmp_path / "thumb"),
        preview_cache_dir=str(tmp_path / "preview"),
    )
    image = Image.new("RGB", (320, 200), "navy")
    cache = Mock()
    cache.get.return_value = image
    key = ("source.jpg", "preview", CACHE_SCHEMA_VERSION, 1)

    assert pipeline._cache_get(cache, key).size == image.size
    assert pipeline._cache_get(cache, key).size == image.size

    cache.get.assert_called_once_with(key)


def test_concurrent_thumbnail_requests_generate_once(tmp_path):
    source = tmp_path / "source.jpg"
    source.write_bytes(b"placeholder")
    pipeline = ImagePipeline(
        thumbnail_cache_dir=str(tmp_path / "thumb"),
        preview_cache_dir=str(tmp_path / "preview"),
    )

    def generate(*_args, **_kwargs):
        time.sleep(0.02)
        return Image.new("RGB", (256, 160), "orange")

    with patch(
        "core.image_pipeline.StandardImageProcessor.process_for_thumbnail",
        side_effect=generate,
    ) as processor:
        with ThreadPoolExecutor(max_workers=4) as executor:
            images = list(
                executor.map(
                    lambda _: pipeline._get_pil_thumbnail(str(source)), range(4)
                )
            )

    assert all(image is not None for image in images)
    processor.assert_called_once()


def test_high_memory_thumbnail_formats_obey_dynamic_decode_limit(tmp_path):
    sources = []
    for index in range(4):
        source = tmp_path / f"source-{index}.heic"
        source.write_bytes(b"placeholder")
        sources.append(str(source))
    with patch(
        "core.app_settings.calculate_high_memory_decode_workers", return_value=2
    ):
        pipeline = ImagePipeline(
            thumbnail_cache_dir=str(tmp_path / "thumb"),
            preview_cache_dir=str(tmp_path / "preview"),
        )
    active = 0
    max_active = 0

    def generate(*_args, **_kwargs):
        nonlocal active, max_active
        active += 1
        max_active = max(max_active, active)
        time.sleep(0.02)
        active -= 1
        return Image.new("RGB", (256, 160), "orange")

    with patch(
        "core.image_pipeline.StandardImageProcessor.process_for_thumbnail",
        side_effect=generate,
    ):
        with ThreadPoolExecutor(max_workers=4) as executor:
            list(executor.map(pipeline._get_pil_thumbnail, sources))

    assert max_active == 2


def test_similarity_grouping_reuses_the_shared_pipeline():
    shared_pipeline = Mock()
    engine = Mock()
    engine.run_analysis_sync.return_value = ({}, {"source.jpg": 2})

    with patch(
        "core.similarity_engine.SimilarityEngine", return_value=engine
    ) as engine_cls:
        result = _run_ml_similarity_pipeline(
            ["source.jpg"], image_pipeline=shared_pipeline
        )

    engine_cls.assert_called_once_with(image_pipeline=shared_pipeline)
    assert result == {"source.jpg": 2}

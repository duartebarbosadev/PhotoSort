import os
import time
from concurrent.futures import ThreadPoolExecutor
from types import SimpleNamespace
from unittest.mock import Mock, patch

from PIL import Image, ImageChops, ImageStat
from PyQt6.QtWidgets import QApplication

from core.caching.preview_cache import PreviewCache
from core.caching.thumbnail_cache import ThumbnailCache
from core.image_pipeline import (
    CACHE_SCHEMA_VERSION,
    REVIEW_PROXY_MAX_RESOLUTION,
    ImagePipeline,
)
from core.image_processing.standard_image_processor import StandardImageProcessor
from core.grouping import _run_ml_similarity_pipeline

_app = QApplication.instance() or QApplication([])


def test_review_pixmap_reuses_highest_quality_cache_without_redundant_lookups():
    preview = Mock()
    preview.isNull.return_value = False
    pipeline = SimpleNamespace(
        get_cached_analysis_qpixmap=Mock(),
        get_cached_preview_qpixmap=Mock(return_value=preview),
        get_cached_thumbnail_qpixmap=Mock(),
    )

    result = ImagePipeline.get_cached_review_qpixmap(
        pipeline,
        "/tmp/photo.jpg",
        thumbnail_apply_orientation=True,
    )

    assert result is preview
    pipeline.get_cached_analysis_qpixmap.assert_not_called()
    pipeline.get_cached_preview_qpixmap.assert_called_once_with(
        "/tmp/photo.jpg", memory_only=True
    )
    pipeline.get_cached_thumbnail_qpixmap.assert_not_called()


def test_review_pixmap_reuses_oriented_shared_thumbnail_without_redecoding(tmp_path):
    source = tmp_path / "rotated.jpg"
    image = Image.new("RGB", (80, 40), "teal")
    exif = image.getexif()
    exif[274] = 6
    image.save(source, exif=exif)
    pipeline = ImagePipeline(
        thumbnail_cache_dir=str(tmp_path / "thumb"),
        preview_cache_dir=str(tmp_path / "preview"),
    )

    with patch(
        "core.image_pipeline.StandardImageProcessor.load_as_pil",
        wraps=StandardImageProcessor.load_as_pil,
    ) as processor:
        oriented = pipeline._get_pil_thumbnail(str(source))
        pixmap = pipeline.get_cached_review_qpixmap(str(source))
        second_pixmap = pipeline.get_cached_review_qpixmap(str(source))

    assert oriented is not None
    assert oriented.height > oriented.width
    assert pixmap is not None
    assert pixmap.height() > pixmap.width()
    assert second_pixmap is not None
    assert processor.call_count == 1


def test_immediate_review_uses_disk_thumbnail_without_source_decode(tmp_path):
    source = tmp_path / "photo.jpg"
    Image.new("RGB", (320, 200), "teal").save(source)
    pipeline = ImagePipeline(
        thumbnail_cache_dir=str(tmp_path / "thumb"),
        preview_cache_dir=str(tmp_path / "preview"),
    )
    assert pipeline.ensure_thumbnail_cached(str(source))
    with pipeline._memory_cache_lock:
        pipeline._memory_cache.clear()
        pipeline._memory_cache_bytes = 0

    with patch(
        "core.image_pipeline.StandardImageProcessor.process_for_thumbnail",
        side_effect=AssertionError("source image must not be decoded on the UI path"),
    ):
        pixmap, preview_is_cached = pipeline.get_immediate_review_qpixmap(str(source))

    assert pixmap is not None and not pixmap.isNull()
    assert preview_is_cached is False


def test_detail_decode_preserves_oriented_source_quality_and_honors_target(tmp_path):
    source = tmp_path / "portrait.jpg"
    image = Image.new("RGB", (120, 60), "teal")
    exif = image.getexif()
    exif[274] = 6
    image.save(source, exif=exif)
    pipeline = ImagePipeline(
        thumbnail_cache_dir=str(tmp_path / "thumb"),
        preview_cache_dir=str(tmp_path / "preview"),
    )

    assert pipeline.get_source_dimensions(str(source)) == (60, 120)
    full = pipeline.load_detail_image(str(source))
    bounded = pipeline.load_detail_image(str(source), (20, 20))

    assert full is not None and full.size == (60, 120)
    assert bounded is not None and bounded.size == (10, 20)


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
        return Image.new("RGB", (640, 400), "orange")

    with patch(
        "core.image_pipeline.StandardImageProcessor.load_as_pil",
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
        "core.image_pipeline.StandardImageProcessor.load_as_pil",
        side_effect=generate,
    ):
        with ThreadPoolExecutor(max_workers=4) as executor:
            list(executor.map(pipeline._get_pil_thumbnail, sources))

    assert max_active == 2


def test_raw_proxy_and_detail_share_one_canonical_appearance(tmp_path):
    source = os.path.join(os.path.dirname(__file__), "samples", "arw_sample.ARW")
    pipeline = ImagePipeline(
        thumbnail_cache_dir=str(tmp_path / "thumb"),
        preview_cache_dir=str(tmp_path / "preview"),
    )

    prepared = pipeline.ensure_review_assets_cached(source)
    proxy = pipeline.preview_cache.get(
        pipeline.preview_cache_key(source, REVIEW_PROXY_MAX_RESOLUTION)
    )
    detail = pipeline.load_detail_image(source)

    assert prepared.success
    assert proxy is not None and detail is not None
    detail.thumbnail(proxy.size, Image.Resampling.LANCZOS)
    difference = ImageStat.Stat(
        ImageChops.difference(proxy.convert("RGB"), detail.convert("RGB"))
    ).mean
    assert max(difference) < 4.0


def test_review_asset_generation_decodes_standard_source_once(tmp_path):
    source = tmp_path / "source.jpg"
    Image.new("RGB", (1200, 800), "orange").save(source)
    pipeline = ImagePipeline(
        thumbnail_cache_dir=str(tmp_path / "thumb"),
        preview_cache_dir=str(tmp_path / "preview"),
    )

    with patch(
        "core.image_pipeline.StandardImageProcessor.load_as_pil",
        wraps=StandardImageProcessor.load_as_pil,
    ) as decode:
        result = pipeline.ensure_review_assets_cached(str(source))

    assert result.success
    assert decode.call_count == 1
    assert (
        pipeline.preview_cache_key(str(source), REVIEW_PROXY_MAX_RESOLUTION)
        in pipeline.preview_cache
    )
    assert pipeline.thumbnail_cache_key(str(source)) in pipeline.thumbnail_cache


def test_standard_display_downsamples_before_rgba_conversion(tmp_path):
    source = tmp_path / "large.jpg"
    Image.new("RGB", (1200, 800), "orange").save(source)
    converted_sizes = []
    original_convert = Image.Image.convert

    def record_convert(image, *args, **kwargs):
        converted_sizes.append(image.size)
        return original_convert(image, *args, **kwargs)

    with patch.object(
        Image.Image,
        "convert",
        autospec=True,
        side_effect=record_convert,
    ):
        result = StandardImageProcessor.load_as_pil(
            str(source),
            target_mode="RGBA",
            target_size=(300, 300),
        )

    assert result is not None
    assert result.size == (300, 200)
    assert converted_sizes[-1] == result.size
    assert (1200, 800) not in converted_sizes


def test_review_capacity_estimation_stops_before_inspecting_more_sources(tmp_path):
    pipeline = ImagePipeline(
        thumbnail_cache_dir=str(tmp_path / "thumb"),
        preview_cache_dir=str(tmp_path / "preview"),
    )

    with patch.object(
        pipeline,
        "get_source_dimensions",
        side_effect=AssertionError("cancelled estimation must not inspect sources"),
    ):
        result = pipeline.estimate_active_review_cache_bytes(
            ["first.jpg", "second.jpg"],
            should_continue_callback=lambda: False,
        )

    assert result is None


def test_standard_proxy_preserves_full_detail_appearance(tmp_path):
    source = tmp_path / "source.jpg"
    Image.effect_noise((3000, 1800), 48).convert("RGB").save(
        source,
        quality=95,
    )
    pipeline = ImagePipeline(
        thumbnail_cache_dir=str(tmp_path / "thumb"),
        preview_cache_dir=str(tmp_path / "preview"),
    )

    prepared = pipeline.ensure_review_assets_cached(str(source))
    proxy = pipeline.preview_cache.get(
        pipeline.preview_cache_key(str(source), REVIEW_PROXY_MAX_RESOLUTION)
    )
    detail = pipeline.load_detail_image(str(source))

    assert prepared.success
    assert proxy is not None and detail is not None
    detail.thumbnail(proxy.size, Image.Resampling.LANCZOS, reducing_gap=3.0)
    difference = ImageStat.Stat(
        ImageChops.difference(proxy.convert("RGB"), detail.convert("RGB"))
    ).mean
    assert max(difference) < 4.0


def test_failed_preview_write_is_not_reported_as_ready(tmp_path):
    source = tmp_path / "source.jpg"
    Image.new("RGB", (1200, 800), "orange").save(source)
    pipeline = ImagePipeline(
        thumbnail_cache_dir=str(tmp_path / "thumb"),
        preview_cache_dir=str(tmp_path / "preview"),
    )

    with (
        patch.object(pipeline.preview_cache, "set", return_value=0),
        patch.object(pipeline.thumbnail_cache, "set") as thumbnail_write,
    ):
        result = pipeline.ensure_review_assets_cached(str(source))

    assert not result.success
    assert not result.preview_ready
    assert result.error == "Preview cache write failed"
    thumbnail_write.assert_not_called()


def test_failed_thumbnail_write_is_not_reported_as_ready(tmp_path):
    source = tmp_path / "source.jpg"
    Image.new("RGB", (1200, 800), "teal").save(source)
    pipeline = ImagePipeline(
        thumbnail_cache_dir=str(tmp_path / "thumb"),
        preview_cache_dir=str(tmp_path / "preview"),
    )

    with patch.object(pipeline.thumbnail_cache, "set", return_value=0):
        result = pipeline.ensure_review_assets_cached(str(source))

    assert not result.success
    assert result.preview_ready
    assert not result.thumbnail_ready
    assert result.error == "Thumbnail cache write failed"


def test_failed_video_thumbnail_write_is_not_reported_as_ready(tmp_path):
    source = tmp_path / "source.mp4"
    source.write_bytes(b"video")
    pipeline = ImagePipeline(
        thumbnail_cache_dir=str(tmp_path / "thumb"),
        preview_cache_dir=str(tmp_path / "preview"),
    )

    with patch.object(
        pipeline,
        "_get_pil_thumbnail",
        return_value=Image.new("RGBA", (256, 144), "black"),
    ):
        result = pipeline.ensure_review_assets_cached(str(source))

    assert not result.success
    assert not result.thumbnail_ready
    assert result.error == "Video thumbnail cache write failed"


def test_missing_thumbnail_is_derived_from_cached_proxy_without_source_decode(tmp_path):
    source = tmp_path / "source.jpg"
    Image.new("RGB", (1200, 800), "teal").save(source)
    pipeline = ImagePipeline(
        thumbnail_cache_dir=str(tmp_path / "thumb"),
        preview_cache_dir=str(tmp_path / "preview"),
    )
    assert pipeline.ensure_review_assets_cached(str(source)).success
    pipeline.thumbnail_cache.delete(pipeline.thumbnail_cache_key(str(source)))

    with patch(
        "core.image_pipeline.StandardImageProcessor.load_as_pil",
        side_effect=AssertionError("cached proxy must own thumbnail regeneration"),
    ):
        result = pipeline.ensure_review_assets_cached(str(source))

    assert result.success


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

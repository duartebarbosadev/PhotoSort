"""Sequential production preview/thumbnail readiness, with disposable caches.

Run the unchanged reference virtualenv with -B, passing reference and photo folder.
Imports/discovery/cache setup are outside phase timing. This measures service work,
not the Qt window, background preload concurrency, or identical rendering recipes.
"""
import json
import logging
import os
import sys
import tempfile
import time
from pathlib import Path


def main():
    setup = time.perf_counter()
    if len(sys.argv) != 3:
        raise SystemExit("reference project and read-only photo folder required")
    reference, root = (Path(value).resolve() for value in sys.argv[1:])
    sys.dont_write_bytecode = True
    sys.path.insert(0, str(reference / "src"))
    logging.disable(logging.CRITICAL)
    with tempfile.TemporaryDirectory(prefix="photosort-reference-review-") as temporary:
        # Configure only this process; never use or clear the reference's caches.
        os.environ["PHOTOSORT_CACHE_ROOT"] = str(Path(temporary) / "other-cache")
        os.environ["PHOTOSORT_DATA_ROOT"] = str(Path(temporary) / "data")
        # The reference registers this opener in main.py before creating its UI.
        from pillow_heif import register_heif_opener
        register_heif_opener()
        from core.image_pipeline import ImagePipeline, REVIEW_PROXY_MAX_RESOLUTION, THUMBNAIL_MAX_SIZE
        from core.media_utils import SUPPORTED_IMAGE_EXTENSIONS, SUPPORTED_RAW_EXTENSIONS

        extensions = set(SUPPORTED_IMAGE_EXTENSIONS) | set(SUPPORTED_RAW_EXTENSIONS)
        paths = sorted(p for p in root.rglob("*") if p.is_file() and not p.is_symlink()
                       and p.suffix.lower() in extensions)
        if not paths:
            raise RuntimeError("Empty photo library")

        def create():
            return ImagePipeline(thumbnail_cache_dir=str(Path(temporary) / "thumbnails"),
                                 preview_cache_dir=str(Path(temporary) / "previews"))

        def close(pipeline):
            pipeline.thumbnail_cache.close()
            pipeline.preview_cache.close()

        pipeline = create()
        setup_ms = (time.perf_counter() - setup) * 1000
        phases, expected = [], None
        succeeded = True
        # Count actual source render requests without replacing their implementation.
        source_renders = 0
        original_render = ImagePipeline.render_display_image

        def measured_render(self, *args, **kwargs):
            nonlocal source_renders
            source_renders += 1
            return original_render(self, *args, **kwargs)

        ImagePipeline.render_display_image = measured_render
        try:
            for phase in ["initial", "warm", "reopened"]:
                if phase == "reopened":
                    close(pipeline)
                    del pipeline
                    pipeline = create()
                before = source_renders
                rows, errors = {}, []
                first_ready_ms = None
                start = time.perf_counter()
                for path in paths:
                    relative = str(path.relative_to(root))
                    result = pipeline.ensure_review_assets_cached(str(path), promote_to_memory=True)
                    if not result.success:
                        errors.append(dict(file=relative, error=result.error))
                        continue
                    preview = pipeline._cache_get(pipeline.preview_cache,
                        pipeline.preview_cache_key(str(path), REVIEW_PROXY_MAX_RESOLUTION))
                    thumbnail = pipeline._cache_get(pipeline.thumbnail_cache,
                        pipeline.thumbnail_cache_key(str(path), True))
                    if preview is None or thumbnail is None:
                        errors.append(dict(file=relative, error="prepared pixels unavailable"))
                        continue
                    if first_ready_ms is None:
                        first_ready_ms = (time.perf_counter() - start) * 1000
                    rows[relative] = dict(preview=preview.size, thumbnail=thumbnail.size)
                    del preview, thumbnail
                elapsed_ms = (time.perf_counter() - start) * 1000
                succeeded &= not errors
                if expected is not None and rows != expected:
                    raise RuntimeError(f"Preview geometry changed on {phase}")
                expected = rows
                disk_bytes = sum(p.stat().st_size for p in Path(temporary).rglob("*") if p.is_file())
                phases.append(dict(phase=phase, count=len(rows), first_ready_ms=first_ready_ms,
                    elapsed_ms=elapsed_ms, source_render_delta=source_renders-before,
                    resident_bytes=pipeline._memory_cache_bytes, disk_bytes=disk_bytes, errors=errors))
            print(json.dumps(dict(implementation="python", setup_ms=setup_ms, count=len(paths),
                preview_edge=max(REVIEW_PROXY_MAX_RESOLUTION), thumbnail_edge=max(THUMBNAIL_MAX_SIZE),
                memory_budget_bytes=pipeline._memory_cache_limit_bytes, concurrent_photo_requests=1,
                phases=phases, rows=expected)))
        finally:
            ImagePipeline.render_display_image = original_render
            close(pipeline)
        if not succeeded:
            raise SystemExit("one or more photos failed; do not compare incomplete work")


if __name__ == "__main__":
    main()

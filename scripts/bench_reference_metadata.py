"""Read metadata through the unchanged Python service using a disposable EXIF cache.

Run with PhotoSort's virtualenv and -B. Imports, discovery, cache construction,
validation hashing and teardown are excluded from each phase's elapsed time.
Reopened means a new cache owner in this process, not a full application restart.
"""
import hashlib
import json
import logging
import sys
import tempfile
import time
from pathlib import Path

reference, fixture = (Path(p).resolve() for p in sys.argv[1:3])
sys.dont_write_bytecode = True
sys.path.insert(0, str(reference / "src"))
from core.metadata_processor import MetadataProcessor
from core.caching.exif_cache import ExifCache
from core.pyexiv2_wrapper import PyExiv2Operations

logging.disable(logging.CRITICAL)
paths = sorted(p for p in fixture.rglob("*") if p.is_file() and p.suffix.lower() in {".jpg", ".jpeg", ".png", ".arw"})
if not paths:
    raise RuntimeError("No photos in fixture")
reads = 0
original = PyExiv2Operations.get_comprehensive_metadata

def photo_dimension(value, exif_key, stored_key):
    # Rust reports declared photo dimensions; Python also retains stored RAW bounds.
    try:
        return int(value[exif_key])
    except (KeyError, TypeError, ValueError):
        return int(value.get(stored_key, 0))


def measured_read(path):
    global reads
    reads += 1
    return original(path)

PyExiv2Operations.get_comprehensive_metadata = staticmethod(measured_read)
phases = []
expected = None
with tempfile.TemporaryDirectory(prefix="photosort-python-metadata-") as folder:
    cache = ExifCache(folder)
    try:
        for phase in ["initial", "warm", "reopened", "rewarmed"]:
            if phase == "reopened":
                cache.close()
                cache = ExifCache(folder)
            before = reads
            started = time.perf_counter()
            values = [MetadataProcessor.get_detailed_metadata(str(path), cache) for path in paths]
            seconds = time.perf_counter() - started
            if any(not value or "error" in value for value in values):
                raise RuntimeError("Metadata extraction failed")
            rows = [[str(path.relative_to(fixture)), photo_dimension(value, "Exif.Photo.PixelXDimension", "pixel_width"),
                     photo_dimension(value, "Exif.Photo.PixelYDimension", "pixel_height"), int(value.get("Exif.Image.Orientation", 1)),
                     value.get("Exif.Image.Model", ""), value.get("Exif.Photo.LensModel", "")]
                    for path, value in zip(paths, values)]
            dimension_differences = [[str(path.relative_to(fixture)), int(value.get("pixel_width", 0)),
                                      int(value.get("pixel_height", 0)), row[1], row[2]]
                                     for path, value, row in zip(paths, values, rows)
                                     if [int(value.get("pixel_width", 0)), int(value.get("pixel_height", 0))] != row[1:3]]
            checksum = hashlib.sha256(json.dumps(rows, ensure_ascii=False, separators=(",", ":")).encode()).hexdigest()
            if expected is not None and checksum != expected:
                raise RuntimeError("Metadata changed between passes")
            expected = checksum
            phases.append(dict(phase=phase, seconds=seconds, count=len(rows), source_reads=reads-before,
                               sha256=checksum, sample_rows=rows[:7], dimension_difference_count=len(dimension_differences),
                               dimension_difference_examples=dimension_differences[:7]))
            del values, rows, dimension_differences  # Do not retain an extra whole-library result between phases.
        limit = cache.get_current_size_limit_bytes()
        cache_bytes = cache.volume()
    finally:
        cache.close()
        PyExiv2Operations.get_comprehensive_metadata = staticmethod(original)
print(json.dumps(dict(implementation="python", phases=phases, cache_limit_bytes=limit, cache_bytes=cache_bytes)))

from fractions import Fraction

from core.metadata_processor import (
    DATE_TAGS_PREFERENCE,
    MetadataProcessor,
    _parse_exif_date,
)


def _first_present(metadata: dict, *keys: str):
    for key in keys:
        value = metadata.get(key)
        if value not in (None, "", "None"):
            return value
    return None


def _fraction_text(value: object) -> str | None:
    if value in (None, ""):
        return None
    text = str(value).strip()
    try:
        if "/" in text:
            fraction = Fraction(text)
            if fraction >= 1:
                return f"{float(fraction):.1f}s"
            return f"1/{round(1 / float(fraction))}s"
        numeric = float(text)
    except TypeError, ValueError, ZeroDivisionError:
        return text
    if numeric >= 1:
        return f"{numeric:.1f}s"
    if numeric <= 0:
        return text
    return f"1/{round(1 / numeric)}s"


def _float_text(
    value: object, prefix: str = "", suffix: str = "", digits: int = 1
) -> str | None:
    if value in (None, ""):
        return None
    try:
        return f"{prefix}{float(value):.{digits}f}{suffix}"
    except TypeError, ValueError:
        return f"{prefix}{value}{suffix}"


def _format_capture_date(metadata: dict) -> str | None:
    for key in DATE_TAGS_PREFERENCE:
        raw_value = metadata.get(key)
        if raw_value in (None, "", "None"):
            continue
        parsed = _parse_exif_date(str(raw_value))
        if parsed is not None:
            if parsed.hour == 0 and parsed.minute == 0 and parsed.second == 0:
                return parsed.strftime("%Y-%m-%d")
            return parsed.strftime("%Y-%m-%d %H:%M")
        return str(raw_value)
    return None


def build_workflow_metadata_rows(
    path: str, exif_disk_cache, *, limit: int = 6
) -> list[tuple[str, str]]:
    """Format cached EXIF data for workflow decision cards without file I/O."""
    metadata = MetadataProcessor.get_cached_detailed_metadata(path, exif_disk_cache)
    rows: list[tuple[str, str]] = []

    if isinstance(metadata, dict):
        capture_date = _format_capture_date(metadata)
        if capture_date:
            rows.append(("Date", capture_date))

        camera_make = _first_present(
            metadata, "Exif.Image.Make", "Xmp.tiff.Make", "Make"
        )
        camera_model = _first_present(
            metadata, "Exif.Image.Model", "Xmp.tiff.Model", "Model"
        )
        if camera_make or camera_model:
            camera_text = " ".join(
                str(part).strip() for part in (camera_make, camera_model) if part
            )
            rows.append(("Camera", camera_text))

        lens = _first_present(
            metadata,
            "Exif.Photo.LensModel",
            "Xmp.aux.Lens",
            "LensModel",
            "LensInfo",
        )
        if lens:
            rows.append(("Lens", str(lens)))

        focal = _float_text(
            _first_present(metadata, "Exif.Photo.FocalLength", "FocalLength"),
            suffix=" mm",
            digits=0,
        )
        aperture = _float_text(
            _first_present(
                metadata,
                "Exif.Photo.FNumber",
                "Exif.Photo.ApertureValue",
                "FNumber",
            ),
            prefix="f/",
            digits=1,
        )
        if focal or aperture:
            rows.append(("Lens", "  ".join(part for part in (focal, aperture) if part)))

        shutter = _fraction_text(
            _first_present(
                metadata,
                "Exif.Photo.ExposureTime",
                "ExposureTime",
                "Exif.Photo.ShutterSpeedValue",
            )
        )
        iso = _first_present(
            metadata,
            "Exif.Photo.ISOSpeedRatings",
            "ISO",
            "EXIF:ISO",
            "EXIF:ISOSpeedRatings",
        )
        if shutter or iso:
            iso_text = f"ISO {iso}" if iso not in (None, "") else None
            rows.append(
                (
                    "Exposure",
                    "  ".join(part for part in (shutter, iso_text) if part),
                )
            )

        width = _first_present(
            metadata,
            "pixel_width",
            "Exif.Photo.PixelXDimension",
            "Exif.Image.ImageWidth",
        )
        height = _first_present(
            metadata,
            "pixel_height",
            "Exif.Photo.PixelYDimension",
            "Exif.Image.ImageLength",
        )
        if width and height:
            rows.append(("Size", f"{width} × {height}"))

    if not rows:
        rows.append(("Metadata", "No EXIF details available"))

    return rows[:limit]

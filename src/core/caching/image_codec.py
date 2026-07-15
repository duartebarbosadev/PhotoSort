"""Compact, backwards-compatible serialization for disk-cached images."""

from io import BytesIO
from typing import Any

from PIL import Image

JPEG_MARKER = b"PSJ1"
WEBP_MARKER = b"PSW1"
PNG_MARKER = b"PSP1"


def _has_transparency(image: Image.Image) -> bool:
    if image.mode not in {"RGBA", "LA"}:
        return False
    alpha = image.getchannel("A")
    return alpha.getextrema()[0] < 255


def encode_cached_image(image: Image.Image, *, quality: int) -> bytes:
    """Encode a PIL image compactly while preserving meaningful transparency."""
    output = BytesIO()
    if _has_transparency(image):
        try:
            image.save(output, format="WEBP", lossless=True, method=4)
            return WEBP_MARKER + output.getvalue()
        except OSError:
            output = BytesIO()
            image.save(output, format="PNG", optimize=True)
            return PNG_MARKER + output.getvalue()

    image.convert("RGB").save(
        output,
        format="JPEG",
        quality=quality,
        optimize=True,
        progressive=True,
    )
    return JPEG_MARKER + output.getvalue()


def decode_cached_image(value: Any) -> Image.Image | None:
    """Decode current byte payloads and legacy pickled PIL image entries."""
    if isinstance(value, Image.Image):
        return value
    if not isinstance(value, bytes) or len(value) <= 4:
        return None
    if value[:4] not in {JPEG_MARKER, WEBP_MARKER, PNG_MARKER}:
        return None

    with Image.open(BytesIO(value[4:])) as image:
        image.load()
        return image.copy()

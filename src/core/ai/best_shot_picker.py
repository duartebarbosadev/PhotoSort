"""
AI Best Shot Picker Module
Communicates with LM Studio (or compatible OpenAI API) to select the best image
from a group of images using vision model analysis.
"""

import base64
import io
import logging
import mimetypes
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Optional, Dict

try:
    from PIL import Image, ImageDraw, ImageFont
except ImportError:  # pragma: no cover - Pillow is an optional dependency in some envs
    Image = None  # type: ignore
    ImageDraw = None  # type: ignore
    ImageFont = None  # type: ignore

from openai import OpenAI

logger = logging.getLogger(__name__)

# Suppress noisy debug logs from third-party libraries
logging.getLogger("httpcore").setLevel(logging.WARNING)
logging.getLogger("httpx").setLevel(logging.WARNING)
logging.getLogger("openai").setLevel(logging.WARNING)


@dataclass
class BestShotResult:
    """Result from best shot analysis."""

    best_image_index: int  # Index of the best image (0-based)
    best_image_path: str  # Path to the best image
    reasoning: str  # AI's reasoning for the selection
    confidence: str  # Confidence level (if provided)
    raw_response: str  # Full AI response


class BestShotPickerError(Exception):
    """Exception raised when best shot picking fails."""

    pass


class BestShotPicker:
    """
    AI-powered best shot picker using vision models via OpenAI-compatible API.

    This class communicates with LM Studio or any OpenAI-compatible API endpoint
    to analyze multiple images and select the best one based on various criteria
    like focus, composition, exposure, and overall quality.
    """

    def __init__(
        self,
        base_url: str = "http://localhost:1234/v1",
        api_key: str = "not-needed",
        model: str = "local-model",
        timeout: int = 120,
    ):
        """
        Initialize the BestShotPicker.

        Args:
            base_url: The base URL for the API endpoint (default: LM Studio local)
            api_key: API key (not needed for local LM Studio)
            model: Model identifier (placeholder for LM Studio)
            timeout: Request timeout in seconds (default: 120 for vision models)
        """
        self.base_url = base_url
        self.api_key = api_key
        self.model = model
        self.timeout = timeout
        self.client = None
        self.debug_overlay_enabled = True
        logger.info("AI best shot debug overlays enabled (default)")

    def _initialize_client(self):
        """Initialize the OpenAI client."""
        if self.client is None:
            self.client = OpenAI(
                base_url=self.base_url, api_key=self.api_key, timeout=self.timeout
            )

    def _get_base64_image(self, image_path: str) -> str:
        """
        Convert an image file to Base64-encoded string.

        Args:
            image_path: Path to the image file

        Returns:
            Base64-encoded string of the image

        Raises:
            FileNotFoundError: If the image file doesn't exist
            BestShotPickerError: If encoding fails
        """
        try:
            with open(image_path, "rb") as image_file:
                return base64.b64encode(image_file.read()).decode("utf-8")
        except FileNotFoundError:
            raise FileNotFoundError(f"Image file not found: {image_path}")
        except Exception as e:
            raise BestShotPickerError(f"Failed to encode image {image_path}: {e}")

    def _get_overlay_encoded_image(
        self,
        image_path: str,
        mime_type: str,
        label: str,
        pil_image: Optional[Any] = None,
    ) -> tuple[str, str]:
        """Return the encoded payload for an image with a debug label overlay."""

        if Image is None or ImageDraw is None or ImageFont is None:
            raise BestShotPickerError(
                "Pillow is required for debug overlay rendering"
            )

        try:
            if pil_image is not None:
                img = pil_image.convert("RGBA")
            else:
                with Image.open(image_path) as opened_img:
                    img = opened_img.convert("RGBA")

            draw = ImageDraw.Draw(img)
            width, height = img.size
            if width <= 0 or height <= 0:
                raise BestShotPickerError(
                    f"Image {image_path} has invalid dimensions"
                )

            text = (label or "?").strip()
            base_dimension = max(48, int(min(width, height) * 0.18))
            padding = max(6, base_dimension // 6)

            try:
                font = ImageFont.truetype("DejaVuSans-Bold.ttf", base_dimension)
            except (OSError, IOError):  # pragma: no cover - font fallback
                font = ImageFont.load_default()

            text_bbox = draw.textbbox((0, 0), text, font=font)
            text_width = text_bbox[2] - text_bbox[0]
            text_height = text_bbox[3] - text_bbox[1]

            rect_width = text_width + padding * 2
            rect_height = text_height + padding * 2
            rect_coords = (
                padding,
                padding,
                padding + rect_width,
                padding + rect_height,
            )

            draw.rectangle(rect_coords, fill=(0, 0, 0, 192))

            text_x = padding + (rect_width - text_width) / 2
            text_y = padding + (rect_height - text_height) / 2
            draw.text((text_x, text_y), text, fill=(255, 255, 255, 255), font=font)

            return self._encode_pil_image(img, mime_type)

        except FileNotFoundError:
            raise
        except Exception as exc:
            source = image_path if pil_image is None else f"{image_path} (preview)"
            raise BestShotPickerError(
                f"Failed to render debug overlay for {source}: {exc}"
            ) from exc

    def _encode_pil_image(
        self, pil_image: Any, mime_type: str
    ) -> tuple[str, str]:
        if Image is None:
            raise BestShotPickerError("Pillow is required for image encoding")

        buffer = io.BytesIO()
        if mime_type == "image/jpeg":
            pil_image.convert("RGB").save(buffer, format="JPEG", quality=90)
            effective_mime = "image/jpeg"
        else:
            pil_image.convert("RGBA").save(buffer, format="PNG")
            effective_mime = "image/png"

        encoded = base64.b64encode(buffer.getvalue()).decode("utf-8")
        return encoded, effective_mime

    def _prepare_encoded_image(
        self,
        image_path: str,
        mime_type: str,
        debug_label: str | None,
        pil_image: Optional[Any] = None,
    ) -> tuple[str, str]:
        """
        Prepare the base64 payload for an image, optionally adding a debug label.

        Returns:
            Tuple containing (base64_data, mime_type_used).
        """

        if debug_label:
            return self._get_overlay_encoded_image(
                image_path, mime_type, debug_label, pil_image
            )

        if pil_image is not None:
            return self._encode_pil_image(pil_image, mime_type)

        return self._get_base64_image(image_path), mime_type

    def prepare_preview_payload(
        self,
        image_path: str,
        pil_image: Any,
        overlay_label: Optional[str],
        mime_type: str = "image/jpeg",
    ) -> tuple[str, str]:
        """Encode a pre-generated preview image for AI analysis."""

        label = overlay_label if self.debug_overlay_enabled else None
        return self._prepare_encoded_image(
            image_path=image_path,
            mime_type=mime_type,
            debug_label=label,
            pil_image=pil_image,
        )

    def _get_image_url(self, image_path: str) -> str:
        """
        Convert an image file path to a file:// URL or return existing URL.

        Args:
            image_path: Path to the image file or URL

        Returns:
            URL string for the image

        Raises:
            FileNotFoundError: If the image file doesn't exist
        """
        # Check if it's already a URL (http/https)
        if image_path.startswith(("http://", "https://")):
            return image_path
        
        # Convert local file path to file:// URL
        try:
            path = Path(image_path)
            if not path.exists():
                raise FileNotFoundError(f"Image file not found: {image_path}")
            
            # Convert to absolute path and create file:// URL
            absolute_path = path.resolve()
            return absolute_path.as_uri()
        except Exception as e:
            raise BestShotPickerError(f"Failed to create URL for image {image_path}: {e}")

    def _build_prompt(
        self, image_count: int, include_debug_instruction: bool = False
    ) -> str:
        """
        Build the prompt for the AI to analyze images.

        Args:
            image_count: Number of images being analyzed

        Returns:
            Formatted prompt string
        """
        prompt = f"""You are an expert photography critic tasked with selecting the best image from a set of {image_count} images.

Analyze each image based on the following criteria:
- **Sharpness and Focus**: Is the subject in focus? Are there motion blur or focus issues?
- **Color/Lightining**: Color & Lighting – accuracy, contrast, saturation, white balance, and visual appeal.
- **Composition**: Does the image follow framing, subject placement, use of space principles?
- **Subject Expression**: For portraits, does the subject have a good expression (eyes open, natural smile)?
- **Technical Quality**: Are there any artifacts, noise, or technical issues?
- **Overall Appeal**: Which image is most visually appealing?
- **Editing Potential**: – how well the image could respond to color grading, retouching, or enhancement.
- **Subject Sharpness** – focus quality, motion blur, and clarity of the main subject.

If any person’s eyes are closed, the photo automatically receives a low rating (1–2) regardless of other factors, unless it's a clear visual choice.

Please analyze each image and then provide your response in the following format:

**Best Image**: [Image number, 1-{image_count}]
**Confidence**: [High/Medium/Low]
**Reasoning**: [Brief explanation of why this image is the best]

Be decisive and pick ONE image as the best, even if the differences are subtle."""

        if include_debug_instruction:
            prompt += (
                "\n\nEach image has a bold verification number in the corner. "
                "In your response add a new line exactly as follows:"
                "\n**Overlay Number**: [the number printed on the selected image]"
                "\nAlso reference the overlay numbers when comparing images in your reasoning so we can confirm alignment."
            )

        return prompt

    def _parse_response(
        self, response: str, image_paths: list[str]
    ) -> BestShotResult:
        """
        Parse the AI response to extract the best image selection.

        Args:
            response: Raw response from the AI
            image_paths: List of image paths in order

        Returns:
            BestShotResult with parsed information

        Raises:
            BestShotPickerError: If parsing fails
        """
        try:
            # Look for "Best Image: X" pattern
            import re

            # Try to find the image number in various formats
            patterns = [
                r"(?i)\*\*Best Image\*\*:\s*(?:Image\s*)?(\d+)",  # **Best Image**: Image 3
                r"(?i)Best Image:\s*(?:Image\s*)?(\d+)",  # Best Image: 3
                r"(?i)Image\s*(\d+)\s+is\s+(?:the\s+)?best",  # Image 3 is best
                r"(?i)select(?:ed)?\s+(?:image\s*)?(\d+)",  # Selected image 3
                r"(?i)choose\s+(?:image\s*)?(\d+)",  # Choose image 3
            ]

            best_index = None
            for pattern in patterns:
                match = re.search(pattern, response)
                if match:
                    image_num = int(match.group(1))
                    logger.debug(f"Parsed image number from response: {image_num}")
                    if 1 <= image_num <= len(image_paths):
                        best_index = image_num - 1  # Convert to 0-based index
                        logger.debug(f"Converted to 0-based index: {best_index}")
                        logger.debug(f"Corresponding path: {image_paths[best_index]}")
                        break
                    else:
                        logger.warning(f"Image number {image_num} out of range (1-{len(image_paths)})")

            if best_index is None:
                logger.warning(f"Could not parse image number from response: {response}")
                # Default to first image if parsing fails
                best_index = 0
                reasoning = "Failed to parse AI response. Defaulting to first image."
                confidence = "Unknown"
            else:
                # Extract reasoning
                reasoning_match = re.search(
                    r"(?i)\*\*Reasoning\*\*:\s*(.+?)(?:\n\n|\Z)", response, re.DOTALL
                )
                if reasoning_match:
                    reasoning = reasoning_match.group(1).strip()
                else:
                    # Try to extract any explanation text
                    reasoning = response.split("**Reasoning**:")[-1].strip()
                    if not reasoning:
                        reasoning = "No detailed reasoning provided."

                # Extract confidence
                confidence_match = re.search(
                    r"(?i)\*\*Confidence\*\*:\s*(\w+)", response
                )
                if confidence_match:
                    confidence = confidence_match.group(1)
                else:
                    confidence = "Not specified"

            return BestShotResult(
                best_image_index=best_index,
                best_image_path=image_paths[best_index],
                reasoning=reasoning,
                confidence=confidence,
                raw_response=response,
            )

        except Exception as e:
            logger.error(f"Failed to parse AI response: {e}")
            raise BestShotPickerError(f"Failed to parse AI response: {e}")

    def select_best_image(
        self,
        image_paths: list[str],
        max_tokens: int = 1000,
        stream: bool = False,
        preview_overrides: Optional[Dict[str, Dict[str, Any]]] = None,
    ) -> BestShotResult:
        """
        Analyze multiple images and select the best one.

        Args:
            image_paths: List of paths to images to analyze
            max_tokens: Maximum tokens in the response
            stream: Whether to stream the response
            preview_overrides: Optional mapping of image paths to pre-encoded
                preview payloads containing ``base64``, ``mime_type``, and an
                optional ``overlay_label``.

        Returns:
            BestShotResult containing the selection and reasoning

        Raises:
            ValueError: If image_paths is empty or contains only one image
            BestShotPickerError: If the API call or analysis fails
        """
        if not image_paths:
            raise ValueError("No images provided for analysis")

        if len(image_paths) == 1:
            logger.info("Only one image provided, returning it as the best")
            return BestShotResult(
                best_image_index=0,
                best_image_path=image_paths[0],
                reasoning="Only one image provided",
                confidence="High",
                raw_response="Single image - no comparison needed",
            )

        logger.info(f"Analyzing {len(image_paths)} images to select the best one")

        prepared_images: list[dict[str, Any]] = []
        skipped_paths: list[str] = []
        overrides = preview_overrides or {}
        mime_type_map = {
            ".jpg": "image/jpeg",
            ".jpeg": "image/jpeg",
            ".jpe": "image/jpeg",
            ".jfif": "image/jpeg",
            ".png": "image/png",
            ".webp": "image/webp",
            ".gif": "image/gif",
            ".bmp": "image/bmp",
            ".tif": "image/tiff",
            ".tiff": "image/tiff",
            ".heif": "image/heif",
            ".heic": "image/heic",
            ".avif": "image/avif",
        }

        for original_index, image_path in enumerate(image_paths):
            debug_label = (
                str(len(prepared_images) + 1) if self.debug_overlay_enabled else None
            )

            try:
                override_payload = overrides.get(image_path)
                overlay_label = None

                if override_payload:
                    base64_image = override_payload.get("base64")
                    if not base64_image:
                        logger.warning(
                            "Preview override missing base64 data for %s; falling back to source",
                            image_path,
                        )
                        override_payload = None
                    else:
                        effective_mime = override_payload.get(
                            "mime_type", "image/jpeg"
                        )
                        overlay_label = override_payload.get("overlay_label") or debug_label

                if not override_payload:
                    ext = Path(image_path).suffix.lower()
                    mime_type = mime_type_map.get(ext)
                    if not mime_type:
                        guessed_type, _ = mimetypes.guess_type(image_path)
                        mime_type = guessed_type or "image/jpeg"
                    if not mime_type.startswith("image/"):
                        logger.debug(
                            "Unsupported mime type %s for %s, defaulting to image/jpeg",
                            mime_type,
                            image_path,
                        )
                        mime_type = "image/jpeg"

                    try:
                        base64_image, effective_mime = self._prepare_encoded_image(
                            image_path, mime_type, debug_label
                        )
                        overlay_label = debug_label
                    except BestShotPickerError as overlay_error:
                        logger.debug(
                            "Failed to prepare debug overlay for %s: %s. "
                            "Falling back to raw encoding without overlay.",
                            image_path,
                            overlay_error,
                        )
                        base64_image = self._get_base64_image(image_path)
                        effective_mime = mime_type
                        overlay_label = None

            except FileNotFoundError:
                logger.warning(f"Image not found: {image_path}, skipping")
                skipped_paths.append(image_path)
                continue
            except BestShotPickerError as e:
                logger.warning(f"Failed to encode image {image_path}: {e}, skipping")
                skipped_paths.append(image_path)
                continue

            prepared_images.append(
                {
                    "original_index": original_index,
                    "path": image_path,
                    "base64": base64_image,
                    "mime_type": effective_mime,
                    "overlay_label": overlay_label,
                }
            )

        if not prepared_images:
            logger.error("No valid images available for analysis after preprocessing")
            raise BestShotPickerError("No valid images to analyze")

        if skipped_paths:
            logger.info(
                "Skipping %d invalid image(s) before analysis", len(skipped_paths)
            )

        logger.info("Image order being sent to AI:")
        for position, item in enumerate(prepared_images, 1):
            logger.info("  Position %d: %s", position, Path(item["path"]).name)

        self._initialize_client()

        content = [
            {
                "type": "text",
                "text": self._build_prompt(
                    len(prepared_images), self.debug_overlay_enabled
                ),
            }
        ]

        for position, item in enumerate(prepared_images, 1):
            image_name = Path(item["path"]).name
            logger.debug(
                "Adding Image %d: %s (original index %d)",
                position,
                image_name,
                item["original_index"],
            )
            encoded_data = item.pop("base64")
            description_lines = [f"\n**Image {position}** ({image_name}):"]
            if self.debug_overlay_enabled and item.get("overlay_label"):
                description_lines.append(
                    f"Overlay number visible on this image: **{item['overlay_label']}**"
                )

            content.append(
                {
                    "type": "text",
                    "text": "\n".join(description_lines),
                }
            )
            content.append(
                {
                    "type": "image_url",
                    "image_url": {
                        "url": f"data:{item['mime_type']};base64,{encoded_data}"
                    },
                }
            )

        # Make API call
        try:
            logger.debug(f"Sending request to {self.base_url}")
            completion = self.client.chat.completions.create(
                model=self.model,
                messages=[{"role": "user", "content": content}],
                max_tokens=max_tokens,
                stream=stream,
            )

            response_text = completion.choices[0].message.content
            logger.debug(f"Received response: {response_text[:200]}...")

            # Parse and return result
            analysis_paths = [item["path"] for item in prepared_images]
            result = self._parse_response(response_text, analysis_paths)

            chosen_entry = prepared_images[result.best_image_index]
            result.best_image_index = chosen_entry["original_index"]
            result.best_image_path = image_paths[result.best_image_index]

            logger.info(
                "Selected image %d/%d: %s",
                result.best_image_index + 1,
                len(image_paths),
                Path(result.best_image_path).name,
            )

            return result

        except Exception as e:
            logger.error(f"API call failed: {e}")
            raise BestShotPickerError(f"Failed to analyze images: {e}")

    def test_connection(self) -> bool:
        """
        Test if the API endpoint is accessible and responding.

        Returns:
            True if connection is successful, False otherwise
        """
        try:
            self._initialize_client()
            # Simple test with a basic text-only message
            response = self.client.chat.completions.create(
                model=self.model,
                messages=[
                    {
                        "role": "user",
                        "content": [{"type": "text", "text": "Hello, respond with OK"}],
                    }
                ],
                max_tokens=10,
            )
            logger.debug(f"Connection test successful: {response.choices[0].message.content if response.choices else 'No response'}")
            return True
        except Exception as e:
            logger.error(f"Connection test failed: {e}")
            return False

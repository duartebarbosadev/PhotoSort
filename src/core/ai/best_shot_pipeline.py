from __future__ import annotations

import base64
import io
import json
import logging
import os
import re
import threading
from dataclasses import dataclass
from enum import Enum
from typing import Dict, List, Optional, Sequence, Tuple, Set

from PIL import Image, ImageDraw, ImageFont

from core.ai.best_photo_selector import BestPhotoSelector
from core.app_settings import (
    get_best_shot_engine,
    get_openai_config,
    DEFAULT_OPENAI_MODEL,
    DEFAULT_OPENAI_BASE_URL,
    DEFAULT_OPENAI_MAX_TOKENS,
    DEFAULT_OPENAI_TIMEOUT,
    DEFAULT_OPENAI_MAX_WORKERS,
)

logger = logging.getLogger(__name__)


class BestShotEngine(str, Enum):
    LOCAL = "local"
    LLM = "llm"


DEFAULT_BEST_SHOT_PROMPT = (
    "You are an expert photography critic tasked with selecting the best image from a set of {image_count} images.\n\n"
    "Analyze each image based on the following criteria:\n"
    "- Sharpness and Focus\n- Color/Lighting\n- Composition\n- Subject Expression\n- Technical Quality\n- Overall Appeal\n- Editing Potential\n- Subject Sharpness\n\n"
    "If any person’s eyes are closed, the photo automatically receives a low rating (1–2).\n\n"
    "Please analyze each image and then provide your response in the following format:\n\n"
    "Best Image: [Image number, 1-{image_count}]\n"
    "Confidence: [High/Medium/Low]\n"
    "Reasoning: [Brief explanation]\n\n"
    "Be decisive and pick ONE image as the best, even if the differences are subtle."
)

DEFAULT_RATING_PROMPT = (
    "You are judging whether a single photo is usable and share-worthy. Rate it from 1 (bad) to 5 (excellent) using these simple rules:\n\n"
    "1 – Unusable: heavy blur or missed focus, subject blinking or eyes closed, severe exposure or color problems.\n"
    "2 – Weak: noticeable softness, awkward expression, distracting clutter, flat lighting. Only use if nothing better exists.\n"
    "3 – OK: sharp enough and exposed correctly but ordinary. Minor distractions or stiff posing are acceptable.\n"
    "4 – Strong: crisp focus, pleasing expression, good composition, looks ready for social media.\n"
    "5 – Outstanding: clean, vibrant, and well-composed. Eye-catching and instagrammable. Portraits with great expression score high.\n\n"
    "Always drop the score if the subject is blinking, eyes are closed, or motion blur makes the shot unusable.\n"
    "Return exactly: {\"rating\": <integer 1-5>, \"reason\": \"<concise justification>\"}"
)


@dataclass
class LLMConfig:
    api_key: Optional[str]
    model: str = DEFAULT_OPENAI_MODEL
    base_url: Optional[str] = DEFAULT_OPENAI_BASE_URL
    max_tokens: int = DEFAULT_OPENAI_MAX_TOKENS
    timeout: int = DEFAULT_OPENAI_TIMEOUT
    best_shot_prompt: Optional[str] = None
    rating_prompt: Optional[str] = None
    max_workers: int = DEFAULT_OPENAI_MAX_WORKERS

    def __post_init__(self) -> None:
        if not self.best_shot_prompt:
            self.best_shot_prompt = DEFAULT_BEST_SHOT_PROMPT
        if not self.rating_prompt:
            self.rating_prompt = DEFAULT_RATING_PROMPT


def _load_font(image_size: Tuple[int, int]) -> ImageFont.ImageFont:
    longer_side = max(image_size)
    font_size = max(24, int(longer_side * 0.08))
    try:
        return ImageFont.truetype("DejaVuSans-Bold.ttf", font_size)
    except Exception:
        return ImageFont.load_default()


def _annotate_image(image: Image.Image, label: str) -> Image.Image:
    annotated = image.copy()
    draw = ImageDraw.Draw(annotated)
    font = _load_font(annotated.size)
    text_bbox = draw.textbbox((0, 0), label, font=font)
    text_width = text_bbox[2] - text_bbox[0]
    text_height = text_bbox[3] - text_bbox[1]
    padding = max(10, text_height // 2)
    position = (padding, padding)
    background_box = (
        position[0] - padding // 2,
        position[1] - padding // 2,
        position[0] + text_width + padding // 2,
        position[1] + text_height + padding // 2,
    )
    draw.rectangle(background_box, fill=(0, 0, 0, 180))
    draw.text(position, label, font=font, fill=(255, 255, 255))
    return annotated


def _image_to_base64(image: Image.Image) -> str:
    buf = io.BytesIO()
    image.save(buf, format="PNG")
    return base64.b64encode(buf.getvalue()).decode("utf-8")


class BaseBestShotStrategy:
    def __init__(
        self,
        models_root: Optional[str],
        image_pipeline,
        llm_config: Optional[LLMConfig] = None,
    ) -> None:
        self.models_root = models_root
        self.image_pipeline = image_pipeline
        self.llm_config = llm_config

    @property
    def max_workers(self) -> int:
        return 4

    def rank_cluster(self, cluster_id: int, image_paths: Sequence[str]) -> List[Dict[str, object]]:
        raise NotImplementedError

    def rate_image(self, image_path: str) -> Optional[Dict[str, object]]:
        raise NotImplementedError

    def shutdown(self) -> None:
        """Clean up resources once processing is done."""

    def validate_connection(self) -> None:
        """Optional connectivity check before work begins."""


class LocalBestShotStrategy(BaseBestShotStrategy):
    def __init__(self, models_root, image_pipeline, llm_config=None) -> None:
        super().__init__(models_root, image_pipeline, llm_config)
        self._thread_local = threading.local()

    def _get_selector(self) -> BestPhotoSelector:
        selector = getattr(self._thread_local, "selector", None)
        if selector is None:
            # Use image pipeline for better RAW and format support
            image_loader = self._create_image_loader() if self.image_pipeline else None
            selector = BestPhotoSelector(
                models_root=self.models_root,
                image_loader=image_loader
            )
            self._thread_local.selector = selector
        return selector

    def _create_image_loader(self):
        """Create an image loader that uses the image pipeline for RAW and format support."""
        def pipeline_image_loader(image_path: str) -> Image.Image:
            try:
                # Use image pipeline to get preview (handles RAW files properly)
                preview = self.image_pipeline.get_preview_image(image_path)
                if preview is not None:
                    if preview.mode != "RGB":
                        preview = preview.convert("RGB")
                    # Ensure required metadata is set
                    preview.info.setdefault("source_path", image_path)
                    preview.info.setdefault("region", "full")
                    return preview
            except Exception as exc:
                logger.warning("Image pipeline failed for %s: %s", image_path, exc)
            
            # Fallback to direct loading for standard formats only
            ext = os.path.splitext(image_path)[1].lower()
            if ext in {'.jpg', '.jpeg', '.png', '.bmp', '.gif', '.tiff', '.tif', '.webp'}:
                try:
                    from PIL import ImageOps
                    with Image.open(image_path) as img:
                        prepared = ImageOps.exif_transpose(img).convert("RGB")
                        prepared.info["source_path"] = image_path
                        prepared.info["region"] = "full"
                        return prepared.copy()
                except Exception as exc:
                    logger.error("Failed to load standard format image %s: %s", image_path, exc)
            else:
                logger.error("Unsupported format for local AI analysis: %s (%s)", ext, image_path)
            
            raise RuntimeError(f"Cannot load image for local AI analysis: {image_path}")
        
        return pipeline_image_loader

    def rank_cluster(self, cluster_id: int, image_paths: Sequence[str]) -> List[Dict[str, object]]:
        logger.info(f"Local AI ranking cluster {cluster_id} with {len(image_paths)} images using local models")
        selector = self._get_selector()
        results = selector.rank_images(image_paths)
        ranked_results = [r.to_dict() for r in results]
        if ranked_results:
            logger.info(f"Completed local AI ranking for cluster {cluster_id}. Best image: {os.path.basename(ranked_results[0]['image_path'])}")
        return ranked_results

    def rate_image(self, image_path: str) -> Optional[Dict[str, object]]:
        logger.info(f"Local AI rating image: {os.path.basename(image_path)}")
        selector = self._get_selector()
        results = selector.rank_images([image_path])
        if not results:
            return None
        result = results[0]
        score = result.composite_score
        rating = max(1, min(5, int(round(score * 4 + 1))))
        logger.info(
            f"Local AI rated {os.path.basename(image_path)} as {rating}/5 (score: {score:.3f})"
        )
        return {
            "image_path": image_path,
            "rating": rating,
            "score": score,
            "metrics": result.metrics,
        }


class LLMBestShotStrategy(BaseBestShotStrategy):
    def __init__(self, models_root, image_pipeline, llm_config: LLMConfig) -> None:
        super().__init__(models_root, image_pipeline, llm_config)
        try:
            from openai import OpenAI  # type: ignore
        except ImportError as exc:  # pragma: no cover
            raise RuntimeError(
                "openai package not installed. Install it to use LLM best-shot engine."
            ) from exc

        if not llm_config.api_key:
            raise RuntimeError(
                "OpenAI API key missing. Set PHOTOSORT_OPENAI_API_KEY or configure via settings."
            )

        self._timeout = llm_config.timeout
        self._base_url = llm_config.base_url or DEFAULT_OPENAI_BASE_URL
        self._client = OpenAI(
            api_key=llm_config.api_key,
            base_url=self._base_url,
            timeout=self._timeout,
        )
        self._model = llm_config.model
        self._max_tokens = llm_config.max_tokens
        self._prompt_template = llm_config.best_shot_prompt
        self._rating_prompt = llm_config.rating_prompt
        self._lock = threading.Lock()
        self._worker_count = llm_config.max_workers

    @property
    def max_workers(self) -> int:
        return max(1, self._worker_count)

    def _with_timeout(self, timeout_seconds: int):
        client = self._client
        if hasattr(client, "with_options"):
            try:
                return client.with_options(timeout=timeout_seconds)
            except Exception:
                return client
        return client

    def _load_preview(self, image_path: str) -> Image.Image:
        """Load image as RGB preview, ensuring compatibility with AI services.
        
        Always uses the image pipeline to handle RAW files and other formats properly,
        as AI services typically don't support RAW formats natively.
        """
        preview = None
        if self.image_pipeline is not None:
            try:
                preview = self.image_pipeline.get_preview_image(image_path)
                if preview is not None and preview.mode != "RGB":
                    preview = preview.convert("RGB")
            except Exception:
                logger.exception("Preview generation failed for %s", image_path)
        
        if preview is None:
            try:
                # Fallback for standard formats only - avoid RAW files
                ext = os.path.splitext(image_path)[1].lower()
                if ext in {'.jpg', '.jpeg', '.png', '.bmp', '.gif', '.tiff', '.tif', '.webp'}:
                    preview = Image.open(image_path).convert("RGB")
                else:
                    raise RuntimeError(f"Unsupported format for AI analysis: {ext}. Preview generation required.")
            except Exception as exc:
                logger.error("Failed to load image %s: %s", image_path, exc)
                raise RuntimeError(f"Cannot load image for AI analysis: {exc}") from exc
        
        return preview

    def _build_messages(self, prompt: str, labelled_images: List[Tuple[int, str]]) -> List[Dict[str, object]]:
        content: List[Dict[str, object]] = [
            {"type": "text", "text": prompt}
        ]
        for index, b64 in labelled_images:
            content.append(
                {
                    "type": "image_url",
                    "image_url": {
                        "url": f"data:image/png;base64,{b64}",
                        "detail": "high",
                    },
                }
            )
        return [{"role": "user", "content": content}]

    def _call_llm(self, messages: List[Dict[str, object]]) -> str:
        with self._lock:
            try:
                response = self._client.chat.completions.create(
                    model=self._model,
                    messages=messages,
                    max_tokens=self._max_tokens,
                )
            except Exception as exc:
                raise RuntimeError(
                    f"LLM request failed for model '{self._model}' at {self._base_url}: {exc}"
                ) from exc
        return response.choices[0].message.content or ""

    def _extract_rating(self, analysis: str) -> Optional[int]:
        if not analysis:
            return None

        # Try JSON parsing first, as the prompt requests structured output
        try:
            parsed = json.loads(analysis)
            if isinstance(parsed, dict) and "rating" in parsed:
                return int(round(float(parsed["rating"])))
        except (ValueError, TypeError, json.JSONDecodeError):
            pass

        patterns = [
            r"\brating\b[^0-9]*([1-5](?:\.[0-9]+)?)",
            r"\boverall rating\b[^0-9]*([1-5](?:\.[0-9]+)?)",
            r"\bscore\b[^0-9]*([1-5](?:\.[0-9]+)?)",
            r"([1-5])\s*/\s*5",
            r"([1-5])\s*out of\s*5",
            r"([1-5])\s*stars",
        ]
        for pattern in patterns:
            match = re.search(pattern, analysis, re.IGNORECASE)
            if match:
                try:
                    return int(round(float(match.group(1))))
                except (ValueError, TypeError):
                    continue

        return None

    def validate_connection(self) -> None:
        probe_timeout = min(max(5, int(self._timeout * 0.25)), max(self._timeout, 5))
        client = self._with_timeout(probe_timeout)
        try:
            response = client.models.list()
        except Exception as exc:
            raise RuntimeError(
                f"Unable to reach LLM endpoint at {self._base_url}: {exc}"
            ) from exc
        data = getattr(response, "data", None)
        model_ids: Set[str] = set()
        if data:
            for entry in data:
                if isinstance(entry, dict):
                    identifier = entry.get("id") or entry.get("name")
                else:
                    identifier = getattr(entry, "id", None) or getattr(
                        entry, "name", None
                    )
                if identifier:
                    model_ids.add(str(identifier))

        if not model_ids:
            raise RuntimeError(
                "LLM endpoint responded but returned zero models; ensure your server exposes an active model."
            )
        if self._model not in model_ids:
            raise RuntimeError(
                f"LLM endpoint reachable, but model '{self._model}' not found. Available models: {', '.join(sorted(model_ids))}."
            )

    def rank_cluster(self, cluster_id: int, image_paths: Sequence[str]) -> List[Dict[str, object]]:
        logger.info(f"AI ranking cluster {cluster_id} with {len(image_paths)} images using LLM strategy")
        
        images = []
        labelled_payloads: List[Tuple[int, str]] = []
        for idx, path in enumerate(image_paths, start=1):
            preview = self._load_preview(path)
            annotated = _annotate_image(preview, str(idx))
            labelled_payloads.append((idx, _image_to_base64(annotated)))
            images.append((idx, path))

        prompt = self._prompt_template.format(image_count=len(image_paths))
        messages = self._build_messages(prompt, labelled_payloads)
        
        logger.info(f"Sending {len(image_paths)} images to LLM for analysis")
        analysis = self._call_llm(messages)
        logger.info(f"Received LLM analysis response (length: {len(analysis)} chars)")

        best_match = re.search(r"Best Image\s*:\s*\[?\s*(\d+)", analysis, re.IGNORECASE)
        best_index = None
        if best_match:
            try:
                candidate = int(best_match.group(1))
                if 1 <= candidate <= len(image_paths):
                    best_index = candidate
                    logger.info(f"LLM selected image {best_index} as best from {len(image_paths)} options")
            except ValueError:
                best_index = None
        
        if best_index is None:
            logger.warning(f"Could not parse best image selection from LLM response: {analysis[:200]}...")

        ranked: List[Dict[str, object]] = []
        for idx, path in images:
            score = 1.0 if idx == best_index else 0.5
            ranked.append(
                {
                    "image_path": path,
                    "composite_score": score,
                    "metrics": {"llm_selected": idx == best_index},
                    "analysis": analysis,
                }
            )

        if best_index is not None:
            ranked.sort(key=lambda item: item["metrics"]["llm_selected"], reverse=True)
            logger.info(f"Completed AI ranking for cluster {cluster_id}. Best image: {os.path.basename(ranked[0]['image_path'])}")
        else:
            logger.warning(f"Completed AI ranking for cluster {cluster_id} but no clear winner identified")
        return ranked

    def rate_image(self, image_path: str) -> Optional[Dict[str, object]]:
        logger.info(f"AI rating image: {os.path.basename(image_path)}")
        
        preview = self._load_preview(image_path)
        annotated = _annotate_image(preview, "1")
        b64 = _image_to_base64(annotated)
        prompt = self._rating_prompt
        messages = self._build_messages(prompt, [(1, b64)])
        
        logger.debug(f"Sending image to LLM for rating analysis")
        analysis = self._call_llm(messages)
        
        rating = self._extract_rating(analysis)
        if rating is not None:
            rating = max(1, min(5, rating))
            logger.info(
                f"AI rated {os.path.basename(image_path)} as {rating}/5"
            )
        else:
            snippet = (analysis or "").strip()[:200]
            logger.warning(
                "AI rating missing or invalid for %s; response sample: %s",
                os.path.basename(image_path),
                snippet or "<empty response>",
            )
        return {
            "image_path": image_path,
            "rating": rating,
            "analysis": analysis,
        }


def create_best_shot_strategy(
    engine: Optional[str] = None,
    *,
    models_root: Optional[str] = None,
    image_pipeline=None,
    llm_config: Optional[LLMConfig] = None,
) -> BaseBestShotStrategy:
    """Create AI strategy for image analysis.
    
    Both LLM and Local strategies now properly support RAW images by using
    the image_pipeline to generate RGB previews suitable for AI analysis.
    """
    engine_name = (engine or get_best_shot_engine() or "local").lower()
    logger.info(f"Creating AI strategy with engine: {engine_name}")
    if engine_name == BestShotEngine.LLM.value:
        config = llm_config or LLMConfig(**get_openai_config())
        logger.info(f"Using LLM strategy with endpoint: {config.base_url}")
        return LLMBestShotStrategy(models_root, image_pipeline, config)
    logger.info("Using local model strategy")
    return LocalBestShotStrategy(models_root, image_pipeline, llm_config)


__all__ = [
    "BestShotEngine",
    "LLMBestShotStrategy",
    "LocalBestShotStrategy",
    "create_best_shot_strategy",
    "LLMConfig",
]

import base64
import io
import json
import logging
import os
import threading
from dataclasses import dataclass
from typing import Any

from PIL import Image

from core.app_settings import (
    get_openai_config,
    DEFAULT_OPENAI_API_KEY,
    DEFAULT_OPENAI_MODEL,
    DEFAULT_OPENAI_BASE_URL,
    DEFAULT_OPENAI_MAX_TOKENS,
    DEFAULT_OPENAI_TIMEOUT,
    DEFAULT_OPENAI_MAX_WORKERS,
)

logger = logging.getLogger(__name__)


DEFAULT_RATING_PROMPT = (
    "Quantitatively evaluate the photograph by inspecting the high-frequency detail (micro-contrast), subject facial cues, noise distribution, tonal balance, color fidelity, compositional geometry, and lighting directionality.\n"
    "Assign each of the following metrics a score from 0–100 (integers) where 50 represents acceptable quality for professional sharing:\n"
    "- sharpness: edge acuity and micro-contrast on the subject's eyes and key textures\n"
    "- noise_control: luminance/chroma noise in mid-tones and shadows (higher = cleaner)\n"
    "- exposure_balance: dynamic range handling, highlight retention, and shadow lift\n"
    "- color_accuracy: white balance correctness and skin tone realism\n"
    "- composition_balance: adherence to composition rules (framing, leading lines, clutter control)\n"
    "- subject_expression: clarity of subject intent (eyes open, engaging expression, lack of motion blur)\n\n"
    "Compute an overall_quality score as the weighted average of the metrics with weights:\n"
    "sharpness 0.25, noise_control 0.15, exposure_balance 0.15, color_accuracy 0.15, composition_balance 0.15, subject_expression 0.15.\n"
    "Map overall_quality to a 1–5 star rating using these deterministic thresholds (include the boundary in the higher rating):\n"
    "1 star <= 40 < 2 star, 2 star <= 55 < 3 star, 3 star <= 70 < 4 star, 4 star <= 85 < 5 star, 5 star >= 85.\n"
    "The same image must always produce the same rating when scored with this rubric.\n"
    "Provide one concise sentence noting the dominant strengths and the limiting flaw(s)."
)


@dataclass(slots=True)
class LLMConfig:
    api_key: str | None
    model: str = DEFAULT_OPENAI_MODEL
    base_url: str | None = DEFAULT_OPENAI_BASE_URL
    max_tokens: int = DEFAULT_OPENAI_MAX_TOKENS
    timeout: int = DEFAULT_OPENAI_TIMEOUT
    rating_prompt: str | None = None
    max_workers: int = DEFAULT_OPENAI_MAX_WORKERS

    def __post_init__(self) -> None:
        if not self.rating_prompt:
            self.rating_prompt = DEFAULT_RATING_PROMPT


def _image_to_base64(image: Image.Image) -> str:
    buf = io.BytesIO()
    image.save(buf, format="PNG")
    return base64.b64encode(buf.getvalue()).decode("utf-8")


class BaseAiRatingStrategy:
    def __init__(
        self,
        image_pipeline,
        llm_config: LLMConfig | None = None,
    ) -> None:
        self.image_pipeline = image_pipeline
        self.llm_config = llm_config

    @property
    def max_workers(self) -> int:
        return 4

    def rate_image(self, image_path: str) -> dict[str, object] | None:
        raise NotImplementedError

    def shutdown(self) -> None:
        """Clean up resources once processing is done."""

    def validate_connection(self) -> None:
        """Optional connectivity check before work begins."""

    def request_cancel(self) -> None:
        """Signal any in-flight operations to halt as soon as possible."""


class LLMAiRatingStrategy(BaseAiRatingStrategy):
    def __init__(self, image_pipeline, llm_config: LLMConfig) -> None:
        super().__init__(image_pipeline, llm_config)
        try:
            from openai import OpenAI  # type: ignore
        except ImportError as exc:  # pragma: no cover
            raise RuntimeError(
                "openai package not installed. Install it to use AI rating."
            ) from exc

        self._timeout = llm_config.timeout
        self._base_url = llm_config.base_url or DEFAULT_OPENAI_BASE_URL
        client_kwargs: dict[str, object] = {
            "base_url": self._base_url,
            "timeout": self._timeout,
        }
        if llm_config.api_key and llm_config.api_key != DEFAULT_OPENAI_API_KEY:
            client_kwargs["api_key"] = llm_config.api_key
        self._client = OpenAI(**client_kwargs)
        self._model = llm_config.model
        self._max_tokens = llm_config.max_tokens
        self._rating_prompt = llm_config.rating_prompt
        self._lock = threading.Lock()
        self._worker_count = llm_config.max_workers
        self._cancel_event = threading.Event()
        self._client_closed = False

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

    def _close_client(self) -> None:
        with self._lock:
            try:
                if hasattr(self._client, "close") and not self._client_closed:
                    self._client.close()
                    self._client_closed = True
            except Exception:
                logger.debug("Failed to close LLM client", exc_info=True)

    def request_cancel(self) -> None:
        self._cancel_event.set()
        self._close_client()

    def shutdown(self) -> None:
        self._close_client()

    def _load_preview(self, image_path: str) -> Image.Image:
        """Load image as RGB preview, ensuring compatibility with AI services.

        Always uses the image pipeline to handle RAW files and other formats properly,
        as AI services typically don't support RAW formats natively.
        """
        if self.image_pipeline is None:
            raise RuntimeError("AI analysis requires the shared image pipeline.")

        from core.image_pipeline import ANALYSIS_CACHE_RESOLUTION

        preview = self.image_pipeline.get_analysis_image(
            image_path,
            target_size=ANALYSIS_CACHE_RESOLUTION,
        )
        if preview is None:
            raise RuntimeError(f"Cannot generate an AI preview for {image_path}.")
        if preview.mode != "RGB":
            preview = preview.convert("RGB")

        return preview

    def _build_messages(
        self,
        prompt: str,
        labelled_images: list[tuple[int, str]],
        *,
        system_prompt: str | None = None,
    ) -> list[dict[str, object]]:
        content: list[dict[str, object]] = [{"type": "text", "text": prompt}]
        for _index, b64 in labelled_images:
            content.append(
                {
                    "type": "image_url",
                    "image_url": {
                        "url": f"data:image/png;base64,{b64}",
                        "detail": "high",
                    },
                }
            )
        messages: list[dict[str, object]] = []
        if system_prompt:
            messages.append({"role": "system", "content": system_prompt})
        messages.append({"role": "user", "content": content})
        return messages

    def _call_llm(
        self,
        messages: list[dict[str, object]],
        *,
        tools: list[dict[str, object]] | None = None,
        tool_choice: str | None = None,
        max_tokens: int | None = None,
    ):
        if self._cancel_event.is_set():
            raise RuntimeError("LLM request cancelled")
        with self._lock:
            try:
                if self._cancel_event.is_set():
                    raise RuntimeError("LLM request cancelled")
                kwargs: dict[str, object] = {
                    "model": self._model,
                    "messages": messages,
                    "max_tokens": max(max_tokens or self._max_tokens, 256),
                    "temperature": 0.3,
                }
                if tools is not None:
                    kwargs["tools"] = tools
                if tool_choice is not None:
                    kwargs["tool_choice"] = tool_choice
                response = self._client.chat.completions.create(**kwargs)
            except Exception as exc:
                if self._cancel_event.is_set():
                    raise RuntimeError("LLM request cancelled") from exc
                raise RuntimeError(
                    f"LLM request failed for model '{self._model}' at {self._base_url}: {exc}"
                ) from exc
        message = response.choices[0].message
        content = getattr(message, "content", None) or ""
        return message, content

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
        model_ids: set[str] = set()
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

    def rate_image(self, image_path: str) -> dict[str, object] | None:
        logger.info(f"AI rating image: {os.path.basename(image_path)}")

        preview = self._load_preview(image_path)
        b64 = _image_to_base64(preview)
        prompt = self._rating_prompt
        system_prompt = (
            "You are a photography scientist performing repeatable image quality audits. "
            "Use the provided evaluation rubric and respond only by calling the provided tool."
        )
        messages = self._build_messages(
            prompt,
            [(1, b64)],
            system_prompt=system_prompt,
        )
        tools = [
            {
                "type": "function",
                "function": {
                    "name": "record_photo_quality",
                    "description": "Store deterministic quality scores for a single photograph.",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "overall_rating": {
                                "type": "integer",
                                "minimum": 1,
                                "maximum": 5,
                                "description": "Overall star rating derived from the weighted quality score (1-5).",
                            },
                            "overall_quality": {
                                "type": "number",
                                "minimum": 0,
                                "maximum": 100,
                                "description": "Weighted quantitative quality score (0-100).",
                            },
                            "confidence": {
                                "type": "string",
                                "enum": ["low", "medium", "high"],
                                "description": "Confidence in the rating after evaluating visual evidence.",
                            },
                            "score_breakdown": {
                                "type": "object",
                                "properties": {
                                    "sharpness": {
                                        "type": "integer",
                                        "minimum": 0,
                                        "maximum": 100,
                                    },
                                    "noise_control": {
                                        "type": "integer",
                                        "minimum": 0,
                                        "maximum": 100,
                                    },
                                    "exposure_balance": {
                                        "type": "integer",
                                        "minimum": 0,
                                        "maximum": 100,
                                    },
                                    "color_accuracy": {
                                        "type": "integer",
                                        "minimum": 0,
                                        "maximum": 100,
                                    },
                                    "composition_balance": {
                                        "type": "integer",
                                        "minimum": 0,
                                        "maximum": 100,
                                    },
                                    "subject_expression": {
                                        "type": "integer",
                                        "minimum": 0,
                                        "maximum": 100,
                                    },
                                },
                                "required": [
                                    "sharpness",
                                    "noise_control",
                                    "exposure_balance",
                                    "color_accuracy",
                                    "composition_balance",
                                    "subject_expression",
                                ],
                            },
                            "notes": {
                                "type": "string",
                                "description": "One concise sentence summarising the key strengths and weaknesses.",
                            },
                        },
                        "required": [
                            "overall_rating",
                            "overall_quality",
                            "confidence",
                            "score_breakdown",
                            "notes",
                        ],
                    },
                },
            }
        ]
        tool_choice = "required"

        logger.debug("Sending image to LLM for rating analysis")
        message, freeform_analysis = self._call_llm(
            messages,
            tools=tools,
            tool_choice=tool_choice,
        )
        analysis = freeform_analysis
        structured_payload: dict[str, Any] = {}
        tool_calls = getattr(message, "tool_calls", None) or []
        if tool_calls:
            try:
                raw_args = tool_calls[0].function.arguments  # type: ignore[attr-defined]
                structured_payload = json.loads(raw_args) if raw_args else {}
            except Exception:
                logger.exception("Failed to parse AI rating tool output")
        else:
            raise RuntimeError(
                "AI rating response did not include the required tool call."
            )

        rating = structured_payload.get("overall_rating")
        if rating is not None:
            rating = max(1, min(5, rating))
            logger.info(f"AI rated {os.path.basename(image_path)} as {rating}/5")
        else:
            snippet = (analysis or "").strip()[:200]
            logger.warning(
                "AI rating missing or invalid for %s; response sample: %s",
                os.path.basename(image_path),
                snippet or "<empty response>",
            )
        if structured_payload and not analysis:
            breakdown = structured_payload.get("score_breakdown", {})
            breakdown_parts = [
                f"{name.replace('_', ' ')} {value}" for name, value in breakdown.items()
            ]
            notes = structured_payload.get("notes")
            confidence = structured_payload.get("confidence")
            summary_bits = []
            if breakdown_parts:
                summary_bits.append(" | ".join(breakdown_parts))
            if notes:
                summary_bits.append(notes)
            if confidence:
                summary_bits.append(f"confidence: {confidence}")
            analysis = " ".join(summary_bits)

        payload = {
            "image_path": image_path,
            "rating": rating,
            "analysis": analysis,
        }
        if structured_payload:
            payload["quality_scores"] = structured_payload
        return payload


def create_ai_rating_strategy(
    *,
    image_pipeline=None,
    llm_config: LLMConfig | None = None,
) -> BaseAiRatingStrategy:
    """Create the configured LLM strategy for per-image ratings."""
    config = llm_config or LLMConfig(**get_openai_config())
    logger.info(f"Creating AI strategy with LLM endpoint: {config.base_url}")
    return LLMAiRatingStrategy(image_pipeline, config)


__all__ = [
    "LLMConfig",
    "BaseAiRatingStrategy",
    "LLMAiRatingStrategy",
    "create_ai_rating_strategy",
]

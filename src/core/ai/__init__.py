"""
AI helper utilities for advanced ranking/scoring pipelines.

This package exposes the LLM-based best-shot pipeline used for ranking clusters
and assigning AI star ratings.
"""

from .best_shot_pipeline import BaseBestShotStrategy, LLMBestShotStrategy, LLMConfig

__all__ = [
    "BaseBestShotStrategy",
    "LLMBestShotStrategy",
    "LLMConfig",
]

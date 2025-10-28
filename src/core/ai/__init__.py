"""
AI Module for PhotoSort

This module contains AI-powered features for PhotoSort, including:
- Best shot picker: Automatically select the best image from a group

The AI features use vision language models through an OpenAI-compatible API,
with default support for LM Studio local server.
"""

from .best_shot_picker import BestShotPicker, BestShotResult, BestShotPickerError

__all__ = ["BestShotPicker", "BestShotResult", "BestShotPickerError"]

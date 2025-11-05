"""
AI helper utilities for advanced ranking/scoring pipelines.

Currently exposes the experimental best-photo selector which chains together
multiple pre-trained models (face detection, eye-state classification, and
image quality scoring) to rank similar shots.
"""

from .best_photo_selector import BestPhotoSelector, BestShotResult  # noqa: F401

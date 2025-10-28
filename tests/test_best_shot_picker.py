"""
Tests for AI Best Shot Picker functionality
"""

import os
import pytest
from unittest.mock import Mock, patch, MagicMock
from pathlib import Path

from core.ai.best_shot_picker import (
    BestShotPicker,
    BestShotResult,
    BestShotPickerError,
)


@pytest.fixture
def sample_images(tmp_path):
    """Create some sample image files for testing."""
    images = []
    for i in range(3):
        img_path = tmp_path / f"test_image_{i}.jpg"
        img_path.write_bytes(b"fake image data")
        images.append(str(img_path))
    return images


@pytest.fixture
def mock_openai_client():
    """Mock OpenAI client for testing."""
    with patch("core.ai.best_shot_picker.OpenAI") as mock_client_class:
        mock_client = MagicMock()
        mock_client_class.return_value = mock_client

        # Mock successful completion
        mock_completion = MagicMock()
        mock_completion.choices = [MagicMock()]
        mock_completion.choices[0].message.content = """
**Best Image**: Image 2
**Confidence**: High
**Reasoning**: This image has the best focus and composition.
"""
        mock_client.chat.completions.create.return_value = mock_completion

        yield mock_client


class TestBestShotPicker:
    """Test suite for BestShotPicker class."""

    def test_initialization_default_params(self):
        """Test initialization with default parameters."""
        picker = BestShotPicker()
        assert picker.base_url == "http://localhost:1234/v1"
        assert picker.api_key == "not-needed"
        assert picker.model == "local-model"
        assert picker.timeout == 120

    def test_initialization_custom_params(self):
        """Test initialization with custom parameters."""
        picker = BestShotPicker(
            base_url="http://custom:8080/v1",
            api_key="custom-key",
            model="custom-model",
            timeout=60,
        )
        assert picker.base_url == "http://custom:8080/v1"
        assert picker.api_key == "custom-key"
        assert picker.model == "custom-model"
        assert picker.timeout == 60

    def test_get_base64_image_success(self, sample_images):
        """Test successful Base64 encoding of an image."""
        picker = BestShotPicker()
        base64_str = picker._get_base64_image(sample_images[0])
        assert isinstance(base64_str, str)
        assert len(base64_str) > 0

    def test_get_base64_image_file_not_found(self):
        """Test Base64 encoding with non-existent file."""
        picker = BestShotPicker()
        with pytest.raises(FileNotFoundError):
            picker._get_base64_image("/nonexistent/file.jpg")

    def test_build_prompt(self):
        """Test prompt building."""
        picker = BestShotPicker()
        prompt = picker._build_prompt(3)
        assert "3 images" in prompt
        assert "Sharpness and Focus" in prompt
        assert "Best Image" in prompt

    def test_parse_response_standard_format(self, sample_images):
        """Test parsing a standard formatted response."""
        picker = BestShotPicker()
        response = """
**Best Image**: Image 2
**Confidence**: High
**Reasoning**: This image has excellent focus and proper exposure.
"""
        result = picker._parse_response(response, sample_images)
        assert result.best_image_index == 1  # 0-based index
        assert result.best_image_path == sample_images[1]
        assert result.confidence == "High"
        assert "excellent focus" in result.reasoning

    def test_parse_response_alternative_formats(self, sample_images):
        """Test parsing various response formats."""
        picker = BestShotPicker()

        # Test format without markdown
        response1 = "Best Image: 1\nThis is the best one."
        result1 = picker._parse_response(response1, sample_images)
        assert result1.best_image_index == 0

        # Test format with "Image X is best"
        response2 = "After analysis, Image 3 is best because of composition."
        result2 = picker._parse_response(response2, sample_images)
        assert result2.best_image_index == 2

    def test_parse_response_invalid_defaults_to_first(self, sample_images):
        """Test that unparseable response defaults to first image."""
        picker = BestShotPicker()
        response = "This response has no clear image selection."
        result = picker._parse_response(response, sample_images)
        assert result.best_image_index == 0
        assert "Failed to parse" in result.reasoning

    def test_select_best_image_single_image(self, sample_images):
        """Test selecting best from a single image."""
        picker = BestShotPicker()
        result = picker.select_best_image([sample_images[0]])
        assert result.best_image_index == 0
        assert result.best_image_path == sample_images[0]
        assert "Only one image" in result.reasoning

    def test_select_best_image_empty_list(self):
        """Test error when no images provided."""
        picker = BestShotPicker()
        with pytest.raises(ValueError, match="No images provided"):
            picker.select_best_image([])

    def test_select_best_image_success(self, sample_images, mock_openai_client):
        """Test successful image selection."""
        picker = BestShotPicker()
        result = picker.select_best_image(sample_images)

        assert result.best_image_index == 1  # Response says "Image 2"
        assert result.best_image_path == sample_images[1]
        assert result.confidence == "High"
        assert "best focus" in result.reasoning

        # Verify API was called
        mock_openai_client.chat.completions.create.assert_called_once()

    def test_select_best_image_api_error(self, sample_images):
        """Test handling of API errors."""
        with patch("core.ai.best_shot_picker.OpenAI") as mock_client_class:
            mock_client = MagicMock()
            mock_client_class.return_value = mock_client
            mock_client.chat.completions.create.side_effect = Exception("API Error")

            picker = BestShotPicker()
            with pytest.raises(BestShotPickerError, match="Failed to analyze images"):
                picker.select_best_image(sample_images)

    def test_select_best_image_skips_missing_files(
        self, sample_images, mock_openai_client
    ):
        """Test that missing files are skipped during analysis."""
        # Add a non-existent file to the list
        images_with_missing = sample_images + ["/nonexistent/image.jpg"]

        picker = BestShotPicker()
        result = picker.select_best_image(images_with_missing)

        # Should still work and return a valid result
        assert result.best_image_index in [0, 1, 2]

    def test_test_connection_success(self):
        """Test successful connection test."""
        with patch("core.ai.best_shot_picker.OpenAI") as mock_client_class:
            mock_client = MagicMock()
            mock_client_class.return_value = mock_client

            mock_completion = MagicMock()
            mock_completion.choices = [MagicMock()]
            mock_client.chat.completions.create.return_value = mock_completion

            picker = BestShotPicker()
            assert picker.test_connection() is True

    def test_test_connection_failure(self):
        """Test failed connection test."""
        with patch("core.ai.best_shot_picker.OpenAI") as mock_client_class:
            mock_client = MagicMock()
            mock_client_class.return_value = mock_client
            mock_client.chat.completions.create.side_effect = Exception(
                "Connection failed"
            )

            picker = BestShotPicker()
            assert picker.test_connection() is False


class TestBestShotResult:
    """Test suite for BestShotResult dataclass."""

    def test_result_creation(self):
        """Test creating a BestShotResult."""
        result = BestShotResult(
            best_image_index=2,
            best_image_path="/path/to/image.jpg",
            reasoning="Great composition",
            confidence="High",
            raw_response="Full response text",
        )

        assert result.best_image_index == 2
        assert result.best_image_path == "/path/to/image.jpg"
        assert result.reasoning == "Great composition"
        assert result.confidence == "High"
        assert result.raw_response == "Full response text"


def test_best_shot_picker_integration(sample_images, mock_openai_client):
    """Integration test for the full best shot picking workflow."""
    picker = BestShotPicker(
        base_url="http://localhost:1234/v1",
        api_key="test-key",
        model="test-model",
        timeout=60,
    )

    # Test connection first
    assert picker.test_connection() is True

    # Select best image
    result = picker.select_best_image(sample_images)

    # Verify result
    assert isinstance(result, BestShotResult)
    assert 0 <= result.best_image_index < len(sample_images)
    assert result.best_image_path in sample_images
    assert len(result.reasoning) > 0
    assert result.confidence in ["High", "Medium", "Low", "Not specified"]


if __name__ == "__main__":
    pytest.main([__file__, "-v"])

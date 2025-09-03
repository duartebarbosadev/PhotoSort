#!/usr/bin/env python3
"""
Simple test for the release notes generator script.
"""

import os
import sys
import unittest
from unittest.mock import patch

# Add scripts directory to path
sys.path.append(os.path.join(os.path.dirname(__file__), "..", "scripts"))

from generate_release_notes import ReleaseNotesGenerator


class TestReleaseNotesGenerator(unittest.TestCase):
    """Test the release notes generator functionality."""

    def test_fallback_notes_creation(self):
        """Test that fallback notes are created when OpenAI fails."""
        generator = ReleaseNotesGenerator("dummy-api-key")

        # Test with some commit messages
        diff_content = "abc123 Add new feature\ndef456 Fix bug in image processing"
        result = generator._create_fallback_notes(diff_content, "v1.0.0")

        self.assertIn("Release v1.0.0", result)
        self.assertIn("abc123 Add new feature", result)
        self.assertIn("def456 Fix bug in image processing", result)
        self.assertIn("automatically generated", result)

    def test_fallback_notes_empty_diff(self):
        """Test fallback notes with empty diff content."""
        generator = ReleaseNotesGenerator("dummy-api-key")

        result = generator._create_fallback_notes("", "v1.0.0")

        self.assertIn("Release v1.0.0", result)
        self.assertIn("could not be generated", result)

    @patch("subprocess.run")
    def test_get_previous_tag(self, mock_run):
        """Test getting the previous tag."""
        generator = ReleaseNotesGenerator("dummy-api-key")

        # Mock git tag output
        mock_run.return_value.stdout = "v1.0.1\nv1.0.0\nv0.9.0\n"
        mock_run.return_value.returncode = 0

        result = generator.get_previous_tag("v1.0.1")
        self.assertEqual(result, "v1.0.0")

        result = generator.get_previous_tag("v1.0.0")
        self.assertEqual(result, "v0.9.0")

    @patch("subprocess.run")
    def test_get_git_diff(self, mock_run):
        """Test getting git diff between tags."""
        generator = ReleaseNotesGenerator("dummy-api-key")

        # Mock git log output
        mock_run.return_value.stdout = "abc123 Add feature\ndef456 Fix bug\n"
        mock_run.return_value.returncode = 0

        result = generator.get_git_diff("v1.0.0", "v1.0.1")

        self.assertIn("abc123 Add feature", result)
        self.assertIn("def456 Fix bug", result)


if __name__ == "__main__":
    unittest.main()

#!/usr/bin/env python3
"""
Simple test for the release notes generator script.
"""

import os
import sys
import unittest
from unittest.mock import patch

# Add scripts directory to path
sys.path.append(os.path.join(os.path.dirname(__file__), "..", ".github", "scripts"))

from generate_release_notes import ReleaseNotesGenerator


class TestReleaseNotesGenerator(unittest.TestCase):
    """Test the release notes generator functionality."""

    def test_fallback_notes_creation(self):
        """Test that fallback notes are created when OpenAI fails."""
        generator = ReleaseNotesGenerator("dummy-api-key")

        # Test with some commit data
        commits = [
            {
                "hash": "abc123456789",
                "subject": "Add new feature",
                "body": "",
                "pr_number": "123",
                "pr_title": "Add new feature",
            },
            {
                "hash": "def456789012",
                "subject": "Fix bug in image processing",
                "body": "",
                "pr_number": None,
                "pr_title": None,
            },
        ]
        result = generator._create_fallback_notes_from_commits(commits, "v1.0.0")

        self.assertIn("Release v1.0.0", result)
        self.assertIn("Add new feature", result)
        self.assertIn("Fix bug in image processing", result)
        self.assertIn("PR #123", result)
        self.assertIn("automatically generated", result)

    def test_fallback_notes_empty_commits(self):
        """Test fallback notes with empty commit list."""
        generator = ReleaseNotesGenerator("dummy-api-key")

        result = generator._create_fallback_notes_from_commits([], "v1.0.0")

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

    def test_extract_pr_info(self):
        """Test extracting PR information from commit messages."""
        generator = ReleaseNotesGenerator("dummy-api-key")

        # Test merge commit format
        subject = "Merge pull request #123 from feature/new-ui"
        body = "Add new user interface components\n\nImprove user experience"
        pr_info = generator._extract_pr_info(subject, body)
        self.assertEqual(pr_info["number"], "123")
        self.assertEqual(pr_info["title"], "Add new user interface components")

        # Test PR number in parentheses
        subject = "Add new feature (#456)"
        body = ""
        pr_info = generator._extract_pr_info(subject, body)
        self.assertEqual(pr_info["number"], "456")
        self.assertEqual(pr_info["title"], "Add new feature")

        # Test no PR information
        subject = "Fix typo in documentation"
        body = ""
        pr_info = generator._extract_pr_info(subject, body)
        self.assertIsNone(pr_info["number"])
        self.assertIsNone(pr_info["title"])

    @patch("subprocess.run")
    def test_get_commits_with_pr_info(self, mock_run):
        """Test getting commits with PR information."""
        generator = ReleaseNotesGenerator("dummy-api-key")

        # Mock git log output with our separator format
        mock_output = """abc123456789|Add new feature (#123)|Implement new UI components===COMMIT_SEPARATOR===def456789012|Fix bug in processing|Fix issue with image processing===COMMIT_SEPARATOR===ghi789012345|Merge pull request #456 from fix/memory-leak|Fix memory leak in image loader

Fixed memory leak that occurred during batch processing===COMMIT_SEPARATOR==="""

        mock_run.return_value.stdout = mock_output
        mock_run.return_value.returncode = 0

        commits = generator.get_commits_with_pr_info("v1.0.0", "v1.0.1")

        self.assertEqual(len(commits), 3)

        # Check first commit
        self.assertEqual(commits[0]["hash"], "abc123456789")
        self.assertEqual(commits[0]["subject"], "Add new feature (#123)")
        self.assertEqual(commits[0]["pr_number"], "123")
        self.assertEqual(commits[0]["pr_title"], "Add new feature")

        # Check second commit (no PR)
        self.assertEqual(commits[1]["hash"], "def456789012")
        self.assertEqual(commits[1]["subject"], "Fix bug in processing")
        self.assertIsNone(commits[1]["pr_number"])

        # Check third commit (merge PR)
        self.assertEqual(commits[2]["hash"], "ghi789012345")
        self.assertEqual(commits[2]["pr_number"], "456")
        self.assertEqual(commits[2]["pr_title"], "Fix memory leak in image loader")


if __name__ == "__main__":
    unittest.main()

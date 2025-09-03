#!/usr/bin/env python3
"""
Tests for the release notes generator script (updated for new implementation).
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

    def setUp(self):
        # Use a dummy key; in unit tests we won't actually call the API.
        self.generator = ReleaseNotesGenerator("dummy-api-key")

    # -------------------------
    # Fallback notes generation
    # -------------------------
    def test_fallback_notes_creation(self):
        """Test that fallback notes are created when OpenAI fails."""
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
        result = self.generator._create_fallback_notes_from_commits(commits, "v1.0.0")

        self.assertIn("Release v1.0.0", result)
        self.assertIn("Add new feature", result)
        self.assertIn("Fix bug in image processing", result)
        self.assertIn("PR #123", result)
        self.assertIn("automatically generated", result)

    def test_fallback_notes_empty_commits(self):
        """Test fallback notes with empty commit list."""
        result = self.generator._create_fallback_notes_from_commits([], "v1.0.0")

        self.assertIn("Release v1.0.0", result)
        self.assertIn("could not be generated", result)

    # -------------------------
    # Git metadata helpers
    # -------------------------
    @patch("subprocess.run")
    def test_get_previous_tag(self, mock_run):
        """Test getting the previous tag."""
        # Mock git tag output (sorted by creatordate desc)
        mock_run.return_value.stdout = "v1.0.1\nv1.0.0\nv0.9.0\n"
        mock_run.return_value.returncode = 0

        self.assertEqual(self.generator.get_previous_tag("v1.0.1"), "v1.0.0")
        self.assertEqual(self.generator.get_previous_tag("v1.0.0"), "v0.9.0")

    # NOTE: get_git_diff() no longer exists in the implementation.
    # We remove the old test_get_git_diff and rely on get_commits_with_pr_info() instead.

    # -------------------------
    # Commit/PR parsing
    # -------------------------
    def test_extract_pr_info(self):
        """Test extracting PR information from commit messages."""
        # Merge commit format
        subject = "Merge pull request #123 from feature/new-ui"
        body = "Add new user interface components\n\nImprove user experience"
        pr_info = self.generator._extract_pr_info(subject, body)
        self.assertEqual(pr_info["number"], "123")
        self.assertEqual(pr_info["title"], "Add new user interface components")

        # PR number in parentheses
        subject = "Add new feature (#456)"
        body = ""
        pr_info = self.generator._extract_pr_info(subject, body)
        self.assertEqual(pr_info["number"], "456")
        self.assertEqual(pr_info["title"], "Add new feature")

        # No PR info
        subject = "Fix typo in documentation"
        body = ""
        pr_info = self.generator._extract_pr_info(subject, body)
        self.assertIsNone(pr_info["number"])
        self.assertIsNone(pr_info["title"])

    @patch("subprocess.run")
    def test_get_commits_with_pr_info(self, mock_run):
        """Test getting commits with PR information."""
        mock_output = """abc123456789|Add new feature (#123)|Implement new UI components===COMMIT_SEPARATOR===
def456789012|Fix bug in processing|Fix issue with image processing===COMMIT_SEPARATOR===
ghi789012345|Merge pull request #456 from fix/memory-leak|Fix memory leak in image loader

Fixed memory leak that occurred during batch processing===COMMIT_SEPARATOR==="""
        mock_run.return_value.stdout = mock_output
        mock_run.return_value.returncode = 0

        commits = self.generator.get_commits_with_pr_info("v1.0.0", "v1.0.1")

        self.assertEqual(len(commits), 3)

        # First commit
        self.assertEqual(commits[0]["hash"], "abc123456789")
        self.assertEqual(commits[0]["subject"], "Add new feature (#123)")
        self.assertEqual(commits[0]["pr_number"], "123")
        self.assertEqual(commits[0]["pr_title"], "Add new feature")

        # Second commit (no PR)
        self.assertEqual(commits[1]["hash"], "def456789012")
        self.assertEqual(commits[1]["subject"], "Fix bug in processing")
        self.assertIsNone(commits[1]["pr_number"])

        # Third commit (merge PR)
        self.assertEqual(commits[2]["hash"], "ghi789012345")
        self.assertEqual(commits[2]["pr_number"], "456")
        self.assertEqual(commits[2]["pr_title"], "Fix memory leak in image loader")

    # -------------------------
    # Sanitizer behavior (new)
    # -------------------------
    def test_sanitize_notes_removes_hash_before_sha(self):
        """Ensure '#<sha>' becomes '<sha>' (no change to '#123')."""
        txt = "Fixes in commit #f525c01 and references #29 for the PR."
        sanitized = self.generator._sanitize_notes(txt, repo_url=None)
        self.assertIn("f525c01", sanitized)  # sha without '#'
        self.assertNotIn("#f525c01", sanitized)  # removed '#'
        self.assertIn("#29", sanitized)  # PR reference remains

    def test_sanitize_notes_links_shas_when_repo_known(self):
        """Bare SHAs are auto-linked to /commit/<sha> when repo_url is provided."""
        txt = "Changes introduced in f525c01 and 1a2b3c4d."
        repo = "https://github.com/owner/repo"
        sanitized = self.generator._sanitize_notes(txt, repo_url=repo)
        self.assertIn("[f525c01](" + repo + "/commit/f525c01)", sanitized)
        self.assertIn("[1a2b3c4d](" + repo + "/commit/1a2b3c4d)", sanitized)

    def test_sanitize_notes_does_not_double_link(self):
        """SHAs already in a /commit/ URL or inside a markdown link should not be re-linked."""
        repo = "https://github.com/duartebarbosadev/PhotoSort"
        txt = (
            f"Already linked: [f525c01]({repo}/commit/f525c01). "
            f"Also as raw URL: {repo}/commit/1a2b3c4d."
        )
        sanitized = self.generator._sanitize_notes(txt, repo_url=repo)
        # Should remain exactly as-is (no duplicate links)
        self.assertIn(f"[f525c01]({repo}/commit/f525c01)", sanitized)
        self.assertIn(f"{repo}/commit/1a2b3c4d", sanitized)
        # No extra '[' around the second SHA
        self.assertNotIn("[1a2b3c4d](", sanitized)


if __name__ == "__main__":
    unittest.main()

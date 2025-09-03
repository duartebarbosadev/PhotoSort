#!/usr/bin/env python3
"""
Script to generate release notes using OpenAI API based on git differences.
This script is designed to be run in CI when a new release is published.
"""

import os
import sys
import subprocess
import argparse
import re
from typing import Optional, List, Dict, Any
import logging

# Configure logging
logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)

try:
    from openai import OpenAI
except ImportError:
    logger.error("OpenAI library not installed. Install with: pip install openai")
    sys.exit(1)


class ReleaseNotesGenerator:
    """Generates release notes using OpenAI API based on git differences."""

    def __init__(self, api_key: str):
        self.client = OpenAI(api_key=api_key)

    def get_previous_tag(self, current_tag: str) -> Optional[str]:
        """Get the previous release tag before the current one."""
        try:
            # Get all tags sorted by creation date
            result = subprocess.run(
                ["git", "tag", "--sort=-creatordate"],
                capture_output=True,
                text=True,
                check=True,
            )

            tags = [tag.strip() for tag in result.stdout.split("\n") if tag.strip()]

            # Find current tag and return the next one
            try:
                current_index = tags.index(current_tag)
                if current_index + 1 < len(tags):
                    return tags[current_index + 1]
                else:
                    logger.info(
                        "No previous tag found, this might be the first release"
                    )
                    return None
            except ValueError:
                logger.error(f"Current tag {current_tag} not found in git tags")
                return None

        except subprocess.CalledProcessError as e:
            logger.error(f"Failed to get git tags: {e}")
            return None

    def get_git_diff(self, from_tag: Optional[str], to_tag: str) -> str:
        """Get git diff between two tags or from beginning if from_tag is None."""
        try:
            if from_tag:
                # Get commits with more detailed information including PR references
                result = subprocess.run(
                    ["git", "log", "--oneline", "--no-merges", f"{from_tag}..{to_tag}"],
                    capture_output=True,
                    text=True,
                    check=True,
                )

                if not result.stdout.strip():
                    # Fallback to show diff with file changes
                    result = subprocess.run(
                        ["git", "diff", "--name-status", f"{from_tag}..{to_tag}"],
                        capture_output=True,
                        text=True,
                        check=True,
                    )

                    if result.stdout.strip():
                        diff_output = f"Files changed:\n{result.stdout}\n\nCommits:\n"
                        commits_result = subprocess.run(
                            [
                                "git",
                                "log",
                                "--oneline",
                                "--no-merges",
                                f"{from_tag}..{to_tag}",
                            ],
                            capture_output=True,
                            text=True,
                        )
                        diff_output += commits_result.stdout
                        return diff_output
            else:
                # First release - get all commits
                result = subprocess.run(
                    ["git", "log", "--oneline", "--no-merges", to_tag],
                    capture_output=True,
                    text=True,
                    check=True,
                )

            return result.stdout

        except subprocess.CalledProcessError as e:
            logger.error(f"Failed to get git diff: {e}")
            return ""

    def get_commits_with_pr_info(
        self, from_tag: Optional[str], to_tag: str
    ) -> List[Dict[str, Any]]:
        """Get commits with PR information extracted from commit messages."""
        try:
            # Use a custom separator to handle multiline commit bodies
            separator = "===COMMIT_SEPARATOR==="

            if from_tag:
                # Get full commit messages between tags
                result = subprocess.run(
                    [
                        "git",
                        "log",
                        f"--pretty=format:%H|%s|%b{separator}",
                        "--no-merges",
                        f"{from_tag}..{to_tag}",
                    ],
                    capture_output=True,
                    text=True,
                    check=True,
                )
            else:
                # First release - get all commits
                result = subprocess.run(
                    [
                        "git",
                        "log",
                        f"--pretty=format:%H|%s|%b{separator}",
                        "--no-merges",
                        to_tag,
                    ],
                    capture_output=True,
                    text=True,
                    check=True,
                )

            commits = []
            if result.stdout.strip():
                # Split by our custom separator
                commit_blocks = result.stdout.strip().split(separator)

                for block in commit_blocks:
                    block = block.strip()
                    if not block:
                        continue

                    # Find the first line which contains hash|subject|body
                    lines = block.split("\n")
                    if lines and "|" in lines[0]:
                        parts = lines[0].split("|", 2)
                        if len(parts) >= 2:
                            commit_hash = parts[0]
                            subject = parts[1]
                            body = parts[2] if len(parts) > 2 else ""

                            # Add any additional body lines (shouldn't be any with this format)
                            if len(lines) > 1:
                                additional_lines = [
                                    line for line in lines[1:] if line.strip()
                                ]
                                if additional_lines:
                                    if body:
                                        body += "\n" + "\n".join(additional_lines)
                                    else:
                                        body = "\n".join(additional_lines)

                            # Extract PR information
                            pr_info = self._extract_pr_info(subject, body)

                            commits.append(
                                {
                                    "hash": commit_hash,
                                    "subject": subject,
                                    "body": body,
                                    "pr_number": pr_info.get("number"),
                                    "pr_title": pr_info.get("title"),
                                }
                            )

            return commits

        except subprocess.CalledProcessError as e:
            logger.error(f"Failed to get commits with PR info: {e}")
            return []

    def _extract_pr_info(self, subject: str, body: str) -> Dict[str, Optional[str]]:
        """Extract PR number and title from commit messages."""
        pr_info = {"number": None, "title": None}

        # Common patterns for PR references in commit messages
        patterns = [
            r"Merge pull request #(\d+) from .+",  # Standard GitHub merge message
            r".*\(#(\d+)\).*",  # PR number in parentheses
            r".*#(\d+).*",  # Simple PR number reference
        ]

        # Check subject first
        for pattern in patterns:
            match = re.search(pattern, subject, re.IGNORECASE)
            if match:
                pr_info["number"] = match.group(1)
                # For merge commits, extract title
                if "Merge pull request" in subject:
                    # Title might be in body or we can clean up the subject
                    lines = body.strip().split("\n")
                    if lines and lines[0].strip():
                        pr_info["title"] = lines[0].strip()
                else:
                    # Use subject as title, removing PR reference
                    clean_title = re.sub(r"\s*\(#\d+\)\s*$", "", subject)
                    pr_info["title"] = clean_title
                break

        # If not found in subject, check body
        if not pr_info["number"]:
            for pattern in patterns:
                match = re.search(pattern, body, re.IGNORECASE)
                if match:
                    pr_info["number"] = match.group(1)
                    break

        return pr_info

    def generate_release_notes(
        self, commits: List[Dict[str, Any]], tag_name: str
    ) -> str:
        """Generate release notes using OpenAI API with commit and PR information."""
        if not commits:
            return f"## Release {tag_name}\n\nNo significant changes found in this release."

        # Format commit information for the AI prompt
        commit_info = []
        pr_links = []

        for commit in commits:
            commit_line = f"- {commit['subject']} ({commit['hash'][:7]})"
            if commit["pr_number"]:
                commit_line += f" [PR #{commit['pr_number']}]"
                if commit["pr_title"] and commit["pr_title"] != commit["subject"]:
                    commit_line += f": {commit['pr_title']}"
                pr_links.append(
                    f"- PR #{commit['pr_number']}: {commit['pr_title'] or commit['subject']}"
                )
            commit_info.append(commit_line)

        commits_text = "\n".join(commit_info)
        pr_section = ""
        if pr_links:
            pr_section = "\n\nRelated Pull Requests:\n" + "\n".join(pr_links)

        prompt = f"""
You are a technical writer creating release notes for a desktop photo management application called PhotoSort.

Based on the following git changes for release {tag_name}, write comprehensive release notes in markdown format.

Commits in this release:
```
{commits_text}
```
{pr_section}

Please create release notes that:
1. Start with a brief summary of the release
2. Group changes into categories like:
   - 🚀 New Features
   - 🐛 Bug Fixes  
   - 🔧 Improvements
   - ⚠️ Breaking Changes (if any)
   - 📚 Documentation
   - 🏗️ Technical Changes
3. Use clear, user-friendly language
4. Focus on user-visible changes
5. Include PR references where available (e.g., "Enhanced UI (#123)")
6. Keep technical details concise but informative

Format the response as clean markdown without code blocks around the entire response.
"""

        try:
            response = self.client.chat.completions.create(
                model="gpt-4o-mini",  # Using cost-effective model
                messages=[
                    {
                        "role": "system",
                        "content": "You are a skilled technical writer who creates clear, informative release notes.",
                    },
                    {"role": "user", "content": prompt},
                ],
                max_tokens=2000,
                temperature=0.3,  # Lower temperature for more consistent output
            )

            return response.choices[0].message.content.strip()

        except Exception as e:
            logger.error(f"Failed to generate release notes with OpenAI: {e}")
            # Fallback to basic release notes
            return self._create_fallback_notes_from_commits(commits, tag_name)

    def _create_fallback_notes_from_commits(
        self, commits: List[Dict[str, Any]], tag_name: str
    ) -> str:
        """Create basic release notes from commit data if OpenAI fails."""
        if not commits:
            return f"## Release {tag_name}\n\nRelease notes could not be generated automatically."

        changes = []
        pr_section = []

        for commit in commits:
            change_line = f"- {commit['subject']}"
            if commit["pr_number"]:
                change_line += f" (PR #{commit['pr_number']})"
                pr_section.append(
                    f"- PR #{commit['pr_number']}: {commit['pr_title'] or commit['subject']}"
                )
            changes.append(change_line)

        result = f"""## Release {tag_name}

### Changes in this release:

{chr(10).join(changes)}"""

        if pr_section:
            result += f"""

### Related Pull Requests:

{chr(10).join(pr_section)}"""

        result += "\n\n*Note: Release notes were automatically generated from commit messages.*"
        return result


def main():
    parser = argparse.ArgumentParser(description="Generate release notes using OpenAI")
    parser.add_argument("tag_name", help="Current release tag name")
    parser.add_argument(
        "--api-key", help="OpenAI API key (or use OPENAI_API_KEY env var)"
    )
    parser.add_argument(
        "--output", help="Output file for release notes (default: stdout)"
    )

    args = parser.parse_args()

    # Get API key from argument or environment
    api_key = args.api_key or os.environ.get("OPENAI_API_KEY")
    if not api_key:
        logger.error(
            "OpenAI API key required. Use --api-key or set OPENAI_API_KEY environment variable."
        )
        sys.exit(1)

    try:
        generator = ReleaseNotesGenerator(api_key)

        # Get previous tag
        previous_tag = generator.get_previous_tag(args.tag_name)
        logger.info(
            f"Generating release notes for {args.tag_name} (previous: {previous_tag})"
        )

        # Get commits with PR information
        commits = generator.get_commits_with_pr_info(previous_tag, args.tag_name)
        if not commits:
            logger.warning("No commits found between releases")

        # Generate release notes
        release_notes = generator.generate_release_notes(commits, args.tag_name)

        # Output results
        if args.output:
            with open(args.output, "w") as f:
                f.write(release_notes)
            logger.info(f"Release notes written to {args.output}")
        else:
            print(release_notes)

    except Exception as e:
        logger.error(f"Failed to generate release notes: {e}")
        sys.exit(1)


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""
Script to generate release notes using OpenAI API based on git differences.
This script is designed to be run in CI when a new release is published.
"""

import os
import sys
import subprocess
import argparse
from typing import Optional
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
                # Get diff between two tags
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

    def generate_release_notes(self, diff_content: str, tag_name: str) -> str:
        """Generate release notes using OpenAI API."""
        if not diff_content.strip():
            return f"## Release {tag_name}\n\nNo significant changes found in this release."

        prompt = f"""
You are a technical writer creating release notes for a desktop photo management application called PhotoSort.

Based on the following git changes for release {tag_name}, write comprehensive release notes in markdown format.

Git changes:
```
{diff_content}
```

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
5. Keep technical details concise but informative

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
            return self._create_fallback_notes(diff_content, tag_name)

    def _create_fallback_notes(self, diff_content: str, tag_name: str) -> str:
        """Create basic release notes if OpenAI fails."""
        lines = diff_content.strip().split("\n")
        changes = []

        for line in lines:
            if line.strip():
                changes.append(f"- {line.strip()}")

        if not changes:
            return f"## Release {tag_name}\n\nRelease notes could not be generated automatically."

        return f"""## Release {tag_name}

### Changes in this release:

{chr(10).join(changes)}

*Note: Release notes were automatically generated from commit messages.*
"""


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

        # Get git diff
        diff_content = generator.get_git_diff(previous_tag, args.tag_name)
        if not diff_content.strip():
            logger.warning("No differences found between releases")

        # Generate release notes
        release_notes = generator.generate_release_notes(diff_content, args.tag_name)

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

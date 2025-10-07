#!/usr/bin/env python3
"""
Script to generate release notes using OpenAI API based on git differences.
Designed to be run in CI when a new release is published.
"""

import os
import sys
import subprocess
import argparse
import re
import logging
from typing import Optional, List, Dict, Any

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

    def __init__(self, api_key: str, model: Optional[str] = None):
        # The OpenAI client reads org/project from env if present
        self.client = OpenAI(api_key=api_key)
        # Allow override via env OPENAI_MODEL; default to a cost-effective model
        self.model = model or os.getenv("OPENAI_MODEL", "gpt-4o-mini")

    @staticmethod
    def _sanitize_notes(text: str, repo_url: Optional[str]) -> str:
        """
        Fix '#<sha>' (e.g., '#f525c01') -> '<sha>' and (optionally) auto-link SHAs.
        Keeps '#123' (issues/PRs) intact.

        - Step 1: remove a stray '#' that precedes a hex-looking SHA (7-40 chars)
        - Step 2: if repo_url is provided, wrap bare SHAs with a commit link
        """
        if not text:
            return text

        # 1) Drop stray '#' before hex SHAs (7-40 chars)
        text = re.sub(r"#([a-f0-9]{7,40})\b", r"\1", text, flags=re.IGNORECASE)

        # 2) Optionally link bare SHAs to commits, avoiding ones already in links/URLs
        if repo_url:

            def _link_sha(m):
                sha = m.group(0)
                return f"[{sha}]({repo_url}/commit/{sha})"

            # Avoid matching SHAs that already appear in a '/commit/<sha>' URL or a markdown link target
            text = re.sub(
                r"(?<!\]\()(?<!/commit/)\b([a-f0-9]{7,40})\b",
                _link_sha,
                text,
                flags=re.IGNORECASE,
            )

        return text

    def get_previous_tag(self, current_tag: str) -> Optional[str]:
        """Get the previous release tag before the current one."""
        try:
            # Get all tags sorted by creation date (newest first)
            result = subprocess.run(
                ["git", "tag", "--sort=-creatordate"],
                capture_output=True,
                text=True,
                check=True,
            )
            tags = [tag.strip() for tag in result.stdout.split("\n") if tag.strip()]

            # Find current tag and return the next one (older tag)
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

    def get_commits_with_pr_info(
        self, from_tag: Optional[str], to_tag: str
    ) -> List[Dict[str, Any]]:
        """Get commits with PR information extracted from commit messages."""
        try:
            separator = "===COMMIT_SEPARATOR==="

            if from_tag:
                cmd = [
                    "git",
                    "log",
                    f"--pretty=format:%H|%s|%b{separator}",
                    "--no-merges",
                    f"{from_tag}..{to_tag}",
                ]
            else:
                # First release - get all commits reachable by the tag
                cmd = [
                    "git",
                    "log",
                    f"--pretty=format:%H|%s|%b{separator}",
                    "--no-merges",
                    to_tag,
                ]

            result = subprocess.run(cmd, capture_output=True, text=True, check=True)

            commits: List[Dict[str, Any]] = []
            if result.stdout.strip():
                commit_blocks = result.stdout.strip().split(separator)

                for block in commit_blocks:
                    block = block.strip()
                    if not block:
                        continue

                    lines = block.split("\n")
                    if lines and "|" in lines[0]:
                        parts = lines[0].split("|", 2)
                        if len(parts) >= 2:
                            commit_hash = parts[0]
                            subject = parts[1]
                            body = parts[2] if len(parts) > 2 else ""

                            # Pull any additional lines if present
                            if len(lines) > 1:
                                additional_lines = [
                                    line for line in lines[1:] if line.strip()
                                ]
                                if additional_lines:
                                    body = (body + "\n" if body else "") + "\n".join(
                                        additional_lines
                                    )

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
        pr_info: Dict[str, Optional[str]] = {"number": None, "title": None}

        patterns = [
            r"Merge pull request #(\d+) from .+",
            r".*\(#(\d+)\).*",
            r".*#(\d+).*",
        ]

        # Check subject first
        for pattern in patterns:
            match = re.search(pattern, subject, re.IGNORECASE)
            if match:
                pr_info["number"] = match.group(1)
                if "Merge pull request" in subject:
                    lines = body.strip().split("\n")
                    if lines and lines[0].strip():
                        pr_info["title"] = lines[0].strip()
                else:
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
        commit_info: List[str] = []
        pr_links: List[str] = []

        for commit in commits:
            line = f"- {commit['subject']} ({commit['hash'][:7]})"
            if commit["pr_number"]:
                line += f" [PR #{commit['pr_number']}]"
                if commit["pr_title"] and commit["pr_title"] != commit["subject"]:
                    line += f": {commit['pr_title']}"
                pr_links.append(
                    f"- PR #{commit['pr_number']}: {commit['pr_title'] or commit['subject']}"
                )
            commit_info.append(line)

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

Formatting rules:
- Use '#123' only for issues/PRs.
- Never prefix commit SHAs with '#'. Write them as short SHAs (e.g., f525c01).

Please create release notes that:
1) Start with a brief summary of the release
2) Group changes into categories like:
   - 🚀 New Features
   - 🔧 Improvements
   - 🐛 Bug Fixes
   - ⚠️ Breaking Changes (if any)
   - 📚 Documentation
   - 🏗️ Technical Changes
3. Use clear, user-friendly language
4. Focus on user-visible changes
5. Include PR references where available (e.g., "Enhanced UI (#123)")
6. Keep technical details concise but informative
7. Keep this formal and about the changes, do not talk to the user directly.
Format the response as clean markdown without code blocks around the entire response.
"""

        try:
            response = self.client.chat.completions.create(
                model=self.model,
                messages=[
                    {
                        "role": "system",
                        "content": "You are a skilled technical writer who creates clear, informative release notes.",
                    },
                    {"role": "user", "content": prompt},
                ],
                max_tokens=2000,
                temperature=0.3,
            )
            return response.choices[0].message.content.strip()

        except Exception as e:
            logger.error(f"Failed to generate release notes with OpenAI: {e}")
            return self._create_fallback_notes_from_commits(commits, tag_name)

    def _create_fallback_notes_from_commits(
        self, commits: List[Dict[str, Any]], tag_name: str
    ) -> str:
        """Create basic release notes from commit data if OpenAI fails."""
        if not commits:
            return f"## Release {tag_name}\n\nRelease notes could not be generated automatically."

        changes: List[str] = []
        pr_section: List[str] = []

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
    parser.add_argument(
        "--model", help="OpenAI model (default from OPENAI_MODEL or gpt-4o-mini)"
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
        generator = ReleaseNotesGenerator(api_key, model=args.model)

        # Get previous tag
        previous_tag = generator.get_previous_tag(args.tag_name)
        logger.info(
            f"Generating release notes for {args.tag_name} (previous: {previous_tag})"
        )

        # Collect commits
        commits = generator.get_commits_with_pr_info(previous_tag, args.tag_name)
        if not commits:
            logger.warning("No commits found between releases")

        # Generate notes via OpenAI (with fallback)
        release_notes = generator.generate_release_notes(commits, args.tag_name)

        # Build repo URL from GitHub Actions env to enable SHA links
        repo_url = None
        gh_server = os.environ.get("GITHUB_SERVER_URL")  # e.g., https://github.com
        gh_repo = os.environ.get("GITHUB_REPOSITORY")  # e.g., owner/repo
        if gh_server and gh_repo:
            repo_url = f"{gh_server.rstrip('/')}/{gh_repo}"

        # Sanitize final text (fix '#<sha>' and link SHAs)
        release_notes = generator._sanitize_notes(release_notes, repo_url)

        # Output
        if args.output:
            with open(args.output, "w", encoding="utf-8") as f:
                f.write(release_notes)
            logger.info(f"Release notes written to {args.output}")
        else:
            print(release_notes)

    except Exception as e:
        logger.error(f"Failed to generate release notes: {e}")
        sys.exit(1)


if __name__ == "__main__":
    main()

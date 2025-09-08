"""
Update Checker Module
Handles checking for new application updates from GitHub releases.
"""

import json
import logging
import time
from dataclasses import dataclass
from typing import Optional, Tuple
from urllib.request import urlopen, Request
from urllib.error import URLError, HTTPError

from core.app_settings import (
    UPDATE_CHECK_TIMEOUT_SECONDS,
    GITHUB_REPO_OWNER,
    GITHUB_REPO_NAME,
    get_update_check_enabled,
    get_last_update_check_time,
    set_last_update_check_time,
    UPDATE_CHECK_INTERVAL_HOURS,
)

logger = logging.getLogger(__name__)


@dataclass
class UpdateInfo:
    """Information about an available update."""

    version: str
    release_url: str
    release_notes: str
    published_at: str
    download_url: Optional[str] = None


class UpdateChecker:
    """Handles checking for application updates from GitHub releases."""

    def __init__(self):
        self.github_api_url = f"https://api.github.com/repos/{GITHUB_REPO_OWNER}/{GITHUB_REPO_NAME}/releases/latest"

    def should_check_for_updates(self) -> bool:
        """Check if enough time has passed since the last update check."""
        if not get_update_check_enabled():
            logger.debug("Automatic update checks are disabled")
            return False

        # Skip automatic updates for development versions
        from core.build_info import VERSION

        current_version = VERSION or "dev"
        if current_version == "dev" or current_version.startswith("dev"):
            logger.debug("Skipping automatic update check for development version")
            return False

        last_check = get_last_update_check_time()
        current_time = int(time.time())
        time_since_last_check = current_time - last_check
        check_interval = UPDATE_CHECK_INTERVAL_HOURS * 3600  # Convert to seconds

        should_check = time_since_last_check >= check_interval
        if should_check:
            logger.debug(
                f"Time for update check. Last check: {time_since_last_check // 3600} hours ago"
            )
        else:
            next_check_hours = (check_interval - time_since_last_check) // 3600
            logger.debug(f"Next update check in {next_check_hours} hours")

        return should_check

    def check_for_updates(
        self, current_version: str
    ) -> Tuple[bool, Optional[UpdateInfo], Optional[str]]:
        """
        Check for updates from GitHub releases.

        Args:
            current_version: The current application version

        Returns:
            Tuple of (update_available, update_info, error_message)
        """
        logger.info("Checking for application updates...")

        try:
            # Update the last check time regardless of result
            set_last_update_check_time(int(time.time()))

            # Make request to GitHub API
            request = Request(
                self.github_api_url,
                headers={
                    "User-Agent": f"{GITHUB_REPO_NAME}/{current_version}",
                    "Accept": "application/vnd.github.v3+json",
                },
            )

            with urlopen(request, timeout=UPDATE_CHECK_TIMEOUT_SECONDS) as response:
                if response.status != 200:
                    error_msg = f"GitHub API returned status {response.status}"
                    logger.warning(error_msg)
                    return False, None, error_msg

                data = json.loads(response.read().decode("utf-8"))

            # Parse release information
            latest_version = data.get("tag_name", "").lstrip(
                "v"
            )  # Remove 'v' prefix if present
            release_url = data.get("html_url", "")
            release_notes = data.get("body", "")
            published_at = data.get("published_at", "")

            if not latest_version:
                error_msg = "Could not parse version from GitHub release"
                logger.warning(error_msg)
                return False, None, error_msg

            # Find download URL for current platform
            download_url = self._find_download_url(data.get("assets", []))

            # Compare versions
            if self._is_newer_version(latest_version, current_version):
                logger.info(
                    f"New version available: {latest_version} (current: {current_version})"
                )
                update_info = UpdateInfo(
                    version=latest_version,
                    release_url=release_url,
                    release_notes=release_notes,
                    published_at=published_at,
                    download_url=download_url,
                )
                return True, update_info, None
            else:
                logger.info(
                    f"Application is up to date (current: {current_version}, latest: {latest_version})"
                )
                return False, None, None

        except HTTPError as e:
            error_msg = f"HTTP error checking for updates: {e.code} {e.reason}"
            logger.warning(error_msg)
            return False, None, error_msg
        except URLError as e:
            error_msg = f"Network error checking for updates: {e.reason}"
            logger.warning(error_msg)
            return False, None, error_msg
        except json.JSONDecodeError as e:
            error_msg = f"Error parsing GitHub API response: {e}"
            logger.warning(error_msg)
            return False, None, error_msg
        except Exception as e:
            error_msg = f"Unexpected error checking for updates: {e}"
            logger.error(error_msg, exc_info=True)
            return False, None, error_msg

    def _find_download_url(self, assets: list) -> Optional[str]:
        """Find the appropriate download URL for the current platform."""
        import platform

        system = platform.system().lower()

        for asset in assets:
            name = asset.get("name", "").lower()
            download_url = asset.get("browser_download_url")

            if not download_url:
                continue

            # Match platform-specific assets
            if system == "windows" and "windows" in name and name.endswith(".exe"):
                return download_url
            elif system == "darwin" and "macos" in name and name.endswith(".dmg"):
                return download_url

        return None

    def _is_newer_version(self, latest: str, current: str) -> bool:
        """
        Compare two version strings to determine if latest is newer than current.
        Handles semantic versioning with optional pre-release suffixes (e.g., 1.0.2a).
        """
        if not current:  # Development version
            return True

        try:
            # Parse version components
            latest_parts = self._parse_version(latest)
            current_parts = self._parse_version(current)

            # Compare version components
            return latest_parts > current_parts

        except Exception as e:
            logger.warning(f"Error comparing versions '{latest}' vs '{current}': {e}")
            # If parsing fails, assume update is available to be safe
            return True

    def _parse_version(self, version: str) -> Tuple[int, ...]:
        """
        Parse a version string into comparable components.
        Examples: "1.0.2" -> (1, 0, 2), "1.0.2a" -> (1, 0, 2, -1)
        Release versions are considered newer than pre-release versions of same number.
        """
        # Handle development versions
        if version.startswith("dev-") or not version:
            return (0,)  # Development versions are always older

        # Remove 'v' prefix if present
        version = version.lstrip("v")

        # Split into numeric and suffix parts
        import re

        match = re.match(r"^(\d+(?:\.\d+)*)([a-zA-Z]*)$", version)
        if not match:
            # If we can't parse it, treat as (0,) to be safe
            return (0,)

        numeric_part, suffix = match.groups()

        # Parse numeric components
        parts = [int(x) for x in numeric_part.split(".")]

        # Handle pre-release suffixes (alpha, beta, etc.)
        if suffix:
            # Pre-release versions are considered older than release versions
            # Map common suffixes to negative numbers for proper ordering
            suffix_map = {"a": -3, "alpha": -3, "b": -2, "beta": -2, "rc": -1}
            suffix_value = suffix_map.get(
                suffix.lower(), -10
            )  # Unknown suffixes are very old
            parts.append(suffix_value)
        else:
            # For release versions (no suffix), add 0 to distinguish from pre-release
            # This ensures 1.0.2 > 1.0.2a (where 1.0.2a has -3 as last component)
            parts.append(0)

        return tuple(parts)

import os
import sys
import json
from unittest.mock import Mock, patch
from urllib.error import URLError, HTTPError

# Ensure project root on path (in case tests run differently)
project_root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
if project_root not in sys.path:  # pragma: no cover - defensive
    sys.path.insert(0, project_root)

# Import after path setup (required for test environment)
# ruff: noqa: E402
from src.core.update_checker import UpdateChecker, UpdateInfo


class TestUpdateChecker:
    """Tests the update checking functionality."""

    def test_version_comparison_basic(self):
        """Test basic version comparison logic."""
        checker = UpdateChecker()

        # Basic version updates
        assert checker._is_newer_version("1.0.1", "1.0.0") is True
        assert checker._is_newer_version("1.1.0", "1.0.0") is True
        assert checker._is_newer_version("2.0.0", "1.0.0") is True

        # No updates needed
        assert checker._is_newer_version("1.0.0", "1.0.0") is False
        assert checker._is_newer_version("1.0.0", "1.0.1") is False

        # Development versions
        assert checker._is_newer_version("1.0.0", "dev") is True
        assert checker._is_newer_version("1.0.0", "") is True
        assert checker._is_newer_version("dev-abc123", "1.0.0") is False

    def test_version_comparison_prerelease(self):
        """Test pre-release version comparison."""
        checker = UpdateChecker()

        # Pre-release to release
        assert checker._is_newer_version("1.0.2", "1.0.2a") is True
        assert checker._is_newer_version("1.0.2", "1.0.2b") is True

        # Release to pre-release
        assert checker._is_newer_version("1.0.2a", "1.0.2") is False
        assert checker._is_newer_version("1.0.2b", "1.0.2") is False

        # Pre-release to pre-release
        assert checker._is_newer_version("1.0.2a", "1.0.1a") is True
        assert checker._is_newer_version("1.0.2b", "1.0.2a") is True

    def test_version_parsing(self):
        """Test version string parsing."""
        checker = UpdateChecker()

        assert checker._parse_version("1.0.0") == (1, 0, 0, 0)
        assert checker._parse_version("1.0.2a") == (1, 0, 2, -3)
        assert checker._parse_version("1.2.3") == (1, 2, 3, 0)
        assert checker._parse_version("2.0.0b") == (2, 0, 0, -2)
        assert checker._parse_version("dev-abc123") == (0,)
        assert checker._parse_version("") == (0,)

    def test_should_check_for_updates_disabled(self):
        """Test update check when disabled."""
        checker = UpdateChecker()

        with patch(
            "src.core.update_checker.get_update_check_enabled", return_value=False
        ):
            assert checker.should_check_for_updates() is False

    def test_should_check_for_updates_dev_version(self):
        """Test update check skipped for development versions."""
        checker = UpdateChecker()

        with (
            patch(
                "src.core.update_checker.get_update_check_enabled", return_value=True
            ),
            patch("src.core.build_info.VERSION", "dev"),
        ):
            assert checker.should_check_for_updates() is False

        with (
            patch(
                "src.core.update_checker.get_update_check_enabled", return_value=True
            ),
            patch("src.core.build_info.VERSION", "dev-abc123"),
        ):
            assert checker.should_check_for_updates() is False

    def test_should_check_for_updates_time_based(self):
        """Test update check timing logic for release versions."""
        checker = UpdateChecker()

        # Test with a release version (not dev)
        with (
            patch(
                "src.core.update_checker.get_update_check_enabled", return_value=True
            ),
            patch("src.core.update_checker.get_last_update_check_time", return_value=0),
            patch("src.core.update_checker.time.time", return_value=100000),
            patch("core.build_info.VERSION", "1.0.0"),
        ):
            assert checker.should_check_for_updates() is True

        # Test time not elapsed yet
        with (
            patch(
                "src.core.update_checker.get_update_check_enabled", return_value=True
            ),
            patch(
                "src.core.update_checker.get_last_update_check_time", return_value=99999
            ),
            patch("src.core.update_checker.time.time", return_value=100000),
            patch("core.build_info.VERSION", "1.0.0"),
        ):
            assert checker.should_check_for_updates() is False

    @patch("src.core.update_checker.urlopen")
    def test_check_for_updates_success(self, mock_urlopen):
        """Test successful update check."""
        checker = UpdateChecker()

        # Mock GitHub API response
        mock_response_data = {
            "tag_name": "v1.0.3",
            "html_url": "https://github.com/duartebarbosadev/PhotoSort/releases/tag/v1.0.3",
            "body": "Bug fixes and improvements",
            "published_at": "2025-01-01T00:00:00Z",
            "assets": [
                {
                    "name": "PhotoSort-Windows-x64.exe",
                    "browser_download_url": "https://github.com/duartebarbosadev/PhotoSort/releases/download/v1.0.3/PhotoSort-Windows-x64.exe",
                }
            ],
        }

        mock_response = Mock()
        mock_response.status = 200
        mock_response.read.return_value = json.dumps(mock_response_data).encode("utf-8")
        mock_urlopen.return_value.__enter__.return_value = mock_response

        with patch("src.core.update_checker.set_last_update_check_time"):
            update_available, update_info, error = checker.check_for_updates("1.0.0")

        assert update_available is True
        assert update_info is not None
        assert update_info.version == "1.0.3"
        assert error is None

    @patch("src.core.update_checker.urlopen")
    def test_check_for_updates_no_update(self, mock_urlopen):
        """Test update check when no update is available."""
        checker = UpdateChecker()

        # Mock GitHub API response with older version
        mock_response_data = {
            "tag_name": "v1.0.0",
            "html_url": "https://github.com/duartebarbosadev/PhotoSort/releases/tag/v1.0.0",
            "body": "Initial release",
            "published_at": "2024-01-01T00:00:00Z",
            "assets": [],
        }

        mock_response = Mock()
        mock_response.status = 200
        mock_response.read.return_value = json.dumps(mock_response_data).encode("utf-8")
        mock_urlopen.return_value.__enter__.return_value = mock_response

        with patch("src.core.update_checker.set_last_update_check_time"):
            update_available, update_info, error = checker.check_for_updates("1.0.0")

        assert update_available is False
        assert update_info is None
        assert error is None

    @patch("src.core.update_checker.urlopen")
    def test_check_for_updates_network_error(self, mock_urlopen):
        """Test update check with network error."""
        checker = UpdateChecker()

        mock_urlopen.side_effect = URLError("Network error")

        with patch("src.core.update_checker.set_last_update_check_time"):
            update_available, update_info, error = checker.check_for_updates("1.0.0")

        assert update_available is False
        assert update_info is None
        assert "Network error" in error

    @patch("src.core.update_checker.urlopen")
    def test_check_for_updates_http_error(self, mock_urlopen):
        """Test update check with HTTP error."""
        checker = UpdateChecker()

        mock_urlopen.side_effect = HTTPError(
            "http://example.com", 404, "Not Found", {}, None
        )

        with patch("src.core.update_checker.set_last_update_check_time"):
            update_available, update_info, error = checker.check_for_updates("1.0.0")

        assert update_available is False
        assert update_info is None
        assert "HTTP error" in error

    @patch("platform.system")
    def test_find_download_url_windows(self, mock_platform):
        """Test finding Windows download URL."""
        checker = UpdateChecker()
        mock_platform.return_value = "Windows"

        assets = [
            {
                "name": "PhotoSort-Windows-x64.exe",
                "browser_download_url": "https://example.com/windows.exe",
            },
            {
                "name": "PhotoSort-macOS-Intel.dmg",
                "browser_download_url": "https://example.com/macos.dmg",
            },
        ]

        url = checker._find_download_url(assets)
        assert url == "https://example.com/windows.exe"

    @patch("platform.system")
    def test_find_download_url_macos(self, mock_platform):
        """Test finding macOS download URL."""
        checker = UpdateChecker()
        mock_platform.return_value = "Darwin"

        assets = [
            {
                "name": "PhotoSort-Windows-x64.exe",
                "browser_download_url": "https://example.com/windows.exe",
            },
            {
                "name": "PhotoSort-macOS-Intel.dmg",
                "browser_download_url": "https://example.com/macos.dmg",
            },
        ]

        url = checker._find_download_url(assets)
        assert url == "https://example.com/macos.dmg"

    @patch("platform.system")
    def test_find_download_url_no_match(self, mock_platform):
        """Test finding download URL when no platform match."""
        checker = UpdateChecker()
        mock_platform.return_value = "Windows"

        assets = [
            {
                "name": "PhotoSort-Linux-x64.tar.gz",
                "browser_download_url": "https://example.com/linux.tar.gz",
            }
        ]

        url = checker._find_download_url(assets)
        assert url is None


class TestUpdateInfo:
    """Tests the UpdateInfo dataclass."""

    def test_update_info_creation(self):
        """Test creating UpdateInfo object."""
        info = UpdateInfo(
            version="1.0.3",
            release_url="https://github.com/example/repo/releases/tag/v1.0.3",
            release_notes="Bug fixes",
            published_at="2025-01-01T00:00:00Z",
            download_url="https://github.com/example/repo/releases/download/v1.0.3/app.exe",
        )

        assert info.version == "1.0.3"
        assert info.release_url == "https://github.com/example/repo/releases/tag/v1.0.3"
        assert info.release_notes == "Bug fixes"
        assert info.published_at == "2025-01-01T00:00:00Z"
        assert (
            info.download_url
            == "https://github.com/example/repo/releases/download/v1.0.3/app.exe"
        )

    def test_update_info_optional_download_url(self):
        """Test UpdateInfo with optional download URL."""
        info = UpdateInfo(
            version="1.0.3",
            release_url="https://github.com/example/repo/releases/tag/v1.0.3",
            release_notes="Bug fixes",
            published_at="2025-01-01T00:00:00Z",
        )

        assert info.download_url is None

import os
import shutil
import logging
from pathlib import Path
import send2trash

logger = logging.getLogger(__name__)


class ImageFileOperations:
    """Handles file system operations for image files."""

    @staticmethod
    def move_path(source_path: str, destination_path: str) -> tuple[bool, str]:
        """Move a file or directory to an exact destination path.

        This lower-level primitive is used by workflows that have already
        resolved naming collisions and therefore cannot use :meth:`move_image`.
        """

        source = Path(source_path)
        destination = Path(destination_path)
        if not source.exists():
            return False, f"Source path does not exist: {source_path}"
        try:
            destination.parent.mkdir(parents=True, exist_ok=True)
            moved_path = source.move(destination)
            logger.info(
                "Moved '%s' to '%s'.",
                source.name,
                destination.name,
            )
            return True, str(moved_path)
        except OSError as exc:
            message = f"Failed to move '{source_path}' to '{destination_path}': {exc}"
            logger.error(message, exc_info=True)
            return False, message

    @staticmethod
    def clear_directory_contents(directory: str) -> tuple[bool, str]:
        """Remove every child of ``directory`` while keeping the directory."""

        if not os.path.isdir(directory):
            return True, "Directory does not exist."
        try:
            with os.scandir(directory) as entries:
                for entry in entries:
                    if entry.is_dir(follow_symlinks=False):
                        shutil.rmtree(entry.path)
                    else:
                        os.unlink(entry.path)
            return True, "Directory contents cleared."
        except OSError as exc:
            message = f"Failed to clear directory '{directory}': {exc}"
            logger.error(message, exc_info=True)
            return False, message

    @staticmethod
    def remove_empty_directory(directory: str) -> bool:
        """Remove an empty directory, returning whether it was removed."""

        try:
            os.rmdir(directory)
            return True
        except OSError:
            return False

    @staticmethod
    def move_image(source_path: str, destination_folder: str) -> tuple[bool, str]:
        """
        Moves an image file from the source path to the destination folder.

        Args:
            source_path (str): The full path to the source image file.
            destination_folder (str): The full path to the destination folder.

        Returns:
            tuple: (bool, str) indicating success status and the new file path if successful,
                   or (False, str) with an error message if failed.
        """
        source = Path(source_path)
        destination_dir = Path(destination_folder)
        if not source.is_file():
            return False, f"Source is not a valid file: {source_path}"
        if not destination_dir.is_dir():
            # Attempt to create destination folder if it doesn't exist
            try:
                destination_dir.mkdir(parents=True, exist_ok=True)
                logger.info("Created destination folder: %s", destination_folder)
            except OSError as e:
                return (
                    False,
                    f"Destination is not a valid folder and could not be created: {destination_folder}. Error: {e}",
                )

        destination = destination_dir / source.name

        if destination.exists():
            # Basic conflict resolution: append a number if file exists
            counter = 1
            while destination.exists():
                destination = destination_dir / (
                    f"{source.stem}_{counter}{source.suffix}"
                )
                counter += 1
            logger.debug("Destination file exists. Renaming to: %s.", destination.name)

        try:
            moved_path = source.move(destination)
            logger.info("Moved '%s' to '%s'.", source.name, destination.name)
            return True, str(moved_path)
        except Exception as e:
            error_msg = f"Error moving file '{source.name}': {e}"
            logger.error(error_msg, exc_info=True)
            return False, error_msg

    @staticmethod
    def move_to_trash(file_path: str) -> tuple[bool, str]:
        """
        Moves a file to the system's trash or recycling bin.

        Args:
            file_path (str): The full path to the file to be moved.

        Returns:
            tuple: (bool, str) indicating success and a message.
        """
        if not os.path.exists(file_path):
            logger.warning("File does not exist when moving to trash: %s", file_path)
            return False, "File does not exist."
        try:
            send2trash.send2trash(file_path)
            logger.info("Moved to trash: %s.", os.path.basename(file_path))
            return True, "File moved to trash."
        except Exception as e:
            error_msg = (
                f"Error moving file to trash: {os.path.basename(file_path)}: {e}"
            )
            logger.error(error_msg, exc_info=True)
            return False, error_msg

    @staticmethod
    def rename_image(old_path: str, new_path: str) -> tuple[bool, str]:
        """
        Renames an image file.

        Args:
            old_path (str): The current path of the file.
            new_path (str): The new path for the file.

        Returns:
            tuple: (bool, str) indicating success and a message.
        """
        if not os.path.exists(old_path):
            return False, "Original file does not exist."
        try:
            os.rename(old_path, new_path)
            logger.info(
                "Renamed '%s' to '%s'",
                os.path.basename(old_path),
                os.path.basename(new_path),
            )
            return True, "File renamed successfully."
        except OSError as e:
            error_msg = f"Error renaming '{os.path.basename(old_path)}': {e}"
            logger.error(error_msg, exc_info=True)
            return False, error_msg

    @staticmethod
    def replace_file(source_path: str, destination_path: str) -> tuple[bool, str]:
        """
        Safely replaces the destination file with the source file.

        Args:
            source_path (str): The path to the source file (e.g., a temporary file).
            destination_path (str): The path to the destination file to be replaced.

        Returns:
            tuple: (bool, str) indicating success and a message.
        """
        source = Path(source_path)
        destination = Path(destination_path)
        if not source.is_file():
            return False, f"Source file not found: {source_path}"
        try:
            source.move(destination)
            logger.info("Replaced '%s' with '%s'.", destination.name, source.name)
            return True, "File replaced successfully."
        except Exception as e:
            error_msg = f"Error replacing file '{destination.name}': {e}"
            logger.error(error_msg, exc_info=True)
            return False, error_msg

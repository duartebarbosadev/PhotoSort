import os
import shutil
import logging

logger = logging.getLogger(__name__)
from typing import Tuple
import send2trash


class ImageFileOperations:
    """Handles file system operations for image files."""

    @staticmethod
    def move_image(source_path: str, destination_folder: str) -> Tuple[bool, str]:
        """
        Moves an image file from the source path to the destination folder.

        Args:
            source_path (str): The full path to the source image file.
            destination_folder (str): The full path to the destination folder.

        Returns:
            tuple: (bool, str) indicating success status and the new file path if successful,
                   or (False, str) with an error message if failed.
        """
        if not os.path.isfile(source_path):
            return False, f"Source is not a valid file: {source_path}"
        if not os.path.isdir(destination_folder):
            # Attempt to create destination folder if it doesn't exist
            try:
                os.makedirs(destination_folder, exist_ok=True)
                logger.info(f"Created destination folder: {destination_folder}")
            except OSError as e:
                return (
                    False,
                    f"Destination is not a valid folder and could not be created: {destination_folder}. Error: {e}",
                )

        filename = os.path.basename(source_path)
        destination_path = os.path.join(destination_folder, filename)

        if os.path.exists(destination_path):
            # Basic conflict resolution: append a number if file exists
            base, ext = os.path.splitext(filename)
            counter = 1
            while os.path.exists(destination_path):
                destination_path = os.path.join(
                    destination_folder, f"{base}_{counter}{ext}"
                )
                counter += 1
            logger.debug(
                f"Destination file exists. Renaming to: {os.path.basename(destination_path)}."
            )

        try:
            shutil.move(source_path, destination_path)
            logger.info(
                f"Moved '{os.path.basename(source_path)}' to '{os.path.basename(destination_path)}'."
            )
            return True, destination_path
        except Exception as e:
            error_msg = f"Error moving file '{os.path.basename(source_path)}': {e}"
            logger.error(error_msg, exc_info=True)
            return False, error_msg

    @staticmethod
    def move_to_trash(file_path: str) -> Tuple[bool, str]:
        """
        Moves a file to the system's trash or recycling bin.

        Args:
            file_path (str): The full path to the file to be moved.

        Returns:
            tuple: (bool, str) indicating success and a message.
        """
        if not os.path.exists(file_path):
            return False, "File does not exist."
        try:
            send2trash.send2trash(file_path)
            logger.info(f"Moved to trash: {os.path.basename(file_path)}.")
            return True, "File moved to trash."
        except Exception as e:
            error_msg = (
                f"Error moving file to trash: {os.path.basename(file_path)}: {e}"
            )
            logger.error(error_msg, exc_info=True)
            return False, error_msg

    @staticmethod
    def rename_image(old_path: str, new_path: str) -> Tuple[bool, str]:
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
    def replace_file(source_path: str, destination_path: str) -> Tuple[bool, str]:
        """
        Safely replaces the destination file with the source file.

        Args:
            source_path (str): The path to the source file (e.g., a temporary file).
            destination_path (str): The path to the destination file to be replaced.

        Returns:
            tuple: (bool, str) indicating success and a message.
        """
        if not os.path.isfile(source_path):
            return False, f"Source file not found: {source_path}"
        try:
            shutil.move(source_path, destination_path)
            logger.info(
                f"Replaced '{os.path.basename(destination_path)}' with '{os.path.basename(source_path)}'."
            )
            return True, "File replaced successfully."
        except Exception as e:
            error_msg = (
                f"Error replacing file '{os.path.basename(destination_path)}': {e}"
            )
            logger.error(error_msg, exc_info=True)
            return False, error_msg

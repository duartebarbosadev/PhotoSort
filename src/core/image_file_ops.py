import os
import shutil
from typing import Tuple

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
                print(f"Created destination folder: {destination_folder}")
            except OSError as e:
                return False, f"Destination is not a valid folder and could not be created: {destination_folder}. Error: {e}"
        
        filename = os.path.basename(source_path)
        destination_path = os.path.join(destination_folder, filename)

        if os.path.exists(destination_path):
            # Basic conflict resolution: append a number if file exists
            # More sophisticated handling might be needed (e.g., user prompt or specific naming scheme)
            base, ext = os.path.splitext(filename)
            counter = 1
            while os.path.exists(destination_path):
                destination_path = os.path.join(destination_folder, f"{base}_{counter}{ext}")
                counter += 1
            print(f"File already exists at original destination. New destination path: {destination_path}")


        try:
            shutil.move(source_path, destination_path)
            print(f"Successfully moved '{source_path}' to '{destination_path}'")
            return True, destination_path
        except Exception as e:
            error_msg = f"Error moving file '{source_path}' to '{destination_path}': {e}"
            print(error_msg)
            return False, error_msg

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

if __name__ == '__main__':
    # Example Usage
    # Create dummy files and folders for testing
    test_src_dir = "test_source_image_ops"
    test_dest_dir = "test_dest_image_ops"
    os.makedirs(test_src_dir, exist_ok=True)
    os.makedirs(test_dest_dir, exist_ok=True)

    sample_file_name = "test_move_image.txt"
    sample_file_path = os.path.join(test_src_dir, sample_file_name)

    with open(sample_file_path, "w") as f:
        f.write("This is a test file for moving.")
    
    print(f"--- Testing ImageFileOperations with: {sample_file_path} ---")

    # Test 1: Move file
    print(f"\nAttempting to move '{sample_file_path}' to '{test_dest_dir}'...")
    success, result_msg = ImageFileOperations.move_image(sample_file_path, test_dest_dir)
    new_file_path = ""
    if success:
        print(f"  Move successful. New path: {result_msg}")
        new_file_path = result_msg
        assert os.path.exists(new_file_path)
        assert not os.path.exists(sample_file_path)
    else:
        print(f"  Move failed: {result_msg}")

    # Test 2: Move again to test conflict (if first move was successful)
    if success:
        # Recreate the source file to test conflict
        with open(sample_file_path, "w") as f:
            f.write("This is a recreated test file for conflict.")
        
        print(f"\nAttempting to move '{sample_file_path}' to '{test_dest_dir}' again (expecting conflict resolution)...")
        success_conflict, result_msg_conflict = ImageFileOperations.move_image(sample_file_path, test_dest_dir)
        if success_conflict:
            print(f"  Conflict move successful. New path: {result_msg_conflict}")
            assert os.path.exists(result_msg_conflict)
            assert not os.path.exists(sample_file_path)
            assert result_msg_conflict != new_file_path # Path should be different due to conflict resolution
        else:
            print(f"  Conflict move failed: {result_msg_conflict}")

    # Test 3: Move to non-existent creatable folder
    non_existent_dest_dir = os.path.join(test_dest_dir, "subfolder_to_create")
    if os.path.exists(sample_file_path): # If previous move failed or was conflict-resolved from original path
        print(f"\nAttempting to move '{sample_file_path}' to creatable folder '{non_existent_dest_dir}'...")
        success_create, result_msg_create = ImageFileOperations.move_image(sample_file_path, non_existent_dest_dir)
        if success_create:
            print(f"  Move to creatable folder successful. New path: {result_msg_create}")
            assert os.path.exists(result_msg_create)
            assert not os.path.exists(sample_file_path)
        else:
            print(f"  Move to creatable folder failed: {result_msg_create}")


    print("\n--- ImageFileOperations Tests Complete ---")

    # Clean up
    shutil.rmtree(test_src_dir, ignore_errors=True)
    shutil.rmtree(test_dest_dir, ignore_errors=True)
    print("Test directories and files cleaned up.")
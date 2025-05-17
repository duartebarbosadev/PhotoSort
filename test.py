import os
from exiftool import ExifToolHelper

# Define the directory containing the images
metadatadir = r"test folder"

# Define common image file extensions
# You can add or remove extensions as needed
image_extensions = ('.arw', '.jpg', '.jpeg', '.png', '.tif', '.tiff', '.cr2', '.nef', '.dng', '.orf', '.rw2')

image_files_to_process = []

# Check if the directory exists
if not os.path.isdir(metadatadir):
    print(f"Error: Directory not found: {metadatadir}")
else:
    # Walk through the directory
    for filename in os.listdir(metadatadir):
        # Check if the file has one of the specified image extensions (case-insensitive)
        if filename.lower().endswith(image_extensions):
            # Construct the full path to the file
            full_path = os.path.join(metadatadir, filename)
            image_files_to_process.append(full_path)

    if not image_files_to_process:
        print(f"No image files found in {metadatadir} with extensions: {image_extensions}")
    else:
        print(f"Found {len(image_files_to_process)} image(s) to process: {image_files_to_process}")
        print("-" * 30)

        try:
            with ExifToolHelper() as et:
                # Pass the list of discovered image file paths
                # Also, it's good practice to include 'SourceFile' to know which file the metadata belongs to
                for d in et.get_tags(image_files_to_process, tags=["SourceFile", "FileSize", "ImageSize", "XMP:Rating"]):
                #for d in et.get_metadata(image_files_to_process):
                    source_file = d.get("SourceFile", "Unknown File") # Get the source file, default to "Unknown"
                    print(f"--- Metadata for: {os.path.basename(source_file)} ---")
                    for k, v in d.items():
                        # Optionally, skip printing SourceFile again if you've already printed it above
                        if k == "SourceFile" and source_file != "Unknown File":
                            continue
                        print(f"  {k} = {v}")
                    print("-" * 30) # Separator for readability
        except FileNotFoundError:
            print("Error: ExifTool not found. Please ensure it's installed and in your system's PATH.")
        except Exception as e:
            print(f"An error occurred: {e}")

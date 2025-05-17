# PhotoSort: Intelligent Photo Library Management

PhotoSort is a powerful desktop application designed to streamline the management of large photo libraries, making it easier than ever to sort, cull, and organize your images. Its primary goal is to help users efficiently identify unwanted photos—whether they are blurry, redundant, or simply not up to par—and quickly remove them to reclaim disk space and bring order to their collections.

✨ *This project was born out of personal necessity and a whole lot of ~vibe coding~ to finally tackle a chaotic photo library!* ✨

## Key Features

*   **Advanced Sorting & Viewing**:
    *   Flexible view modes: List, Icons, Grid, and Chronological by Date.
    *   Organize images within their existing folder structures.
*   **Intelligent Culling Tools**:
    *   **Ratings & Labels**: Assign star ratings and color labels for quick categorization.
    *   **Blur Detection**: Automatically identify and flag blurry photos.
    *   **Similarity Analysis**: Group visually similar images to easily spot duplicates or near-duplicates.
*   **Efficient Workflow**:
    *   **Robust Filtering**: Filter images by ratings, similarity clusters, or filenames.
    *   **Background Processing**: Intensive operations (scanning, thumbnailing, analysis) run in the background for a responsive UI.
    *   **Optimized Image Handling**: Supports a wide range of formats, including various RAW types, with efficient caching.
*   **File Management**:
    *   Move unwanted photos to the system trash.
*   **User Interface**:
    *   Built with PyQt6.
    *   Includes a comfortable dark theme.

## Technology Stack

*   **Core Language**: Python
*   **GUI Framework**: PyQt6
*   **Image Processing**:
    *   Pillow (PIL Fork)
    *   rawpy (for RAW image processing)
    *   OpenCV (cv2 - for blur detection via Laplacian variance)
*   **Machine Learning / AI**:
    *   SentenceTransformers (with CLIP models like `clip-ViT-B-32` for image embeddings)
    *   scikit-learn (for DBSCAN clustering)
    *   NumPy (for numerical operations)
*   **Metadata**:
    *   ExifTool (for reading/writing XMP metadata like ratings and labels)
*   **Packaging/Misc**:
    *   `send2trash` (for moving files to trash)

## Getting Started

### Prerequisites

*   **Python 3.x**: Download from [python.org](https://www.python.org/).
*   **ExifTool**: Essential for reading and writing metadata (ratings, labels).
    *   Download the ExifTool executable from the [official ExifTool website](https://exiftool.org/).
    *   **Windows**: Rename the downloaded `exiftool(-k).exe` to `exiftool.exe` and place it in a directory that is part of your system's PATH (e.g., `C:\Windows`), or add its directory to the PATH environment variable.
    *   **macOS/Linux**: Place the `exiftool` executable in a directory included in your system's PATH (e.g., `/usr/local/bin/`). Ensure it's executable (`chmod +x exiftool`).
*   **CUDA (Optional, for GPU Acceleration)**:
    *   For significantly faster image embedding generation (used in Similarity Analysis), a CUDA-enabled NVIDIA GPU is beneficial.
    *   You'll need:
        1.  NVIDIA GPU drivers.
        2.  CUDA Toolkit installed. You can find versions compatible with PyTorch on the [PyTorch website](https://pytorch.org/get-started/locally/).
    *   If CUDA is not available or not set up correctly, the application will automatically fall back to using the CPU for these tasks, which will be slower. The application checks CUDA availability using `torch.cuda.is_available()` (see [`src/core/app_settings.py`](src/core/app_settings.py:18)).

### Installation & Running

1.  **Clone the repository (if applicable):**
    ```bash
    git clone <your-repository-url>
    cd PhotoSort
    ```

2.  **Create a virtual environment (recommended):**
    ```bash
    python -m venv venv
    source venv/bin/activate  # On Windows: venv\Scripts\activate
    ```

3.  **Install dependencies:**
    Ensure your [`requirements.txt`](requirements.txt) is up-to-date with all necessary packages (PyQt6, Pillow, rawpy, opencv-python, sentence-transformers, scikit-learn, numpy, send2trash, exiftool (PyExifTool).
    ```bash
    pip install -r requirements.txt
    ```

4.  **Run the application:**
    The main entry point is [`src/main.py`](src/main.py:147).
    ```bash
    python src/main.py
    ```

## Usage

1.  **Open Folder**: Use "File" > "Open Folder..." to select a directory containing your images.
2.  **Scanning**: The application will scan for supported image files. Thumbnails and basic metadata will be loaded.
3.  **Navigate & View**:
    *   Use the left pane to browse files and folders.
    *   Switch between List, Icons, Grid, or Date views using the buttons at the bottom.
    *   Select an image to see a larger preview in the center pane.
4.  **Rate & Label**:
    *   Use the star buttons (or number keys 0-5) under the preview to assign ratings.
    *   Use the color squares to assign labels.
5.  **Analyze Images**:
    *   **Detect Blurriness**: Go to "View" > "Detect Blurriness" to analyze images. Blurred images will be visually indicated.
    *   **Analyze Similarity**: Go to "View" > "Analyze Similarity" to generate embeddings and cluster similar photos. Use the "Group by Similarity" option and cluster filters to explore groups.
6.  **Filter**: Use the filter dropdowns and search bar at the bottom to narrow down the displayed images.
7.  **Delete**:
    *   Select an image and press `Delete` to move it to the trash.
8.  **Settings**:
    *   Access "Settings" > "Manage Cache" to clear thumbnail/preview caches or adjust the preview cache size limit.
    *   Toggle "Enable Auto RAW Edits" for automatic adjustments to RAW previews and thumbnails.

8.  **Keyboard Shortcuts**: Speed up your workflow with these shortcuts:
    *   **Rating**:
        *   `0` - `5`: Assign 0 to 5 stars to the selected image.
    *   **Navigation**:
        *   `Left Arrow` / `A`: Navigate to the previous image.
        *   `Right Arrow` / `D`: Navigate to the next image.
    *   **File Operations**:
        *   `Delete`: Move the selected image to the system trash.
    *   **Interface**:
        *   `Ctrl+F` (or `Cmd+F` on macOS): Focus the search input field.
        *   `Esc`: If the search input is focused, unfocus it and return focus to the image list/grid.
    *   **Similarity Group Navigation** (when "Group by Similarity" is active):
        *   `Ctrl+1` through `Ctrl+9`: Jump to the 1st through 9th image within the currently selected/viewed similarity cluster.

## Future Enhancements (Ideas)

*   More sophisticated duplicate detection (e.g., based on perceptual hashing or exact content).
*   Batch processing for metadata edits.
*   Customizable sorting criteria.
*   Integration with cloud storage providers.
*   Advanced search capabilities (e.g., by EXIF data).

## Contributing

Contributions are welcome! Please feel free to submit a pull request or open an issue for bugs, feature requests, or suggestions.

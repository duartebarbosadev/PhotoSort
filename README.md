# PhotoSort: Intelligent Photo Library Management

PhotoSort is a powerful desktop application focused on speed designed to streamline the management of large photo libraries, making it easier than ever to sort, cull, and organize your images.

**Warning - Use this at your personal risk. Always use backups.**

## Key Features

* **Advanced Sorting & Viewing**:
  * Flexible view modes: List, Icons, Grid, and Chronological by Date.
  * **Real-time Search**: Instantly filter images by filename with live search results.
* **Intelligent Culling Tools**:
  * **Ratings & Labels**: Assign star ratings and color labels for quick categorization.
  * **Blur Detection**: Automatically identify and flag blurry photos.
  * **Similarity Analysis**: Group visually similar images to easily spot duplicates or near-duplicates.
* **Efficient Workflow**:
  * **Advanced Filtering System**: 
    * Filter images by ratings (Show All, 1+ Stars, 2+ Stars, etc.)
    * Filter by similarity clusters when grouping is active
    * Combine text search with rating and cluster filters
    * Real-time filtering with immediate visual feedback
  * **Fast Processing**: Intensive operations (scanning, thumbnailing, analysis) run once in batch to ensure fast image scrolling.
  * **Optimized Image Handling**: Supports a wide range of formats, including various RAW types, with efficient caching.
* **File Management**:
  * Move unwanted photos to the system trash.
  * **Intelligent Image Rotation**: Smart rotation system that automatically tries lossless metadata rotation first, with optional fallback to pixel rotation when needed. Supports all major formats with format-specific optimization.

## Technology Stack

* **Core Language**: Python
* **GUI Framework**: PyQt6
* **Image Processing**:
  * Pillow (PIL Fork)
  * rawpy (for RAW image processing)
  * OpenCV (cv2 - for blur detection via Laplacian variance)
  * pyexiv2 (for metadata reading/writing and image rotation)
* **Machine Learning / AI**:
  * SentenceTransformers (with CLIP models like `clip-ViT-B-32` for image embeddings)
  * scikit-learn (for DBSCAN clustering)
  * NumPy (for numerical operations)
* **Packaging/Misc**:
  * `send2trash` (for moving files to trash)

## Getting Started

### Prerequisites

* **Python 3.x**: Download from [python.org](https://www.python.org/).
* **jpegtran (Optional, for Lossless JPEG Rotation)**:
  * **Windows**: Download from [jpegclub.org](http://jpegclub.org/jpegtran/) or install via Chocolatey: `choco install libjpeg-turbo`
  * **macOS**: `brew install jpeg-turbo`
  * **Linux**: `sudo apt-get install libjpeg-turbo-progs` (Ubuntu/Debian) or `sudo yum install libjpeg-turbo-utils` (RedHat/CentOS)
* **CUDA (Optional, for GPU Acceleration)**:
  * For significantly faster image embedding generation (used in Similarity Analysis), a CUDA-enabled NVIDIA GPU is beneficial.
  * You'll need:
    1. NVIDIA GPU drivers.
    2. CUDA Toolkit installed. You can find versions compatible with PyTorch on the [PyTorch website](https://pytorch.org/get-started/locally/).
  * If CUDA is not available or not set up correctly, the application will automatically fall back to using the CPU for these tasks, which will be slower. The application checks CUDA availability using `torch.cuda.is_available()` (see [`src/core/app_settings.py`](src/core/app_settings.py:18)).

### Installation & Running

1. **Clone the repository (if applicable):**

   ```bash
   git clone <your-repository-url>
   cd PhotoSort
   ```
2. **Create a virtual environment (recommended):**

   ```bash
   python -m venv venv
   source venv/bin/activate  # On Windows: venv\Scripts\activate
   ```
3. **Install dependencies:**
   Ensure your [`requirements.txt`](requirements.txt) is up-to-date with all necessary packages (PyQt6, Pillow, rawpy, opencv-python, sentence-transformers, scikit-learn, numpy, send2trash, pyexiv2

   ```bash
   pip install -r requirements.txt
   ```
4. **Run the application:**
   The main entry point is [`src/main.py`](src/main.py:147).

   ```bash
   python -m src.main [--folder FOLDER_PATH] [--clear-cache]

   # Examples:
   #   Open a specific folder at startup:
   #       python -m src.main --folder "C:/Users/MyUser/Pictures"
   #   Clear all caches before starting:
   #       python -m src.main --clear-cache
   #   Open folder and clear caches:
   #       python -m src.main --folder "C:/Users/MyUser/Pictures" --clear-cache
   ```

## Usage

1. **Open Folder**: Use "File" > "Open Folder..." to select a directory containing your images.
2. **Scanning**: The application will scan for supported image files. Thumbnails and metadata will be loaded.
3. **Navigate & View**:

   * Use the left pane to browse files and folders.
   * Switch between List, Icons, Grid, or Date views using the buttons at the bottom.
   * Select an image to see a larger preview in the center pane.
4. **Rate & Label**:

   * Use the star buttons (or number keys 0-5) under the preview to assign ratings.
   * Use the color squares to assign labels.
5. **Analyze Images**:

   * **Detect Blurriness**: Go to "View" > "Detect Blurriness" to analyze images. Blurred images will be visually indicated.
   * **Analyze Similarity**: Go to "View" > "Analyze Similarity" to generate embeddings and cluster similar photos. Use the "Group by Similarity" option and cluster filters to explore groups.
6. **Search & Filter**: 
   * **Real-time Search**: Use the search bar at the bottom to instantly filter images by filename. Results update as you type.
   * **Rating Filters**: Use the rating dropdown to show only images with specific star ratings (Show All, 1+ Stars, 2+ Stars, etc.).
   * **Similarity Filters**: When "Group by Similarity" is active, use the cluster filter to view specific similarity groups.
   * **Combined Filtering**: All filters work together - combine search text with rating and cluster filters for precise results.
7. **Rotate Images**:

   * **Via Context Menu**: Right-click any image → choose rotation direction (90° clockwise, counterclockwise, or 180°).
   * **Via Keyboard**: Use `Ctrl+R` (clockwise), `Ctrl+Shift+R` (counterclockwise), or `Ctrl+Alt+R` (180°).
   * **Via Image Menu**: Access rotation commands through the "Image" menu.
   * **Intelligent Rotation Process**:
     1. **Metadata-First**: Always attempts lossless XMP orientation metadata update first
     2. **Smart Fallback**: If metadata rotation fails, offers appropriate alternatives based on format:
        * **JPEG**: Lossless `jpegtran` rotation → Lossy re-encoding (with user confirmation)
        * **PNG/TIFF/BMP**: Metadata -> Lossy re-encoding (with user confirmation)
        * **RAW formats**: Metadata-only
     3. **User Control**: Asks for confirmation before quality-reducing operations
   * **Quality Preservation**: Maximizes image quality by prioritizing lossless methods
   * **Transparent Feedback**: Clear messages indicate whether rotation was lossless or lossy
8. **Delete**:

   * Select an image and press `Delete` to move it to the trash.
9. **Settings**:

   * Access "Settings" > "Manage Cache" to clear thumbnail/preview caches or adjust the preview cache size limit.
   * Toggle "Enable Auto RAW Edits" for automatic adjustments to RAW previews.
10. **Keyboard Shortcuts**: Speed up your workflow with these shortcuts:

    * **Rating**:
      * `CTRL+0` - `CTRL+5`: Assign 0 to 5 stars to the selected image.
    * **Navigation**:
      * `Down Arrow/Up Arrow`: Navigate to the next/previous image.
      * `Left Arrow/Right Arrow`: Navigate to the previous/next image of the same group (doesn't jump groups automatically).
    * **File Operations**:
      * `Delete`: Move the selected image to the system trash.
    * **Image Rotation**:
      * `Ctrl+R`: Rotate selected image 90° clockwise.
      * `Ctrl+Shift+R`: Rotate selected image 90° counterclockwise.
      * `Ctrl+Alt+R`: Rotate selected image 180°.
    * **Interface**:
      * `Ctrl+F` (or `Cmd+F` on macOS): Focus the search input field for instant filename filtering.
      * `Esc`: If the search input is focused, unfocus it and return focus to the image list/grid.
      * `I`: Toggle image details sidebar.
    * **Settings**:
      * Rotation confirmation preferences can be disabled via the lossy rotation dialog's "Don't ask again" checkbox.
    * **Similarity Group Navigation** (when "Group by Similarity" is active):
      * `1` through `9`: Jump to the 1st through 9th image within the currently selected/viewed similarity cluster.
    * **Command-line Arguments**:
      * `--folder FOLDER_PATH`: Open the specified folder immediately after startup
      * `--clear-cache`: Clear all image and metadata caches before starting

## Future Enhancements (Ideas)

* **Enhanced Search Capabilities**:
  * Search by EXIF metadata (camera model, settings, date ranges)
  * Search by color labels and custom tags
  * Saved search presets
* **Sort/Order by Rating**: Implement functionality to sort or reorder images directly based on their assigned star ratings.
* **AI-Driven Exposure Analysis**: Introduce a feature to detect and flag images with potentially good or problematic exposure (e.g., under/overexposed).
* **Automated Best Shot Selection in Clusters**:
  * Within similarity clusters, automatically suggest or select the "best" image(s).
  * Criteria could include: lowest blurriness score, optimal exposure, AI composition analysis, no one with eyes close etc.
* **Advanced AI Object/Scene Detections & Grouping**:
  * **Car Model Recognition**: Identify and allow grouping by specific car models in photos.
  * **Face Recognition/Clustering**: Detect faces and group photos by the people present.
* **Side by Side** Select Multiple images and see them side by side, with zoom lock etc.
* **Video Support**

## Contributing

Contributions are welcome! Please feel free to submit a pull request or open an issue for bugs, feature requests, or suggestions.

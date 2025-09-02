# PhotoSort: Intelligent Photo Library Management

PhotoSort is a powerful desktop application focused on speed designed to streamline the management of large photo libraries, making it easier than ever to sort, cull, and organize your images.

**Warning - Use this at your personal risk. Always use backups.**

## Key Features

* **Intelligent Culling Tools**:
  * **Ratings & Labels**: Assign star ratings for quick categorization.
  * **Blur Detection**: Automatically identify and flag blurry photos.
  * **AI Orientation Detection**: Auto-detects the correct image orientation using a lightweight ONNX model and proposes rotations.
  * **Similarity Analysis**: Group visually similar images to easily spot duplicates or near-duplicates.
  * **Fast Processing**: Intensive operations (scanning, thumbnailing, analysis) run once in batch to ensure fast image scrolling.
  * **Optimized Image Handling**: Supports a wide range of formats, including various RAW types, with efficient caching.
  * **Intelligent Image Rotation**: Smart rotation system that automatically tries lossless metadata rotation first, with optional fallback to pixel rotation when needed.
- **Metadata Display**: Shows EXIF information (camera model, exposure settings, etc.).

## Getting Started

### Prerequisites

* **Python 3.x**: Download from [python.org](https://www.python.org/).
* **jpegtran (Optional, for Lossless JPEG Rotation)**:
  * **Windows**: Download from [jpegclub.org](http://jpegclub.org/jpegtran/) or install via Chocolatey: `choco install libjpeg-turbo`
  * **macOS**: `brew install jpeg-turbo`
  * **Linux**: `sudo apt-get install libjpeg-turbo-progs` (Ubuntu/Debian) or `sudo yum install libjpeg-turbo-utils` (RedHat/CentOS)

### Hardware Acceleration (Optional, Recommended)

For significantly faster AI-powered features like **Rotation Detection** and **Similarity Analysis**, it is highly recommended to install the appropriate ONNX Runtime package for your hardware. The application will automatically use the best available hardware (GPU > CPU).

First, uninstall the basic CPU package to avoid conflicts:

```bash
pip uninstall onnxruntime
```

Then, install the package corresponding to your hardware:

#### For NVIDIA GPUs (CUDA)

```bash
# Requires NVIDIA CUDA Toolkit & cuDNN
pip install onnxruntime-gpu
```

#### For Apple Silicon (M1/M2/M3)

```bash
# Uses Apple's Metal Performance Shaders (MPS)
pip install onnxruntime-silicon
```

#### For AMD GPUs (ROCm) - Untested

```bash
# Requires AMD ROCm driver/libraries
pip install onnxruntime-rocm
```

### AI Model Setup (Required for Rotation Detection)

To use the **Auto Rotate Images** feature (`Ctrl+R`), you need to download the pre-trained orientation detection model.

1.  **Create a `models` directory** in the root of the project.
2.  **Download the model file**:
    *   **Link**: [Download orientation_model_v2_0.9882.onnx from Hugging Face](https://huggingface.co/DuarteBarbosa/deep-image-orientation-detection/tree/main)
3.  **Place the downloaded model file inside the `models` directory.**

The application will automatically detect and load the model when you use the rotation detection feature.

### Installation & Running

1. **Clone the repository (if applicable):**

   ```bash
   git clone https://github.com/duartebarbosadev/photosort
   cd PhotoSort
   ```
2. **Create a virtual environment (recommended):**

   ```bash
   python -m venv venv
   source venv/bin/activate  # On Windows: venv\Scripts\activate
   ```
3. **Install dependencies:**
   Ensure your [`requirements.txt`](requirements.txt) is up-to-date with all necessary packages (PyQt6, Pillow, rawpy, opencv-python, sentence-transformers, scikit-learn, numpy, send2trash, pyexiv2, onnxruntime).

   ```bash
   pip install -r requirements.txt
   ```
4. **Run the application:**
   The main entry point is [`src/main.py`](src/main.py).

   ```bash
   python -m src.main [--folder FOLDER_PATH] [--clear-cache]

   # Examples:
   #   Open a specific folder at startup:
   #       python -m src.main --folder "C:/Users/MyUser/Pictures"
   #   Clear all caches before starting:
   #       python -m src.main --clear-cache
   #   Open folder and clear caches (useful for development):
   #       python -m src.main --folder "C:/Users/MyUser/Pictures" --clear-cache
   ```

### Exporting Logs

To capture detailed logs for debugging, you can enable file logging by setting an environment variable before running the application.

*   **macOS/Linux**:
    ```bash
    export PHOTOSORT_ENABLE_FILE_LOGGING=true
    python -m src.main
    ```
*   **Windows (Command Prompt)**:
    ```bash
    set PHOTOSORT_ENABLE_FILE_LOGGING=true
    python -m src.main
    ```
*   **Windows (PowerShell)**:
    ```powershell
    $env:PHOTOSORT_ENABLE_FILE_LOGGING="true"
    python -m src.main
    ```

Logs will be saved to `~/.photosort_logs/photosort_app.log`.

## **Keyboard Shortcuts**:

**File Management**

* **Open Folder:** `Ctrl/Cmd+O`
* **Exit:** `Ctrl+F4/Cmd+Q`

**Image Viewing and Navigation**

* **Show/Hide Image Details Sidebar:** `I`
* **Find Image:** `Cmd+F`
* **Focus on a specific image in the grid (1-9):** `1` through `9`
* **Zoom In:** `+`
* **Zoom Out:** `-`
* **Fit to View:** `0`
* **Actual Size (100%):** `A`
* **Synchronize Pan & Zoom:** `F3`
* **Single View:** `F1`
* **Side by Side View:** `F2`
* **List View:** `Alt+1`
* **Icons View:** `Alt+2`
* **Grid View:** `Alt+3`
* **Rotation View:** `Alt+4`

**Arrow Key Navigation**

* **Navigate Between Images:** `Arrow Keys` (←, →, ↑, ↓) or `H`, `J`, `K`, `L` (Vim-style)
  * Automatically skips files marked for deletion (with "(DELETED)" in filename)
  * Left/Right (or H/L): Navigate within the same group/folder
  * Up/Down (or K/J): Navigate sequentially through all visible images
**Navigate Including Deleted Images:**
  * Windows/Linux: `Ctrl+Arrow Keys` or `Ctrl+H/J/K/L`
  * macOS: `Cmd+Arrow Keys` or `Cmd+H/J/K/L`
  * Same navigation as above, but **does not skip** files marked for deletion
  * Useful for reviewing files before committing deletions
  * Allows access to deleted files for comparison or unmarking

**Image Rotation**

* **Auto Rotate Images:** `Ctrl+R`
* **Rotate Clockwise:** `R`
* **Rotate Counterclockwise:** `Shift+R`
* **Rotate 180°:** `Alt+R`
* **Accept Rotation Suggestion:** `Y`
* **Decline Rotation Suggestion:** `N`
* **Accept All Rotations Suggestions:** `Shift+Y`
* **Decline All Rotations Suggestions:** `Shift+N` 

**Image Analysis and Organization**

* **Show Images in Folders:** `F`
* **Group by Similarity:** `S`
* **Show Thumbnails:** `T`
* **Analyze Similarity:** `Ctrl+S`
* **Detect Blurriness:** `Ctrl+B`
* **Auto Rotate Images:** `Ctrl+R`

**Image Deletion**

* **Mark for Deletion:** `D`
* **Commit Marked Deletions:** `Shift+D`
* **Clear Marked Deletions:** `Alt+D`

**Rating**

* **Rate 0-5:**
  * Windows/Linux: `Ctrl+0` through `Ctrl+5`
  * macOS: `Cmd+0` through `Cmd+5`

**Application Settings**

* **Manage Cache:** `F9`

**Help**

* **About:** `F12`

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

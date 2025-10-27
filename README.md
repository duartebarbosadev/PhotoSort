
# PhotoSort: Photo Library Culler

<div align="center">
  <img src="assets/main-window-screenshot.png" alt="PhotoSort Main Window" />
</div>

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

- **Update Notifications**: Automatically checks for new releases and notifies users when updates are available, with direct download links.
- **Performance Modes**: Configurable threading system (Settings → Preferences, `F10`) to balance between system responsiveness (Balanced) and maximum processing speed (Performance).
- **Metadata Display**: Shows EXIF information (camera model, exposure settings, etc.).

## Getting Started

### Installation & Running

If you prefer a ready-to-run binary, pre-built executables are published on the project's GitHub Releases page. Download the release, then run the downloaded executable directly — no Python virtual environment required. You can find releases here:
https://github.com/duartebarbosadev/photosort/releases

1. **Clone the repository:**

   ```bash
   git clone https://github.com/duartebarbosadev/photosort
   cd PhotoSort
   ```

2. **Install system dependencies (macOS only):**

   On macOS, the `pyexiv2` library requires certain system libraries to be installed via Homebrew:

   ```bash
   brew install brotli inih gettext
   ```

   > **Note**: These dependencies are only required on macOS. Windows and Linux users can skip this step.

3. **Create a virtual environment (recommended):**

   ```bash
   python -m venv venv
   source venv/bin/activate  # On Windows: venv\Scripts\activate
   ```

4. **Install dependencies:**
   Choose the appropriate requirements file based on your hardware:

   #### For CPU (Default)
   ```bash
   pip install -r requirements.txt
   ```

   #### For NVIDIA CUDA GPU Acceleration
   ```bash
   pip install -r requirements-cuda.txt
   ```

   > **Note**: The CUDA version requires NVIDIA CUDA Toolkit and cuDNN to be installed on your system.
   
   > **Note**: These packages are mutually exclusive. If switching between CPU and CUDA versions, create separate virtual environments or uninstall the current onnx package before installing the other.

5. **Run the application:**
   The main entry point is [`src/main.py`](src/main.py).

  ```
  python -m src.main [--folder FOLDER_PATH] [--clear-cache]

  # Examples:
  #   Open a specific folder at startup:
  #       python -m src.main --folder "C:/Users/MyUser/Pictures"
  #   Clear all caches before starting:
  #       python -m src.main --clear-cache
  #   Open folder and clear caches (useful for development):
  #       python -m src.main --folder "C:/Users/MyUser/Pictures" --clear-cache
  ```

### AI Model Setup (Required for Rotation Detection)

To use the **Auto Rotate Images** feature (`Ctrl+R`), you need to download the pre-trained orientation detection model.

1. **Create a `models` directory** in the root of the project.
2. **Download the model file**:
   * **Link**: [Download orientation_model_v2_0.9882.onnx from Hugging Face](https://huggingface.co/DuarteBarbosa/deep-image-orientation-detection/tree/main)
3. **Place the downloaded model file inside the `models` directory.**

The application will automatically detect and load the model when you use the rotation detection feature.

### Experimental: AI Best Shot Ranking

PhotoSort includes an experimental multi-model pipeline (see
`core/ai/best_photo_selector.py`) that chains together Hugging Face models to
pick the best frame inside a stack of similar photos.

**⚠️ Important: Models are NOT included in the repository**

Due to size (~500MB–600MB) and licensing restrictions, the required models must be
downloaded separately. The application will automatically detect missing models and
guide you through the download process via a dialog.

**Required Models** (all must be placed in the `models/` directory):

1. **Face detector** – Download the ONNX package from
   [`qualcomm/MediaPipe-Face-Detection`](https://huggingface.co/qualcomm/MediaPipe-Face-Detection)
   and extract `model.onnx` to `models/job_*/model.onnx` (or create a folder like
   `models/MediaPipe-Face-Detection_FaceDetector_float/model.onnx`).

2. **Eye-state classifier** – Download all files from
   [`MichalMlodawski/open-closed-eye-classification-mobilev2`](https://huggingface.co/MichalMlodawski/open-closed-eye-classification-mobilev2)
   and place them under `models/open-closed-eye-classification-mobilev2/`.

3. **Aesthetic predictor** – Download all files from
   [`shunk031/aesthetics-predictor-v2-sac-logos-ava1-l14-linearMSE`](https://huggingface.co/shunk031/aesthetics-predictor-v2-sac-logos-ava1-l14-linearMSE)
   and place them under `models/aesthetic_predictor/`. This bundle includes the
   CLIP vision backbone plus the regression head used to score each frame.

Once the models are in place you can rank a folder (or the members of a
similarity cluster) from a Python shell:

```python
from core.ai.best_photo_selector import BestPhotoSelector

selector = BestPhotoSelector()
results = selector.rank_directory("/path/to/similar/stack")

best = results[0]
print("Best image:", best.image_path)
print("Composite:", best.composite_score)
print("Metrics:", best.metrics)
```

The selector yields a sorted `BestShotResult` list that includes the composite
score plus sub-metrics (eyes open probability, technical sharpness, framing
score, etc.). These scores can be fed back into the UI to highlight the best
candidate inside a similarity cluster.

In the desktop app, run **View → Analyze Similarity** first, then trigger
**View → Analyze Best Shots** to execute the same ranking pipeline on every
cluster. PhotoSort will label the best image per group (and expose the metrics
in the item tooltip) so you can cull stacks faster.

### Exporting Logs

To capture detailed logs for debugging, you can enable file logging by setting an environment variable before running the application.

* **macOS/Linux**:
  ```bash
  export PHOTOSORT_ENABLE_FILE_LOGGING=true
  python -m src.main
  ```
* **Windows (Command Prompt)**:
  ```bash
  set PHOTOSORT_ENABLE_FILE_LOGGING=true
  python -m src.main
  ```
* **Windows (PowerShell)**:
  ```powershell
  $env:PHOTOSORT_ENABLE_FILE_LOGGING="true"
  python -m src.main
  ```

Logs will be saved to `~/.photosort_logs/photosort_app.log`.

## **Keyboard Shortcuts**:

![PhotoSort Keyboard Shortcuts](assets/keyboard-layout.png)

> **Note:** For the "Focus on image (1-9)" actions, if multiple images are highlighted, pressing `1` will show the first highlighted image, `2` the second, and so on.

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


# PhotoSort: Photo Library Culler

<div align="center">
  <img src="assets/main-window-screenshot.png" alt="PhotoSort Main Window" />
</div>

PhotoSort is a fast, powerful desktop application for managing large photo libraries, making it easier than ever to sort, cull, and organize your photos.

**Use this at your personal risk. Always use backups.**

## Key Features

* **Intelligent Culling Tools**:
  * **Ratings & Labels**: Assign star ratings for quick categorization.
  * **Blur Detection**: Automatically identify and flag blurry photos.
  * **AI Orientation Detection**: Auto-detects the correct image orientation using a fine-tuned EfficientNetV2 ONNX model and proposes rotations.
  * **Similarity Analysis**: Group visually similar images to easily spot duplicates or near-duplicates.
  * **Pick Best (Local AI Ranking)**: Score each similarity cluster locally using technical quality checks plus an aesthetic model, with preview-cache reuse and RAW support.
  * **Fast Processing**: Intensive operations (scanning, thumbnailing, analysis) run once in batch to ensure fast image scrolling.
  * **Optimized Image Handling**: Supports a wide range of formats, including various RAW types, with efficient caching.
  * **Video Browsing Support**: Scan and browse common video formats with playback and first-frame thumbnails (analysis and ratings remain image-only).
  * **Intelligent Image Rotation**: Smart rotation system that automatically tries lossless metadata rotation first, with optional fallback to pixel rotation when needed.
  * **AI Best-Shot Ranking**: Send stacks to an OpenAI-compatible vision model (e.g. Qwen3-VL) to pick the keeper frame automatically.
  * **AI Star Ratings**: Ask the configured AI engine to score individual photos with 1–5 stars.
- **Performance Modes**: Configurable threading system (Settings → Preferences, `F10`) to balance between system responsiveness (Balanced) and maximum processing speed (Performance).
- **Metadata Display**: Shows EXIF information (camera model, exposure settings, etc.).

## AI Models Used

PhotoSort uses a mix of local models and configurable external AI endpoints:

- **Similarity analysis**: [`facebook/dinov2-small`](https://huggingface.co/facebook/dinov2-small) by default, with `facebook/dinov2-base` and a configurable grouping threshold available in Preferences, for visual image embeddings and crop-aware similarity clustering.
- **Pick Best local aesthetic scoring**: [`cafeai/cafe_aesthetic`](https://huggingface.co/cafeai/cafe_aesthetic) via `transformers`.
- **Pick Best local technical scoring**: OpenCV face/eye cascades plus MediaPipe Face Mesh for blur / eye-state / face-quality heuristics.
- **Auto-rotation**: the local ONNX orientation classifier from [deep-image-orientation-detection](https://github.com/duartebarbosadev/deep-image-orientation-detection), a fine-tuned EfficientNetV2 model that predicts whether an image should stay at `0°` or be corrected by `90°`, `180°`, or `270°`. PhotoSort loads `orientation_model*.onnx` files from the project `models/` directory.
- **AI Best Shot ranking and AI star ratings**: any **OpenAI-compatible vision model** you configure in Preferences.
  Default example in app settings: `Qwen3-VL-30B-A3B-Instruct-MLX-4bit` at `http://127.0.0.1:8000/v1`.

## Getting Started

### Installation & Running

You can download the latest prebuilt binaries from the [Releases page](https://github.com/duartebarbosadev/photosort/releases):

- **Windows**: Download the `.exe` file and run it directly (no separate installer required).
- **macOS**: Download the `.dmg`, open it, then drag **PhotoSort** to your **Applications** folder.
- **Windows**: Download the `.exe` file and run it directly (no separate installer required).
- **macOS**: Download the `.dmg`, open it, then drag **PhotoSort** to your **Applications** folder.
If you prefer to build from source or want to contribute:

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

> **Python version:** PhotoSort is tested on Python 3.12. Newer interpreters may work, but 3.12 is the supported target for now.
> This matters for `mediapipe`, which is installed as part of the default requirements used by Pick Best local scoring.

3. **Create a Python 3.12 virtual environment (recommended):**

   ```bash
   python3.12 -m venv venv
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
  python -m src.main [--folder FOLDER_PATH | --last-folder] [--clear-cache]

  # Examples:
  #   Open a specific folder at startup:
  #       python -m src.main --folder "C:/Users/MyUser/Pictures"
  #   Open the most recent folder at startup:
  #       python -m src.main --last-folder
  #   Clear all caches before starting:
  #       python -m src.main --clear-cache
  #   Open folder and clear caches (useful for development):
  #       python -m src.main --folder "C:/Users/MyUser/Pictures" --clear-cache
  ```

### AI Model Setup

#### Rotation Detection Model

To use the **Auto Rotate Images** feature (`Ctrl+R`), download the pre-trained ONNX model used by PhotoSort's rotation detector.

PhotoSort integrates the model published in [deep-image-orientation-detection](https://github.com/duartebarbosadev/deep-image-orientation-detection), which is trained to classify images into the four uprightness classes `0°`, `90°`, `180°`, and `270°`.

1. In PhotoSort, open **About → Models Folder**.
2. **Download the model file**:
   * **Link**: [Open the deep-image-orientation-detection releases page](https://github.com/duartebarbosadev/deep-image-orientation-detection/releases)
   * Download the latest `orientation_model*.onnx` asset.
3. **Place the downloaded model file inside the folder opened by PhotoSort.**.

The application will automatically detect and load the newest matching `orientation_model*.onnx` file when you use the rotation detection feature, so versioned filenames such as `orientation_model_v2_0.9882.onnx` work without being renamed.

#### Local Model Downloads

The following local models are downloaded or installed on first use:

- `facebook/dinov2-small` or `facebook/dinov2-base` for similarity embeddings. PhotoSort asks before downloading the selected model, then reuses the local Hugging Face cache for offline runs. Preferences also include a similarity grouping threshold.
- `cafeai/cafe_aesthetic` for Pick Best local aesthetic scoring

If you are running offline, warm these models once while online first so they are present in your local Hugging Face cache.

#### AI Best Shot Ranking & Ratings

PhotoSort relies on an OpenAI-compatible vision model to rank
similar shots and request AI star ratings. Configure the endpoint under
**Preferences → AI Rating Engine** (`F10`) by providing the API key (optional for
local deployments), base URL, model name, prompt templates, max tokens, timeout,
and concurrency. Any server that implements the OpenAI Chat Completions API with
vision support will work.

The app default is configured for a local OpenAI-compatible server using:

- Model: `Qwen3-VL-30B-A3B-Instruct-MLX-4bit`
- Base URL: `http://127.0.0.1:8000/v1`

**Using the results**  
- **Similarity stacks**: After running **View → Analyze Similarity**, launch
  **View → Analyze Best Shots** (`Ctrl+B`) to automatically pick a winner for every cluster
  (metrics appear in the UI tooltips). For ad-hoc comparisons select a handful of
  images and trigger **View → Analyze Best Shots (Selected)** (`Alt+B`) to rank
  just that group.
- **Pick Best**: After running similarity, use the **Pick Best** workflow step to score each cluster locally using cached previews plus aesthetic and technical analysis.
- **AI star ratings**: To score every visible image, run **View → AI Rate Images**
  (`Ctrl+A`). The ratings are stored in your XMP sidecars/metadata cache so
  they survive reloads, and you can filter the library using the standard rating
  controls.

### Exporting Logs

To capture detailed logs for debugging, you can enable file logging by setting an environment variable before running the application.

* **macOS/Linux**:
  ```bash
  export PHOTOSORT_ENABLE_FILE_LOGGING=true
  export PHOTOSORT_LOG_LEVEL=DEBUG
  python -m src.main
  ```
* **Windows (Command Prompt)**:
  ```bash
  set PHOTOSORT_ENABLE_FILE_LOGGING=true
  set PHOTOSORT_LOG_LEVEL=DEBUG
  python -m src.main
  ```
* **Windows (PowerShell)**:
  ```powershell
  $env:PHOTOSORT_ENABLE_FILE_LOGGING="true"
  $env:PHOTOSORT_LOG_LEVEL="DEBUG"
  python -m src.main
  ```

Logs will be saved to `~/.photosort_logs/photosort_app.log`.

## **Keyboard Shortcuts**:

![PhotoSort Keyboard Shortcuts](assets/keyboard-layout.png)

> **Note:** For the "Focus on image (1-9)" actions, if multiple images are highlighted, pressing `1` will show the first highlighted image, `2` the second, and so on.


## Security Notes

`diskcache` (used for our thumbnail/preview/EXIF/rating/analysis caches) has
an open advisory (PYSEC-2026-2447) for unsafe pickle deserialization. We're
intentionally ignoring it in CI for now — there's no patched `diskcache`
release. It's a non-issue in practice anyway:
exploiting it needs write access to your own private cache folder, which
means you'd already have code execution as yourself. We'll revisit if a real
fix or safe replacement shows up.

## Contributing

Contributions are welcome! Please feel free to submit a pull request or open an issue for bugs, feature requests, or suggestions.

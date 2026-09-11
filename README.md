<h1 align="center">PhotoSort</h1>

<p align="center">
  <strong>Choose which photos to keep before you start editing.</strong>
</p>

<p align="center">
  <a href="https://www.python.org/downloads/"><img src="https://img.shields.io/badge/Python-3.14-3776AB?style=flat-square&amp;logo=python&amp;logoColor=white" alt="Python 3.14"></a>
  <a href="https://www.riverbankcomputing.com/software/pyqt/"><img src="https://img.shields.io/badge/PyQt-6-41CD52?style=flat-square&amp;logo=qt&amp;logoColor=white" alt="PyQt 6"></a>
  <a href="https://github.com/duartebarbosadev/PhotoSort/releases"><img src="https://img.shields.io/github/v/release/duartebarbosadev/PhotoSort?style=flat-square&amp;label=release" alt="Latest release"></a>
  <a href="https://github.com/duartebarbosadev/PhotoSort/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/duartebarbosadev/PhotoSort/ci.yml?branch=main&amp;style=flat-square&amp;label=build" alt="Build status"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-Apache%202.0-blue?style=flat-square" alt="License: Apache 2.0"></a>
</p>

PhotoSort helps you sort through a folder of photos and decide which ones to
keep. You can find blurry shots and duplicates, compare similar photos side by
side, and organize your favorites into folders. Mark the photos you don’t want
as you go, then review them before moving anything to Trash.

[![PhotoSort showing a night photograph in Cull](assets/main-window-screenshot.webp)](assets/main-window-screenshot.webp)

<p align="center">
  <a href="https://github.com/duartebarbosadev/PhotoSort/releases">Download</a> ·
  <a href="#supported-files">Supported files</a> ·
  <a href="#choose-your-workflow">Workflows</a> ·
  <a href="#useful-shortcuts">Shortcuts</a> ·
  <a href="#ai-usage">AI Usage</a>
</p>

## Download

Download PhotoSort from the [Releases page](https://github.com/duartebarbosadev/PhotoSort/releases).

## Supported files

PhotoSort opens JPEG, PNG, TIFF, WebP, and many camera RAW formats. Star ratings
and rotations are saved directly to EXIF and XMP metadata so they carry over to
other photo editors. You can also browse and play common video formats, but
ratings and analysis only work with photos.

## Choose your workflow

There are five steps along the bottom of the window. Use whichever ones you
need. You don't have to go through them all or follow a particular order.

| Step | What it's for |
| --- | --- |
| **Organize** | Arrange photos into folders. |
| **Easy Delete** | Find duplicates, blurry shots, and exposure problems. |
| **Fix Rotation** | Correct sideways or upside-down photos. |
| **Pick Best** | Choose favorites from similar shots. |
| **Cull** | Review photos manually and choose what to keep. |

### Organize

Arrange photos into folders and check the result before moving anything.
RAW+JPEG pairs and XMP sidecars can move together.

[![Organize previewing USA, Porto, and Australia folders](assets/organize-screenshot.webp)](assets/organize-screenshot.webp)

### Easy Delete

PhotoSort flags duplicates, blurry shots, and photos that may be too dark or
bright. Compare them and decide which ones to keep.

[![Easy Delete comparing a softer photo with a sharper version](assets/easy-delete-screenshot.webp)](assets/easy-delete-screenshot.webp)

### Fix Rotation

Check the suggested rotation next to the original. Adjust it if needed, then
apply the changes.

[![Fix Rotation showing the original and suggested correction](assets/fix-rotation-screenshot.webp)](assets/fix-rotation-screenshot.webp)

### Pick Best

Compare similar photos with suggestions based on image quality and faces.
Choose your favorites, or keep them all.

[![Pick Best comparing similar photos](assets/pick-best-screenshot.webp)](assets/pick-best-screenshot.webp)

### Cull

Go through your photos at your own pace. Compare shots side by side, give your
favorites a star rating, and mark the ones you don't want. Review your marks
before sending anything to Trash.

[![Cull showing a night photograph for review](assets/main-window-screenshot.webp)](assets/main-window-screenshot.webp)

### Useful shortcuts

See the [complete keyboard guide](docs/keyboard-shortcuts.md) for separate maps
and shortcut tables for all five workflows.

Here are the most important shortcuts you should know:

| What you want to do | Shortcut |
| --- | --- |
| Browse photos | Up / Down arrows |
| Split view with multiple photos | Shift + Up / Down arrows |
| Highlight a photo to quickly compare (when multiple are selected) | 1, 2, 3... |
| Mark or unmark a photo for deletion | D |
| Unmark all photos | Alt+D |
| Review and apply your changes | Shift+Enter |
| Send selected photos to Trash | Delete / Backspace |
| Give a star rating, or clear it with 0 | Ctrl+0 to Ctrl+5 |
| Show photo details | I |
| Fit the photo / show actual size | 0 / A |
| Show one photo / compare side by side | F1 / F2 |
| Open Preferences | F10 |

## AI Usage

PhotoSort runs all AI models **100% locally on your computer**. No photos, data,
or embeddings are ever sent to external servers or cloud APIs, and no accounts or
API keys are required. You can also browse, organize, and cull photos manually
without using any AI features.

### Where AI is used

- **Subject & similarity grouping (Cull & Pick Best):** Uses Meta's **DINOv2** (`facebook/dinov2-small`) vision transformer to cluster bursts and similar scenes based on visual content. You can also select `facebook/dinov2-base` in Preferences.
- **Aesthetic quality scoring (Pick Best):** Uses **BeIT Aesthetic** (`cafeai/cafe_aesthetic`) to evaluate composition and lighting, alongside **MediaPipe / OpenCV** for face and open-eye detection.
- **Orientation correction (Fix Rotation):** Uses an ONNX neural network ([deep-image-orientation-detection](https://github.com/duartebarbosadev/deep-image-orientation-detection)) to detect upside-down or sideways photos.
- *(Note: Duplicate detection and blur analysis in Easy Delete use fast, traditional image processing algorithms rather than neural networks).*

### Models & offline use

- **Automatic download:** Models are downloaded automatically on first use and cached locally for future runs.
- **Offline ready:** Once downloaded, all models run completely offline. If you plan to work without internet, run the features once beforehand to cache the models.
- **Hardware acceleration:** Neural networks automatically leverage Apple Silicon (MPS) or NVIDIA (CUDA) when available, falling back to CPU.
- **Manual rotation model setup:** The rotation model can also be installed manually by downloading `orientation_model*.onnx` from [the model's releases page](https://github.com/duartebarbosadev/deep-image-orientation-detection/releases) and placing it in **Help → About → Models Folder**.

## Questions or feedback?

[Open an issue](https://github.com/duartebarbosadev/PhotoSort/issues) if something
isn't working, a step is confusing, or you'd like a feature. For bugs, include
your PhotoSort version, operating system, file formats, and what you did before
the problem happened. A small sample photo helps if you can share one.

## Running from source

You only need these instructions if you want to run the development version
or work on the code. You'll need **Python 3.14.x**.

```bash
git clone https://github.com/duartebarbosadev/PhotoSort.git
cd PhotoSort
python3.14 -m venv venv
```

Activate the environment:

```bash
# macOS / Linux
source venv/bin/activate

# Windows Command Prompt
venv\Scripts\activate.bat
```

On macOS, install the system libraries used by `pyexiv2`:

```bash
brew install brotli inih gettext
```

Install dependencies and start PhotoSort:

```bash
python -m pip install -r requirements.txt
python -m src.main
```

For an NVIDIA CUDA environment, use `requirements-cuda.txt` instead. Use
separate environments for the CPU and CUDA variants of ONNX Runtime.

Useful launch options:

```bash
python -m src.main --folder "/path/to/photos"
python -m src.main --last-folder
python -m src.main --clear-cache
```

`--clear-cache` deletes saved previews and other cached data. PhotoSort will
build them again when needed. `--clear-models` deletes downloaded models, so
you will need to download them again to use those features.

### Debug logs

Set `PHOTOSORT_ENABLE_FILE_LOGGING=true` and `PHOTOSORT_LOG_LEVEL=DEBUG` before
launching. For example, on macOS/Linux:

```bash
PHOTOSORT_ENABLE_FILE_LOGGING=true PHOTOSORT_LOG_LEVEL=DEBUG python -m src.main
```

On Windows PowerShell:

```powershell
$env:PHOTOSORT_ENABLE_FILE_LOGGING="true"
$env:PHOTOSORT_LOG_LEVEL="DEBUG"
python -m src.main
```

Logs are saved as `photosort_app.log` in:

- macOS: `~/Library/Logs/PhotoSort/`
- Windows: `%LOCALAPPDATA%\PhotoSort\Logs\`
- Linux: `$XDG_STATE_HOME/PhotoSort/`, or `~/.local/state/PhotoSort/` by default.

Review logs for personal paths and metadata before sharing them.

## Contributing

Bug reports, usability feedback, and pull requests are welcome. See
[DEVELOPER_GUIDE.md](DEVELOPER_GUIDE.md) for architecture and
[AGENTS.md](AGENTS.md) for development and verification guidelines.

```bash
python -m pip install -r requirements-dev.txt
python -m pytest
```

PhotoSort is licensed under [Apache 2.0](LICENSE).

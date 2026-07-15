# PhotoSort

PhotoSort is a fast desktop photo-culling and organizing tool for large photo
libraries. It is designed to be the step before editing.

<div align="center">
  <img src="assets/main-window-screenshot.png" alt="PhotoSort Main Window" />
</div>

**Use this at your personal risk. Always use backups.**

## What PhotoSort does

PhotoSort is a good fit if you want to move quickly through a folder, compare
similar frames, and make fast culling decisions with its keyboard based controls. It is focused on a fast review workflow rather than being a full Lightroom replacement.

PhotoSort's main features include ultra fast image browsing (even RAW),
keyboard-driven review, side-by-side comparison, ratings and metadata, visual
similarity groups, blur detection, and video browsing. The workflow is divided
into focused steps so you can organize, review, and cull a folder without
committing every decision immediately.

## Quick start

1. Download the latest release from the [GitHub Releases
   page](https://github.com/duartebarbosadev/PhotoSort/releases).
2. Choose the normal CPU build unless you already know that you want the NVIDIA
   CUDA build; see [Downloads](#downloads).
3. Start PhotoSort and open a folder containing photos or videos.
4. Review the workflow steps in order, or skip directly to the step you need.

### Windows

Download and extract `PhotoSort-Windows-x64.zip`, open the extracted folder,
and run `PhotoSort.exe`. There is no separate installer.

### macOS

Download `PhotoSort-macOS-AppleSilicon.dmg`, open it, and drag **PhotoSort** to
the **Applications** folder. This release is for Apple Silicon Macs.

Release builds are not signed. If macOS warns that it cannot
verify the developer or that the app cannot be opened:

1. Close the warning and make sure **PhotoSort** is in **Applications**.
2. Control-click (or right-click) **PhotoSort**, choose **Open**, then choose
   **Open** again in the confirmation dialog.
3. If the confirmation option is not shown, open **System Settings → Privacy &
   Security**, scroll to the Security section, and click **Open Anyway** for
   PhotoSort. Confirm with your Mac password or Touch ID if prompted.

You normally only need to approve the application once. Only do this for a
release downloaded from the official [GitHub Releases
page](https://github.com/duartebarbosadev/PhotoSort/releases).

## The basic culling workflow

PhotoSort does not immediately delete an image when you mark it. Decisions are
staged so you can review them and change your mind.

### 1. Organize

Plan a new folder structure using the current folder, similarity, face, date,
location, or mixed grouping modes. Review the proposed changes in the **After**
tree, rename groups if needed, and apply them explicitly. PhotoSort can move
RAW+JPEG pairs and XMP sidecars together when companion-file handling is
enabled. The run also writes a `grouping-manifest.json` in the output folder.

Use the grouping and review tools to build a Keep or Favorites folder, and
adjust the proposed destinations before applying the changes.

### 2. Easy Delete

Find obvious rejects such as blurry, very dark, very bright, or near-duplicate
images. Review the suggestions and stage the images you want to remove. Nothing
is sent to the Trash until you confirm later in Cull.

### 3. Fix Rotation

PhotoSort can detect images whose orientation appears wrong and show a proposed
correction. Accept only the changes you want. It tries a lossless metadata
rotation first and uses pixel rotation as a fallback when necessary.

### 4. Pick Best

Compare similar shots and choose the keeper in each group. The local Pick Best
workflow combines technical checks with an aesthetic model; choices are still
reviewable and staged as Keep or Trash decisions.

### 5. Cull

Review every staged decision in one place. Only after you confirm are selected
files moved to the operating system's Trash or Recycle Bin. The application does
not permanently erase them itself, so recovery is normally handled through the
system Trash/Recycle Bin.

## Everyday controls

- **Up/Down** (or **J/K**): move through images.
- **Left/Right** (or **H/L**): move within a similarity group.
- **D**: mark the current selection for deletion.
- **Shift+D**: commit marked deletions and move them to the Trash/Recycle Bin.
- **Alt+D**: clear deletion marks before committing.
- **Ctrl-click**: select multiple images for comparison or batch actions.
- **Shift+Up/Down**: compare nearby images side by side when available.
- **Ctrl+S**: analyze visually similar images.
- **Ctrl+B**: analyze best shots in similarity groups.
- **Alt+B**: analyze only the selected images as a best-shot set.
- **Ctrl+R**: detect incorrect image orientation.
- **Ctrl+A**: request AI star ratings for visible images.
- **F10**: open Preferences.

The workflow pages show their own relevant shortcuts. The complete shortcut map
is also available here:

![PhotoSort Keyboard Shortcuts](assets/keyboard-layout.png)

For “Focus on image (1–9)”, the number selects that position among the images
currently highlighted.

## Ratings and metadata

You can assign 1–5 star ratings, filter by rating, and use ratings while
reviewing a library. Ratings are written to image metadata/XMP sidecars where
supported, so compatible applications such as Lightroom can read them.

Ratings are useful for marking keepers, filtering a library, and continuing the
workflow in another photo application. To place selected photos in a Keep or
Favorites folder, use the reviewed Organize workflow.

## AI-assisted features

The core culling workflow does not require an online service or a local large
language model. PhotoSort uses a mix of local machine-learning models and
optional AI services:

The following features use local, smaller models or conventional computer vision:

- **Similarity Analysis** groups visually similar images.
- **AI Orientation Detection** proposes 0°, 90°, 180°, or 270° corrections.
- **Pick Best local scoring** combines local aesthetic and technical checks.

AI Star Ratings and AI Best-Shot Ranking use an OpenAI-compatible vision model.
You can connect them to a paid API or to a local model server, but they are
optional and are not needed for browsing, organizing, similarity analysis, or
the standard culling workflow:

- **AI Star Ratings** scores individual photos from 1–5 stars.
- **AI Best-Shot Ranking** ranks a selected stack or every similarity group.

The local models are downloaded or installed on first use, with a confirmation
where applicable. Similarity uses
[`facebook/dinov2-small`](https://huggingface.co/facebook/dinov2-small) by
default; Pick Best local scoring uses
[`cafeai/cafe_aesthetic`](https://huggingface.co/cafeai/cafe_aesthetic). For
orientation detection, download an `orientation_model*.onnx` file from the
[deep-image-orientation-detection releases](https://github.com/duartebarbosadev/deep-image-orientation-detection/releases)
and place it in the Models Folder opened from **About → Models Folder**.

For LLM features, configure an OpenAI-compatible endpoint in **Preferences →
AI Rating Engine** (`F10`). A local server is optional; the example default is
`Qwen3-VL-30B-A3B-Instruct-MLX-4bit` at `http://127.0.0.1:8000/v1`.

## AI disclosure

AI was used extensively to help make PhotoSort exist. In my head, the choice
was between letting AI help me build this project or not doing it because I would have to spend months manually creating this. And also there are already tools that work "goodish" like lightroom to cull, or just the normal image preview, but I wanted to create something better. But even with AI it still took me months.
I am not going to spend a lot more time than I would have simply because
some people are against AI. AI is a tool, and I use it alongside my own
judgment, testing, and experience.

I also personally cull all my photos with PhotoSort, so I am comfortable with
the AI-assisted work that went into it.

## Building from source

This section is for contributors and users who want to run the development
version. Prebuilt releases do not require Python, Bash, or these dependencies.

PhotoSort source builds require Python 3.14.x. Other feature releases are not
supported until the native image and machine-learning dependencies have been
validated against them.

```bash
git clone https://github.com/duartebarbosadev/PhotoSort
cd PhotoSort
python3.14 -m venv venv
source venv/bin/activate  # Windows: venv\Scripts\activate
pip install -r requirements.txt
python -m src.main
```

On macOS, install the system libraries required by `pyexiv2` first:

```bash
brew install brotli inih gettext
```

For a source installation with NVIDIA CUDA acceleration, install
`requirements-cuda.txt` instead of `requirements.txt`. Do not install both
ONNX Runtime variants in the same environment; use separate virtual
environments when switching.

Useful development commands:

```bash
python -m src.main --folder "C:/Users/MyUser/Pictures"
python -m src.main --last-folder
python -m src.main --clear-cache
```

See [DEVELOPER_GUIDE.md](DEVELOPER_GUIDE.md) for architecture and contribution
guidance, and [PACKAGING.md](PACKAGING.md) for desktop release builds.

## Logs

Enable file logging when reporting a problem:

```bash
PHOTOSORT_ENABLE_FILE_LOGGING=true PHOTOSORT_LOG_LEVEL=DEBUG python -m src.main
```

On Windows, set the same variables in Command Prompt or PowerShell before
starting PhotoSort. Logs are saved to `~/.photosort_logs/photosort_app.log`.

## Contributing

Contributions, bug reports, feature requests, and workflow feedback are
welcome. Please open an issue or submit a pull request.

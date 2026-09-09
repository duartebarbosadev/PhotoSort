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

PhotoSort is a fast, powerful desktop application for managing large photo
libraries, making it easier than ever to sort, cull, and organize your photos.
It helps you go through your shots, compare similar photos, and decide what to
keep. You can give your favorites a star rating, sort photos into folders, and
mark the ones you don't want for deletion—delete them whenever you're ready, or
simply keep them if you change your mind.

[![PhotoSort showing a night photograph in Cull](assets/main-window-screenshot.webp)](assets/main-window-screenshot.png)

<p align="center">
  <a href="https://github.com/duartebarbosadev/PhotoSort/releases">Download PhotoSort</a> ·
  <a href="#your-first-cull">Get started</a> ·
  <a href="#choose-your-workflow">The five steps</a> ·
  <a href="#can-i-undo-it">Can I undo it?</a>
</p>

## Is it for me?

If you've come home with hundreds of photos and want to pick the good ones
before editing, that's what PhotoSort is for. You can:

- Browse with the keyboard and quickly mark photos you don't want.
- Compare similar shots side by side to check focus or expressions.
- Get help finding duplicates, blurry photos, and photos that need rotating.
- Give photos star ratings and organize them into folders.

PhotoSort isn't a photo editor or a full library catalog. It opens common image
formats and many RAW formats. You can also browse and play common video formats,
but ratings and analysis only work with photos.

**You don't need a subscription, API key, or local LLM.** The AI features run on
your computer. Some need a model download the first time you use them, but you
can browse, compare, rate, and cull photos without setting any of that up.

## Download and open

Download PhotoSort from the [Releases page](https://github.com/duartebarbosadev/PhotoSort/releases).
You don't need to install Python or run any commands.

| Your computer | Download | How to open it |
| --- | --- | --- |
| Windows | `PhotoSort-Windows-x64.zip` | Extract the ZIP and run `PhotoSort.exe` from the extracted folder. |
| Windows with an NVIDIA GPU | `PhotoSort-Windows-x64-CUDA.zip` | Extract the ZIP and run `PhotoSort.exe`. |
| Apple Silicon Mac | `PhotoSort-macOS-AppleSilicon.dmg` | Open the DMG and drag PhotoSort to Applications. |

If you're not sure which Windows download to choose, **use the regular one**.
The CUDA version can make some analysis faster on compatible NVIDIA GPUs.
Both versions have the same features. Check the release notes for requirements
and download names. On Linux, you can [run from source](#running-from-source).

The downloads aren't signed yet, so your computer may show a warning when you
open them. Use the official release page linked above. If you'd like to check a
download with VirusTotal, scan the file you downloaded. Not every release has
been scanned. Some releases also include a `.sha256` file to check that the
download is complete and matches the published file. That isn't a malware scan.

## Your first cull

Try a copy of a small folder first so you can get a feel for the app. Keep
backups of your originals.

1. Open PhotoSort and choose **File → Open Folder**.
2. Click **Cull** at the bottom. You can start here and skip the other steps.
3. Use **Up/Down** to browse. Press **D** to mark a photo for deletion. Press it
   again to unmark it. The file stays where it is until you confirm deletion.
4. Give your favorites **5 stars** with the stars below the photo, or press
   **Ctrl+5** (**⌘5** on macOS). You can then filter by rating to find them again.
5. When you're ready, press **Shift+Enter**. Check the list of changes and
   confirm which photos you want to send to the Trash or Recycle Bin.

Changed your mind? **Alt+D** (**⌥D** on macOS) clears the deletion marks.

**Delete/Backspace works differently from D.** It asks to send the selected
photos to the Trash straight away, instead of marking them for later.

### Compare similar shots

Select a few photos in the browser to see them side by side. Click **Compare**
above the photo, then **Sync** to zoom and move around them together. This is
useful for checking focus or small differences between shots. Click **Single**
to go back to one photo.

## Choose your workflow

There are five steps along the bottom of the window. Use whichever ones you
need. You don't have to go through them all or follow a particular order. If
you leave a step with changes you haven't applied, PhotoSort asks what you want
to do with them.

| Step | What it's for |
| --- | --- |
| **Organize** | Sort photos into folders. See the current layout in **Before** and your changes in **After** before moving anything. |
| **Easy Delete** | Look through suggestions for duplicates, blur, and photos that are very dark or bright. |
| **Fix Rotation** | Find sideways or upside down photos and check the suggested correction. |
| **Pick Best** | Compare similar shots and choose which ones to keep. You can keep more than one, or all of them. |
| **Cull** | Browse your photos, compare them, add ratings, and mark rejects yourself. |

You make the final choice. A suggestion might not be right for intentional blur,
a silhouette, or a photo you simply like.

### Organize your favorites and keepers

One simple way to work is to give favorites **5 stars**, leave other keepers
unmarked, and press **D** for photos you don't want. Stars don't move files.

If you'd rather have a `Favorites` or `Keep` folder:

1. Open **Organize** and select **Current**.
2. Right click in the **After** tree and choose **Create folder**. Give it a
   name, such as `Favorites` or `Keep`.
3. Drag photos into that folder in the After tree.
4. Check the layout, then apply and confirm the changes.

There isn't a single shortcut that sends a photo to Favorites. You do that
through Organize.

If you shoot RAW+JPEG, choose **Yes, move companions** when asked to move
matching files together. This can include RAW/JPEG files with the same name
and XMP sidecars. Check the files and destinations before applying. Other
photo actions don't necessarily include the matching RAW or JPEG.

[![Organize previewing USA, Porto, and Australia folders](assets/organize-screenshot.webp)](assets/organize-screenshot.png)

### Easy Delete

PhotoSort points out possible rejects for you to check. Choose **Keep** or
**Trash** for each photo, then confirm. Here, a softer photo is being compared
with a sharper version.

[![Easy Delete comparing a softer photo with a sharper version](assets/easy-delete-screenshot.webp)](assets/easy-delete-screenshot.png)

### Fix Rotation

See the original next to the suggested rotation. You can change the rotation
if it doesn't look right, then confirm it before applying.

[![Fix Rotation showing the original and suggested correction](assets/fix-rotation-screenshot.webp)](assets/fix-rotation-screenshot.png)

### Pick Best

Compare the photos in each group and choose the ones you like. PhotoSort gives
you a suggestion based on its local scoring, but you're free to keep any of
them. This example compares three similar photos with different sharpness.

[![Pick Best comparing similar photos](assets/pick-best-screenshot.webp)](assets/pick-best-screenshot.png)

### Useful shortcuts

These shortcuts are for **Cull**. Each step has its own controls. To show them,
turn on **Settings → Preferences → Show shortcuts in the footer**.
On macOS, use **⌘** instead of Ctrl and **⌥** instead of Alt.

| What you want to do | Shortcut |
| --- | --- |
| Browse photos | Up / Down |
| Mark or unmark a photo for deletion | D |
| Clear deletion marks | Alt+D |
| Review and apply your changes | Shift+Enter |
| Send selected photos to Trash, after confirming | Delete / Backspace |
| Give a star rating, or clear it with 0 | Ctrl+0 to Ctrl+5 |
| Show photo details | I |
| Fit the photo / show actual size | 0 / A |
| Show one photo / compare side by side | F1 / F2 |
| Open Preferences | F10 |

Pressing **1 to 9** on its own focuses a photo in a comparison. It doesn't give
it a star rating.

## Can I undo it?

**Some choices can be changed before you apply them, but there isn't an Undo
button for everything.**

| What you do | What happens |
| --- | --- |
| Browse, compare, or run analysis | PhotoSort makes previews and suggestions. It doesn't replace your originals. |
| Mark a photo for deletion | Nothing is deleted yet. Unmark it or clear the marks to change your mind. |
| Edit a folder layout in Organize | The changes wait for you to apply them. You can still change the plan. |
| Confirm deletion | Files go to the system Trash or Recycle Bin. Restore them there if needed. PhotoSort doesn't have its own restore button. |
| Apply changes in Organize | Files are moved or renamed and folders are created. There isn't a button to undo the whole operation. |
| Give a star rating | The rating is saved to the photo's metadata where supported, without waiting for Apply. Choose another rating or 0 to change it. |
| Apply a rotation or rotate manually | PhotoSort changes orientation metadata where possible. Some formats need the image pixels rotated instead, so rotating back may not fully restore the original. |

Keep backups, especially before moving files or applying rotations.

PhotoSort writes ratings to XMP metadata where supported. Editors that read
those ratings can use them too. Try a few photos with your editor first, since
support varies by format and the editor may need to reload the metadata.

## Do I need to set up AI?

Only if you want to use the analysis features. They run on your computer, and
you don't need to send photos to a vision API. The older LLM setup has been
removed.

| Feature | What you need |
| --- | --- |
| Group similar photos | PhotoSort asks to download a model the first time, then reuses it. |
| Pick Best | Uses local quality checks and a model that may need downloading the first time. |
| Fix Rotation | Needs an `orientation_model*.onnx` file. The app explains what to do if it's missing. |

To install the rotation model yourself, download an `orientation_model*.onnx`
file from [the model's releases page](https://github.com/duartebarbosadev/deep-image-orientation-detection/releases).
In PhotoSort, open **Help → About → Models Folder** and put the file there.
You don't need to rename it.

Download any models you need before working offline. The first scan or analysis
of a large folder can take a while. Previews also use disk space. You can adjust
cache and performance settings in **Settings → Preferences**.

<details>
<summary>Which models does PhotoSort use?</summary>

- Similarity: `facebook/dinov2-small` by default. You can choose
  `facebook/dinov2-base` in Preferences.
- Pick Best aesthetic scoring: `cafeai/cafe_aesthetic`.
- Pick Best face and eye checks: OpenCV and MediaPipe.
- Rotation: the ONNX model from
  [deep-image-orientation-detection](https://github.com/duartebarbosadev/deep-image-orientation-detection).

</details>

## Questions or feedback?

[Open an issue](https://github.com/duartebarbosadev/PhotoSort/issues) if something
isn't working, a step is confusing, or you'd like a feature. For bugs, include
your PhotoSort version, operating system, file formats, and what you did before
the problem happened. A small sample photo helps if you can share one.

The screenshots here were taken on macOS with the project's test images.
Older releases may look different.

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

# PhotoSort Rust

An independent native photo workspace rebuilt from PhotoSort's functional behavior.
Rust owns the UI, catalog, image pipeline, local inference and file operations.
The original Python project is not a runtime dependency and was not modified.

Embedded RGB/grayscale and CMYK JPEG ICC profiles are normalized to sRGB once in the shared
image pipeline, so thumbnails, previews, original detail and standard-photo
analysis agree. Profile transforms are reused in a bounded cache. Untagged images
retain the sRGB assumption. Unsupported or invalid profiles report a preview
error rather than silently presenting an accurate-color claim.

In every workflow, the shared inspector and photo context menu can reveal the
clicked file in Finder, Explorer or the system file manager. Videos can open in
the default external player; both actions run off the UI thread.

## Open the app

Packaging creates `target/PhotoSort Rust.app`. The bundle contains its models,
ONNX Runtime and native image libraries. It can be moved independently of this
source directory. This is a locally signed development build, not a notarized release.

The current verified development bundle is
`.tools/native-vips/PhotoSort Rust.app`. It includes the libvips image backend and
the latest shared move, video and comparison fixes. From the repository root:

```sh
open ".tools/native-vips/PhotoSort Rust.app"
```

The older `.tools/native/PhotoSort Rust.app` is a previous build. The current
bundle's source/executable hashes, full test results and native interaction checks
are recorded in [the latest validation report](docs/diagnostics/settings-access-2026-10-08/README.md).
These checks cover this macOS development build; they do not establish Windows,
CUDA or DirectML runtime readiness.

To rebuild the current variant using the existing verified SIMD prefix, keep
compiler output temporary and clean it after packaging:

```sh
(
  build_dir="$(mktemp -d)"
  trap 'cargo clean --target-dir "$build_dir"' EXIT
  CARGO_TARGET_DIR="$build_dir" python3 scripts/package_macos.py \
    --native-imaging --native-prefix .tools/vips-simd \
    --output ".tools/native-vips/PhotoSort Rust.app"
)
```

On a fresh checkout, first follow the [native dependency setup](crates/imaging-vips/README.md)
and use the prefix it creates (`.tools/vips` by default). Model/runtime assets are
installed separately as described in [asset provenance](resources/ASSETS.md).
Running `scripts/package_macos.py` without these options builds the default image
backend into `target/PhotoSort Rust.app`.

From source with the default image backend:

```sh
brew install libraw exiv2 pkg-config
cargo run --release -- --folder /absolute/path/to/photos
```

`photosort --last-folder` reopens the most recent available library; `--help`
and `--version` run without opening the UI. `--diagnostics` prints a read-only JSON
report and exits: build capabilities, shared model SHA-256/size verification,
runtime-file presence, data/cache paths and the orientation override. It returns
failure when enabled AI model verification fails, its runtime file is missing,
or the configured orientation override is missing/empty.
It does not load the runtime, run inference or initialize graphics. Diagnostics
cannot be combined with folder-opening or clearing options.

`--check-runtime` goes further: it verifies the files, loads the native inference
engine and runs two CPU passes on an embedded licensed fixture through the shared
scene, aesthetic, face geometry, recognition and orientation services. Its JSON
reports each stage, counters and session reuse. Six sessions must be initialized
once and reused on the second pass. It uses one inference thread, never opens a
window or a photo library, and writes no analysis cache. It exits with failure in
minimal builds or when any prerequisite, inference contract or reuse check fails.
This checks native AI execution; it is not a graphics/video smoke test or an
accuracy/speed benchmark. A custom orientation override is exercised in place of
the default orientation model, whose inventory row is marked unselected.
Diagnostic JSON uses display text for paths containing non-UTF-8 bytes; actual
filesystem/model paths retain their original bytes.


`--clear-models` removes only known model copies in PhotoSort Rust's managed data
folder before normal startup. It preserves bundled resources, custom override
files outside that managed set, runtime libraries, photos and review state.
Unknown files/subdirectories are retained; linked paths are refused. If a managed
file changes during clearing, the command reports the incomplete operation and
exits instead of opening the app. Bundled copies remain usable without a download.
It can be combined with `--clear-cache` and a startup folder.

`--smoke-test` opens the native application and requests normal quit two seconds
after its first UI frame. `--smoke-delay-ms N` sets 200–60000 milliseconds and
requires `--smoke-test`. It supports the normal folder arguments; without one it
opens the welcome screen. The timer runs once, after keyboard/widget handling,
through the shared departure guard. Existing staged choices, unfinished reviews,
open plans or active file work can prevent automatic exit. Cancel keeps the app
open without repeating the timer. Use a disposable clean library for unattended
checks and an external timeout to detect hangs. This is a startup/lifecycle check,
not proof that a library finished scanning or that AI/codec checks passed.
It uses normal preferences, caches and bookmark/session persistence.

Menu offers recent folders, Settings,
the keyboard guide and About. Settings can hide optional middle steps and shortcut
hints; Organize and Cull remain available. Easy Delete blur, near-black and
overexposed thresholds apply to future analyses and preserve current reviews. Ctrl/Cmd+Alt+1–5 and Previous/Next use
the same workflow guards as clicking a step. Ctrl/Cmd+O opens a library,
Ctrl/Cmd+F focuses its search field (outside Pick Best), and F12 opens About.
In Cull, Alt+1/2/3 selects list/icons/grid; F and S toggle the folder and
similarity-group navigator. Text fields keep ordinary typing and selection keys;
modal dialogs and active file work keep keyboard ownership. The keyboard guide
scrolls in small windows. Cull navigation settings can jump
Up/Down (and Previous/Next) by exact star rating or skip single-photo similarity
groups. Left/Right cycles through visible photos, with an option to stay within
the current similarity group. Ctrl/Cmd includes marked photos and bypasses movement
filters; these preferences never hide photos. Rating jumps take priority over group
skipping. Unknown ratings remain unknown until shared metadata loading completes.

Conservative, Standard and Broad settings control future Cull/Pick Best same-subject analyses. Dense regions, face identities and complete-link verification share model evidence and cached pair results.

Settings controls how ratings and rotation are saved: embed metadata where the
format supports it, or use sidecars only. Existing XMP sidecars remain authoritative;
unsupported formats use a sidecar. The review plan names the destination. Embedded
edits modify and verify an independent temporary copy before replacing the photo;
pixels are not re-encoded. RAW/JPEG pairs sharing a sidecar must agree on its edits.

Settings also offers CPU, Automatic, Apple Core ML, NVIDIA CUDA and Windows
DirectML inference. CPU remains the default. Provider/model failures fall back to
CPU with an explanation; settings take effect on the next analysis. Apple model
preparation may take minutes on first use. See [native acceleration](docs/ACCELERATION.md)
for measured Apple results, setup requirements and unverified Windows/Linux limits.

Settings offers Balanced, Maximum and Custom AI thread allocation. Changes apply
to the next analysis or rotation run, preserving current reviews and cached
results. This sets model-inference threads, not a whole-app CPU usage limit.

Settings also shows shared cache usage and controls preview RAM, original-detail
RAM and disk-cache limits. These bound retained cache entries, not total app
memory: displayed textures, active decodes, metadata and models need extra space.
Disk limits are enforced periodically during writes and when settings are saved.
The disk budget includes previews, technical measurements, full-content hashes and AI evidence.
Unchanged photos reuse measurements and aesthetic scores after restart; analysis loads
pixels only when required evidence is missing. Sensitivity changes reclassify cached
measurements without decoding photos again. Set it to zero to disable persistent caching; active grouping may still need temporary disk space. Clearing reusable caches runs in the
image worker and preserves current reviews, decisions, bookmarks and model files.
Usage is a snapshot; use Refresh for a new reading. `--clear-cache` clears generated
disk previews and analysis evidence before normal startup without loading models.

Rust 1.92+ and C/C++ compilers are required. HEIF uses system ImageIO on macOS.
Linux/Windows full builds also require Exiv2 0.28+, LibRaw and libheif 1.23+ development libraries and a HEVC decoder plugin (for example libde265);
the shared core and native image pipeline have been tested on Linux aarch64 in an isolated Debian container. Linux desktop interaction/distribution and Windows native builds remain unverified. See [Linux validation](docs/diagnostics/linux-native-2026-10-05/README.md). `cargo run --no-default-features`
builds the raster-photo/manual-review version without native codecs or neural inference.
On macOS, videos play in the shared review canvas with native AVFoundation decoding,
seeking, volume and automatic pause on focus loss. Native video thumbnails need no
FFmpeg installation. Windows/Linux default builds use GStreamer 1.20+ through
its maintained Rust bindings for the same in-app controls and cached posters.
Install its development/runtime libraries plus playback, video conversion,
scaling, rotation, audio-output and relevant decoder plugins. On Debian, these are
`libgstreamer1.0-dev`, `libgstreamer-plugins-base1.0-dev`,
`gstreamer1.0-plugins-base`, `gstreamer1.0-plugins-good` and `gstreamer1.0-libav`.
Windows needs the matching official GStreamer MSVC runtime and development
installation on the build/runtime search paths. `--no-default-features --features
portable-video` enables playback without RAW/HEIF/AI dependencies. Builds without
`portable-video` retain FFmpeg-on-PATH posters and external playback.
The GStreamer sink retains at most one queued frame and prepares at most a
1280-pixel edge for display. Decoding, audio timing and rotation belong to the
library; PhotoSort owns selection, cancellation, UI state and cache reuse.
Real Linux codec tests are run separately from desktop, audible audio-sync and
Windows validation; availability of hardware decoder plugins is not a hardware
acceleration claim.

MOV/MP4/M4V capture dates come from embedded movie headers when EXIF/XMP dates
are absent. Sorting, calendar navigation and date organization share this metadata
without decoding frames. Invalid or missing dates retain filename/filesystem
fallbacks; movie times are interpreted as UTC and displayed in local time.

Reviewed file plans survive an interrupted run. Reopen the same library and use
**Review changes** to inspect and retry unfinished operations. Recorded completions
are not repeated. If an action stopped before its completion was recorded, the
review warns that it may already be partly applied; inspect its exact source and
destination paths before retrying. Discard abandons the unfinished plan and keeps
completed files and the audit log. Recovery does not apply file operations by itself. Pending
moves offer **Edit destination…** to correct the folder or base name while keeping
photo companions together and preserving extensions. Saving updates the plan;
review the revised paths before choosing **Retry unfinished changes**.

## Five steps, one library

1. **Organize:** preserve folders, capture date, camera, offline location, visual
   similarity, faces, or date followed by similarity. Organize defaults to the selected
   library folder; an alternate destination remains available. Edit folder hierarchies, drag selected files or folders between groups, and edit file stems;
   review collision-safe moves with RAW/JPEG/XMP companions. Location offers country,
   country/city or country/region/city folders. Changing levels reuses loaded
   metadata and asks before replacing custom location folder edits. Date, camera and
   location grouping automatically read missing metadata without AI analysis;
   cancel or resume that read while browsing. Generated folder edits and move
   previews wait for complete metadata, including photos outside the viewport. Reset destinations
   restores only selected files to their initial grouping proposal, keeping
   other edits and staged names. File and folder context menus offer the same action.
2. **Easy Delete:** categorized sharpness/exposure and duplicate reviews, paired
   Keep/Trash choices, confirm/advance/revoke, reset one/all and accept filtered
   defaults. Exact copies are paired without face comparison; near pairs use camera-aligned
   subject-change checks with lazy, shared face evidence and cached assessments.
   Provisional choices never change shared Trash marks before confirmation.
3. **Fix Rotation:** compare original and corrected views, use manual quarter turns
   or local orientation suggestions, then confirm each choice. Confirm/advance,
   revoke, reset and apply only confirmed corrections through a reviewed plan.
   Proposed nonzero corrections are cross-checked against rotated views of the
   same prepared image. Disagreement without a majority starts on the original,
   shows an uncertainty explanation and requires individual confirmation;
   Confirm all skips those photos. Upright predictions retain a one-pass fast path.
   Rotation evidence is cached across runs and restarts, including uncertainty.
   Unchanged photos need no repeated decoding or model inference. Photo, sidecar,
   model and recipe changes invalidate the evidence; the shared cache limit and
   Clear cache controls apply. Confirmations and manual choices are not cached.
4. **Pick Best:** progressive two-photo tournaments within shared same-subject groups. Choose
   Keep/Trash, confirm, keep both, revisit comparisons, or reset. BeIT aesthetics,
   whole-image sharpness, relative-sharpness gating and eye-closure penalties
   provide suggestions; choices do not assign star ratings.
5. **Cull:** virtualized grid, up to nine comparison panes, native-pixel detail inspection, synchronized or independent zoom/pan,
   exact/minimum-star and similarity-group filters, filename/capture-time sorting,
   shared background metadata reading, cancellable background filtering/sorting,
   ratings, staged Trash marks, and navigation that skips marked photos
   unless Ctrl/Cmd is held.

In Organize and Cull review, right-click a photo or video to mark or unmark the
other displayed comparison items together. The menu shows the affected count;
it leaves the clicked item and any selection outside the comparison unchanged.
These are staged marks: use Review changes before sending files to Trash.
Guided reviews keep their existing Keep/Trash and confirmation controls.

Cull’s **Groups** menu moves selected photos into a new or existing group. The
picker shows counts and an example filename. Corrections survive reanalysis,
restart and verified file moves; Undo and Restore automatic grouping are available.
Manual membership also guides newly built Pick Best reviews. Existing reviews
keep their comparisons until you explicitly analyze again. No files move when
you edit similarity groups.

Steps are optional and revisitable. Decisions persist per folder. No image is
uploaded and no analysis starts during scanning. Read the [native design and current limits](docs/COMPATIBILITY.md)
before using the new app on an existing editing workflow.

## Local AI setup

**Menu → AI models** verifies installed models, reports missing or damaged files,
and repairs them from the app bundle or a prepared ONNX file. DINOv2 and SFace can
also be downloaded directly from pinned upstream revisions. Transfers show their
size and progress, can be canceled, and verify exact size and SHA-256 before
replacing any installed file. Existing models survive failed transfers.

Missing prerequisites stop analysis before image decoding and preserve current
reviews. After repair, **Retry analysis** resumes the requested workflow explicitly.
Model setup has its own lazy worker, so browsing and photo operations stay
independent of slow downloads. Closing the panel leaves a transfer running;
quitting cancels it. No downloads occur automatically and no photos are uploaded.

The other four prepared models come with the full bundle and support verified
import/restore. Public native export downloads for them are not yet published.
The model manager does not download executable ONNX Runtime libraries; restore
the full application if its native runtime is missing. Minimal builds keep manual
review and explain that AI inference is unavailable.

## Design and development

The UI uses egui/eframe with wgpu. Its custom photo canvas, virtualized rows and
on-demand repainting fit this workload without a web frontend runtime. Navigation
adapts between a sidebar and compact stepper. See [architecture](docs/ARCHITECTURE.md)
and the [reference audit](docs/REFERENCE.md).

```sh
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo test --no-default-features
python3 scripts/package_macos.py
```

The optional `native-imaging` feature uses libvips and Little CMS for JPEG, PNG,
TIFF and GIF through the same shared image pipeline. Build the pinned private
dependency with `python3 scripts/build_vips.py` (Highway SIMD enabled by default),
then add `--native-imaging` to the macOS packaging command. For the existing
verified SIMD prefix, also pass `--native-prefix .tools/vips-simd`. The separately verified development bundle is at
`.tools/native-vips/PhotoSort Rust.app`; normal builds still use the default
backend. See [native imaging](crates/imaging-vips/README.md) for requirements,
validation and measured color differences.

The macOS packager accepts `--output /path/to/PhotoSort.app` and honors
`CARGO_TARGET_DIR`. To keep compiler output out of the project and remove it after
packaging (including failed builds):

```sh
(
  build_dir="$(mktemp -d)"
  trap 'cargo clean --target-dir "$build_dir"' EXIT
  CARGO_TARGET_DIR="$build_dir" python3 scripts/package_macos.py \
    --output "/tmp/PhotoSort Rust.app"
)
```

The bundle contains its models and runtime. Packaging verifies the local code
signature and a streamed resource manifest before replacing the output bundle.
By default this is an ad-hoc signed local build, not a notarized distribution.
Use `--sign-identity` with an existing certificate name or SHA-1 for consistent
development signing; see [macOS signing and folder access](docs/MACOS_SIGNING.md).
Ad-hoc rebuilds can trigger a new macOS folder-access prompt. The packager's
integrity and output-safety regressions run without compiling Rust:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s scripts -p 'test_package_macos.py'
```

All workflows share one catalog, one bounded image pipeline, one lazy model
registry and one mutation worker. CPU proxy cache: 128 MiB; canonical source cache: 256 MiB; browser GPU textures: 96 MiB;
disk proxies: 1 GiB, pruned in 32 MiB write batches. Inspection GPU textures hold visible crops rather than whole original photos. These are cache limits, not
whole-process memory limits: full camera decoding and optional neural models need
additional working memory. Scanning is batched and analysis yields between photos. All workflows share neutral
1024-pixel analysis inputs. RAW browsing and analysis share LibRaw's embedded
camera preview, with a neutral half-size fallback for missing or corrupt previews.
Full RAW rendering is deferred until the view needs more pixels or the user selects
100%. Camera previews can differ in color and crop from full RAW detail.

Models are already installed in this working copy. Large weights/native runtime
are excluded from Git; [asset provenance](resources/ASSETS.md) and SHA-256 manifest
record the exact files. Python conversion scripts are build tools only.

To investigate orientation suggestions without changing photos, the native audit
prints quarter-turn predictions, model scores, view counts, uncertainty,
resize-filter comparisons and pipeline/session reuse counters as JSON. These
results use the current production consistency policy. It uses a disposable photo cache:

```sh
(
  audit_build_dir="$(mktemp -d)"
  trap 'cargo clean --target-dir "$audit_build_dir"' EXIT
  CARGO_TARGET_DIR="$audit_build_dir" cargo run --release --locked \
    --example audit_orientation -- /path/to/photo.jpg
)
```

Model scores are softmax outputs, not calibrated probabilities of correctness.
Restoring an image to its starting orientation does not prove that starting
orientation was upright. See the [recorded diagnostic](docs/diagnostics/orientation-2026-10-02.json)
for the supplied samples and the original single-pass night-city failure.
That historical report predates the production consistency policy; new runs
identify their policy explicitly. The policy checks consistency rather than
calibrating confidence, and an incorrect initial upright prediction can still
be missed by the fast path.

Feature parity work is active; see the [parity checklist](docs/PARITY.md).
See [validation](docs/VALIDATION.md) for executed checks and remaining boundaries.

Real reference fixtures can be checked explicitly without modifying them:

```sh
PHOTOSORT_SAMPLE_DIR=/path/to/PhotoSort/tests/samples \
  cargo test --release --test reference_samples -- --ignored --nocapture
```


Cull offers **Photos · no group ordering**, **Groups · earliest photo**, and
**Groups · similarity then time**. Run Analyze library to create similarity
groups. Group ordering keeps each group's photos together; the existing photo
sort controls their order within the group. Photos without a group remain
available at the end. The selected ordering is restored with the folder's view.


Cull includes an optional **Library navigator** in its browse controls. Expand
folders or calendar years/months, or choose a similarity group; the existing
photo grid/list shows the result alongside your rating and search filters.
Previous/Next moves between sibling nodes. Left/Right cycles within the current
folder or day unless similarity cycling is enabled. Browsing never moves files.


## Development build storage

Development and test builds use line-table debug information and disable
incremental compilation. This retains filename/line information in backtraces
while avoiding full variable/type debug data and retained incremental state.
Recompiling changed Rust code can take longer. Release optimization is unchanged.
The test profile inherits the development profile, so normal full/minimal test
commands use the same policy. Environment or user Cargo configuration overrides
can change it; this is a storage reduction policy, not a hard disk quota.

For an explicit full-debug session, use `CARGO_PROFILE_DEV_DEBUG=2
CARGO_INCREMENTAL=1 cargo run` (or set those variables in your shell). Clean those
artifacts afterwards if disk space matters. `cargo clean` removes generated build
output, including `target/PhotoSort Rust.app`; source, fixtures and model resources
remain. The 1 October cleanup removed 573,236 generated files; Cargo reported
97.1 GiB logical size while `du -sh` had reported `61G` allocated. The project then
occupied 668 MB. A subsequent fresh full/minimal validation with the smaller
profile reached a sampled 4.29 GiB in a temporary target directory, then cleaned
it automatically. This is not a hard bound or a controlled comparison with old
accumulated output.

Profile behavior follows the [Cargo profile documentation](https://doc.rust-lang.org/cargo/reference/profiles.html).


## Application updates

Menu → Check for updates uses one lazy application-owned worker, independent of
photo scanning, decoding and AI jobs. Settings and the update dialog share the
persistent daily-check preference. Eligible builds check once on startup when at
least 24 hours have elapsed since the previous attempt; automatic failures are
quiet, and an available release appears in the Menu label. Manual checks show
errors, cancellation, the latest stable version and plain-text release notes.
Long notes use a UTF-8-safe 16 KiB preview prepared by the background worker;
the release link stays visible below the scroll area. The release page opens only
when its link is clicked. No automatic download or
installation occurs.

This development checkout has **no release repository configured**. It reports
that condition explicitly and makes no update request. A distributor must supply
`PHOTOSORT_RELEASE_REPOSITORY=owner/repository` at **build time**, pointing to the
repository publishing the Rust application, and retain the default `updates`
feature. Do not use the Python application's release feed. No repository was
created or release published by this implementation. Minimal builds explain that
online checks are unavailable.

Release tags must be `1.2.3` or `v1.2.3` semantic versions. Stable release checks
reject drafts, prereleases and malformed data, ignore build metadata when
comparing precedence, and never offer a downgrade. Prerelease app versions do not
check automatically. There is no token, photo data or library path in requests.
The response is limited to 2 MiB, HTTPS requests have a 12-second deadline, and
cancel/shutdown interrupts stalled header/body waits. Redirects are rejected;
repository renames require a build with the updated release identity.

Protocol and version ordering follow the
[GitHub releases API](https://docs.github.com/en/rest/releases/releases#get-the-latest-release)
and [semver crate](https://docs.rs/semver/latest/semver/struct.Version.html).

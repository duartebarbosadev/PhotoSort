# Native design and current limits

PhotoSort Rust is designed around photographic results, responsive interaction and
safe file operations. The original application is a workflow reference. Matching
its appearance or numerical implementation is not a product requirement. There
is no Python runtime dependency. Historical comparison fixtures remain useful
regression evidence, but do not establish photographic accuracy or relative speed.
See [PARITY.md](PARITY.md) for the detailed implementation status and
[VALIDATION.md](VALIDATION.md) for the checks actually performed.

## Photo review and organization

The five steps are Organize, Easy Delete, Fix Rotation, Pick Best and Cull. They
share one catalog, image pipeline, metadata owner, model registry and file planner.
Actions remain staged until their plan is reviewed and applied. Trash uses the
operating system's recoverable Trash; permanent deletion is absent.

Organize stages folder hierarchies, names, drag/drop and selection moves, including
RAW/JPEG/XMP companions and scanned supporting files. Hidden and empty folders
are included. Leading dots are preserved; symlinks are not followed or moved.
Departure guards resolve drafts before leaving. Unapplied drafts are not restored
after restart. Accepted file plans now survive process interruption and restart:
reopening the same library restores exact unfinished operations for explicit review.
Recorded completions migrate ratings, manual groups and bookmarks; an operation
interrupted before its completion record remains uncertain, with source/destination
checks intact. Recovery never automatically applies file actions. Discard runs on
the filesystem worker and keeps audit logs and completed files. Pending Move
cohorts can change destination folder and base filename within the reviewed
output library, keeping companions together and preserving their extensions and
casing. Other action types and output-root changes are not editable. On
Unix, matching file identities can recover individual completed members and
recognize a staged hard-link move. Removing the remaining source link is a
reviewed Finish staged move action and requires explicit Retry. Cross-device
copies, ambiguous identities and Windows partial-move inference remain unsupported.
Save plan and quit waits for durable session/view acknowledgement and preserves
new ratings and manual groups without automatically retrying file operations.
Destinations are not overwritten. Empty source-folder cleanup is conditional on
successful prerequisite moves. Explicit in-place case-only folder renames now use
exclusive directory renaming, preserve file identities and rebase dependent moves
and saved decisions. Planning and mutation preflight are cancellable. The complete
reviewed subtree must still match; links and unreviewed additions prevent the
rename. macOS sample-copy checks passed; Linux/Windows filesystem behavior still
needs native verification.

Cull provides virtualized Grid/List/Icons, multiselection, two-to-nine-photo
comparison, rating/group/date/folder filters, group ordering, and a cached
expandable library navigator. All filters intersect. Counts cover the whole
library. Navigation supports rating jumps, group cycling and skipping marked
photos, with Ctrl/Cmd bypass. Unknown metadata is not treated as zero stars.
Metadata indexing does not decode images or initialize neural models. MOV/MP4/M4V
movie-header dates now feed the same sorting, calendar and grouping snapshots,
after EXIF/XMP and before filename/filesystem fallbacks. The bounded header reader
supports 32/64-bit creation timestamps and cancellation without initializing video
playback. Other container date tags and historical camera timezone quirks remain
outside this reader's scope.

Manual similarity groups store photo membership rather than generated group
numbers. Moving into a group pins its current members together with the selected
photos. These corrections survive analysis, restart and verified file moves.
Undo restores the last edit in the current session. Selected/all restoration
uses current automatic groups, or leaves photos ungrouped until Analyze if no
automatic snapshot is available. Groups guide newly built Pick Best reviews;
existing tournaments retain their comparisons until explicit reanalysis.
Group edits do not move files. Organize has its own destination-draft editor.

## Imaging and local analysis

Display uses a fast proxy followed by original-pixel inspection on demand. Shared
bounded caches and visible-region textures limit retained image memory. Staged
rotations are display transforms shared by thumbnails, filmstrip and preview.
Analysis has its own neutral 1024-edge input under the same pipeline owner. RAW
uses embedded JPEG where available, otherwise neutral half-size LibRaw processing.
Standard images and embedded RAW JPEG previews now honor RGB/grayscale ICC
profiles through a shared sRGB normalization step. Native 16-bit precision is
preserved until the existing display conversion. Corrected pixels are reused by
preview/detail/analysis and persisted proxies. Real system Display P3 and Adobe
RGB profiles agree with Apple ColorSync within the tested four-level 8-bit bound.
ICC-tagged Adobe CMYK and YCCK JPEGs retain their original ink channels through
`jpeg-decoder` before the same cached `moxcms` conversion; ordinary JPEGs retain
the fast existing decoder. This describes the default Rust image/color backend,
which does not support CMYK TIFF or other non-RGB/gray profiles outside the JPEG
fallback. The current macOS bundle enables `native-imaging`: its shared libvips /
Little CMS adapter also supports profiled CMYK TIFF. Adapter regressions cover
8/16-bit packed, planar and LZW fixtures; they do not establish every TIFF variant.
See [native image support and validation](../crates/imaging-vips/README.md).
Invalid, oversized or mismatched profiles report a preview error. Untagged
images still assume sRGB; standalone PNG gamma/chromaticity/CICP interpretation,
HDR/wide-gamut output, monitor calibration, and broader RAW/platform color accuracy
remain unverified or outside this normalization. The existing ImageIO HEIF and
LibRaw rendering recipes are unchanged.

General visual grouping uses six regional DINO descriptors and streaming,
orientation-aware DBSCAN. Mixed organization partitions by date first. Cull and
Pick Best use shared subject evidence and complete-link grouping. People Organize
uses native SFace identity embeddings after five-point face alignment. Multiple
main people receive one stable combination destination, such as People 001 + 002;
small background faces do not create extra destinations. Unclear, very small or
ambiguous repeated identities remain Unassigned. Conservative group admission
prevents transitive identity chains. Native checks covered seven original samples
and seven attributed portrait/group fixtures; broad portrait/scene accuracy
and large diverse-library CPU performance still need measurement.

Easy Delete combines exposure/sharpness review, full-content exact copies and
subject-sensitive near-duplicate verification with alignment, masked structural
comparison and lazy face evidence. Completed pair results are cached. Pick Best
uses aesthetics, sharpness and eye-closure evidence as suggestions; users confirm
the outcome. Eye suggestions can fail on small, obscured or angled faces. There
is no claim of perfect automatic keeper selection.

Technical measurements, full-content hashes and aesthetic evidence persist under
source/sidecar and recipe identities. Reopening an unchanged library can reuse
complete evidence without decoding pixels or initializing models. Missing or
invalid records are recomputed; new thresholds classify existing measurements.
The shared cache controls include these records.

Models initialize lazily and run locally. Thread budgets configure the shared
ONNX pools. Regional DINO crops use bounded native batches where supported;
Core ML specializes DINO to one image and reuses that session across crops.
Settings now offers CPU, Automatic, Core ML, CUDA and DirectML through the shared
model registry, with per-model CPU fallback and visible explanations. Apple sample,
session-reuse and native cancellation checks pass. Actual CUDA/DirectML hardware
execution, broader accelerator performance and other-model batching remain to be
validated. CPU stays the default because cold Apple compilation and memory costs
are substantial; see [ACCELERATION.md](ACCELERATION.md).
Menu → AI models now provides verified inventory, import, bundled-copy repair and
pinned DINOv2/SFace/orientation downloads. The other three prepared models require
the full bundle or a prepared import until native exports are published. Missing models or
runtime prerequisites pause analysis and preserve reviews; retry is explicit.
Transfers are cancellable during network waits and remove incomplete temporary
files. Native model inference calls may finish their current unit before analysis
cancellation.

## Persistence and platform limits

Ratings and rotations support Embed where supported or Sidecars only. Native
embedded writes use an independent temporary copy, verify EXIF/XMP values and
source identity, then atomically replace without re-encoding pixels. Unsupported
formats fall back to sidecars. Embedded HEIF/container transforms, broader camera
interoperability and broader platform/volume attributes need more coverage. A
macOS probe of the existing staging copy and production metadata bridge preserves
Finder-tag/custom extended attributes, ACLs, ownership, file flags, permissions and
creation time on all seven disposable samples; modification time updates after
editing. This is a bridge/copy-path check, with explicit limits in
`diagnostics/metadata-attributes-2026-10-06/README.md`.

Saved views and manual group intent live outside disposable caches. Folder and
workflow bookmarks restore focus, filters, selection and presentation without
applying file operations. Changed/replaced photos invalidate saved identities.
Existing sessions from the original application are not imported.

macOS videos now play inside the shared review canvas using AVFoundation, with
play/pause, seeking, volume and focus/modal pause. Frames and native thumbnails
are bounded to a 1280-pixel long edge. Windows/Linux default builds use the shared
GStreamer backend for embedded playback and posters; builds without portable-video
retain external playback and FFmpeg posters. The Linux aarch64 backend has passed
its portable-feature suite, while Windows runtime and Linux desktop interaction
remain unverified; see `diagnostics/portable-video-2026-10-06/README.md`. Muted macOS
fixtures and native playback controls passed; audible A/V synchronization,
HDR/color fidelity and native compact resizing still need validation. Finder/
Explorer reveal and external-player launch now share one lazy bounded desktop
service. Service and all-workflow routing tests pass. Packaged macOS checks passed
for Organize/Cull reveal, the exact context-clicked file without changing review
focus, quoted/Unicode filenames and external QuickTime launch. Windows/Linux
native desktop handoff still needs platform verification.

Manual update checks and persistent daily startup checks are implemented, with
cancellation, a non-interrupting update indicator and bounded plain-text release
notes. This development build has no configured Rust release repository and
reports that limitation. A published release feed and native available-release/
network checks remain pending; the Python application's feed is not reused.

The macOS application is a locally signed development bundle, not a notarized
release. Cmd+Q and menu Quit enter the shared departure guard. Native sample
checks passed for Cancel, Discard, repeated Quit and confirmed review changes;
the same-frame event race is covered by a renderer/application regression. Saved
plan quit and two restart checks also passed. Dock Quit, OS logout and Windows/
Linux behavior still need platform testing.

Synthetic performance checks establish local behavior only. They do not establish
a speed ratio against the original application or sustained whole-library memory
use. Live UI checks, real-library accuracy, and workload-matched measurements
remain separate acceptance criteria.


## Startup maintenance and diagnostics

`--clear-models` clears the six catalogued managed model filenames before startup,
using the same model ownership as import/download setup. Bundled resources,
external overrides, unknown files, runtime libraries, photos and saved sessions
are outside that scope. The operation preflights the complete known set, refuses
links, rechecks each identity before removal and reports partial failures.

`--diagnostics` is a standalone read-only JSON command. It reuses the shared
model inspector and reports build capabilities, model verification, runtime
presence and override presence. It neither initializes a graphical window nor
loads ONNX Runtime. `files_ready` is file readiness for the enabled build, not
proof of ABI compatibility, successful inference or UI health. A minimal build
reports AI as disabled rather than requiring AI files. `--check-runtime` additionally exercises all six native AI sessions through the
shared production services on an embedded licensed fixture, then repeats the work
to check session reuse. The default is CPU; `--runtime-backend` selects cpu,
auto, coreml, cuda or directml without changing saved preferences. Per-model JSON
provider records explain CPU fallback. Passing/exit status establish functional
inference and reuse, not accelerator selection or per-operator hardware placement.
It reports per-stage failures and rejects unsupported minimal builds. It tests the configured orientation override's input/output
contract when present. Automatic GUI smoke exit, codec/video/package relocation
checks and wider photographic accuracy remain separate work.


## Evidence reconciliation — 2026-10-05

The recovery, multi-person, update-check and quit descriptions above were checked
against the current shared implementations and recorded validation, replacing
stale claims of missing features. Relevant evidence includes
`diagnostics/multiple-people-native-2026-10-04.json`,
`diagnostics/partial-companions-native-2026-10-04.json`,
`diagnostics/resumable-quit-native-2026-10-04.json`, the durable destination editing
and update-check sections in VALIDATION.md, and the current default/minimal suite
record in `diagnostics/desktop-final-validation-2026-10-05.json`. Historical native
checks establish the stated scenarios, not fresh execution on every later build.


## Context paths — 6 October 2026

Library views and Organize now share Open in default app, Reveal and full/relative
path copying. Organize also copies the proposed destination, preserving companion
extensions. Preview moves remains authoritative for collision-resolved paths.
New or ambiguously merged draft folders have no original directory to open;
copying their displayed relative/proposed path remains available. This replaces
the reference's fallback of opening the library root for a missing draft folder.
Non-UTF8 filesystem paths remain valid OS handoff targets but are not copied as
lossy clipboard text. Open terminal here is now implemented through the same desktop service. Native macOS menu/clipboard checks now pass for source and staged paths, Unicode paths and compact Organize controls; automated pointer, clipboard-output and worker tests cover shared behavior. See diagnostics/path-actions-native-2026-10-06/README.md.


## Open terminal here — 6 October 2026

All library layouts and Organize file/folder context menus expose the same action.
A file targets its original containing folder; a folder targets itself. Unavailable
sources and draft folders with no unique original source cannot launch a terminal.
The worker validates the source once and resolves the directory off the UI thread.
No shell command containing a photo path is constructed.

- macOS: LaunchServices opens the folder with Apple's Terminal application.
- Linux: `xdg-terminal-exec` uses the user's configured preferred terminal. A
  missing launcher or unconfigured terminal produces an actionable error. This
  deliberately replaces the Python implementation's file-manager fallback.
- Windows: built-in Windows PowerShell opens in a new console with the directory
  passed as process working-directory state. Windows chooses its configured
  console host; PhotoSort supplies no PowerShell script or `cmd /K` expression.

Linux/Windows terminal children are retained for nonblocking exit reaping; live
terminals do not keep desktop handoff busy or get terminated when PhotoSort exits.
The UI requests a one-second maintenance frame only while such children remain.
Shell profiles and terminal preferences still control normal shell startup.
The user confirmed a native macOS file action opened Terminal in the correct containing folder (A café/Day1) and closed it afterward; tool policy prevented independent Terminal inspection. Windows/Linux native window behavior still requires separate verification; command construction and shared-worker tests do not establish it.


## HEIF write capability audit — 6 October 2026

Both installed piexif 1.1.3 (used by the reference rotation path) and native Exiv2
0.28.9 reject HEIF writes on all eight pinned orientation fixtures. Disposable
copies remain byte-identical. Rust continues to write non-destructive XMP sidecars;
embedded HEIF/container transforms are a possible interoperability improvement,
not an established working Python behavior. No library or write policy changed.
See `diagnostics/heif-write-feasibility-2026-10-06/README.md` for the source trace,
probe scope and unadopted library candidate.


## Move cancellation — 7 October 2026

Shared Move staging now responds to worker cancellation between companions and
between 1 MiB fallback-copy chunks. Before source removal, cancellation removes
staged destinations and preserves all sources plus the exact reviewed retry.
After source removal begins, the companion cohort finishes together. Cleanup
failures remain operation errors. Native filesystem calls already in progress
cannot be preempted, and this change adds no in-progress UI Cancel button.
Shared recovery/retry tests, the full native suite and seven-sample move checks
pass; see diagnostics/move-cancel-2026-10-07/README.md.


### Shared Stop control — 7 October 2026

The shared footer now offers Stop for running file plans in every workflow. It
preserves the accepted plan, completes any current indivisible operation, and
leaves unfinished work available for Retry. Move staging uses the cancellation
and rollback boundary described above. Stop stays visible in compact windows;
retries get fresh cancellation state. Automated all-workflow pointer, recovery
and source-preservation tests pass, together with the complete native suite.
Native Stop/Retry also passes on 20,007 disposable images: 893 completed before
Stop, 19,114 retained and retried, then zero unfinished operations. All hashes
match; original profiles were restored and fixtures removed. See
diagnostics/stop-apply-2026-10-07/README.md. This supersedes the earlier statement
that only worker cancellation existed without a UI control.


### Fallback move attributes — 7 October 2026

Shared fallback moves now preserve modification/access times and source
permissions, with private Unix partial copies and existing cancellable rollback.
Same-volume hard links retain their fast path. The before/after disposable probe
and three shared regressions pass, as do the full suite, strict Clippy and rebuilt
bundle/runtime checks. Actual cross-volume/platform execution and extended
attributes are not claimed. See diagnostics/copy-attributes-2026-10-07/README.md.


### Compact video review — 7 October 2026

Short video review slots now use the existing compact controls, preserving
picture space. A rendered-mesh regression covers all five workflows and texture
reuse during resizing. Full validation and rebuilt native/runtime checks pass.
The formerly locked native seeking check is complete on disposable macOS video
fixtures; audio remained muted. See diagnostics/video-layout-2026-10-07/README.md
and native-checks.json for exact interaction coverage and platform limits.

# Reference interaction audit

9 October 2026, Rust `979b0e6`, Python `7c62143`. This is a source audit of the
reference application menus, Preferences, Cache Management, image context menu,
Organize context actions and workflow-panel toggle. It is not a fresh native run
or an assertion that every widget in both applications is equivalent.

The [snapshot](diagnostics/interaction-audit-2026-10-09/snapshot.json) records
reference handler locations and hashes, current source equivalence to the last
full validation, and the packaged executable identity. No application code,
profile, sample or original-project file was changed for this audit.

## Application actions

Python owners below are in `src/ui/menu_manager.py` unless stated otherwise;
Rust owners are relative to `src/`. Menu placement and wording are intentionally
different. A shared action can be available through a toolbar or selection menu
without recreating Python's menu bar.

| Reference action and handler | Rust implementation or explicit difference |
| --- | --- |
| Open / recent folders / Exit, `connect_signals`, `update_recent_folders_menu` | `ui/shortcuts.rs`, `ui/preferences.rs`, `ui/transitions.rs` use shared open/departure/close handling. Opening recent folders follows the same guards as opening a new folder. |
| Find, `main_window.py::_focus_search_input` | Shared search selection in `ui/shortcuts.rs`; works in searchable workflows. Pick Best uses cluster/comparison selection rather than a text search. Python's handler always targets the Cull left-panel search input. |
| Folder/similarity views and Alt+1–3 layouts, `_create_view_menu` | `ui/navigator.rs::set_navigator`, `ui/shortcuts.rs::view_shortcut`; folder/group/date navigator and Grid/List/Icons share the browsing state. Review hides the navigator to give photos room. |
| Back to Grouping, `main_window.py::_return_to_grouping_source` | Rust's Organize step is revisitable through shared navigation, without loading a separate workflow catalog. This is not an exact alias: Python reloads its remembered source root. Returning to a different previously opened root in Rust uses Recent folders/Open. |
| Analyze Similarity, `connect_signals` | Ctrl/Cmd+S and workflow controls use `analyze_current_workflow`; rotation uses the same admission rules and its orientation worker. The applicable analysis depends on the current step. |
| Details sidebar, `_create_view_menu` | Details button / I toggle the shared inspector in `ui/navigation.rs` and `ui/workspace.rs`. |
| Rating and cluster filters, cluster ordering, `_create_filter_menu` | `ui/browsing.rs` and `core/browsing/` own filters/order. Rating jumps and singleton skipping are persistent Cull navigation settings; they affect movement without hiding photos. |
| Clockwise/counterclockwise/180-degree rotation; mark/unmark deletion, `connect_signals`, `show_image_context_menu` | Shared staged decisions in `ui/selection.rs` and `ui/workspace.rs`; R/Shift+R/Alt+R where manual review owns the keys. Guided reviews retain their own choice/reset bindings. Selection actions cover multiple photos. |
| Commit marked deletions / clear marks, `connect_signals` | Review changes / Shift+Enter opens the shared plan before mutation; Alt+D and the shared clear-marks menu remove staged Trash marks. No permanent-delete mode. |
| Zoom, Fit, actual size, numbered focus, single/comparison and synchronized pan/zoom | `ui/review.rs` and shared keyboard routing own Fit/1:1/zoom/focus/Sync. Comparison selection is bounded to nine photos; F2 selects a useful comparison rather than merely allocating empty viewer slots. |
| Show in Explorer, `_open_image_in_explorer` | `ui/desktop.rs` dispatches Reveal through `core/desktop.rs` and the background desktop worker. The same service owns Open and Terminal actions. |
| Move to New Cluster, `_move_selection_to_new_cluster` | `ui/grouping.rs::group_controls` supports new/existing groups, undo and automatic-group restoration. Membership corrections persist and do not move files. |
| Preferences F10 / Cache Management F9 | One draft and shared controls in `ui/preferences.rs`; Ctrl/Cmd+, is also supported. Cache work runs through its shared owner. |
| About F12 / Check for Updates | Existing About and update dialog. A Rust release feed is still unconfigured; an implemented client is not a working public update service. |
| Replay intro video, `dialog_manager.py::show_intro_video_dialog` | Deliberately replaced by the Rust welcome screen, numbered step descriptions, contextual controls and keyboard guide. No old-interface video is embedded. First-time-user usability has not been established by this source audit. |

## Panel visibility by workflow

`main_window.py::_active_workflow_left_panel` and
`toggle_workflow_left_panel` establish that Ctrl/Cmd+Shift+L hides a workflow
content panel, not the five-step navigation. Rust does not implement that key.
The intended space-saving behavior is accounted for as follows:

| Workflow | Python panel | Rust presentation |
| --- | --- | --- |
| Organize | Before tree | Files and destination structure live in a separate arrangement editor; compact windows switch Files/Folders. Selection preview uses the shared canvas and temporarily replaces the editor contents. There is no persistent Before tree beside the main photo review to hide. |
| Easy Delete | Review list | Category filters, Grid/Review, previous/next and the shared filmstrip replace the permanent left review list. The filmstrip yields space automatically in short windows. |
| Fix Rotation | Review list | Suggestions-only filter, Grid/Review and shared photo navigation; no independent left review-list panel. |
| Pick Best | Review list | Cluster and comparison choosers in `ui/pick_review.rs` replace the permanent list. |
| Cull | Left browser | Review replaces the browser with the photo canvas. The Library navigator has its own toggle and becomes a bounded popup in narrow windows; Details is independently collapsible. |

No unrelated step-navigation toggle should be added merely to reuse the old key.
These are deliberate layout decisions, not proof that the new onboarding or every
small-window arrangement has been user-tested.

## Preferences and cache management

The authoritative reference handlers are
`dialog_manager.py::show_preferences_dialog` (including `save_preferences`) and
`show_cache_management_dialog`; labels alone can be misleading.

| Reference control | Rust ownership and decision |
| --- | --- |
| Footer shortcut hints; optional Easy Delete/Fix Rotation/Pick Best steps | Same capabilities in `core/preferences.rs` and `ui/preferences.rs`; Organize/Cull remain available. |
| Balanced/Performance/Custom CPU count | Shared `core/processing.rs` settings, explicitly scoped to AI thread allocation. Rust also offers automatic/CPU/Core ML/CUDA/DirectML selection with per-model fallback status. This is not a cap on total process CPU use. |
| Conservative/Standard/Broad same-subject strictness | Shared settings for the next Cull/Pick Best analysis. Rust preserves the current review rather than silently rebuilding it on Save. |
| Blur, near-black and overexposed thresholds | Shared Easy Delete analysis configuration; current review choices remain unchanged until a new analysis. Rust's blur explanation matches its direction: lower values flag fewer photos. |
| “Near-duplicate distance” | Intentionally not copied as an equivalent sensitivity slider. In `workers/easy_delete_worker.py`, it gates the exact-byte comparison; nonidentical same-frame candidates use a separate fixed similarity gate and comparator. Rust handles exact cohorts independently and uses its subject-safe comparison. Its visual-grouping distance setting is a different parameter, not a renamed equivalent. |
| “Similarity Model” section | The current Python dialog has strictness selection but no model chooser. Its loader pins DINOv2-small. Rust's verified model catalog/setup replaces stale claims about selectable Python models. |
| Separate thumbnail/preview/EXIF/analysis clears and separate EXIF disk allowance | Deliberately consolidated into shared reusable-cache usage/clear, preview/detail memory budgets and one persistent-cache budget. Rust does not offer category-specific clearing or a separate EXIF disk slider. Clearing reusable caches preserves decisions, saved positions and downloaded model assets. |
| Delete All Models | Rust reviews removal of each managed copy in `ui/models.rs` and retains bundled assets. Reanalysis may require import/download; the UI states that consequence. No blanket deletion of external or bundled model locations. |
| Open Cache Folder | **Fixed since the audited `979b0e6` baseline.** Settings has "Open cache folder", which sends the configured cache root to the shared desktop service and shows a readable error when that fails (`src/ui/preferences.rs`, covered by `src/ui/preferences/tests.rs`). |

## Organize context actions

`grouping_step_widget.py::_populate_common_context_actions` and
`_populate_after_edit_actions` map to the arrangement editor, shared path menu,
tree controls and reviewed operation planner:

- Open/Reveal/Terminal and full/relative/projected path copy use
  `ui/desktop.rs::locations_menu`. Proposed destinations remain separate from
  existing paths and are confirmed by the plan.
- Rename file/folder, create folder/parent, move selected items, nest/unnest and
  reset use `ui/organize.rs` with `core/organize.rs::Draft`; companion handling and
  final filesystem actions belong to the shared planner/executor.
- Python's **Put back in original folder** first uses
  `_original_group_labels_by_path`, populated from the initial grouping plan;
  only entries without that baseline fall back to source-relative folders.
  Rust's **Reset file destinations** accurately describes restoring generated
  destinations for the selection. It does not promise to undo already-applied
  filesystem changes.
- Trash marks and immediate Trash requests converge on reviewed operations;
  original whole-folder deletion excludes the library root and ambiguous merged
  destinations. Expansion actions use the shared virtual tree controls.
- Python's optional collision dialog displays up to ten projected duplicate
  destinations. Rust resolves/reports destinations in Preview moves and refuses
  overwrite in the executor. It does not reproduce a separate informational
  collision popup.

## Outcome and validation boundary

The scoped reference inventory is now accounted for. **Open cache folder remains
an implementation follow-up**, and the explicitly listed differences must not be
described as exact UI parity. No new test run was needed for these documentation
changes: all 211 source fingerprints and the executable match the previous
[Organize-mode validation](diagnostics/organize-modes-2026-10-09/README.md), whose
68 commands passed. That evidence retains its original scope and ignored tests.
Distribution, hardware, accuracy and long-session gaps remain in the
[completion audit](history/COMPLETION_AUDIT.md).

## Follow-up

The [cache-folder task](diagnostics/cache-folder-2026-10-09/README.md) implements
and locally validates the baseline omission recorded above, including bounded
Settings errors. This report and its snapshot still describe the audited
`979b0e6` baseline; the follow-up has a separate current-source validation record.

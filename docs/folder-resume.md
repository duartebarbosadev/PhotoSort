# Folder view resume

Opening a folder restores its most recent workflow automatically. Each workflow
has its own position within that folder; opening a different folder does not
replace the previous folder's bookmark. Normal startup still waits for a folder
selection (`--folder` and `--last-folder` retain their existing explicit behavior).

Saved presentation includes Cull's layout, rating/search/cluster filters, sorting,
selected files, comparison view, and scroll position; Organize's grouping mode,
selections in both trees, and their scroll positions; and each review workflow's
current photo or pair, list position, and applicable category/detail/focus controls.
When files no longer exist, surviving selections are restored and otherwise the
workflow keeps its normal fallback view. Existing post-deletion neighbor selection
remains the owner of selection changes after Apply.

Bookmarks contain no deletion marks, rotation choices, grouping edits, ratings,
or review confirmations. Closing and changing workflows/folders use the existing
Apply/Discard/Cancel dialogs. A restored Pick Best pair starts unconfirmed if its
old comparison no longer exists; the old tournament's decisions are not replayed.

## Ownership and lifetime

- `FolderResumeController` owns the active folder's presentation state, captures
  it on workflow/folder departure and accepted close, and checkpoints changed
  views once per second. Loading views do not overwrite saved positions.
- Workflow adapters capture and restore only their own presentation. Shared
  scroll/selection helpers avoid separate persistence implementations per page.
- `FolderViewStore` writes versioned JSON files with atomic replacement under
  PhotoSort's persistent application data (`folder-views`), keyed by normalized
  absolute folder path. No sidecars are written and clearing image caches does
  not erase bookmarks. A renamed/moved folder is treated as a different folder.
- `WorkerManager` serializes bookmark I/O away from the UI thread and delivers
  results through the existing modal-safe UI dispatcher. Writes drain during
  accepted shutdown; cancelling analysis does not discard them. Folder generation
  checks reject late reads/restores. Missing, corrupt, or incompatible records
  fall back to normal loading; I/O failures are logged.
- Resume waits for the existing scan, shared image preparation, and required
  workflow analysis. It starts no independent decoding, metadata/model loading,
  cache, scanner, or analysis pipeline. Existing caches remain authoritative.

There is no intentionally duplicated scan, decode, analysis, or persistence
implementation. The small adapter methods differ because each workflow presents
a different kind of selection. Expensive analysis is not serialized into a second
bookmark cache; a workflow may need to prepare its normal results before selection
can be restored.

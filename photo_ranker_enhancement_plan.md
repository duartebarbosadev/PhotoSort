# PhotoRanker Enhancement Plan

This document outlines the plan to add new image organization features to the PhotoRanker application.

## Feature 1: Organize by Date

**Goal:** Allow users to view images organized chronologically by date, grouped by Year and then Month, with clear date headers/separators. This will be a new view mode.

**Detailed Plan:**

1.  **Date Metadata Extraction:**
    *   Modify `src/core/rating_fetcher.py` to extract date information using `exiftool`.
    *   Prioritize common EXIF/XMP date tags (e.g., `EXIF:DateTimeOriginal`, `XMP:DateCreated`).
    *   Implement fallbacks:
        *   Parse common date patterns from filenames (e.g., `YYYYMMDD`, `YYYY-MM-DD`).
        *   Use filesystem creation/modification time.
    *   Handle images with missing/unparseable dates under an "Unknown Date" section.
    *   Update `metadata_fetched` signal in `RatingFetcher` to include the extracted date (`datetime.date` or `None`).
    *   Add detailed logging for the date extraction process (which tags/fallbacks are used).

2.  **UI Implementation (`src/ui/main_window.py`):**
    *   **New View Mode Button:** Add a "Date" button to the view mode controls, connected to `_set_view_mode_date()`.
    *   **Date View Widget:** Use `DroppableTreeView` configured for hierarchical display (Year -> Month -> Image).
    *   **Model Rebuilding:** Implement `_rebuild_model_view_by_date()` to populate the `QStandardItemModel` with the date structure. Sort years and months chronologically ("Unknown Date" group handled). Sort images within groups.
    *   **View Mode Slot:** Implement `_set_view_mode_date()` to switch visibility, configure the tree view, and call the date-specific model rebuild.
    *   **Helper Functions:** Implement `_find_first_image_item_in_date_view()` and `_find_last_image_item_in_date_view()` for navigation.

3.  **Logic and Integration (`src/ui/main_window.py`):**
    *   **File Selection:** Update `_handle_file_selection_changed()` to handle selection of Year/Month headers vs. image items and display relevant info (including date) in the status bar.
    *   **Navigation:** Update `_navigate_previous()` and `_navigate_next()` to correctly traverse the hierarchical date view, skipping headers.
    *   **Filtering:** Update `_apply_filter()` to recursively handle visibility in the tree, ensuring headers remain visible if they contain filtered images.
    *   **Caching:** Implement `self.date_cache` to store extracted dates.

**Mermaid Diagram (High-Level Flow for Date Organization):**

```mermaid
graph TD
    A[User Selects Folder] --> B{MainWindow: _load_folder};
    B --> C[FileScanner: scan_directory];
    C -- files_found --> D{MainWindow: _handle_files_found};
    D -- scan_finished --> E{MainWindow: _handle_scan_finished};
    E --> F[RatingFetcher: fetch_ratings_and_dates (with fallbacks)];
    F -- metadata_fetched (path, rating, label, date) --> G{MainWindow: _handle_metadata_fetched};
    G -- rating_fetch_finished --> H{MainWindow: _rebuild_model_view / _apply_filter};

    I[User Clicks "Date View" Button] --> J{MainWindow: _set_view_mode_date};
    J --> K{MainWindow: _rebuild_model_view_by_date};
    K --> L[DateViewWidget: Displays Images by Year/Month with Headers];
```

## Feature 2: Organize by Similarity (Content/Subjects)

**Goal:** Automatically group images based on visual similarity of their content or subjects. This is a more complex feature and will be tackled after the "Organize by Date" feature.

**Chosen Approach:**

*   **Feature Extraction:** Use `sentence-transformers` library to load a pre-trained CLIP model (e.g., `clip-ViT-B-32`) for generating semantic image embeddings.
*   **Clustering:** Use `scikit-learn` library for clustering the generated embeddings (e.g., KMeans, DBSCAN).

**Detailed Plan (Preliminary):**

1.  **Library Integration & Setup:**
    *   Add dependencies: `sentence-transformers`, `torch` (or `tensorflow`/`jax` depending on backend), `scikit-learn`, `Pillow`. Update `requirements.txt`.
    *   Implement logic to handle downloading the chosen CLIP model (potentially on first use or with user confirmation).

2.  **Feature Extraction Engine:**
    *   Implement a new class (e.g., `SimilarityEngine` in `src/core/`).
    *   Responsibilities:
        *   Load images using `Pillow`.
        *   Load the CLIP model using `sentence-transformers`.
        *   Preprocess images as required by the CLIP model.
        *   Generate embeddings (feature vectors) for images.
        *   Implement robust caching for embeddings (e.g., disk-based using file paths and model name) to avoid re-computation.
        *   Run the embedding generation process in a background thread (`QThread`) to avoid freezing the UI, emitting progress updates.

3.  **Clustering/Grouping:**
    *   In `SimilarityEngine` or a related class:
        *   Apply a clustering algorithm from `scikit-learn` (e.g., start with KMeans or DBSCAN) to the cached embeddings.
        *   Determine a strategy for choosing clustering parameters (e.g., fixed number of clusters for KMeans, automatic estimation or user input for DBSCAN's `eps`).
        *   Store the resulting cluster assignments (e.g., map file path to cluster ID).

4.  **UI Implementation:**
    *   Design how similarity groups will be presented. Options to consider:
        *   **New "Similarity View" Mode:** Similar to Date View, potentially showing clusters as top-level items.
        *   **Cluster Tags/Filter:** Add cluster IDs as virtual tags and allow filtering based on them in existing views.
        *   **Dedicated Panel:** A separate dock widget or dialog to browse clusters and their images.
    *   Implement the chosen UI approach in `MainWindow`.

5.  **Logic and Integration:**
    *   Integrate the `SimilarityEngine` into `MainWindow`.
    *   Define trigger for similarity analysis (e.g., button, after scan completes).
    *   Update `_rebuild_model_view` or create new methods to display clusters based on the chosen UI.
    *   Adapt navigation and filtering to work with the similarity groupings.

**Implementation Notes & Considerations:**

*   **Performance:** Feature extraction is computationally intensive. Background threading is crucial. GPU acceleration (if available via `torch`) significantly speeds this up. Consider processing images in batches.
*   **Model Downloads:** CLIP models are large. Inform the user and handle downloads gracefully.
*   **Caching:** Efficient caching of embeddings is vital for usability on subsequent runs.
*   **Clustering Parameters:** Finding optimal clustering parameters automatically is challenging. May require heuristics or user adjustments.
*   **Scalability:** Performance might degrade with very large numbers of images (tens of thousands+).

**Mermaid Diagram (High-Level Flow for Similarity Organization - Updated):**

```mermaid
graph TD
    M[Analysis Triggered (e.g., Button Click)] --> N{SimilarityEngine: Start Analysis};
    N --> O[Load/Generate Embeddings (using sentence-transformers/CLIP)];
    O --> P[Cache Embeddings];
    P --> Q[Cluster Embeddings (using scikit-learn)];
    Q --> R[Generate Cluster Assignments (Image Path -> Cluster ID)];
    R --> S{MainWindow: Update UI (e.g., Similarity View / Filter Update)};

    T[User Interacts with Similarity UI] --> U[Display Images from Selected Cluster];
```

## Prioritization

1.  Implement **Feature 1: Organize by Date** first. (Completed)
2.  Begin detailed work on **Feature 2: Organize by Similarity**, starting with integrating `sentence-transformers` and `scikit-learn`.
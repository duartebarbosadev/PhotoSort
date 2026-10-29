//! One presentation checkpoint/restore owner for every workflow.
use super::*;
use crate::core::views::{Bookmark, Saved};
impl PhotoSortApp {
    pub(super) fn capture_bookmark(&self) -> Bookmark {
        let comparison = if self.step == Step::PickBest {
            Vec::new()
        } else {
            self.comparison.clone()
        };
        let mut categories: Vec<_> = self.delete_categories.iter().copied().collect();
        categories.sort();
        let (selected, anchor) = self.selection.snapshot();
        Bookmark {
            missing_focus: false,
            editor: None,
            active: self.active,
            selected,
            anchor,
            comparison,
            review: self.review,
            inspector: self.inspector,
            layout: self.library_layout,
            query: self.query.clone(),
            filter: self.filter,
            browse: self.browse.clone(),
            navigator: self.navigator.clone(),
            group_anchor: self.browse.group.and_then(|group| {
                self.media
                    .iter()
                    .find(|m| self.cull_groups.get(&m.id) == Some(&group))
                    .map(|m| m.id)
            }),
            group_mode: self.group_mode,
            delete_current: self.delete_current,
            delete_categories: categories,
            rotation_queue_only: self.rotation_queue_only,
            pick_focus: false,
            view: self.inspection_view,
            independent_views: self.independent_views.clone(),
            sync_views: self.sync_views,
            scroll: self.library_offset,
        }
    }
    pub(super) fn checkpoint_bookmarks(&mut self) {
        if self.scanning || self.catalog.is_none() || self.busy || self.plan_open {
            return;
        }
        let Some(root) = self.root.clone() else {
            return;
        };
        if self.capture_saved_view()
            && let Err(error) = self.runtime.io(IoWork::SaveViews {
                root,
                views: self.saved_views.clone(),
            })
        {
            self.error = Some(error);
        }
    }
    pub(super) fn capture_saved_view(&mut self) -> bool {
        let step_changed = self.saved_views.step != Some(self.step);
        self.saved_views.step = Some(self.step);
        let mut changed = step_changed;
        if self.pending_bookmark.is_none()
            && self.analysis_run.is_none()
            && self.view_data_ready.contains(&self.step)
        {
            let view = self.capture_bookmark();
            changed |= self.saved_views.bookmarks.get(&self.step) != Some(&view);
            self.saved_views.bookmarks.insert(self.step, view);
        }
        if changed {
            self.saved_views
                .capture_catalog_fingerprints(self.catalog.as_ref().unwrap(), &self.index);
        }
        changed
    }
    pub(super) fn discard_staged_choices(&mut self) {
        // Review reset can replace the current pair. Preserve the presentation first,
        // and keep subsequent departure/exit checkpoints from recording that reset.
        self.checkpoint_bookmarks();
        if self.pending_bookmark.is_none() {
            self.pending_bookmark = self.saved_views.bookmarks.get(&self.step).cloned();
        }
        self.decisions.clear();
        // Back to the defaults: Easy Delete, Fix Rotation and Pick Best stage
        // their suggestions again.
        self.delete_review.restore_suggested(Default::default());
        self.rotation_review.restore_staged(Default::default());
        self.pick_review.restore_suggested(Default::default());
        self.changed();
    }
    pub(super) fn prepare_bookmark(&mut self) {
        self.pending_bookmark = self.saved_views.bookmarks.get(&self.step).cloned();
        self.resume_started = false;
        self.resume_suspended = false;
        self.library_restore_offset = None;
        self.library_offset = 0.0;
        if let Some(view) = &self.pending_bookmark {
            self.query = view.query.clone();
            // Fix Rotation has no photo filter to show or clear, so a view
            // saved when it had one restores every photo.
            self.filter = if self.step == Step::Rotation {
                Filter::All
            } else {
                view.filter
            };
            self.browse = view.browse.clone();
            self.navigator = view.navigator.clone();
            // Group numbers belong to an analysis run. Resolve the saved member afterwards.
            if view.group_anchor.is_some() {
                self.browse.group = None;
            }
            self.library_layout = view.layout;
            self.review = view.review;
            // Narrow windows show Details as a drawer over the photos; only
            // reopen it on request, never from a view saved in a wide window.
            self.inspector = view.inspector && self.was_wide;
            self.group_mode = view.group_mode;
            self.delete_categories = view.delete_categories.iter().copied().collect();
            self.rotation_queue_only = view.rotation_queue_only;
            self.active = view.active;
            self.selection
                .restore(view.selected.iter().copied(), view.anchor);
            if self.step != Step::PickBest {
                self.comparison = view.comparison.clone();
            }
            self.library_offset = view.scroll;
            self.library_restore_offset = Some(view.scroll);
            self.library_reveal = false;
        }
        self.discard_stale_model_retry();
        self.filter_dirty = true;
    }
    pub(super) fn restore_bookmark(&mut self) {
        if self.scanning
            || self.busy
            || self.close_authorized
            || self.after_apply.is_some()
            || self.resume_suspended
        {
            return;
        }
        let Some(view) = self.pending_bookmark.as_ref() else {
            return;
        };
        if !self.resume_started
            || (!self.view_data_ready.contains(&self.step) && self.analysis_run.is_none())
        {
            self.resume_started = true;
            match self.step {
                Step::PickBest if !self.view_data_ready.contains(&self.step) => {
                    self.start_ready_analysis()
                }
                Step::EasyDelete if !self.view_data_ready.contains(&self.step) => self.analyze(),
                Step::Cull
                    if (view.group_anchor.is_some()
                        || view.browse.group_sort != crate::core::browsing::GroupSort::Flat)
                        && self.cull_groups.is_empty() =>
                {
                    self.analyze()
                }
                Step::Rotation
                    if view.rotation_queue_only && self.rotation_review.ids().is_empty() =>
                {
                    if crate::core::models::orientation_model().is_some() {
                        self.detect_rotations();
                    } else {
                        self.rotation_queue_only = false;
                        self.filter_dirty = true;
                    }
                }
                _ => {}
            }
        }
        if self.busy
            || self.analysis_run.is_some()
            || self.metadata_run.is_some()
            || self.filter_dirty
            || self.browse_run.is_some()
        {
            return;
        }
        let view = self.pending_bookmark.clone().unwrap();
        if self.step == Step::Cull {
            let group = view
                .group_anchor
                .and_then(|id| self.cull_groups.get(&id).copied());
            if self.browse.group != group {
                self.browse.group = group;
                self.filter_dirty = true;
                return;
            }
        }
        if self.step == Step::PickBest {
            // The group of the saved photo, if it is still one.
            self.pick_current = view
                .active
                .filter(|id| self.pick_review.group_of(*id).is_some());
        } else if self.step == Step::EasyDelete {
            if let Some(id) = view.delete_current.filter(|id| {
                self.delete_review.entry(*id).is_some()
                    && self
                        .index
                        .get(id)
                        .is_some_and(|i| self.filtered.contains(i))
            }) {
                self.delete_current = Some(id);
            }
        } else {
            self.comparison = view
                .comparison
                .iter()
                .copied()
                .filter(|id| {
                    self.index
                        .get(id)
                        .is_some_and(|i| self.filtered.contains(i))
                })
                .collect();
            self.selection
                .restore(view.selected.iter().copied(), view.anchor);
            self.selection.reconcile(&self.media, &self.filtered);
        }
        let candidates = if matches!(self.step, Step::EasyDelete | Step::PickBest) {
            self.comparison_ids()
        } else {
            self.filtered.iter().map(|i| self.media[*i].id).collect()
        };
        self.active = view
            .active
            .filter(|id| candidates.contains(id))
            .or_else(|| {
                if view.active.is_some()
                    || view.missing_focus
                    || matches!(self.step, Step::EasyDelete | Step::PickBest)
                {
                    candidates.first().copied()
                } else {
                    None
                }
            });
        self.focused_slot = self
            .active
            .and_then(|id| self.comparison_ids().iter().position(|p| *p == id))
            .unwrap_or(0) as u64;
        self.inspection_view = view.view;
        self.independent_views = view.independent_views;
        self.sync_views = view.sync_views;
        self.library_offset = view.scroll;
        self.library_restore_offset = Some(view.scroll);
        self.library_reveal = false;
        self.pending_bookmark = None;
        self.view_data_ready.insert(self.step);
        self.view_started = Instant::now();
    }
    /// A different library starts every step with a clean view: Group by
    /// "Current folders", no search or filter, every Easy Delete tab, no
    /// selection, zoom or open panel, scrolled to the top. Staged changes are
    /// not views; the new folder's own session restores them. App-wide
    /// choices stay: preferences, thumbnail and photo sizes, panel widths.
    pub(super) fn reset_folder_views(&mut self) {
        let view = Bookmark::default();
        self.pending_bookmark = None;
        self.active = view.active;
        self.selection.clear();
        self.comparison = view.comparison;
        self.single_view_stash.clear();
        self.review = self.step != Step::Organize;
        self.inspector = view.inspector && self.was_wide;
        self.library_layout = view.layout;
        self.query = view.query;
        self.filter = view.filter;
        self.browse = view.browse;
        self.navigator = view.navigator;
        self.browse_filters_open = false;
        self.group_mode = view.group_mode;
        self.delete_current = view.delete_current;
        self.delete_categories = view.delete_categories.into_iter().collect();
        self.delete_panel = false;
        self.rotation_queue_only = view.rotation_queue_only;
        self.pick_current = None;
        self.pick_panel = false;
        self.inspection_view = view.view;
        self.independent_views = view.independent_views;
        self.fit_zooms.clear();
        self.sync_views = view.sync_views;
        self.focused_slot = 0;
        self.library_offset = view.scroll;
        self.library_reveal = true;
        // Organize's sidebar focus, preview, "Show original", renames and
        // face merges belong to the folder; the thumbnail size does not.
        self.board = organize_board::Board {
            tile: self.board.tile,
            ..Default::default()
        };
        self.forget_scroll = true;
        self.filter_dirty = true;
    }
    pub(super) fn load_bookmarks(&mut self, views: Saved) {
        self.saved_views = views;
        let step = self.saved_views.step.unwrap_or(self.step);
        self.step = if self.preferences.visible(step) {
            step
        } else {
            self.preferences.neighbor(step, true).unwrap_or(Step::Cull)
        };
        self.review = self.step != Step::Organize;
        self.prepare_bookmark();
    }
}

#[cfg(test)]
#[path = "bookmarks/tests.rs"]
mod tests;

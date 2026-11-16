//! Application ownership of durable group edits; all layouts use these commands.
use super::*;
use crate::core::manual_groups::{Edit, Resolved};
pub(super) struct Task {
    run: u64,
    scene: bool,
    pub cancelled: Arc<std::sync::atomic::AtomicBool>,
    pub source: Option<u64>,
    record_undo: bool,
}
impl PhotoSortApp {
    pub(super) fn scene_editing(&self) -> bool {
        self.step == Step::Organize && self.group_mode == GroupMode::Visual
    }
    pub(super) fn library_selection_mode(&self) -> bool {
        matches!(self.step, Step::Cull | Step::Organize) && !self.review
    }
    fn groups_ready(&self) -> bool {
        (self.step == Step::Cull
            || (self.scene_editing()
                && self.visual_groups.is_some()
                && !self.organize_drafts.contains_key(&GroupMode::Visual)))
            && !self.busy
            && !self.scanning
            && !self.filter_dirty
            && self.browse_run.is_none()
            && self.analysis_run.is_none()
            && self.group_task.is_none()
            && !self.plan_open
            && self.departure.is_none()
    }
    /// A Cull photo's similarity group when it has a best photo: the
    /// group, its best photo and how many photos it holds, counting those a
    /// filter hides.
    pub(super) fn similar_group(&self, id: MediaId) -> Option<(usize, MediaId, usize)> {
        if self.step != Step::Cull {
            return None;
        }
        let group = *self.cull_groups.get(&id)?;
        let (best, size) = *self.group_best.get(&group)?;
        Some((group, best, size))
    }
    /// The photo's place in its group, in Pick Best's order: 1 for the
    /// best photo.
    pub(super) fn group_rank(&self, id: MediaId, group: usize) -> usize {
        let only = HashSet::from([group]);
        self.ranked_groups(Some(&only))
            .get(&group)
            .and_then(|ordered| ordered.iter().position(|other| *other == id))
            .map_or(1, |place| place + 1)
    }
    /// Each Cull group's photos from best to worst, for the groups in
    /// `only` or for all of them: see `ranked_groups`.
    pub(super) fn ranked_groups(
        &self,
        only: Option<&HashSet<usize>>,
    ) -> HashMap<usize, Vec<MediaId>> {
        ranked_groups(
            &self.cull_groups,
            &self.analysis,
            &self.metadata,
            |id| {
                self.index.get(&id).map(|i| {
                    crate::core::dates::capture_order(&self.media[*i], self.metadata.get(&id))
                })
            },
            only,
        )
    }
    pub(super) fn publish_groups(&mut self, groups: Arc<HashMap<MediaId, usize>>) {
        // Restore a group filter by photo identity, not yesterday's AI numbering.
        let anchor = self.browse.group.and_then(|group| {
            self.active
                .filter(|id| self.cull_groups.get(id) == Some(&group))
                .or_else(|| {
                    self.media
                        .iter()
                        .find(|m| self.cull_groups.get(&m.id) == Some(&group))
                        .map(|m| m.id)
                })
        });
        self.cull_groups = groups;
        self.group_best = group_best(&self.ranked_groups(None), &self.analysis);
        if self.browse.group.is_some() {
            self.browse.group = anchor.and_then(|id| self.cull_groups.get(&id).copied());
        }
        // Numeric collapsed group keys must not attach to a different AI cohort.
        self.navigator
            .collapsed
            .retain(|key| !matches!(key, crate::core::browsing::tree::Key::Group(_)));
        self.runtime.browsing.groups(self.cull_groups.clone());
        self.filter_dirty = true;
        self.library_reveal = true;
    }
    pub(super) fn resolve_groups(
        &mut self,
        automatic: Arc<HashMap<MediaId, usize>>,
        edit: Edit,
        source: Option<u64>,
        record_undo: bool,
    ) {
        self.resolve_group_edit(false, automatic, edit, source, record_undo);
    }
    fn resolve_group_edit(
        &mut self,
        scene: bool,
        automatic: Arc<HashMap<MediaId, usize>>,
        edit: Edit,
        source: Option<u64>,
        record_undo: bool,
    ) {
        let Some(catalog) = self.catalog.clone() else {
            return;
        };
        self.group_serial += 1;
        let run = self.group_serial;
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        match self.runtime.io(IoWork::EditGroups {
            cancelled: cancelled.clone(),
            run,
            catalog,
            automatic,
            manual: if scene {
                self.scene_groups.clone()
            } else {
                self.manual_groups.clone()
            },
            scene,
            edit,
        }) {
            Ok(()) => {
                self.group_task = Some(Task {
                    cancelled,
                    run,
                    scene,
                    source,
                    record_undo,
                });
                self.busy = true;
                self.clear_status();
            }
            Err(error) => self.error = Some(error),
        }
    }
    pub(super) fn cancel_group_edit(&mut self) {
        if let Some(task) = self.group_task.take() {
            task.cancelled
                .store(true, std::sync::atomic::Ordering::Release);
            self.busy = false;
            if task.source.is_some() {
                self.cancel_analysis();
            }
            self.clear_status();
        }
    }
    fn edit_groups(&mut self, edit: Edit, record_undo: bool) {
        if !self.groups_ready() {
            return;
        }
        self.pending_bookmark = None;
        let scene = self.scene_editing();
        let automatic = if scene {
            self.automatic_scenes.clone()
        } else {
            self.automatic_groups.clone()
        };
        self.resolve_group_edit(scene, automatic, edit, None, record_undo);
    }
    pub(super) fn finish_group_edit(&mut self, result: Resolved) {
        if self
            .group_task
            .as_ref()
            .is_none_or(|task| task.run != result.run)
        {
            return;
        }
        let task = self.group_task.take().unwrap();
        self.busy = false;
        if task.scene {
            if result.changed {
                self.scene_undo = task.record_undo.then_some(result.previous);
            }
            self.scene_groups = result.manual;
            self.automatic_scenes = result.automatic;
            self.scene_choices = result.choices;
            self.visual_groups = Some(result.groups);
            self.filter_dirty = true;
            if result.changed {
                self.save_session();
            }
            self.set_organize_status(GroupMode::Visual, String::new());
            return;
        }
        if task.record_undo && result.changed {
            self.group_undo = Some(result.previous);
        }
        let changed = result.changed;
        if changed && !task.record_undo {
            self.group_undo = None;
        }
        self.manual_groups = result.manual;
        self.automatic_groups = result.automatic;
        self.publish_groups(Arc::new(result.groups));
        if changed {
            self.save_session();
        }
        self.set_workflow_status(
            Step::Cull,
            if changed && self.manual_groups.is_empty() && self.automatic_groups.is_empty() {
                "Manual groups removed · Analyze to build automatic groups".into()
            } else {
                String::new()
            },
        );
        if task.source.is_some() && self.pick_run == task.source && self.analysis_run.is_none() {
            self.finish_pick_analysis();
        } else {
            self.start_ready_analysis();
        }
        if self.step == Step::Cull || self.metadata_organize_mode() {
            self.start_metadata_index();
        }
    }
    fn selected_group_members(&self) -> Vec<MediaId> {
        if self.scene_editing() && self.review {
            if self.comparison.is_empty() {
                self.active.into_iter().collect()
            } else {
                self.comparison.clone()
            }
        } else {
            self.selection.ordered(&self.media, &self.filtered)
        }
    }
    /// Dragging photos onto a similar set's header: Cull, with groups to move into.
    pub(super) fn photo_drag_enabled(&self) -> bool {
        !self.review
            && match self.step {
                Step::Cull => !self.cull_groups.is_empty() && self.groups_ready(),
                // Onto a proposed folder in Organize's board.
                Step::Organize => self.board_editable(),
                _ => false,
            }
    }
    /// Moves dropped photos into `group` with the same edit as the Groups menu.
    pub(super) fn drop_on_group(&mut self, drag: &PhotoDrag, group: usize) {
        if !self.groups_ready() || drag.0.is_empty() {
            return;
        }
        self.edit_groups(
            Edit::Move {
                selected: drag.0.clone(),
                target: Some(group),
            },
            true,
        );
    }
    /// A label at the pointer while photos are being dragged.
    pub(super) fn photo_drag_overlay(&self, ctx: &egui::Context) {
        let Some(drag) = egui::DragAndDrop::payload::<PhotoDrag>(ctx) else {
            return;
        };
        let Some(pointer) = ctx.pointer_latest_pos() else {
            return;
        };
        ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
        let count = drag.0.len();
        let text = if self.step == Step::Organize {
            format!(
                "Move {count} {} to a folder",
                if count == 1 { "item" } else { "items" }
            )
        } else if self.grouped_view() {
            format!(
                "Move {count} {} to a similar set",
                if count == 1 { "photo" } else { "photos" }
            )
        } else {
            "Group similar photos into sets, then drop on a set".to_owned()
        };
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Tooltip,
            egui::Id::new("photo-drag"),
        ));
        let galley = painter.layout_no_wrap(text, medium(13.0), ON_ACCENT);
        let rect = egui::Rect::from_min_size(
            pointer + egui::vec2(14.0, 10.0),
            galley.size() + egui::vec2(20.0, 10.0),
        );
        painter.rect_filled(rect, rect.height() / 2.0, ACCENT);
        painter.galley(rect.min + egui::vec2(10.0, 5.0), galley, ON_ACCENT);
    }
    fn move_selected_group(&mut self, target: Option<usize>) {
        let mut selected = self.selected_group_members();
        if self.scene_editing() {
            selected.retain(|id| self.automatic_scenes.contains_key(id));
        }
        self.edit_groups(Edit::Move { selected, target }, true);
    }
    fn group_targets(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().max_height(240.).show_rows(
            ui,
            button_row_height(ui),
            if self.scene_editing() {
                self.scene_choices.len()
            } else {
                self.browse_groups.len()
            },
            |ui, rows| {
                for row in rows {
                    let scene = self.scene_editing();
                    let group = if scene {
                        self.scene_choices[row].0
                    } else {
                        self.browse_groups[row]
                    };
                    let label = if scene {
                        let (_, count, example) = self.scene_choices[row];
                        let name = self
                            .index
                            .get(&example)
                            .map(|i| self.shown_name(&self.media[*i]))
                            .unwrap_or_default();
                        let photos = if count == 1 { "photo" } else { "photos" };
                        format!("Scene {group:03} · {count} {photos} · {name}")
                    } else {
                        self.group_choices.get(&group).map_or_else(
                            || format!("Group {group}"),
                            |choice| {
                                let name = self
                                    .index
                                    .get(&choice.example)
                                    .map(|i| self.shown_name(&self.media[*i]))
                                    .unwrap_or_default();
                                let photos = if choice.count == 1 { "photo" } else { "photos" };
                                format!("Group {group} · {} {photos} · {name}", choice.count)
                            },
                        )
                    };
                    if ui
                        .add(egui::Button::new(&label).truncate())
                        .on_hover_text(&label)
                        .clicked()
                    {
                        self.move_selected_group(Some(group));
                        ui.close();
                    }
                }
            },
        );
    }
    pub(super) fn group_controls(&mut self, ui: &mut egui::Ui) {
        let scene = self.scene_editing();
        let undo = if scene {
            self.scene_undo.clone()
        } else {
            self.group_undo.clone()
        };
        let empty = if scene {
            self.scene_groups.is_empty()
        } else {
            self.manual_groups.is_empty()
        };
        ui.add_enabled_ui(self.groups_ready(), |ui| {
            ui.menu_button(if scene { "Scenes" } else { "Groups" }, |ui| {
                let selected = self.selected_group_members();
                ui.set_max_width(300.);
                ui.add_enabled_ui(!selected.is_empty(), |ui| {
                    if ui.button("Move selected to new group").clicked() {
                        self.move_selected_group(None);
                        ui.close();
                    }
                    ui.menu_button("Move selected to existing group", |ui| {
                        self.group_targets(ui)
                    });
                    if ui
                        .button("Restore automatic grouping for selected")
                        .clicked()
                    {
                        let selected = selected.clone();
                        self.edit_groups(Edit::Restore(selected), true);
                        ui.close();
                    }
                });
                ui.separator();
                if ui
                    .add_enabled(undo.is_some(), egui::Button::new("Undo last group edit"))
                    .clicked()
                    && let Some(previous) = undo.clone()
                {
                    self.edit_groups(Edit::Replace(previous), false);
                    ui.close();
                }
                if ui
                    .add_enabled(!empty, egui::Button::new("Restore all automatic groups"))
                    .clicked()
                {
                    self.edit_groups(Edit::RestoreAll, true);
                    ui.close();
                }
            });
        });
    }
}

#[cfg(test)]
#[path = "grouping/tests.rs"]
mod tests;

/// Photos being dragged from the grid onto a similarity group.
pub(super) struct PhotoDrag(pub(super) Vec<MediaId>);

/// Each group's photos from best to worst, by the one rule Pick Best also
/// suggests from (`pick_review::rank`): a smaller copy never over the photo
/// it copies, then sharpness, eyes and score. `order` is what settles a tie,
/// the capture order, as in Pick Best. Only the groups in `only`, when given.
pub(super) fn ranked_groups<K: Ord>(
    groups: &HashMap<MediaId, usize>,
    analysis: &HashMap<MediaId, crate::core::analysis::Analysis>,
    metadata: &HashMap<MediaId, Metadata>,
    order: impl Fn(MediaId) -> K,
    only: Option<&HashSet<usize>>,
) -> HashMap<usize, Vec<MediaId>> {
    let mut members: HashMap<usize, Vec<MediaId>> = HashMap::new();
    for (id, group) in groups {
        if only.is_none_or(|only| only.contains(group)) {
            members.entry(*group).or_default().push(*id);
        }
    }
    members
        .into_iter()
        .map(|(group, mut ids)| {
            ids.sort_by_cached_key(|id| (order(*id), id.0));
            let ranked = crate::core::pick_review::rank(&ids, analysis, metadata);
            (group, ranked.ordered)
        })
        .collect()
}

/// The best photo of every ranked group that has a choice to make, by
/// group, with how many photos the group holds: its first photo, when the
/// group has two or more and that photo has a score.
pub(super) fn group_best(
    ranked: &HashMap<usize, Vec<MediaId>>,
    analysis: &HashMap<MediaId, crate::core::analysis::Analysis>,
) -> HashMap<usize, (MediaId, usize)> {
    ranked
        .iter()
        .filter_map(|(group, ordered)| {
            let best = *ordered.first()?;
            let scored = analysis.get(&best).is_some_and(|a| a.score.is_finite());
            (ordered.len() > 1 && scored).then_some((*group, (best, ordered.len())))
        })
        .collect()
}

/// A similarity group's mark in Review's photo strip: muted hues that stay
/// clear of the accent, Trash red and Keep green.
pub(super) fn group_color(group: usize) -> Color32 {
    const HUES: [Color32; 6] = [
        Color32::from_rgb(0x5F, 0xA8, 0xA6),
        Color32::from_rgb(0x8F, 0x86, 0xC9),
        Color32::from_rgb(0x6A, 0x93, 0xD6),
        Color32::from_rgb(0xB0, 0xA8, 0x5E),
        Color32::from_rgb(0xC2, 0x82, 0xA4),
        Color32::from_rgb(0x7F, 0xA8, 0x74),
    ];
    HUES[group % HUES.len()]
}

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
        (self.step == Step::Cull || self.scene_editing()) && !self.review
    }
    fn groups_ready(&self) -> bool {
        (self.step == Step::Cull
            || (self.scene_editing()
                && self.visual_groups.is_some()
                && !self.organize_drafts.contains_key(&GroupMode::Visual)
                && self.organize_editor.is_none()))
            && !self.busy
            && !self.scanning
            && !self.filter_dirty
            && self.browse_run.is_none()
            && self.analysis_run.is_none()
            && self.group_task.is_none()
            && !self.plan_open
            && self.departure.is_none()
            && self.review_notice.is_none()
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
                self.set_status("Updating photo groups…".into());
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
            self.set_status("Group update cancelled · previous groups retained".into());
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
            self.set_organize_status(GroupMode::Visual, format!("Scenes updated · {} photos manually grouped · used by the next Easy Delete analysis", self.scene_groups.len()));
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
                format!(
                    "Groups ready · {} photos manually grouped",
                    self.manual_groups.len()
                )
            },
        );
        if task.source.is_some() && self.pick_run == task.source && self.analysis_run.is_none() {
            self.finish_pick_analysis();
        } else {
            self.start_ready_pick_analysis();
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
                            .map(|i| self.media[*i].name())
                            .unwrap_or_default();
                        format!("Scene {group:03} · {count} photos · {name}")
                    } else {
                        self.group_choices.get(&group).map_or_else(
                            || format!("Group {group}"),
                            |choice| {
                                let name = self
                                    .index
                                    .get(&choice.example)
                                    .map(|i| self.media[*i].name())
                                    .unwrap_or_default();
                                format!("Group {group} · {} photos · {name}", choice.count)
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
                ui.label(if scene { "Scene corrections survive reanalysis and guide the next Easy Delete analysis. Mixed date grouping is independent." } else { "Manual groups stay together after reanalysis. They also guide new Pick Best reviews." });
                ui.label(format!("{} selected", selected.len()));
                ui.label("Photos only. Folder locations are unchanged.");
                ui.separator();
                ui.add_enabled_ui(!selected.is_empty(), |ui| {
                    if ui.button("Move selected to new group").clicked() {
                        self.move_selected_group(None); ui.close();
                    }
                    ui.menu_button("Move selected to existing group", |ui| {
                        ui.label("The target’s current members stay together.");
                        self.group_targets(ui);

                    });
                    if ui.button("Restore automatic grouping for selected").clicked() {
                        let selected = selected.clone();
                        self.edit_groups(Edit::Restore(selected),true); ui.close();
                    }
                });
                ui.separator();
                if ui.add_enabled(undo.is_some(),egui::Button::new("Undo last group edit")).clicked()
                    && let Some(previous) = undo.clone() {
                        self.edit_groups(Edit::Replace(previous),false); ui.close();
                    }
                if ui.add_enabled(!empty,egui::Button::new("Restore all automatic groups")).clicked() {
                    self.edit_groups(Edit::RestoreAll,true); ui.close();
                }
            });
        });
    }
}

#[cfg(test)]
#[path = "grouping/tests.rs"]
mod tests;
